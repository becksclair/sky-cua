//! Device-fact probing and capability-profile construction.
//!
//! `phone_connect` detects what a device is (manufacturer/brand/model/codename,
//! Android SDK/release), how it is connected, its display geometry, and cheap
//! privileged-state signals (root, Shizuku, device-owner). Those facts populate
//! the device portion of a [`PhoneCapabilityProfile`]; backend-availability and
//! action affordances are layered on by the ADB/companion/scrcpy lanes.
//!
//! Everything that touches a device goes through [`CommandRunner`]; the pure
//! classification helpers ([`classify_target_device`], the getprop parser) are
//! unit-tested without a runner.

use sky_cua_platform::model::{
    PhoneBackendCapabilities, PhoneCapabilityProfile, PhoneCapabilityRefreshState,
    PhoneCompanionCapabilities, PhoneConnectionKind, PhoneScrcpyCapabilities,
    PhoneTargetDeviceKind, PixelSize,
};

use super::adb;
use super::command::{CommandError, CommandRunner, resolve_adb_path};

/// The getprop keys probed at session start. Order is stable for the batched
/// `getprop` parse and for deterministic tests. Documented surface for the
/// integrator; the batched probe reads the full `getprop` dump rather than
/// these keys individually.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) const DEVICE_PROPERTY_KEYS: &[&str] = &[
    "ro.product.manufacturer",
    "ro.product.brand",
    "ro.product.model",
    "ro.product.device",
    "ro.build.version.sdk",
    "ro.build.version.release",
    "ro.build.characteristics",
];

/// Raw device identity probed from `getprop`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct DeviceProperties {
    pub(super) manufacturer: Option<String>,
    pub(super) brand: Option<String>,
    pub(super) model: Option<String>,
    pub(super) device: Option<String>,
    pub(super) android_sdk: Option<u32>,
    pub(super) android_release: Option<String>,
    /// `ro.build.characteristics` (e.g. `tablet`, `emulator`, `nosdcard`).
    pub(super) characteristics: Option<String>,
}

/// Cheap privileged-state signals. Each probe is best-effort; a `false`/`None`
/// means "not detected", never "proven absent".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PrivilegedState {
    pub(super) root_available: bool,
    pub(super) shizuku_available: bool,
    pub(super) device_owner: bool,
}

/// Backend-availability summary for a freshly-detected session. Every capability
/// is false; the ADB/companion/scrcpy lanes flip the ones they prove.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) fn empty_backend_capabilities() -> PhoneBackendCapabilities {
    PhoneBackendCapabilities {
        adb: false,
        companion: false,
        scrcpy: false,
        screenshot: false,
        gestures: false,
        text_input: false,
        key_input: false,
        accessibility_tree: false,
        notifications: false,
        app_management: false,
        host_visible_overlay: false,
        screenshot_synthetic_cursor: false,
        phone_native_overlay: false,
    }
}

/// Parse `getprop` `[key]: [value]` lines (the format `getprop` with no args
/// prints) into a [`DeviceProperties`]. Tolerates missing keys, blank lines, and
/// values containing spaces/brackets.
pub(super) fn parse_getprop(stdout: &str) -> DeviceProperties {
    let mut props = DeviceProperties::default();
    for line in stdout.lines() {
        let Some((key, value)) = parse_getprop_line(line) else {
            continue;
        };
        match key.as_str() {
            "ro.product.manufacturer" => props.manufacturer = nonempty(&value),
            "ro.product.brand" => props.brand = nonempty(&value),
            "ro.product.model" => props.model = nonempty(&value),
            "ro.product.device" => props.device = nonempty(&value),
            "ro.build.version.sdk" => props.android_sdk = value.trim().parse().ok(),
            "ro.build.version.release" => props.android_release = nonempty(&value),
            "ro.build.characteristics" => props.characteristics = nonempty(&value),
            _ => {}
        }
    }
    props
}

/// Parse one `[key]: [value]` getprop line into `(key, value)`.
fn parse_getprop_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let (key_part, value_part) = trimmed.split_once(':')?;
    let key = key_part
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let value = value_part
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Classify the connected device for compatibility lanes from its properties and
/// connection kind.
///
/// - emulator serials / `emulator` characteristics → [`PhoneTargetDeviceKind::Emulator`]
/// - Samsung Galaxy S26 Ultra (`SM-S94x` model family, or brand+model match) →
///   [`PhoneTargetDeviceKind::GalaxyS26Ultra`]
/// - Xiaomi/Redmi tablets (Redmi brand + tablet characteristics, or `Redmi Pad`
///   model) → [`PhoneTargetDeviceKind::RedmiTablet`]
/// - everything else → [`PhoneTargetDeviceKind::UnknownAndroid`]
pub(super) fn classify_target_device(
    props: &DeviceProperties,
    connection_kind: PhoneConnectionKind,
) -> PhoneTargetDeviceKind {
    if connection_kind == PhoneConnectionKind::Emulator
        || props
            .characteristics
            .as_deref()
            .is_some_and(|c| c.to_ascii_lowercase().contains("emulator"))
        || props
            .model
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("Android SDK built for x86"))
    {
        return PhoneTargetDeviceKind::Emulator;
    }

    let manufacturer = props
        .manufacturer
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let brand = props.brand.as_deref().unwrap_or("").to_ascii_lowercase();
    let model = props.model.as_deref().unwrap_or("");
    let model_lower = model.to_ascii_lowercase();

    // Galaxy S26 Ultra: Samsung's S-series Ultra model code family. The S26
    // Ultra carries an SM-S94x code; match the family plus the marketing model.
    let is_samsung = manufacturer.contains("samsung") || brand.contains("samsung");
    if is_samsung
        && (model_lower.contains("galaxy s26 ultra")
            || model.starts_with("SM-S948")
            || model.starts_with("SM-S946"))
    {
        return PhoneTargetDeviceKind::GalaxyS26Ultra;
    }

    // Redmi-family tablet (incl. HyperOS): Xiaomi/Redmi brand on a tablet.
    let is_xiaomi = manufacturer.contains("xiaomi")
        || brand.contains("xiaomi")
        || brand.contains("redmi")
        || manufacturer.contains("redmi");
    let is_tablet = props
        .characteristics
        .as_deref()
        .is_some_and(|c| c.to_ascii_lowercase().contains("tablet"))
        || model_lower.contains("pad")
        || model_lower.contains("tablet");
    if is_xiaomi && is_tablet {
        return PhoneTargetDeviceKind::RedmiTablet;
    }

    PhoneTargetDeviceKind::UnknownAndroid
}

/// Detect a HyperOS version string from a Xiaomi/Redmi build, if present. Reads
/// `ro.mi.os.version.name` / `ro.miui.ui.version.name` style props from a
/// pre-parsed `getprop` map line scan. Returns e.g. `"OS2.0.1.0"` or a MIUI
/// version; the integrator surfaces it as `hyperos_version`.
pub(super) fn parse_hyperos_version(getprop_stdout: &str) -> Option<String> {
    for key in [
        "ro.mi.os.version.name",
        "ro.mi.os.version.incremental",
        "ro.miui.ui.version.name",
    ] {
        for line in getprop_stdout.lines() {
            if let Some((k, v)) = parse_getprop_line(line) {
                if k == key {
                    if let Some(value) = nonempty(&v) {
                        return Some(value);
                    }
                }
            }
        }
    }
    None
}

/// Batch-probe device properties through `getprop`.
///
/// A single `adb -s S shell getprop` dump is parsed for all keys at once, which
/// is cheaper and more atomic than one `getprop <key>` per property. Reachable
/// through [`detect_profile`].
pub(super) async fn probe_properties(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
) -> Result<(DeviceProperties, Option<String>), CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let argv = serial_args(serial, &["shell", "getprop"]);
    let output = runner.run(&adb, &argv).await?;
    let stdout = output.stdout_string();
    Ok((parse_getprop(&stdout), parse_hyperos_version(&stdout)))
}

/// Cheap privileged-state probes.
///
/// - root: `adb shell su -c id` reporting `uid=0`, or `which su` returning a
///   path. Treated as available only on a positive signal.
/// - Shizuku: presence of the Shizuku package via `pm path moe.shizuku.privileged.api`.
/// - device-owner: `dpm list-owners` / `dumpsys device_policy` reporting an
///   active device owner.
///
/// Reachable through [`detect_profile`].
pub(super) async fn probe_privileged_state(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
) -> PrivilegedState {
    let adb = resolve_adb_path(configured_adb_path);
    let mut state = PrivilegedState::default();

    let root_argv = serial_args(serial, &["shell", "su", "-c", "id"]);
    if let Ok(output) = runner.run(&adb, &root_argv).await {
        state.root_available = output.success() && output.stdout_string().contains("uid=0");
    }

    let shizuku_argv = serial_args(
        serial,
        &["shell", "pm", "path", "moe.shizuku.privileged.api"],
    );
    if let Ok(output) = runner.run(&adb, &shizuku_argv).await {
        state.shizuku_available = output.success() && output.stdout_string().contains("package:");
    }

    let owner_argv = serial_args(serial, &["shell", "dumpsys", "device_policy"]);
    if let Ok(output) = runner.run(&adb, &owner_argv).await {
        state.device_owner = parse_device_owner(&output.stdout_string());
    }

    state
}

/// True when `dumpsys device_policy` reports an active device owner.
///
/// A real device owner appears as a bare `Device Owner:` assignment naming an
/// admin component. Unrelated mentions such as `Device Owner Type: -1`,
/// `Device Owner: null`, or `Device Owner: none` are not owners, so only an
/// assignment whose value names a component (`ComponentInfo`/package-class)
/// counts. The looser substring check this replaced misfired on Samsung's
/// `Device Owner Type: -1` line and reported a non-existent owner.
pub(super) fn parse_device_owner(stdout: &str) -> bool {
    for line in stdout.lines() {
        let lower = line.trim().to_ascii_lowercase();
        let Some((_, rest)) = lower.split_once("device owner") else {
            continue;
        };
        // Require a bare `device owner:` assignment; skip `device owner type:`,
        // `device owner mode:`, and similar non-owner keys.
        let Some(value) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() || value == "null" || value == "none" || value.starts_with('-') {
            continue;
        }
        if value.contains("componentinfo") || value.contains('/') || value.contains('.') {
            return true;
        }
    }
    false
}

fn serial_args<'a>(serial: &'a str, tail: &[&'a str]) -> Vec<&'a str> {
    let mut argv = Vec::with_capacity(tail.len() + 2);
    argv.push("-s");
    argv.push(serial);
    argv.extend_from_slice(tail);
    argv
}

/// Build a capability profile for a session.
///
/// Probes device properties, display geometry, privileged state, and target
/// classification through `runner`, then assembles the device-fact portion of a
/// [`PhoneCapabilityProfile`]. Backend-capability fields (companion/scrcpy,
/// action affordances) are left at their absent defaults for the companion and
/// snapshot/cursor lanes to fill. Probe failures degrade individual fields to
/// `None` rather than failing the whole detection, so an ADB-only device still
/// yields a usable profile.
///
/// Signature is fixed by the spine: the manager's cache test calls this with a
/// `FakeCommandRunner`, a session id/serial, the companion package, a timestamp,
/// and the refresh state. With an unscripted runner every probe errors and the
/// profile carries only identifiers — never fabricated device state.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) async fn detect_profile(
    runner: &dyn CommandRunner,
    session_id: &str,
    serial: &str,
    companion_package: &str,
    detected_at_ms: u64,
    refresh_state: PhoneCapabilityRefreshState,
) -> PhoneCapabilityProfile {
    detect_profile_with_path(
        runner,
        None,
        session_id,
        serial,
        companion_package,
        detected_at_ms,
        refresh_state,
    )
    .await
}

/// [`detect_profile`] with an explicit configured adb path. The integrator wires
/// this once it threads the resolved selection through `phone_connect`.
/// Reachable through [`detect_profile`].
pub(super) async fn detect_profile_with_path(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    session_id: &str,
    serial: &str,
    companion_package: &str,
    detected_at_ms: u64,
    refresh_state: PhoneCapabilityRefreshState,
) -> PhoneCapabilityProfile {
    let connection_kind = adb::classify_connection_kind(serial);

    let (props, hyperos_version) = probe_properties(runner, configured_adb_path, serial)
        .await
        .unwrap_or_default();

    let display = adb::display_geometry(runner, configured_adb_path, serial)
        .await
        .ok();
    // Live rotation probe. `wm size` reports the natural (unrotated) resolution,
    // so the aspect-derived label is wrong on a rotated device. Prefer the live
    // `dumpsys` rotation; only fall back to the aspect-derived label when the
    // probe yields nothing. A downstream lane compares snapshot rotation to this
    // value, so it must reflect the device's current orientation, not its panel
    // aspect ratio.
    let live_rotation = adb::screen_rotation(runner, configured_adb_path, serial).await;
    // The exact quarter (0/90/180/270) the live probe reported, preserved
    // alongside the coarse orientation label. The label only distinguishes
    // portrait/landscape, so downstream host content-rect math (which needs the
    // real 180/270 quarters) reads this numeric value and only falls back to the
    // label-derived quarter when no live rotation was probed.
    let display_rotation_degrees = live_rotation.map(|rotation| i32::from(rotation.degrees));
    let (display_size, density_dpi, orientation) = match display {
        Some(geometry) => {
            let orientation = live_rotation.map_or_else(
                || orientation_label(geometry.width, geometry.height),
                |rotation| rotation.label.to_string(),
            );
            (
                Some(PixelSize {
                    width: geometry.width,
                    height: geometry.height,
                }),
                geometry.density_dpi,
                Some(orientation),
            )
        }
        // No display geometry, but a live rotation may still be available.
        None => (
            None,
            None,
            live_rotation.map(|rotation| rotation.label.to_string()),
        ),
    };

    let privileged = probe_privileged_state(runner, configured_adb_path, serial).await;
    let target_device_kind = classify_target_device(&props, connection_kind);

    PhoneCapabilityProfile {
        profile_id: format!("{session_id}-profile"),
        session_id: session_id.to_string(),
        serial: serial.to_string(),
        detected_at_ms,
        stale: matches!(refresh_state, PhoneCapabilityRefreshState::Stale),
        refresh_state,
        manufacturer: props.manufacturer,
        brand: props.brand,
        model: props.model,
        device: props.device,
        target_device_kind,
        hyperos_version,
        android_sdk: props.android_sdk,
        android_release: props.android_release,
        display_size,
        density_dpi,
        orientation,
        display_rotation_degrees,
        connection_kind,
        companion: PhoneCompanionCapabilities::absent(companion_package),
        scrcpy: PhoneScrcpyCapabilities::absent(),
        root_available: privileged.root_available,
        shizuku_available: privileged.shizuku_available,
        device_owner: privileged.device_owner,
        available_actions: Vec::new(),
        unavailable_actions: Vec::new(),
    }
}

/// Coarse portrait/landscape label from display dimensions.
fn orientation_label(width: u32, height: u32) -> String {
    if width > height {
        "landscape".to_string()
    } else {
        "portrait".to_string()
    }
}

#[cfg(test)]
mod tests;
