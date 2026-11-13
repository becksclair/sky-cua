//! Generation-checked ownership of the isolated desktop's processes.
//!
//! Every process the lane must clean up is recorded when it is spawned or
//! adopted as a `(pid, start_ticks)` generation. Teardown trusts only that
//! recorded generation, re-validated from `/proc`; a D-Bus name owner or an xpra
//! listing is discovery, never authority to signal. Signals go through a pidfd
//! opened at teardown time, so a pid recycled between validation and the signal
//! can never be hit, and teardown waits for the process to exit before the
//! caller may forget its record.

use std::fs;
#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd;
use std::os::fd::{AsRawFd, OwnedFd};
#[cfg(target_os = "linux")]
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// How long a recorded process gets to exit after `SIGTERM` before `SIGKILL`.
const TERM_GRACE: Duration = Duration::from_secs(3);
/// How long a recorded process gets to exit after `SIGKILL`.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// Poll interval for the `/proc` fallback when pidfds are unavailable.
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// One process generation: a pid plus its kernel start time in clock ticks.
/// Together they identify a single process for the life of the boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProcessIdentity {
    pub(super) pid: u32,
    pub(super) start_ticks: u64,
}

/// What `/proc` says about a recorded generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Liveness {
    /// The generation has exited: the pid is gone, belongs to a newer process,
    /// or is a zombie or dead entry awaiting reaping.
    Gone,
    /// The exact recorded generation is running.
    Alive,
    /// The entry exists but could not be read or parsed. Treated as running so
    /// a record is never discarded on uncertainty.
    Unknown,
}

/// Read the current liveness of a recorded generation.
pub(super) fn liveness(expected: &ProcessIdentity) -> Liveness {
    match fs::read_to_string(format!("/proc/{}/stat", expected.pid)) {
        Ok(stat) => stat_liveness(&stat, expected.start_ticks),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Liveness::Gone,
        Err(_) => Liveness::Unknown,
    }
}

/// Classify one `/proc/<pid>/stat` line against the expected start time. The
/// command name may contain `") "`, so fields are read after the last one.
pub(super) fn stat_liveness(stat: &str, expected_start_ticks: u64) -> Liveness {
    let Some((_, fields)) = stat.rsplit_once(") ") else {
        return Liveness::Unknown;
    };
    let fields: Vec<&str> = fields.split_whitespace().collect();
    let (Some(state), Some(start_ticks)) = (fields.first(), fields.get(19)) else {
        return Liveness::Unknown;
    };
    match start_ticks.parse::<u64>() {
        Ok(start_ticks) if start_ticks != expected_start_ticks => Liveness::Gone,
        Ok(_) if matches!(*state, "Z" | "X" | "x") => Liveness::Gone,
        Ok(_) => Liveness::Alive,
        Err(_) => Liveness::Unknown,
    }
}

/// Whether any process currently runs as `pid`. A zombie or dead entry has
/// exited even though its `/proc` entry remains until it is reaped; an entry
/// that exists but cannot be read counts as running.
pub(super) fn pid_is_running(pid: u32) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !stat
            .rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().next())
            .is_some_and(|state| matches!(state, "Z" | "X" | "x")),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

/// The start time of the process currently holding `pid`.
pub(super) fn process_start_ticks(pid: u32) -> Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("cannot read start time for pid {pid}"))?;
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| anyhow!("malformed /proc/{pid}/stat"))?;
    fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| anyhow!("missing start time in /proc/{pid}/stat"))?
        .parse::<u64>()
        .with_context(|| format!("invalid start time in /proc/{pid}/stat"))
}

/// Record the generation currently holding `pid`.
pub(super) fn identify(pid: u32) -> Result<ProcessIdentity> {
    Ok(ProcessIdentity {
        pid,
        start_ticks: process_start_ticks(pid)?,
    })
}

/// Whether `actual` (from `/proc/<pid>/exe`) is the `expected` executable. A
/// binary replaced by a package upgrade while the process runs reads as
/// `"<path> (deleted)"`; it is still the same recorded process.
#[cfg(target_os = "linux")]
pub(super) fn exe_matches(actual: &Path, expected: &Path) -> bool {
    if actual == expected {
        return true;
    }
    let (Some(actual), Some(expected)) = (actual.to_str(), expected.to_str()) else {
        return false;
    };
    actual
        .strip_suffix(" (deleted)")
        .is_some_and(|stripped| stripped == expected)
}

/// The NUL-separated arguments of `pid`.
#[cfg(target_os = "linux")]
pub(super) fn cmdline_args(pid: u32) -> Result<Vec<String>> {
    let raw = fs::read(format!("/proc/{pid}/cmdline"))
        .with_context(|| format!("cannot read command line for pid {pid}"))?;
    Ok(raw
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect())
}

/// Terminate exactly the recorded generation and wait for it to exit.
///
/// `SIGTERM` first, then `SIGKILL` after [`TERM_GRACE`]. Returns `Ok` once the
/// generation is gone, and `Err` if it cannot be verified or outlives both
/// signals, so the caller keeps its record for a retry.
pub(super) fn terminate_generation(expected: &ProcessIdentity) -> Result<()> {
    let pid = expected.pid;
    let pidfd = match pidfd_open(pid) {
        Ok(fd) => Some(fd),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(()),
        // No pidfd support (old kernel) or a sandbox that forbids it: fall back to
        // kill(2) on the validated generation, polled for exit.
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOSYS | libc::EPERM)) => None,
        Err(error) => {
            return Err(error).with_context(|| format!("cannot open pidfd for pid {pid}"));
        }
    };
    // Validate after the pidfd exists: if the pid now runs the recorded
    // generation, the pidfd refers to that same process, so a later recycle of
    // the number cannot redirect the signal.
    match liveness(expected) {
        Liveness::Gone => return Ok(()),
        Liveness::Unknown => bail!("cannot verify recorded pid {pid} before signalling it"),
        Liveness::Alive => {}
    }
    for (signal, grace) in [(libc::SIGTERM, TERM_GRACE), (libc::SIGKILL, KILL_GRACE)] {
        match &pidfd {
            Some(fd) => {
                if let Err(error) = pidfd_send_signal(fd, signal)
                    && error.raw_os_error() != Some(libc::ESRCH)
                {
                    return Err(error).with_context(|| format!("failed to signal pid {pid}"));
                }
                if pidfd_wait_exit(fd, grace)? {
                    return Ok(());
                }
            }
            None => {
                // SAFETY: plain `kill`; the generation was validated just above.
                if unsafe { libc::kill(pid as libc::pid_t, signal) } != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(error).with_context(|| format!("failed to signal pid {pid}"));
                    }
                }
                if poll_until_gone(expected, grace) {
                    return Ok(());
                }
            }
        }
    }
    bail!("recorded pid {pid} is still running after SIGTERM and SIGKILL")
}

#[cfg(target_os = "linux")]
fn pidfd_open(pid: u32) -> std::io::Result<OwnedFd> {
    // SAFETY: pidfd_open takes a pid and flags and returns a new fd or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the kernel returned a fresh, owned file descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

#[cfg(not(target_os = "linux"))]
fn pidfd_open(_pid: u32) -> std::io::Result<OwnedFd> {
    Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
}

#[cfg(target_os = "linux")]
fn pidfd_send_signal(fd: &OwnedFd, signal: libc::c_int) -> std::io::Result<()> {
    // SAFETY: a valid pidfd, a signal number, no siginfo, no flags.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn pidfd_send_signal(_fd: &OwnedFd, _signal: libc::c_int) -> std::io::Result<()> {
    Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
}

/// Wait up to `timeout` for the pidfd to report exit. A pidfd becomes readable
/// when its process exits, including while it is an unreaped zombie.
fn pidfd_wait_exit(fd: &OwnedFd, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let mut poll_fd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = remaining.as_millis().min(i32::MAX as u128) as libc::c_int;
        // SAFETY: one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut poll_fd, 1, millis) };
        if ready > 0 {
            return Ok(true);
        }
        if ready == 0 {
            return Ok(false);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error).context("failed to wait on a pidfd");
        }
    }
}

fn poll_until_gone(expected: &ProcessIdentity, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if liveness(expected) == Liveness::Gone {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(FALLBACK_POLL_INTERVAL);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Child, Command};

    fn stat_line(state: &str, start_ticks: u64) -> String {
        // Fields after the comm: state is field 3, start time is field 22.
        let mut fields = vec!["0"; 20];
        fields[0] = state;
        let start = start_ticks.to_string();
        fields[19] = &start;
        format!("42 (odd ) name) {}", fields.join(" "))
    }

    #[test]
    fn stat_liveness_requires_the_recorded_generation_and_a_running_state() {
        assert_eq!(stat_liveness(&stat_line("S", 7), 7), Liveness::Alive);
        assert_eq!(stat_liveness(&stat_line("S", 8), 7), Liveness::Gone);
        assert_eq!(stat_liveness(&stat_line("Z", 7), 7), Liveness::Gone);
        assert_eq!(stat_liveness(&stat_line("X", 7), 7), Liveness::Gone);
        assert_eq!(stat_liveness("garbage", 7), Liveness::Unknown);
        assert_eq!(stat_liveness("1 (a) S 1 2", 7), Liveness::Unknown);
    }

    #[test]
    fn exe_matches_tolerates_only_a_deleted_suffix() {
        let expected = PathBuf::from("/usr/lib/Xorg");
        assert!(exe_matches(&expected, &expected));
        assert!(exe_matches(Path::new("/usr/lib/Xorg (deleted)"), &expected));
        assert!(!exe_matches(Path::new("/usr/bin/Xorg"), &expected));
        assert!(!exe_matches(
            Path::new("/usr/lib/Xorg2 (deleted)"),
            &expected
        ));
    }

    #[test]
    fn pid_is_running_excludes_zombies() {
        assert!(pid_is_running(std::process::id()));
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        // Unreaped, the exited child is a zombie with a /proc entry.
        let deadline = Instant::now() + Duration::from_secs(5);
        while pid_is_running(pid) {
            assert!(Instant::now() < deadline, "the child never exited");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(Path::new(&format!("/proc/{pid}")).exists());
        child.wait().unwrap();
    }

    fn spawn(script: &str) -> (Child, ProcessIdentity) {
        let child = Command::new("sh").arg("-c").arg(script).spawn().unwrap();
        let identity = identify(child.id()).unwrap();
        (child, identity)
    }

    #[test]
    fn terminate_generation_stops_a_cooperative_process() {
        let (mut child, identity) = spawn("exec sleep 30");
        terminate_generation(&identity).expect("the recorded process should stop");
        assert!(child.wait().is_ok());
        assert_eq!(liveness(&identity), Liveness::Gone);
    }

    #[test]
    fn terminate_generation_escalates_when_sigterm_is_ignored() {
        let (mut child, identity) = spawn("trap '' TERM; while :; do sleep 1; done");
        // Give the shell time to install the trap before signalling.
        std::thread::sleep(Duration::from_millis(200));
        let started = Instant::now();
        terminate_generation(&identity).expect("SIGKILL should stop a TERM-immune process");
        assert!(started.elapsed() >= TERM_GRACE);
        let status = child.wait().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn terminate_generation_never_signals_a_different_generation() {
        let (mut child, mut identity) = spawn("exec sleep 30");
        identity.start_ticks += 1;
        terminate_generation(&identity).expect("a mismatched generation is already gone");
        assert!(
            child.try_wait().unwrap().is_none(),
            "the live process was signalled"
        );
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
