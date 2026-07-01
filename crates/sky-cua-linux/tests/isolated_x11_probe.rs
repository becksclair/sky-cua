//! Regression guard for the isolated-xpra-desktop env recipe.
//!
//! The feasibility spike proved a load-bearing finding: with `DISPLAY=:N` set,
//! `XDG_SESSION_TYPE=x11` set, `WAYLAND_DISPLAY` unset, and a reachable X server
//! on `:N`, the Linux backend selects the X11 lane (`session_kind=X11`,
//! `capture_backend=X11`, `input_backend=XTest`) even though
//! `detect_compositor()`'s system-wide `/proc` scan still sees the host's
//! `kwin_wayland`. The `XDG_SESSION_TYPE=x11` early-return in
//! `infer_session_kind` short-circuits the `/proc` vote.
//!
//! This test encodes that finding so a future change to `env_probe` cannot
//! silently break it. It is `#[cfg(unix)]` gated, spins a throwaway headless X
//! server (xpra preferred, Xvfb fallback), and skips cleanly when neither
//! provider is installed.
//!
//! It joins the `serial-integration` nextest group (`max-threads = 1`) because
//! it mutates process-global `DISPLAY`/`XDG_SESSION_TYPE`/`WAYLAND_DISPLAY` and
//! spawns a child X server; running it concurrently with other display/socket
//! tests would race.

#![cfg(unix)]

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use sky_cua_linux::env_probe::probe_environment;
use sky_cua_platform::model::{CaptureBackendKind, InputBackendKind, SessionKind};

/// Which headless X server provider the host offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayProvider {
    Xpra,
    Xvfb,
}

/// Restores the three display-selecting environment variables to their
/// original values when dropped, so the test cannot leak its sandbox env into
/// the rest of the process (even on a panic).
struct EnvRestore {
    display: Option<OsString>,
    xdg_session_type: Option<OsString>,
    wayland_display: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            display: env::var_os("DISPLAY"),
            xdg_session_type: env::var_os("XDG_SESSION_TYPE"),
            wayland_display: env::var_os("WAYLAND_DISPLAY"),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        restore_var("DISPLAY", self.display.take());
        restore_var("XDG_SESSION_TYPE", self.xdg_session_type.take());
        restore_var("WAYLAND_DISPLAY", self.wayland_display.take());
    }
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

/// Tears down the throwaway display when dropped, so a panic mid-test still
/// reaps the X server and lock file. Teardown filters strictly by the display
/// number `:N`; it never uses a broad `pkill -f` pattern (the spike's footgun
/// that killed the running shell).
struct DisplayGuard {
    provider: DisplayProvider,
    display: String,
    number: u32,
    child: Option<std::process::Child>,
}

impl DisplayGuard {
    fn teardown(&mut self) {
        match self.provider {
            DisplayProvider::Xpra => {
                // `xpra stop :N` targets exactly the one display.
                let _ = Command::new("xpra").arg("stop").arg(&self.display).status();
            }
            DisplayProvider::Xvfb => {
                // Kill only the child we spawned (by handle, never by pattern).
                if let Some(mut child) = self.child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        remove_stale_lock(self.number);
    }
}

impl Drop for DisplayGuard {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[test]
fn isolated_x11_env_recipe_selects_x11_lane() {
    let Some(provider) = detect_display_provider() else {
        eprintln!(
            "skip: isolated_x11_env_recipe_selects_x11_lane requires a headless X server \
             (xpra or Xvfb); neither is installed"
        );
        return;
    };

    // xdpyinfo is the readiness probe and is also what the backend uses to
    // decide the X server is reachable; without it the probe cannot confirm the
    // X11 lane, so treat its absence as a skip rather than a failure.
    if !command_available("xdpyinfo") {
        eprintln!("skip: isolated_x11_env_recipe_selects_x11_lane requires xdpyinfo on PATH");
        return;
    }
    // xdotool is what `xtest_is_available()` shells to; without it the backend
    // cannot select the XTest input lane and the assertion would be testing the
    // host's missing tooling rather than the env recipe.
    if !command_available("xdotool") {
        eprintln!("skip: isolated_x11_env_recipe_selects_x11_lane requires xdotool on PATH");
        return;
    }

    let number = pick_free_display_number()
        .expect("a free X display number should exist for the throwaway server");
    let display = format!(":{number}");

    // Restore the process env regardless of how this test exits. Capture before
    // we start the server so a startup failure still restores cleanly.
    let _env_restore = EnvRestore::capture();

    let mut guard = start_display(provider, &display, number).unwrap_or_else(|error| {
        panic!("failed to start throwaway {provider:?} display {display}: {error}")
    });

    wait_for_display_ready(&display, Duration::from_secs(10)).unwrap_or_else(|error| {
        // Drop the guard explicitly before panicking so the half-started server
        // is reaped (the guard's Drop would also run on unwind, but being
        // explicit keeps the failure message close to the teardown).
        guard.teardown();
        panic!("throwaway display {display} never became ready: {error}");
    });

    // Apply the spike's env recipe in-process. Per-process isolation under
    // nextest makes this safe; the EnvRestore guard undoes it on exit.
    set_var("DISPLAY", &display);
    set_var("XDG_SESSION_TYPE", "x11");
    remove_var("WAYLAND_DISPLAY");

    // `probe_environment` is async and side-effect free (it probes the
    // environment and portal versions; it does not open a capture session), so
    // a throwaway current-thread runtime is enough to drive it.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("building a tokio runtime for the probe should succeed");
    let environment = runtime
        .block_on(probe_environment())
        .expect("probe_environment should succeed against the throwaway X11 display");

    // Assert before relying on the guard's Drop; if any assertion fails the
    // guard still tears the display down during unwind.
    assert_eq!(
        environment.session_kind,
        SessionKind::X11,
        "XDG_SESSION_TYPE=x11 with DISPLAY={display} must select the X11 session lane even though \
         the /proc compositor scan still sees the host compositor (compositor={:?})",
        environment.compositor
    );
    assert_eq!(
        environment.capture_backend,
        CaptureBackendKind::X11,
        "the X11 session lane must select X11 capture"
    );
    assert_eq!(
        environment.input_backend,
        InputBackendKind::XTest,
        "the X11 session lane must select XTest input"
    );

    // Explicit teardown so success leaves no orphan server or lock; the guard's
    // Drop is the panic-path backstop.
    guard.teardown();
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

fn command_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
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
) -> std::io::Result<DisplayGuard> {
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
            Ok(DisplayGuard {
                provider,
                display: display.to_string(),
                number,
                child: None,
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
            Ok(DisplayGuard {
                provider,
                display: display.to_string(),
                number,
                child: Some(child),
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
        let ready = Command::new("xdpyinfo")
            .arg("-display")
            .arg(display)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if ready {
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

fn set_var(name: &str, value: impl AsRef<std::ffi::OsStr>) {
    // SAFETY: serial-integration group + per-process nextest isolation; no
    // concurrent env access.
    unsafe { env::set_var(name, value.as_ref()) }
}

fn remove_var(name: &str) {
    // SAFETY: see `set_var`.
    unsafe { env::remove_var(name) }
}
