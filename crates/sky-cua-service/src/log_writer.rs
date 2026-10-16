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
//!
//! Hard bound: rotation can fail exactly when output is most runaway (an
//! `EMFILE` accept loop cannot open the fresh log). No log file ever takes
//! more than [`DAEMON_LOG_HARD_CAP_BYTES`], counting stderr-fallback bytes when
//! fd 2 points at that same file; a stderr target the writer does not own gets
//! [`STDERR_FALLBACK_BUDGET_BYTES`]. Events past a bound are dropped and
//! counted, and the count is reported in the next fresh log. A failed rotation
//! is retried at most once per [`ROTATE_RETRY_INTERVAL`] rather than per event.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sky_cua_platform::log_rotation::{DAEMON_LOG_ROTATE_BYTES, guarded_rotate_oversized};
use tracing_subscriber::fmt::MakeWriter;

/// Absolute per-generation ceiling for a log file, reached only when rotation
/// keeps failing.
const HARD_CAP_ROTATIONS: u64 = 4;
const DAEMON_LOG_HARD_CAP_BYTES: u64 = HARD_CAP_ROTATIONS * DAEMON_LOG_ROTATE_BYTES;

/// Ceiling for the stderr fallback between successful opens. In production fd 2
/// is a log file too (the client's spawn log or the last adopted handle), so
/// the fallback must be bounded like any other generation.
const STDERR_FALLBACK_BUDGET_BYTES: u64 = DAEMON_LOG_ROTATE_BYTES;

/// Minimum spacing between rotation attempts once one has failed, so a
/// persistent failure costs a lock/stat/open per interval, not per event.
const ROTATE_RETRY_INTERVAL: Duration = Duration::from_secs(1);

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
    hard_cap: u64,
    /// Bytes known to be on fd 2's target since the last adopted handle.
    stderr_bytes: u64,
    stderr_budget: u64,
    /// fd 2 was dup2'd onto the last adopted handle, so stderr-fallback bytes
    /// land in that file and share its hard cap.
    stderr_shares_file: bool,
    /// Earliest time the next rotation may be attempted after a failed one.
    rotate_retry_at: Option<Instant>,
    /// Events dropped at a hard bound since the last adopted handle.
    dropped_events: u64,
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
        Self::with_limits(
            path,
            DAEMON_LOG_ROTATE_BYTES,
            DAEMON_LOG_HARD_CAP_BYTES,
            STDERR_FALLBACK_BUDGET_BYTES,
            true,
        )
    }

    #[cfg(test)]
    fn with_cap(path: PathBuf, cap: u64) -> Self {
        Self::with_limits(path, cap, cap * HARD_CAP_ROTATIONS, cap, false)
    }

    fn with_limits(
        path: PathBuf,
        cap: u64,
        hard_cap: u64,
        stderr_budget: u64,
        _redirect_stderr: bool,
    ) -> Self {
        let old_path = old_path_for(&path);
        let mut inner = Inner {
            path,
            old_path,
            file: None,
            bytes: 0,
            cap,
            hard_cap,
            stderr_bytes: 0,
            stderr_budget,
            stderr_shares_file: false,
            rotate_retry_at: None,
            dropped_events: 0,
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
    /// On failure `file` stays `None` (writes fall back to stderr).
    fn open_rotating(&mut self) {
        // Reuse the runtime rotation path for the initial oversized case so the
        // open-time and steady-state guards are identical.
        if self.try_reopen() && self.bytes > self.cap {
            let _ = self.rotate();
        }
    }

    /// Rotate `<path>` to `<path>.old` and reopen a fresh log. The rename is
    /// guarded by the shared cross-process protocol in
    /// `sky_cua_platform::log_rotation` so the concurrent client-side spawn
    /// rotation cannot clobber a freshly rotated generation.
    ///
    /// Best-effort: any failure leaves the current handle in place (we keep
    /// appending to the oversized file rather than losing output). Returns
    /// whether a fresh, under-cap log is now adopted.
    fn rotate(&mut self) -> bool {
        let Some(file) = self.file.as_ref() else {
            return false;
        };
        // A contended lock means another rotator is mid-flight — leave our handle
        // alone and let whichever file wins keep receiving appends.
        if !guarded_rotate_oversized(file, &self.path, &self.old_path, self.cap) {
            return false;
        }
        // On success the old handle and its advisory lock are dropped. On
        // failure (e.g. EMFILE) keep appending to the rotated inode, bounded by
        // the hard cap, rather than degrading to stderr, which is the same inode
        // in production but uncapped. Every later event retries the reopen.
        if !self.try_reopen()
            && let Some(file) = self.file.as_ref()
        {
            let _ = file.unlock();
        }
        // A failed rename reopens the same oversized inode; that is not a
        // successful rotation.
        self.bytes <= self.cap
    }

    /// Open a fresh append handle at `path` and adopt it. On failure the
    /// current state (handle or degraded `None`) is left untouched.
    fn try_reopen(&mut self) -> bool {
        match open_append(&self.path) {
            Ok((file, len)) => {
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
            // Reset before any early exit so a persistently missing path stays
            // rate-limited instead of re-checking on every event.
            self.inode_write_count = 0;
            self.inode_last_check = now;

            use std::os::unix::fs::MetadataExt;
            let Ok(file_meta) = file.metadata() else {
                return;
            };
            let stale = match std::fs::metadata(&self.path) {
                Ok(path_meta) => {
                    file_meta.dev() != path_meta.dev() || file_meta.ino() != path_meta.ino()
                }
                Err(error) => error.kind() == io::ErrorKind::NotFound,
            };
            // Emit only after a successful reopen: fd 2 then points at the fresh
            // log, so the diagnostic cannot accumulate in a capped generation.
            if stale && self.try_reopen() {
                self.emit_stale_diagnostic();
            }
        }
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
        self.stderr_bytes = 0;
        // An under-cap handle is a fresh generation, so a pending retry is moot.
        // An oversized reopen (failed rename, or a reopen after write errors)
        // keeps the throttle so repeated reopens cannot defeat it.
        if len <= self.cap {
            self.rotate_retry_at = None;
        }
        #[cfg(unix)]
        {
            self.stderr_shares_file = self.redirect_stderr;
        }
        self.write_dropped_notice();
    }

    /// Report events dropped at a hard bound, if the notice fits under the
    /// hard cap; otherwise keep counting until a generation with room.
    fn write_dropped_notice(&mut self) {
        if self.dropped_events == 0 {
            return;
        }
        let notice = format!(
            "{{\"type\":\"sky_cua_log_events_dropped\",\"count\":{}}}\n",
            self.dropped_events
        );
        let len = notice.len() as u64;
        if self.bytes.saturating_add(len) > self.hard_cap {
            return;
        }
        if let Some(file) = self.file.as_mut()
            && file.write_all(notice.as_bytes()).is_ok()
        {
            self.bytes = self.bytes.saturating_add(len);
            self.dropped_events = 0;
        }
    }

    /// Send `buf` to the stderr fallback unless its budget is spent.
    fn write_stderr_fallback(&mut self, buf: &[u8]) {
        let limit = if self.stderr_shares_file {
            self.hard_cap
        } else {
            self.stderr_budget
        };
        if self.stderr_bytes >= limit {
            self.dropped_events = self.dropped_events.saturating_add(1);
            return;
        }
        self.stderr_bytes = self.stderr_bytes.saturating_add(buf.len() as u64);
        let _ = io::stderr().write_all(buf);
    }

    /// Append `buf` to the log, rotating first if the cached count is over the
    /// cap. Returns `false` if the bytes could not be written to the file (so
    /// the caller can fall back to stderr).
    fn write_event(&mut self, buf: &[u8]) -> bool {
        if self.file.is_none() {
            // Degraded (open or write previously failed): retry the open on
            // every event so a transient failure (ENOSPC, EIO, missing dir)
            // does not silence file logging for the daemon's lifetime.
            self.try_reopen();
        }
        if self.bytes > self.cap
            && self
                .rotate_retry_at
                .is_none_or(|retry_at| Instant::now() >= retry_at)
        {
            self.rotate_retry_at = (!self.rotate()).then(|| Instant::now() + ROTATE_RETRY_INTERVAL);
        }
        self.check_stale_inode();
        let Some(file) = self.file.as_mut() else {
            return false;
        };
        if self.bytes >= self.hard_cap {
            // Dropping is a deliberate outcome, not a failure: report it as
            // handled so the caller does not spill into the stderr fallback.
            self.dropped_events = self.dropped_events.saturating_add(1);
            return true;
        }
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
                // drop it and let the next event try to reopen. When fd 2 is
                // the same file, the fallback inherits its byte count.
                if self.stderr_shares_file {
                    self.stderr_bytes = self.bytes;
                }
                self.file = None;
                self.bytes = 0;
                false
            }
        }
    }
}

fn open_append(path: &Path) -> io::Result<(File, u64)> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    Ok((file, len))
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
            // Degrade to stderr when the file is unavailable, within its budget.
            // A stderr error is ignored — logging must never fail the hot path.
            inner.write_stderr_fallback(buf);
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
mod tests;
