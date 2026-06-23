//! Pure parsers for stable `adb` text output.
//!
//! Every function here is total over malformed/empty input (it returns `None`,
//! an empty collection, or an `Unknown` classification rather than panicking)
//! and never touches a [`super::CommandRunner`]. This keeps the classification
//! logic unit-testable in isolation from process execution.

use sky_cua_platform::model::{PhoneConnectionKind, PhoneDevice, PhoneDeviceState};

/// One parsed line from `adb devices -l`, before mapping into a [`PhoneDevice`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) struct AdbDeviceLine {
    pub(in crate::phone) serial: String,
    pub(in crate::phone) state: PhoneDeviceState,
    pub(in crate::phone) connection_kind: PhoneConnectionKind,
    pub(in crate::phone) model: Option<String>,
    pub(in crate::phone) product: Option<String>,
    pub(in crate::phone) device: Option<String>,
    pub(in crate::phone) transport_id: Option<String>,
}

impl AdbDeviceLine {
    /// Lower into the public model type.
    pub(in crate::phone) fn into_device(self) -> PhoneDevice {
        PhoneDevice {
            serial: self.serial,
            state: self.state,
            connection_kind: self.connection_kind,
            model: self.model,
            product: self.product,
            device: self.device,
            transport_id: self.transport_id,
            // The ADB wire parse never knows operator policy; the host
            // device-list path marks primaries against `[phone]
            // primary_target_models`.
            primary: false,
        }
    }
}

/// Parse `adb version` stdout into a version string (e.g. `1.0.41`).
pub(in crate::phone) fn parse_version(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("android debug bridge version") {
            return line.rsplit("version").next().map(|v| v.trim().to_string());
        }
    }
    None
}

/// Parse `adb devices` (no `-l`) stdout to decide whether the server is up and
/// answering. Returns the number of listed transports (header excluded). The
/// server being reachable is signalled by the presence of the
/// `List of devices attached` header.
#[cfg_attr(not(test), expect(dead_code))]
pub(in crate::phone) fn parse_server_status(stdout: &str) -> Option<usize> {
    let mut saw_header = false;
    let mut count = 0;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("list of devices attached") {
            saw_header = true;
            continue;
        }
        if saw_header && !trimmed.is_empty() {
            count += 1;
        }
    }
    saw_header.then_some(count)
}

/// Parse `adb devices -l` stdout into typed device lines.
///
/// Handles the header line, blank lines, the `serial state key:value...` long
/// format, `unauthorized`/`offline`/`no permissions` states, emulator serials,
/// and `host:port` wireless serials. Malformed lines are skipped.
pub(in crate::phone) fn parse_devices_l(stdout: &str) -> Vec<AdbDeviceLine> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("list of devices attached") {
            continue;
        }
        // adb prints transient connection notices to the same stream.
        if lower.starts_with("* daemon") || lower.starts_with("adb server") {
            continue;
        }

        let mut fields = trimmed.split_whitespace();
        let Some(serial) = fields.next() else {
            continue;
        };
        // The state token is everything up to the first `key:value` pair, but
        // adb emits states as single tokens except "no permissions" which spans
        // two. Reconstruct by scanning until a `key:value` token or end.
        let rest: Vec<&str> = fields.collect();
        let (state_str, kv_start) = split_state(&rest);
        let state = classify_device_state(&state_str);

        let mut model = None;
        let mut product = None;
        let mut device = None;
        let mut transport_id = None;
        for token in &rest[kv_start..] {
            if let Some((key, value)) = token.split_once(':') {
                match key {
                    "model" => model = nonempty(value),
                    "product" => product = nonempty(value),
                    "device" => device = nonempty(value),
                    "transport_id" => transport_id = nonempty(value),
                    _ => {}
                }
            }
        }

        out.push(AdbDeviceLine {
            connection_kind: classify_connection_kind(serial),
            serial: serial.to_string(),
            state,
            model,
            product,
            device,
            transport_id,
        });
    }
    out
}

/// Split the post-serial tokens into the state phrase and the index where
/// `key:value` metadata begins.
fn split_state(rest: &[&str]) -> (String, usize) {
    if rest.is_empty() {
        return (String::new(), 0);
    }
    // "no permissions ..." is the only multi-word state adb emits.
    if rest.len() >= 2 && rest[0] == "no" && rest[1].starts_with("permissions") {
        return ("no permissions".to_string(), 2);
    }
    (rest[0].to_string(), 1)
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Map an adb device-state token to [`PhoneDeviceState`].
pub(in crate::phone) fn classify_device_state(state: &str) -> PhoneDeviceState {
    match state.trim().to_ascii_lowercase().as_str() {
        "device" => PhoneDeviceState::Device,
        "unauthorized" => PhoneDeviceState::Unauthorized,
        "offline" => PhoneDeviceState::Offline,
        "no permissions" => PhoneDeviceState::NoPermissions,
        "connecting" | "authorizing" => PhoneDeviceState::Connecting,
        "bootloader" | "fastboot" => PhoneDeviceState::Bootloader,
        "recovery" | "sideload" => PhoneDeviceState::Recovery,
        _ => PhoneDeviceState::Unknown,
    }
}

/// Classify the transport kind from a device serial.
///
/// - `emulator-NNNN` → emulator
/// - `host:5555` → legacy `adb tcpip` wireless
/// - `host:port` (non-5555, including `adb-...._adb-tls-connect`) → Android 11+
///   wireless debugging
/// - everything else → USB
pub(in crate::phone) fn classify_connection_kind(serial: &str) -> PhoneConnectionKind {
    if serial.starts_with("emulator-") {
        return PhoneConnectionKind::Emulator;
    }
    // mDNS wireless-debugging serials look like `adb-XXXX-YYYY._adb-tls-connect._tcp`.
    if serial.contains("_adb-tls-connect") || serial.contains("_adb-tls-pairing") {
        return PhoneConnectionKind::WirelessDebugging;
    }
    if let Some((host, port)) = serial.rsplit_once(':') {
        if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() {
            return if port == "5555" {
                PhoneConnectionKind::LegacyTcpip
            } else {
                PhoneConnectionKind::WirelessDebugging
            };
        }
    }
    PhoneConnectionKind::Usb
}

/// Parse `adb mdns services` stdout into `(name, type, address)` tuples. Skips
/// the header line and malformed rows.
pub(in crate::phone) fn parse_mdns_services(stdout: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("list of discovered") {
            continue;
        }
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() >= 3 {
            out.push((
                fields[0].to_string(),
                fields[1].to_string(),
                fields[2].to_string(),
            ));
        }
    }
    out
}

/// Parse `wm size` stdout, preferring an `Override size:` line over the
/// `Physical size:` line. Returns `(width, height)`.
pub(in crate::phone) fn parse_wm_size(stdout: &str) -> Option<(u32, u32)> {
    let mut physical = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(dims) = trimmed
            .strip_prefix("Override size:")
            .and_then(parse_dimensions)
        {
            return Some(dims);
        }
        if let Some(dims) = trimmed
            .strip_prefix("Physical size:")
            .and_then(parse_dimensions)
        {
            physical = Some(dims);
        }
    }
    physical
}

fn parse_dimensions(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.trim().split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// Parse `wm density` stdout, preferring an `Override density:` line over the
/// `Physical density:` line.
pub(in crate::phone) fn parse_wm_density(stdout: &str) -> Option<u32> {
    let mut physical = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Override density:") {
            return value.trim().parse().ok();
        }
        if let Some(value) = trimmed.strip_prefix("Physical density:") {
            physical = value.trim().parse().ok();
        }
    }
    physical
}

/// The device's live screen rotation, parsed from `dumpsys`.
///
/// `quarter_turns` is the rotation in 90-degree steps (0/1/2/3) as Android
/// reports it via `Surface.ROTATION_*`. `degrees` is the same value scaled to
/// 0/90/180/270. `label` is the coarse "portrait"/"landscape" classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::phone) struct DeviceRotation {
    pub(in crate::phone) quarter_turns: u8,
    pub(in crate::phone) degrees: u16,
    pub(in crate::phone) label: &'static str,
}

impl DeviceRotation {
    fn from_quarter_turns(quarter_turns: u8) -> Self {
        let quarter_turns = quarter_turns % 4;
        // Assumption: portrait-native phone. For a device whose natural
        // orientation is portrait (the common phone case this lane targets),
        // even quarter-turns (0, 2) leave it portrait and odd quarter-turns
        // (1, 3) put it in landscape. Tablets with a landscape-native panel
        // invert this, but `wm size` cannot disambiguate natural orientation,
        // so this lane documents the portrait-native assumption rather than
        // guessing. Callers fall back to the aspect-derived label when the
        // probe yields nothing.
        let label = if quarter_turns.is_multiple_of(2) {
            "portrait"
        } else {
            "landscape"
        };
        Self {
            quarter_turns,
            degrees: u16::from(quarter_turns) * 90,
            label,
        }
    }
}

/// Parse the live screen rotation from `dumpsys input` (or `dumpsys display`).
///
/// Two stable sources are recognized, in priority order:
/// 1. `dumpsys input` exposes one or more `SurfaceOrientation: N` lines in the
///    per-display reader info, where `N` is 0/1/2/3 (`Surface.ROTATION_*`).
/// 2. `dumpsys display` exposes `mCurrentOrientation=N` (and some builds a bare
///    `rotation=N`) on the logical-display record.
///
/// The first recognized value wins. Anything unrecognized (no matching key, a
/// non-0..=3 value, empty input) returns `None` so the caller falls back to the
/// aspect-ratio-derived orientation label. This keeps the parser total over
/// malformed input and conservative when the dump shape is unfamiliar.
pub(in crate::phone) fn parse_rotation(stdout: &str) -> Option<DeviceRotation> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(value) = rotation_value_after(trimmed, "SurfaceOrientation") {
            return Some(DeviceRotation::from_quarter_turns(value));
        }
    }
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(value) = rotation_value_after(trimmed, "mCurrentOrientation") {
            return Some(DeviceRotation::from_quarter_turns(value));
        }
        if let Some(value) = rotation_value_after(trimmed, "rotation") {
            return Some(DeviceRotation::from_quarter_turns(value));
        }
    }
    None
}

/// If `line` contains `key` followed by a `:`/`=` separator and a 0..=3 integer,
/// return that integer. Tolerates surrounding whitespace and trailing tokens
/// (e.g. `SurfaceOrientation: 1` or `mCurrentOrientation=3,`). Returns `None`
/// for any value outside the rotation range so a stray match never fabricates a
/// rotation.
fn rotation_value_after(line: &str, key: &str) -> Option<u8> {
    let idx = line.find(key)?;
    let after = &line[idx + key.len()..];
    let after = after.trim_start();
    let after = after
        .strip_prefix(':')
        .or_else(|| after.strip_prefix('='))?
        .trim_start();
    let token: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    let value: u8 = token.parse().ok()?;
    (value <= 3).then_some(value)
}

/// Current foreground app parsed from `dumpsys window` / `dumpsys activity`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) struct ForegroundApp {
    pub(in crate::phone) package: String,
    pub(in crate::phone) activity: Option<String>,
}

/// Parse the current foreground component from `mCurrentFocus`,
/// `mResumedActivity`, or `mFocusedApp` lines. Handles the
/// `package/.Activity` and `package/com.x.Activity` forms.
pub(in crate::phone) fn parse_current_focus(stdout: &str) -> Option<ForegroundApp> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.contains("mCurrentFocus")
            || trimmed.contains("mResumedActivity")
            || trimmed.contains("mFocusedApp")
        {
            if let Some(app) = extract_component(trimmed) {
                return Some(app);
            }
        }
    }
    None
}

/// Extract a `package/activity` component from a dumpsys line. The component is
/// the last whitespace-separated token containing a `/`, with a trailing `}`
/// stripped.
fn extract_component(line: &str) -> Option<ForegroundApp> {
    let token = line
        .split_whitespace()
        .filter(|t| t.contains('/'))
        .next_back()?;
    let token = token.trim_end_matches(['}', ',']);
    let (package, activity) = token.split_once('/')?;
    if package.is_empty() || package.contains('=') {
        return None;
    }
    let activity = if activity.is_empty() {
        None
    } else if let Some(stripped) = activity.strip_prefix('.') {
        Some(format!("{package}.{stripped}"))
    } else {
        Some(activity.to_string())
    };
    Some(ForegroundApp {
        package: package.to_string(),
        activity,
    })
}

/// Parse `pm list packages` stdout (`package:com.example`) into bare package
/// names. Skips malformed/blank lines.
pub(in crate::phone) fn parse_package_list(stdout: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(pkg) = trimmed.strip_prefix("package:") {
            // `pm list packages -f` appends `=com.pkg`; strip the apk path.
            let name = pkg.rsplit('=').next().unwrap_or(pkg).trim();
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// Extract the stable `INSTALL_FAILED_*` / `INSTALL_PARSE_FAILED_*` class from
/// adb install output, or `Failure [...]` text. Returns `None` when no failure
/// marker is present.
pub(in crate::phone) fn parse_install_failure(output: &str) -> Option<String> {
    for token in output.split(|c: char| c.is_whitespace() || c == '[' || c == ']') {
        let token = token.trim_end_matches(|c: char| c.is_ascii_punctuation() && c != '_');
        if token.starts_with("INSTALL_FAILED_") || token.starts_with("INSTALL_PARSE_FAILED_") {
            return Some(token.to_string());
        }
    }
    let lower = output.to_ascii_lowercase();
    if lower.contains("failure") || lower.contains("error") {
        return Some("INSTALL_FAILED_UNKNOWN".to_string());
    }
    None
}
