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

use sky_cua_platform::log_rotation::{DAEMON_LOG_ROTATE_BYTES, guarded_rotate_oversized};
use tracing_subscriber::fmt::MakeWriter;

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
    /// Re-point fd 2 at every fresh log handle so panic output and the stderr
    /// fallback follow the live log across rotations (the inherited stderr fd
    /// stays bound to the pre-rotation inode otherwise). Production-only: the
    /// daemon enables it; tests must not hijack the test harness's stderr.
    /// Unix-only — on Windows panic capture ends at the first runtime rotation.
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

    fn with_cap_and_redirect(path: PathBuf, cap: u64, redirect_stderr: bool) -> Self {
        let old_path = old_path_for(&path);
        let mut inner = Inner {
            path,
            old_path,
            file: None,
            bytes: 0,
            cap,
            redirect_stderr,
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
        let Some(file) = self.file.as_mut() else {
            return false;
        };
        match file.write_all(buf) {
            Ok(()) => {
                self.bytes = self.bytes.saturating_add(buf.len() as u64);
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
}
