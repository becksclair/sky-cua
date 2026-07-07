//! Destination for the spawned daemon's stderr.
//!
//! The daemon's stderr carries its tracing output — the only runtime record
//! the service produces — so it is appended to a per-endpoint log in the
//! sky-cua state dir instead of being discarded. Logging setup must never
//! block a daemon launch: any failure falls back to discarding stderr.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use sky_cua_platform::log_rotation::{DAEMON_LOG_ROTATE_BYTES, guarded_rotate_oversized};
use sky_cua_platform::sky_cua_state_dir;

/// Stderr destination for a daemon spawn. `stem` names the per-endpoint log
/// file (`<stem>.log`); distinct daemons (default vs. isolated-desktop socket)
/// must use distinct stems so their logs do not interleave.
pub(crate) fn daemon_stderr_destination(stem: &str) -> Stdio {
    match open_daemon_log(stem) {
        Ok(file) => Stdio::from(file),
        Err(_) => Stdio::null(),
    }
}

fn open_daemon_log(stem: &str) -> std::io::Result<File> {
    let dir = sky_cua_state_dir()?;
    std::fs::create_dir_all(&dir)?;
    open_rotating_log(&dir, stem)
}

/// Absolute path of the log file the daemon spawned for `stem` writes to
/// (`<state-dir>/<stem>.log`). Handed to the daemon via `DAEMON_LOG_PATH_ENV` so
/// it can self-rotate at runtime. `None` when the per-user state dir cannot be
/// resolved, in which case the daemon falls back to plain stderr.
pub(crate) fn daemon_log_path(stem: &str) -> Option<PathBuf> {
    let dir = sky_cua_state_dir().ok()?;
    Some(dir.join(format!("{stem}.log")))
}

fn open_rotating_log(dir: &Path, stem: &str) -> std::io::Result<File> {
    let path = dir.join(format!("{stem}.log"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    if file.metadata()?.len() <= DAEMON_LOG_ROTATE_BYTES {
        return Ok(file);
    }
    rotate_oversized_log(dir, stem, file)
}

/// Rotate `<stem>.log` to `<stem>.log.old` and reopen a fresh log. `file` is
/// an already-open handle to the oversized inode currently (or recently) at
/// the log path.
fn rotate_oversized_log(dir: &Path, stem: &str, file: File) -> std::io::Result<File> {
    let path = dir.join(format!("{stem}.log"));
    // The rename is guarded by the shared cross-process protocol in
    // `sky_cua_platform::log_rotation` — the daemon's runtime rotator uses the
    // same sequence, so neither side can clobber the other's fresh log. A
    // contended lock means the other rotator is mid-flight; appending to
    // whichever file wins is fine.
    if !guarded_rotate_oversized(
        &file,
        &path,
        &dir.join(format!("{stem}.log.old")),
        DAEMON_LOG_ROTATE_BYTES,
    ) {
        return Ok(file);
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sky-cua-daemon-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn small_log_appends_without_rotation() {
        let dir = temp_dir("append");
        std::fs::write(dir.join("daemon-service.log"), b"first\n").unwrap();

        let mut file = open_rotating_log(&dir, "daemon-service").unwrap();
        file.write_all(b"second\n").unwrap();

        let content = std::fs::read_to_string(dir.join("daemon-service.log")).unwrap();
        assert_eq!(content, "first\nsecond\n");
        assert!(!dir.join("daemon-service.log.old").exists());
    }

    #[test]
    fn oversized_log_rotates_to_old_generation_and_starts_fresh() {
        let dir = temp_dir("rotate");
        let seed = vec![b'x'; (DAEMON_LOG_ROTATE_BYTES + 1) as usize];
        std::fs::write(dir.join("daemon-service.log"), &seed).unwrap();

        let mut file = open_rotating_log(&dir, "daemon-service").unwrap();
        file.write_all(b"fresh\n").unwrap();

        let rotated = std::fs::metadata(dir.join("daemon-service.log.old")).unwrap();
        assert_eq!(rotated.len(), seed.len() as u64);
        let content = std::fs::read_to_string(dir.join("daemon-service.log")).unwrap();
        assert_eq!(content, "fresh\n");
    }

    #[test]
    fn contended_lock_skips_rotation_and_appends() {
        let dir = temp_dir("contended");
        let seed = vec![b'z'; (DAEMON_LOG_ROTATE_BYTES + 1) as usize];
        let path = dir.join("daemon-service.log");
        std::fs::write(&path, &seed).unwrap();

        // flock-style locks are per open file description, so an independent
        // handle in the same process contends like a concurrent spawner would.
        let holder = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        holder.lock().unwrap();

        let file = open_rotating_log(&dir, "daemon-service").unwrap();
        drop(file);

        assert!(
            !dir.join("daemon-service.log.old").exists(),
            "a contended lock must skip rotation"
        );
        assert_eq!(std::fs::metadata(&path).unwrap().len(), seed.len() as u64);
    }

    #[test]
    fn re_check_after_lock_skips_rename_when_path_was_already_rotated() {
        let dir = temp_dir("recheck");
        let seed = vec![b'w'; (DAEMON_LOG_ROTATE_BYTES + 1) as usize];
        let path = dir.join("daemon-service.log");
        std::fs::write(&path, &seed).unwrap();
        // Handle to the oversized inode, as a spawner would hold pre-rotation.
        let stale = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();

        // A concurrent winner completes the rotation before our lock lands.
        std::fs::rename(&path, dir.join("daemon-service.log.old")).unwrap();
        std::fs::write(&path, b"fresh\n").unwrap();

        let _file = rotate_oversized_log(&dir, "daemon-service", stale).unwrap();

        let rotated = std::fs::metadata(dir.join("daemon-service.log.old")).unwrap();
        assert_eq!(
            rotated.len(),
            seed.len() as u64,
            "the re-check must not rename the fresh log over the rotated generation"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
    }

    #[test]
    fn rotation_replaces_previous_old_generation() {
        let dir = temp_dir("regen");
        std::fs::write(dir.join("daemon-service.log.old"), b"ancient\n").unwrap();
        let seed = vec![b'y'; (DAEMON_LOG_ROTATE_BYTES + 1) as usize];
        std::fs::write(dir.join("daemon-service.log"), &seed).unwrap();

        let _file = open_rotating_log(&dir, "daemon-service").unwrap();

        let rotated = std::fs::metadata(dir.join("daemon-service.log.old")).unwrap();
        assert_eq!(rotated.len(), seed.len() as u64);
    }
}
