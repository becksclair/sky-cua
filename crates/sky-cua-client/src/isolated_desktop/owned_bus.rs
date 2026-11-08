//! The client-owned sandbox `dbus-daemon` fallback: spawn, persistence,
//! recovery, and reaping.
//!
//! On hosts where xpra reports no D-Bus session bus, bring-up starts a
//! client-owned `dbus-daemon`. To make that bus a SESSION resource (reusable
//! across client restarts, reaped on session teardown) rather than a
//! client-scoped one, its address and process generation (pid plus start time)
//! are recorded under the runtime dir at `$XDG_RUNTIME_DIR/sky-cua/isolated-bus-<N>`. This module owns that record and
//! the process lifecycle around it; the parent ([`super`]) drives it from
//! `ensure`/`stop`. The parent is `#[cfg(unix)]` (declared in `main.rs`), so this
//! submodule inherits that gate. The owned-bus unit tests live in the parent's
//! test module next to the socket-path tests; all of them serialize
//! `XDG_RUNTIME_DIR` mutation through `crate::test_env::env_lock()`.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

use super::owned_process::{self, Liveness, ProcessIdentity};
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

/// `$XDG_RUNTIME_DIR/sky-cua/isolated-bus-<N>`, recording a client-owned sandbox
/// `dbus-daemon`'s address and pid. It exists only for the owned-bus fallback
/// (xpra reported no session bus), so a later client can recover the bus on reuse
/// — its address is not otherwise discoverable from `xpra info` — and so teardown
/// can reap it without the originating handle. `None` when no runtime dir exists.
fn isolated_bus_state_path(display_number: u32) -> Option<PathBuf> {
    sky_cua_runtime_dir().map(|dir| dir.join(format!("isolated-bus-{display_number}")))
}

/// Persist a client-owned sandbox bus's address and process generation.
/// Best-effort: a failure only costs a later reuse falling back to a fresh start.
pub(super) fn persist_owned_bus(display_number: u32, address: &str, pid: u32) {
    let Some(path) = isolated_bus_state_path(display_number) else {
        return;
    };
    let Ok(identity) = owned_process::identify(pid) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format_owned_bus(address, &identity));
}

fn format_owned_bus(address: &str, identity: &ProcessIdentity) -> String {
    format!("{address}\n{}\n{}\n", identity.pid, identity.start_ticks)
}

/// Parse a persisted client-owned sandbox bus record: a non-empty address, a pid
/// greater than 1, and that pid's start time. Anything else is not a record this
/// version can verify, so it is never used to signal.
pub(super) fn parse_owned_bus(raw: &str) -> Option<(String, ProcessIdentity)> {
    let mut lines = raw.lines();
    let address = lines.next()?.trim().to_string();
    let pid = lines.next()?.trim().parse::<u32>().ok()?;
    let start_ticks = lines.next()?.trim().parse::<u64>().ok()?;
    if address.is_empty() || pid <= 1 {
        return None;
    }
    Some((address, ProcessIdentity { pid, start_ticks }))
}

/// Read a persisted client-owned sandbox bus record, if present and well-formed.
pub(super) fn read_owned_bus(display_number: u32) -> Option<(String, ProcessIdentity)> {
    let path = isolated_bus_state_path(display_number)?;
    parse_owned_bus(&std::fs::read_to_string(&path).ok()?)
}

/// Remove the persisted client-owned sandbox bus record for `display_number`.
pub(super) fn remove_owned_bus_state(display_number: u32) {
    if let Some(path) = isolated_bus_state_path(display_number) {
        let _ = std::fs::remove_file(path);
    }
}

/// Recover a still-live client-owned sandbox bus address for reuse. Returns the
/// address only when the recorded `dbus-daemon` generation is still running; a
/// record for an exited bus is removed and `None` returned so the caller restarts
/// rather than pointing the daemon at a dead bus.
pub(super) fn recover_owned_bus(display_number: u32) -> Option<String> {
    let (address, identity) = read_owned_bus(display_number)?;
    match owned_process::liveness(&identity) {
        Liveness::Alive if pid_is_dbus_daemon(identity.pid) => Some(address),
        Liveness::Unknown => None,
        _ => {
            remove_owned_bus_state(display_number);
            None
        }
    }
}

/// Reap a persisted client-owned sandbox `dbus-daemon` and remove its record.
/// Only the exact recorded generation is signalled, and the record is kept when
/// that generation cannot be verified or does not exit, so a retry can finish
/// the job. Safe when no owned bus was persisted.
pub(super) fn reap_persisted_owned_bus(display_number: u32) -> Result<()> {
    let Some(path) = isolated_bus_state_path(display_number) else {
        return Ok(());
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let Some((_, identity)) = parse_owned_bus(&raw) else {
        // Not a verifiable record: never signal on it, but do not let it linger.
        remove_owned_bus_state(display_number);
        return Ok(());
    };
    match owned_process::liveness(&identity) {
        Liveness::Gone => {}
        Liveness::Unknown => bail!(
            "cannot verify the client-owned sandbox bus pid {}; keeping its record",
            identity.pid
        ),
        Liveness::Alive => {
            if !pid_is_dbus_daemon(identity.pid) {
                bail!(
                    "recorded sandbox bus pid {} is running but is not a dbus-daemon; \
                     refusing to signal it",
                    identity.pid
                );
            }
            owned_process::terminate_generation(&identity)
                .context("failed to stop the client-owned sandbox bus")?;
        }
    }
    remove_owned_bus_state(display_number);
    Ok(())
}

/// Whether `pid` is a live `dbus-daemon` process, by its `/proc/<pid>/comm`
/// basename.
pub(super) fn pid_is_dbus_daemon(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|comm| comm.trim() == "dbus-daemon")
        .unwrap_or(false)
}
