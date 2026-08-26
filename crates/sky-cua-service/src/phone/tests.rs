//! Phone service unit tests: manager routing through a `FakeCommandRunner`, the
//! capability-profile cache TTL rule, and the stub backend constructors the
//! parallel lanes will replace.

use std::sync::Arc;

use sky_cua_platform::config::{PhoneConfig, resolve_phone_selection};
use sky_cua_platform::model::{
    PhoneAppListRequest, PhoneAppResponseKind, PhoneBackendKind, PhoneCapabilityRefreshState,
    PhoneConnectRequest, PhoneListDevicesRequest, PhoneNotificationsRequest,
    PhonePairWirelessRequest, PhoneRequest, PhoneResponse, PhoneSessionSelector,
    PhoneStatusRequest, PhoneTapRequest,
};

use super::command::{CommandRunner, FakeCommandRunner, RealCommandRunner};
use super::manager::PhoneManager;
use super::{adb, cursor, device, mapping, scrcpy};

fn manager_with(runner: Arc<dyn CommandRunner>) -> PhoneManager {
    let selection = resolve_phone_selection(&PhoneConfig::default());
    PhoneManager::with_runner(runner, selection)
}

fn selector() -> PhoneSessionSelector {
    PhoneSessionSelector {
        session_id: Some("sess-1".to_string()),
        serial: Some("emulator-5554".to_string()),
        device_id: None,
        alias: None,
        appshot_id: None,
    }
}

#[tokio::test]
async fn status_request_routes_through_command_runner_and_stays_honest() {
    let runner = Arc::new(FakeCommandRunner::new());
    let mut manager = manager_with(runner.clone());

    match manager
        .handle(PhoneRequest::Status(PhoneStatusRequest::default()))
        .await
    {
        PhoneResponse::Status(report) => {
            assert!(!report.adb_available);
            assert_eq!(report.default_backend, PhoneBackendKind::None);
            assert!(report.sessions.is_empty());
            // The unscripted fake runner returns a structured NotImplemented
            // error for `adb version`; `probe_host` surfaces that as the
            // command diagnostic and keeps `adb_available` false.
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneCommandNotImplemented")
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn list_devices_returns_empty_with_diagnostic() {
    let runner = Arc::new(FakeCommandRunner::new());
    let mut manager = manager_with(runner);

    match manager
        .handle(PhoneRequest::ListDevices(PhoneListDevicesRequest {
            include_mdns: true,
        }))
        .await
    {
        PhoneResponse::Devices(response) => {
            assert!(response.devices.is_empty());
            // With no scripted `adb version` result the runner reports a
            // structured NotImplemented error rather than fabricating devices.
            assert!(response.adb_path.is_none());
            assert!(
                response
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneCommandNotImplemented")
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn connect_does_not_fabricate_a_session() {
    let runner = Arc::new(FakeCommandRunner::new());
    let mut manager = manager_with(runner);

    // Connect/observe/screenshot/refresh have no live device in Phase 1, so they
    // must answer with the honest host-status view, never a `Connected` session.
    match manager
        .handle(PhoneRequest::Connect(PhoneConnectRequest::default()))
        .await
    {
        PhoneResponse::Status(report) => assert!(report.sessions.is_empty()),
        other => panic!("connect must not fabricate a session, got: {other:?}"),
    }
}

#[tokio::test]
async fn tap_returns_action_response_with_no_backend() {
    let runner = Arc::new(FakeCommandRunner::new());
    let mut manager = manager_with(runner);

    match manager
        .handle(PhoneRequest::Tap(PhoneTapRequest {
            session: selector(),
            phone_snapshot_id: Some("snap-1".to_string()),
            x: 10.0,
            y: 20.0,
            use_device_coordinates: false,
        }))
        .await
    {
        PhoneResponse::Action(response) => {
            assert_eq!(response.action, "phone_tap");
            assert_eq!(response.backend, PhoneBackendKind::None);
            assert_eq!(response.session_id, "sess-1");
            // No session was ever connected for this selector, so the tap is
            // rejected before any backend dispatch with a structured no-session
            // diagnostic (the agent must call phone_connect first).
            assert!(
                response
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneNoSession")
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn app_list_returns_app_response_kind_list() {
    let runner = Arc::new(FakeCommandRunner::new());
    let mut manager = manager_with(runner);

    match manager
        .handle(PhoneRequest::AppList(PhoneAppListRequest {
            session: selector(),
            include_system: false,
            limit: None,
        }))
        .await
    {
        PhoneResponse::App(response) => {
            assert_eq!(response.kind, PhoneAppResponseKind::List);
            assert!(!response.success);
            assert!(response.apps.is_empty());
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn notification_reply_collapses_to_notifications_response() {
    let runner = Arc::new(FakeCommandRunner::new());
    let mut manager = manager_with(runner);

    match manager
        .handle(PhoneRequest::Notifications(PhoneNotificationsRequest {
            session: selector(),
            limit: Some(5),
        }))
        .await
    {
        PhoneResponse::Notifications(response) => {
            assert!(!response.listener_enabled);
            assert!(response.events.is_empty());
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn pair_wireless_echoes_host_port_without_pairing_code() {
    let runner = Arc::new(FakeCommandRunner::new());
    let mut manager = manager_with(runner);

    match manager
        .handle(PhoneRequest::PairWireless(PhonePairWirelessRequest {
            host_port: "192.168.1.5:37000".to_string(),
            pairing_code: "424242".to_string(),
        }))
        .await
    {
        PhoneResponse::PairedWireless(response) => {
            assert!(!response.paired);
            assert_eq!(response.host_port, "192.168.1.5:37000");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn real_command_runner_maps_missing_binary_to_spawn_error() {
    // The real runner now executes processes; a missing binary must surface a
    // structured spawn error rather than pretending success or contacting a
    // device. (Deeper real-runner coverage lives in `command::tests`.)
    let runner = RealCommandRunner;
    let error = runner
        .run("sky-cua-nonexistent-binary-xyz", &[])
        .await
        .expect_err("missing binary must not pretend success");
    assert_eq!(error.code(), "PhoneCommandSpawnFailed");
}

#[tokio::test]
async fn fake_command_runner_records_invocations_in_order() {
    let runner = FakeCommandRunner::new();
    runner.push_stdout("Android Debug Bridge version 1.0.41");
    let output = runner
        .run("adb", &["version"])
        .await
        .expect("scripted command");
    assert!(output.success());
    assert!(output.stdout_string().contains("1.0.41"));
    assert_eq!(runner.recorded_calls(), vec!["adb version".to_string()]);
}

#[test]
fn capability_cache_marks_profile_stale_past_ttl() {
    let runner = Arc::new(FakeCommandRunner::new());
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    selection.capability_cache_ttl_ms = 1_000;
    let mut manager = PhoneManager::with_runner(runner, selection);

    let detected_at = 10_000;
    let profile = futures_block_on(device::detect_profile(
        &FakeCommandRunner::new(),
        "sess-1",
        "emulator-5554",
        "com.skycua.phonecompanion",
        detected_at,
        PhoneCapabilityRefreshState::Detected,
    ));
    manager.insert_profile_for_tests(profile, detected_at);

    // Within TTL: a request that reuses the still-fresh cached profile reports
    // Reused (the stored detected state stays Detected; only this per-request
    // clone flips), and is not stale.
    let fresh = manager
        .cached_profile_for_tests("sess-1", detected_at + 500)
        .expect("cached profile");
    assert!(!fresh.stale);
    assert_eq!(fresh.refresh_state, PhoneCapabilityRefreshState::Reused);

    // Past TTL: marked stale.
    let stale = manager
        .cached_profile_for_tests("sess-1", detected_at + 5_000)
        .expect("cached profile");
    assert!(stale.stale);
    assert_eq!(stale.refresh_state, PhoneCapabilityRefreshState::Stale);
}

#[test]
fn stub_backend_constructors_report_nothing_available() {
    // The seam constructors the parallel lanes will replace must default to the
    // honest "nothing available" shape so no stub fabricates device state.
    let caps = device::empty_backend_capabilities();
    assert!(!caps.adb && !caps.companion && !caps.scrcpy);

    let companion = crate::phone::protocol::absent_companion("com.skycua.phonecompanion");
    assert!(!companion.installed);
    assert_eq!(companion.package_name, "com.skycua.phonecompanion");

    let scrcpy = scrcpy::absent_scrcpy();
    assert!(!scrcpy.installed && !scrcpy.active);

    let cursor = cursor::no_cursor_capabilities();
    assert!(!cursor.host_visible_overlay && !cursor.phone_native_overlay);

    let mapping = mapping::identity_mapping(
        "map-1",
        "sess-1",
        "emulator-5554",
        sky_cua_platform::model::PixelSize {
            width: 1080,
            height: 2400,
        },
        PhoneManager::now_ms_for_tests(),
    );
    assert_eq!(mapping.rotation_degrees, 0);
    assert_eq!(mapping.device_rect.width, 1080.0);

    assert_eq!(
        adb::not_implemented_diagnostic().code,
        "PhoneAdbNotImplemented"
    );
}

/// Minimal synchronous block-on for the handful of `async fn` stub calls a
/// `#[test]` (non-tokio) needs. Avoids adding a new dependency: we drive the
/// future with a no-op waker on the current thread. All stub futures are
/// immediately ready, so a single poll completes them.
fn futures_block_on<F: std::future::Future>(mut future: F) -> F::Output {
    use std::pin::Pin;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn noop_raw_waker() -> RawWaker {
        fn no_op(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker {
            noop_raw_waker()
        }
        let vtable = &RawWakerVTable::new(clone, no_op, no_op, no_op);
        RawWaker::new(std::ptr::null(), vtable)
    }

    let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
    let mut context = Context::from_waker(&waker);
    // SAFETY: `future` is owned and not moved after pinning.
    let mut future = unsafe { Pin::new_unchecked(&mut future) };
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => continue,
        }
    }
}
