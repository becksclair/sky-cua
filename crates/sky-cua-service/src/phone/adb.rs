//! ADB baseline backend.
//!
//! Owns host-tooling probing (`adb` path/version/server status), device
//! enumeration (`adb devices -l`, mDNS), pairing/connect, and shell primitives:
//! `screencap`, `input tap/swipe/text/keyevent`, `wm size`/`wm density`,
//! `dumpsys`, `pm`/`am`, install, and forward. Coordinate control is routed by
//! the phone manager and may deliberately avoid the ADB input primitives.
//! Everything here goes through [`CommandRunner`], never
//! `std::process::Command` directly.
//!
//! The module is split into small, individually test-covered units:
//! - pure parsers ([`parse`]) that turn stable `adb` text output into typed
//!   values and never touch a runner, and
//! - thin async wrappers that build argv, run a command through the seam, and
//!   classify failures into structured diagnostics.
//!
//! The integrator wires the async wrappers into `manager.rs`. Wrappers that are
//! not yet called outside tests keep the
//! `#[cfg_attr(not(test), expect(dead_code))]` idiom the spine uses so non-test
//! builds stay clean.

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneBackendKind, PhoneListDevicesResponse, PhoneSettingsScreen,
    PhoneStatusReport,
};

use super::command::{CommandError, CommandOutput, CommandRunner, resolve_adb_path};

mod install;
mod parse;
mod permissions;

#[cfg(test)]
mod tests;

// Re-export the install/forward primitives so callers reach them as `adb::*`.
// The companion lane consumes `install_replace`/`forward_tcp`/`InstallOutcome`
// for the bootstrap path; `app_install` consumes the single/split/multi-package
// variants. All are wired, so no unused-import expectation is needed.
pub(super) use install::{
    InstallOutcome, forward_tcp, install_multi_package, install_multiple, install_replace,
    install_single, uninstall_package,
};
// Re-export the companion secure-settings enablement surface the companion lane
// consumes as `adb::*` to make a freshly deployed companion immediately usable.
pub(super) use permissions::{
    ACCESSIBILITY_SERVICE_CLASS_SUFFIX, NOTIFICATION_LISTENER_CLASS_SUFFIX, SecureServiceOutcome,
    SecureServiceState, ensure_notification_listener, ensure_secure_list_service,
};
// Re-export the parser surface the sibling `device` lane and the integrator
// consume as `adb::*`. The remaining parsers (`parse_server_status`,
// `classify_device_state`, `AdbDeviceLine`) are reachable to the child `tests`
// module via `super::parse::*`.
pub(super) use parse::{
    DeviceRotation, ForegroundApp, classify_connection_kind, parse_current_focus, parse_devices_l,
    parse_mdns_services, parse_package_list, parse_rotation, parse_version, parse_wm_density,
    parse_wm_size,
};

/// Stable `PhoneAdbNotImplemented` diagnostic. Retained for the ADB lane's own
/// tests (the wired wrappers emit concrete command/exit diagnostics instead, so
/// this is not reached on the live path).
#[cfg_attr(not(test), expect(dead_code))]
pub(super) fn not_implemented_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneAdbNotImplemented".to_string(),
        message: "phone ADB backend is not implemented yet (Phase 1 contract spine)".to_string(),
        details: None,
    }
}

/// Convert a [`CommandError`] into a structured diagnostic, keyed by the error's
/// stable code so clients route on the field, not the prose.
pub(super) fn command_error_diagnostic(
    command_line: &str,
    error: &CommandError,
) -> DiagnosticEntry {
    DiagnosticEntry {
        code: error.code().to_string(),
        message: format!("`{command_line}` could not run: {error}"),
        details: None,
    }
}

/// Diagnostic for an `adb` command that ran but exited non-zero. Stderr is the
/// load-bearing field for adb errors, so it is surfaced (bounded) in `details`.
fn nonzero_exit_diagnostic(command_line: &str, output: &CommandOutput) -> DiagnosticEntry {
    let code = output.status.map_or(-1, |c| c);
    let stderr = output.stderr_string();
    DiagnosticEntry {
        code: "PhoneAdbCommandFailed".to_string(),
        message: format!("`{command_line}` exited with status {code}"),
        details: (!stderr.trim().is_empty()).then(|| bound(stderr.trim(), 600)),
    }
}

/// Bound a string to `max` bytes on a char boundary so diagnostics never carry
/// unbounded device output. Shared with the `install` child module.
pub(in crate::phone::adb) fn bound(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

/// Build the argv for a serial-scoped `adb -s <serial> <args...>` invocation.
/// Shared with the `install` child module.
pub(in crate::phone::adb) fn serial_args<'a>(serial: &'a str, tail: &[&'a str]) -> Vec<&'a str> {
    let mut argv = Vec::with_capacity(tail.len() + 2);
    argv.push("-s");
    argv.push(serial);
    argv.extend_from_slice(tail);
    argv
}

/// Render a `(program, args)` pair as a single command line for diagnostics.
/// Shared with the `install` child module.
pub(in crate::phone::adb) fn command_line(program: &str, args: &[&str]) -> String {
    let mut line = String::from(program);
    for arg in args {
        line.push(' ');
        line.push_str(arg);
    }
    line
}

// ===========================================================================
// Host status / device enumeration
// ===========================================================================

/// Host-tooling readiness for `phone_status`.
///
/// Runs `adb version` (availability + version), then probes server reachability
/// with `adb devices`. There is no `adb server-status` subcommand; `adb devices`
/// implicitly starts the server if it is not already running and succeeds when
/// the server is reachable, so its exit status is what `adb_server_running`
/// reports. `adb_available` is true only when `adb version` actually succeeds; a
/// missing binary becomes a structured `PhoneCommandSpawnFailed` diagnostic and
/// leaves availability false.
///
/// The manager threads its configured adb path through
/// [`probe_host_with_path`]; this no-config convenience wrapper is retained for
/// the ADB lane's own tests.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) async fn probe_host(
    runner: &dyn CommandRunner,
    enabled: bool,
    companion_enabled: bool,
) -> PhoneStatusReport {
    probe_host_with_path(runner, None, enabled, companion_enabled).await
}

/// Host-tooling readiness with an explicit configured adb path (from
/// `PhoneConfig.adb_path`). This is the wired entry point the manager calls,
/// threading its resolved selection in; [`probe_host`] is the no-config wrapper
/// retained for the ADB lane's own tests.
pub(super) async fn probe_host_with_path(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    enabled: bool,
    companion_enabled: bool,
) -> PhoneStatusReport {
    let adb = resolve_adb_path(configured_adb_path);
    let mut diagnostics = Vec::new();

    let mut adb_available = false;
    let mut adb_version = None;
    match runner.run(&adb, &["version"]).await {
        Ok(output) if output.success() => {
            adb_available = true;
            adb_version = parse_version(&output.stdout_string());
        }
        Ok(output) => diagnostics.push(nonzero_exit_diagnostic(
            &command_line(&adb, &["version"]),
            &output,
        )),
        Err(error) => diagnostics.push(command_error_diagnostic(
            &command_line(&adb, &["version"]),
            &error,
        )),
    }

    // Server reachability is only meaningful when adb itself is present. `adb
    // devices` doubles as the probe: it auto-starts the server if needed and
    // succeeds when the server is reachable.
    let mut adb_server_running = None;
    let mut mdns_available = false;
    if adb_available {
        match runner.run(&adb, &["devices"]).await {
            Ok(output) => adb_server_running = Some(output.success()),
            Err(error) => diagnostics.push(command_error_diagnostic(
                &command_line(&adb, &["devices"]),
                &error,
            )),
        }
        if let Ok(output) = runner.run(&adb, &["mdns", "check"]).await {
            mdns_available = output.success() && parse_mdns_ready(&output.stdout_string());
        }
    }

    PhoneStatusReport {
        enabled,
        adb_available,
        adb_path: adb_available.then(|| adb.clone()),
        adb_version,
        adb_server_running,
        scrcpy_available: false,
        scrcpy_path: None,
        scrcpy_version: None,
        companion_enabled,
        mdns_available,
        default_serial: None,
        default_backend: PhoneBackendKind::None,
        sessions: Vec::new(),
        devices: Vec::new(),
        diagnostics,
    }
}

/// `adb mdns check` reports "mdns daemon version ..." when the backend is
/// available, or "ERROR: ..." / "unknown command" otherwise.
fn parse_mdns_ready(stdout: &str) -> bool {
    let lower = stdout.to_ascii_lowercase();
    lower.contains("mdns daemon version") || lower.contains("mdns backend")
}

/// Device enumeration for `phone_list_devices`.
///
/// Parses `adb devices -l` into typed [`PhoneDevice`]s and, when `include_mdns`
/// is set and adb exposes it, augments diagnostics with `adb mdns services`
/// readiness. Availability/version of adb itself is reported so the client can
/// distinguish "adb missing" from "no devices".
///
/// The manager threads its configured adb path through
/// [`list_devices_with_path`]; this no-config convenience wrapper is retained for
/// the ADB lane's own tests.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) async fn list_devices(
    runner: &dyn CommandRunner,
    include_mdns: bool,
) -> PhoneListDevicesResponse {
    list_devices_with_path(runner, None, include_mdns).await
}

/// Device enumeration with an explicit configured adb path.
pub(super) async fn list_devices_with_path(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    include_mdns: bool,
) -> PhoneListDevicesResponse {
    let adb = resolve_adb_path(configured_adb_path);
    let mut diagnostics = Vec::new();
    let mut devices = Vec::new();
    let mut adb_path = None;
    let mut adb_version = None;

    match runner.run(&adb, &["version"]).await {
        Ok(output) if output.success() => {
            adb_path = Some(adb.clone());
            adb_version = parse_version(&output.stdout_string());
        }
        Ok(output) => diagnostics.push(nonzero_exit_diagnostic(
            &command_line(&adb, &["version"]),
            &output,
        )),
        Err(error) => diagnostics.push(command_error_diagnostic(
            &command_line(&adb, &["version"]),
            &error,
        )),
    }

    if adb_path.is_some() {
        match runner.run(&adb, &["devices", "-l"]).await {
            Ok(output) if output.success() => {
                for line in parse_devices_l(&output.stdout_string()) {
                    devices.push(line.into_device());
                }
            }
            Ok(output) => diagnostics.push(nonzero_exit_diagnostic(
                &command_line(&adb, &["devices", "-l"]),
                &output,
            )),
            Err(error) => diagnostics.push(command_error_diagnostic(
                &command_line(&adb, &["devices", "-l"]),
                &error,
            )),
        }

        if include_mdns {
            match runner.run(&adb, &["mdns", "services"]).await {
                Ok(output) if output.success() => {
                    // mDNS services are diagnostic context; do not synthesize
                    // PhoneDevices from them until a connect proves the target.
                    let services = parse_mdns_services(&output.stdout_string());
                    if !services.is_empty() {
                        diagnostics.push(DiagnosticEntry {
                            code: "PhoneMdnsServices".to_string(),
                            message: format!("{} mDNS service(s) advertised", services.len()),
                            details: None,
                        });
                    }
                }
                Ok(output) => diagnostics.push(nonzero_exit_diagnostic(
                    &command_line(&adb, &["mdns", "services"]),
                    &output,
                )),
                Err(error) => diagnostics.push(command_error_diagnostic(
                    &command_line(&adb, &["mdns", "services"]),
                    &error,
                )),
            }
        }
    }

    PhoneListDevicesResponse {
        devices,
        adb_path,
        adb_version,
        diagnostics,
    }
}

// ===========================================================================
// Pairing / connect / disconnect
// ===========================================================================

/// Outcome of a pairing/connect/disconnect transport operation. Carries no
/// sensitive material (pairing codes are never echoed back).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TransportOutcome {
    pub(super) success: bool,
    /// Bounded, code-free human message from adb's stdout/stderr.
    pub(super) message: String,
}

/// Pair with an Android 11+ wireless-debugging endpoint.
///
/// The `pairing_code` is written to `adb pair host:port` over stdin so it is
/// never exposed in host argv, logs, returned messages, or diagnostics.
pub(super) async fn pair_wireless(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    host_port: &str,
    pairing_code: &str,
) -> Result<TransportOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let mut stdin = pairing_code.as_bytes().to_vec();
    stdin.push(b'\n');
    let output = runner
        .run_with_stdin(&adb, &["pair", host_port], &stdin)
        .await?;
    let combined = format!("{}{}", output.stdout_string(), output.stderr_string());
    Ok(TransportOutcome {
        success: output.success()
            && combined
                .to_ascii_lowercase()
                .contains("successfully paired"),
        message: bound(combined.trim(), 400),
    })
}

/// `adb connect host:port` for wireless targets.
pub(super) async fn connect(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    host_port: &str,
) -> Result<TransportOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let output = runner.run(&adb, &["connect", host_port]).await?;
    let combined = format!("{}{}", output.stdout_string(), output.stderr_string());
    let lower = combined.to_ascii_lowercase();
    // adb returns exit 0 even on "failed to connect"; classify on text.
    let connected = lower.contains("connected to") && !lower.contains("failed to connect");
    Ok(TransportOutcome {
        success: connected,
        message: bound(combined.trim(), 400),
    })
}

/// `adb disconnect host:port` scoped to one wireless target.
pub(super) async fn disconnect(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    host_port: &str,
) -> Result<TransportOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let output = runner.run(&adb, &["disconnect", host_port]).await?;
    let combined = format!("{}{}", output.stdout_string(), output.stderr_string());
    let lower = combined.to_ascii_lowercase();
    Ok(TransportOutcome {
        success: output.success() && !lower.contains("no such device"),
        message: bound(combined.trim(), 400),
    })
}

// ===========================================================================
// Screenshot
// ===========================================================================

/// Capture a PNG screenshot via `adb -s <serial> exec-out screencap -p`.
///
/// `exec-out` (not `shell`) is used so the PNG bytes are not mangled by the
/// shell's CRLF translation. Returns the raw PNG bytes on success.
pub(super) async fn screencap_png(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
) -> Result<Vec<u8>, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let argv = serial_args(serial, &["exec-out", "screencap", "-p"]);
    let output = runner.run(&adb, &argv).await?;
    if !output.success() {
        return Err(CommandError::Spawn {
            program: command_line(&adb, &argv),
            message: bound(output.stderr_string().trim(), 400),
        });
    }
    Ok(output.stdout)
}

// ===========================================================================
// Input primitives
// ===========================================================================

/// Result of an `adb shell input` primitive: whether the command exited 0 and a
/// bounded message for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) struct InputOutcome {
    pub(in crate::phone) success: bool,
    pub(in crate::phone) message: String,
}

pub(in crate::phone::adb) fn input_outcome(
    adb: &str,
    argv: &[&str],
    output: &CommandOutput,
) -> InputOutcome {
    InputOutcome {
        success: output.success(),
        message: if output.success() {
            String::new()
        } else {
            format!(
                "{} -> {}",
                command_line(adb, argv),
                bound(output.stderr_string().trim(), 200)
            )
        },
    }
}

/// `adb -s S shell input tap x y` (device coordinates, already rounded by the
/// mapping lane).
#[allow(dead_code)]
pub(super) async fn input_tap(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    x: i32,
    y: i32,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let (xs, ys) = (x.to_string(), y.to_string());
    let argv = serial_args(serial, &["shell", "input", "tap", &xs, &ys]);
    let output = runner.run(&adb, &argv).await?;
    Ok(input_outcome(&adb, &argv, &output))
}

/// `adb -s S shell input swipe x1 y1 x2 y2 [duration_ms]`.
#[allow(dead_code)]
pub(super) async fn input_swipe(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    start: (i32, i32),
    end: (i32, i32),
    duration_ms: Option<u32>,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let (x1, y1) = (start.0.to_string(), start.1.to_string());
    let (x2, y2) = (end.0.to_string(), end.1.to_string());
    let mut tail = vec!["shell", "input", "swipe", &x1, &y1, &x2, &y2];
    let duration = duration_ms.map(|d| d.to_string());
    if let Some(duration) = duration.as_deref() {
        tail.push(duration);
    }
    let argv = serial_args(serial, &tail);
    let output = runner.run(&adb, &argv).await?;
    Ok(input_outcome(&adb, &argv, &output))
}

/// `adb -s S shell "input text '<text>'"`.
///
/// The full `input text ...` command is passed as a single device-shell argument
/// with the literal text wrapped in single quotes by [`single_quote_for_shell`].
/// Single-quoting makes every character literal to the device shell — spaces, a
/// literal `%`, and shell metacharacters all survive unchanged — so the delivered
/// text matches the request exactly. This replaces the legacy `%s`-for-space
/// scheme, which corrupted any text containing a literal `%`.
pub(super) async fn input_text(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    text: &str,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let command = format!("input text {}", single_quote_for_shell(text));
    let argv = serial_args(serial, &["shell", &command]);
    let output = runner.run(&adb, &argv).await?;
    Ok(input_outcome(&adb, &argv, &output))
}

/// `adb -s S shell input keyevent <key>` where `key` is a keycode name or
/// number normalized by [`normalize_keyevent`].
pub(super) async fn input_keyevent(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    key: &str,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let normalized = normalize_keyevent(key);
    let argv = serial_args(serial, &["shell", "input", "keyevent", &normalized]);
    let output = runner.run(&adb, &argv).await?;
    Ok(input_outcome(&adb, &argv, &output))
}

/// Wrap `text` in single quotes for a POSIX device shell so every character is
/// literal. Spaces, a literal `%`, and shell metacharacters (`$`, `` ` ``, `&`,
/// `;`, `|`, `<`, `>`, `*`, quotes, etc.) all survive unchanged. An embedded
/// single quote is emitted as the standard `'\''` sequence (close quote, escaped
/// quote, reopen quote), which is the only character that cannot appear verbatim
/// inside a single-quoted string.
pub(super) fn single_quote_for_shell(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for ch in text.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Normalize a key request to the token `adb shell input keyevent` accepts.
/// Numbers and `KEYCODE_*` names pass through; bare names like `back`/`home`
/// are upper-cased and `KEYCODE_`-prefixed.
pub(super) fn normalize_keyevent(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty() {
        return trimmed.to_string();
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("KEYCODE_") {
        upper
    } else {
        format!("KEYCODE_{upper}")
    }
}

// ===========================================================================
// Display geometry
// ===========================================================================

/// Display geometry probed from `wm size` and `wm density`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisplayGeometry {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) density_dpi: Option<u32>,
}

/// Probe `wm size` (and `wm density`) for the device display. Prefers the
/// override line when present (`Override size:`), as that reflects the active
/// resolution agents see. Reachable through [`super::device::detect_profile`].
pub(super) async fn display_geometry(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
) -> Result<DisplayGeometry, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let size_argv = serial_args(serial, &["shell", "wm", "size"]);
    let size_output = runner.run(&adb, &size_argv).await?;
    let (width, height) =
        parse_wm_size(&size_output.stdout_string()).ok_or_else(|| CommandError::Spawn {
            program: command_line(&adb, &size_argv),
            message: "could not parse `wm size` output".to_string(),
        })?;

    let density_argv = serial_args(serial, &["shell", "wm", "density"]);
    let density_dpi = match runner.run(&adb, &density_argv).await {
        Ok(output) if output.success() => parse_wm_density(&output.stdout_string()),
        _ => None,
    };

    Ok(DisplayGeometry {
        width,
        height,
        density_dpi,
    })
}

/// Probe the device's live screen rotation.
///
/// `wm size` reports the natural (unrotated) resolution and so cannot reveal the
/// current orientation. This probes `dumpsys input` for the live
/// `SurfaceOrientation` instead, falling back to `dumpsys display` when the
/// input dump does not expose it. Returns `None` (rather than erroring) when the
/// command fails or the dump shape is unrecognized, so the caller degrades to the
/// aspect-ratio-derived label. Reachable through
/// [`super::device::detect_profile`].
pub(super) async fn screen_rotation(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
) -> Option<DeviceRotation> {
    let adb = resolve_adb_path(configured_adb_path);
    let input_argv = serial_args(serial, &["shell", "dumpsys", "input"]);
    if let Ok(output) = runner.run(&adb, &input_argv).await
        && output.success()
        && let Some(rotation) = parse_rotation(&output.stdout_string())
    {
        return Some(rotation);
    }
    let display_argv = serial_args(serial, &["shell", "dumpsys", "display"]);
    if let Ok(output) = runner.run(&adb, &display_argv).await
        && output.success()
        && let Some(rotation) = parse_rotation(&output.stdout_string())
    {
        return Some(rotation);
    }
    None
}

// ===========================================================================
// Foreground app / app inventory / launch
// ===========================================================================

/// Current foreground app via `dumpsys activity activities` / `window`.
///
/// Both probes are success-gated: a failed `dumpsys` (e.g. a transient wireless
/// drop) must not be read as "no foreground app". The result distinguishes
/// three outcomes:
/// - `Ok(Some(app))` — a probe ran and a focus was parsed;
/// - `Ok(None)` — both probes ran successfully but neither exposed a focus
///   (genuinely no resolvable foreground app);
/// - `Err(..)` — neither probe yielded a focus and at least one of them failed
///   (the result is unknown, not "none"), so the caller emits a diagnostic
///   rather than reporting an empty foreground.
pub(super) async fn foreground_app(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
) -> Result<Option<ForegroundApp>, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    // `dumpsys window` exposes mCurrentFocus on every modern Android; fall back
    // to activity dump for OEMs that move it.
    let window_argv = serial_args(serial, &["shell", "dumpsys", "window"]);
    let window_output = runner.run(&adb, &window_argv).await?;
    if window_output.success()
        && let Some(app) = parse_current_focus(&window_output.stdout_string())
    {
        return Ok(Some(app));
    }
    let activity_argv = serial_args(serial, &["shell", "dumpsys", "activity", "activities"]);
    let activity_output = runner.run(&adb, &activity_argv).await?;
    if activity_output.success()
        && let Some(app) = parse_current_focus(&activity_output.stdout_string())
    {
        return Ok(Some(app));
    }
    // No focus from either probe. If at least one probe failed, the foreground is
    // unknown (not empty): surface a structured failure keyed on the failing
    // command so the caller's existing `Err` arm emits a diagnostic instead of a
    // misleading `Ok(None)`.
    if !window_output.success() {
        return Err(CommandError::Spawn {
            program: command_line(&adb, &window_argv),
            message: format!(
                "`dumpsys window` exited with status {} and no fallback focus was found",
                window_output.status.map_or(-1, |c| c)
            ),
        });
    }
    if !activity_output.success() {
        return Err(CommandError::Spawn {
            program: command_line(&adb, &activity_argv),
            message: format!(
                "`dumpsys activity activities` exited with status {} and no focus was found",
                activity_output.status.map_or(-1, |c| c)
            ),
        });
    }
    // Both probes ran cleanly but neither exposed a focus: genuinely none.
    Ok(None)
}

/// List installed packages via `pm list packages`. When `launchable_only`,
/// passes nothing special (the integrator filters with a launcher query in the
/// companion lane); when `include_system` is false, `-3` restricts to
/// third-party packages. Returns bare package names.
pub(super) async fn list_packages(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    include_system: bool,
) -> Result<Vec<String>, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let mut tail = vec!["shell", "pm", "list", "packages"];
    if !include_system {
        tail.push("-3");
    }
    let argv = serial_args(serial, &tail);
    let output = runner.run(&adb, &argv).await?;
    Ok(parse_package_list(&output.stdout_string()))
}

/// Launch an app by package via the monkey launcher intent (works without a
/// known component): `monkey -p <pkg> -c android.intent.category.LAUNCHER 1`.
pub(super) async fn launch_package(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    package: &str,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    // `package` is untrusted free text; single-quote it so the on-device shell
    // treats it as one literal argument (adb rejoins argv with spaces and runs
    // the result through `sh -c`, so an unquoted metacharacter would be live).
    let command = format!(
        "monkey -p {} -c android.intent.category.LAUNCHER 1",
        single_quote_for_shell(package)
    );
    let argv = serial_args(serial, &["shell", &command]);
    let output = runner.run(&adb, &argv).await?;
    let combined = format!("{}{}", output.stdout_string(), output.stderr_string());
    // monkey exits 0 even when no launchable activity exists; classify on text.
    let no_activity = combined
        .to_ascii_lowercase()
        .contains("no activities found");
    Ok(InputOutcome {
        success: output.success() && !no_activity,
        message: if no_activity {
            format!("no launchable activity for {package}")
        } else {
            String::new()
        },
    })
}

/// Launch an activity/deep link/intent URI via `am start`. When `package` is
/// supplied it is passed with `-p` to scope the resolution.
pub(super) async fn start_intent(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    intent_uri: &str,
    package: Option<&str>,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    // `intent_uri` and `package` are untrusted free text; single-quote each so
    // the on-device shell treats them as literal arguments. adb rejoins the argv
    // after `shell` with spaces and runs it through `sh -c`, so any unquoted
    // shell metacharacter (`;`, `$(...)`, backticks, `&&`) would otherwise be
    // interpreted on the device.
    let mut command = format!(
        "am start -a android.intent.action.VIEW -d {}",
        single_quote_for_shell(intent_uri)
    );
    if let Some(package) = package {
        command.push_str(" -p ");
        command.push_str(&single_quote_for_shell(package));
    }
    let argv = serial_args(serial, &["shell", &command]);
    let output = runner.run(&adb, &argv).await?;
    let combined = format!("{}{}", output.stdout_string(), output.stderr_string());
    let failed = combined.to_ascii_lowercase().contains("error:");
    Ok(InputOutcome {
        success: output.success() && !failed,
        message: bound(combined.trim(), 300),
    })
}

/// `am force-stop <package>`, with a post-stop foreground verification.
///
/// `am force-stop` exits 0 even when the package does not exist, is protected,
/// or the stop is otherwise ineffective, so the exit code alone is not proof the
/// app was stopped. After a clean exit this re-checks the foreground app: if the
/// target package is still foreground, the outcome is reported as a failure with
/// a structured message rather than a false success.
///
/// The check is inherently racy — the app (or its launcher) can relaunch the
/// package between the stop and the foreground probe — so a `false` here means
/// "still foreground at the moment we looked", not "force-stop is broken". When
/// the foreground probe itself cannot run, the original exit-code outcome is
/// kept rather than inventing a failure.
pub(super) async fn force_stop(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    package: &str,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    // `package` is untrusted free text; single-quote it so the on-device shell
    // treats it as one literal argument (see `start_intent` for the mechanism).
    let command = format!("am force-stop {}", single_quote_for_shell(package));
    let argv = serial_args(serial, &["shell", &command]);
    let output = runner.run(&adb, &argv).await?;
    let mut outcome = input_outcome(&adb, &argv, &output);

    // Only verify when the command itself exited cleanly; a non-zero exit is
    // already a failure carrying its own message. The probe runs only when the
    // stop exited 0 (the let-chain short-circuits otherwise).
    if outcome.success
        && let Ok(Some(foreground)) = foreground_app(runner, configured_adb_path, serial).await
        && foreground.package == package
    {
        outcome.success = false;
        outcome.message = format!(
            "`am force-stop {package}` exited 0 but {package} is still the \
             foreground app; the stop was ineffective (protected/ineligible \
             package, or the app relaunched immediately)"
        );
    }

    Ok(outcome)
}

/// Open an Android settings screen via the documented `am start` action for each
/// [`PhoneSettingsScreen`]. App-scoped screens (`AppDetails`) require a package.
pub(super) async fn open_settings(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    screen: PhoneSettingsScreen,
    package: Option<&str>,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let action = settings_action(screen);
    // `action` is a hardcoded constant; only the app-details package is
    // untrusted, so single-quote the `package:` data for the on-device shell.
    let mut command = format!("am start -a {action}");
    if matches!(screen, PhoneSettingsScreen::AppDetails)
        && let Some(package) = package
    {
        command.push_str(" -d ");
        command.push_str(&single_quote_for_shell(&format!("package:{package}")));
    }
    let argv = serial_args(serial, &["shell", &command]);
    let output = runner.run(&adb, &argv).await?;
    let combined = format!("{}{}", output.stdout_string(), output.stderr_string());
    let failed = combined.to_ascii_lowercase().contains("error:");
    Ok(InputOutcome {
        success: output.success() && !failed,
        message: bound(combined.trim(), 300),
    })
}

/// The `am start -a <ACTION>` settings action string for each screen.
pub(super) fn settings_action(screen: PhoneSettingsScreen) -> &'static str {
    match screen {
        PhoneSettingsScreen::Accessibility => "android.settings.ACCESSIBILITY_SETTINGS",
        PhoneSettingsScreen::NotificationAccess => {
            "android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS"
        }
        PhoneSettingsScreen::OverlayPermission => {
            "android.settings.action.MANAGE_OVERLAY_PERMISSION"
        }
        PhoneSettingsScreen::AppDetails => "android.settings.APPLICATION_DETAILS_SETTINGS",
        PhoneSettingsScreen::WirelessDebugging => {
            "android.settings.APPLICATION_DEVELOPMENT_SETTINGS"
        }
        PhoneSettingsScreen::BatteryOptimization => {
            "android.settings.IGNORE_BATTERY_OPTIMIZATION_SETTINGS"
        }
    }
}

// Install/forward primitives live in the `install` child module to keep this
// file under the god-file threshold. They are re-exported below as `adb::*`.
