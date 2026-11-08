//! Recorded ownership of the private xpra desktop's root processes.
//!
//! `xpra stop` cannot clean up after an xpra server that has already died: the
//! session's Xorg server and private session `dbus-daemon` keep running, and the
//! rest of the session (openbox, portals, AT-SPI) hangs off those two, so they
//! survive too and the display stays locked. Both roots are recorded as process
//! generations as soon as the display answers, and teardown stops exactly those
//! recorded generations after re-validating them from `/proc`.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::atspi::bus_daemon_pid;
use super::owned_bus::pid_is_dbus_daemon;
use super::owned_process::{self, Liveness, ProcessIdentity};
use super::sky_cua_runtime_dir;

const STATE_VERSION: u32 = 1;

/// One recorded process generation plus the executable it was running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RecordedProcess {
    pub(super) identity: ProcessIdentity,
    pub(super) exe: PathBuf,
}

/// `$XDG_RUNTIME_DIR/sky-cua/isolated-xpra-<N>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct XpraSessionState {
    pub(super) version: u32,
    pub(super) display: String,
    pub(super) x_server: RecordedProcess,
    /// The xpra-launched session bus. `None` when the client owns the bus,
    /// which the owned-bus record already covers.
    pub(super) session_bus: Option<RecordedProcess>,
}

fn state_path(display_number: u32) -> Option<PathBuf> {
    sky_cua_runtime_dir().map(|dir| dir.join(format!("isolated-xpra-{display_number}.json")))
}

/// Whether a recorded xpra session exists for `display_number`.
pub(super) fn has_record(display_number: u32) -> bool {
    state_path(display_number).is_some_and(|path| path.exists())
}

/// The pid in an X server lock file: ten space-padded digits and a newline.
pub(super) fn parse_x_lock_pid(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok().filter(|pid| *pid > 1)
}

/// Whether an X server's arguments tie it to this lane's display: either the
/// exact `:N` argument, or a path inside xpra's per-display session directory
/// `$XDG_RUNTIME_DIR/xpra/<N>/` (xpra passes `-logfile` and `-configdir` there).
pub(super) fn x_server_args_match(
    args: &[String],
    display_number: u32,
    runtime_dir: Option<&Path>,
) -> bool {
    let display = format!(":{display_number}");
    if args.contains(&display) {
        return true;
    }
    let Some(runtime_dir) = runtime_dir else {
        return false;
    };
    let session_dir = runtime_dir.join("xpra").join(display_number.to_string());
    args.iter()
        .any(|arg| Path::new(arg).starts_with(&session_dir))
}

/// Whether a process's arguments are xpra's server for display `:N`. xpra
/// rewrites its process title, so the arguments may arrive as one space-joined
/// string; tokens are compared after splitting on whitespace.
pub(super) fn is_xpra_server_for(args: &[String], display_number: u32) -> bool {
    let display = format!(":{display_number}");
    let tokens: Vec<&str> = args.iter().flat_map(|arg| arg.split_whitespace()).collect();
    tokens.iter().any(|token| {
        Path::new(token)
            .file_name()
            .is_some_and(|name| name == "xpra")
    }) && tokens
        .iter()
        .any(|token| matches!(*token, "start-desktop" | "start"))
        && tokens.contains(&display.as_str())
}

/// Whether an xpra server process for `:N` is still running for this user.
/// Positive evidence that xpra is alive, independent of `xpra list` and
/// `xdpyinfo`, which can both fail transiently under load.
pub(super) fn xpra_server_running(display_number: u32) -> bool {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            return false;
        };
        use std::os::unix::fs::MetadataExt;
        entry.metadata().is_ok_and(|meta| meta.uid() == uid)
            && owned_process::pid_is_running(pid)
            && owned_process::cmdline_args(pid)
                .is_ok_and(|args| is_xpra_server_for(&args, display_number))
    })
}

fn xdg_runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn recorded_exe(pid: u32) -> Result<PathBuf> {
    let exe = fs::read_link(format!("/proc/{pid}/exe"))
        .with_context(|| format!("cannot read the executable of pid {pid}"))?;
    let text = exe.to_string_lossy();
    Ok(match text.strip_suffix(" (deleted)") {
        Some(stripped) => PathBuf::from(stripped),
        None => exe,
    })
}

/// Record the lane's X server and, when xpra launched it, its session bus.
/// Called as soon as the display answers, and again once the bus is known, so a
/// client that dies mid bring-up still leaves a record teardown can act on.
pub(super) fn record_session(display_number: u32, session_bus: Option<&str>) -> Result<()> {
    let path = state_path(display_number).ok_or_else(|| {
        anyhow!("cannot record the xpra session because XDG_RUNTIME_DIR is unset")
    })?;
    let lock_path = format!("/tmp/.X{display_number}-lock");
    let lock =
        fs::read_to_string(&lock_path).with_context(|| format!("cannot read {lock_path}"))?;
    let pid = parse_x_lock_pid(&lock).ok_or_else(|| anyhow!("{lock_path} holds no valid pid"))?;
    let args = owned_process::cmdline_args(pid)?;
    if !x_server_args_match(&args, display_number, xdg_runtime_dir().as_deref()) {
        bail!(
            "the X server pid {pid} in {lock_path} does not belong to xpra display :{display_number}"
        );
    }
    let x_server = RecordedProcess {
        identity: owned_process::identify(pid)?,
        exe: recorded_exe(pid)?,
    };
    let session_bus = match session_bus {
        Some(address) => {
            let pid = bus_daemon_pid(address)?;
            if !pid_is_dbus_daemon(pid) {
                bail!("the private session bus reported pid {pid}, which is not a dbus-daemon");
            }
            Some(RecordedProcess {
                identity: owned_process::identify(pid)?,
                exe: recorded_exe(pid)?,
            })
        }
        None => None,
    };
    persist(
        &path,
        &XpraSessionState {
            version: STATE_VERSION,
            display: format!(":{display_number}"),
            x_server,
            session_bus,
        },
    )
}

fn persist(path: &Path, state: &XpraSessionState) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("xpra session state path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let mut bytes = serde_json::to_vec(state)?;
    bytes.push(b'\n');
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("failed to persist xpra session state {}", path.display()))
}

/// Stop the recorded xpra roots for `display_number` and forget the record.
///
/// An exited generation needs nothing. A live one is signalled only when it
/// still runs the recorded executable (and, for the X server, still belongs to
/// this display); otherwise, or when it cannot be verified or does not exit,
/// the record is kept and an error returned so the failure is visible and a
/// retry can finish. Safe when no record exists.
pub(super) fn teardown_recorded(display_number: u32) -> Result<()> {
    let Some(path) = state_path(display_number) else {
        return Ok(());
    };
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let state = match serde_json::from_slice::<XpraSessionState>(&raw) {
        Ok(state) if state.version == STATE_VERSION => state,
        _ => {
            // Not a record this version can verify: never signal on it.
            tracing::warn!(state = %path.display(), "discarding an unreadable xpra session record");
            let _ = fs::remove_file(&path);
            return Ok(());
        }
    };
    let runtime_dir = xdg_runtime_dir();
    let mut errors = Vec::new();
    let mut roots = vec![("X server", &state.x_server, true)];
    if let Some(bus) = &state.session_bus {
        roots.push(("session bus", bus, false));
    }
    for (name, recorded, is_x_server) in roots {
        if let Err(error) = stop_root(
            recorded,
            is_x_server,
            display_number,
            runtime_dir.as_deref(),
        ) {
            errors.push(format!("{name}: {error:#}"));
        }
    }
    if errors.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    bail!(
        "the xpra session on :{display_number} was not fully torn down: {}",
        errors.join("; ")
    )
}

fn stop_root(
    recorded: &RecordedProcess,
    is_x_server: bool,
    display_number: u32,
    runtime_dir: Option<&Path>,
) -> Result<()> {
    let pid = recorded.identity.pid;
    match owned_process::liveness(&recorded.identity) {
        Liveness::Gone => return Ok(()),
        Liveness::Unknown => bail!("cannot verify recorded pid {pid}"),
        Liveness::Alive => {}
    }
    let exe = fs::read_link(format!("/proc/{pid}/exe"))
        .with_context(|| format!("cannot read the executable of recorded pid {pid}"))?;
    if !owned_process::exe_matches(&exe, &recorded.exe) {
        bail!(
            "recorded pid {pid} now runs {}, expected {}; refusing to signal it",
            exe.display(),
            recorded.exe.display()
        );
    }
    if is_x_server
        && !x_server_args_match(
            &owned_process::cmdline_args(pid)?,
            display_number,
            runtime_dir,
        )
    {
        bail!(
            "recorded pid {pid} no longer belongs to display :{display_number}; refusing to signal it"
        );
    }
    owned_process::terminate_generation(&recorded.identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn x_lock_pid_parses_the_padded_lock_format() {
        assert_eq!(parse_x_lock_pid("    456999\n"), Some(456_999));
        assert_eq!(parse_x_lock_pid("1\n"), None);
        assert_eq!(parse_x_lock_pid("garbage"), None);
        assert_eq!(parse_x_lock_pid(""), None);
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn x_server_args_match_only_this_display() {
        let runtime = Path::new("/run/user/1000");
        let xorg = args(&[
            "/usr/lib/Xorg",
            "-novtswitch",
            "-logfile",
            "/run/user/1000/xpra/147/Xorg.log",
            "-configdir",
            "/run/user/1000/xpra/147/xorg",
        ]);
        assert!(x_server_args_match(&xorg, 147, Some(runtime)));
        assert!(!x_server_args_match(&xorg, 14, Some(runtime)));
        assert!(!x_server_args_match(&xorg, 1470, Some(runtime)));
        assert!(!x_server_args_match(&xorg, 147, None));
        assert!(x_server_args_match(&args(&["Xvfb", ":147"]), 147, None));
        assert!(!x_server_args_match(&args(&["Xvfb", ":1470"]), 147, None));
        let foreign = args(&["Xvfb", ":148", "-fbdir", "/run/user/1000/xpra/1470/fb"]);
        assert!(!x_server_args_match(&foreign, 147, Some(runtime)));
    }

    #[test]
    fn xpra_server_args_are_recognised_only_for_this_display() {
        let joined = args(&["/usr/bin/xpra start-desktop :147 --start=openbox --daemon=yes"]);
        assert!(is_xpra_server_for(&joined, 147));
        let split = args(&["/usr/bin/python3", "/usr/bin/xpra", "start-desktop", ":147"]);
        assert!(is_xpra_server_for(&split, 147));
        assert!(!is_xpra_server_for(&joined, 14));
        assert!(!is_xpra_server_for(&joined, 1470));
        assert!(!is_xpra_server_for(
            &args(&["/usr/bin/xpra", "attach", ":147"]),
            147
        ));
        assert!(!is_xpra_server_for(
            &args(&["/usr/bin/xpra", "stop", ":147"]),
            147
        ));
        assert!(!is_xpra_server_for(
            &args(&["/usr/lib/Xorg", "start", ":147"]),
            147
        ));
    }

    #[test]
    fn xpra_session_state_round_trips() {
        let state = XpraSessionState {
            version: STATE_VERSION,
            display: ":147".to_string(),
            x_server: RecordedProcess {
                identity: ProcessIdentity {
                    pid: 456_999,
                    start_ticks: 123,
                },
                exe: PathBuf::from("/usr/lib/Xorg"),
            },
            session_bus: None,
        };
        let json = serde_json::to_vec(&state).unwrap();
        assert_eq!(
            serde_json::from_slice::<XpraSessionState>(&json).unwrap(),
            state
        );
    }

    fn with_runtime_dir<T>(name: &str, body: impl FnOnce(&Path) -> T) -> T {
        let _guard = crate::test_env::env_lock();
        let old = std::env::var_os("XDG_RUNTIME_DIR");
        let temp = std::env::temp_dir().join(format!("sky-cua-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(temp.join("sky-cua")).unwrap();
        // SAFETY: serialized by the crate-wide test env lock; restored below.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &temp) };
        let result = body(&temp);
        match old {
            Some(value) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", value) },
            None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
        }
        let _ = fs::remove_dir_all(&temp);
        result
    }

    /// A stand-in X server: a real process whose arguments name display :997.
    /// `sh -c 'read line' :997` keeps `:997` in its own arguments and blocks on a
    /// builtin, so it has no child that could outlive it.
    fn spawn_fake_x_server() -> (std::process::Child, RecordedProcess) {
        let child = std::process::Command::new("sh")
            .args(["-c", "read line", ":997"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        // Until execve lands, /proc still shows the forked test binary.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !owned_process::cmdline_args(pid)
            .is_ok_and(|args| x_server_args_match(&args, 997, None))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the stand-in never exec'd"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let recorded = RecordedProcess {
            identity: owned_process::identify(pid).unwrap(),
            exe: recorded_exe(pid).unwrap(),
        };
        (child, recorded)
    }

    fn write_state(runtime: &Path, x_server: RecordedProcess) {
        let state = XpraSessionState {
            version: STATE_VERSION,
            display: ":997".to_string(),
            x_server,
            session_bus: None,
        };
        persist(&runtime.join("sky-cua/isolated-xpra-997.json"), &state).unwrap();
    }

    #[test]
    fn teardown_stops_a_recorded_x_server_and_forgets_it() {
        with_runtime_dir("xpra-teardown", |runtime| {
            let (mut child, recorded) = spawn_fake_x_server();
            write_state(runtime, recorded);
            teardown_recorded(997).expect("the recorded server should stop");
            assert!(child.wait().unwrap().signal().is_some());
            assert!(!has_record(997));
        });
    }

    #[test]
    fn teardown_never_signals_a_mismatched_x_server_and_keeps_the_record() {
        with_runtime_dir("xpra-mismatch", |runtime| {
            let (mut child, mut recorded) = spawn_fake_x_server();
            recorded.exe = PathBuf::from("/usr/lib/Xorg");
            write_state(runtime, recorded);
            let error = teardown_recorded(997).expect_err("an exe mismatch must refuse");
            assert!(format!("{error:#}").contains("refusing to signal"));
            assert!(
                child.try_wait().unwrap().is_none(),
                "a mismatched process was signalled"
            );
            assert!(has_record(997));
            child.kill().unwrap();
            child.wait().unwrap();
        });
    }

    #[test]
    fn teardown_of_an_exited_generation_needs_no_signal() {
        with_runtime_dir("xpra-exited", |runtime| {
            let (mut child, mut recorded) = spawn_fake_x_server();
            recorded.identity.start_ticks += 1;
            write_state(runtime, recorded);
            teardown_recorded(997).expect("an exited generation is already gone");
            assert!(
                child.try_wait().unwrap().is_none(),
                "a different generation was signalled"
            );
            assert!(!has_record(997));
            child.kill().unwrap();
            child.wait().unwrap();
        });
    }
}
