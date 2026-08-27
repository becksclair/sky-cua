//! Host-side finite inbound content transfers for phone-control.v2.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use sha2::{Digest, Sha256};
use sky_cua_platform::{
    appshot_artifacts_dir,
    model::{ContentChunkHeader, ContentTransferCommit, ContentTransferDeclaration},
};
use uuid::Uuid;
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
const MAX_ACTIVE_TRANSFERS: usize = 8;
const MAX_ACTIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_COMMITTED_ARTIFACTS: usize = 32;
const MAX_COMMITTED_BYTES: u64 = 256 * 1024 * 1024;

struct ActiveTransfer {
    declaration: ContentTransferDeclaration,
    temp_path: PathBuf,
    file: File,
    hasher: Sha256,
    next_index: u64,
    next_offset: u64,
}

struct CommittedArtifact {
    path: PathBuf,
    sha256: String,
    mime_type: String,
    size: u64,
    epoch: u64,
    expires_at_ms: u64,
}

impl Drop for ActiveTransfer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temp_path);
    }
}

/// Owns private temporary files and the small committed-artifact index for one
/// authenticated link. Dropping it removes every incomplete transfer.
#[derive(Default)]
pub(crate) struct InboundContentStore {
    active: HashMap<String, ActiveTransfer>,
    committed: HashMap<String, CommittedArtifact>,
    active_bytes: u64,
    committed_bytes: u64,
    now_override: Option<u64>,
}

struct TempPath(Option<PathBuf>);
impl Drop for TempPath {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn decode_chunk(bytes: &[u8]) -> io::Result<(ContentChunkHeader, &[u8])> {
    let id_len = usize::from(
        *bytes
            .first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing transfer id"))?,
    );
    if id_len == 0 || bytes.len() < 29 + id_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "truncated chunk header",
        ));
    }
    let id = std::str::from_utf8(&bytes[1..1 + id_len])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid transfer id"))?
        .to_owned();
    let mut p = 1 + id_len;
    let take8 = |b: &[u8], p: &mut usize| {
        let x = u64::from_be_bytes(b[*p..*p + 8].try_into().unwrap());
        *p += 8;
        x
    };
    let epoch = take8(bytes, &mut p);
    let index = take8(bytes, &mut p);
    let offset = take8(bytes, &mut p);
    let length = u32::from_be_bytes(bytes[p..p + 4].try_into().unwrap());
    p += 4;
    let payload = &bytes[p..];
    if payload.len() != length as usize || payload.len() > 256 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunk length mismatch",
        ));
    }
    Ok((
        ContentChunkHeader {
            transfer_id: id,
            chunk_index: index,
            offset,
            length,
            link_epoch: epoch,
        },
        payload,
    ))
}

impl InboundContentStore {
    fn now(&self) -> u64 {
        self.now_override.unwrap_or_else(now_ms)
    }
    #[cfg(test)]
    fn set_now(&mut self, now: u64) {
        self.now_override = Some(now);
    }
    pub(crate) fn declare(
        &mut self,
        declaration: ContentTransferDeclaration,
        epoch: u64,
    ) -> io::Result<()> {
        if declaration.link_epoch != epoch
            || self.active.contains_key(&declaration.transfer_id)
            || self.active.len() >= MAX_ACTIVE_TRANSFERS
            || self
                .active_bytes
                .saturating_add(declaration.content.size_bytes)
                > MAX_ACTIVE_BYTES
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid or duplicate transfer declaration",
            ));
        }
        let dir = appshot_artifacts_dir();
        fs::create_dir_all(&dir)?;
        let temp_path = dir.join(format!(
            ".phone-transfer-{}-{}",
            declaration.transfer_id,
            Uuid::new_v4()
        ));
        let mut temp_guard = TempPath(Some(temp_path.clone()));
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(&temp_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))?;
        }
        let declared_bytes = declaration.content.size_bytes;
        self.active.insert(
            declaration.transfer_id.clone(),
            ActiveTransfer {
                declaration,
                temp_path,
                file,
                hasher: Sha256::new(),
                next_index: 0,
                next_offset: 0,
            },
        );
        self.active_bytes = self.active_bytes.saturating_add(declared_bytes);
        temp_guard.0 = None;
        Ok(())
    }

    pub(crate) fn chunk(&mut self, bytes: &[u8], epoch: u64) -> io::Result<()> {
        let (header, payload) = decode_chunk(bytes)?;
        let transfer = self
            .active
            .get_mut(&header.transfer_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown transfer"))?;
        let d = &transfer.declaration;
        if header.link_epoch != epoch
            || d.link_epoch != epoch
            || header.chunk_index != transfer.next_index
            || header.offset != transfer.next_offset
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk epoch, index, or offset mismatch",
            ));
        }
        let expected = if header.chunk_index + 1 == d.chunk_count {
            d.content.size_bytes.saturating_sub(header.offset)
        } else {
            u64::from(d.chunk_bytes)
        };
        if u64::from(header.length) != expected
            || header.offset.saturating_add(u64::from(header.length)) > d.content.size_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chunk length mismatch",
            ));
        }
        transfer.file.seek(SeekFrom::Start(header.offset))?;
        transfer.file.write_all(payload)?;
        transfer.hasher.update(payload);
        transfer.next_index += 1;
        transfer.next_offset = transfer
            .next_offset
            .saturating_add(u64::from(header.length));
        Ok(())
    }

    pub(crate) fn commit(
        &mut self,
        commit: ContentTransferCommit,
        epoch: u64,
    ) -> io::Result<String> {
        let mut transfer = self
            .active
            .remove(&commit.transfer_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown transfer"))?;
        self.active_bytes = self
            .active_bytes
            .saturating_sub(transfer.declaration.content.size_bytes);
        let d = &transfer.declaration;
        let digest = transfer
            .hasher
            .clone()
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if commit.link_epoch != epoch
            || d.link_epoch != epoch
            || transfer.next_index != d.chunk_count
            || transfer.next_offset != d.content.size_bytes
            || commit.size_bytes != d.content.size_bytes
            || commit.sha256 != d.content.sha256
            || digest != d.content.sha256
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "transfer commit mismatch",
            ));
        }
        transfer.file.flush()?;
        transfer.file.sync_all()?;
        let dir = appshot_artifacts_dir();
        let final_name = format!(
            "phone-content-{}",
            d.content.content_id.replace(['/', '\\'], "_")
        );
        let final_path = dir.join(final_name);
        if self.committed.len() >= MAX_COMMITTED_ARTIFACTS
            || self.committed_bytes.saturating_add(d.content.size_bytes) > MAX_COMMITTED_BYTES
        {
            return Err(io::Error::other("committed artifact bound exceeded"));
        }
        fs::rename(&transfer.temp_path, &final_path)?;
        self.committed_bytes = self.committed_bytes.saturating_add(d.content.size_bytes);
        let expires = d
            .content
            .expires_at_ms
            .unwrap_or_else(|| self.now().saturating_add(15 * 60 * 1000));
        self.committed.insert(
            d.content.content_id.clone(),
            CommittedArtifact {
                path: final_path,
                sha256: d.content.sha256.clone(),
                mime_type: d.content.mime_type.clone(),
                size: d.content.size_bytes,
                epoch: d.link_epoch,
                expires_at_ms: expires,
            },
        );
        Ok(d.content.content_id.clone())
    }

    pub(crate) fn abort(&mut self, transfer_id: &str) {
        if let Some(t) = self.active.remove(transfer_id) {
            self.active_bytes = self
                .active_bytes
                .saturating_sub(t.declaration.content.size_bytes);
        }
    }
    pub(crate) fn abort_epoch(&mut self, epoch: u64) {
        let ids = self
            .active
            .iter()
            .filter(|(_, t)| t.declaration.link_epoch == epoch)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.abort(&id);
        }
    }
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn read_artifact(
        &mut self,
        content_id: &str,
        device_epoch: u64,
    ) -> io::Result<Vec<u8>> {
        self.read_artifact_inner(content_id, device_epoch, None)
    }

    pub(crate) fn read_artifact_verified(
        &mut self,
        content_id: &str,
        device_epoch: u64,
        expected_size: u64,
        expected_sha256: &str,
    ) -> io::Result<Vec<u8>> {
        self.read_artifact_inner(
            content_id,
            device_epoch,
            Some((expected_size, expected_sha256)),
        )
    }

    pub(crate) fn describe_artifact(
        &mut self,
        content_id: &str,
        device_epoch: u64,
    ) -> io::Result<(String, u64, String, u64)> {
        self.remove_if_expired(content_id)?;
        let artifact = self
            .committed
            .get(content_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown artifact"))?;
        if artifact.epoch != device_epoch {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "artifact epoch mismatch",
            ));
        }
        Ok((
            artifact.sha256.clone(),
            artifact.size,
            artifact.mime_type.clone(),
            artifact.expires_at_ms,
        ))
    }

    pub(crate) fn release_artifact(
        &mut self,
        content_id: &str,
        device_epoch: u64,
    ) -> io::Result<()> {
        let artifact = self
            .committed
            .get(content_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown artifact"))?;
        if artifact.epoch != device_epoch {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "artifact epoch mismatch",
            ));
        }
        let artifact = self.committed.remove(content_id).expect("artifact exists");
        self.committed_bytes = self.committed_bytes.saturating_sub(artifact.size);
        let _ = fs::remove_file(artifact.path);
        Ok(())
    }

    fn read_artifact_inner(
        &mut self,
        content_id: &str,
        device_epoch: u64,
        expected: Option<(u64, &str)>,
    ) -> io::Result<Vec<u8>> {
        self.remove_if_expired(content_id)?;
        let artifact = self
            .committed
            .get(content_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown artifact"))?;
        if artifact.epoch != device_epoch {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "artifact epoch mismatch",
            ));
        }
        if expected.is_some_and(|(expected_size, expected_sha256)| {
            expected_size != artifact.size || expected_sha256 != artifact.sha256
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "artifact size or digest does not match ContentRef",
            ));
        }
        let mut f = File::open(&artifact.path)?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn remove_if_expired(&mut self, content_id: &str) -> io::Result<()> {
        let expired = self
            .committed
            .get(content_id)
            .is_some_and(|artifact| artifact.expires_at_ms <= self.now());
        if expired {
            let artifact = self
                .committed
                .remove(content_id)
                .expect("artifact entry exists");
            self.committed_bytes = self.committed_bytes.saturating_sub(artifact.size);
            let _ = fs::remove_file(artifact.path);
            return Err(io::Error::new(io::ErrorKind::NotFound, "artifact expired"));
        }
        Ok(())
    }
}

impl Drop for InboundContentStore {
    fn drop(&mut self) {
        for (_, _t) in self.active.drain() {}
        for (_, artifact) in self.committed.drain() {
            let _ = fs::remove_file(artifact.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{
        ContentPersistence, ContentRef, ContentSource, encode_content_chunk,
    };

    use super::*;

    fn declaration(size: u64, sha: &str) -> ContentTransferDeclaration {
        ContentTransferDeclaration {
            transfer_id: "t".into(),
            device_id: "d".into(),
            link_epoch: 3,
            content: ContentRef {
                content_id: "shot".into(),
                device_id: Some("d".into()),
                link_epoch: Some(3),
                mime_type: "image/png".into(),
                filename: None,
                size_bytes: size,
                sha256: sha.into(),
                source: ContentSource::Screenshot,
                expires_at_ms: None,
                persistence: ContentPersistence::Temporary,
            },
            chunk_bytes: 3,
            chunk_count: size.div_ceil(3),
        }
    }

    #[test]
    fn multi_chunk_commit_and_read_is_atomic() {
        let bytes = b"abcdef";
        let sha = format!("{:x}", Sha256::digest(bytes));
        let mut s = InboundContentStore::default();
        s.declare(declaration(6, &sha), 3).unwrap();
        let a = encode_content_chunk(
            &ContentChunkHeader {
                transfer_id: "t".into(),
                chunk_index: 0,
                offset: 0,
                length: 3,
                link_epoch: 3,
            },
            &bytes[..3],
        )
        .unwrap();
        let b = encode_content_chunk(
            &ContentChunkHeader {
                transfer_id: "t".into(),
                chunk_index: 1,
                offset: 3,
                length: 3,
                link_epoch: 3,
            },
            &bytes[3..],
        )
        .unwrap();
        s.chunk(&a, 3).unwrap();
        s.chunk(&b, 3).unwrap();
        assert!(s.read_artifact("shot", 3).is_err());
        s.commit(
            ContentTransferCommit {
                transfer_id: "t".into(),
                size_bytes: 6,
                sha256: sha.clone(),
                link_epoch: 3,
            },
            3,
        )
        .unwrap();
        assert_eq!(s.read_artifact("shot", 3).unwrap(), bytes);
        assert_eq!(s.read_artifact_verified("shot", 3, 6, &sha).unwrap(), bytes);
        assert!(s.read_artifact_verified("shot", 3, 5, &sha).is_err());
        assert!(
            s.read_artifact_verified("shot", 3, 6, &"00".repeat(32))
                .is_err()
        );
    }

    fn chunk(id: &str, index: u64, offset: u64, epoch: u64, bytes: &[u8]) -> Vec<u8> {
        encode_content_chunk(
            &ContentChunkHeader {
                transfer_id: id.into(),
                chunk_index: index,
                offset,
                length: bytes.len() as u32,
                link_epoch: epoch,
            },
            bytes,
        )
        .unwrap()
    }

    #[test]
    fn digest_mismatch_removes_temp_and_never_commits() {
        let mut d = declaration(3, &format!("{:x}", Sha256::digest(b"bad")));
        d.transfer_id = "digest-mismatch".into();
        d.content.content_id = "digest-mismatch-content".into();
        let mut s = InboundContentStore::default();
        s.declare(d, 3).unwrap();
        s.chunk(&chunk("digest-mismatch", 0, 0, 3, b"abc"), 3)
            .unwrap();
        assert!(
            s.commit(
                ContentTransferCommit {
                    transfer_id: "digest-mismatch".into(),
                    size_bytes: 3,
                    sha256: "0".repeat(64),
                    link_epoch: 3
                },
                3
            )
            .is_err()
        );
        assert!(s.read_artifact("digest-mismatch-content", 3).is_err());
        assert!(s.active.is_empty());
    }

    #[test]
    fn skipped_and_duplicate_indices_are_rejected_without_append() {
        let mut d = declaration(6, &format!("{:x}", Sha256::digest(b"abcdef")));
        d.transfer_id = "ordering".into();
        d.content.content_id = "ordering-content".into();
        let mut s = InboundContentStore::default();
        s.declare(d, 3).unwrap();
        assert!(s.chunk(&chunk("ordering", 1, 3, 3, b"def"), 3).is_err());
        s.chunk(&chunk("ordering", 0, 0, 3, b"abc"), 3).unwrap();
        assert!(s.chunk(&chunk("ordering", 0, 0, 3, b"abc"), 3).is_err());
        assert_eq!(s.active.get("ordering").unwrap().next_offset, 3);
    }

    #[test]
    fn stale_epoch_is_rejected_and_abort_cleans_temp() {
        let mut d = declaration(3, &format!("{:x}", Sha256::digest(b"abc")));
        d.transfer_id = "stale".into();
        d.content.content_id = "stale-content".into();
        let mut s = InboundContentStore::default();
        assert!(s.declare(d.clone(), 2).is_err());
        s.declare(d, 3).unwrap();
        assert!(s.chunk(&chunk("stale", 0, 0, 2, b"abc"), 2).is_err());
        s.abort("stale");
        assert!(s.active.is_empty());
    }

    #[test]
    fn drop_cleans_incomplete_temp_and_committed_path_is_private() {
        let temp_path = {
            let mut d = declaration(3, &format!("{:x}", Sha256::digest(b"abc")));
            d.transfer_id = "drop-cleanup".into();
            d.content.content_id = "drop-content".into();
            let mut s = InboundContentStore::default();
            s.declare(d, 3).unwrap();
            s.active["drop-cleanup"].temp_path.clone()
        };
        assert!(!temp_path.exists());
        let mut d = declaration(3, &format!("{:x}", Sha256::digest(b"abc")));
        d.transfer_id = "private-path".into();
        d.content.content_id = "private-content".into();
        let mut s = InboundContentStore::default();
        s.declare(d, 3).unwrap();
        s.chunk(&chunk("private-path", 0, 0, 3, b"abc"), 3).unwrap();
        s.commit(
            ContentTransferCommit {
                transfer_id: "private-path".into(),
                size_bytes: 3,
                sha256: format!("{:x}", Sha256::digest(b"abc")),
                link_epoch: 3,
            },
            3,
        )
        .unwrap();
        let path = &s.committed.get("private-content").unwrap().path;
        assert_ne!(path.to_string_lossy(), "private-content");
        assert!(path.starts_with(appshot_artifacts_dir()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn active_count_and_bytes_caps_are_exact_and_rollback() {
        let mut s = InboundContentStore::default();
        for i in 0..8 {
            let mut d = declaration(1, &format!("{:x}", Sha256::digest([i as u8])));
            d.transfer_id = format!("cap-{i}");
            d.content.content_id = format!("cap-content-{i}");
            s.declare(d, 3).unwrap();
        }
        let mut ninth = declaration(1, &format!("{:x}", Sha256::digest([9u8])));
        ninth.transfer_id = "cap-9".into();
        ninth.content.content_id = "cap-content-9".into();
        assert!(s.declare(ninth, 3).is_err());
        assert_eq!(s.active.len(), 8);
        let mut bytes = declaration(64 * 1024 * 1024, &"00".repeat(32));
        bytes.transfer_id = "bytes-cap".into();
        bytes.content.content_id = "bytes-content".into();
        bytes.chunk_bytes = 262144;
        bytes.chunk_count = 256;
        assert!(s.declare(bytes, 3).is_err());
        s.abort("cap-0");
        assert_eq!(s.active.len(), 7);
        let mut b = InboundContentStore::default();
        let mut exact = declaration(64 * 1024 * 1024, &"00".repeat(32));
        exact.transfer_id = "exact-bytes".into();
        exact.content.content_id = "exact-content".into();
        exact.chunk_bytes = 262144;
        exact.chunk_count = 256;
        b.declare(exact, 3).unwrap();
        let mut over = declaration(1, &format!("{:x}", Sha256::digest([1u8])));
        over.transfer_id = "over-bytes".into();
        over.content.content_id = "over-content".into();
        assert!(b.declare(over, 3).is_err());
    }

    #[test]
    fn expired_artifact_is_removed_and_capacity_released() {
        let bytes = b"abc";
        let mut d = declaration(3, &format!("{:x}", Sha256::digest(bytes)));
        d.transfer_id = "expiry".into();
        d.content.content_id = "expiry-content".into();
        d.content.expires_at_ms = Some(10);
        let mut s = InboundContentStore::default();
        s.set_now(0);
        s.declare(d, 3).unwrap();
        s.chunk(&chunk("expiry", 0, 0, 3, bytes), 3).unwrap();
        s.commit(
            ContentTransferCommit {
                transfer_id: "expiry".into(),
                size_bytes: 3,
                sha256: format!("{:x}", Sha256::digest(bytes)),
                link_epoch: 3,
            },
            3,
        )
        .unwrap();
        assert_eq!(
            s.describe_artifact("expiry-content", 3).unwrap().2,
            "image/png"
        );
        assert_eq!(s.read_artifact("expiry-content", 3).unwrap(), bytes);
        s.set_now(10);
        assert!(s.describe_artifact("expiry-content", 3).is_err());
        assert!(!s.committed.contains_key("expiry-content"));
        assert_eq!(s.committed_bytes, 0);
    }
}
