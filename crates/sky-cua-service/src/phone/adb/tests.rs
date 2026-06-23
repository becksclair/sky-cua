//! ADB parser and command-wrapper tests. Parsers are exercised directly over
//! representative `adb` output (normal, unauthorized, offline, malformed, empty,
//! multi-device). Wrappers run through a [`FakeCommandRunner`] so no real device
//! is touched.

use sky_cua_platform::model::{PhoneConnectionKind, PhoneDeviceState, PhoneSettingsScreen};

use super::parse::{classify_device_state, parse_install_failure, parse_server_status};
use super::*;
use crate::phone::command::{CommandError, CommandOutput, FakeCommandRunner};

#[test]
fn version_parses_from_real_banner() {
    let stdout = "Android Debug Bridge version 1.0.41\nVersion 35.0.2-12317457\n";
    assert_eq!(parse_version(stdout).as_deref(), Some("1.0.41"));
}

#[test]
fn version_none_on_empty_or_garbage() {
    assert_eq!(parse_version(""), None);
    assert_eq!(parse_version("totally unrelated"), None);
}

#[test]
fn server_status_counts_transports() {
    let stdout = "List of devices attached\nemulator-5554\tdevice\n10.0.0.5:5555\tdevice\n";
    assert_eq!(parse_server_status(stdout), Some(2));
}

#[test]
fn server_status_zero_with_header_only() {
    assert_eq!(parse_server_status("List of devices attached\n\n"), Some(0));
}

#[test]
fn server_status_none_without_header() {
    assert_eq!(parse_server_status("error: cannot connect to daemon"), None);
}

#[test]
fn devices_l_parses_usb_with_metadata() {
    let stdout = "List of devices attached\nR5CT30ABCDE device usb:1-3 product:p3qxxx model:SM_S948B device:p3q transport_id:4\n";
    let devices = parse_devices_l(stdout);
    assert_eq!(devices.len(), 1);
    let d = &devices[0];
    assert_eq!(d.serial, "R5CT30ABCDE");
    assert_eq!(d.state, PhoneDeviceState::Device);
    assert_eq!(d.connection_kind, PhoneConnectionKind::Usb);
    assert_eq!(d.model.as_deref(), Some("SM_S948B"));
    assert_eq!(d.product.as_deref(), Some("p3qxxx"));
    assert_eq!(d.device.as_deref(), Some("p3q"));
    assert_eq!(d.transport_id.as_deref(), Some("4"));
}

#[test]
fn devices_l_classifies_unauthorized_offline_nopermissions() {
    let stdout = "List of devices attached\nABC123\tunauthorized\nDEF456\toffline\nGHI789  no permissions (user in plugdev group); see [http://...]\n";
    let devices = parse_devices_l(stdout);
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[0].state, PhoneDeviceState::Unauthorized);
    assert_eq!(devices[1].state, PhoneDeviceState::Offline);
    assert_eq!(devices[2].state, PhoneDeviceState::NoPermissions);
    assert_eq!(devices[2].serial, "GHI789");
}

#[test]
fn devices_l_classifies_emulator_and_wireless() {
    let stdout = "List of devices attached\nemulator-5554 device product:sdk model:A device:e transport_id:1\n10.0.0.5:5555 device product:p model:X device:d transport_id:2\n172.16.255.58:38781 device product:p model:Y device:d transport_id:3\nadb-RF8-abc._adb-tls-connect._tcp device transport_id:5\n";
    let devices = parse_devices_l(stdout);
    assert_eq!(devices.len(), 4);
    assert_eq!(devices[0].connection_kind, PhoneConnectionKind::Emulator);
    assert_eq!(devices[1].connection_kind, PhoneConnectionKind::LegacyTcpip);
    assert_eq!(
        devices[2].connection_kind,
        PhoneConnectionKind::WirelessDebugging
    );
    assert_eq!(
        devices[3].connection_kind,
        PhoneConnectionKind::WirelessDebugging
    );
}

#[test]
fn devices_l_empty_and_header_only() {
    assert!(parse_devices_l("").is_empty());
    assert!(parse_devices_l("List of devices attached\n").is_empty());
}

#[test]
fn devices_l_skips_daemon_notices() {
    let stdout = "* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\nemulator-5554\tdevice\n";
    let devices = parse_devices_l(stdout);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].serial, "emulator-5554");
}

#[test]
fn classify_connection_kind_edges() {
    assert_eq!(
        classify_connection_kind("emulator-5556"),
        PhoneConnectionKind::Emulator
    );
    assert_eq!(
        classify_connection_kind("R5CT30ABCDE"),
        PhoneConnectionKind::Usb
    );
    assert_eq!(
        classify_connection_kind("192.168.1.9:5555"),
        PhoneConnectionKind::LegacyTcpip
    );
    assert_eq!(
        classify_connection_kind("192.168.1.9:41237"),
        PhoneConnectionKind::WirelessDebugging
    );
    assert_eq!(
        classify_connection_kind("weird:serial"),
        PhoneConnectionKind::Usb
    );
}

#[test]
fn classify_device_state_unknown_for_garbage() {
    assert_eq!(classify_device_state("zzz"), PhoneDeviceState::Unknown);
    assert_eq!(classify_device_state(""), PhoneDeviceState::Unknown);
    assert_eq!(
        classify_device_state("recovery"),
        PhoneDeviceState::Recovery
    );
}

#[test]
fn mdns_services_parses_rows() {
    let stdout = "List of discovered mdns services\nadb-RF8N abc._adb-tls-connect._tcp 172.16.255.58:38781\n";
    let services = parse_mdns_services(stdout);
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].0, "adb-RF8N");
}

#[test]
fn mdns_services_empty() {
    assert!(parse_mdns_services("List of discovered mdns services\n").is_empty());
    assert!(parse_mdns_services("").is_empty());
}

#[test]
fn wm_size_prefers_override() {
    let stdout = "Physical size: 1440x3120\nOverride size: 1080x2340\n";
    assert_eq!(parse_wm_size(stdout), Some((1080, 2340)));
}

#[test]
fn wm_size_physical_only() {
    assert_eq!(
        parse_wm_size("Physical size: 1080x2400\n"),
        Some((1080, 2400))
    );
}

#[test]
fn wm_size_malformed_is_none() {
    assert_eq!(parse_wm_size("Physical size: garbage\n"), None);
    assert_eq!(parse_wm_size(""), None);
}

#[test]
fn wm_density_prefers_override() {
    assert_eq!(
        parse_wm_density("Physical density: 560\nOverride density: 420\n"),
        Some(420)
    );
}

#[test]
fn wm_density_none_on_garbage() {
    assert_eq!(parse_wm_density("Physical density: x\n"), None);
}

#[test]
fn rotation_from_dumpsys_input_portrait() {
    // `dumpsys input` exposes the live rotation as `SurfaceOrientation: N`.
    let stdout = "\
Input Reader State:
  Device 4: touchscreen
    SurfaceWidth: 1080px
    SurfaceHeight: 2400px
    SurfaceOrientation: 0
";
    let rotation = super::parse::parse_rotation(stdout).expect("rotation");
    assert_eq!(rotation.quarter_turns, 0);
    assert_eq!(rotation.degrees, 0);
    assert_eq!(rotation.label, "portrait");
}

#[test]
fn rotation_from_dumpsys_input_landscape() {
    let stdout = "    SurfaceOrientation: 1\n";
    let rotation = super::parse::parse_rotation(stdout).expect("rotation");
    assert_eq!(rotation.quarter_turns, 1);
    assert_eq!(rotation.degrees, 90);
    assert_eq!(rotation.label, "landscape");
}

#[test]
fn rotation_from_dumpsys_input_upside_down_portrait() {
    let stdout = "SurfaceOrientation: 2\n";
    let rotation = super::parse::parse_rotation(stdout).expect("rotation");
    assert_eq!(rotation.quarter_turns, 2);
    assert_eq!(rotation.degrees, 180);
    assert_eq!(rotation.label, "portrait");
}

#[test]
fn rotation_falls_back_to_dumpsys_display() {
    // No `SurfaceOrientation`; the `dumpsys display` form is recognized instead.
    let stdout = "\
Logical Displays:
  Display 0:
    mCurrentOrientation=3
";
    let rotation = super::parse::parse_rotation(stdout).expect("rotation");
    assert_eq!(rotation.quarter_turns, 3);
    assert_eq!(rotation.degrees, 270);
    assert_eq!(rotation.label, "landscape");
}

#[test]
fn rotation_none_on_unrecognized_or_out_of_range() {
    // Empty, unrelated, and out-of-range values all yield None so the caller
    // falls back to the aspect-derived label rather than fabricating a rotation.
    assert!(super::parse::parse_rotation("").is_none());
    assert!(super::parse::parse_rotation("nothing rotational here\n").is_none());
    assert!(super::parse::parse_rotation("SurfaceOrientation: 7\n").is_none());
    assert!(super::parse::parse_rotation("SurfaceOrientation: \n").is_none());
}

#[tokio::test]
async fn screen_rotation_reads_dumpsys_input() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "input"],
        "    SurfaceOrientation: 1\n",
    );
    let rotation = screen_rotation(&runner, None, "S").await.expect("rotation");
    assert_eq!(rotation.label, "landscape");
}

#[tokio::test]
async fn screen_rotation_falls_back_to_dumpsys_display() {
    let runner = FakeCommandRunner::new();
    // `dumpsys input` runs but exposes no rotation; `dumpsys display` does.
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "input"],
        "Input Reader State:\n  no orientation here\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "display"],
        "  mCurrentOrientation=0\n",
    );
    let rotation = screen_rotation(&runner, None, "S").await.expect("rotation");
    assert_eq!(rotation.label, "portrait");
}

#[tokio::test]
async fn screen_rotation_none_when_no_source_resolves() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "input"],
        "nothing\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "display"],
        "still nothing\n",
    );
    assert!(screen_rotation(&runner, None, "S").await.is_none());
}

#[test]
fn current_focus_parses_dotted_activity() {
    let stdout = "  mCurrentFocus=Window{a1b2c3 u0 com.android.settings/.MainActivity}\n";
    let app = parse_current_focus(stdout).expect("focus");
    assert_eq!(app.package, "com.android.settings");
    assert_eq!(
        app.activity.as_deref(),
        Some("com.android.settings.MainActivity")
    );
}

#[test]
fn current_focus_parses_fully_qualified_activity() {
    let stdout = "mResumedActivity: ActivityRecord{x u0 com.example/com.example.ui.Home t42}\n";
    let app = parse_current_focus(stdout).expect("focus");
    assert_eq!(app.package, "com.example");
    assert_eq!(app.activity.as_deref(), Some("com.example.ui.Home"));
}

#[test]
fn current_focus_none_when_absent_or_garbage() {
    assert!(parse_current_focus("mCurrentFocus=null").is_none());
    assert!(parse_current_focus("").is_none());
}

#[test]
fn package_list_parses_and_strips_paths() {
    let stdout = "package:com.android.chrome\npackage:/data/app/Foo.apk=com.foo.bar\n\n";
    assert_eq!(
        parse_package_list(stdout),
        vec!["com.android.chrome", "com.foo.bar"]
    );
}

#[test]
fn package_list_empty() {
    assert!(parse_package_list("").is_empty());
    assert!(parse_package_list("garbage line").is_empty());
}

#[test]
fn install_failure_extracts_known_class() {
    let out = "Performing Streamed Install\nFailure [INSTALL_FAILED_VERSION_DOWNGRADE]\n";
    assert_eq!(
        parse_install_failure(out).as_deref(),
        Some("INSTALL_FAILED_VERSION_DOWNGRADE")
    );
}

#[test]
fn install_failure_strips_trailing_punctuation_from_known_class() {
    let out =
        "Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package signatures do not match]";
    assert_eq!(
        parse_install_failure(out).as_deref(),
        Some("INSTALL_FAILED_UPDATE_INCOMPATIBLE")
    );
}

#[test]
fn install_failure_parse_failed_class() {
    let out = "Failure [INSTALL_PARSE_FAILED_NO_CERTIFICATES]";
    assert_eq!(
        parse_install_failure(out).as_deref(),
        Some("INSTALL_PARSE_FAILED_NO_CERTIFICATES")
    );
}

#[test]
fn install_failure_unknown_on_generic_error() {
    assert_eq!(
        parse_install_failure("adb: error: something went wrong").as_deref(),
        Some("INSTALL_FAILED_UNKNOWN")
    );
}

#[test]
fn install_failure_none_on_success() {
    assert_eq!(parse_install_failure("Success\n"), None);
}

#[test]
fn single_quote_for_shell_makes_text_literal() {
    // Spaces and a literal `%` survive verbatim inside single quotes (the old
    // `%s`-for-space scheme corrupted "50% off" into "50%%soff").
    assert_eq!(single_quote_for_shell("hello world"), "'hello world'");
    assert_eq!(single_quote_for_shell("50% off"), "'50% off'");
    // Double quotes and shell metacharacters are literal inside single quotes.
    assert_eq!(single_quote_for_shell("a&b;c|d"), "'a&b;c|d'");
    assert_eq!(single_quote_for_shell("$(rm -rf /)"), "'$(rm -rf /)'");
    assert_eq!(single_quote_for_shell("say \"hi\""), "'say \"hi\"'");
    // An embedded single quote uses the standard close/escape/reopen sequence.
    assert_eq!(single_quote_for_shell("it's"), "'it'\\''s'");
    assert_eq!(single_quote_for_shell("'"), "''\\'''");
}

#[tokio::test]
async fn input_text_passes_single_quoted_command_argument() {
    let runner = FakeCommandRunner::new();
    // The whole `input text '...'` is one device-shell argument, so a literal `%`
    // and spaces reach `input` unchanged.
    runner.set_stdout("adb", &["-s", "S", "shell", "input text '50% off'"], "");
    let outcome = input_text(&runner, None, "S", "50% off")
        .await
        .expect("input text");
    assert!(outcome.success);
    // The recorded argv shape mirrors how tap/swipe build their argv: a single
    // `shell` command argument, here carrying the quoted text.
    let calls = runner.recorded_calls();
    assert!(
        calls
            .iter()
            .any(|c| c == "adb -s S shell input text '50% off'"),
        "expected single-quoted shell command, got {calls:?}"
    );
}

#[test]
fn normalize_keyevent_forms() {
    assert_eq!(normalize_keyevent("4"), "4");
    assert_eq!(normalize_keyevent("back"), "KEYCODE_BACK");
    assert_eq!(normalize_keyevent("KEYCODE_HOME"), "KEYCODE_HOME");
    assert_eq!(normalize_keyevent("keycode_enter"), "KEYCODE_ENTER");
}

#[test]
fn settings_actions_are_concrete() {
    assert_eq!(
        settings_action(PhoneSettingsScreen::Accessibility),
        "android.settings.ACCESSIBILITY_SETTINGS"
    );
    assert!(
        settings_action(PhoneSettingsScreen::NotificationAccess).contains("NOTIFICATION_LISTENER")
    );
    assert!(!settings_action(PhoneSettingsScreen::WirelessDebugging).contains(' '));
}

#[tokio::test]
async fn probe_host_reports_available_when_version_succeeds() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout("adb", &["version"], "Android Debug Bridge version 1.0.41");
    runner.set_stdout("adb", &["devices"], "List of devices attached\n");
    runner.set_stdout(
        "adb",
        &["mdns", "check"],
        "mdns daemon version zeroconf 1.2",
    );
    let report = probe_host(&runner, true, true).await;
    assert!(report.adb_available);
    assert_eq!(report.adb_version.as_deref(), Some("1.0.41"));
    assert_eq!(report.adb_server_running, Some(true));
    assert!(report.mdns_available);
    assert!(report.diagnostics.is_empty());
}

#[tokio::test]
async fn probe_host_missing_adb_is_unavailable_with_spawn_diagnostic() {
    let runner = FakeCommandRunner::new();
    runner.set_error(
        "adb",
        &["version"],
        CommandError::Spawn {
            program: "adb".to_string(),
            message: "No such file or directory".to_string(),
        },
    );
    let report = probe_host(&runner, true, false).await;
    assert!(!report.adb_available);
    assert!(report.adb_path.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PhoneCommandSpawnFailed")
    );
}

#[tokio::test]
async fn list_devices_parses_two_devices() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout("adb", &["version"], "Android Debug Bridge version 1.0.41");
    runner.set_stdout(
        "adb",
        &["devices", "-l"],
        "List of devices attached\nemulator-5554 device model:Emu\nR5CT unauthorized\n",
    );
    let response = list_devices(&runner, false).await;
    assert_eq!(response.devices.len(), 2);
    assert_eq!(response.adb_version.as_deref(), Some("1.0.41"));
}

#[tokio::test]
async fn list_devices_without_adb_reports_no_path() {
    let runner = FakeCommandRunner::new();
    let response = list_devices(&runner, false).await;
    assert!(response.devices.is_empty());
    assert!(response.adb_path.is_none());
    assert!(!response.diagnostics.is_empty());
}

#[tokio::test]
async fn screencap_returns_png_bytes() {
    let runner = FakeCommandRunner::new();
    runner.set_output(
        "adb",
        &["-s", "emulator-5554", "exec-out", "screencap", "-p"],
        CommandOutput {
            status: Some(0),
            stdout: vec![0x89, b'P', b'N', b'G'],
            stderr: Vec::new(),
        },
    );
    let bytes = screencap_png(&runner, None, "emulator-5554")
        .await
        .expect("png");
    assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
}

#[tokio::test]
async fn screencap_failure_maps_to_spawn_error() {
    let runner = FakeCommandRunner::new();
    runner.set_output(
        "adb",
        &["-s", "S", "exec-out", "screencap", "-p"],
        CommandOutput {
            status: Some(1),
            stdout: Vec::new(),
            stderr: b"screencap: permission denied".to_vec(),
        },
    );
    let error = screencap_png(&runner, None, "S").await.expect_err("err");
    assert_eq!(error.code(), "PhoneCommandSpawnFailed");
}

#[tokio::test]
async fn input_tap_builds_expected_argv() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "input", "tap", "100", "250"],
        "",
    );
    let outcome = input_tap(&runner, None, "S", 100, 250).await.expect("tap");
    assert!(outcome.success);
    assert_eq!(
        runner.recorded_calls(),
        vec!["adb -s S shell input tap 100 250".to_string()]
    );
}

#[tokio::test]
async fn input_swipe_includes_duration() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &[
            "-s", "S", "shell", "input", "swipe", "10", "20", "30", "40", "300",
        ],
        "",
    );
    let outcome = input_swipe(&runner, None, "S", (10, 20), (30, 40), Some(300))
        .await
        .expect("swipe");
    assert!(outcome.success);
}

#[tokio::test]
async fn input_text_single_quotes_spaces_literally() {
    let runner = FakeCommandRunner::new();
    // Spaces are literal inside the single-quoted shell argument; no `%s` scheme.
    runner.set_stdout("adb", &["-s", "S", "shell", "input text 'hi there'"], "");
    let outcome = input_text(&runner, None, "S", "hi there")
        .await
        .expect("text");
    assert!(outcome.success);
    assert_eq!(
        runner.recorded_calls(),
        vec!["adb -s S shell input text 'hi there'".to_string()]
    );
}

#[tokio::test]
async fn connect_classifies_failure_text_despite_exit_zero() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["connect", "10.0.0.5:5555"],
        "failed to connect to '10.0.0.5:5555'",
    );
    let outcome = connect(&runner, None, "10.0.0.5:5555")
        .await
        .expect("connect");
    assert!(!outcome.success);
}

#[tokio::test]
async fn connect_success_text() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["connect", "10.0.0.5:5555"],
        "connected to 10.0.0.5:5555",
    );
    let outcome = connect(&runner, None, "10.0.0.5:5555")
        .await
        .expect("connect");
    assert!(outcome.success);
}

#[tokio::test]
async fn pair_never_echoes_pairing_code() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["pair", "10.0.0.5:37000"],
        "Successfully paired to 10.0.0.5:37000",
    );
    let outcome = pair_wireless(&runner, None, "10.0.0.5:37000", "424242")
        .await
        .expect("pair");
    assert!(outcome.success);
    assert!(!outcome.message.contains("424242"));
    assert_eq!(
        runner.recorded_calls(),
        vec!["adb pair 10.0.0.5:37000".to_string()]
    );
}

// install/forward wrapper tests live in the `install` child module.

#[tokio::test]
async fn launch_package_detects_missing_activity() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &[
            "-s",
            "S",
            "shell",
            "monkey -p 'com.no.launcher' -c android.intent.category.LAUNCHER 1",
        ],
        "** No activities found to run, monkey aborted.",
    );
    let outcome = launch_package(&runner, None, "S", "com.no.launcher")
        .await
        .expect("launch");
    assert!(!outcome.success);
}

#[tokio::test]
async fn foreground_app_falls_back_to_activity_dump() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "window"],
        "no focus here\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "activity", "activities"],
        "mResumedActivity: ActivityRecord{x u0 com.example/.Home t1}\n",
    );
    let app = foreground_app(&runner, None, "S")
        .await
        .expect("ok")
        .expect("some app");
    assert_eq!(app.package, "com.example");
}

#[tokio::test]
async fn foreground_app_failed_fallback_probe_is_not_silent_none() {
    // `dumpsys window` succeeds but exposes no focus; the activity-dump fallback
    // then fails (transient wireless drop). The foreground is UNKNOWN, not empty,
    // so this must surface as `Err`, never a silent `Ok(None)`.
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "window"],
        "no focus here\n",
    );
    runner.set_output(
        "adb",
        &["-s", "S", "shell", "dumpsys", "activity", "activities"],
        CommandOutput {
            status: Some(1),
            stdout: Vec::new(),
            stderr: b"error: device offline".to_vec(),
        },
    );
    let result = foreground_app(&runner, None, "S").await;
    assert!(
        result.is_err(),
        "a failed fallback probe must not be reported as Ok(None): {result:?}"
    );
}

#[tokio::test]
async fn foreground_app_clean_probes_with_no_focus_is_ok_none() {
    // Both probes run cleanly but neither exposes a focus: genuinely no
    // resolvable foreground app, which is an honest `Ok(None)`.
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "window"],
        "no focus here\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "activity", "activities"],
        "nothing resumed here\n",
    );
    let result = foreground_app(&runner, None, "S").await.expect("ok");
    assert!(result.is_none());
}

#[tokio::test]
async fn force_stop_still_foreground_is_reported_ineffective() {
    // `am force-stop` exits 0, but the target is still foreground afterward, so
    // the stop was ineffective and must not be reported as a success.
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "am force-stop 'com.target'"],
        "",
    );
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "window"],
        "  mCurrentFocus=Window{a u0 com.target/.MainActivity}\n",
    );
    let outcome = force_stop(&runner, None, "S", "com.target")
        .await
        .expect("force-stop");
    assert!(!outcome.success);
    assert!(outcome.message.contains("ineffective"));
}

#[tokio::test]
async fn force_stop_no_longer_foreground_is_success() {
    // `am force-stop` exits 0 and a different app is now foreground: an honest
    // success.
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "am force-stop 'com.target'"],
        "",
    );
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "dumpsys", "window"],
        "  mCurrentFocus=Window{a u0 com.android.launcher3/.Launcher}\n",
    );
    let outcome = force_stop(&runner, None, "S", "com.target")
        .await
        .expect("force-stop");
    assert!(outcome.success);
    assert!(outcome.message.is_empty());
}

#[tokio::test]
async fn force_stop_keeps_nonzero_exit_failure() {
    // A non-zero exit is already a failure; the verification path must not run or
    // overwrite that outcome.
    let runner = FakeCommandRunner::new();
    runner.set_output(
        "adb",
        &["-s", "S", "shell", "am force-stop 'com.target'"],
        CommandOutput {
            status: Some(1),
            stdout: Vec::new(),
            stderr: b"Error: cannot stop".to_vec(),
        },
    );
    let outcome = force_stop(&runner, None, "S", "com.target")
        .await
        .expect("force-stop");
    assert!(!outcome.success);
}

#[tokio::test]
async fn display_geometry_parses_size_and_density() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "wm", "size"],
        "Physical size: 1080x2400\n",
    );
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", "wm", "density"],
        "Physical density: 420\n",
    );
    let geometry = display_geometry(&runner, None, "S")
        .await
        .expect("geometry");
    assert_eq!(geometry.width, 1080);
    assert_eq!(geometry.height, 2400);
    assert_eq!(geometry.density_dpi, Some(420));
}

#[tokio::test]
async fn open_settings_app_details_attaches_package() {
    let runner = FakeCommandRunner::new();
    runner.set_stdout(
        "adb",
        &[
            "-s",
            "S",
            "shell",
            "am start -a android.settings.APPLICATION_DETAILS_SETTINGS -d 'package:com.example'",
        ],
        "Starting: Intent { ... }",
    );
    let outcome = open_settings(
        &runner,
        None,
        "S",
        PhoneSettingsScreen::AppDetails,
        Some("com.example"),
    )
    .await
    .expect("settings");
    assert!(outcome.success);
}

#[tokio::test]
async fn start_intent_single_quotes_uri_and_package() {
    // Untrusted free-text fields reach the on-device shell as single-quoted
    // literals, so shell metacharacters cannot break out into a command. adb
    // rejoins the argv after `shell` and runs it through `sh -c`, so quoting is
    // the boundary that prevents on-device injection.
    let runner = FakeCommandRunner::new();
    let argv = "am start -a android.intent.action.VIEW -d 'about:blank$(reboot)' -p 'com.x;rm -rf'";
    runner.set_stdout(
        "adb",
        &["-s", "S", "shell", argv],
        "Starting: Intent { ... }",
    );
    let outcome = start_intent(
        &runner,
        None,
        "S",
        "about:blank$(reboot)",
        Some("com.x;rm -rf"),
    )
    .await
    .expect("intent");
    assert!(outcome.success);
    let calls = runner.recorded_calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("-d 'about:blank$(reboot)'") && c.contains("-p 'com.x;rm -rf'")),
        "expected single-quoted uri and package, got {calls:?}"
    );
}
