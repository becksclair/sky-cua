//! Surface-neutral content descriptors used by AppShots and phone-control.v2.

use serde::{Deserialize, Serialize};

/// Default finite-transfer chunk size for phone-control.v2.
pub const PHONE_CONTENT_DEFAULT_CHUNK_BYTES: u32 = 256 * 1024;
/// Maximum UTF-8 JSON control-frame size accepted by phone-control.v2.
pub const PHONE_CONTROL_MAX_JSON_BYTES: u32 = 1024 * 1024;
/// Default lease for temporary content created on the phone.
pub const PHONE_CONTENT_DEFAULT_LEASE_MS: u64 = 15 * 60 * 1000;
/// Default lease for private host-side AppShot artifacts.
pub const APPSHOT_ARTIFACT_DEFAULT_LEASE_MS: u64 = 60 * 60 * 1000;
/// Largest UTF-8 transfer identifier encoded in a binary chunk header.
pub const PHONE_CONTENT_MAX_TRANSFER_ID_BYTES: usize = u8::MAX as usize;
/// Fixed scalar bytes after the length-prefixed transfer identifier.
pub const PHONE_CONTENT_CHUNK_FIXED_HEADER_BYTES: usize = 1 + 8 + 8 + 8 + 4;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentSource {
    CompanionBlob,
    HostPrivateArtifact,
    SharedPath,
    MediaStore,
    Saf,
    ContentUri,
    Clipboard,
    Screenshot,
    CameraPhoto,
    CameraVideo,
    CameraPreview,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentPersistence {
    Temporary,
    PersistedMediaStore,
    PersistedSaf,
    PersistedHostPath,
}

/// A typed reference to binary content. Bytes never appear in this descriptor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentRef {
    pub content_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_epoch: Option<u64>,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub source: ContentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    pub persistence: ContentPersistence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "ContentTransferDeclarationWire")]
pub struct ContentTransferDeclaration {
    pub transfer_id: String,
    pub device_id: String,
    pub link_epoch: u64,
    pub content: ContentRef,
    pub chunk_bytes: u32,
    pub chunk_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ContentTransferDeclarationWire {
    transfer_id: String,
    device_id: String,
    link_epoch: u64,
    content: ContentRef,
    chunk_bytes: u32,
    chunk_count: u64,
}

impl TryFrom<ContentTransferDeclarationWire> for ContentTransferDeclaration {
    type Error = String;

    fn try_from(value: ContentTransferDeclarationWire) -> Result<Self, Self::Error> {
        let transfer_id_bytes = value.transfer_id.len();
        if transfer_id_bytes == 0 || transfer_id_bytes > PHONE_CONTENT_MAX_TRANSFER_ID_BYTES {
            return Err("transfer_id must contain 1..=255 UTF-8 bytes".into());
        }
        if value.chunk_bytes == 0 || value.chunk_bytes > PHONE_CONTENT_DEFAULT_CHUNK_BYTES {
            return Err("chunk_bytes must be in 1..=262144".into());
        }
        let expected_chunks = if value.content.size_bytes == 0 {
            0
        } else {
            1 + ((value.content.size_bytes - 1) / u64::from(value.chunk_bytes))
        };
        if value.chunk_count != expected_chunks {
            return Err("chunk_count does not match content size and chunk_bytes".into());
        }
        if value.content.sha256.len() != 64
            || !value
                .content
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("sha256 must be 64 lowercase hexadecimal characters".into());
        }
        if let Some(content_device_id) = value.content.device_id.as_deref()
            && content_device_id != value.device_id
        {
            return Err("content device_id does not match transfer declaration".into());
        }
        if let Some(content_epoch) = value.content.link_epoch
            && content_epoch != value.link_epoch
        {
            return Err("content link_epoch does not match transfer declaration".into());
        }
        Ok(Self {
            transfer_id: value.transfer_id,
            device_id: value.device_id,
            link_epoch: value.link_epoch,
            content: value.content,
            chunk_bytes: value.chunk_bytes,
            chunk_count: value.chunk_count,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentChunkHeader {
    pub transfer_id: String,
    pub chunk_index: u64,
    pub offset: u64,
    pub length: u32,
    pub link_epoch: u64,
}

/// Encode one complete phone-control.v2 WebSocket binary chunk message.
pub fn encode_content_chunk(
    header: &ContentChunkHeader,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    let id = header.transfer_id.as_bytes();
    if id.is_empty() || id.len() > PHONE_CONTENT_MAX_TRANSFER_ID_BYTES {
        return Err("transfer_id must contain 1..=255 UTF-8 bytes".into());
    }
    if payload.len() > PHONE_CONTENT_DEFAULT_CHUNK_BYTES as usize {
        return Err("chunk payload exceeds 256 KiB".into());
    }
    if header.length as usize != payload.len() {
        return Err("chunk header length does not match payload".into());
    }
    let mut out =
        Vec::with_capacity(PHONE_CONTENT_CHUNK_FIXED_HEADER_BYTES + id.len() + payload.len());
    out.push(id.len() as u8);
    out.extend_from_slice(id);
    out.extend_from_slice(&header.link_epoch.to_be_bytes());
    out.extend_from_slice(&header.chunk_index.to_be_bytes());
    out.extend_from_slice(&header.offset.to_be_bytes());
    out.extend_from_slice(&header.length.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Decode one complete phone-control.v2 WebSocket binary chunk message.
pub fn decode_content_chunk(bytes: &[u8]) -> Result<(ContentChunkHeader, &[u8]), String> {
    let Some(&id_len) = bytes.first() else {
        return Err("binary chunk is missing transfer_id length".into());
    };
    let id_len = usize::from(id_len);
    if id_len == 0 {
        return Err("transfer_id must not be empty".into());
    }
    let header_len = PHONE_CONTENT_CHUNK_FIXED_HEADER_BYTES + id_len;
    if bytes.len() < header_len {
        return Err("binary chunk header is truncated".into());
    }
    let transfer_id = std::str::from_utf8(&bytes[1..1 + id_len])
        .map_err(|_| "transfer_id is not valid UTF-8")?
        .to_owned();
    let mut cursor = 1 + id_len;
    let take_u64 = |input: &[u8], cursor: &mut usize| {
        let value = u64::from_be_bytes(
            input[*cursor..*cursor + 8]
                .try_into()
                .expect("bounded header"),
        );
        *cursor += 8;
        value
    };
    let link_epoch = take_u64(bytes, &mut cursor);
    let chunk_index = take_u64(bytes, &mut cursor);
    let offset = take_u64(bytes, &mut cursor);
    let length = u32::from_be_bytes(
        bytes[cursor..cursor + 4]
            .try_into()
            .expect("bounded header"),
    );
    cursor += 4;
    let payload = &bytes[cursor..];
    if payload.len() != length as usize {
        return Err("binary chunk payload length mismatch".into());
    }
    if payload.len() > PHONE_CONTENT_DEFAULT_CHUNK_BYTES as usize {
        return Err("chunk payload exceeds 256 KiB".into());
    }
    Ok((
        ContentChunkHeader {
            transfer_id,
            chunk_index,
            offset,
            length,
            link_epoch,
        },
        payload,
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentTransferCommit {
    pub transfer_id: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub link_epoch: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_ref_contains_no_inline_bytes() {
        let value = serde_json::to_value(ContentRef {
            content_id: "content-1".into(),
            device_id: Some("device-1".into()),
            link_epoch: Some(7),
            mime_type: "image/png".into(),
            filename: Some("shot.png".into()),
            size_bytes: 42,
            sha256: "ab".repeat(32),
            source: ContentSource::Screenshot,
            expires_at_ms: Some(1234),
            persistence: ContentPersistence::Temporary,
        })
        .expect("content serializes");
        assert!(value.get("data").is_none());
        assert!(value.get("data_base64").is_none());
        assert_eq!(value["link_epoch"], 7);
    }

    #[test]
    fn transfer_declaration_rejects_conflicting_content_identity() {
        let base = serde_json::json!({
            "transfer_id": "transfer-1",
            "device_id": "device-1",
            "link_epoch": 7,
            "content": {
                "content_id": "content-1",
                "device_id": "device-2",
                "link_epoch": 6,
                "mime_type": "image/png",
                "size_bytes": 1,
                "sha256": "00",
                "source": "screenshot",
                "persistence": "temporary"
            },
            "chunk_bytes": 262144,
            "chunk_count": 1
        });
        assert!(serde_json::from_value::<ContentTransferDeclaration>(base).is_err());
    }

    #[test]
    fn binary_chunk_matches_cross_language_golden_layout() {
        let header = ContentChunkHeader {
            transfer_id: "t1".into(),
            chunk_index: 0,
            offset: 0,
            length: 3,
            link_epoch: 1,
        };
        let encoded = encode_content_chunk(&header, &[1, 2, 3]).expect("encodes");
        let encoded_hex = encoded
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            encoded_hex,
            "02743100000000000000010000000000000000000000000000000000000003010203"
        );
        let (decoded, payload) = decode_content_chunk(&encoded).expect("decodes");
        assert_eq!(decoded, header);
        assert_eq!(payload, [1, 2, 3]);
    }

    #[test]
    fn binary_chunk_rejects_empty_id_truncation_and_length_mismatch() {
        assert!(decode_content_chunk(&[]).is_err());
        assert!(decode_content_chunk(&[0]).is_err());
        assert!(decode_content_chunk(&[2, b't']).is_err());
        let mut valid = encode_content_chunk(
            &ContentChunkHeader {
                transfer_id: "t1".into(),
                chunk_index: 0,
                offset: 0,
                length: 1,
                link_epoch: 1,
            },
            &[1],
        )
        .unwrap();
        valid.push(2);
        assert!(decode_content_chunk(&valid).is_err());
    }
}
