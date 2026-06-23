//! Integration tests for the wired `PhoneManager`: real session lifecycle,
//! deterministic backend routing, snapshot validation, and capability freshness,
//! all driven through a scripted [`FakeCommandRunner`] so no real device is
//! touched.

use std::sync::Arc;

use sky_cua_platform::config::{PhoneConfig, resolve_phone_selection};
use sky_cua_platform::model::{
    PhoneBackendKind, PhoneCapabilityRefreshState, PhoneConnectRequest, PhoneConnectionKind,
    PhoneObserveRequest, PhoneRequest, PhoneResponse, PhoneScreenshotRequest, PhoneSessionSelector,
    PhoneTapRequest, PhoneTypeTextRequest, PixelSize,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::PhoneManager;
use crate::phone::command::FakeCommandRunner;

const SERIAL: &str = "emulator-5554";
const COMPANION_TOKEN: &str = "app-op-token";
const COMPANION_CERT_SHA256: &str =
    "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";

/// A valid PNG of the given size, used as the scripted `screencap` payload.
fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

/// A minimal [`WindowInfo`] for the window-adoption matching tests: only the
/// title and pid are load-bearing for `find_adoptable_scrcpy_window`.
fn window_info_for_tests(
    window_id: &str,
    title: Option<&str>,
    pid: Option<u32>,
) -> sky_cua_platform::model::WindowInfo {
    sky_cua_platform::model::WindowInfo {
        window_id: window_id.to_string(),
        title: title.map(str::to_string),
        app_id: None,
        wm_class: None,
        pid,
        bounds: None,
        display: None,
        display_intersections: Vec::new(),
        workspace: None,
        focused: false,
        hidden: false,
        client_type: None,
        backend: "test".to_string(),
        terminal: None,
    }
}

/// Build an ADB-only manager (companion disabled) over a scripted fake runner
/// that makes `phone_connect` for [`SERIAL`] succeed against a fake emulator.
fn adb_only_manager() -> (PhoneManager, Arc<FakeCommandRunner>) {
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);

    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.companion_enabled = false;
    selection.companion_auto_install = false;
    selection.capability_cache_ttl_ms = 30_000;
    let manager = PhoneManager::with_runner(runner.clone(), selection);
    (manager, runner)
}

/// Script the device-property/geometry/privileged probes a `detect_profile`
/// pass runs for [`SERIAL`], shared by the ADB-only and companion managers.
fn script_device_probes(runner: &FakeCommandRunner) {
    runner.set_stdout("adb", &["version"], "Android Debug Bridge version 1.0.41");
    runner.set_stdout(
        "adb",
        &["devices"],
        "List of devices attached\nemulator-5554\tdevice\n",
    );
    runner.set_stdout(
        "adb",
        &["devices", "-l"],
        "List of devices attached\nemulator-5554          device product:sdk_gphone model:Pixel transport_id:1\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "getprop"],
        "[ro.product.manufacturer]: [Google]\n[ro.product.brand]: [google]\n[ro.product.model]: [Pixel]\n[ro.product.device]: [generic]\n[ro.build.version.sdk]: [34]\n[ro.build.version.release]: [14]\n[ro.build.characteristics]: [emulator]\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "wm", "size"],
        "Physical size: 1080x2400\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "wm", "density"],
        "Physical density: 440\n",
    );
    runner.set_stdout("adb", &["-s", SERIAL, "shell", "su", "-c", "id"], "");
    runner.set_stdout(
        "adb",
        &[
            "-s",
            SERIAL,
            "shell",
            "pm",
            "path",
            "moe.shizuku.privileged.api",
        ],
        "",
    );
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "dumpsys", "device_policy"],
        "Device policy: none\n",
    );
    // Companion secure-settings services already enabled (steady state): an
    // install-bearing bootstrap reads these and finds nothing to do, so its
    // permission-enable step is silent in manager tests. The enable/merge paths
    // are unit-tested in `adb::permissions`.
    runner.set_stdout(
        "adb",
        &[
            "-s",
            SERIAL,
            "shell",
            "settings",
            "get",
            "secure",
            "enabled_accessibility_services",
        ],
        "com.skycua.phonecompanion/com.skycua.phonecompanion.service.SkyAccessibilityService\n",
    );
    runner.set_stdout(
        "adb",
        &[
            "-s",
            SERIAL,
            "shell",
            "settings",
            "get",
            "secure",
            "enabled_notification_listeners",
        ],
        "com.skycua.phonecompanion/com.skycua.phonecompanion.service.SkyNotificationListenerService\n",
    );
    // The notification listener is (re-)asserted via `cmd notification
    // allow_listener` on every install-bearing bootstrap to force the bind.
    runner.set_stdout(
        "adb",
        &[
            "-s",
            SERIAL,
            "shell",
            "cmd notification allow_listener 'com.skycua.phonecompanion/com.skycua.phonecompanion.service.SkyNotificationListenerService'",
        ],
        "",
    );
}

fn script_verified_installed_companion(
    runner: &FakeCommandRunner,
    package: &str,
    version_code: u64,
    version_name: &str,
) {
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "pm", "path", package],
        "package:/data/app/companion.apk",
    );
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "dumpsys", "package", package],
        &format!(
            "versionCode={version_code}\nversionName={version_name}\nSHA-256 cert digest: {COMPANION_CERT_SHA256}\n"
        ),
    );
}

fn script_installed_companion_without_cert(
    runner: &FakeCommandRunner,
    package: &str,
    version_code: u64,
    version_name: &str,
) {
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "pm", "path", package],
        "package:/data/app/companion.apk",
    );
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "dumpsys", "package", package],
        &format!("versionCode={version_code}\nversionName={version_name}\n"),
    );
}

/// A companion-enabled manager (auto-install on) over a scripted runner. The
/// companion package is reported as not installed (so the bootstrap decides
/// `Install`), and the install command is scripted by the caller so each test
/// can drive success vs. an `INSTALL_FAILED_*` failure. The RPC capability probe
/// has no server to reach, so the session degrades to ADB baseline while still
/// capturing the install outcome diagnostic.
fn companion_manager() -> (PhoneManager, Arc<FakeCommandRunner>) {
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    // Companion not installed -> decide_install == Install.
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    runner.set_stdout("adb", &["-s", SERIAL, "shell", "pm", "path", &package], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    selection.capability_cache_ttl_ms = 30_000;
    let manager = PhoneManager::with_runner(runner.clone(), selection);
    (manager, runner)
}

fn selector(session_id: &str) -> PhoneSessionSelector {
    PhoneSessionSelector {
        session_id: Some(session_id.to_string()),
        serial: None,
    }
}

async fn connect(manager: &mut PhoneManager) -> sky_cua_platform::model::PhoneSession {
    match manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some(SERIAL.to_string()),
            backend: None,
            install_companion: false,
            start_scrcpy: false,
        }))
        .await
    {
        PhoneResponse::Connected(session) => session,
        other => panic!("connect did not return a session: {other:?}"),
    }
}

#[tokio::test]
async fn connect_builds_a_real_session_and_profile() {
    let (mut manager, _runner) = adb_only_manager();
    let session = connect(&mut manager).await;

    assert_eq!(session.serial, SERIAL);
    assert_eq!(session.connection_kind, PhoneConnectionKind::Emulator);
    // The profile carries the probed device facts, not a fabricated stub.
    assert_eq!(session.capability_profile.model.as_deref(), Some("Pixel"));
    assert_eq!(session.capability_profile.android_sdk, Some(34));
    assert_eq!(
        session.capability_profile.display_size,
        Some(sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        })
    );
    // ADB-only: no companion, baseline capabilities present.
    assert!(session.capabilities.adb);
    assert!(!session.capabilities.companion);
    assert!(
        !session.capabilities.gestures,
        "quick capabilities must not advertise coordinate input without a companion gesture lane"
    );
    assert!(!session.capability_profile.companion.rpc_reachable);
    // The action menu is tailored: companion-gated actions are unavailable,
    // including coordinate gestures now that visible feedback is part of the
    // coordinate-action contract.
    assert!(
        session
            .capability_profile
            .unavailable_actions
            .iter()
            .any(|a| a.action == "phone_accessibility_tree")
    );
    assert!(
        session
            .capability_profile
            .unavailable_actions
            .iter()
            .any(|a| a.action == "phone_tap")
    );
}

#[tokio::test]
async fn status_reports_active_session_after_connect() {
    let (mut manager, _runner) = adb_only_manager();
    let session = connect(&mut manager).await;

    match manager
        .handle(PhoneRequest::Status(
            sky_cua_platform::model::PhoneStatusRequest::default(),
        ))
        .await
    {
        PhoneResponse::Status(report) => {
            assert_eq!(report.sessions.len(), 1);
            assert_eq!(report.sessions[0].session_id, session.session_id);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn status_refresh_devices_populates_device_list() {
    let (mut manager, _runner) = adb_only_manager();

    match manager
        .handle(PhoneRequest::Status(
            sky_cua_platform::model::PhoneStatusRequest {
                refresh_devices: true,
            },
        ))
        .await
    {
        PhoneResponse::Status(report) => {
            assert_eq!(report.devices.len(), 1);
            assert_eq!(report.devices[0].serial, SERIAL);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn disabled_phone_use_blocks_connect_side_effects() {
    let (mut manager, _runner) = adb_only_manager();
    manager.selection.enabled = false;

    match manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest::default()))
        .await
    {
        PhoneResponse::Status(report) => {
            assert!(!report.enabled);
            assert!(report.sessions.is_empty());
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|diag| diag.code == "PhoneUseDisabled"),
                "disabled response must carry a structured diagnostic: {:?}",
                report.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn connect_rejects_serial_absent_from_device_list() {
    let (mut manager, _runner) = adb_only_manager();
    let resp = manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some("phone-smoke-nonexistent-serial".to_string()),
            backend: None,
            install_companion: false,
            start_scrcpy: false,
        }))
        .await;
    match resp {
        PhoneResponse::Status(report) => {
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneDeviceUnavailable"),
                "expected PhoneDeviceUnavailable diagnostic, got {:?}",
                report.diagnostics
            );
            assert!(
                report.sessions.is_empty(),
                "no session should be minted for a bogus serial"
            );
        }
        other => panic!("expected Status rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn connect_is_idempotent_for_same_serial() {
    let (mut manager, _runner) = adb_only_manager();
    let first = connect(&mut manager).await;
    let second = connect(&mut manager).await;
    // Same serial reuses the session id rather than minting a second session.
    assert_eq!(first.session_id, second.session_id);
}

#[tokio::test]
async fn connect_forced_adb_bootstraps_installed_companion_but_keeps_adb_dispatch() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    selection.companion_apk_path = "../../resources/android/phone-companion.apk".to_string();
    script_installed_companion_without_cert(&runner, &package, 1, "1.0.0");
    let apk = selection.companion_apk_path.clone();
    runner.set_stdout("adb", &["-s", SERIAL, "install", "-r", &apk], "Success");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = false;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);

    match manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some(SERIAL.to_string()),
            backend: Some(PhoneBackendKind::Adb),
            install_companion: false,
            start_scrcpy: false,
        }))
        .await
    {
        PhoneResponse::Connected(session) => {
            assert_eq!(session.backend, PhoneBackendKind::Adb);
            assert!(
                session.capability_profile.companion.rpc_reachable,
                "forced adb input must still bootstrap an installed companion: {:?}",
                session.capability_profile.companion
            );
            assert!(session.capability_profile.companion.native_overlay);
        }
        other => panic!("unexpected: {other:?}"),
    }

    let calls = runner.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|call| call == &format!("adb -s {SERIAL} install -r {apk}")),
        "forced adb connect must not install/update when install is not allowed: {calls:?}"
    );
    assert!(
        calls.iter().any(|call| call.contains(" forward tcp:")),
        "forced adb connect must forward companion RPC: {calls:?}"
    );
    let active = server
        .request_for("overlay_active")
        .expect("forced adb connect must light the companion overlay");
    let parsed: serde_json::Value = serde_json::from_str(&active).expect("json body");
    assert_eq!(parsed["params"]["active"], serde_json::json!(true));
}

#[tokio::test]
async fn screenshot_then_tap_requires_companion_for_input() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    // ADB `screencap` returns a device-resolution frame, so the capture size
    // matches the profile's recorded display_size (1080x2400) — the realistic
    // contract the snapshot orientation/resolution guard validates against.
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );
    let snapshot_id = match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: true,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => {
            assert_eq!(shot.backend, PhoneBackendKind::Adb);
            assert_eq!(
                shot.capability_profile_id,
                session.capability_profile.profile_id
            );
            assert!(shot.inline_image.is_some());
            assert_eq!(shot.device_size.width, 1080);
            shot.phone_snapshot_id
        }
        other => panic!("unexpected: {other:?}"),
    };

    // A tap referencing the fresh snapshot still fails closed: screenshots may
    // use ADB, but coordinate input requires the companion gesture lane so the
    // device receives visible agent feedback.
    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: Some(snapshot_id),
            x: 10.0,
            y: 20.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.action, "phone_tap");
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert_eq!(
                action.capability_profile_id,
                session.capability_profile.profile_id
            );
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneCompanionRequired"),
                "{:?}",
                action.diagnostics
            );
            assert!(
                action.cursor.is_none(),
                "failed companion-required action must not move cursor"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn text_only_screenshot_persists_path_backed_png() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );

    match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => {
            assert_eq!(shot.backend, PhoneBackendKind::Adb);
            assert!(shot.inline_image.is_none());
            let path = shot
                .screenshot_path
                .as_deref()
                .expect("text-only screenshot should return a saved PNG path");
            assert!(path.ends_with(".png"));
            assert!(std::path::Path::new(path).is_file(), "{path}");
            let _ = std::fs::remove_file(path);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn forced_adb_screenshot_honors_requested_backend_with_companion_available() {
    let caps = r#"{"version":"2.0.0","version_code":20,"package":"com.skycua.phonecompanion","accessibility_enabled":true,"can_perform_gestures":true,"can_retrieve_window_content":true,"can_take_screenshot":true,"notification_listener_enabled":true,"native_overlay":true,"native_overlay_pass_through":true,"screenshot_api_level":34,"screenshot_supported":true,"gesture_supported":true}"#;
    let server = CapabilitiesCompanion::start(caps).await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 20, "2.0.0");
    let port = server.port;
    let port_arg = format!("tcp:{port}");
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );
    selection.companion_enabled = true;
    selection.companion_rpc_port = port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);
    let session = connect(&mut manager).await;
    assert!(session.capability_profile.companion.screenshot);

    match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: Some(PhoneBackendKind::Adb),
            include_image_data: false,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => {
            assert_eq!(shot.backend, PhoneBackendKind::Adb);
            assert!(shot.diagnostics.is_empty(), "{:?}", shot.diagnostics);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn direct_device_coordinates_reject_nonfinite_and_out_of_bounds() {
    let (mut manager, _runner) = adb_only_manager();
    let session = connect(&mut manager).await;

    for (x, y, code) in [
        (f64::NAN, 1.0, "PhoneMappingNonFinite"),
        (1080.0, 1.0, "PhoneMappingOutOfBounds"),
    ] {
        match manager
            .handle(PhoneRequest::Tap(PhoneTapRequest {
                session: selector(&session.session_id),
                phone_snapshot_id: None,
                x,
                y,
                use_device_coordinates: true,
            }))
            .await
        {
            PhoneResponse::Action(action) => {
                assert_eq!(action.backend, PhoneBackendKind::None);
                assert!(
                    action.diagnostics.iter().any(|d| d.code == code),
                    "expected {code}, got {:?}",
                    action.diagnostics
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}

#[tokio::test]
async fn screenshot_with_undecodable_png_is_a_structured_failure() {
    // A truncated/non-PNG `screencap` payload (e.g. a partial pull over a flaky
    // wireless link) must not become a degenerate 0x0 "successful" screenshot. It
    // routes through `screenshot_failure` with `PhoneScreencapDecodeFailed`, no
    // backend, and no registered snapshot the agent could later act against.
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: b"not a png at all".to_vec(),
            stderr: Vec::new(),
        },
    );

    let snapshot_id = match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: true,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => {
            assert_eq!(shot.backend, PhoneBackendKind::None);
            assert!(shot.phone_snapshot_id.is_empty());
            assert!(shot.inline_image.is_none());
            assert!(
                shot.diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneScreencapDecodeFailed"),
                "expected a decode-failure diagnostic, got {:?}",
                shot.diagnostics
            );
            // The failure response carries no fabricated 0x0 device size from the
            // undecodable bytes; it falls back to the profile's known display size.
            assert_ne!(shot.device_size.width, 0);
            shot.phone_snapshot_id
        }
        other => panic!("unexpected: {other:?}"),
    };

    // No usable snapshot was registered: a tap referencing the (empty) id is
    // rejected rather than dispatched against a degenerate 0x0 frame.
    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: Some(snapshot_id),
            x: 1.0,
            y: 1.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(!action.diagnostics.is_empty());
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn tap_with_stale_snapshot_is_rejected() {
    let (mut manager, runner) = adb_only_manager();
    // A tiny snapshot TTL so the snapshot ages out immediately.
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(200, 400),
            stderr: Vec::new(),
        },
    );

    let snapshot_id = match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => shot.phone_snapshot_id,
        other => panic!("unexpected: {other:?}"),
    };

    // Use an obviously unknown id: it must be rejected with a structured snapshot
    // diagnostic, never silently dispatched.
    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: Some(format!("{snapshot_id}-mutated")),
            x: 0.0,
            y: 0.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code.starts_with("PhoneSnapshot")),
                "{:?}",
                action.diagnostics
            );
            assert!(
                action.cursor.is_none(),
                "failed action must not move cursor"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn tap_without_snapshot_requires_one() {
    let (mut manager, _runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: None,
            x: 1.0,
            y: 2.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneSnapshotRequired")
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn device_coordinate_tap_still_requires_companion() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: None,
            x: 100.0,
            y: 200.0,
            use_device_coordinates: true,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneCompanionRequired"),
                "{:?}",
                action.diagnostics
            );
            assert!(
                runner
                    .recorded_calls()
                    .iter()
                    .all(|call| !call.contains(" shell input tap ")),
                "ADB tap fallback must not be used: {:?}",
                runner.recorded_calls()
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn disconnect_removes_the_session() {
    let (mut manager, _runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    match manager
        .handle(PhoneRequest::Disconnect(
            sky_cua_platform::model::PhoneDisconnectRequest {
                session: selector(&session.session_id),
                keep_wireless: false,
            },
        ))
        .await
    {
        PhoneResponse::Disconnected(response) => {
            assert!(response.disconnected);
            assert_eq!(response.session_id, session.session_id);
        }
        other => panic!("unexpected: {other:?}"),
    }
    // A follow-up action on the now-gone session reports no session.
    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: None,
            x: 1.0,
            y: 1.0,
            use_device_coordinates: true,
        }))
        .await
    {
        PhoneResponse::Action(action) => assert!(
            action
                .diagnostics
                .iter()
                .any(|d| d.code == "PhoneNoSession")
        ),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn refresh_capabilities_rebuilds_the_profile() {
    let (mut manager, _runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    match manager
        .handle(PhoneRequest::RefreshCapabilities(
            sky_cua_platform::model::PhoneRefreshCapabilitiesRequest {
                session: selector(&session.session_id),
            },
        ))
        .await
    {
        PhoneResponse::Capabilities(profile) => {
            assert_eq!(profile.serial, SERIAL);
            assert_eq!(
                profile.refresh_state,
                sky_cua_platform::model::PhoneCapabilityRefreshState::Refreshed
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Freshness semantics: the connect that detects a profile reports `Detected`,
/// but the next request that reuses the still-fresh cached profile (here a
/// `phone_screenshot` within TTL) reports `Reused`. The stored cache value stays
/// `Detected`; only the per-request clone flips.
#[tokio::test]
async fn within_ttl_reuse_reports_reused_state() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    // The connect response itself carries the freshly-detected state.
    assert_eq!(
        session.capability_profile.refresh_state,
        PhoneCapabilityRefreshState::Detected
    );

    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );

    // A second request within TTL reuses the cached profile and reports Reused.
    match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => {
            assert_eq!(
                shot.profile_refresh_state,
                PhoneCapabilityRefreshState::Reused,
                "a within-TTL reuse must report Reused, not the cached Detected"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }

    // The stored cache value is untouched: still Detected at its source.
    let stored = manager
        .cached_profile_for_tests(&session.session_id, super::now_ms())
        .expect("cached profile");
    // (cached_profile flips the clone to Reused too, but the underlying detected
    // state has not been mutated — a fresh refresh would still read Detected.)
    assert_eq!(
        stored.refresh_state,
        PhoneCapabilityRefreshState::Reused,
        "the per-request clone reports Reused within TTL"
    );
}

/// Freshness semantics: a within-TTL `phone_observe` (the primary perception
/// tool) likewise reports `Reused` rather than the cached `Detected`.
#[tokio::test]
async fn within_ttl_observe_reports_reused_state() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );

    match manager
        .handle(PhoneRequest::Observe(PhoneObserveRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
            include_accessibility: false,
            include_notifications: false,
        }))
        .await
    {
        PhoneResponse::Observe(observe) => {
            assert_eq!(
                observe.profile_refresh_state,
                PhoneCapabilityRefreshState::Reused,
                "a within-TTL observe must report Reused"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn stale_observe_refreshes_profile_before_reporting_actions() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );
    selection.companion_enabled = true;
    selection.companion_auto_install = false;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    selection.capability_cache_ttl_ms = 30_000;
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);
    let detected_at = PhoneManager::now_ms_for_tests().saturating_sub(60_000);
    manager.insert_companion_session_for_tests(
        "sess-stale",
        SERIAL,
        server.port,
        COMPANION_TOKEN,
        detected_at,
    );

    match manager
        .handle(PhoneRequest::Observe(PhoneObserveRequest {
            session: selector("sess-stale"),
            backend: None,
            include_image_data: false,
            include_accessibility: false,
            include_notifications: false,
        }))
        .await
    {
        PhoneResponse::Observe(observe) => {
            assert_eq!(
                observe.profile_refresh_state,
                PhoneCapabilityRefreshState::Reused,
                "stale observe should silently refresh before reporting actions"
            );
            let tap = observe
                .available_actions
                .iter()
                .find(|action| action.action == "phone_tap")
                .expect("phone_tap should remain available after refresh");
            assert_eq!(
                tap.backend,
                PhoneBackendKind::Companion,
                "refreshed profile must keep pointer actions companion-backed"
            );
            assert!(
                observe
                    .unavailable_actions
                    .iter()
                    .all(|action| action.action != "phone_tap" && action.action != "phone_swipe"),
                "refreshed profile must not report pointer actions unavailable: {:?}",
                observe.unavailable_actions
            );
            assert!(
                runner
                    .recorded_calls()
                    .iter()
                    .all(|call| !call.contains(".SetupActivity")),
                "stale observe must not foreground the companion setup UI: {:?}",
                runner.recorded_calls()
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Drift invalidation: a captured frame whose dimensions differ from the cached
/// display's rotation-adjusted expected screenshot extent marks the cached
/// profile stale, so subsequent routing re-proves backends.
#[tokio::test]
async fn capture_with_drifted_size_marks_profile_stale() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    // The connected profile recorded display_size 1080x2400 and rotation 0. A
    // capture with different dimensions is a live resolution/orientation drift.
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1440, 2400),
            stderr: Vec::new(),
        },
    );

    let before = manager
        .cached_profile_for_tests(&session.session_id, super::now_ms())
        .expect("cached profile");
    assert!(!before.stale, "profile starts fresh");

    let response = manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await;
    match response {
        PhoneResponse::Screenshot(shot) => {
            assert_eq!(
                shot.phone_snapshot_id, "",
                "a drifted capture must not advertise an actionable snapshot"
            );
            assert!(
                shot.diagnostics
                    .iter()
                    .any(|diag| diag.code == "PhoneCapabilityProfileDrifted"),
                "drift must be visible as a structured refresh-required diagnostic"
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let after = manager
        .cached_profile_for_tests(&session.session_id, super::now_ms())
        .expect("cached profile");
    assert!(
        after.stale,
        "a capture whose size drifts from the cached display_size must mark stale"
    );
    assert_eq!(after.refresh_state, PhoneCapabilityRefreshState::Stale);
}

/// A stored stale profile means the device/profile facts themselves need to be
/// re-detected (for example after display drift), not merely that the companion
/// TTL expired. A live companion RPC refresh must not clear that stored stale bit
/// without re-running the full device profile probe.
#[tokio::test]
async fn stored_stale_profile_rebuilds_instead_of_live_companion_refresh() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = false;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);
    let detected_at = PhoneManager::now_ms_for_tests();
    manager.insert_companion_session_for_tests(
        "sess-drift",
        SERIAL,
        server.port,
        COMPANION_TOKEN,
        detected_at,
    );

    let cached = manager
        .profiles
        .get_mut("sess-drift")
        .expect("cached profile");
    cached.profile.stale = true;
    cached.profile.refresh_state = PhoneCapabilityRefreshState::Stale;

    let ctx = manager
        .fresh_action_context(&selector("sess-drift"))
        .await
        .expect("fresh context after rebuild");
    assert!(
        !ctx.profile.stale,
        "full rebuild should clear stored drift stale"
    );

    let calls = runner.recorded_calls();
    assert!(
        calls.iter().any(|call| call.contains(".SetupActivity")),
        "stored drift stale must take the full bootstrap/rebuild path, not only live capabilities RPC: {calls:?}"
    );
    assert!(
        server
            .recorded()
            .iter()
            .any(|request| request.contains("\"method\":\"capabilities\"")),
        "rebuild still proves the companion after setup"
    );
}

/// Text/key operations are ADB-only in v1. They must not run the foregrounding
/// companion bootstrap before dispatching, or the text/key event can land in the
/// companion setup UI instead of the current target app.
#[tokio::test]
async fn stale_keyboard_action_does_not_launch_setup_activity() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = false;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);
    let session = connect(&mut manager).await;

    if let Some(entry) = manager.sessions.get_mut(&session.session_id) {
        entry.companion = None;
    }
    let cached = manager
        .profiles
        .get_mut(&session.session_id)
        .expect("cached profile");
    cached.detected_at_ms = super::now_ms().saturating_sub(60_000);
    runner.set_stdout("adb", &["-s", SERIAL, "shell", "input text 'hello'"], "");
    let before_call_count = runner.recorded_calls().len();

    match manager
        .handle(PhoneRequest::TypeText(PhoneTypeTextRequest {
            session: selector(&session.session_id),
            text: "hello".to_string(),
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::Adb);
            assert!(action.diagnostics.is_empty(), "{action:?}");
            assert_eq!(
                action.profile_refresh_state,
                PhoneCapabilityRefreshState::Stale
            );
        }
        other => panic!("unexpected: {other:?}"),
    }

    let new_calls = &runner.recorded_calls()[before_call_count..];
    assert!(
        new_calls
            .iter()
            .any(|call| call == &format!("adb -s {SERIAL} shell input text 'hello'")),
        "keyboard action should still dispatch through ADB: {new_calls:?}"
    );
    assert!(
        new_calls
            .iter()
            .all(|call| !call.contains(".SetupActivity")),
        "keyboard action must not foreground companion setup before typing: {new_calls:?}"
    );
}

/// Snapshot-invalid coordinate actions should fail before any stale capability
/// refresh can launch the companion setup activity. They cannot dispatch anyway,
/// so setup would only steal foreground.
#[tokio::test]
async fn stale_snapshotless_tap_does_not_launch_setup_activity() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = false;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);
    let session = connect(&mut manager).await;

    if let Some(entry) = manager.sessions.get_mut(&session.session_id) {
        entry.companion = None;
    }
    let cached = manager
        .profiles
        .get_mut(&session.session_id)
        .expect("cached profile");
    cached.detected_at_ms = super::now_ms().saturating_sub(60_000);
    let before_call_count = runner.recorded_calls().len();

    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: None,
            x: 10.0,
            y: 20.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|diag| diag.code == "PhoneSnapshotRequired"),
                "{action:?}"
            );
            assert_eq!(
                action.profile_refresh_state,
                PhoneCapabilityRefreshState::Stale
            );
        }
        other => panic!("unexpected: {other:?}"),
    }

    let new_calls = &runner.recorded_calls()[before_call_count..];
    assert!(
        new_calls.is_empty(),
        "snapshotless tap must fail before setup or input side effects: {new_calls:?}"
    );
}

#[tokio::test]
async fn observe_capture_failure_reports_no_backend() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1440, 2400),
            stderr: Vec::new(),
        },
    );

    let response = manager
        .handle(PhoneRequest::Observe(PhoneObserveRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
            include_accessibility: false,
            include_notifications: false,
        }))
        .await;
    match response {
        PhoneResponse::Observe(observe) => {
            assert_eq!(observe.backend, PhoneBackendKind::None);
            assert!(
                observe.phone_snapshot_id.is_none(),
                "a failed capture must not advertise an actionable snapshot"
            );
            assert!(
                observe
                    .diagnostics
                    .iter()
                    .any(|diag| diag.code == "PhoneCapabilityProfileDrifted"),
                "capture failure reason must survive in observe diagnostics"
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

/// A landscape phone capture naturally swaps the `wm size` dimensions. That is
/// not profile drift when the cached live rotation already says 90/270 degrees.
#[tokio::test]
async fn capture_with_rotation_swapped_size_keeps_profile_fresh() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    manager.set_display_rotation_for_tests(&session.session_id, Some(90));
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(2400, 1080),
            stderr: Vec::new(),
        },
    );

    let response = manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await;
    match response {
        PhoneResponse::Screenshot(shot) => {
            assert!(
                !shot.phone_snapshot_id.is_empty(),
                "a rotation-matched capture must remain actionable"
            );
            assert!(
                !shot
                    .diagnostics
                    .iter()
                    .any(|diag| diag.code == "PhoneCapabilityProfileDrifted"),
                "rotation-matched landscape dimensions are not drift"
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let after = manager
        .cached_profile_for_tests(&session.session_id, super::now_ms())
        .expect("cached profile");
    assert!(!after.stale);
    assert_eq!(after.refresh_state, PhoneCapabilityRefreshState::Reused);
}

#[tokio::test]
async fn tap_accepts_landscape_snapshot_when_rotation_matches_profile() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    manager.set_display_rotation_for_tests(&session.session_id, Some(90));
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(2400, 1080),
            stderr: Vec::new(),
        },
    );
    let snapshot_id = match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => shot.phone_snapshot_id,
        other => panic!("unexpected: {other:?}"),
    };

    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: Some(snapshot_id),
            x: 10.0,
            y: 20.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneCompanionRequired"),
                "rotation-matched landscape snapshot must pass snapshot validation and fail only on companion routing: {:?}",
                action.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Drift invalidation does not fire when the captured frame matches the cached
/// `display_size`: a same-size capture leaves the profile fresh.
#[tokio::test]
async fn capture_with_matching_size_keeps_profile_fresh() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );
    let _ = manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await;
    let after = manager
        .cached_profile_for_tests(&session.session_id, super::now_ms())
        .expect("cached profile");
    assert!(
        !after.stale,
        "a same-size capture must not drift-invalidate the profile"
    );
}

/// Snapshot orientation rejection: a snapshot captured at 1080x2400 is rejected
/// with `PhoneSnapshotOrientationMismatch` once the profile reports the swapped
/// 2400x1080 (the device rotated after capture).
#[tokio::test]
async fn tap_rejects_snapshot_on_orientation_flip() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );
    let snapshot_id = match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => shot.phone_snapshot_id,
        other => panic!("unexpected: {other:?}"),
    };

    // The device rotates: the profile now reports the swapped dimensions.
    manager.set_display_size_for_tests(
        &session.session_id,
        PixelSize {
            width: 2400,
            height: 1080,
        },
    );

    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: Some(snapshot_id),
            x: 10.0,
            y: 20.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneSnapshotOrientationMismatch"),
                "an orientation flip must reject with PhoneSnapshotOrientationMismatch: {:?}",
                action.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Snapshot resolution rejection: a snapshot captured at 1080x2400 is rejected
/// with `PhoneSnapshotResolutionMismatch` once the profile reports a different,
/// non-swapped resolution (1440x3120).
#[tokio::test]
async fn tap_rejects_snapshot_on_resolution_change() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );
    let snapshot_id = match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => shot.phone_snapshot_id,
        other => panic!("unexpected: {other:?}"),
    };

    // The display resolution changes (not a clean orientation swap).
    manager.set_display_size_for_tests(
        &session.session_id,
        PixelSize {
            width: 1440,
            height: 3120,
        },
    );

    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: Some(snapshot_id),
            x: 10.0,
            y: 20.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneSnapshotResolutionMismatch"),
                "a resolution change must reject with PhoneSnapshotResolutionMismatch: {:?}",
                action.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Snapshot match: a snapshot captured at the profile's current size is accepted
/// (the orientation/resolution guard does not false-reject a matching frame).
#[tokio::test]
async fn tap_accepts_snapshot_when_size_matches_profile() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );
    let snapshot_id = match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session.session_id),
            backend: None,
            include_image_data: false,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => shot.phone_snapshot_id,
        other => panic!("unexpected: {other:?}"),
    };

    // The profile's display_size (1080x2400) matches the snapshot capture size.
    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(&session.session_id),
            phone_snapshot_id: Some(snapshot_id),
            x: 10.0,
            y: 20.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(action.backend, PhoneBackendKind::None);
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneCompanionRequired"),
                "a matching snapshot must pass snapshot validation and fail only on companion routing: {:?}",
                action.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// The companion bootstrap install path surfaces a structured success diagnostic
/// (the install-decision code) when `adb install -r` reports success, even though
/// the RPC probe has no server and the session degrades to ADB baseline.
#[tokio::test]
async fn companion_bootstrap_surfaces_install_success_outcome() {
    let (mut manager, runner) = companion_manager();
    let apk = "resources/android/phone-companion.apk";
    // adb install -r succeeds.
    runner.set_stdout("adb", &["-s", SERIAL, "install", "-r", apk], "Success");
    // adb forward succeeds so the probe is attempted (and then fails: no server).
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "forward", "tcp:47683", "tcp:47683"],
        "",
    );

    let session = connect(&mut manager).await;
    // No real companion RPC: the session degrades to ADB baseline.
    assert!(!session.capability_profile.companion.rpc_reachable);

    // The companion status surfaces the captured install outcome diagnostic.
    match manager
        .handle(PhoneRequest::CompanionStatus(
            sky_cua_platform::model::PhoneCompanionStatusRequest {
                session: selector(&session.session_id),
            },
        ))
        .await
    {
        PhoneResponse::CompanionStatus(status) => {
            assert!(
                status
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "CompanionInstall"),
                "install success outcome must be surfaced: {:?}",
                status.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// A failed companion setup intent (the `am start .SetupActivity` that delivers
/// the session token) is surfaced as a distinct `PhoneCompanionSetupIntentFailed`
/// diagnostic, so a misdelivered token reads as a setup failure rather than a
/// confusing later `unauthorized`. The token is never echoed.
#[tokio::test]
async fn companion_bootstrap_surfaces_setup_intent_failure() {
    let (mut manager, runner) = companion_manager();
    let apk = "resources/android/phone-companion.apk";
    // Install and forward succeed so the bootstrap reaches the setup intent.
    runner.set_stdout("adb", &["-s", SERIAL, "install", "-r", apk], "Success");
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "forward", "tcp:47683", "tcp:47683"],
        "",
    );
    // The setup-intent `am start .SetupActivity` carries a random token/expiry, so
    // it is not keyed: the fake runner returns a failure for the unscripted call,
    // standing in for an `am start` that did not deliver the token.

    let session = connect(&mut manager).await;
    assert!(!session.capability_profile.companion.rpc_reachable);

    match manager
        .handle(PhoneRequest::CompanionStatus(
            sky_cua_platform::model::PhoneCompanionStatusRequest {
                session: selector(&session.session_id),
            },
        ))
        .await
    {
        PhoneResponse::CompanionStatus(status) => {
            assert!(
                status
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneCompanionSetupIntentFailed"),
                "a failed setup intent must surface a distinct diagnostic: {:?}",
                status.diagnostics
            );
            // The session token must never appear in any diagnostic message.
            assert!(
                status
                    .diagnostics
                    .iter()
                    .all(|d| !d.message.to_ascii_lowercase().contains("token=")),
                "the session token must never be echoed into a diagnostic"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// A simulated `INSTALL_FAILED_*` failure is captured as a structured diagnostic
/// keyed by the adb failure class, proving the install result is no longer
/// swallowed.
#[tokio::test]
async fn companion_bootstrap_surfaces_install_failure_class() {
    let (mut manager, runner) = companion_manager();
    let apk = "resources/android/phone-companion.apk";
    // adb install -r fails with a recognizable INSTALL_FAILED_* class.
    runner.set_output(
        "adb",
        &["-s", SERIAL, "install", "-r", apk],
        crate::phone::command::CommandOutput {
            status: Some(1),
            stdout: Vec::new(),
            stderr: b"Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]".to_vec(),
        },
    );

    let session = connect(&mut manager).await;
    assert!(!session.capability_profile.companion.rpc_reachable);

    // phone_install_companion re-bootstraps and reports the failure class.
    match manager
        .handle(PhoneRequest::InstallCompanion(
            sky_cua_platform::model::PhoneInstallCompanionRequest {
                session: selector(&session.session_id),
                force_reinstall: true,
                allow_downgrade: false,
            },
        ))
        .await
    {
        PhoneResponse::Action(action) => {
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "INSTALL_FAILED_INSUFFICIENT_STORAGE"),
                "install failure class must be surfaced, not swallowed: {:?}",
                action.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// `phone_status` resolves scrcpy and probes its version into the report. A real
/// existing file stands in for the scrcpy binary so `resolve_scrcpy` returns
/// `Found`, and the fake runner scripts `<path> --version`.
#[tokio::test]
async fn status_reports_resolved_scrcpy_path_and_version() {
    // Use an existing regular file as the scrcpy binary so resolution succeeds
    // (resolve_scrcpy validates the configured path exists on disk).
    let scrcpy_path = std::env::current_exe().expect("test exe path");
    let scrcpy_path_str = scrcpy_path.to_string_lossy().into_owned();

    let runner = Arc::new(FakeCommandRunner::new());
    runner.set_stdout("adb", &["version"], "Android Debug Bridge version 1.0.41");
    runner.set_stdout("adb", &["devices"], "List of devices attached\n");
    runner.set_stdout(
        &scrcpy_path_str,
        &["--version"],
        "scrcpy 4.0 <https://github.com/Genymobile/scrcpy>\n",
    );

    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.scrcpy_path = Some(scrcpy_path_str.clone());
    let mut manager = PhoneManager::with_runner(runner, selection);

    match manager
        .handle(PhoneRequest::Status(
            sky_cua_platform::model::PhoneStatusRequest::default(),
        ))
        .await
    {
        PhoneResponse::Status(report) => {
            assert!(report.scrcpy_available);
            assert_eq!(
                report.scrcpy_path.as_deref(),
                Some(scrcpy_path_str.as_str())
            );
            assert_eq!(report.scrcpy_version.as_deref(), Some("4.0"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// `phone_connect` resolves scrcpy and probes its version into the cached
/// capability profile (idle until a mirror launches).
#[tokio::test]
async fn connect_populates_scrcpy_version_in_profile() {
    let scrcpy_path = std::env::current_exe().expect("test exe path");
    let scrcpy_path_str = scrcpy_path.to_string_lossy().into_owned();

    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    runner.set_stdout(&scrcpy_path_str, &["--version"], "scrcpy 2.7\nINFO: foo\n");

    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.companion_enabled = false;
    selection.companion_auto_install = false;
    selection.scrcpy_path = Some(scrcpy_path_str);
    let mut manager = PhoneManager::with_runner(runner, selection);

    let session = connect(&mut manager).await;
    assert!(session.capability_profile.scrcpy.installed);
    assert_eq!(
        session.capability_profile.scrcpy.version.as_deref(),
        Some("2.7")
    );
    assert!(!session.capability_profile.scrcpy.active);
}

/// A minimal in-process companion RPC server that replies to every `POST /rpc`
/// with one scripted JSON result body (echoing the request id), after validating
/// the bearer token. Returns the bound port and a shutdown guard.
struct FakeCompanion {
    port: u16,
    _shutdown: oneshot::Sender<()>,
}

impl FakeCompanion {
    async fn start(result_json: &'static str) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind fake companion");
        let port = listener.local_addr().expect("addr").port();
        let (tx, mut rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        tokio::spawn(async move {
                            let mut buf = Vec::new();
                            let mut chunk = [0u8; 4096];
                            // Read until headers are complete, then drain the body.
                            loop {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => n,
                                };
                                buf.extend_from_slice(&chunk[..n]);
                                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            let text = String::from_utf8_lossy(&buf);
                            let id = text
                                .split("\"id\":")
                                .nth(1)
                                .and_then(|s| s.split([',', '}']).next())
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .unwrap_or(1);
                            let has_token = text.contains(COMPANION_TOKEN);
                            let body = if has_token {
                                format!(
                                    r#"{{"protocol_version":1,"ok":true,"id":{id},"result":{result_json}}}"#
                                )
                            } else {
                                format!(
                                    r#"{{"protocol_version":1,"ok":false,"id":{id},"error":{{"code":"unauthorized","message":"bad token"}}}}"#
                                )
                            };
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.flush().await;
                        });
                    }
                }
            }
        });
        Self {
            port,
            _shutdown: tx,
        }
    }
}

/// A method-aware fake companion: serves a per-method error for `capabilities`
/// (modeling an older companion that lacks it) and a scripted ok body for every
/// other method (notably `health`). Validates the bearer token like the real app.
struct MethodAwareCompanion {
    port: u16,
    _shutdown: oneshot::Sender<()>,
}

impl MethodAwareCompanion {
    async fn start(capabilities_error_code: &'static str, health_result: &'static str) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let (tx, mut rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        tokio::spawn(async move {
                            let mut buf = Vec::new();
                            let mut chunk = [0u8; 4096];
                            let header_end = loop {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => n,
                                };
                                buf.extend_from_slice(&chunk[..n]);
                                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    break p;
                                }
                            };
                            // Drain the body using Content-Length.
                            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                            let content_length = headers
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            let mut body = buf[header_end + 4..].to_vec();
                            while body.len() < content_length {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => n,
                                };
                                body.extend_from_slice(&chunk[..n]);
                            }
                            let text = String::from_utf8_lossy(&body);
                            let id = text
                                .split("\"id\":")
                                .nth(1)
                                .and_then(|s| s.split([',', '}']).next())
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .unwrap_or(1);
                            let is_capabilities = text.contains("\"method\":\"capabilities\"");
                            let resp_body = if is_capabilities {
                                format!(
                                    r#"{{"protocol_version":1,"ok":false,"id":{id},"error":{{"code":"{capabilities_error_code}","message":"no capabilities method"}}}}"#
                                )
                            } else {
                                format!(
                                    r#"{{"protocol_version":1,"ok":true,"id":{id},"result":{health_result}}}"#
                                )
                            };
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                resp_body.len(),
                                resp_body
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.flush().await;
                        });
                    }
                }
            }
        });
        Self {
            port,
            _shutdown: tx,
        }
    }
}

/// When the companion is reachable but cannot serve the richer `capabilities`
/// method, the bootstrap falls back to a `health` probe and builds capabilities
/// from it (the `capabilities_from_health` path), reporting a reachable companion.
#[tokio::test]
async fn bootstrap_falls_back_to_health_when_capabilities_unavailable() {
    let health = r#"{"version":"1.0.0","version_code":1,"package":"com.skycua.phonecompanion","accessibility_enabled":true,"can_perform_gestures":true,"can_retrieve_window_content":true,"can_take_screenshot":true,"notification_listener_enabled":false,"native_overlay":true,"native_overlay_pass_through":true}"#;
    let server = MethodAwareCompanion::start("unsupported_api", health).await;

    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    // Companion already installed with a verified matching cert -> UpToDate; no
    // install command needed.
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port = server.port;
    let port_arg = format!("tcp:{port}");
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    selection.companion_rpc_port = port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner, selection);

    let session = connect(&mut manager).await;
    let companion = &session.capability_profile.companion;
    assert!(
        companion.rpc_reachable,
        "health fallback must keep the companion reachable: {:?}",
        session.capability_profile.companion
    );
    // capabilities_from_health derives gesture_dispatch/screenshot from the raw
    // permission booleans (no support detail available).
    assert!(companion.gesture_dispatch);
    assert!(companion.screenshot);
    assert!(!companion.notifications);
}

/// Regression: a connect with the companion ENABLED and already installed must
/// bootstrap (forward + token + RPC probe) even when auto-install is OFF, so an
/// installed companion is connected instead of being silently left on the ADB
/// baseline. Install/update stays gated by auto-install; the connect does not.
#[tokio::test]
async fn connect_bootstraps_enabled_companion_even_without_auto_install() {
    let health = r#"{"version":"1.0.0","version_code":1,"package":"com.skycua.phonecompanion","accessibility_enabled":true,"can_perform_gestures":true,"can_retrieve_window_content":true,"can_take_screenshot":true,"notification_listener_enabled":false,"native_overlay":true,"native_overlay_pass_through":true}"#;
    let server = MethodAwareCompanion::start("unsupported_api", health).await;

    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port = server.port;
    let port_arg = format!("tcp:{port}");
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    // The crux of the regression: auto-install OFF must NOT skip the bootstrap.
    selection.companion_auto_install = false;
    selection.companion_rpc_port = port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner, selection);

    let session = connect(&mut manager).await;
    assert!(
        session.capability_profile.companion.rpc_reachable,
        "enabled+installed companion must connect even with auto_install off: {:?}",
        session.capability_profile.companion
    );
}

/// A companion-routed `phone_app_launch` dispatches through the companion RPC and
/// reports `backend=Companion`.
#[tokio::test]
async fn app_launch_prefers_companion_when_reachable() {
    let server = FakeCompanion::start(r#"{"ok":true}"#).await;
    let runner = Arc::new(FakeCommandRunner::new());
    let selection = resolve_phone_selection(&PhoneConfig::default());
    let mut manager = PhoneManager::with_runner(runner, selection);
    let now = PhoneManager::now_ms_for_tests();
    manager.insert_companion_session_for_tests("sess-1", SERIAL, server.port, COMPANION_TOKEN, now);

    match manager
        .handle(PhoneRequest::AppLaunch(
            sky_cua_platform::model::PhoneAppLaunchRequest {
                session: selector("sess-1"),
                package_name: "com.example.app".to_string(),
            },
        ))
        .await
    {
        PhoneResponse::App(response) => {
            assert!(response.success, "{:?}", response.diagnostics);
            assert_eq!(response.backend, PhoneBackendKind::Companion);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// When the companion RPC is not reachable (server refuses the connection), the
/// launch falls back to the ADB monkey path and reports `backend=Adb`.
#[tokio::test]
async fn app_launch_falls_back_to_adb_when_companion_unreachable() {
    let runner = Arc::new(FakeCommandRunner::new());
    // ADB monkey launch is scripted to succeed.
    runner.set_stdout(
        "adb",
        &[
            "-s",
            SERIAL,
            "shell",
            "monkey -p 'com.example.app' -c android.intent.category.LAUNCHER 1",
        ],
        "Events injected: 1",
    );
    let selection = resolve_phone_selection(&PhoneConfig::default());
    let mut manager = PhoneManager::with_runner(runner, selection);
    let now = PhoneManager::now_ms_for_tests();
    // Port 1 is unbindable/closed: the companion RPC connect is refused, forcing
    // the ADB fallback.
    manager.insert_companion_session_for_tests("sess-1", SERIAL, 1, COMPANION_TOKEN, now);

    match manager
        .handle(PhoneRequest::AppLaunch(
            sky_cua_platform::model::PhoneAppLaunchRequest {
                session: selector("sess-1"),
                package_name: "com.example.app".to_string(),
            },
        ))
        .await
    {
        PhoneResponse::App(response) => {
            assert!(response.success, "{:?}", response.diagnostics);
            assert_eq!(response.backend, PhoneBackendKind::Adb);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// A fallback-class companion failure on `phone_notifications` must invalidate
/// the cached companion capability, not just drop the live runtime: the cached
/// profile stops advertising a reachable companion, so subsequent coordinate
/// routing fails closed while screenshot routing can still fall back to ADB.
#[tokio::test]
async fn notifications_fallback_invalidates_cached_companion_capability() {
    let runner = Arc::new(FakeCommandRunner::new());
    let selection = resolve_phone_selection(&PhoneConfig::default());
    let mut manager = PhoneManager::with_runner(runner, selection);
    let now = PhoneManager::now_ms_for_tests();
    // Port 1 is closed: the companion RPC connect is refused (a fallback-class
    // transport failure).
    manager.insert_companion_session_for_tests("sess-1", SERIAL, 1, COMPANION_TOKEN, now);

    // Before the failure the cached profile advertises a reachable companion.
    let before = manager
        .cached_profile_for_tests("sess-1", now)
        .expect("cached profile");
    assert!(before.companion.rpc_reachable);

    match manager
        .handle(PhoneRequest::Notifications(
            sky_cua_platform::model::PhoneNotificationsRequest {
                session: selector("sess-1"),
                limit: Some(5),
            },
        ))
        .await
    {
        PhoneResponse::Notifications(response) => {
            assert_eq!(response.backend, PhoneBackendKind::None);
            assert!(!response.diagnostics.is_empty());
        }
        other => panic!("unexpected: {other:?}"),
    }

    // The cached companion capability is now invalidated: coordinate actions no
    // longer have a fallback, while screenshots can still degrade to ADB.
    let after = manager
        .cached_profile_for_tests("sess-1", now)
        .expect("cached profile");
    assert!(
        !after.companion.rpc_reachable,
        "the cached profile must stop advertising a reachable companion"
    );
    // The stale bool and refresh_state move in lockstep on invalidation: a Stale
    // refresh_state always implies stale=true.
    assert!(
        after.stale,
        "invalidation must set the stale bool in lockstep with refresh_state"
    );
    assert_eq!(after.refresh_state, PhoneCapabilityRefreshState::Stale);
    assert!(
        manager.coordinate_backend(&after).is_err(),
        "coordinate routing must require the companion after invalidation"
    );
    assert_eq!(
        manager.screenshot_backend(&after),
        PhoneBackendKind::Adb,
        "screenshot routing must fall back to ADB after invalidation"
    );
}

/// `phone_app_force_stop` must stay on ADB even when a companion is reachable: a
/// non-privileged companion cannot force-stop.
#[tokio::test]
async fn app_force_stop_stays_on_adb_with_companion() {
    let server = FakeCompanion::start(r#"{"ok":true}"#).await;
    let runner = Arc::new(FakeCommandRunner::new());
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "am force-stop 'com.example.app'"],
        "",
    );
    let selection = resolve_phone_selection(&PhoneConfig::default());
    let mut manager = PhoneManager::with_runner(runner, selection);
    let now = PhoneManager::now_ms_for_tests();
    manager.insert_companion_session_for_tests("sess-1", SERIAL, server.port, COMPANION_TOKEN, now);

    match manager
        .handle(PhoneRequest::AppForceStop(
            sky_cua_platform::model::PhoneAppForceStopRequest {
                session: selector("sess-1"),
                package_name: "com.example.app".to_string(),
            },
        ))
        .await
    {
        PhoneResponse::App(response) => {
            assert!(response.success, "{:?}", response.diagnostics);
            assert_eq!(response.backend, PhoneBackendKind::Adb);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn app_list_routes_through_adb_pm() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "pm", "list", "packages", "-3"],
        "package:com.example.one\npackage:com.example.two\n",
    );
    match manager
        .handle(PhoneRequest::AppList(
            sky_cua_platform::model::PhoneAppListRequest {
                session: selector(&session.session_id),
                include_system: false,
                limit: None,
            },
        ))
        .await
    {
        PhoneResponse::App(response) => {
            assert!(response.success);
            assert_eq!(response.backend, PhoneBackendKind::Adb);
            assert_eq!(response.apps.len(), 2);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// `phone_connect` with `start_scrcpy` against a manager whose scrcpy resolves to
/// a missing path must not spawn a mirror, must keep scrcpy inactive, and must
/// surface a structured `PhoneScrcpyLaunchFailed` diagnostic rather than aborting
/// the connect.
#[tokio::test]
async fn connect_with_start_scrcpy_missing_binary_degrades_with_diagnostic() {
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);

    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.companion_enabled = false;
    selection.companion_auto_install = false;
    // A configured path that does not exist forces resolve_scrcpy -> Missing, so
    // the launch fails before any spawn is attempted.
    selection.scrcpy_path = Some("/nonexistent/scrcpy-binary".to_string());
    let mut manager = PhoneManager::with_runner(runner, selection);

    let session = match manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some(SERIAL.to_string()),
            backend: None,
            install_companion: false,
            start_scrcpy: true,
        }))
        .await
    {
        PhoneResponse::Connected(session) => session,
        other => panic!("expected a connected session, got: {other:?}"),
    };

    // The session exists and is honest: scrcpy is not active and no managed
    // process/window is claimed.
    assert!(!session.capability_profile.scrcpy.active);
    assert!(!session.managed_process);
    assert!(session.window_title.is_none());
    assert!(!session.capabilities.scrcpy);
    assert!(!session.capabilities.host_visible_overlay);

    // The launch failure is surfaced as a structured diagnostic via the session's
    // connect diagnostics (companion-status surfacing path).
    match manager
        .handle(PhoneRequest::CompanionStatus(
            sky_cua_platform::model::PhoneCompanionStatusRequest {
                session: selector(&session.session_id),
            },
        ))
        .await
    {
        PhoneResponse::CompanionStatus(status) => assert!(
            status
                .diagnostics
                .iter()
                .any(|d| d.code == "PhoneScrcpyLaunchFailed"),
            "scrcpy launch failure must be surfaced: {:?}",
            status.diagnostics
        ),
        other => panic!("unexpected: {other:?}"),
    }
}

/// `phone_disconnect` on a session that owns a managed scrcpy mirror kills the
/// child, marks the process stopped, and reports a structured `PhoneScrcpyStopped`
/// diagnostic so the teardown is honest about stopping the managed mirror.
#[tokio::test]
async fn disconnect_stops_managed_scrcpy_mirror() {
    let (mut manager, _runner) = adb_only_manager();
    let session_id = manager.insert_scrcpy_session_for_tests(SERIAL);

    match manager
        .handle(PhoneRequest::Disconnect(
            sky_cua_platform::model::PhoneDisconnectRequest {
                session: selector(&session_id),
                keep_wireless: false,
            },
        ))
        .await
    {
        PhoneResponse::Disconnected(response) => {
            assert!(response.disconnected);
            assert_eq!(response.session_id, session_id);
            assert!(
                response
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneScrcpyStopped"),
                "disconnect must report stopping the managed mirror: {:?}",
                response.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }

    // The session and its runtime are gone.
    assert!(
        manager
            .cached_profile_for_tests(&session_id, super::now_ms())
            .is_none()
    );
}

/// An idempotent `phone_connect{start_scrcpy:true}` against an existing session
/// whose mirror was torn down (crash or operator close, surfaced by
/// `poll_scrcpy_liveness` clearing the runtime) must re-establish the mirror, not
/// silently report `scrcpy.active=false`. Here the daemon has a primed adoption
/// candidate, so the relaunch adopts the existing window deterministically.
#[tokio::test]
async fn reconnect_with_start_scrcpy_relaunches_dead_mirror_via_adoption() {
    let (mut manager, _runner) = adb_only_manager();
    // A managed mirror for SERIAL exists, then dies mid-session.
    let session_id = manager.insert_scrcpy_session_for_tests(SERIAL);
    manager.kill_scrcpy_child_for_tests(&session_id).await;

    // The liveness watchdog tears the dead runtime down: scrcpy is now inactive and
    // no runtime remains, the exact state a relaunching reconnect must repair.
    let crashed = manager.poll_scrcpy_liveness();
    assert_eq!(crashed, vec![(session_id.clone(), false)]);
    let downed = manager.session_view(&session_id).expect("session survives");
    assert!(
        !downed.capability_profile.scrcpy.active,
        "the crashed mirror must read inactive before the relaunch"
    );

    // The daemon primed an adoptable window for this serial (a previous run's
    // mirror, or one the operator left up), so the relaunch adopts it rather than
    // spawning real scrcpy.
    manager.set_scrcpy_adoption_candidate(Some(super::ScrcpyAdoptionCandidate {
        serial: SERIAL.to_string(),
        pid: Some(7777),
        window_title: format!("sky-cua-phone-{SERIAL}"),
    }));

    let reconnected = match manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some(SERIAL.to_string()),
            backend: None,
            install_companion: false,
            start_scrcpy: true,
        }))
        .await
    {
        PhoneResponse::Connected(session) => session,
        other => panic!("expected a reconnected session, got: {other:?}"),
    };

    // Same session (idempotent reconnect), but the mirror is re-established.
    assert_eq!(reconnected.session_id, session_id);
    assert!(
        reconnected.capability_profile.scrcpy.active,
        "a reconnect that asks for scrcpy must re-establish the torn-down mirror"
    );
    assert!(reconnected.capabilities.scrcpy);
    assert!(reconnected.managed_process);
    assert_eq!(
        reconnected.window_title.as_deref(),
        Some(format!("sky-cua-phone-{SERIAL}").as_str())
    );
    // The re-established mirror is adopted, so we own no child to poll/kill.
    assert_eq!(
        manager.scrcpy_ownership_for_tests(&session_id),
        Some(crate::phone::scrcpy::ScrcpyOwnership::Adopted)
    );
    assert_eq!(
        manager.scrcpy_has_owned_child_for_tests(&session_id),
        Some(false)
    );
}

/// When the relaunch on reconnect cannot re-establish the mirror (no adoptable
/// window and the scrcpy binary resolves to a missing path), the reconnect must
/// stay honest: scrcpy reports inactive and a structured `PhoneScrcpyLaunchFailed`
/// diagnostic is surfaced rather than a silent no-op.
#[tokio::test]
async fn reconnect_with_start_scrcpy_surfaces_diagnostic_when_relaunch_fails() {
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.companion_enabled = false;
    selection.companion_auto_install = false;
    selection.capability_cache_ttl_ms = 30_000;
    // A configured path that does not exist forces resolve_scrcpy -> Missing, so the
    // relaunch fails before any spawn is attempted.
    selection.scrcpy_path = Some("/nonexistent/scrcpy-binary".to_string());
    let mut manager = PhoneManager::with_runner(runner, selection);

    // A managed mirror for SERIAL exists, then dies and is torn down.
    let session_id = manager.insert_scrcpy_session_for_tests(SERIAL);
    manager.kill_scrcpy_child_for_tests(&session_id).await;
    assert_eq!(
        manager.poll_scrcpy_liveness(),
        vec![(session_id.clone(), false)]
    );

    // No adoption candidate is primed, so the relaunch falls through to a real
    // launch, which fails because the binary is missing.
    let reconnected = match manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some(SERIAL.to_string()),
            backend: None,
            install_companion: false,
            start_scrcpy: true,
        }))
        .await
    {
        PhoneResponse::Connected(session) => session,
        other => panic!("expected a reconnected session, got: {other:?}"),
    };

    // The session is honest: scrcpy did not come back up, and nothing fabricated a
    // managed mirror.
    assert_eq!(reconnected.session_id, session_id);
    assert!(!reconnected.capability_profile.scrcpy.active);
    assert!(!reconnected.capabilities.scrcpy);
    assert!(!reconnected.managed_process);

    // The relaunch failure is surfaced as a structured diagnostic via the session's
    // companion-status surfacing path, the same channel a fresh-connect launch
    // failure uses.
    match manager
        .handle(PhoneRequest::CompanionStatus(
            sky_cua_platform::model::PhoneCompanionStatusRequest {
                session: selector(&session_id),
            },
        ))
        .await
    {
        PhoneResponse::CompanionStatus(status) => assert!(
            status
                .diagnostics
                .iter()
                .any(|d| d.code == "PhoneScrcpyLaunchFailed"),
            "a failed mirror relaunch must surface a structured diagnostic: {:?}",
            status.diagnostics
        ),
        other => panic!("unexpected: {other:?}"),
    }
}

// ===========================================================================
// Host-window mapping + host-visible cursor overlay (Increment B)
// ===========================================================================

const SCRCPY_SERIAL: &str = "scrcpy-dev";

/// `scrcpy_window_to_map` offers a target only for an active, unmapped managed
/// mirror, and returns the pid/title/device-size/rotation the daemon needs.
#[tokio::test]
async fn scrcpy_window_to_map_targets_active_unmapped_session_only() {
    let (mut manager, _runner) = adb_only_manager();

    // No scrcpy session at all: nothing to map.
    assert!(manager.scrcpy_window_to_map().is_none());

    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        "portrait",
    );

    let target = manager
        .scrcpy_window_to_map()
        .expect("active unmapped mirror is a mapping target");
    assert_eq!(target.session_id, session_id);
    assert_eq!(
        target.window_title,
        format!("sky-cua-phone-{SCRCPY_SERIAL}")
    );
    assert_eq!(target.pid, Some(424_242));
    assert_eq!(
        target.device_size,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        }
    );
    assert_eq!(target.rotation_degrees, 0);

    // After mapping, the session is no longer offered.
    let host_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 1080.0,
        height: 2400.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(
        &target.session_id,
        &host_window,
        target.device_size.clone(),
        target.rotation_degrees,
    ));
    assert!(manager.scrcpy_window_to_map().is_none());
}

/// A device held at ROTATION_270 carries the exact quarter through the mapping
/// target, not the label-collapsed 90. Before the fix, `scrcpy_window_to_map`
/// re-derived rotation from the orientation label ("landscape" -> 90), which
/// could not distinguish 90 from 270 and silently collapsed both to 90. With the
/// live `display_rotation_degrees` quarter threaded through the profile, the
/// daemon receives the real 270 so the host content-rect math is correct.
#[tokio::test]
async fn scrcpy_window_to_map_preserves_live_270_quarter() {
    let (mut manager, _runner) = adb_only_manager();

    // Landscape label (shared by 90 and 270), but the live quarter is 270.
    let session_id = manager.insert_mappable_scrcpy_session_with_rotation_for_tests(
        SCRCPY_SERIAL,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        "landscape",
        Some(270),
    );

    let target = manager
        .scrcpy_window_to_map()
        .expect("active unmapped mirror is a mapping target");
    assert_eq!(target.session_id, session_id);
    // The exact quarter survives: not the label-derived 90.
    assert_eq!(target.rotation_degrees, 270);

    // The same 270 drives a real content-rect mapping (the 2400x1080 rotated
    // frame fits a landscape window), confirming the quarter is honored end to
    // end rather than rejected or collapsed.
    let host_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 2400.0,
        height: 1080.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(
        &target.session_id,
        &host_window,
        target.device_size.clone(),
        target.rotation_degrees,
    ));
    assert!(manager.scrcpy_window_to_map().is_none());
}

/// A managed mirror whose desktop window never registers is offered to the
/// daemon's mapping poll at most once: after the bounded retry round is marked
/// exhausted, `scrcpy_window_to_map` stops re-offering it, so the daemon does not
/// re-run its ~2s poll on every subsequent phone request. The mirror stays
/// honestly unmapped.
#[tokio::test]
async fn never_mapping_window_is_offered_at_most_once() {
    let (mut manager, _runner) = adb_only_manager();
    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        "portrait",
    );

    // First poll offers the target (the daemon would now run its bounded retry).
    let first = manager
        .scrcpy_window_to_map()
        .expect("active unmapped mirror is offered once");
    assert_eq!(first.session_id, session_id);

    // The daemon's retry round fails to find/map the window and marks it exhausted.
    manager.mark_scrcpy_mapping_exhausted(&session_id);

    // It is not re-offered on subsequent polls, and stays honestly unmapped.
    assert!(
        manager.scrcpy_window_to_map().is_none(),
        "an exhausted, never-mapping mirror must not be re-offered every request"
    );
    let view = manager.session_view(&session_id).expect("session view");
    assert!(view.capability_profile.scrcpy.active);
    assert!(!view.capability_profile.scrcpy.host_window_mapped);
}

/// `set_scrcpy_window_mapping` flips `host_window_mapped` true on the profile and
/// the session capabilities. The host-visible overlay plane additionally requires
/// a reachable companion overlay: the host no longer draws the phone cursor, so the
/// host plane is the device overlay made visible by a mapped scrcpy mirror.
#[tokio::test]
async fn set_scrcpy_window_mapping_marks_host_window_mapped() {
    let (mut manager, _runner) = adb_only_manager();
    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        "portrait",
    );

    // Before mapping: active mirror, but the host plane is off.
    let before = manager
        .session_view(&session_id)
        .expect("session before mapping");
    assert!(before.capability_profile.scrcpy.active);
    assert!(!before.capability_profile.scrcpy.host_window_mapped);
    assert!(!before.capabilities.host_visible_overlay);

    let host_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 1080.0,
        height: 2400.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(
        &session_id,
        &host_window,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        0,
    ));

    let after = manager
        .session_view(&session_id)
        .expect("session after mapping");
    assert!(after.capability_profile.scrcpy.active);
    assert!(after.capability_profile.scrcpy.host_window_mapped);
    // No companion in this session, so nothing draws into the mirror: the host
    // plane stays honestly off even though the scrcpy window is mapped.
    assert!(!after.capabilities.host_visible_overlay);

    // The host-visible plane is the companion overlay made visible by the mapped
    // mirror, so it turns on only once a reachable companion native overlay is
    // present. Drive both capability builders directly off the mapped profile.
    let mut profile = after.capability_profile.clone();
    assert!(!manager.backend_capabilities(&profile).host_visible_overlay);
    assert!(!manager.cursor_capabilities(&profile).host_visible_overlay);

    profile.companion.rpc_reachable = true;
    profile.companion.native_overlay = true;
    let backend = manager.backend_capabilities(&profile);
    assert!(backend.host_visible_overlay);
    assert!(backend.phone_native_overlay);
    let cursor = manager.cursor_capabilities(&profile);
    assert!(cursor.host_visible_overlay);
    assert!(cursor.phone_native_overlay);
}

/// A degenerate host window leaves the session honestly unmapped.
#[tokio::test]
async fn set_scrcpy_window_mapping_rejects_degenerate_window() {
    let (mut manager, _runner) = adb_only_manager();
    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        "portrait",
    );
    let zero_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 2400.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(!manager.set_scrcpy_window_mapping(
        &session_id,
        &zero_window,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        0,
    ));
    let session = manager.session_view(&session_id).expect("session");
    assert!(!session.capability_profile.scrcpy.host_window_mapped);
    assert!(manager.host_overlay_session().is_none());
}

/// `poll_scrcpy_liveness` detects a host-mapped managed mirror whose child exited
/// mid-session: it reports `(session_id, true)`, downgrades the cached scrcpy
/// capability to inactive/unmapped (so `host_overlay_enabled` is false), and tears
/// down the dead runtime so the host-overlay plane is gone.
#[tokio::test]
async fn poll_scrcpy_liveness_downgrades_host_mapped_crashed_mirror() {
    use crate::phone::scrcpy;

    let (mut manager, _runner) = adb_only_manager();
    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        "portrait",
    );

    // Map the window so the mirror owns a live host-overlay plane before the crash.
    let host_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 1080.0,
        height: 2400.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(
        &session_id,
        &host_window,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        0,
    ));
    let mapped = manager.session_view(&session_id).expect("mapped session");
    assert!(mapped.capability_profile.scrcpy.active);
    assert!(mapped.capability_profile.scrcpy.host_window_mapped);
    assert!(scrcpy::host_overlay_enabled(
        &mapped.capability_profile.scrcpy
    ));
    assert_eq!(
        manager.host_overlay_session().as_deref(),
        Some(session_id.as_str())
    );

    // The mirror dies mid-session: its stand-in child has now exited.
    manager.kill_scrcpy_child_for_tests(&session_id).await;

    let crashed = manager.poll_scrcpy_liveness();
    assert_eq!(
        crashed,
        vec![(session_id.clone(), true)],
        "a host-mapped crashed mirror must be reported as (session_id, was_host_mapped=true)"
    );

    // The session survives but its scrcpy capability is downgraded and the host
    // overlay plane is gone.
    let after = manager
        .session_view(&session_id)
        .expect("session survives a scrcpy crash");
    assert!(
        !after.capability_profile.scrcpy.active,
        "crashed mirror must be inactive"
    );
    assert!(
        !after.capability_profile.scrcpy.host_window_mapped,
        "crashed mirror must be unmapped"
    );
    assert!(
        !scrcpy::host_overlay_enabled(&after.capability_profile.scrcpy),
        "a crashed mirror must not keep the host overlay enabled"
    );
    assert!(
        after.capability_profile.scrcpy.reason.is_some(),
        "the downgrade must carry a structured reason"
    );
    assert!(
        manager.host_overlay_session().is_none(),
        "no host-mapped mirror remains after the crash"
    );

    // A second poll is a no-op: the dead runtime has been torn down.
    assert!(manager.poll_scrcpy_liveness().is_empty());
}

/// A still-running managed mirror is not flagged by `poll_scrcpy_liveness`: its
/// stand-in child is alive, so the capability stays active and no crash is
/// reported.
#[tokio::test]
async fn poll_scrcpy_liveness_leaves_live_mirror_untouched() {
    let (mut manager, _runner) = adb_only_manager();
    let session_id = manager.insert_scrcpy_session_for_tests(SCRCPY_SERIAL);

    assert!(
        manager.poll_scrcpy_liveness().is_empty(),
        "a live mirror must not be reported as crashed"
    );
    let view = manager.session_view(&session_id).expect("session view");
    assert!(view.capability_profile.scrcpy.active);
}

// ===========================================================================
// Host-window re-map on resize / host-scale change (Fix 1)
// ===========================================================================

/// Once a session is host-mapped, `scrcpy_window_to_map` stops offering it but
/// `scrcpy_window_to_remap` offers it for a drift re-check, carrying the same
/// pid/title/device-size/rotation the daemon needs to re-query the window.
#[tokio::test]
async fn scrcpy_window_to_remap_offers_only_mapped_session() {
    let (mut manager, _runner) = adb_only_manager();
    let device_size = sky_cua_platform::model::PixelSize {
        width: 1080,
        height: 2400,
    };
    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        device_size.clone(),
        "portrait",
    );

    // Before mapping: nothing to re-map (the initial map is still pending).
    assert!(manager.scrcpy_window_to_remap().is_none());

    let host_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 540.0,
        height: 1200.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(&session_id, &host_window, device_size.clone(), 0));

    // After mapping: the initial-map offer is gone, but the re-map offer is live.
    assert!(manager.scrcpy_window_to_map().is_none());
    let target = manager
        .scrcpy_window_to_remap()
        .expect("a mapped mirror is offered for the drift re-check");
    assert_eq!(target.session_id, session_id);
    assert_eq!(
        target.window_title,
        format!("sky-cua-phone-{SCRCPY_SERIAL}")
    );
    assert_eq!(target.device_size, device_size);
}

/// Re-mapping with an unchanged window rect is an idempotent no-op: the content
/// rect is identical, so `set_scrcpy_window_mapping` returns `true` without
/// churning the cached profile (its detected-at timestamp is preserved). A
/// changed rect recomputes the content rect, so a later host point reflects the
/// new geometry.
#[tokio::test]
async fn remap_is_idempotent_on_unchanged_rect_and_recomputes_on_drift() {
    let (mut manager, _runner) = adb_only_manager();
    let device_size = sky_cua_platform::model::PixelSize {
        width: 1080,
        height: 2400,
    };
    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        device_size.clone(),
        "portrait",
    );

    // Map at a 540x1200 window (exact 0.5 scale), origin (100, 50).
    let host_window = sky_cua_platform::model::RectF {
        x: 100.0,
        y: 50.0,
        width: 540.0,
        height: 1200.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(&session_id, &host_window, device_size.clone(), 0));
    let mapped_at = manager
        .cached_profile_for_tests(&session_id, super::now_ms())
        .expect("cached profile after mapping")
        .detected_at_ms;
    // device_to_host(200, 400) = (100 + 0.5*200, 50 + 0.5*400) = (200, 250).
    assert_eq!(
        manager.device_point_to_host_for_tests(&session_id, 200.0, 400.0),
        Some((200.0, 250.0))
    );

    // Re-map with the SAME rect: idempotent no-op. Still mapped, and the cached
    // profile is not re-inserted (its detected-at timestamp is unchanged).
    assert!(manager.set_scrcpy_window_mapping(&session_id, &host_window, device_size.clone(), 0));
    let after_noop = manager
        .cached_profile_for_tests(&session_id, super::now_ms())
        .expect("cached profile after no-op re-map")
        .detected_at_ms;
    assert_eq!(
        after_noop, mapped_at,
        "an unchanged re-map must not churn the cached profile"
    );

    // The operator resizes the window: 1080x2400 (exact 1.0 scale), origin (0, 0).
    let resized = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 1080.0,
        height: 2400.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(&session_id, &resized, device_size.clone(), 0));
    // device_to_host(200, 400) now = (0 + 1.0*200, 0 + 1.0*400) = (200, 400).
    assert_eq!(
        manager.device_point_to_host_for_tests(&session_id, 200.0, 400.0),
        Some((200.0, 400.0)),
        "a resized window must recompute the host mapping"
    );
    // The mirror stays host-mapped after the recompute.
    let view = manager.session_view(&session_id).expect("session view");
    assert!(view.capability_profile.scrcpy.host_window_mapped);
}

/// If the daemon re-checks a previously mapped scrcpy window and the host window
/// vanished or lost bounds, the manager must drop the host mapping immediately so
/// the overlay plane cannot keep drawing against a stale rectangle.
#[tokio::test]
async fn clear_scrcpy_window_mapping_drops_only_host_plane() {
    let (mut manager, _runner) = adb_only_manager();
    let device_size = sky_cua_platform::model::PixelSize {
        width: 1080,
        height: 2400,
    };
    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        device_size.clone(),
        "portrait",
    );
    let host_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 540.0,
        height: 1200.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(&session_id, &host_window, device_size, 0));
    assert!(manager.scrcpy_window_to_remap().is_some());

    assert!(manager.clear_scrcpy_window_mapping(&session_id));

    let view = manager.session_view(&session_id).expect("session view");
    assert!(view.capability_profile.scrcpy.active);
    assert!(!view.capability_profile.scrcpy.host_window_mapped);
    assert!(!view.capabilities.host_visible_overlay);
    assert!(manager.scrcpy_window_to_remap().is_none());
    assert!(manager.scrcpy_window_to_map().is_none());
    assert_eq!(
        manager.device_point_to_host_for_tests(&session_id, 200.0, 400.0),
        None
    );
}

// ===========================================================================
// Window adoption: reuse an existing mirror instead of spawning (Fix 2)
// ===========================================================================

/// `find_adoptable_scrcpy_window` matches a pre-existing window by the
/// deterministic `sky-cua-phone-<safe-serial>` title and reports its pid, and
/// returns `None` when no window carries the title.
#[tokio::test]
async fn find_adoptable_scrcpy_window_matches_by_title() {
    let (manager, _runner) = adb_only_manager();
    let title = format!("sky-cua-phone-{SCRCPY_SERIAL}");
    let windows = vec![
        window_info_for_tests("w-other", Some("unrelated"), Some(1)),
        window_info_for_tests("w-mirror", Some(&title), Some(4242)),
    ];

    let candidate = manager
        .find_adoptable_scrcpy_window(SCRCPY_SERIAL, &windows)
        .expect("a window with the deterministic title is adoptable");
    assert_eq!(candidate.serial, SCRCPY_SERIAL);
    assert_eq!(candidate.window_title, title);
    assert_eq!(candidate.pid, Some(4242));

    // No matching title: nothing to adopt.
    let none = manager.find_adoptable_scrcpy_window(
        SCRCPY_SERIAL,
        &[window_info_for_tests("w-other", Some("unrelated"), Some(1))],
    );
    assert!(none.is_none());
}

/// Adoption is skipped when a managed mirror for the serial is already tracked:
/// a window we own must never be shadowed by an adopted duplicate.
#[tokio::test]
async fn find_adoptable_scrcpy_window_skips_already_managed_serial() {
    let (mut manager, _runner) = adb_only_manager();
    // A managed (sky-cua-launched) mirror is already tracked for this serial.
    let _session_id = manager.insert_scrcpy_session_for_tests(SCRCPY_SERIAL);
    let title = format!("sky-cua-phone-{SCRCPY_SERIAL}");
    let windows = vec![window_info_for_tests("w-mirror", Some(&title), Some(4242))];

    assert!(
        manager
            .find_adoptable_scrcpy_window(SCRCPY_SERIAL, &windows)
            .is_none(),
        "must not adopt a window when a managed mirror for the serial is already owned"
    );
}

/// A connect that names an explicit serial adopts a primed candidate instead of
/// spawning: the resulting runtime is `Adopted`, owns no child (so the liveness
/// watchdog cannot try_wait it), and `poll_scrcpy_liveness` never downgrades it.
#[tokio::test]
async fn adopted_session_is_not_polled_as_crashed() {
    let (mut manager, _runner) = adb_only_manager();
    let device_size = sky_cua_platform::model::PixelSize {
        width: 1080,
        height: 2400,
    };
    let session_id = manager.insert_adopted_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        device_size.clone(),
        "portrait",
    );

    // The runtime is adopted and carries no child we own.
    assert_eq!(
        manager.scrcpy_ownership_for_tests(&session_id),
        Some(crate::phone::scrcpy::ScrcpyOwnership::Adopted)
    );
    assert_eq!(
        manager.scrcpy_has_owned_child_for_tests(&session_id),
        Some(false),
        "an adopted runtime must not carry a child the watchdog could try_wait"
    );

    // The liveness watchdog must never report an adopted window as crashed: we do
    // not own its process, so there is nothing to poll.
    assert!(
        manager.poll_scrcpy_liveness().is_empty(),
        "an adopted mirror must not be downgraded by the crash watchdog"
    );
    let view = manager.session_view(&session_id).expect("session view");
    assert!(
        view.capability_profile.scrcpy.active,
        "an adopted mirror stays active across a liveness poll"
    );

    // It maps into a host window like any mirror, enabling the host overlay plane.
    let host_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 540.0,
        height: 1200.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(&session_id, &host_window, device_size, 0));
    assert_eq!(
        manager.host_overlay_session().as_deref(),
        Some(session_id.as_str())
    );
    // Even mapped, it is still never polled as crashed.
    assert!(manager.poll_scrcpy_liveness().is_empty());
}

// ===========================================================================
// Reporting fidelity: connect failure, install strategy, companion identity
// ===========================================================================

const WIRELESS_SERIAL: &str = "192.168.1.50:5555";

/// A failed `adb connect` for a wireless `host:port` target surfaces the
/// actionable failure reason as a `PhoneConnectFailed` diagnostic on the response,
/// so a refused/timed-out connect is not lost behind the generic
/// `PhoneDeviceUnavailable` message. `adb connect` exits 0 even on failure, so the
/// outcome is classified on its text ("failed to connect to ...").
#[tokio::test]
async fn wireless_connect_failure_surfaces_connect_failed_diagnostic() {
    let runner = Arc::new(FakeCommandRunner::new());
    runner.set_stdout("adb", &["version"], "Android Debug Bridge version 1.0.41");
    // The device list does not include the wireless target, so it never resolves
    // to an authorized device after the connect attempt.
    runner.set_stdout("adb", &["devices"], "List of devices attached\n");
    // `adb connect` reports a classified failure (exit 0, "failed to connect" text).
    runner.set_stdout(
        "adb",
        &["connect", WIRELESS_SERIAL],
        "failed to connect to '192.168.1.50:5555': Connection refused",
    );

    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.companion_enabled = false;
    selection.companion_auto_install = false;
    let mut manager = PhoneManager::with_runner(runner, selection);

    match manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some(WIRELESS_SERIAL.to_string()),
            backend: None,
            install_companion: false,
            start_scrcpy: false,
        }))
        .await
    {
        PhoneResponse::Status(report) => {
            let connect_failed = report
                .diagnostics
                .iter()
                .find(|d| d.code == "PhoneConnectFailed")
                .expect("a failed adb connect must surface a PhoneConnectFailed diagnostic");
            assert!(
                connect_failed.message.contains("Connection refused"),
                "the connect failure reason must survive into the diagnostic: {:?}",
                connect_failed.message
            );
            // The generic unavailable diagnostic is still present, but the
            // actionable connect reason is no longer the only thing surfaced.
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneDeviceUnavailable")
            );
            assert!(report.sessions.is_empty());
        }
        other => panic!("expected a Status rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn explicit_install_companion_honors_force_reinstall_and_allow_downgrade() {
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 20, "2.0.0");
    let apk = selection.companion_apk_path.clone();
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "install", "-r", "-d", &apk],
        "Success",
    );
    selection.companion_enabled = true;
    selection.companion_auto_install = false;
    selection.companion_allow_downgrade = false;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);
    let session = connect(&mut manager).await;

    match manager
        .handle(PhoneRequest::InstallCompanion(
            sky_cua_platform::model::PhoneInstallCompanionRequest {
                session: selector(&session.session_id),
                force_reinstall: true,
                allow_downgrade: true,
            },
        ))
        .await
    {
        PhoneResponse::Action(action) => {
            assert!(
                action
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "CompanionUpdate"),
                "force_reinstall should surface an explicit update/install attempt: {:?}",
                action.diagnostics
            );
        }
        other => panic!("unexpected: {other:?}"),
    }

    assert!(
        runner
            .recorded_calls()
            .iter()
            .any(|call| call == &format!("adb -s {SERIAL} install -r -d {apk}")),
        "explicit allow_downgrade must add -d to the install argv; recorded: {:?}",
        runner.recorded_calls()
    );
}

/// Drive a successful single/multiple/multi-package install and assert the
/// response echoes the matching `install_strategy`, so the caller can tell the
/// install path that ran from the request it sent.
async fn assert_install_reports_strategy(
    mode: sky_cua_platform::model::PhoneAppInstallMode,
    install_args: &[&str],
    expected: sky_cua_platform::model::PhoneInstallStrategy,
) {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_stdout("adb", install_args, "Success");

    match manager
        .handle(PhoneRequest::AppInstall(
            sky_cua_platform::model::PhoneAppInstallRequest {
                session: selector(&session.session_id),
                apk_paths: vec!["/tmp/a.apk".to_string(), "/tmp/b.apk".to_string()],
                mode,
                reinstall: false,
                allow_downgrade: false,
                allow_test_apk: false,
                grant_runtime_permissions: false,
            },
        ))
        .await
    {
        PhoneResponse::App(response) => {
            assert!(response.success, "{:?}", response.diagnostics);
            assert_eq!(
                response.install_strategy,
                Some(expected),
                "a successful install must echo its strategy"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn single_install_reports_single_strategy() {
    assert_install_reports_strategy(
        sky_cua_platform::model::PhoneAppInstallMode::Single,
        &["-s", SERIAL, "install", "/tmp/a.apk"],
        sky_cua_platform::model::PhoneInstallStrategy::Single,
    )
    .await;
}

#[tokio::test]
async fn multiple_install_reports_multiple_strategy() {
    assert_install_reports_strategy(
        sky_cua_platform::model::PhoneAppInstallMode::Multiple,
        &["-s", SERIAL, "install-multiple", "/tmp/a.apk", "/tmp/b.apk"],
        sky_cua_platform::model::PhoneInstallStrategy::Multiple,
    )
    .await;
}

#[tokio::test]
async fn multi_package_install_reports_multi_package_strategy() {
    assert_install_reports_strategy(
        sky_cua_platform::model::PhoneAppInstallMode::MultiPackage,
        &[
            "-s",
            SERIAL,
            "install-multi-package",
            "/tmp/a.apk",
            "/tmp/b.apk",
        ],
        sky_cua_platform::model::PhoneInstallStrategy::MultiPackage,
    )
    .await;
}

/// A failed install reports no strategy: `install_strategy` is set on the success
/// arm only, so a failure leaves it `None`.
#[tokio::test]
async fn failed_install_reports_no_strategy() {
    let (mut manager, runner) = adb_only_manager();
    let session = connect(&mut manager).await;
    runner.set_output(
        "adb",
        &["-s", SERIAL, "install", "/tmp/a.apk"],
        crate::phone::command::CommandOutput {
            status: Some(1),
            stdout: Vec::new(),
            stderr: b"Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]".to_vec(),
        },
    );

    match manager
        .handle(PhoneRequest::AppInstall(
            sky_cua_platform::model::PhoneAppInstallRequest {
                session: selector(&session.session_id),
                apk_paths: vec!["/tmp/a.apk".to_string()],
                mode: sky_cua_platform::model::PhoneAppInstallMode::Single,
                reinstall: false,
                allow_downgrade: false,
                allow_test_apk: false,
                grant_runtime_permissions: false,
            },
        ))
        .await
    {
        PhoneResponse::App(response) => {
            assert!(!response.success);
            assert!(
                response.install_strategy.is_none(),
                "a failed install must not report a strategy"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// A minimal in-process companion that serves a scripted `capabilities` result
/// for the `capabilities` method (and a bare ok for anything else), echoing the
/// request id. Like [`MethodAwareCompanion`] it does NOT validate the bearer
/// token, so it stands in for a reachable companion under the real bootstrap's
/// randomly-minted session token (which the host never exposes to the test).
struct CapabilitiesCompanion {
    port: u16,
    _shutdown: oneshot::Sender<()>,
}

impl CapabilitiesCompanion {
    async fn start(capabilities_result: &'static str) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let (tx, mut rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        tokio::spawn(async move {
                            let mut buf = Vec::new();
                            let mut chunk = [0u8; 4096];
                            let header_end = loop {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => n,
                                };
                                buf.extend_from_slice(&chunk[..n]);
                                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    break p;
                                }
                            };
                            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                            let content_length = headers
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            let mut body = buf[header_end + 4..].to_vec();
                            while body.len() < content_length {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => n,
                                };
                                body.extend_from_slice(&chunk[..n]);
                            }
                            let text = String::from_utf8_lossy(&body);
                            let id = text
                                .split("\"id\":")
                                .nth(1)
                                .and_then(|s| s.split([',', '}']).next())
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .unwrap_or(1);
                            let resp_body = format!(
                                r#"{{"protocol_version":1,"ok":true,"id":{id},"result":{capabilities_result}}}"#
                            );
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                resp_body.len(),
                                resp_body
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.flush().await;
                        });
                    }
                }
            }
        });
        Self {
            port,
            _shutdown: tx,
        }
    }
}

/// A reachable companion connect (the full `capabilities` probe path) populates
/// `installed_cert_sha256` from the cert the host parsed during
/// `read_installed_companion`, and surfaces the expected packaged-APK SHA-256 on
/// `apk_sha256`. The cert digest matches the configured expected cert so the
/// up-to-date decision proceeds to the RPC probe.
#[tokio::test]
async fn reachable_companion_reports_installed_cert_and_apk_sha256() {
    // The signing cert the fake device reports via `dumpsys package`. It matches
    // the configured expected cert (case/colon-insensitive) so decide_install
    // stays UpToDate and the RPC probe runs.
    const CERT: &str = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
    const APK_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let caps = r#"{"version":"2.0.0","version_code":20,"package":"com.skycua.phonecompanion","accessibility_enabled":true,"can_perform_gestures":true,"can_retrieve_window_content":true,"can_take_screenshot":true,"notification_listener_enabled":true,"native_overlay":true,"native_overlay_pass_through":true,"screenshot_api_level":34,"screenshot_supported":true,"gesture_supported":true}"#;
    let server = CapabilitiesCompanion::start(caps).await;

    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    // Companion installed; `dumpsys package` reports the matching cert digest.
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "pm", "path", &package],
        "package:/data/app/companion.apk",
    );
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "dumpsys", "package", &package],
        &format!(
            "    versionCode=20 minSdk=26\n    versionName=2.0.0\n    SHA-256 cert digest: {CERT}\n"
        ),
    );
    let port = server.port;
    let port_arg = format!("tcp:{port}");
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    selection.companion_rpc_port = port;
    selection.companion_expected_cert_sha256 = Some(CERT.to_string());
    selection.companion_apk_sha256 = Some(APK_SHA.to_string());
    let mut manager = PhoneManager::with_runner(runner, selection);

    let session = connect(&mut manager).await;
    let companion = &session.capability_profile.companion;
    assert!(
        companion.rpc_reachable,
        "the companion must come up reachable: {companion:?}"
    );
    assert_eq!(
        companion.installed_cert_sha256.as_deref(),
        Some(CERT),
        "the reachable report must carry the installed cert the host parsed"
    );
    assert_eq!(
        companion.apk_sha256.as_deref(),
        Some(APK_SHA),
        "the reachable report must surface the expected packaged-APK SHA-256"
    );
}

/// An unreachable companion connect (the RPC endpoint never comes up) still
/// surfaces the expected packaged-APK SHA-256 on `apk_sha256`, mirroring the
/// reachable path. The apk hash is report-only expected metadata, not a live gate.
#[tokio::test]
async fn unreachable_companion_reports_expected_apk_sha256() {
    const APK_SHA: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    let (mut manager, runner) = companion_manager();
    let apk = "resources/android/phone-companion.apk";
    runner.set_stdout("adb", &["-s", SERIAL, "install", "-r", apk], "Success");
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "forward", "tcp:47683", "tcp:47683"],
        "",
    );
    // Inject the expected packaged-APK hash into the selection before connect.
    manager.set_companion_apk_sha256_for_tests(APK_SHA);

    let session = connect(&mut manager).await;
    let companion = &session.capability_profile.companion;
    // No RPC server: the companion is unreachable, but the expected apk hash is
    // still reported (report-only metadata, surfaced like the reachable path).
    assert!(!companion.rpc_reachable);
    assert_eq!(
        companion.apk_sha256.as_deref(),
        Some(APK_SHA),
        "the unreachable report must still surface the expected packaged-APK SHA-256"
    );
}

/// Proof-gate (Phase 5): the phone capture/observe path does NOT hide the
/// host-visible desktop overlay, and it does not need to. The phone screenshot is
/// captured ON THE DEVICE (`adb exec-out screencap` here, or the companion's
/// on-device screenshot), so the host-desktop scrcpy-window overlay physically
/// cannot appear in the returned device PNG. This documents the intentional no-op:
/// a managed scrcpy session captures a correct device-resolution image with no
/// host-overlay-hide step in the path.
#[tokio::test]
async fn device_capture_needs_no_host_overlay_hide() {
    let (mut manager, runner) = adb_only_manager();
    // A managed scrcpy session owns a host-visible mirror window/overlay plane.
    let session_id = manager.insert_mappable_scrcpy_session_for_tests(
        SCRCPY_SERIAL,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        "portrait",
    );
    let host_window = sky_cua_platform::model::RectF {
        x: 0.0,
        y: 0.0,
        width: 1080.0,
        height: 2400.0,
        space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
    };
    assert!(manager.set_scrcpy_window_mapping(
        &session_id,
        &host_window,
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        0,
    ));
    // The host overlay plane is live for this session.
    assert_eq!(
        manager.host_overlay_session().as_deref(),
        Some(session_id.as_str())
    );

    // The screenshot is captured ON THE DEVICE via `adb exec-out screencap`. There
    // is no host-overlay-hide call in the capture path; the device PNG cannot
    // contain the host scrcpy-window overlay because that overlay lives on the
    // host desktop, not the device framebuffer.
    runner.set_output(
        "adb",
        &["-s", SCRCPY_SERIAL, "exec-out", "screencap", "-p"],
        crate::phone::command::CommandOutput {
            status: Some(0),
            stdout: png_bytes(1080, 2400),
            stderr: Vec::new(),
        },
    );

    match manager
        .handle(PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: selector(&session_id),
            // No companion is proven, so the screenshot routes to the ADB
            // device-capture path (scrcpy frames are not pulled as stills in v1).
            backend: Some(PhoneBackendKind::Adb),
            include_image_data: true,
        }))
        .await
    {
        PhoneResponse::Screenshot(shot) => {
            assert_eq!(shot.backend, PhoneBackendKind::Adb);
            assert!(shot.inline_image.is_some(), "a device PNG was returned");
            assert_eq!(shot.device_size.width, 1080);
            assert_eq!(shot.device_size.height, 2400);
            assert!(shot.diagnostics.is_empty(), "{:?}", shot.diagnostics);
        }
        other => panic!("unexpected: {other:?}"),
    }

    // The host overlay plane is untouched by the device capture: nothing in the
    // capture path hides it, and nothing needs to.
    assert_eq!(
        manager.host_overlay_session().as_deref(),
        Some(session_id.as_str()),
        "device capture must not touch the host overlay plane"
    );
}

/// A companion RPC server that mirrors the real companion's gesture contract:
/// a stroke with a non-positive `duration_ms` is rejected with the structured
/// `bad_request` error (`duration_ms must be positive`), exactly as Android's
/// `dispatchGesture` does; a positive duration is dispatched. This lets a unit
/// test catch a regression to a zero-duration tap, which a permissive fake that
/// always answers `dispatched:true` would silently pass.
struct GestureDurationValidatingCompanion {
    port: u16,
    _shutdown: oneshot::Sender<()>,
}

impl GestureDurationValidatingCompanion {
    async fn start() -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let (tx, mut rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        tokio::spawn(async move {
                            let mut buf = Vec::new();
                            let mut chunk = [0u8; 4096];
                            let header_end = loop {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => n,
                                };
                                buf.extend_from_slice(&chunk[..n]);
                                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    break p;
                                }
                            };
                            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                            let content_length = headers
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            let mut body = buf[header_end + 4..].to_vec();
                            while body.len() < content_length {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => n,
                                };
                                body.extend_from_slice(&chunk[..n]);
                            }
                            let text = String::from_utf8_lossy(&body);
                            let id = text
                                .split("\"id\":")
                                .nth(1)
                                .and_then(|s| s.split([',', '}']).next())
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .unwrap_or(1);
                            let duration = text
                                .split("\"duration_ms\":")
                                .nth(1)
                                .and_then(|s| s.split([',', '}']).next())
                                .and_then(|s| s.trim().parse::<i64>().ok())
                                .unwrap_or(0);
                            let resp_body = if duration > 0 {
                                format!(
                                    r#"{{"protocol_version":1,"ok":true,"id":{id},"result":{{"dispatched":true}}}}"#
                                )
                            } else {
                                format!(
                                    r#"{{"protocol_version":1,"ok":false,"id":{id},"error":{{"code":"bad_request","message":"duration_ms must be positive"}}}}"#
                                )
                            };
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                resp_body.len(),
                                resp_body
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.flush().await;
                        });
                    }
                }
            }
        });
        Self {
            port,
            _shutdown: tx,
        }
    }
}

/// Regression: a companion tap must dispatch a gesture with a POSITIVE stroke
/// duration. Android `dispatchGesture` rejects a zero-duration stroke
/// (`bad_request: duration_ms must be positive`), which previously broke every
/// tap on a companion-enabled device. The validating fake rejects a non-positive
/// duration the same way, so this fails if the tap ever regresses to duration 0.
#[tokio::test]
async fn companion_tap_dispatches_positive_gesture_duration() {
    let server = GestureDurationValidatingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    let selection = resolve_phone_selection(&PhoneConfig::default());
    let mut manager = PhoneManager::with_runner(runner, selection);
    let now = PhoneManager::now_ms_for_tests();
    manager.insert_companion_session_for_tests("sess-1", SERIAL, server.port, COMPANION_TOKEN, now);
    // Prove the companion gesture capability so the tap routes to the companion
    // rather than the ADB baseline.
    manager
        .profiles
        .get_mut("sess-1")
        .expect("cached profile")
        .profile
        .companion
        .gesture_dispatch = true;

    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector("sess-1"),
            phone_snapshot_id: None,
            x: 100.0,
            y: 200.0,
            use_device_coordinates: true,
        }))
        .await
    {
        PhoneResponse::Action(action) => {
            assert_eq!(
                action.backend,
                PhoneBackendKind::Companion,
                "tap must route to and succeed on the companion: {:?}",
                action.diagnostics
            );
            assert!(action.diagnostics.is_empty(), "{:?}", action.diagnostics);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

// ===========================================================================
// Phone-side agent overlay wiring (overlay_active / overlay_gesture)
// ===========================================================================

/// A method-aware fake companion that RECORDS every request body it serves, so a
/// test can assert which overlay RPCs the manager issued (and with what params).
/// It is token-agnostic (like the other manager fakes) and replies with a
/// method-appropriate ok result: a full `capabilities` body so the bootstrap
/// proves a reachable companion, and the matching ok result for the overlay/
/// gesture methods. The recorded request bodies are exposed through a shared
/// `Arc<Mutex<Vec<String>>>`.
struct RecordingCompanion {
    port: u16,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
    _shutdown: oneshot::Sender<()>,
}

impl RecordingCompanion {
    async fn start() -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind recording companion");
        let port = listener.local_addr().expect("addr").port();
        let requests: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, mut rx) = oneshot::channel::<()>();
        let sink = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let sink = Arc::clone(&sink);
                        tokio::spawn(async move {
                            let mut buf = Vec::new();
                            let mut chunk = [0u8; 4096];
                            let header_end = loop {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => n,
                                };
                                buf.extend_from_slice(&chunk[..n]);
                                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    break p;
                                }
                            };
                            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                            let content_length = headers
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            let mut body = buf[header_end + 4..].to_vec();
                            while body.len() < content_length {
                                let n = match stream.read(&mut chunk).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => n,
                                };
                                body.extend_from_slice(&chunk[..n]);
                            }
                            let text = String::from_utf8_lossy(&body).to_string();
                            sink.lock().expect("requests lock").push(text.clone());

                            let id = text
                                .split("\"id\":")
                                .nth(1)
                                .and_then(|s| s.split([',', '}']).next())
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .unwrap_or(1);
                            let result = if text.contains("\"method\":\"capabilities\"") {
                                r#"{"version":"2.0.0","version_code":20,"package":"com.skycua.phonecompanion","accessibility_enabled":true,"can_perform_gestures":true,"can_retrieve_window_content":true,"can_take_screenshot":true,"notification_listener_enabled":true,"native_overlay":true,"native_overlay_pass_through":true,"screenshot_api_level":34,"screenshot_supported":true,"gesture_supported":true}"#.to_string()
                            } else if text.contains("\"method\":\"overlay_active\"") {
                                r#"{"active":true,"glow_supported":true}"#.to_string()
                            } else if text.contains("\"method\":\"overlay_gesture\"") {
                                r#"{"animated":true}"#.to_string()
                            } else if text.contains("\"method\":\"gesture\"") {
                                r#"{"dispatched":true}"#.to_string()
                            } else {
                                r#"{"ok":true}"#.to_string()
                            };
                            let resp_body = format!(
                                r#"{{"protocol_version":1,"ok":true,"id":{id},"result":{result}}}"#
                            );
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                resp_body.len(),
                                resp_body
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.flush().await;
                        });
                    }
                }
            }
        });
        Self {
            port,
            requests,
            _shutdown: tx,
        }
    }

    /// The recorded request bodies, in order.
    fn recorded(&self) -> Vec<String> {
        self.requests.lock().expect("requests lock").clone()
    }

    /// The first recorded request body whose `method` is `method`, if any.
    fn request_for(&self, method: &str) -> Option<String> {
        let needle = format!("\"method\":\"{method}\"");
        self.recorded()
            .into_iter()
            .find(|body| body.contains(&needle))
    }
}

/// A successful phone tap on a companion-reachable session animates the phone-side
/// agent overlay: the real tap dispatches through the companion gesture lane and
/// `finish_action` issues an `overlay_gesture` with kind `tap` and the device
/// point.
#[tokio::test]
async fn tap_animates_overlay_gesture_when_companion_reachable() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    let selection = resolve_phone_selection(&PhoneConfig::default());
    let mut manager = PhoneManager::with_runner(runner, selection);
    let now = PhoneManager::now_ms_for_tests();
    manager.insert_companion_session_for_tests("sess-1", SERIAL, server.port, COMPANION_TOKEN, now);

    let action = match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector("sess-1"),
            phone_snapshot_id: None,
            x: 100.0,
            y: 200.0,
            use_device_coordinates: true,
        }))
        .await
    {
        PhoneResponse::Action(action) => action,
        other => panic!("unexpected: {other:?}"),
    };
    assert_eq!(
        action.backend,
        PhoneBackendKind::Companion,
        "{:?}",
        action.diagnostics
    );

    let body = server
        .request_for("overlay_gesture")
        .expect("a successful tap must animate the overlay");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(parsed["params"]["kind"], "tap");
    let points = parsed["params"]["points"].as_array().expect("points");
    assert_eq!(points.len(), 1);
    assert_eq!(points[0]["x"], serde_json::json!(100.0));
    assert_eq!(points[0]["y"], serde_json::json!(200.0));
}

/// A successful swipe animates the overlay with kind `swipe` and the start/end
/// point path, carrying the request's gesture duration as the animation hint.
#[tokio::test]
async fn swipe_animates_overlay_gesture_with_two_points() {
    use sky_cua_platform::model::PhoneSwipeRequest;

    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    let selection = resolve_phone_selection(&PhoneConfig::default());
    let mut manager = PhoneManager::with_runner(runner, selection);
    let now = PhoneManager::now_ms_for_tests();
    manager.insert_companion_session_for_tests("sess-1", SERIAL, server.port, COMPANION_TOKEN, now);

    let action = match manager
        .handle(PhoneRequest::Swipe(PhoneSwipeRequest {
            session: selector("sess-1"),
            phone_snapshot_id: None,
            start_x: 10.0,
            start_y: 20.0,
            end_x: 30.0,
            end_y: 40.0,
            duration_ms: Some(400),
            use_device_coordinates: true,
        }))
        .await
    {
        PhoneResponse::Action(action) => action,
        other => panic!("unexpected: {other:?}"),
    };
    assert_eq!(
        action.backend,
        PhoneBackendKind::Companion,
        "{:?}",
        action.diagnostics
    );

    let body = server
        .request_for("overlay_gesture")
        .expect("a successful swipe must animate the overlay");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(parsed["params"]["kind"], "swipe");
    assert_eq!(parsed["params"]["duration_ms"], serde_json::json!(400));
    let points = parsed["params"]["points"].as_array().expect("points");
    assert_eq!(points.len(), 2);
    assert_eq!(points[0]["x"], serde_json::json!(10.0));
    assert_eq!(points[1]["y"], serde_json::json!(40.0));
}

/// A connect that brings up a reachable companion lights the persistent edge glow
/// (`overlay_active(true)`), and the matching disconnect turns it off
/// (`overlay_active(false)`).
#[tokio::test]
async fn connect_and_disconnect_toggle_overlay_active() {
    use sky_cua_platform::model::PhoneDisconnectRequest;

    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    // Companion already installed with a verified matching cert -> UpToDate; the
    // forward and the RPC port match the recording server so the bootstrap probe
    // reaches it.
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner, selection);

    let session = connect(&mut manager).await;
    assert!(
        session.capability_profile.companion.rpc_reachable,
        "the recording companion must come up reachable: {:?}",
        session.capability_profile.companion
    );
    let active = server
        .request_for("overlay_active")
        .expect("connect must light the edge glow");
    let parsed: serde_json::Value = serde_json::from_str(&active).expect("json body");
    assert_eq!(parsed["params"]["active"], serde_json::json!(true));

    // Disconnect must turn the glow off before the session is torn down.
    let _ = manager
        .handle(PhoneRequest::Disconnect(PhoneDisconnectRequest {
            session: selector(&session.session_id),
            keep_wireless: true,
        }))
        .await;
    let off = server
        .recorded()
        .into_iter()
        .filter(|body| body.contains("\"method\":\"overlay_active\""))
        .find(|body| body.contains("\"active\":false"))
        .expect("disconnect must turn the edge glow off");
    let parsed: serde_json::Value = serde_json::from_str(&off).expect("json body");
    assert_eq!(parsed["params"]["active"], serde_json::json!(false));
}

#[tokio::test]
async fn idle_watchdog_turns_overlay_active_off_without_disconnect() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner, selection);

    let session = connect(&mut manager).await;
    assert!(
        server.request_for("overlay_active").is_some(),
        "connect must light the edge glow"
    );

    let expired = manager
        .expire_idle_companion_overlays(session.created_at_ms + 20_000)
        .await;
    assert_eq!(expired, vec![session.session_id.clone()]);
    assert!(
        manager.sessions.contains_key(&session.session_id),
        "idle overlay expiry must not remove the phone session"
    );
    let off = server
        .recorded()
        .into_iter()
        .filter(|body| body.contains("\"method\":\"overlay_active\""))
        .find(|body| body.contains("\"active\":false"))
        .expect("idle expiry must turn the edge glow off");
    let parsed: serde_json::Value = serde_json::from_str(&off).expect("json body");
    assert_eq!(parsed["params"]["active"], serde_json::json!(false));
}

#[tokio::test]
async fn session_activity_relights_overlay_after_idle_expiry() {
    use sky_cua_platform::model::PhoneCompanionStatusRequest;

    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 2, "0.1.1");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner, selection);

    let session = connect(&mut manager).await;
    let _ = manager
        .expire_idle_companion_overlays(session.created_at_ms + 20_000)
        .await;

    let _ = manager
        .handle(PhoneRequest::CompanionStatus(PhoneCompanionStatusRequest {
            session: selector(&session.session_id),
        }))
        .await;

    let relit = server
        .recorded()
        .into_iter()
        .filter(|body| body.contains("\"method\":\"overlay_active\""))
        .filter(|body| body.contains("\"active\":true"))
        .count();
    assert_eq!(
        relit, 2,
        "connect lights once and post-idle session activity must relight once"
    );
}

// ===========================================================================
// Config wiring: primary_target_models / wireless_auto_connect /
// companion_operator_mode / visible_overlay
// ===========================================================================

/// Script a two-device `adb devices -l` listing: a generic USB device first, then
/// a device whose `model` matches a configured primary target. Shared by the
/// primary-target ordering tests.
fn script_two_device_listing(runner: &FakeCommandRunner) {
    runner.set_stdout("adb", &["version"], "Android Debug Bridge version 1.0.41");
    runner.set_stdout(
        "adb",
        &["devices", "-l"],
        "List of devices attached\n\
         emulator-5554          device product:sdk_gphone model:Pixel transport_id:1\n\
         R5CT30ABCDE            device product:dm3q model:SM_S938B transport_id:2\n",
    );
}

/// `phone_list_devices` marks a device whose model matches `[phone]
/// primary_target_models` as primary and sorts it ahead of the non-primaries,
/// preserving adb's order within each group.
#[tokio::test]
async fn list_devices_front_loads_configured_primary_models() {
    let runner = Arc::new(FakeCommandRunner::new());
    script_two_device_listing(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    // Case-insensitive match against the second device's `model` (`SM_S938B`).
    selection.primary_target_models = vec!["sm_s938b".to_string()];
    let manager = PhoneManager::with_runner(runner, selection);

    let devices = manager.list_devices_for_tests().await.devices;
    assert_eq!(devices.len(), 2);
    // The configured primary is front-loaded and marked.
    assert_eq!(devices[0].serial, "R5CT30ABCDE");
    assert!(devices[0].primary, "configured primary must be marked");
    // The non-primary follows and is unmarked.
    assert_eq!(devices[1].serial, "emulator-5554");
    assert!(!devices[1].primary);
}

/// With no configured primary targets the listing is byte-identical to adb's
/// order and no device is marked primary (default behavior is unchanged).
#[tokio::test]
async fn list_devices_unchanged_without_configured_primaries() {
    let runner = Arc::new(FakeCommandRunner::new());
    script_two_device_listing(&runner);
    // Default selection has an empty primary_target_models.
    let selection = resolve_phone_selection(&PhoneConfig::default());
    assert!(selection.primary_target_models.is_empty());
    let manager = PhoneManager::with_runner(runner, selection);

    let devices = manager.list_devices_for_tests().await.devices;
    assert_eq!(devices.len(), 2);
    // adb order preserved; nothing marked primary.
    assert_eq!(devices[0].serial, "emulator-5554");
    assert_eq!(devices[1].serial, "R5CT30ABCDE");
    assert!(devices.iter().all(|device| !device.primary));
}

/// With `wireless_auto_connect` enabled, `phone_connect` runs `adb connect` for
/// the configured wireless `host:port` default BEFORE serial resolution, even when
/// the explicit request targets a different (USB) device. Isolating on a distinct
/// explicit target proves the pre-connect comes from the auto-connect path, not
/// from the existing wireless-targeting `adb connect`.
#[tokio::test]
async fn connect_auto_connects_configured_wireless_default() {
    const WIRELESS: &str = "192.168.1.42:39000";
    let runner = Arc::new(FakeCommandRunner::new());
    // The explicit USB target is a real, authorized device; the wireless default
    // is a separate pre-configured link that only the auto-connect path touches.
    script_device_probes(&runner);
    runner.set_stdout(
        "adb",
        &["connect", WIRELESS],
        "connected to 192.168.1.42:39000",
    );

    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.companion_enabled = false;
    selection.companion_auto_install = false;
    selection.wireless_auto_connect = true;
    selection.default_serial = Some(WIRELESS.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);

    // Explicit USB target distinct from the wireless default.
    let _ = manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some(SERIAL.to_string()),
            backend: None,
            install_companion: false,
            start_scrcpy: false,
        }))
        .await;

    let connect_line = format!("adb connect {WIRELESS}");
    assert!(
        runner
            .recorded_calls()
            .iter()
            .any(|call| call == &connect_line),
        "wireless_auto_connect must pre-connect the wireless default; recorded: {:?}",
        runner.recorded_calls()
    );
}

/// With `wireless_auto_connect` at its default (false), the configured wireless
/// default is NOT pre-connected when the explicit target is a different (USB)
/// device — the auto-connect path is the only thing that would touch it, so its
/// absence proves default behavior is unchanged.
#[tokio::test]
async fn connect_does_not_auto_connect_when_disabled() {
    const WIRELESS: &str = "192.168.1.42:39000";
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);

    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    assert!(!selection.wireless_auto_connect);
    selection.companion_enabled = false;
    selection.companion_auto_install = false;
    selection.default_serial = Some(WIRELESS.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);

    let _ = manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest {
            serial: Some(SERIAL.to_string()),
            backend: None,
            install_companion: false,
            start_scrcpy: false,
        }))
        .await;

    let connect_line = format!("adb connect {WIRELESS}");
    assert!(
        !runner
            .recorded_calls()
            .iter()
            .any(|call| call == &connect_line),
        "no adb connect must touch the wireless default with auto-connect off; recorded: {:?}",
        runner.recorded_calls()
    );
}

/// With `companion_operator_mode` off, the silent auto-install convenience is
/// suppressed: a connect against a NOT-installed companion does not run
/// `adb ... install -r`, even though `companion_auto_install` is on.
#[tokio::test]
async fn connect_suppresses_auto_install_when_operator_mode_off() {
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    // Companion not installed -> decide_install == Install.
    runner.set_stdout("adb", &["-s", SERIAL, "shell", "pm", "path", &package], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    // The crux: operator mode off must gate the silent install convenience.
    selection.companion_operator_mode = false;
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);

    let _ = connect(&mut manager).await;

    assert!(
        !runner
            .recorded_calls()
            .iter()
            .any(|call| call.contains(" install ")),
        "operator_mode off must suppress the silent auto-install; recorded: {:?}",
        runner.recorded_calls()
    );
}

/// An installed companion that is trusted but older than the staged APK is still
/// governed by the same install gate: connect may forward/probe it, but must not
/// run `adb install -r` unless auto-install or an explicit install request allows it.
#[tokio::test]
async fn connect_suppresses_trusted_companion_update_when_auto_install_off() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 1, "1.0.0");
    let port_arg = format!("tcp:{}", server.port);
    runner.set_stdout("adb", &["-s", SERIAL, "forward", &port_arg, &port_arg], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = false;
    selection.companion_rpc_port = server.port;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let apk = selection.companion_apk_path.clone();
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);

    let session = connect(&mut manager).await;

    assert!(
        session.capability_profile.companion.rpc_reachable,
        "an installed companion should still be forwarded/probed without installing"
    );
    let calls = runner.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|call| call == &format!("adb -s {SERIAL} install -r {apk}")),
        "older trusted companion must not update when auto-install is off: {calls:?}"
    );
    assert!(
        calls.iter().any(|call| call.contains(" forward tcp:")),
        "connect should still forward installed companion RPC: {calls:?}"
    );
}

/// With operator mode at its default (true) and auto-install on, the same
/// not-installed connect DOES run the install convenience (default behavior).
#[tokio::test]
async fn connect_runs_auto_install_with_operator_mode_default() {
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    runner.set_stdout("adb", &["-s", SERIAL, "shell", "pm", "path", &package], "");
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    // operator_mode left at its default (true).
    assert!(selection.companion_operator_mode);
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);

    let _ = connect(&mut manager).await;

    assert!(
        runner
            .recorded_calls()
            .iter()
            .any(|call| call.contains(" install ")),
        "operator_mode default (true) + auto_install must run the install; recorded: {:?}",
        runner.recorded_calls()
    );
}

/// With `visible_overlay` off, a connect/tap against a reachable companion still
/// dispatches real input through the companion gesture lane but issues NO
/// visible-overlay calls (`overlay_active` / `overlay_gesture`), and the session's
/// cursor capabilities report the resolved disabled state honestly.
#[tokio::test]
async fn visible_overlay_off_suppresses_companion_overlay_calls() {
    let server = RecordingCompanion::start().await;
    let runner = Arc::new(FakeCommandRunner::new());
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    // The crux: the visible overlay is disabled in config.
    selection.visible_overlay = false;
    let mut manager = PhoneManager::with_runner(runner, selection);
    let now = PhoneManager::now_ms_for_tests();
    manager.insert_companion_session_for_tests("sess-1", SERIAL, server.port, COMPANION_TOKEN, now);

    // A successful tap must NOT animate the overlay when it is disabled.
    let action = match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector("sess-1"),
            phone_snapshot_id: None,
            x: 100.0,
            y: 200.0,
            use_device_coordinates: true,
        }))
        .await
    {
        PhoneResponse::Action(action) => action,
        other => panic!("unexpected: {other:?}"),
    };
    // The real input still dispatched through the companion gesture lane (the
    // suppression is cosmetic only).
    assert_eq!(
        action.backend,
        PhoneBackendKind::Companion,
        "{:?}",
        action.diagnostics
    );
    assert!(
        server.request_for("overlay_gesture").is_none(),
        "visible_overlay=false must suppress overlay_gesture; recorded: {:?}",
        server.recorded()
    );
    assert!(
        server.request_for("overlay_active").is_none(),
        "visible_overlay=false must suppress overlay_active; recorded: {:?}",
        server.recorded()
    );
}

/// `cursor_capabilities` reports the resolved `visible_overlay=false` state
/// honestly: both visible planes are off with a config-grounded reason, even for a
/// companion-reachable profile that would otherwise advertise a native overlay.
/// The screenshot-synthetic plane is independent and tracks `screenshot_cursor`.
#[test]
fn cursor_capabilities_report_visible_overlay_disabled_honestly() {
    use sky_cua_platform::model::PhoneCompanionCapabilities;

    let runner = Arc::new(FakeCommandRunner::new());
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.visible_overlay = false;
    assert!(
        selection.screenshot_cursor,
        "default keeps synthetic cursor on"
    );
    let manager = PhoneManager::with_runner(runner, selection);

    let mut profile = manager.detached_profile_for_tests();
    let mut companion = PhoneCompanionCapabilities::absent("com.skycua.phonecompanion");
    companion.rpc_reachable = true;
    companion.native_overlay = true;
    profile.companion = companion;

    let caps = manager.cursor_capabilities(&profile);
    assert!(!caps.host_visible_overlay);
    assert!(!caps.phone_native_overlay);
    // The synthetic marker is a separate plane and stays on.
    assert!(caps.screenshot_synthetic_cursor);
    let reason = caps
        .visible_overlay_reason
        .expect("a disabled overlay must report a reason");
    assert!(
        reason.contains("visible_overlay"),
        "reason must name the config toggle: {reason}"
    );
}
