//! Regression test for inherited file-descriptor hygiene.
//!
//! A launcher holding a listening socket without `FD_CLOEXEC` (observed in
//! the field: an Electron DevTools port) must not leak that socket into the
//! long-lived service daemon, or the port stays bound after the launcher
//! exits.

#![cfg(target_os = "linux")]

use std::net::TcpListener;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use sky_cua_platform::SERVICE_SOCKET_PATH_ENV;

fn clear_cloexec(fd: i32) {
    // SAFETY: plain fcntl flag manipulation on a descriptor we own.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        assert!(flags >= 0, "F_GETFD failed");
        assert!(
            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) >= 0,
            "F_SETFD failed"
        );
    }
}

fn socket_inode(fd: i32) -> u64 {
    // SAFETY: fstat into a zeroed buffer for a descriptor we own.
    unsafe {
        let mut stat: libc::stat = std::mem::zeroed();
        assert_eq!(libc::fstat(fd, &mut stat), 0, "fstat failed");
        stat.st_ino
    }
}

fn child_holds_socket(pid: u32, inode: u64) -> bool {
    let needle = format!("socket:[{inode}]");
    let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        std::fs::read_link(entry.path())
            .is_ok_and(|target| target.to_string_lossy() == needle.as_str())
    })
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn leak_probe_detects_an_inherited_listener() {
    // Negative control: a child that performs no fd hygiene must be seen
    // holding the leaked socket, proving the /proc probe works.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind leak listener");
    let fd = listener.as_raw_fd();
    clear_cloexec(fd);
    let inode = socket_inode(fd);

    let child = Command::new("sleep")
        .arg("10")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sleep");
    let child = KillOnDrop(child);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = false;
    while Instant::now() < deadline {
        if child_holds_socket(child.0.id(), inode) {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(seen, "leak probe failed to observe an inherited socket");
}

#[test]
fn daemon_does_not_retain_inherited_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind leak listener");
    let fd = listener.as_raw_fd();
    clear_cloexec(fd);
    let inode = socket_inode(fd);

    let socket_path: PathBuf = std::env::temp_dir().join(format!(
        "sky-cua-fd-hygiene-{}-{}.sock",
        std::process::id(),
        inode
    ));
    let _ = std::fs::remove_file(&socket_path);

    let child = Command::new(env!("CARGO_BIN_EXE_sky-cua-service"))
        .arg("daemon")
        .env(SERVICE_SOCKET_PATH_ENV, &socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn service daemon");
    let mut child = KillOnDrop(child);
    let pid = child.0.id();

    // close_inherited_fds runs first thing in main, but allow for exec and
    // startup scheduling before asserting.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut released = false;
    while Instant::now() < deadline {
        if let Some(status) = child.0.try_wait().expect("try_wait") {
            panic!("service daemon exited early during fd hygiene test: {status}");
        }
        if !child_holds_socket(pid, inode) {
            released = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = std::fs::remove_file(&socket_path);
    assert!(
        released,
        "service daemon still holds the launcher's listening socket (inode {inode})"
    );
}
