//! Lifecycle management for the private xpra/Openbox desktop the computer-use
//! agent drives in isolated mode.
//!
//! The client owns the sandbox: it idempotently brings up a headless
//! `xpra start-desktop` display, describes it (display number, settled
//! geometry, private session-bus address, isolated daemon socket), exposes the
//! environment the isolated daemon must run under, optionally launches a
//! read-only viewer on the user's real screen, and tears the session down.
//!
//! The daemon stays ignorant of xpra (see the ExecPlan Decision Log): it simply
//! probes the sanitized environment and selects the X11 lane. Everything
//! xpra-specific lives here.
//!
//! The D-Bus sandbox bus is obtained from xpra's own default `--dbus-launch`:
//! `xpra info :N` reports the private bus address under the field
//! `dbus.env.DBUS_SESSION_BUS_ADDRESS` (confirmed by the Milestone 1b spike on
//! xpra 6.4.4). A client-owned `dbus-daemon` fallback exists for hosts where
//! that field is absent.

#[cfg(target_os = "linux")]
mod atspi;
mod lifecycle_lease;
mod owned_bus;
mod probe;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use sky_cua_platform::config::{
    CODEX_BROWSER_SOCKET_PATH_ENV, Lifecycle, ResolvedIsolatedDesktop, ViewerMode,
};

// The client-owned sandbox-bus fallback (spawn/persist/recover/reap) lives in its
// own submodule; the parent drives it from `ensure`/`stop`.
#[cfg(target_os = "linux")]
use atspi::{ensure_session as ensure_atspi_session, terminate_session as terminate_atspi_session};
use lifecycle_lease::IsolatedDesktopLifecycleLease;
use owned_bus::{
    persist_owned_bus, reap_persisted_owned_bus, recover_owned_bus, start_owned_session_bus,
};
// Re-export the pure parser/probe surface so the parent module references it
// unqualified and so the public-API types keep their existing visibility.
pub use probe::{DisplayGeometry, IsolatedDesktopDependencies};
use probe::{
    XPRA_INFO_DBUS_ADDRESS_KEY, XPRA_INFO_XAUTHORITY_KEY, ensure_dependencies,
    first_free_display_number, parse_display_number, parse_resolution, parse_xdpyinfo_dimensions,
    parse_xpra_info_dbus_address, parse_xpra_info_xauthority, read_x_lock_names,
    xpra_list_has_live_display,
};

/// Bounded poll budget waiting for `xdpyinfo` to reach a freshly started
/// display. xpra's daemon-mode startup is sub-second on the development host;
/// this leaves generous headroom on slower hosts without a fixed sleep.
const DISPLAY_READY_TIMEOUT: Duration = Duration::from_secs(20);
/// Interval between `xdpyinfo` reachability probes.
const DISPLAY_READY_POLL_INTERVAL: Duration = Duration::from_millis(150);
/// Settle budget after the display is first reachable. A `start-desktop`
/// display reports a transient screen mode at startup before settling on the
/// requested geometry (spike finding); geometry is read only after this.
const DISPLAY_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
/// Interval between geometry-stability probes while the display settles.
const DISPLAY_SETTLE_POLL_INTERVAL: Duration = Duration::from_millis(150);
/// The settled geometry must report the same dimensions across this many
/// consecutive probes before it is accepted, so the transient startup mode
/// cannot be mistaken for the final geometry.
const DISPLAY_SETTLE_STABLE_READS: usize = 3;

/// A live handle to the private xpra desktop. Constructed by [`ensure`], it
/// records everything the spine needs to spawn a sandboxed daemon and watch the
/// desktop.
///
/// [`ensure`]: IsolatedDesktopHandle::ensure
#[derive(Debug)]
pub struct IsolatedDesktopHandle {
    /// The X11 display string, e.g. `":100"`.
    display: String,
    /// The settled virtual-display geometry.
    geometry: DisplayGeometry,
    /// The sandbox `DBUS_SESSION_BUS_ADDRESS` the isolated daemon runs under.
    dbus_address: String,
    /// The Xauthority file used by xpra's Xorg server.
    xauthority: String,
    /// The isolated daemon's IPC socket path
    /// (`$XDG_RUNTIME_DIR/sky-cua/service-isolated-<N>.sock`).
    socket_path: PathBuf,
    /// Whether this handle started the client-owned sandbox `dbus-daemon` (the
    /// xpra-provided bus is the common case, `false`). The owned bus is a SESSION
    /// resource: its address and pid are recorded under the runtime dir (see
    /// [`persist_owned_bus`]) and it is reaped on session teardown — the ephemeral
    /// shutdown and explicit [`stop`] paths — not when this handle drops, so it
    /// survives a client exit for reuse.
    ///
    /// [`stop`]: IsolatedDesktopHandle::stop
    owns_bus: bool,
}

impl IsolatedDesktopHandle {
    /// Idempotently ensure the private xpra desktop exists for `cfg`, returning
    /// a handle that describes it.
    ///
    /// If a healthy xpra session already owns the target display it is reused;
    /// otherwise a fresh `xpra start-desktop` is launched with the spike-proven
    /// flag set and the requested resolution. A `display` of `"auto"` scans for
    /// a free display number, persisting the choice to
    /// `$XDG_RUNTIME_DIR/sky-cua/isolated-display` so later calls reuse it.
    pub fn ensure(cfg: &ResolvedIsolatedDesktop) -> Result<Self> {
        // Name the missing dependency up front so the fail-closed error in the
        // spine is actionable, instead of a generic spawn failure surfacing from
        // deep inside `start_xpra_desktop`/`xpra_session_is_healthy`.
        ensure_dependencies()?;

        let display = resolve_display(&cfg.display)?;
        let display_number = parse_display_number(&display)
            .ok_or_else(|| anyhow!("invalid isolated desktop display string: {display}"))?;
        let _lifecycle_lease = IsolatedDesktopLifecycleLease::acquire(display_number)?;
        let socket_path = isolated_socket_path(display_number)?;

        // Size the virtual display. An explicit `"<w>x<h>"` is used verbatim;
        // `"auto"` (the default) becomes three-quarters of the largest connected
        // monitor so the read-only viewer is a comfortable window on the real
        // screen. The parsed geometry lets the settle loop wait for the display to
        // reach that mode rather than the transient startup mode xpra reports
        // first (spike finding).
        let resolution = resolve_resolution(&cfg.resolution);
        let expected = parse_resolution(&resolution);

        if xpra_session_is_healthy(&display)? {
            // Recover the sandbox bus the reused session is on. Prefer the address
            // xpra recorded; otherwise a session brought up earlier with the
            // client-owned-bus fallback persisted its address, so reuse that bus if
            // its `dbus-daemon` is still alive. Erroring (rather than guessing) is
            // the safe floor: an unset bus would let the daemon fall back to the
            // user's real session bus.
            let (xpra_dbus_address, xauthority) = xpra_info_environment(&display)?;
            let dbus_address = match xpra_dbus_address {
                Some(address) => address,
                None => recover_owned_bus(display_number).ok_or_else(|| {
                    anyhow!(
                        "reused xpra session {display} reports no sandbox \
                         {XPRA_INFO_DBUS_ADDRESS_KEY} and no live client-owned sandbox \
                         bus was found to reuse"
                    )
                })?,
            };
            // A reused session is already settled at its live mode; pass `None`
            // (as `status()` does) so the settle budget is not spun waiting for it
            // to re-reach `cfg.resolution` when the live mode and config differ.
            let geometry = read_settled_geometry(&display, None)?;
            #[cfg(target_os = "linux")]
            if let Err(error) =
                ensure_atspi_session(&display, &dbus_address, &xauthority, display_number)
                    .and_then(|()| verify_xpra_after_atspi_bootstrap(&display))
            {
                terminate_atspi_session(display_number);
                return Err(error).with_context(|| {
                    format!(
                        "failed to bootstrap the private AT-SPI registry for reused xpra session {display}"
                    )
                });
            }
            // This handle reuses, but does not own, the bus: reaping stays with
            // session teardown via the persisted record.
            return Ok(Self {
                display,
                geometry,
                dbus_address,
                xauthority,
                socket_path,
                owns_bus: false,
            });
        }

        start_xpra_desktop(&display, &resolution, cfg)?;

        // The session is now up. If any step that finishes bringing it online
        // fails, the just-started xpra session (and its `/tmp/.X<N>-lock`) would
        // be orphaned with no handle to reap it — the exact host residue this
        // feature exists to avoid. Tear it back down before propagating the error.
        let (geometry, dbus_address, xauthority, owns_bus, bus_child) =
            match finish_session_bringup(&display, expected) {
                Ok(values) => values,
                Err(error) => {
                    let _ = stop_xpra_desktop(&display);
                    remove_stale_display_lock(&display);
                    return Err(error);
                }
            };

        // A client-owned sandbox bus uses a non-deterministic address held only in
        // this process. Persist its address and pid before AT-SPI bootstrap so a
        // failed bootstrap can reap it through the normal session-owned path. The
        // xpra-provided bus needs no record — it is re-read from `xpra info` on
        // reuse.
        if owns_bus && let Some(pid) = bus_child.as_ref().map(Child::id) {
            persist_owned_bus(display_number, &dbus_address, pid);
        }

        // AT-SPI is isolated-desktop readiness, not a daemon-side repair. Bring
        // up org.a11y.Bus and a responsive registry on the private session bus
        // before returning a handle, so neither the service nor an application it
        // launches can race an uninitialized accessibility session. The host
        // repair path deliberately rejects this non-host bus.
        #[cfg(target_os = "linux")]
        if let Err(error) =
            ensure_atspi_session(&display, &dbus_address, &xauthority, display_number)
                .and_then(|()| verify_xpra_after_atspi_bootstrap(&display))
        {
            terminate_atspi_session(display_number);
            if owns_bus {
                reap_persisted_owned_bus(display_number);
                if let Some(mut child) = bus_child {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            let _ = stop_xpra_desktop(&display);
            remove_stale_display_lock(&display);
            return Err(error).with_context(|| {
                format!(
                    "failed to bootstrap the private AT-SPI registry for xpra session {display}"
                )
            });
        }

        // Dropping `bus_child` here does NOT kill the daemon (std `Child` does not
        // kill on drop); the bus survives as a session-scoped process, reaped on
        // teardown via the persisted record.
        drop(bus_child);

        Ok(Self {
            display,
            geometry,
            dbus_address,
            xauthority,
            socket_path,
            owns_bus,
        })
    }

    /// The X11 display string, e.g. `":100"`.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// The settled virtual-display geometry.
    pub fn geometry(&self) -> DisplayGeometry {
        self.geometry
    }

    /// The isolated daemon's IPC socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Whether this handle owns a client-started `dbus-daemon`.
    pub fn owns_bus(&self) -> bool {
        self.owns_bus
    }

    /// Environment variables to **set** on the isolated daemon so it (and every
    /// helper it spawns) lands on the private X11 desktop with the private
    /// session bus and the isolated socket.
    pub fn spawn_env(&self) -> Vec<(String, String)> {
        build_spawn_env(
            &self.display,
            &self.dbus_address,
            &self.xauthority,
            &self.socket_path,
        )
    }

    /// Environment variables to **remove** on the isolated daemon. Clearing
    /// `WAYLAND_DISPLAY` keeps toolkit apps from preferring Wayland and escaping
    /// the sandbox; clearing `AT_SPI_BUS_ADDRESS` prevents an inherited host-bus
    /// override from bypassing discovery through the private session bus.
    pub fn removed_env(&self) -> Vec<&'static str> {
        removed_env()
    }

    /// Launch the configured read-only viewer.
    ///
    /// [`ViewerMode::Attach`] spawns `xpra attach :N --readonly` using the
    /// client's own (user-session) environment so the viewer window renders on
    /// the user's real screen. [`ViewerMode::Html5`] starts the xpra HTML5
    /// listener and logs the URL. [`ViewerMode::None`] is a no-op.
    pub fn launch_viewer(&self, mode: ViewerMode) -> Result<()> {
        match mode {
            ViewerMode::Attach => {
                // Inherit the client's (user-session) environment so the viewer
                // window appears on the user's real screen, not inside the
                // sandbox.
                Command::new("xpra")
                    .arg("attach")
                    .arg(&self.display)
                    .arg("--readonly")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .with_context(|| {
                        format!(
                            "failed to attach a read-only viewer to xpra {}",
                            self.display
                        )
                    })?;
                Ok(())
            }
            ViewerMode::Html5 => {
                // The HTML5 listener is bound on the existing session; the bind
                // address is logged so an operator can open it.
                let output = Command::new("xpra")
                    .arg("html5")
                    .arg(&self.display)
                    .stdin(Stdio::null())
                    .output()
                    .with_context(|| {
                        format!(
                            "failed to start the xpra HTML5 listener for {}",
                            self.display
                        )
                    })?;
                let url = String::from_utf8_lossy(&output.stdout);
                let url = url.trim();
                if url.is_empty() {
                    tracing::info!(
                        xpra_display = %self.display,
                        "started the xpra HTML5 listener (no URL reported)"
                    );
                } else {
                    tracing::info!(
                        xpra_display = %self.display,
                        url = %url,
                        "xpra HTML5 viewer URL"
                    );
                }
                Ok(())
            }
            ViewerMode::None => Ok(()),
        }
    }

    /// Tear the private desktop down: stop the xpra session, reap the sandbox
    /// `dbus-daemon` if client-owned, and remove a stale `/tmp/.X<N>-lock`.
    ///
    /// Teardown filters strictly by the known display number so it can never
    /// reap the user's real session.
    pub fn stop(&self) -> Result<()> {
        if let Some(number) = parse_display_number(&self.display) {
            let _lifecycle_lease = IsolatedDesktopLifecycleLease::acquire(number)?;
            // Verify and terminate the exact private AT-SPI owners while their
            // buses are still reachable. This is a no-op for older sessions that
            // have no persisted AT-SPI ownership record.
            #[cfg(target_os = "linux")]
            terminate_atspi_session(number);
            return stop_locked(&self.display, number, &self.socket_path);
        }
        stop_xpra_desktop(&self.display)
    }
}

// `IsolatedDesktopHandle` has no `Drop`: the client-owned sandbox bus is a SESSION
// resource, not a client one. Reaping it on drop would kill the bus when a
// persistent-session client exits, leaving the still-running session bus-less and
// un-reusable. The bus is reaped only on session teardown — the ephemeral
// shutdown path and the explicit `stop` (free function, via the persisted record)
// — so it survives across client restarts for reuse.

/// Read-only status of the configured isolated desktop. Reports the resolved
/// selection (enabled/viewer/lifecycle), whether the target display is currently
/// up and its geometry, and the presence of the required dependencies — all as
/// structured fields so a client learns the truth without parsing prose. Does
/// not start anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedDesktopStatus {
    /// Whether isolated mode is enabled by the resolved configuration.
    pub enabled: bool,
    /// The resolved display string the status was probed against.
    pub display: String,
    /// Whether a healthy xpra session currently owns the display.
    pub up: bool,
    /// The settled geometry, present only when the display is up.
    pub geometry: Option<DisplayGeometry>,
    /// The configured read-only viewer mode.
    pub viewer: ViewerMode,
    /// The configured session lifecycle.
    pub lifecycle: Lifecycle,
    /// Presence of the external binaries isolated mode depends on.
    pub dependencies: IsolatedDesktopDependencies,
}

/// Probe the configured isolated desktop without starting it. Resolves the
/// display (honoring a persisted `auto` choice) and reports the structured
/// selection, liveness/geometry, and dependency presence. The dependency probe
/// is always run so the status is honest even when the display is down because a
/// binary is missing.
pub fn status(cfg: &ResolvedIsolatedDesktop) -> Result<IsolatedDesktopStatus> {
    let display = resolve_display(&cfg.display)?;
    let dependencies = IsolatedDesktopDependencies::probe();
    // Only probe liveness when xpra itself is present; without it `xpra list`
    // would surface a spawn error rather than a clean "down" status.
    let (up, geometry) = if dependencies.xpra && xpra_session_is_healthy(&display)? {
        // A live session has already settled; no expected target is needed.
        (true, Some(read_settled_geometry(&display, None)?))
    } else {
        (false, None)
    };
    Ok(IsolatedDesktopStatus {
        enabled: cfg.enabled,
        display,
        up,
        geometry,
        viewer: cfg.viewer,
        lifecycle: cfg.lifecycle,
        dependencies,
    })
}

/// Tear down the isolated desktop for `cfg` without first bringing it up.
/// Resolves the configured display, stops the xpra session, reaps the dedicated
/// isolated daemon and a persisted client-owned sandbox bus, and removes a stale
/// `/tmp/.X<N>-lock` plus the daemon socket. Filters strictly by the resolved
/// display number, so it can never touch the user's real session.
pub fn stop(cfg: &ResolvedIsolatedDesktop) -> Result<String> {
    let display = resolve_display(&cfg.display)?;
    if let Some(number) = parse_display_number(&display) {
        let _lifecycle_lease = IsolatedDesktopLifecycleLease::acquire(number)?;
        #[cfg(target_os = "linux")]
        terminate_atspi_session(number);
        let socket_path = isolated_socket_path(number)?;
        stop_locked(&display, number, &socket_path)?;
        return Ok(display);
    }
    stop_xpra_desktop(&display)?;
    remove_stale_display_lock(&display);
    Ok(display)
}

/// Finish display teardown while the caller holds the per-display lifecycle
/// lease. AT-SPI termination intentionally happens in the caller first, while
/// both private buses are reachable.
fn stop_locked(display: &str, display_number: u32, socket_path: &Path) -> Result<()> {
    stop_xpra_desktop(display)?;
    reap_persisted_owned_bus(display_number);
    terminate_isolated_daemon(socket_path);
    remove_stale_display_lock(display);
    Ok(())
}

#[cfg(target_os = "linux")]
fn verify_xpra_after_atspi_bootstrap(display: &str) -> Result<()> {
    if xpra_session_is_healthy(display)? {
        Ok(())
    } else {
        bail!("xpra session {display} stopped during private AT-SPI bootstrap")
    }
}

/// Build the daemon spawn environment for the given display, sandbox bus, and
/// isolated socket. Pure so it can be unit-tested without a real xpra.
fn build_spawn_env(
    display: &str,
    dbus_address: &str,
    xauthority: &str,
    socket_path: &Path,
) -> Vec<(String, String)> {
    let codex_socket_path = socket_path.with_extension("codex-browser.sock");
    vec![
        ("DISPLAY".to_string(), display.to_string()),
        ("XDG_SESSION_TYPE".to_string(), "x11".to_string()),
        ("QT_QPA_PLATFORM".to_string(), "xcb".to_string()),
        ("GDK_BACKEND".to_string(), "x11".to_string()),
        ("NO_AT_BRIDGE".to_string(), "0".to_string()),
        ("ACCESSIBILITY_ENABLED".to_string(), "1".to_string()),
        (
            "DBUS_SESSION_BUS_ADDRESS".to_string(),
            dbus_address.to_string(),
        ),
        ("XAUTHORITY".to_string(), xauthority.to_string()),
        (
            "SKY_CUA_SERVICE_SOCKET_PATH".to_string(),
            socket_path.to_string_lossy().into_owned(),
        ),
        (
            CODEX_BROWSER_SOCKET_PATH_ENV.to_string(),
            codex_socket_path.to_string_lossy().into_owned(),
        ),
    ]
}

/// Environment variables to remove on the isolated daemon.
fn removed_env() -> Vec<&'static str> {
    vec!["WAYLAND_DISPLAY", "AT_SPI_BUS_ADDRESS"]
}

/// Resolve a configured display string to a concrete `":N"`. The literal
/// `"auto"` scans for a free display number and persists the choice so later
/// calls reuse it.
fn resolve_display(configured: &str) -> Result<String> {
    if configured != "auto" {
        return Ok(configured.to_string());
    }

    // Reuse a previously persisted choice only when it is still ours to use:
    // free (no `/tmp/.X<N>-lock`, so we can start xpra there) or already a live
    // xpra session (which we reuse). If a foreign server now owns the persisted
    // number, fall through to a fresh scan rather than colliding with it or
    // silently adopting a session we did not create.
    if let Some(path) = isolated_display_state_path()
        && let Ok(raw) = std::fs::read_to_string(&path)
        && let Some(number) = parse_display_number(raw.trim())
    {
        let display = format!(":{number}");
        // Reuse the persisted number when a live `xpra` session is on it (our own
        // session, by the number we persisted), or when it is genuinely free (no
        // live server and no stale `/tmp/.X<N>-lock`) so we can restart there. A
        // reachable *non-xpra* server, or a number held with no lock, falls through
        // to a fresh scan rather than colliding with it. We deliberately do NOT
        // require a stricter ownership probe here: a live xpra session on the number
        // we persisted is treated as ours. (A foreign xpra adopting our exact number
        // after ours died is possible but rare; accepting that is far better than
        // false-negativing our own healthy session and spawning a duplicate desktop,
        // which a bus-based ownership check did.)
        let reuse = if xdpyinfo_reachable(&display) {
            xpra_session_is_live(&display)?
        } else {
            !PathBuf::from(format!("/tmp/.X{number}-lock")).exists()
        };
        if reuse {
            return Ok(display);
        }
    }

    let lock_names = read_x_lock_names()?;
    let number = first_available_display_number(&lock_names);
    let display = format!(":{number}");

    if let Some(path) = isolated_display_state_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Persisting is best-effort: a failure to write only costs a fresh scan
        // next time, never correctness.
        let _ = std::fs::write(&path, format!("{number}\n"));
    }

    Ok(display)
}

/// Like [`first_free_display_number`] but also skips any display that is
/// actually reachable, not only those with a `/tmp/.X<N>-lock`. A live X server
/// can hold a display with no such lock (an abstract X socket, or a relocated
/// `TMPDIR`); scanning lock files alone could otherwise hand back an occupied
/// number that `xpra start-desktop` would then fail to bind.
fn first_available_display_number(lock_names: &BTreeSet<String>) -> u32 {
    let mut number = first_free_display_number(lock_names);
    while lock_names.contains(&format!(".X{number}-lock"))
        || xdpyinfo_reachable(&format!(":{number}"))
    {
        number += 1;
    }
    number
}

/// `$XDG_RUNTIME_DIR/sky-cua/isolated-display`, where the chosen `auto` display
/// number is persisted, or `None` when no runtime dir is available.
fn isolated_display_state_path() -> Option<PathBuf> {
    sky_cua_runtime_dir().map(|dir| dir.join("isolated-display"))
}

/// The isolated daemon socket path for display number `N`:
/// `$XDG_RUNTIME_DIR/sky-cua/service-isolated-<N>.sock`.
fn isolated_socket_path(display_number: u32) -> Result<PathBuf> {
    let dir = sky_cua_runtime_dir().ok_or_else(|| {
        anyhow!(
            "cannot resolve an isolated desktop socket path because XDG_RUNTIME_DIR \
             is not set"
        )
    })?;
    Ok(dir.join(format!("service-isolated-{display_number}.sock")))
}

/// `$XDG_RUNTIME_DIR/sky-cua`, or `None` when `XDG_RUNTIME_DIR` is unset/empty.
fn sky_cua_runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(|dir| PathBuf::from(dir).join("sky-cua"))
}

/// Geometry used when `resolution = "auto"` but no monitor can be probed (no
/// reachable X/XWayland display to query with `xrandr`).
const RESOLUTION_FALLBACK: &str = "1920x1080";

/// Resolve a configured resolution to a concrete `"<width>x<height>"`. An explicit
/// value is used verbatim; the literal `"auto"` becomes three-quarters of the
/// largest connected monitor (so the read-only viewer is a comfortable window on
/// the user's real screen), falling back to [`RESOLUTION_FALLBACK`] when no
/// monitor can be probed.
fn resolve_resolution(configured: &str) -> String {
    if configured != "auto" {
        return configured.to_string();
    }
    largest_monitor_dimensions()
        .map(|(width, height)| {
            let (width, height) = three_quarter_even(width, height);
            format!("{width}x{height}")
        })
        .unwrap_or_else(|| RESOLUTION_FALLBACK.to_string())
}

/// Three-quarter scale, floored to even dimensions (some X servers reject odd
/// virtual-display modes).
fn three_quarter_even(width: u32, height: u32) -> (u32, u32) {
    let scaled = |value: u32| {
        let v = value * 3 / 4;
        v - (v % 2)
    };
    (scaled(width), scaled(height))
}

/// The largest connected monitor's pixel dimensions, via `xrandr --current` on
/// the client's (user-session) display. `None` when `xrandr` is unavailable or no
/// monitor is connected; the client then uses [`RESOLUTION_FALLBACK`].
fn largest_monitor_dimensions() -> Option<(u32, u32)> {
    let output = Command::new("xrandr").arg("--current").output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_largest_connected_mode(&String::from_utf8_lossy(&output.stdout))
}

/// Largest connected-output mode from `xrandr --current` output, by pixel area.
/// Pure over the captured text so it is testable without a real X server. Lines
/// whose second token is not `connected` (headers, `disconnected` outputs, mode
/// rows) are ignored.
fn parse_largest_connected_mode(output: &str) -> Option<(u32, u32)> {
    output
        .lines()
        .filter(|line| line.split_whitespace().nth(1) == Some("connected"))
        .filter_map(|line| {
            line.split_whitespace().find_map(|token| {
                let (width, height) = token.split('+').next()?.split_once('x')?;
                Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
            })
        })
        .max_by_key(|(width, height)| u64::from(*width) * u64::from(*height))
}

/// Start a fresh `xpra start-desktop` session with the spike-proven flag set
/// and the resolved resolution applied to the virtual display.
fn start_xpra_desktop(
    display: &str,
    resolution: &str,
    cfg: &ResolvedIsolatedDesktop,
) -> Result<()> {
    let status = Command::new("xpra")
        .arg("start-desktop")
        .arg(display)
        .arg(format!("--start={}", cfg.window_manager))
        .arg(format!("--resize-display={resolution}"))
        .arg("--daemon=yes")
        .arg("--notifications=no")
        .arg("--bell=no")
        .arg("--webcam=no")
        .arg("--pulseaudio=no")
        .arg("--mdns=no")
        .arg("--start-new-commands=no")
        .arg("--systemd-run=no")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to launch xpra start-desktop on {display}"))?;
    if !status.success() {
        bail!("xpra start-desktop on {display} exited with {status}");
    }
    Ok(())
}

/// Finish bringing a just-started xpra session online: wait for the display to
/// become reachable, read its settled geometry, and resolve the sandbox session
/// bus (preferring xpra's own `--dbus-launch`, falling back to a client-owned
/// `dbus-daemon`). Split out of [`IsolatedDesktopHandle::ensure`] so a failure
/// here can tear the just-started session back down instead of leaking it.
fn finish_session_bringup(
    display: &str,
    expected: Option<DisplayGeometry>,
) -> Result<(DisplayGeometry, String, String, bool, Option<Child>)> {
    wait_for_display(display)?;
    let geometry = read_settled_geometry(display, expected)?;
    // Primary path: xpra's own default dbus launch records the sandbox bus
    // address in `xpra info`. Fall back to a client-owned dbus-daemon only if
    // xpra did not provide one.
    let (xpra_dbus_address, xauthority) = xpra_info_environment(display)?;
    let (dbus_address, owns_bus, bus_child) = match xpra_dbus_address {
        Some(address) => (address, false, None),
        None => {
            let (address, child) = start_owned_session_bus().context(
                "xpra did not report a sandbox session bus and a client-owned \
                 dbus-daemon could not be started",
            )?;
            (address, true, Some(child))
        }
    };
    Ok((geometry, dbus_address, xauthority, owns_bus, bus_child))
}

/// Bounded-poll `xdpyinfo` until the display is reachable. No fixed sleep.
fn wait_for_display(display: &str) -> Result<()> {
    let deadline = Instant::now() + DISPLAY_READY_TIMEOUT;
    loop {
        if xdpyinfo_reachable(display) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "xpra display {display} did not become reachable within {:?}",
                DISPLAY_READY_TIMEOUT
            );
        }
        std::thread::sleep(DISPLAY_READY_POLL_INTERVAL);
    }
}

/// Whether `xdpyinfo -display :N` succeeds.
fn xdpyinfo_reachable(display: &str) -> bool {
    Command::new("xdpyinfo")
        .arg("-display")
        .arg(display)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Read the settled geometry. A `start-desktop` display reports a transient
/// mode at startup before settling on the requested resolution (spike finding:
/// `2048x1536` was observed before `1920x1080`), and that transient mode can be
/// reported stably for several reads. When `expected` is known (parsed from the
/// requested resolution) the loop waits for the display to actually reach it;
/// otherwise it falls back to accepting a value that repeats across
/// [`DISPLAY_SETTLE_STABLE_READS`] consecutive probes. In either case the most
/// recent reading is returned once the settle budget elapses.
fn read_settled_geometry(
    display: &str,
    expected: Option<DisplayGeometry>,
) -> Result<DisplayGeometry> {
    let deadline = Instant::now() + DISPLAY_SETTLE_TIMEOUT;
    let mut last: Option<DisplayGeometry> = None;
    let mut stable = 0usize;
    loop {
        let raw = xdpyinfo_dimensions(display)?;
        let geometry = parse_xdpyinfo_dimensions(&raw).ok_or_else(|| {
            anyhow!("could not parse a display geometry from xdpyinfo on {display}")
        })?;
        // The requested mode is authoritative: as soon as the display reports
        // it, the transient startup mode is behind us.
        if expected == Some(geometry) {
            return Ok(geometry);
        }
        if last == Some(geometry) {
            stable += 1;
            // Only accept a stable-but-unexpected reading when no specific mode
            // was requested; otherwise keep waiting for the requested mode.
            if expected.is_none() && stable >= DISPLAY_SETTLE_STABLE_READS {
                return Ok(geometry);
            }
        } else {
            stable = 1;
            last = Some(geometry);
        }
        if Instant::now() >= deadline {
            // The settle budget elapsed; trust the most recent reading rather
            // than failing — the display is reachable and reporting a size.
            return Ok(geometry);
        }
        std::thread::sleep(DISPLAY_SETTLE_POLL_INTERVAL);
    }
}

/// Run `xdpyinfo -display :N` and return its stdout.
fn xdpyinfo_dimensions(display: &str) -> Result<String> {
    let output = Command::new("xdpyinfo")
        .arg("-display")
        .arg(display)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to read xdpyinfo geometry on {display}"))?;
    if !output.status.success() {
        bail!("xdpyinfo on {display} exited with {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether a healthy xpra session currently owns `display`.
///
/// `xpra list` is consulted first (it is cheap and explicit about live vs dead
/// sessions); the display is then confirmed reachable so a `LIVE` listing for a
/// half-dead server does not yield a false positive.
fn xpra_session_is_healthy(display: &str) -> Result<bool> {
    let listed = xpra_list_has_live_display(&xpra_list_output()?, display);
    if !listed {
        return Ok(false);
    }
    Ok(xdpyinfo_reachable(display))
}

/// Whether the already-reachable `display` hosts a live `xpra` session (as
/// opposed to a foreign non-xpra X server occupying the number). A reused session
/// is identified by our persisted display number plus liveness, deliberately NOT
/// by whether it reports a private D-Bus bus: that bus is launched asynchronously
/// by xpra's `--dbus-launch` child and is absent entirely on owned-bus-fallback
/// sessions, so a bus-based check would false-negative one of our own healthy
/// sessions and make `resolve_display` spawn a SECOND desktop. The caller probes
/// reachability first, so this does not re-run `xdpyinfo`.
fn xpra_session_is_live(display: &str) -> Result<bool> {
    Ok(xpra_list_has_live_display(&xpra_list_output()?, display))
}

/// Capture `xpra list` stdout, tolerating a non-zero exit (xpra returns a
/// non-success status when there are no sessions on some versions).
fn xpra_list_output() -> Result<String> {
    let output = Command::new("xpra")
        .arg("list")
        .stdin(Stdio::null())
        .output()
        .context("failed to run xpra list")?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

/// Read the sandbox session-bus address and Xauthority file xpra recorded for
/// `display`. The authority is required because detached launch setup clears
/// inherited graphical credentials before applying this isolated environment.
fn xpra_info_environment(display: &str) -> Result<(Option<String>, String)> {
    let output = Command::new("xpra")
        .arg("info")
        .arg(display)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run xpra info on {display}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let xauthority = parse_xpra_info_xauthority(&text).ok_or_else(|| {
        anyhow!("xpra info on {display} did not report a non-empty {XPRA_INFO_XAUTHORITY_KEY}")
    })?;
    Ok((parse_xpra_info_dbus_address(&text), xauthority))
}

/// Stop the xpra session on `display`. A failure to stop (for example because
/// the session already exited) is downgraded to a warning so teardown remains
/// idempotent.
fn stop_xpra_desktop(target_display: &str) -> Result<()> {
    let output = Command::new("xpra")
        .arg("stop")
        .arg(target_display)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run xpra stop on {target_display}"))?;
    if !output.status.success() {
        tracing::warn!(
            xpra_display = %target_display,
            xpra_status = %output.status,
            "xpra stop did not report success (the session may already be gone)"
        );
    }
    Ok(())
}

/// Remove a stale `/tmp/.X<N>-lock` left by a crashed server. Filters strictly
/// by the display number so it can never remove the user's real session lock
/// for a different display.
fn remove_stale_display_lock(display: &str) {
    let Some(number) = parse_display_number(display) else {
        return;
    };
    let lock_path = PathBuf::from(format!("/tmp/.X{number}-lock"));
    // Only remove a stale lock if the display is no longer reachable; a
    // still-reachable display means the lock is live and must not be touched.
    if xdpyinfo_reachable(display) {
        return;
    }
    let _ = std::fs::remove_file(&lock_path);
}

/// Best-effort termination of the isolated `sky-cua-service` daemon that owns
/// `socket_path`, plus removal of its stale socket. The daemon records its pid
/// in `<socket>.lock`; this reads that pid, verifies it is a live
/// `sky-cua-service` process (so an unrelated or recycled pid is never
/// signalled — in particular never the user's real daemon on a different
/// socket), and `SIGTERM`s it. Safe to call when no daemon is present. This is
/// what keeps an explicit teardown, or an ephemeral shutdown, from leaving an
/// orphan daemon pointing at the stopped display.
fn terminate_isolated_daemon(socket_path: &Path) {
    if let Ok(Some(pid)) = crate::daemon_singleton::read_owner_pid(socket_path)
        && crate::daemon_singleton::pid_is_sky_cua_service(pid)
    {
        // SAFETY: `kill` with SIGTERM has no Rust-side memory-safety
        // preconditions; the pid was read from the isolated socket's singleton
        // lock and verified to be a live sky-cua-service process.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    // Remove the stale socket so the next daemon binds cleanly, but NEVER the
    // singleton lock file. The daemon (sky-cua-service `ipc_server`) deliberately
    // never unlinks `<socket>.lock`: SIGTERM is asynchronous and we do not wait
    // for exit, so unlinking the lock while the just-signalled daemon still holds
    // its `flock` would let a re-`ensure` acquire a fresh lock on a new inode and
    // bind a SECOND daemon — the exact stomp the singleton guard prevents. The
    // `flock` releases when the daemon exits; the next daemon reuses the lock file.
    let _ = std::fs::remove_file(socket_path);
}

#[cfg(test)]
mod tests;
