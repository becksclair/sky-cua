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
