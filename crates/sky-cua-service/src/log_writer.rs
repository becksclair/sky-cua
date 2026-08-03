//! Size-capped, self-rotating log writer for the daemon's tracing output.
//!
//! The daemon is a long-lived singleton. Routing its tracing output straight at
//! the inherited stderr fd (whose file the client opened and rotated only at
//! spawn) let a runaway warn/error loop append without any size bound until the
//! next respawn. This writer moves the size cap into the daemon: it appends to
//! the same per-endpoint log path the client hands it via `DAEMON_LOG_PATH_ENV`
//! and rotates `<path>` to `<path>.old` once it crosses
//! [`DAEMON_LOG_ROTATE_BYTES`], keeping exactly one rotated generation. The
//! rename runs the shared guarded protocol in
//! `sky_cua_platform::log_rotation`, the same one the client-side spawn
//! rotation uses, so the two rotators cannot clobber each other's fresh log.
//! On unix each fresh handle is also dup2'd onto fd 2 so panic output and the
//! stderr fallback follow the live log across rotations.
//!
//! Hot path: each tracing event takes one mutex lock and compares a cached
//! running byte count against the cap — no `stat` syscall per write. Only when
//! the cached count crosses the cap does the slow path (advisory lock, re-check,
//! rename, reopen) run. The writer never panics and never blocks startup: a
//! failed open, rotation, or write degrades to writing the event to stderr (or,
//! for a poisoned lock, recovers the guard) rather than propagating.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sky_cua_platform::log_rotation::{DAEMON_LOG_ROTATE_BYTES, guarded_rotate_oversized};
use tracing_subscriber::fmt::MakeWriter;

/// Check the log inode every N writes (or wallclock seconds) so the daemon
/// reopens its append handle after an external rename rather than silently
/// appending to the now-unlinked inode.
#[cfg(unix)]
const ROTATING_LOG_INODE_CHECK_INTERVAL: u64 = 1024;
#[cfg(unix)]
const ROTATING_LOG_INODE_CHECK_WALLCLOCK: Duration = Duration::from_secs(10);

/// A `MakeWriter` that appends tracing output to a fixed log path and rotates it
/// in place once it exceeds the cap. Cheap to clone (shares one locked handle).
#[derive(Clone)]
pub(crate) struct RotatingLog {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    path: PathBuf,
    old_path: PathBuf,
    /// The current append handle, or `None` if it could not be opened — in which
    /// case writes degrade to stderr and each subsequent event retries the open.
    file: Option<File>,
    /// Cached running byte count of `file`, kept so the hot path never stats.
    bytes: u64,
    cap: u64,
    /// Write count since the last inode staleness check.
    #[cfg(unix)]
    inode_write_count: u64,
    /// Wall clock of the last inode staleness check.
    #[cfg(unix)]
    inode_last_check: Instant,
    /// Re-point fd 2 at every fresh log handle so panic output and the stderr
    /// fallback follow the live log across rotations (the inherited stderr fd
    /// stays bound to the pre-rotation inode otherwise). Production-only: the
    /// daemon enables it; tests must not hijack the test harness's stderr.
    /// Unix-only — on Windows panic capture ends at the first runtime rotation,
    /// and this state is structurally absent.
    #[cfg(unix)]
    redirect_stderr: bool,
}

impl RotatingLog {
    /// Open (creating and rotating if already oversized) the log at `path`.
    ///
    /// Never fails: if the file cannot be opened the writer degrades to stderr,
    /// so tracing setup and daemon startup are never blocked by a logging error.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self::with_cap_and_redirect(path, DAEMON_LOG_ROTATE_BYTES, true)
    }

    #[cfg(test)]
    fn with_cap(path: PathBuf, cap: u64) -> Self {
        Self::with_cap_and_redirect(path, cap, false)
    }

    fn with_cap_and_redirect(path: PathBuf, cap: u64, _redirect_stderr: bool) -> Self {
        let old_path = old_path_for(&path);
        let mut inner = Inner {
            path,
            old_path,
            file: None,
            bytes: 0,
            cap,
            #[cfg(unix)]
            inode_write_count: 0,
            #[cfg(unix)]
            inode_last_check: Instant::now(),
            #[cfg(unix)]
            redirect_stderr: _redirect_stderr,
        };
        inner.open_rotating();
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl Inner {
    /// Open the log for append, rotating first if it is already over the cap.
    /// On any failure `file` is left `None` (writes fall back to stderr).
    fn open_rotating(&mut self) {
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
                self.adopt(file, len);
                if len > self.cap {
                    // Reuse the runtime rotation path for the initial oversized
                    // case so the open-time and steady-state guards are identical.
                    self.rotate();
                }
            }
            Err(_) => {
                self.file = None;
                self.bytes = 0;
            }
        }
    }

    /// Rotate `<path>` to `<path>.old` and reopen a fresh log. The rename is
    /// guarded by the shared cross-process protocol in
    /// `sky_cua_platform::log_rotation` so the concurrent client-side spawn
    /// rotation cannot clobber a freshly rotated generation.
    ///
    /// Best-effort: any failure leaves the current handle in place (we keep
    /// appending to the oversized file rather than losing output).
    fn rotate(&mut self) {
        let Some(file) = self.file.as_ref() else {
            // No handle to rotate; try to (re)establish one.
            self.reopen_after_rotate();
            return;
        };

        // A contended lock means another rotator is mid-flight — leave our handle
        // alone and let whichever file wins keep receiving appends.
        if !guarded_rotate_oversized(file, &self.path, &self.old_path, self.cap) {
            return;
        }
        // Dropping/replacing the handle below releases the advisory lock with it.
        self.reopen_after_rotate();
    }

    /// Reopen a fresh append handle at `path`, resetting the cached byte count.
    /// On failure degrade to stderr (`file = None`).
    fn reopen_after_rotate(&mut self) {
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
                self.adopt(file, len);
            }
            Err(_) => {
                self.file = None;
                self.bytes = 0;
            }
        }
    }

    /// Reopen the log fd when the inode has been replaced externally. Unlike
    /// the rotation reopen, on failure we keep the old fd so writes continue
    /// landing somewhere (even if the path has moved) rather than falling back
    /// to stderr. Returns whether a fresh handle was obtained.
    #[cfg(unix)]
    fn reopen_if_stale(&mut self) -> bool {
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
                self.adopt(file, len);
                true
            }
            Err(_) => false,
        }
    }

    /// Check whether the open file handle still points at the same file as the
    /// path. If an external rename replaced the on-disk file (while our handle
    /// still appends to the old inode), reopen a new handle to the fresh file.
    ///
    /// The check fires at most once every N writes OR every 10s of wall clock,
    /// whichever comes first, so the hot path adds at most a counter compare and
    /// a saturating duration check — no syscall — between most events.
    ///
    /// On Unix the comparison uses (dev, ino). On non-Unix this is a no-op.
    fn check_stale_inode(&mut self) {
        #[cfg(unix)]
        {
            let Some(file) = self.file.as_ref() else {
                return;
            };

            let now = Instant::now();
            let since_check = now.saturating_duration_since(self.inode_last_check);
            if self.inode_write_count < ROTATING_LOG_INODE_CHECK_INTERVAL
                && since_check < ROTATING_LOG_INODE_CHECK_WALLCLOCK
            {
                return;
            }

            use std::os::unix::fs::MetadataExt;
            let file_meta = match file.metadata() {
                Ok(m) => m,
                Err(_) => return,
            };
            let path_meta = match std::fs::metadata(&self.path) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    self.emit_stale_diagnostic();
                    self.reopen_if_stale();
                    return;
                }
                Err(_) => return,
            };

            if file_meta.dev() != path_meta.dev() || file_meta.ino() != path_meta.ino() {
                self.emit_stale_diagnostic();
                self.reopen_if_stale();
            }

            self.inode_write_count = 0;
            self.inode_last_check = now;
        }
        #[cfg(not(unix))]
        {}
    }

    #[cfg(unix)]
    fn emit_stale_diagnostic(&self) {
        let path_str = self.path.to_string_lossy();
        let escaped = serde_json::to_string(&path_str.as_ref())
            .unwrap_or_else(|_| format!("\"{}\"", path_str));
        eprintln!(
            "{{\"type\":\"sky_cua_log_reopened_after_external_rename\",\"path\":{}}}",
            escaped
        );
    }

    /// Install a fresh handle (and byte count), re-pointing fd 2 at it when
    /// stderr redirection is enabled so panics keep landing in the live log.
    fn adopt(&mut self, file: File, len: u64) {
        #[cfg(unix)]
        if self.redirect_stderr {
            use std::os::unix::io::AsRawFd;
            // SAFETY: dup2 onto fd 2 replaces the process's stderr with a valid
            // open descriptor; both fds remain owned and are not closed here.
            unsafe {
                let _ = libc::dup2(file.as_raw_fd(), 2);
            }
        }
        self.bytes = len;
        self.file = Some(file);
    }

    /// Append `buf` to the log, rotating first if the cached count is over the
    /// cap. Returns `false` if the bytes could not be written to the file (so
    /// the caller can fall back to stderr).
    fn write_event(&mut self, buf: &[u8]) -> bool {
        if self.file.is_none() {
            // Degraded (open or write previously failed): retry the open on
            // every event so a transient failure (ENOSPC, EIO, missing dir)
            // does not silence file logging for the daemon's lifetime.
            self.reopen_after_rotate();
        }
        if self.bytes > self.cap {
            self.rotate();
        }
        self.check_stale_inode();
        let Some(file) = self.file.as_mut() else {
            return false;
        };
        match file.write_all(buf) {
            Ok(()) => {
                self.bytes = self.bytes.saturating_add(buf.len() as u64);
                #[cfg(unix)]
                {
                    self.inode_write_count = self.inode_write_count.saturating_add(1);
                }
                true
            }
            Err(_) => {
                // The handle went bad (e.g. the underlying inode was removed);
                // drop it and let the next event try to reopen.
                self.file = None;
                self.bytes = 0;
                false
            }
        }
    }
}

fn old_path_for(path: &Path) -> PathBuf {
    let mut old = path.as_os_str().to_owned();
    old.push(".old");
    PathBuf::from(old)
}

/// The per-event writer handed to the fmt layer. Holds a clone of the shared
/// state; each write locks once.
pub(crate) struct RotatingHandle {
    inner: Arc<Mutex<Inner>>,
}

impl Write for RotatingHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !inner.write_event(buf) {
            // Degrade to stderr so nothing is silently dropped when the file is
            // unavailable. Ignore a stderr error too — logging must never fail
            // the daemon's hot path.
            let _ = io::stderr().write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(file) = inner.file.as_mut() {
            let _ = file.flush();
        }
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RotatingLog {
    type Writer = RotatingHandle;

    fn make_writer(&'a self) -> Self::Writer {
        RotatingHandle {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-service-log-writer-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(log: &RotatingLog, bytes: &[u8]) {
        let mut handle = log.make_writer();
        handle.write_all(bytes).unwrap();
    }

    #[test]
    fn appends_to_existing_log_without_rotation() {
        let dir = temp_dir("append");
        let path = dir.join("daemon-service.log");
        std::fs::write(&path, b"first\n").unwrap();

        let log = RotatingLog::with_cap(path.clone(), 1024);
        write(&log, b"second\n");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        assert!(!old_path_for(&path).exists());
    }

    #[test]
    fn rotates_once_the_cap_is_exceeded_and_starts_fresh() {
        let dir = temp_dir("rotate");
        let path = dir.join("daemon-service.log");
        let cap = 64;

        let log = RotatingLog::with_cap(path.clone(), cap);
        // Cross the cap; the next event triggers rotation.
        write(&log, &vec![b'x'; (cap + 1) as usize]);
        write(&log, b"post-rotation\n");

        let rotated = std::fs::read(old_path_for(&path)).unwrap();
        assert_eq!(rotated.len(), (cap + 1) as usize);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "post-rotation\n");
    }

    #[test]
    fn oversized_existing_log_rotates_at_open() {
        let dir = temp_dir("open-rotate");
        let path = dir.join("daemon-service.log");
        let cap = 32;
        let seed = vec![b'y'; (cap + 1) as usize];
        std::fs::write(&path, &seed).unwrap();

        let log = RotatingLog::with_cap(path.clone(), cap);
        write(&log, b"fresh\n");

        let rotated = std::fs::read(old_path_for(&path)).unwrap();
        assert_eq!(rotated.len(), seed.len());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
    }

    #[test]
    fn rotation_replaces_a_previous_old_generation() {
        let dir = temp_dir("regen");
        let path = dir.join("daemon-service.log");
        let cap = 16;
        std::fs::write(old_path_for(&path), b"ancient\n").unwrap();

        let log = RotatingLog::with_cap(path.clone(), cap);
        write(&log, &vec![b'z'; (cap + 1) as usize]);
        write(&log, b"new\n");

        // The oversized current generation replaced the ancient .old.
        let rotated = std::fs::read(old_path_for(&path)).unwrap();
        assert_eq!(rotated.len(), (cap + 1) as usize);
    }

    #[test]
    fn degraded_writer_recovers_when_the_path_becomes_writable() {
        // A transient open/write failure must not silence file logging for the
        // daemon's lifetime: each degraded event retries the open.
        let dir = temp_dir("recover");
        let path = dir.join("missing-parent").join("daemon-service.log");

        let log = RotatingLog::with_cap(path.clone(), 1024);
        write(&log, b"lost-to-stderr\n");
        assert!(!path.exists());

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write(&log, b"recovered\n");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "recovered\n");
    }

    #[test]
    fn unwritable_path_degrades_without_panicking() {
        // A path whose parent does not exist cannot be opened; the writer must
        // degrade to stderr rather than panic or block.
        let dir = temp_dir("degrade");
        let path = dir.join("missing-parent").join("daemon-service.log");

        let log = RotatingLog::with_cap(path.clone(), 64);
        // Must not panic.
        write(&log, b"event\n");

        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn rotating_log_reopens_after_external_rename() {
        let dir = temp_dir("rename-reopen");
        let path = dir.join("daemon-service.log");
        let renamed = dir.join("daemon-service.log.renamed");

        let log = RotatingLog::with_cap(path.clone(), 65536);

        // Write 1024 times. Each write checks `inode_write_count < 1024` before
        // incrementing, so the check never fires during the loop — at the end
        // `inode_write_count` is 1024 but the threshold never triggered. Push
        // the counter to 1023 and age the wall-clock so the next write fires
        // the wall-clock check.
        for _ in 0..1024 {
            write(&log, b"x\n");
        }
        {
            let mut guard = log.inner.lock().unwrap();
            guard.inode_write_count = 1023;
            guard.inode_last_check = Instant::now() - Duration::from_secs(11);
        }

        // Verify the inode check correctly detects the stale handle
        // and reopens.
        std::fs::rename(&path, &renamed).unwrap();

        // This write triggers the inode check, detects the stale inode,
        // and reopens a fresh handle at the original path.
        write(&log, b"post-rename\n");

        let old_data = std::fs::read_to_string(&renamed).unwrap();
        assert_eq!(old_data.lines().filter(|l| *l == "x").count(), 1024);

        let new_data = std::fs::read_to_string(&path).unwrap();
        assert_eq!(new_data, "post-rename\n");
    }

    #[cfg(unix)]
    #[test]
    fn rotating_log_reopens_at_low_event_rate() {
        let dir = temp_dir("low-rate-reopen");
        let path = dir.join("daemon-service.log");
        let renamed = dir.join("daemon-service.log.renamed");

        let log = RotatingLog::with_cap(path.clone(), 65536);
        write(&log, b"first\n");

        // Push the last-check timestamp far enough back that the wall-clock
        // branch triggers on the next write regardless of the write count.
        {
            let mut guard = log.inner.lock().unwrap();
            guard.inode_last_check = Instant::now() - Duration::from_secs(11);
        }

        std::fs::rename(&path, &renamed).unwrap();
        write(&log, b"post-rename\n");

        let old_data = std::fs::read_to_string(&renamed).unwrap();
        assert_eq!(old_data, "first\n");

        let new_data = std::fs::read_to_string(&path).unwrap();
        assert_eq!(new_data, "post-rename\n");
    }

    #[cfg(unix)]
    #[test]
    fn rotating_log_revalidates_before_first_post_quiet_write() {
        let dir = temp_dir("pre-quiet");
        let path = dir.join("daemon-service.log");
        let renamed = dir.join("daemon-service.log.renamed");

        let log = RotatingLog::with_cap(path.clone(), 65536);
        write(&log, b"before\n");

        // Age last-check so the wall-clock branch fires on the next write.
        {
            let mut guard = log.inner.lock().unwrap();
            guard.inode_last_check = Instant::now() - Duration::from_secs(11);
        }

        std::fs::rename(&path, &renamed).unwrap();
        write(&log, b"after\n");

        // The "after" write should have landed in a fresh file at the original
        // path, proving the check ran before the append.
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, "after\n");
        assert!(
            std::fs::read_to_string(&renamed)
                .unwrap()
                .contains("before")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rotating_log_handles_path_temporarily_missing() {
        let dir = temp_dir("missing-path");
        let path = dir.join("daemon-service.log");

        let log = RotatingLog::with_cap(path.clone(), 65536);
        write(&log, b"existing\n");
        assert!(path.exists());

        // Remove the file entirely (no rename — the path disappears).
        std::fs::remove_file(&path).unwrap();

        // Force an inode check on the next write.
        {
            let mut guard = log.inner.lock().unwrap();
            guard.inode_write_count = 1023;
            guard.inode_last_check = Instant::now() - Duration::from_secs(11);
        }

        // The check sees NotFound → reopens a fresh file at the path.
        write(&log, b"recreated\n");

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "recreated\n");
    }
}
