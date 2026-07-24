//! Host-leak regression guard for isolated-desktop application launch.
//!
//! The feasibility spike found two escape vectors by which an app launched into
//! the sandbox could surface on the user's real desktop: (1) `WAYLAND_DISPLAY`
//! still set, so Qt prefers Wayland over X11; and (2) a single-instance toolkit
//! app (KDE's KDBusService — e.g. `kcalc`) reaching the user's REAL session bus
//! over an inherited host `DBUS_SESSION_BUS_ADDRESS` and being activated there,
//! escaping even with the display vars correct. The fix is the daemon-environment
//! sandbox — `DISPLAY=:N`, `QT_QPA_PLATFORM=xcb`, `GDK_BACKEND=x11`,
//! `WAYLAND_DISPLAY` removed, and `DBUS_SESSION_BUS_ADDRESS` pointed at the
//! private sandbox session bus — applied once at spawn and inherited by every
//! launched application.
//!
//! This test guards one half of the regression: that
//! [`LinuxDesktopBackend::launch_application`] launches a child which INHERITS
//! the process (daemon) environment verbatim and adds no display/session
//! tinkering of its own. It applies the full sandbox env to this process —
//! including a PRIVATE throwaway `dbus-daemon` session bus so a single-instance
//! app cannot reach the user's real bus — exercises `launch_application` against
//! a throwaway sandbox display `:N`, then asserts the launched application:
//!   1. is PRESENT on the sandbox display `:N`,
//!   2. is ABSENT from the user's real (host) display, and
//!   3. carries the sandbox markers in `/proc/<pid>/environ` — `DISPLAY=:N`,
//!      no `WAYLAND_DISPLAY`, `QT_QPA_PLATFORM=xcb`, and the private sandbox
//!      `DBUS_SESSION_BUS_ADDRESS`.
//!
//! The OTHER half — that the sandbox env recipe itself is correct (i.e. that
//! `WAYLAND_DISPLAY` is removed and `QT_QPA_PLATFORM=xcb`/`GDK_BACKEND=x11` are
//! set on the isolated daemon) — is pinned by the client-crate unit tests
//! `isolated_desktop_builds_spawn_env` and
//! `isolated_desktop_removes_only_wayland_display_env` (the producers of
//! `IsolatedDesktopHandle::spawn_env`/`removed_env`, which live in `sky-cua-client`
//! and so cannot be called from this `sky-cua-linux` test). Together the two
//! guards forbid the spike regression: a correct recipe (unit-tested) inherited
//! verbatim (this test).
//!
//! Leak-safety is by construction: `QT_QPA_PLATFORM=xcb` with no
//! `WAYLAND_DISPLAY` keeps the toolkit off Wayland, and the private session bus
//! denies a single-instance app any route to the user's real session — so the
//! app lands on `:N` or fails, and cannot surface on the user's desktop even if
//! the assertion mechanism regresses. (Both halves are required: the display
//! vars alone do NOT contain a KDBusService app, which is why omitting the
//! private bus once let `kcalc` escape onto the host during bring-up.)
//!
//! It is `#[cfg(unix)]` gated, spins a throwaway headless X server (xpra
//! preferred, Xvfb fallback) and a throwaway `dbus-daemon` session bus, and
//! skips cleanly when xpra/Xvfb, xdpyinfo, xdotool, dbus-daemon, or a launchable
//! GUI app (kcalc, then xmessage) is missing. It joins the `serial-integration`
//! nextest group because it mutates process-global
//! `DISPLAY`/`XDG_SESSION_TYPE`/`WAYLAND_DISPLAY`/`QT_QPA_PLATFORM`/`GDK_BACKEND`
//! and spawns a child X server.

#![cfg(unix)]

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::{CLIENT_CLEARED_SESSION_ENV_KEYS_ENV, GRAPHICAL_SESSION_ENV_KEYS};

use sky_cua_linux::backend::LinuxDesktopBackend;

/// Which headless X server provider the host offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayProvider {
    Xpra,
    Xvfb,
}

/// A GUI application to launch into the sandbox. `kcalc` is the spike's Qt
/// repro (the toolkit app that escaped); `xmessage` is the pure-Xlib fallback
/// that needs no Qt/KDE stack but still exercises the same launch path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchApp {
    /// `kcalc -- ` with no extra args.
    Kcalc,
    /// `xmessage <text>`.
    Xmessage,
}

impl LaunchApp {
    fn command(self) -> &'static str {
        match self {
            LaunchApp::Kcalc => "kcalc",
            LaunchApp::Xmessage => "xmessage",
        }
    }

    fn args(self) -> Vec<String> {
        match self {
            LaunchApp::Kcalc => Vec::new(),
            LaunchApp::Xmessage => vec!["sky-cua isolated leak guard".to_string()],
        }
    }
}

/// Restores, on drop, every environment variable the test mutates or the
/// backend's `hydrate_session_env` can set, so the test cannot leak its sandbox
/// env into the rest of the process even on a panic. It covers the full
/// `GRAPHICAL_SESSION_ENV_KEYS` set (what hydration touches — including
/// `XDG_RUNTIME_DIR`/`XDG_CURRENT_DESKTOP`/`DESKTOP_SESSION`), `XAUTHORITY`, the
/// launch-only toolkit vars, and the cleared-keys signal the test sets. Captures
/// the ORIGINAL `DISPLAY` so the host-absence assertion can query the user's real
/// display.
struct EnvRestore {
    saved: Vec<(&'static str, Option<OsString>)>,
    display_at_capture: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            saved: restorable_env_keys()
                .into_iter()
                .map(|key| (key, env::var_os(key)))
                .collect(),
            display_at_capture: env::var_os("DISPLAY"),
        }
    }

    /// The user's real display string captured before the sandbox env was
    /// applied, e.g. `":0"`. `None` when the host had no `DISPLAY` (a
    /// Wayland-only host with no queryable X root).
    fn original_display(&self) -> Option<String> {
        self.display_at_capture
            .as_ref()
            .and_then(|value| value.to_str())
            .map(str::to_string)
            .filter(|display| !display.is_empty())
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in std::mem::take(&mut self.saved) {
            restore_var(key, value);
        }
    }
}

/// Every key the test mutates or the backend's `hydrate_session_env` can set: the
/// full graphical-session set plus `XAUTHORITY`, the launch-only toolkit vars,
/// and the cleared-keys signal. `GRAPHICAL_SESSION_ENV_KEYS` already includes
/// DISPLAY/XDG_SESSION_TYPE/WAYLAND_DISPLAY/DBUS_SESSION_BUS_ADDRESS/XDG_RUNTIME_DIR/
/// XDG_CURRENT_DESKTOP/DESKTOP_SESSION.
fn restorable_env_keys() -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = GRAPHICAL_SESSION_ENV_KEYS.to_vec();
    keys.push("XAUTHORITY");
    keys.push("QT_QPA_PLATFORM");
    keys.push("GDK_BACKEND");
    keys.push(CLIENT_CLEARED_SESSION_ENV_KEYS_ENV);
    keys
}

fn restore_var(name: &str, value: Option<OsString>) {
    match value {
        // SAFETY: this test is in the `serial-integration` nextest group
        // (max-threads = 1) and nextest runs each test in its own process, so
        // no other thread is reading or writing the environment concurrently.
        Some(value) => unsafe { env::set_var(name, value) },
        None => unsafe { env::remove_var(name) },
    }
}

/// Kills the launched application and tears down the throwaway display when
/// dropped, so a panic mid-test still reaps both. Teardown filters strictly by
/// the display number `:N` and the launched pid; it never uses a broad
/// `pkill -f` pattern (the spike's footgun that killed the running shell).
struct LeakGuard {
    provider: DisplayProvider,
    display: String,
    number: u32,
    server_child: Option<std::process::Child>,
    app_pid: Option<u32>,
    /// The throwaway private session `dbus-daemon` (the sandbox bus), reaped by
    /// handle so no broker outlives the test.
    bus_child: Option<std::process::Child>,
}

impl LeakGuard {
    fn teardown(&mut self) {
        // Kill the launched app first (by pid, never by pattern) so it cannot
        // outlive the display.
        if let Some(pid) = self.app_pid.take() {
            // SAFETY: SIGTERM has no Rust-side memory-safety preconditions; the
            // pid is the application this test launched moments ago.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
        // Reap the throwaway sandbox bus broker (by handle, never by pattern).
        if let Some(mut bus) = self.bus_child.take() {
            let _ = bus.kill();
            let _ = bus.wait();
        }
        match self.provider {
            DisplayProvider::Xpra => {
                // `xpra stop :N` targets exactly the one display.
                let _ = Command::new("xpra").arg("stop").arg(&self.display).status();
            }
            DisplayProvider::Xvfb => {
                // Kill only the child we spawned (by handle, never by pattern).
                if let Some(mut child) = self.server_child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        remove_stale_lock(self.number);
    }
}

impl Drop for LeakGuard {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[test]
fn isolated_app_launch_leak_guard_keeps_app_off_host() {
    let Some(provider) = detect_display_provider() else {
        eprintln!(
            "skip: isolated_app_launch_leak_guard_keeps_app_off_host requires a headless X server \
             (xpra or Xvfb); neither is installed"
        );
        return;
    };

    // xdpyinfo is the readiness probe; xdotool is the window-query mechanism for
    // both the sandbox-present and host-absent assertions. Without either the
    // test would be asserting on missing tooling rather than the env recipe.
    if !command_available("xdpyinfo") {
        eprintln!(
            "skip: isolated_app_launch_leak_guard_keeps_app_off_host requires xdpyinfo on PATH"
        );
        return;
    }
    if !command_available("xdotool") {
        eprintln!(
            "skip: isolated_app_launch_leak_guard_keeps_app_off_host requires xdotool on PATH"
        );
        return;
    }
    // dbus-daemon backs the PRIVATE sandbox session bus. Without it the launch
    // would fall back to the host bus and a single-instance toolkit app could
    // escape onto the user's desktop, so skip rather than run an unsafe launch.
    if !command_available("dbus-daemon") {
        eprintln!(
            "skip: isolated_app_launch_leak_guard_keeps_app_off_host requires dbus-daemon for the \
             private sandbox session bus"
        );
        return;
    }

    let Some(app) = detect_launch_app() else {
        eprintln!(
            "skip: isolated_app_launch_leak_guard_keeps_app_off_host requires a launchable GUI app \
             (kcalc or xmessage); neither is installed"
        );
        return;
    };

    let number = pick_free_display_number()
        .expect("a free X display number should exist for the throwaway server");
    let display = format!(":{number}");

    // Capture the original env (including the host DISPLAY) before any mutation
    // so the EnvRestore guard undoes everything on every exit path and the
    // host-absence assertion can target the user's real display.
    let env_restore = EnvRestore::capture();
    let host_display = env_restore.original_display();

    let mut guard = start_display(provider, &display, number).unwrap_or_else(|error| {
        panic!("failed to start throwaway {provider:?} display {display}: {error}")
    });

    wait_for_display_ready(&display, Duration::from_secs(10)).unwrap_or_else(|error| {
        guard.teardown();
        panic!("throwaway display {display} never became ready: {error}");
    });

    // Apply the FULL sandbox env to THIS process (mirroring the isolated
    // daemon's spawn_env / removed_env). launch_application inherits the process
    // env verbatim, so this env is the entire thing that contains the launched
    // app. Both halves are load-bearing:
    //   - Display: DISPLAY=:N + QT_QPA_PLATFORM=xcb + GDK_BACKEND=x11, with
    //     WAYLAND_DISPLAY removed, so a Qt/GTK app speaks X11 to :N and cannot
    //     reach the user's real Wayland session.
    //   - D-Bus: DBUS_SESSION_BUS_ADDRESS pointed at a PRIVATE throwaway session
    //     bus. Without this, a single-instance toolkit app (KDE's KDBusService —
    //     e.g. kcalc) reaches the user's REAL session bus, is activated there,
    //     and shows a window on the host desktop EVEN with the display vars
    //     correct. That is the D-Bus escape the design closes with xpra's sandbox
    //     bus; the test must replicate it, or it both under-tests the sandbox and
    //     risks throwing a real window onto the user's screen.
    set_var("DISPLAY", &display);
    set_var("XDG_SESSION_TYPE", "x11");
    set_var("QT_QPA_PLATFORM", "xcb");
    set_var("GDK_BACKEND", "x11");
    remove_var("WAYLAND_DISPLAY");
    remove_var("XAUTHORITY");
    // Replicate the daemon's spawn contract: the client tells the isolated daemon
    // which graphical-session keys it deliberately cleared, so the daemon's
    // session-env hydration (LinuxDesktopBackend::new -> hydrate_session_env) does
    // NOT re-add them from the host session. Without this the backend re-hydrates
    // WAYLAND_DISPLAY=wayland-0 from the live KDE session and the launched app
    // inherits it — the exact silent re-entry the real daemon suppresses.
    set_var(
        CLIENT_CLEARED_SESSION_ENV_KEYS_ENV,
        cleared_graphical_keys_json(),
    );
    let (sandbox_bus_address, sandbox_bus_child) =
        start_sandbox_session_bus().unwrap_or_else(|error| {
            guard.teardown();
            panic!("failed to start a throwaway sandbox session bus: {error}");
        });
    guard.bus_child = Some(sandbox_bus_child);
    set_var("DBUS_SESSION_BUS_ADDRESS", &sandbox_bus_address);

    // Drive the async backend on a throwaway current-thread runtime, exactly as
    // the env-recipe probe test does.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("building a tokio runtime for the launch should succeed");

    let backend = LinuxDesktopBackend::new();
    let launched = runtime
        .block_on(backend.launch_application(app.command(), &app.args()))
        .unwrap_or_else(|error| {
            guard.teardown();
            panic!(
                "launch_application({}) failed against sandbox {display}: {error}",
                app.command()
            );
        });
    let pid = launched.pid;
    guard.app_pid = Some(pid);

    // Assertion 1: the launched window is PRESENT on the sandbox display :N.
    // Bounded poll: GUI startup is not instantaneous. Prefer matching by pid
    // (xdotool reads _NET_WM_PID); fall back to "any window on :N maps to our
    // process tree" via the pid match only — we do not loosen to a name match,
    // because a name match could in principle catch a host window.
    let present_on_sandbox = wait_for_window_on_display(&display, pid, Duration::from_secs(15));
    if !present_on_sandbox {
        guard.teardown();
        panic!(
            "launched {} (pid {pid}) never appeared on the sandbox display {display}; \
             xdotool search --all --pid {pid} on {display} found no window",
            app.command()
        );
    }

    // Assertion 2: the launched window is ABSENT from the user's real (host)
    // display. Query the host root window list for this pid; assert none.
    match host_display.as_deref() {
        Some(host) if host != display && xdpyinfo_reachable(host) => {
            let host_windows = windows_for_pid_on_display(host, pid);
            assert!(
                host_windows.is_empty(),
                "leak detected: launched {} (pid {pid}) has window(s) {host_windows:?} on the \
                 user's real display {host}; the sandbox env must keep it on {display} only",
                app.command()
            );
        }
        _ => {
            // Host is Wayland-only (no DISPLAY) or its X root is not queryable, so
            // a positive host-window query is unavailable. The /proc environ
            // checks below are then the authoritative leak guard: DISPLAY=:N with
            // no WAYLAND_DISPLAY keeps the app off the user's Wayland session, and
            // the sandbox-bus-present / host-bus-absent pair forecloses the
            // KDBusService activation vector. Both are host-agnostic, so this skip
            // only forgoes the redundant window observation; it does not leave an
            // actual escape vector unguarded.
            eprintln!(
                "note: host display {host_display:?} is not a queryable X server; relying on the \
                 /proc/<pid>/environ assertions (sandbox display + sandbox bus, host bus absent) \
                 for host-leak safety"
            );
        }
    }

    // Assertion 3: /proc/<pid>/environ carries the sandbox markers. This proves
    // the child inherited the sandbox env and therefore could not have reached
    // the user's Wayland session, independent of any window-query result.
    let environ = read_proc_environ(pid).unwrap_or_else(|error| {
        guard.teardown();
        panic!(
            "could not read /proc/{pid}/environ for launched {}: {error}",
            app.command()
        );
    });
    assert!(
        environ
            .iter()
            .any(|entry| entry == &format!("DISPLAY={display}")),
        "launched {} (pid {pid}) environ must contain DISPLAY={display}; got display entries {:?}",
        app.command(),
        environ
            .iter()
            .filter(|entry| entry.starts_with("DISPLAY="))
            .collect::<Vec<_>>()
    );
    assert!(
        !environ
            .iter()
            .any(|entry| entry.starts_with("WAYLAND_DISPLAY=")),
        "launched {} (pid {pid}) environ must NOT contain WAYLAND_DISPLAY (the spike's escape \
         vector); got {:?}",
        app.command(),
        environ
            .iter()
            .filter(|entry| entry.starts_with("WAYLAND_DISPLAY="))
            .collect::<Vec<_>>()
    );
    assert!(
        !environ.iter().any(|entry| entry.starts_with("XAUTHORITY=")),
        "launched {} (pid {pid}) environ must NOT contain XAUTHORITY; got {:?}",
        app.command(),
        environ
            .iter()
            .filter(|entry| entry.starts_with("XAUTHORITY="))
            .collect::<Vec<_>>()
    );
    assert!(
        environ.iter().any(|entry| entry == "QT_QPA_PLATFORM=xcb"),
        "launched {} (pid {pid}) environ must contain QT_QPA_PLATFORM=xcb so Qt cannot prefer \
         Wayland; got {:?}",
        app.command(),
        environ
            .iter()
            .filter(|entry| entry.starts_with("QT_QPA_PLATFORM="))
            .collect::<Vec<_>>()
    );
    // The launched app must be on the PRIVATE sandbox session bus, not the
    // user's real bus. This is the assertion that catches the D-Bus single-
    // instance escape (a host-bus toolkit app gets activated on the user's
    // session and shows a window on the host desktop).
    assert!(
        environ
            .iter()
            .any(|entry| entry == &format!("DBUS_SESSION_BUS_ADDRESS={sandbox_bus_address}")),
        "launched {} (pid {pid}) environ must carry the sandbox \
         DBUS_SESSION_BUS_ADDRESS={sandbox_bus_address} (not the host session bus, the KDBusService \
         escape vector); got {:?}",
        app.command(),
        environ
            .iter()
            .filter(|entry| entry.starts_with("DBUS_SESSION_BUS_ADDRESS="))
            .collect::<Vec<_>>()
    );
    // And it must carry NO OTHER session-bus address — in particular not the
    // user's real bus. Checked unconditionally (every `DBUS_SESSION_BUS_ADDRESS`
    // entry must equal the sandbox bus), so it bites even on a bus-less host where
    // there was no host bus to compare against. Together with the sandbox-bus
    // assertion above this is necessary and sufficient to keep a single-instance
    // toolkit app (kcalc's KDBusService) from activating on the user's session and
    // surfacing a window — and it is the authoritative guard on a Wayland-only
    // host, where Assertion 2's host-window query is unavailable.
    assert!(
        !environ.iter().any(|entry| {
            entry.starts_with("DBUS_SESSION_BUS_ADDRESS=")
                && entry != &format!("DBUS_SESSION_BUS_ADDRESS={sandbox_bus_address}")
        }),
        "launched {} (pid {pid}) environ must carry ONLY the sandbox session bus \
         DBUS_SESSION_BUS_ADDRESS={sandbox_bus_address}; any other bus entry is the KDBusService \
         host-activation vector. Got {:?}",
        app.command(),
        environ
            .iter()
            .filter(|entry| entry.starts_with("DBUS_SESSION_BUS_ADDRESS="))
            .collect::<Vec<_>>()
    );

    // Explicit teardown so success leaves no orphan app, server, or lock; the
    // guard's Drop is the panic-path backstop. The EnvRestore guard runs on
    // scope exit and restores the process env.
    guard.teardown();
    drop(env_restore);
}

/// Prefer xpra (the production isolated-desktop provider); fall back to Xvfb.
fn detect_display_provider() -> Option<DisplayProvider> {
    if command_available("xpra") {
        Some(DisplayProvider::Xpra)
    } else if command_available("Xvfb") {
        Some(DisplayProvider::Xvfb)
    } else {
        None
    }
}

/// Prefer the Qt repro `kcalc`; fall back to pure-Xlib `xmessage`.
fn detect_launch_app() -> Option<LaunchApp> {
    if command_on_path("kcalc") {
        Some(LaunchApp::Kcalc)
    } else if command_on_path("xmessage") {
        Some(LaunchApp::Xmessage)
    } else {
        None
    }
}

fn command_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Whether `name` resolves on `PATH`. Unlike [`command_available`], this does
/// not run the program with `--version` (kcalc/xmessage do not all support it),
/// so it is the right probe for GUI apps we must not actually start during
/// detection.
fn command_on_path(name: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The JSON array the client passes to the daemon via
/// `CLIENT_CLEARED_SESSION_ENV_KEYS`, listing the graphical-session and
/// spawn-only keys it cleared so `hydrate_session_env` will not re-add graphical
/// values from the host session.
/// Built by hand (matching `serde_json::to_string(&[&str])`) to avoid a
/// dev-dependency just for this one string.
fn cleared_graphical_keys_json() -> String {
    let quoted: Vec<String> = GRAPHICAL_SESSION_ENV_KEYS
        .iter()
        .copied()
        .chain(std::iter::once("XAUTHORITY"))
        .map(|key| format!("\"{key}\""))
        .collect();
    format!("[{}]", quoted.join(","))
}

/// Start a private throwaway `dbus-daemon --session` to stand in for the
/// sandbox's session bus, returning its printed address and the child to reap.
/// The launched app registers single-instance services here instead of on the
/// user's real bus, which is what prevents a KDE app (KDBusService) from being
/// activated on the host session and showing a window on the user's desktop.
fn start_sandbox_session_bus() -> std::io::Result<(String, std::process::Child)> {
    use std::io::{BufRead, BufReader};

    let mut child = Command::new("dbus-daemon")
        .arg("--session")
        .arg("--print-address")
        .arg("--nofork")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("dbus-daemon did not expose a stdout pipe"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let address = line.trim().to_string();
    if address.is_empty() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(std::io::Error::other(
            "dbus-daemon printed an empty session bus address",
        ));
    }
    Ok((address, child))
}

/// Picks a display number not currently claimed by a `/tmp/.X<N>-lock`, so the
/// throwaway server never collides with the user's real session or a concurrent
/// test run. Searches the conventional headless range.
fn pick_free_display_number() -> Option<u32> {
    (90u32..=199).find(|number| !lock_path(*number).exists())
}

fn lock_path(number: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/.X{number}-lock"))
}

fn remove_stale_lock(number: u32) {
    let lock = lock_path(number);
    if lock.exists() {
        let _ = std::fs::remove_file(&lock);
    }
}

fn start_display(
    provider: DisplayProvider,
    display: &str,
    number: u32,
) -> std::io::Result<LeakGuard> {
    match provider {
        DisplayProvider::Xpra => {
            // The flag set proven during the spike: a daemonized desktop server
            // running Openbox, with the noisy peripherals disabled.
            let status = Command::new("xpra")
                .arg("start-desktop")
                .arg(display)
                .args([
                    "--start=openbox",
                    "--daemon=yes",
                    "--notifications=no",
                    "--bell=no",
                    "--webcam=no",
                    "--pulseaudio=no",
                    "--mdns=no",
                    "--start-new-commands=no",
                    "--systemd-run=no",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?;
            if !status.success() {
                return Err(std::io::Error::other(format!(
                    "xpra start-desktop {display} exited with {status}"
                )));
            }
            Ok(LeakGuard {
                provider,
                display: display.to_string(),
                number,
                server_child: None,
                app_pid: None,
                bus_child: None,
            })
        }
        DisplayProvider::Xvfb => {
            // Xvfb stays in the foreground; keep the child handle so teardown
            // kills exactly this process and nothing else.
            let child = Command::new("Xvfb")
                .arg(display)
                .args(["-screen", "0", "1920x1080x24"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            Ok(LeakGuard {
                provider,
                display: display.to_string(),
                number,
                server_child: Some(child),
                app_pid: None,
                bus_child: None,
            })
        }
    }
}

/// Bounded poll on `xdpyinfo -display :N` until the server answers or the
/// deadline passes. No fixed sleep; the display is considered ready the instant
/// xdpyinfo succeeds.
fn wait_for_display_ready(display: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if xdpyinfo_reachable(display) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "xdpyinfo -display {display} did not succeed within {timeout:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Whether `xdpyinfo -display <display>` succeeds (the X server is reachable).
fn xdpyinfo_reachable(display: &str) -> bool {
    Command::new("xdpyinfo")
        .arg("-display")
        .arg(display)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Bounded poll until a window owned by `pid` (via `_NET_WM_PID`) appears on
/// `display`, or the deadline passes. GUI startup is not instantaneous.
fn wait_for_window_on_display(display: &str, pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !windows_for_pid_on_display(display, pid).is_empty() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Window ids on `display` whose `_NET_WM_PID` equals `pid`, via
/// `xdotool search --all --pid <pid>`. `--all` requires every match criterion;
/// with only `--pid` it returns exactly the windows owned by that process.
/// Returns an empty vec when xdotool finds nothing or fails (no window yet).
fn windows_for_pid_on_display(display: &str, pid: u32) -> Vec<String> {
    let output = Command::new("xdotool")
        .env("DISPLAY", display)
        .arg("search")
        .arg("--all")
        .arg("--pid")
        .arg(pid.to_string())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    // xdotool exits non-zero when there are no matches; treat that as empty.
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Read `/proc/<pid>/environ` and split it into `KEY=VALUE` entries. The file
/// is NUL-separated; a trailing empty entry from a terminating NUL is dropped.
fn read_proc_environ(pid: u32) -> Result<Vec<String>, String> {
    let raw = std::fs::read(format!("/proc/{pid}/environ"))
        .map_err(|error| format!("reading /proc/{pid}/environ: {error}"))?;
    Ok(raw
        .split(|byte| *byte == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect())
}

fn set_var(name: &str, value: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: serial-integration group + per-process nextest isolation; no
    // concurrent env access.
    unsafe { env::set_var(name, value.as_ref()) }
}

fn remove_var(name: &str) {
    // SAFETY: see `set_var`.
    unsafe { env::remove_var(name) }
}
