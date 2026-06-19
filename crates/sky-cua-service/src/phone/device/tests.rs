//! Device-fact parser and probe tests. Getprop parsing and target-device
//! classification are covered directly; the batched probes run through a
//! [`FakeCommandRunner`].

use sky_cua_platform::model::{PhoneConnectionKind, PhoneTargetDeviceKind};

use super::*;
use crate::phone::command::{CommandOutput, FakeCommandRunner};

const GALAXY_GETPROP: &str = "\
[ro.product.manufacturer]: [samsung]
[ro.product.brand]: [samsung]
[ro.product.model]: [SM-S948B]
[ro.product.device]: [p3q]
[ro.build.version.sdk]: [36]
[ro.build.version.release]: [16]
[ro.build.characteristics]: [phone]
";

const REDMI_TABLET_GETPROP: &str = "\
[ro.product.manufacturer]: [Xiaomi]
[ro.product.brand]: [Redmi]
[ro.product.model]: [Redmi Pad 2 Pro]
[ro.product.device]: [tablet_device]
[ro.build.version.sdk]: [36]
[ro.build.version.release]: [16]
[ro.build.characteristics]: [tablet]
[ro.mi.os.version.name]: [OS3.1.0.0]
";

const EMULATOR_GETPROP: &str = "\
[ro.product.manufacturer]: [Google]
[ro.product.brand]: [google]
[ro.product.model]: [sdk_gphone64_x86_64]
[ro.build.version.sdk]: [35]
[ro.build.version.release]: [15]
[ro.build.characteristics]: [emulator]
";

#[test]
fn parse_getprop_extracts_all_fields() {
    let props = parse_getprop(GALAXY_GETPROP);
    assert_eq!(props.manufacturer.as_deref(), Some("samsung"));
    assert_eq!(props.brand.as_deref(), Some("samsung"));
    assert_eq!(props.model.as_deref(), Some("SM-S948B"));
    assert_eq!(props.device.as_deref(), Some("p3q"));
    assert_eq!(props.android_sdk, Some(36));
    assert_eq!(props.android_release.as_deref(), Some("16"));
    assert_eq!(props.characteristics.as_deref(), Some("phone"));
}

#[test]
fn parse_getprop_tolerates_empty_and_malformed() {
    let props = parse_getprop("");
    assert!(props.manufacturer.is_none());
    assert!(props.android_sdk.is_none());
    let garbage = parse_getprop("not a getprop line\n[]: []\n[ro.product.model]: []\n");
    assert!(garbage.model.is_none());
}

#[test]
fn parse_getprop_handles_values_with_spaces() {
    let props = parse_getprop(REDMI_TABLET_GETPROP);
    assert_eq!(props.model.as_deref(), Some("Redmi Pad 2 Pro"));
}

#[test]
fn parse_hyperos_version_reads_mi_os_name() {
    assert_eq!(
        parse_hyperos_version(REDMI_TABLET_GETPROP).as_deref(),
        Some("OS3.1.0.0")
    );
    assert_eq!(parse_hyperos_version(GALAXY_GETPROP), None);
}

#[test]
fn classify_galaxy_s26_ultra() {
    let props = parse_getprop(GALAXY_GETPROP);
    assert_eq!(
        classify_target_device(&props, PhoneConnectionKind::Usb),
        PhoneTargetDeviceKind::GalaxyS26Ultra
    );
}

#[test]
fn classify_redmi_tablet() {
    let props = parse_getprop(REDMI_TABLET_GETPROP);
    assert_eq!(
        classify_target_device(&props, PhoneConnectionKind::WirelessDebugging),
        PhoneTargetDeviceKind::RedmiTablet
    );
}

#[test]
fn classify_emulator_by_characteristics() {
    let props = parse_getprop(EMULATOR_GETPROP);
    assert_eq!(
        classify_target_device(&props, PhoneConnectionKind::Usb),
        PhoneTargetDeviceKind::Emulator
    );
}

#[test]
fn classify_emulator_by_connection_kind() {
    let props = DeviceProperties::default();
    assert_eq!(
        classify_target_device(&props, PhoneConnectionKind::Emulator),
        PhoneTargetDeviceKind::Emulator
    );
}

#[test]
fn classify_unknown_for_generic_phone() {
    let props = parse_getprop(
        "[ro.product.manufacturer]: [OnePlus]\n[ro.product.brand]: [OnePlus]\n[ro.product.model]: [CPH2581]\n[ro.build.characteristics]: [phone]\n",
    );
    assert_eq!(
        classify_target_device(&props, PhoneConnectionKind::Usb),
        PhoneTargetDeviceKind::UnknownAndroid
    );
}

#[test]
fn classify_redmi_phone_is_not_tablet() {
    let props = parse_getprop(
        "[ro.product.manufacturer]: [Xiaomi]\n[ro.product.brand]: [Redmi]\n[ro.product.model]: [Redmi Note 13]\n[ro.build.characteristics]: [phone]\n",
    );
    assert_eq!(
        classify_target_device(&props, PhoneConnectionKind::Usb),
        PhoneTargetDeviceKind::UnknownAndroid
    );
}

#[test]
fn device_owner_detected_and_negated() {
    assert!(parse_device_owner(
        "Current Device Policy Manager state:\n  Device Owner: ComponentInfo{com.x/.Admin}\n"
    ));
    assert!(parse_device_owner(
        "  Device Owner: com.example.mdm/.DeviceAdminReceiver\n"
    ));
    assert!(!parse_device_owner("Device owner: null"));
    assert!(!parse_device_owner("no policy info here"));
    // Regression: Samsung/Android 16 `dumpsys device_policy` reports a
    // `Device Owner Type:` line with no actual owner. The old substring check
    // misfired on it; this must NOT be read as an owner.
    assert!(!parse_device_owner(
        "  Device provisioned: true\n  Device Owner Type: -1\n  Has PO:\n"
    ));
}

fn galaxy_runner(serial: &str) -> FakeCommandRunner {
    let runner = FakeCommandRunner::new();
    runner.set_output(
        "adb",
        &["-s", serial, "shell", "getprop"],
        CommandOutput {
            status: Some(0),
            stdout: GALAXY_GETPROP.as_bytes().to_vec(),
            stderr: Vec::new(),
        },
    );
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "wm", "size"],
        "Physical size: 1440x3120\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "wm", "density"],
        "Physical density: 600\n",
    );
    // Live rotation probe: held in portrait (SurfaceOrientation 0).
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "dumpsys", "input"],
        "    SurfaceOrientation: 0\n",
    );
    runner.set_output(
        "adb",
        &["-s", serial, "shell", "su", "-c", "id"],
        CommandOutput {
            status: Some(1),
            stdout: Vec::new(),
            stderr: b"su: not found".to_vec(),
        },
    );
    runner.set_stdout(
        "adb",
        &[
            "-s",
            serial,
            "shell",
            "pm",
            "path",
            "moe.shizuku.privileged.api",
        ],
        "",
    );
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "dumpsys", "device_policy"],
        "Device Owner: null\n",
    );
    runner
}

#[tokio::test]
async fn detect_profile_populates_device_facts() {
    let serial = "R5CT30ABCDE";
    let runner = galaxy_runner(serial);
    let profile = detect_profile(
        &runner,
        "sess-1",
        serial,
        "com.skycua.phonecompanion",
        12_345,
        PhoneCapabilityRefreshState::Detected,
    )
    .await;
    assert_eq!(profile.session_id, "sess-1");
    assert_eq!(profile.serial, serial);
    assert_eq!(profile.detected_at_ms, 12_345);
    assert!(!profile.stale);
    assert_eq!(profile.manufacturer.as_deref(), Some("samsung"));
    assert_eq!(profile.model.as_deref(), Some("SM-S948B"));
    assert_eq!(profile.android_sdk, Some(36));
    assert_eq!(profile.android_release.as_deref(), Some("16"));
    assert_eq!(
        profile.target_device_kind,
        PhoneTargetDeviceKind::GalaxyS26Ultra
    );
    assert_eq!(profile.connection_kind, PhoneConnectionKind::Usb);
    let display = profile.display_size.expect("display size");
    assert_eq!(display.width, 1440);
    assert_eq!(display.height, 3120);
    assert_eq!(profile.density_dpi, Some(600));
    assert_eq!(profile.orientation.as_deref(), Some("portrait"));
    // The exact quarter is preserved alongside the label: ROTATION_0 → 0 degrees.
    assert_eq!(profile.display_rotation_degrees, Some(0));
    assert!(!profile.root_available);
    assert!(!profile.shizuku_available);
    assert!(!profile.device_owner);
    assert!(!profile.companion.installed);
    assert!(!profile.scrcpy.installed);
}

#[tokio::test]
async fn detect_profile_orientation_uses_live_rotation_over_aspect() {
    // The panel is portrait-aspect (1440x3120 → aspect says "portrait"), but the
    // device is physically rotated to landscape (SurfaceOrientation 1). The live
    // probe must win so the downstream rotation comparison sees the real state.
    let serial = "R5CT30ABCDE";
    let runner = galaxy_runner(serial);
    // Override the portrait rotation the helper scripts with a landscape one.
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "dumpsys", "input"],
        "    SurfaceOrientation: 1\n",
    );
    let profile = detect_profile(
        &runner,
        "sess-rot",
        serial,
        "com.skycua.phonecompanion",
        1,
        PhoneCapabilityRefreshState::Detected,
    )
    .await;
    // Display geometry is still the natural (portrait) resolution.
    let display = profile.display_size.expect("display size");
    assert_eq!(display.width, 1440);
    assert_eq!(display.height, 3120);
    // Orientation reflects the live landscape rotation, not the panel aspect.
    assert_eq!(profile.orientation.as_deref(), Some("landscape"));
    // ROTATION_90: the exact quarter is preserved.
    assert_eq!(profile.display_rotation_degrees, Some(90));
}

#[tokio::test]
async fn detect_profile_preserves_rotation_270_quarter() {
    // ROTATION_270 (SurfaceOrientation 3) is the seam-line case: it shares the
    // "landscape" label with ROTATION_90, so the label alone cannot tell them
    // apart. The profile must carry the exact 270 quarter so the downstream host
    // content-rect math does not collapse it to 90.
    let serial = "R5CT30ABCDE";
    let runner = galaxy_runner(serial);
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "dumpsys", "input"],
        "    SurfaceOrientation: 3\n",
    );
    let profile = detect_profile(
        &runner,
        "sess-rot-270",
        serial,
        "com.skycua.phonecompanion",
        1,
        PhoneCapabilityRefreshState::Detected,
    )
    .await;
    // The label is the coarse "landscape" (shared with 90)...
    assert_eq!(profile.orientation.as_deref(), Some("landscape"));
    // ...but the exact quarter is the full 270, not collapsed to 90.
    assert_eq!(profile.display_rotation_degrees, Some(270));
}

#[tokio::test]
async fn detect_profile_preserves_rotation_180_quarter() {
    // ROTATION_180 (upside-down portrait) shares the "portrait" label with
    // ROTATION_0, so only the numeric quarter distinguishes them.
    let serial = "R5CT30ABCDE";
    let runner = galaxy_runner(serial);
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "dumpsys", "input"],
        "    SurfaceOrientation: 2\n",
    );
    let profile = detect_profile(
        &runner,
        "sess-rot-180",
        serial,
        "com.skycua.phonecompanion",
        1,
        PhoneCapabilityRefreshState::Detected,
    )
    .await;
    assert_eq!(profile.orientation.as_deref(), Some("portrait"));
    assert_eq!(profile.display_rotation_degrees, Some(180));
}

#[tokio::test]
async fn detect_profile_orientation_falls_back_to_aspect_without_probe() {
    // No rotation probe scripted: the aspect-derived label is the fallback. With a
    // portrait panel that means "portrait".
    let serial = "R5CT30ABCDE";
    let runner = galaxy_runner(serial);
    // Make the rotation probe yield nothing (input and display both unrecognized).
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "dumpsys", "input"],
        "no rotation here\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", serial, "shell", "dumpsys", "display"],
        "no rotation here either\n",
    );
    let profile = detect_profile(
        &runner,
        "sess-fallback",
        serial,
        "com.skycua.phonecompanion",
        1,
        PhoneCapabilityRefreshState::Detected,
    )
    .await;
    assert_eq!(profile.orientation.as_deref(), Some("portrait"));
    // No live probe means no exact quarter; consumers fall back to the label.
    assert_eq!(profile.display_rotation_degrees, None);
}

#[tokio::test]
async fn detect_profile_degrades_to_identifiers_without_a_device() {
    let runner = FakeCommandRunner::new();
    let profile = detect_profile(
        &runner,
        "sess-x",
        "emulator-5554",
        "com.skycua.phonecompanion",
        999,
        PhoneCapabilityRefreshState::Detected,
    )
    .await;
    assert_eq!(profile.serial, "emulator-5554");
    assert!(profile.manufacturer.is_none());
    assert!(profile.display_size.is_none());
    assert_eq!(profile.connection_kind, PhoneConnectionKind::Emulator);
    assert_eq!(profile.target_device_kind, PhoneTargetDeviceKind::Emulator);
}

#[test]
fn property_keys_are_stable() {
    assert!(DEVICE_PROPERTY_KEYS.contains(&"ro.product.model"));
    assert!(DEVICE_PROPERTY_KEYS.contains(&"ro.build.version.sdk"));
}

#[test]
fn empty_backend_capabilities_are_all_false() {
    let caps = empty_backend_capabilities();
    assert!(!caps.adb && !caps.companion && !caps.scrcpy && !caps.screenshot);
}
