//! Cross-process guarded rotation for the daemon log.
//!
//! Two independent rotators act on the same `<stem>.log`: the client rotates
//! at daemon spawn (`sky-cua-client`'s `daemon_log`) and the daemon rotates at
//! runtime (`sky-cua-service`'s `log_writer`). Their safety against each other
//! is a protocol, not a convention: if either side renamed without the lock +
//! re-check sequence below, a loser could rename the winner's fresh log over
//! the just-rotated `.old` generation and destroy it. Both sides call this one
//! function so the protocol cannot drift.

use std::fs::File;
use std::path::Path;

/// Rotate the log once it grows past this size; one rotated generation
/// (`.old`) is kept. Shared so both rotators enforce the same cap.
pub const DAEMON_LOG_ROTATE_BYTES: u64 = 8 * 1024 * 1024;

/// Attempt the guarded `<path>` → `<old_path>` rotation for an oversized log.
///
/// `file` must be an open handle to the inode currently (or recently) at
/// `path`. Takes an advisory lock on that inode; a contended lock means the
/// other rotator is mid-flight, so nothing is renamed and `false` is returned
/// (the caller keeps appending to whichever file wins). With the lock held,
/// the size is re-checked *by path*: a rotator that finished before the lock
/// was acquired has already swapped the path to a fresh file, and renaming
/// again would destroy the generation it just rotated. Returns `true` when the
/// caller held the lock (whether or not a rename happened) and should reopen
/// `path` for a fresh handle; the lock releases when `file` is dropped.
pub fn guarded_rotate_oversized(file: &File, path: &Path, old_path: &Path, cap: u64) -> bool {
    if file.try_lock().is_err() {
        return false;
    }
    let still_oversized = std::fs::metadata(path)
        .map(|metadata| metadata.len() > cap)
        .unwrap_or(false);
    if still_oversized {
        let _ = std::fs::rename(path, old_path);
    }
    true
}
