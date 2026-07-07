//! The client-owned sandbox `dbus-daemon` fallback: spawn, persistence,
//! recovery, and reaping.
//!
//! On hosts where xpra reports no D-Bus session bus, bring-up starts a
//! client-owned `dbus-daemon`. To make that bus a SESSION resource (reusable
//! across client restarts, reaped on session teardown) rather than a
//! client-scoped one, its address and pid are recorded under the runtime dir at
//! `$XDG_RUNTIME_DIR/sky-cua/isolated-bus-<N>`. This module owns that record and
//! the process lifecycle around it; the parent ([`super`]) drives it from
//! `ensure`/`stop`. The parent is `#[cfg(unix)]` (declared in `main.rs`), so this
//! submodule inherits that gate. The owned-bus unit tests live in the parent's
//! test module because they share its `ENV_LOCK` (serializing `XDG_RUNTIME_DIR`
//! mutation with the socket-path tests).

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

use super::sky_cua_runtime_dir;

/// Start a client-owned `dbus-daemon --session --print-address` for the
/// sandbox, returning its bus address and the child to reap. Used only when
/// xpra did not provide a session bus.
pub(super) fn start_owned_session_bus() -> Result<(String, Child)> {
    let mut child = Command::new("dbus-daemon")
        .arg("--session")
        .arg("--print-address")
        .arg("--nofork")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn a client-owned dbus-daemon for the sandbox")?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("dbus-daemon did not expose a stdout pipe"))?;
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("failed to read the sandbox dbus-daemon address")?;
    let address = line.trim().to_string();
    if address.is_empty() {
        let _ = child.kill();
        let _ = child.wait();
        bail!("the sandbox dbus-daemon printed an empty address");
    }
    Ok((address, child))
}

/// Best-effort termination of a client-owned sandbox `dbus-daemon` by pid.
#[cfg(unix)]
fn reap_owned_session_bus(pid: u32) {
    // SAFETY: `kill` with SIGTERM has no Rust-side memory safety preconditions;
    // the pid is the client-spawned sandbox dbus-daemon recorded on the handle.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn reap_owned_session_bus(_pid: u32) {}

/// `$XDG_RUNTIME_DIR/sky-cua/isolated-bus-<N>`, recording a client-owned sandbox
/// `dbus-daemon`'s address and pid. It exists only for the owned-bus fallback
/// (xpra reported no session bus), so a later client can recover the bus on reuse
/// — its address is not otherwise discoverable from `xpra info` — and so teardown
/// can reap it without the originating handle. `None` when no runtime dir exists.
fn isolated_bus_state_path(display_number: u32) -> Option<PathBuf> {
    sky_cua_runtime_dir().map(|dir| dir.join(format!("isolated-bus-{display_number}")))
}

/// Persist a client-owned sandbox bus's address and pid. Best-effort: a failure
/// only costs a later reuse falling back to a fresh start.
pub(super) fn persist_owned_bus(display_number: u32, address: &str, pid: u32) {
    let Some(path) = isolated_bus_state_path(display_number) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{address}\n{pid}\n"));
}

/// Read a persisted client-owned sandbox bus's `(address, pid)`, if present and
/// well-formed (non-empty address, pid > 1).
pub(super) fn read_owned_bus(display_number: u32) -> Option<(String, u32)> {
    let path = isolated_bus_state_path(display_number)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let mut lines = raw.lines();
    let address = lines.next()?.trim().to_string();
    let pid = lines.next()?.trim().parse::<u32>().ok()?;
    if address.is_empty() || pid <= 1 {
        return None;
    }
    Some((address, pid))
}

/// Remove the persisted client-owned sandbox bus record for `display_number`.
pub(super) fn remove_owned_bus_state(display_number: u32) {
    if let Some(path) = isolated_bus_state_path(display_number) {
        let _ = std::fs::remove_file(path);
    }
}

/// Recover a still-live client-owned sandbox bus address for reuse. Returns the
/// address only when the recorded pid is still a live `dbus-daemon`; a stale
/// record (the bus died) is removed and `None` returned so the caller restarts
/// rather than pointing the daemon at a dead bus.
pub(super) fn recover_owned_bus(display_number: u32) -> Option<String> {
    let (address, pid) = read_owned_bus(display_number)?;
    if pid_is_dbus_daemon(pid) {
        Some(address)
    } else {
        remove_owned_bus_state(display_number);
        None
    }
}

/// Reap a persisted client-owned sandbox `dbus-daemon` by its recorded pid (only
/// when that pid is still a live `dbus-daemon`, so a recycled pid is never
/// signalled) and remove the record. Best-effort; safe when no owned bus was
/// persisted. This is the handle-less teardown path (explicit `stop`) that could
/// not previously reach a client-owned bus.
pub(super) fn reap_persisted_owned_bus(display_number: u32) {
    if let Some((_, pid)) = read_owned_bus(display_number)
        && pid_is_dbus_daemon(pid)
    {
        reap_owned_session_bus(pid);
    }
    remove_owned_bus_state(display_number);
}

/// Whether `pid` is a live `dbus-daemon` process, by its `/proc/<pid>/comm`
/// basename. Used to avoid recovering or signalling a recycled, unrelated pid.
pub(super) fn pid_is_dbus_daemon(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|comm| comm.trim() == "dbus-daemon")
        .unwrap_or(false)
}
