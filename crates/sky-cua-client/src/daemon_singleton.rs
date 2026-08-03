//! Shared helpers for locating and identifying the singleton `sky-cua-service`
//! daemon that owns a given IPC socket.
//!
//! Both the normal daemon launcher (`service_launcher`) and the isolated-desktop
//! teardown (`isolated_desktop`) need to find the daemon's pid from its socket
//! and verify a candidate pid really is a `sky-cua-service` process before
//! signalling it. Keeping the lock-file convention and the process-identity check
//! in one place means the two call sites cannot drift apart (the lock-file format
//! is written by `crates/sky-cua-service/src/ipc_server.rs`: `<socket>.lock`
//! holding the daemon's decimal pid).

#![cfg(unix)]

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The singleton lock path for a daemon socket: `<socket>.lock`.
pub(crate) fn socket_lock_path(socket_path: &Path) -> PathBuf {
    let mut lock_name = socket_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("service.sock"));
    lock_name.push(".lock");
    socket_path.with_file_name(lock_name)
}

/// The persistent client lifecycle lease path for a daemon socket.
pub(crate) fn lifecycle_lock_path(socket_path: &Path) -> PathBuf {
    let mut lock_name = socket_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("service.sock"));
    lock_name.push(".lifecycle.lock");
    socket_path.with_file_name(lock_name)
}

/// An endpoint-scoped, process-wide replacement/startup lease.
///
/// The file is deliberately persistent: unlinking a flock file can create two
/// independently locked inodes at the same path. Closing this handle releases
/// the lease.
pub(crate) struct LifecycleLease {
    _file: File,
}

impl LifecycleLease {
    /// Try to acquire the lifecycle lease without blocking in the kernel.
    pub(crate) fn try_acquire(socket_path: &Path) -> Result<Option<Self>> {
        let path = lifecycle_lock_path(socket_path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create lifecycle lease directory {}",
                    parent.display()
                )
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to open lifecycle lease {}", path.display()))?;
        loop {
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result == 0 {
                return Ok(Some(Self { _file: file }));
            }
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EINTR) => continue,
                Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => {
                    return Ok(None);
                }
                _ => {
                    return Err(error).with_context(|| {
                        format!("failed to acquire lifecycle lease {}", path.display())
                    });
                }
            }
        }
    }
}

/// Read the daemon pid recorded in the socket's singleton lock file, if present
/// and greater than 1 (init/invalid owners are rejected). Surfaces a parse error
/// only for a non-empty but malformed lock; a missing or empty lock yields
/// `Ok(None)`.
pub(crate) fn read_owner_pid(socket_path: &Path) -> Result<Option<u32>> {
    let lock_path = socket_lock_path(socket_path);
    let Ok(raw) = std::fs::read_to_string(&lock_path) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let pid = trimmed.parse::<u32>().with_context(|| {
        format!(
            "invalid sky-cua-service singleton owner pid in {}",
            lock_path.display()
        )
    })?;
    Ok((pid > 1).then_some(pid))
}

/// Whether `pid` is a live `sky-cua-service` process (by `/proc/<pid>/exe` or its
/// cmdline), so termination/teardown never signals an unrelated or recycled pid —
/// in particular never the user's real daemon, which lives on a different socket.
pub(crate) fn pid_is_sky_cua_service(pid: u32) -> bool {
    let proc_root = PathBuf::from(format!("/proc/{pid}"));
    let exe_name = std::fs::read_link(proc_root.join("exe"))
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_os_string()));
    let cmdline = std::fs::read(proc_root.join("cmdline")).ok();
    process_identity_looks_like_sky_cua_service(exe_name.as_deref(), cmdline.as_deref())
}

/// Pure identity check over an exe basename and cmdline bytes, split out so it is
/// unit-testable without a live `/proc`.
fn process_identity_looks_like_sky_cua_service(
    exe_name: Option<&OsStr>,
    cmdline: Option<&[u8]>,
) -> bool {
    exe_name.is_some_and(|name| name == "sky-cua-service")
        || cmdline.is_some_and(|bytes| {
            bytes
                .split(|byte| *byte == 0)
                .filter_map(|part| std::str::from_utf8(part).ok())
                .any(|part| {
                    Path::new(part)
                        .file_name()
                        .is_some_and(|name| name == "sky-cua-service")
                })
        })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn socket_lock_path_sits_next_to_socket() {
        assert_eq!(
            socket_lock_path(Path::new("/tmp/sky-cua/service.sock")),
            PathBuf::from("/tmp/sky-cua/service.sock.lock")
        );
        assert_eq!(
            socket_lock_path(Path::new(
                "/run/user/1000/sky-cua/service-isolated-100.sock"
            )),
            PathBuf::from("/run/user/1000/sky-cua/service-isolated-100.sock.lock")
        );
        assert_eq!(
            lifecycle_lock_path(Path::new("/tmp/sky-cua/service.sock")),
            PathBuf::from("/tmp/sky-cua/service.sock.lifecycle.lock")
        );
    }

    #[test]
    fn lifecycle_lease_excludes_competitors_and_is_reacquired_after_drop() {
        if let Some(socket_path) = std::env::var_os("SKY_CUA_LIFECYCLE_LEASE_HELPER_SOCKET") {
            let acquired = LifecycleLease::try_acquire(Path::new(&socket_path))
                .expect("helper lease attempt")
                .is_some();
            let expected = std::env::var_os("SKY_CUA_LIFECYCLE_LEASE_HELPER_EXPECT")
                .is_some_and(|value| value == "acquired");
            assert_eq!(acquired, expected);
            return;
        }

        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-lifecycle-lease-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let socket_path = temp_dir.join("missing-parent/service.sock");
        assert!(!socket_path.parent().expect("socket parent").exists());

        let lease = LifecycleLease::try_acquire(&socket_path)
            .expect("first lease attempt")
            .expect("first lease should be acquired");
        assert!(socket_path.parent().expect("socket parent").is_dir());
        run_lifecycle_lease_helper(&socket_path, "excluded");
        drop(lease);
        run_lifecycle_lease_helper(&socket_path, "acquired");
        assert!(lifecycle_lock_path(&socket_path).exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    fn run_lifecycle_lease_helper(socket_path: &Path, expected: &str) {
        let status = std::process::Command::new(std::env::current_exe().expect("current test exe"))
            .args([
                "--exact",
                "daemon_singleton::tests::lifecycle_lease_excludes_competitors_and_is_reacquired_after_drop",
            ])
            .env("SKY_CUA_LIFECYCLE_LEASE_HELPER_SOCKET", socket_path)
            .env("SKY_CUA_LIFECYCLE_LEASE_HELPER_EXPECT", expected)
            .status()
            .expect("run lifecycle lease helper process");
        assert!(status.success(), "lifecycle lease helper failed");
    }

    #[test]
    fn read_owner_pid_parses_valid_lock_and_rejects_init_or_garbage() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-daemon-singleton-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let socket_path = temp_dir.join("service.sock");

        fs::write(socket_lock_path(&socket_path), "4242\n").expect("write lock pid");
        assert_eq!(read_owner_pid(&socket_path).expect("parses"), Some(4242));

        // pid 1 / 0 are never returned (init / invalid owner).
        fs::write(socket_lock_path(&socket_path), "1").expect("write lock pid");
        assert_eq!(read_owner_pid(&socket_path).expect("parses"), None);

        // Missing lock yields Ok(None); a non-empty malformed lock is an error.
        let _ = fs::remove_file(socket_lock_path(&socket_path));
        assert_eq!(read_owner_pid(&socket_path).expect("parses"), None);
        fs::write(socket_lock_path(&socket_path), "not-a-pid").expect("write lock pid");
        assert!(read_owner_pid(&socket_path).is_err());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn process_identity_matches_service_binary_name() {
        assert!(process_identity_looks_like_sky_cua_service(
            Some(OsStr::new("sky-cua-service")),
            None,
        ));
        assert!(process_identity_looks_like_sky_cua_service(
            None,
            Some(b"/home/bex/.local/share/sky-cua/bin/sky-cua-service\0daemon\0"),
        ));
        assert!(!process_identity_looks_like_sky_cua_service(
            Some(OsStr::new("unrelated")),
            Some(b"/usr/bin/unrelated\0"),
        ));
    }
}
