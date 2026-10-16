//! Inherited file-descriptor hygiene for long-lived sky-cua processes.
//!
//! The MCP client, service daemon, and overlay host are spawned by arbitrary
//! host process trees (Codex Desktop, codex app-server, shells, smoke
//! harnesses). Descriptors the launcher holds without `FD_CLOEXEC` are
//! inherited across `fork`/`exec` and then live as long as the daemon does.
//! Observed in the field: an Electron host's DevTools listening socket
//! (`--remote-debugging-port`) was inherited through
//! codex app-server -> sky-cua-client -> sky-cua-service, so the port stayed
//! bound after the host exited and every later host launch failed with
//! "Address already in use".
//!
//! Each long-lived binary calls [`close_inherited_fds`] as the first thing in
//! `main`, before any runtime (tokio epoll/eventfd), logging, or socket setup
//! opens descriptors of its own. Startup-side closing is deliberate: it also
//! protects the first sky-cua process in the chain, which spawn-side
//! `pre_exec` hygiene in our own spawners could never cover.

/// Close every inherited file descriptor above stdio (fd 3 and up).
///
/// Best effort: uses the `close_range(2)` syscall (Linux 5.9+). On kernels
/// without it, or on non-Linux platforms, this is a no-op and the previous
/// inherit-everything behavior remains.
///
/// Must be called before the process opens descriptors it needs (async
/// runtime, listeners, log files); stdio (0-2) is preserved.
pub fn close_inherited_fds() {
    #[cfg(target_os = "linux")]
    // SAFETY: close_range only closes descriptors in the given range; at
    // this point in startup the process owns nothing above stdio.
    unsafe {
        let _ = libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32);
    }
}

#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};

/// Soft `RLIMIT_NOFILE` this process inherited, recorded by
/// [`raise_nofile_soft_limit`]; `u64::MAX` means the limit was never raised.
#[cfg(unix)]
static INHERITED_NOFILE_SOFT: AtomicU64 = AtomicU64::new(u64::MAX);

/// Current `RLIMIT_NOFILE`, or `None` if `getrlimit` failed (errno is set).
/// Async-signal-safe.
#[cfg(unix)]
fn nofile_limit() -> Option<libc::rlimit> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit only writes the provided struct.
    (unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0).then_some(limit)
}

/// Raise the soft `RLIMIT_NOFILE` to the hard limit, remembering the inherited
/// value for [`restore_inherited_nofile_limit`]. Returns `(previous, raised)`
/// when the limit changed, `None` when it was already at the hard limit.
#[cfg(unix)]
pub fn raise_nofile_soft_limit() -> std::io::Result<Option<(u64, u64)>> {
    let mut limit = nofile_limit().ok_or_else(std::io::Error::last_os_error)?;
    if limit.rlim_cur >= limit.rlim_max {
        return Ok(None);
    }
    let previous = limit.rlim_cur;
    limit.rlim_cur = limit.rlim_max;
    // SAFETY: setrlimit only reads the provided struct.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    INHERITED_NOFILE_SOFT.store(previous, Ordering::Relaxed);
    Ok(Some((previous, limit.rlim_cur)))
}

/// Restore the inherited soft `RLIMIT_NOFILE` in a child about to `exec`, so
/// launched programs see the conventional limit (code using `select(2)` or
/// sizing tables by the fd limit misbehaves at the raised value).
///
/// Async-signal-safe: an atomic load plus raw syscalls, so it may run in
/// `pre_exec`. No-op when the limit was never raised.
#[cfg(unix)]
pub fn restore_inherited_nofile_limit() {
    let inherited = INHERITED_NOFILE_SOFT.load(Ordering::Relaxed);
    if inherited == u64::MAX {
        return;
    }
    if let Some(mut limit) = nofile_limit() {
        limit.rlim_cur = inherited;
        // SAFETY: setrlimit only reads the provided struct.
        let _ = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn soft_nofile() -> u64 {
        nofile_limit().unwrap().rlim_cur
    }

    #[test]
    fn raise_then_restore_round_trips_the_inherited_soft_limit() {
        let before = soft_nofile();
        let raised = raise_nofile_soft_limit().unwrap();
        if let Some((previous, now)) = raised {
            assert_eq!(previous, before);
            assert_eq!(soft_nofile(), now);
            assert!(now > before);
        }
        restore_inherited_nofile_limit();
        assert_eq!(soft_nofile(), before);
    }
}
