use super::capture_reuse::reuse_unchanged_capture;
use super::desktop::action_requires_snapshot_context;
use super::session_presence::request_should_hold_presence;
use super::{
    OverlayController, ServiceDaemon, SessionPresenceConfig, SessionStore, SnapshotManager,
};
use image::{ImageBuffer, Rgba};
use serde_json::json;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AgentCursorPoint, AgentCursorState, AppInfo,
    AppSelector, AppStateSnapshot, BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
    BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserRequest, BrowserResponse, BrowserTargetKind,
    CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace,
    DisplayTarget, ElementNode, EnvironmentInfo, InputBackendKind, ModelImageFormat,
    PhoneAppListRequest, PhoneAppResponseKind, PhoneConnectRequest, PhoneListDevicesRequest,
    PhoneRequest, PhoneResponse, PhoneStatusRequest, PhoneTapRequest, PixelSize,
    PortalCapabilities, RectF, SemanticBackendKind, ServiceRequest, ServiceResponse, SessionKind,
    SessionPresenceAction, SessionPresenceIntent, SessionPresenceStatus, ToolAvailability,
    ToolCapabilities, WindowInfo, WindowTarget,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Debug, Clone)]
struct FakeBackend {
    snapshot: AppStateSnapshot,
    outcome: ActionOutcome,
    presence: Option<Arc<PresenceRecorder>>,
}

#[derive(Debug, Default)]
struct PresenceRecorder {
    ensure_calls: AtomicUsize,
    release_calls: AtomicUsize,
    last_intent: std::sync::Mutex<Option<SessionPresenceIntent>>,
    last_relock: std::sync::Mutex<Option<bool>>,
}

#[derive(Debug, Clone)]
struct BlockingBackend {
    snapshot: AppStateSnapshot,
    outcome: ActionOutcome,
    execute_calls: Arc<AtomicUsize>,
    first_execute_started: Arc<Notify>,
    second_execute_started: Arc<Notify>,
    release_first_execute: Arc<Notify>,
}

#[async_trait::async_trait]
impl DesktopBackend for FakeBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        Ok(self.snapshot.environment.clone())
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(Vec::new())
    }

    async fn get_app_state(
        &self,
        _selector: Option<AppSelector>,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        Ok(self.snapshot.clone())
    }

    async fn execute_action(&self, _request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        Ok(self.outcome.clone())
    }

    async fn ensure_session_presence(
        &self,
        intent: SessionPresenceIntent,
    ) -> Result<SessionPresenceStatus, BackendError> {
        if let Some(presence) = &self.presence {
            presence.record_ensure(intent);
        }
        Ok(SessionPresenceStatus {
            backend: "fake".to_string(),
            supported: true,
            unlock_supported: true,
            locked: Some(false),
            lock_inhibited: intent.inhibit_lock,
            suspend_inhibited: intent.inhibit_suspend,
            detail: "fake session presence ensured".to_string(),
        })
    }

    async fn release_session_presence(
        &self,
        relock: bool,
    ) -> Result<SessionPresenceStatus, BackendError> {
        if let Some(presence) = &self.presence {
            presence.record_release(relock);
        }
        Ok(SessionPresenceStatus {
            backend: "fake".to_string(),
            supported: true,
            unlock_supported: true,
            locked: Some(relock),
            lock_inhibited: false,
            suspend_inhibited: false,
            detail: "fake session presence released".to_string(),
        })
    }
}

#[async_trait::async_trait]
impl DesktopBackend for BlockingBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        Ok(self.snapshot.environment.clone())
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(Vec::new())
    }

    async fn get_app_state(
        &self,
        _selector: Option<AppSelector>,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        Ok(self.snapshot.clone())
    }

    async fn execute_action(&self, _request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let call = self.execute_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 1 {
            self.first_execute_started.notify_one();
            self.release_first_execute.notified().await;
        } else if call == 2 {
            self.second_execute_started.notify_one();
        }
        Ok(self.outcome.clone())
    }
}

/// A backend whose `probe_environment` never resolves, standing in for
/// an unbounded AT-SPI zbus call or a PipeWire teardown deadlock. Used
/// to prove `with_desktop_deadline` bounds the desktop request lane
/// (plan 017): a request against this backend must be abandoned at the
/// deadline rather than hanging forever, and the shared lane mutex must
/// be free again for the very next request on the same daemon.
#[derive(Debug, Clone, Default)]
struct HangingBackend {
    reset_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl DesktopBackend for HangingBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        std::future::pending::<()>().await;
        unreachable!("probe_environment must never resolve in the deadline test")
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(Vec::new())
    }

    async fn get_app_state(
        &self,
        _selector: Option<AppSelector>,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        std::future::pending::<()>().await;
        unreachable!("get_app_state must never resolve in the deadline test")
    }

    async fn execute_action(&self, _request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::Internal,
            "HangingBackend does not support execute_action",
        ))
    }

    async fn reset_desktop_session_state(&self) {
        self.reset_calls.fetch_add(1, Ordering::SeqCst);
    }
}

fn request(action: ActionName, arguments: serde_json::Value) -> ActionRequest {
    ActionRequest {
        action,
        snapshot_id: None,
        element_index: None,
        arguments,
        resolved_element: None,
        resolved_target_element: None,
        resolved_capture: None,
        resolved_focused_app: None,
        environment: None,
    }
}

#[test]
fn only_activity_requests_trigger_automatic_session_presence() {
    assert!(!request_should_hold_presence(&ServiceRequest::Health));
    assert!(!request_should_hold_presence(&ServiceRequest::Doctor));
    assert!(!request_should_hold_presence(
        &ServiceRequest::AgentCursorStatus
    ));
    assert!(!request_should_hold_presence(&ServiceRequest::Browser {
        request: BrowserRequest::Status,
    }));
    assert!(request_should_hold_presence(&ServiceRequest::Browser {
        request: BrowserRequest::Click {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "tab".to_string(),
            x: 1.0,
            y: 2.0,
        },
    }));
    assert!(request_should_hold_presence(&ServiceRequest::Screenshot {
        target: None,
        display_target: None,
    }));
    assert!(request_should_hold_presence(
        &ServiceRequest::ExecuteAction {
            request: Box::new(request(ActionName::Click, json!({"x": 1.0, "y": 2.0}),)),
        },
    ));
    // Read-only phone perception does not hold presence; a device-mutating
    // phone write (tap) does, like a desktop write.
    assert!(!request_should_hold_presence(&ServiceRequest::Phone {
        request: PhoneRequest::Status(PhoneStatusRequest::default()),
    }));
    assert!(!request_should_hold_presence(&ServiceRequest::Phone {
        request: PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
    }));
    assert!(request_should_hold_presence(&ServiceRequest::Phone {
        request: PhoneRequest::Tap(PhoneTapRequest {
            session: Default::default(),
            phone_snapshot_id: None,
            x: 1.0,
            y: 2.0,
            use_device_coordinates: true,
        }),
    }));
}

#[tokio::test]
async fn phone_requests_route_through_manager_to_matching_response_variants() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());

    // status -> Status, never a fabricated session.
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::Status(PhoneStatusRequest::default()),
        })
        .await
    {
        ServiceResponse::Phone {
            response: PhoneResponse::Status(report),
        } => {
            assert!(report.sessions.is_empty());
            assert!(!report.adb_available);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    // list_devices -> Devices with an honest diagnostic.
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
        })
        .await
    {
        ServiceResponse::Phone {
            response: PhoneResponse::Devices(response),
        } => {
            assert!(response.devices.is_empty());
            assert!(!response.diagnostics.is_empty());
        }
        other => panic!("unexpected response: {other:?}"),
    }

    // connect -> Status (no live device in Phase 1; never a Connected session).
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::Connect(PhoneConnectRequest::default()),
        })
        .await
    {
        ServiceResponse::Phone {
            response: PhoneResponse::Status(report),
        } => assert!(report.sessions.is_empty()),
        other => panic!("connect must not fabricate a session: {other:?}"),
    }

    // tap with no active session -> Action with no backend and a structured
    // no-session diagnostic (connect must run first).
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::Tap(PhoneTapRequest {
                session: Default::default(),
                phone_snapshot_id: Some("snap".to_string()),
                x: 5.0,
                y: 6.0,
                use_device_coordinates: false,
            }),
        })
        .await
    {
        ServiceResponse::Phone {
            response: PhoneResponse::Action(response),
        } => {
            assert_eq!(response.action, "phone_tap");
            assert_eq!(
                response.backend,
                sky_cua_platform::model::PhoneBackendKind::None
            );
            assert!(
                response
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "PhoneNoSession")
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    // app_list -> App with kind List.
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::AppList(PhoneAppListRequest::default()),
        })
        .await
    {
        ServiceResponse::Phone {
            response: PhoneResponse::App(response),
        } => assert_eq!(response.kind, PhoneAppResponseKind::List),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn screenshot_rejects_mixed_selectors_at_service_boundary() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());

    match daemon
        .handle(ServiceRequest::Screenshot {
            target: Some(WindowTarget {
                window_id: Some("w1".to_string()),
                ..Default::default()
            }),
            display_target: Some(DisplayTarget {
                display_id: Some("kwin:HDMI-A-1".to_string()),
                display_name: None,
                display_index: None,
            }),
        })
        .await
    {
        ServiceResponse::Error { code, message } => {
            assert_eq!(code, "InvalidRequest");
            assert!(message.contains("exactly one capture selector"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn snapshotless_physical_actions_do_not_require_cached_snapshot_context() {
    assert!(!action_requires_snapshot_context(&request(
        ActionName::Click,
        json!({"x": 10.0, "y": 20.0}),
    )));
    assert!(!action_requires_snapshot_context(&request(
        ActionName::TypeText,
        json!({"text": "hello"}),
    )));
    assert!(!action_requires_snapshot_context(&request(
        ActionName::PressKey,
        json!({"key": "Enter"}),
    )));
}

#[test]
fn element_and_semantic_actions_require_cached_snapshot_context() {
    let mut click = request(ActionName::Click, json!({}));
    click.element_index = Some(3);
    assert!(action_requires_snapshot_context(&click));

    assert!(action_requires_snapshot_context(&request(
        ActionName::Drag,
        json!({"to_element_index": 4}),
    )));
    assert!(action_requires_snapshot_context(&request(
        ActionName::SetValue,
        json!({"value": "hello"}),
    )));
    assert!(action_requires_snapshot_context(&request(
        ActionName::ActivateElement,
        json!({}),
    )));
}

#[test]
fn direct_backend_ref_only_bypasses_action_target_resolution() {
    assert!(!action_requires_snapshot_context(&request(
        ActionName::PerformAction,
        json!({"element_identifier": ":1.2:/node/3", "action_name": "press"}),
    )));
    assert!(action_requires_snapshot_context(&request(
        ActionName::Drag,
        json!({"element_identifier": ":1.2:/node/3", "to_element_index": 4}),
    )));
}

#[tokio::test]
async fn cursor_status_requests_round_trip_through_daemon_handle() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    let state = AgentCursorState {
        visible: true,
        sequence: 99,
        model_point: Some(AgentCursorPoint {
            x: 12.0,
            y: 34.0,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: Some("stream".to_string()),
        }),
        native_point: None,
        snapshot_id: Some("snap".to_string()),
        source_action: Some(ActionName::Click),
        updated_at_ms: 0,
    };

    match daemon
        .handle(ServiceRequest::SetAgentCursor { state })
        .await
    {
        ServiceResponse::SetAgentCursor {
            state: Some(state),
            diagnostics,
            ..
        } => {
            assert_eq!(state.sequence, 1);
            assert!(diagnostics.is_empty());
        }
        other => panic!("unexpected response: {other:?}"),
    }

    match daemon.handle(ServiceRequest::AgentCursorStatus).await {
        ServiceResponse::AgentCursorStatus {
            capabilities,
            state: Some(state),
            diagnostics,
        } => {
            assert!(capabilities.screenshot_synthetic_cursor);
            assert_eq!(state.sequence, 1);
            assert!(diagnostics.is_empty());
        }
        other => panic!("unexpected response: {other:?}"),
    }

    match daemon
        .handle(ServiceRequest::HideAgentCursor {
            reason: Some("capture".to_string()),
        })
        .await
    {
        ServiceResponse::HideAgentCursor {
            state: Some(state),
            diagnostics,
            ..
        } => {
            assert!(!state.visible);
            assert!(
                diagnostics
                    .iter()
                    .any(|entry| entry.code == "AgentCursorHidden")
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    match daemon.handle(ServiceRequest::ShowAgentCursor).await {
        ServiceResponse::ShowAgentCursor {
            state: Some(state), ..
        } => assert!(state.visible),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn last_client_cleanup_hides_agent_cursor_state() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    let state = AgentCursorState {
        visible: true,
        sequence: 99,
        model_point: Some(AgentCursorPoint {
            x: 12.0,
            y: 34.0,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: Some("stream".to_string()),
        }),
        native_point: None,
        snapshot_id: Some("snap".to_string()),
        source_action: Some(ActionName::Click),
        updated_at_ms: 0,
    };

    let _ = daemon
        .handle(ServiceRequest::SetAgentCursor { state })
        .await;
    daemon.hide_agent_cursor_after_last_client().await;

    match daemon.handle(ServiceRequest::AgentCursorStatus).await {
        ServiceResponse::AgentCursorStatus {
            state: Some(state), ..
        } => assert!(!state.visible),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn get_app_state_attaches_cursor_state_and_synthetic_screenshot() {
    let dir = unique_temp_dir("daemon-get-state");
    let source = dir.join("capture.png");
    ImageBuffer::from_pixel(96, 96, Rgba([240u8, 240, 240, 255]))
        .save(&source)
        .expect("write source image");
    let daemon = daemon_with(
        snapshot(Some(capture_with_path(&source)), Vec::new()),
        success_outcome(),
    );
    let state = AgentCursorState {
        visible: true,
        sequence: 0,
        model_point: Some(AgentCursorPoint {
            x: 48.0,
            y: 48.0,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: Some("stream".to_string()),
        }),
        native_point: None,
        snapshot_id: Some("snap".to_string()),
        source_action: Some(ActionName::Click),
        updated_at_ms: 0,
    };
    let _ = daemon
        .handle(ServiceRequest::SetAgentCursor { state })
        .await;

    match daemon
        .handle(ServiceRequest::GetAppState {
            selector: None,
            capture_screen: CaptureScreenMode::Always,
        })
        .await
    {
        ServiceResponse::GetAppState { snapshot } => {
            assert!(snapshot.agent_cursor.is_some());
            let capture = snapshot.capture.expect("capture should remain present");
            let output = capture.screenshot_path.expect("synthetic screenshot path");
            assert!(output.ends_with("capture.agent-cursor.png"));
            let rendered = image::open(&output).expect("open output").to_rgba8();
            assert!(
                rendered
                    .pixels()
                    .any(|pixel| pixel != &Rgba([240u8, 240, 240, 255]))
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn execute_action_updates_cursor_state_for_explicit_click() {
    let daemon = daemon_with(
        snapshot(Some(capture_with_rect()), Vec::new()),
        success_outcome(),
    );
    let _ = daemon
        .handle(ServiceRequest::GetAppState {
            selector: None,
            capture_screen: CaptureScreenMode::Always,
        })
        .await;

    let mut click = request(ActionName::Click, json!({"x": 42.0, "y": 24.0}));
    click.snapshot_id = Some("snap".to_string());

    match daemon
        .handle(ServiceRequest::ExecuteAction {
            request: Box::new(click),
        })
        .await
    {
        ServiceResponse::ExecuteAction { outcome } => {
            let state = outcome.agent_cursor.expect("outcome cursor state");
            let point = state.model_point.expect("model point");
            assert_eq!(point.x, 42.0);
            assert_eq!(point.y, 24.0);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    match daemon.handle(ServiceRequest::AgentCursorStatus).await {
        ServiceResponse::AgentCursorStatus {
            state: Some(state), ..
        } => assert_eq!(state.sequence, 1),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn service_runtime_health_bypasses_blocked_desktop_request() {
    let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
    let first_started = backend.first_execute_started.clone();
    let release_first = backend.release_first_execute.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));

    let action_daemon = daemon.clone();
    let action_task = tokio::spawn(async move {
        let action = request(ActionName::Click, json!({"x": 42.0, "y": 24.0}));
        action_daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(action),
            })
            .await
    });

    first_started.notified().await;
    let health = tokio::time::timeout(Duration::from_millis(100), async {
        daemon.handle(ServiceRequest::Health).await
    })
    .await;
    assert!(
        health.is_ok(),
        "health should bypass the blocked desktop lane"
    );

    release_first.notify_one();
    match action_task.await.expect("action task") {
        ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn service_runtime_browser_open_bypasses_blocked_desktop_request() {
    let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
    let first_started = backend.first_execute_started.clone();
    let release_first = backend.release_first_execute.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));

    let action_daemon = daemon.clone();
    let action_task = tokio::spawn(async move {
        let action = request(ActionName::Click, json!({"x": 42.0, "y": 24.0}));
        action_daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(action),
            })
            .await
    });

    first_started.notified().await;
    let browser_open = tokio::time::timeout(Duration::from_millis(100), async {
        daemon
            .handle(ServiceRequest::Browser {
                request: BrowserRequest::Open {
                    target: Some(BrowserTargetKind::UserChrome),
                    url: Some("file:///etc/passwd".to_string()),
                },
            })
            .await
    })
    .await
    .expect("browser_open should bypass the blocked desktop lane");
    match browser_open {
        ServiceResponse::Browser {
            response: BrowserResponse::Open { response },
        } => {
            assert!(response.tab.is_none());
            assert_eq!(response.diagnostics.len(), 1);
            let expected_code = if cfg!(target_os = "windows") {
                "BrowserBridgeUnsupported"
            } else {
                "BrowserOpenUrlUnsupported"
            };
            assert_eq!(response.diagnostics[0].code, expected_code);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    release_first.notify_one();
    match action_task.await.expect("action task") {
        ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn service_runtime_browser_status_bypasses_blocked_desktop_request() {
    let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
    let first_started = backend.first_execute_started.clone();
    let release_first = backend.release_first_execute.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));

    let action_daemon = daemon.clone();
    let action_task = tokio::spawn(async move {
        let action = request(ActionName::Click, json!({"x": 42.0, "y": 24.0}));
        action_daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(action),
            })
            .await
    });

    first_started.notified().await;
    let browser_status = tokio::time::timeout(Duration::from_millis(500), async {
        daemon
            .handle(ServiceRequest::Browser {
                request: BrowserRequest::Status,
            })
            .await
    })
    .await
    .expect("browser_status should bypass the blocked desktop lane");
    match browser_status {
        ServiceResponse::Browser {
            response: BrowserResponse::Status { report },
        } => {
            assert_eq!(report.browser_integration, None);
            let expected_code = if cfg!(target_os = "windows") {
                "BrowserBridgeUnsupported"
            } else {
                "BrowserIntegrationDeferred"
            };
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == expected_code)
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    release_first.notify_one();
    match action_task.await.expect("action task") {
        ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn browser_snapshot_rejects_oversized_text_limit_at_service_boundary() {
    let daemon = daemon_with(
        snapshot(Some(capture_with_rect()), Vec::new()),
        success_outcome(),
    );

    match daemon
        .handle(ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "tab-1".to_string(),
                text_limit: Some(BROWSER_SNAPSHOT_MAX_TEXT_LIMIT + 1),
                element_offset: None,
                element_limit: None,
                element_query: None,
            },
        })
        .await
    {
        ServiceResponse::Error { code, message } => {
            assert_eq!(code, "InvalidRequest");
            assert!(message.contains(&BROWSER_SNAPSHOT_MAX_TEXT_LIMIT.to_string()));
        }
        other => panic!("expected invalid request response, got: {other:?}"),
    }
}

#[tokio::test]
async fn browser_snapshot_rejects_oversized_element_limit_at_service_boundary() {
    let daemon = daemon_with(
        snapshot(Some(capture_with_rect()), Vec::new()),
        success_outcome(),
    );

    match daemon
        .handle(ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "tab-1".to_string(),
                text_limit: None,
                element_offset: None,
                element_limit: Some(BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT + 1),
                element_query: None,
            },
        })
        .await
    {
        ServiceResponse::Error { code, message } => {
            assert_eq!(code, "InvalidRequest");
            assert!(message.contains(&BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT.to_string()));
        }
        other => panic!("expected invalid request response, got: {other:?}"),
    }
}

#[tokio::test]
async fn service_runtime_serializes_desktop_lane_requests() {
    let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
    let first_started = backend.first_execute_started.clone();
    let second_started = backend.second_execute_started.clone();
    let release_first = backend.release_first_execute.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));

    let first_daemon = daemon.clone();
    let first_task = tokio::spawn(async move {
        let action = request(ActionName::Click, json!({"x": 1.0, "y": 2.0}));
        first_daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(action),
            })
            .await
    });
    first_started.notified().await;

    let second_daemon = daemon.clone();
    let second_task = tokio::spawn(async move {
        let action = request(ActionName::Click, json!({"x": 3.0, "y": 4.0}));
        second_daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(action),
            })
            .await
    });

    assert!(
        tokio::time::timeout(Duration::from_millis(100), second_started.notified())
            .await
            .is_err(),
        "second desktop request should wait for the first desktop request"
    );
    release_first.notify_one();
    tokio::time::timeout(Duration::from_secs(1), second_started.notified())
        .await
        .expect("second request should enter after first is released");

    match first_task.await.expect("first task") {
        ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
        other => panic!("unexpected response: {other:?}"),
    }
    match second_task.await.expect("second task") {
        ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
        other => panic!("unexpected response: {other:?}"),
    }
}

/// Core invariant for plan 017: a desktop request whose backend call
/// never resolves (standing in for the observed unbounded AT-SPI zbus
/// call / PipeWire teardown deadlock) must be abandoned at the deadline
/// with a structured error rather than hanging forever, AND the shared
/// `desktop_lane` mutex it was holding must be free again immediately
/// afterward — a second request on the SAME daemon must not itself wait
/// behind the (now-dropped) hung future. Uses a short env-overridden
/// deadline so the test itself stays fast; this test runs in its own
/// nextest process, so setting the env var before the first
/// `desktop_request_deadline()` call (which caches it) is race-free.
#[tokio::test]
async fn desktop_lane_deadline_frees_the_lane() {
    unsafe { std::env::set_var("SKY_CUA_DESKTOP_REQUEST_DEADLINE_MS", "50") };
    let backend = HangingBackend::default();
    let reset_calls = backend.reset_calls.clone();
    let daemon = daemon_with_backend(Box::new(backend));

    let started = std::time::Instant::now();
    let first = daemon.handle(ServiceRequest::Doctor).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the deadline should fire promptly instead of hanging, took {:?}",
        started.elapsed()
    );
    match first {
        ServiceResponse::Error { code, .. } => {
            assert_eq!(
                code,
                BackendErrorCode::DesktopRequestDeadlineExceeded.as_str()
            );
        }
        other => panic!("expected a desktop_request_deadline_exceeded error: {other:?}"),
    }
    assert_eq!(
        reset_calls.load(Ordering::SeqCst),
        1,
        "an elapsed deadline must reset backend session state exactly once"
    );

    // THE core invariant: desktop_lane must be free again. A second
    // request on the same daemon must return promptly, not wait behind
    // the first (dropped, not merely finished) hung future.
    let started = std::time::Instant::now();
    let second = daemon.handle(ServiceRequest::Doctor).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the lane must be free after the first deadline fired, took {:?}",
        started.elapsed()
    );
    match second {
        ServiceResponse::Error { code, .. } => {
            assert_eq!(
                code,
                BackendErrorCode::DesktopRequestDeadlineExceeded.as_str()
            );
        }
        other => panic!("expected the second request to also time out cleanly: {other:?}"),
    }
}

#[tokio::test]
async fn automatic_session_presence_acquires_once_and_releases_after_idle() {
    let presence = Arc::new(PresenceRecorder::default());
    let daemon = daemon_with_backend_and_presence_config(
        Box::new(FakeBackend {
            snapshot: snapshot(None, Vec::new()),
            outcome: success_outcome(),
            presence: Some(presence.clone()),
        }),
        SessionPresenceConfig {
            enabled: true,
            idle_release: Duration::from_millis(5),
            unlock: true,
            relock: true,
            inhibit_lock: true,
            inhibit_suspend: true,
        },
    );

    for _ in 0..2 {
        let action = request(ActionName::Click, json!({"x": 1.0, "y": 2.0}));
        match daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(action),
            })
            .await
        {
            ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
            other => panic!("unexpected response: {other:?}"),
        }
    }
    assert_eq!(presence.ensure_calls(), 1);
    assert_eq!(presence.release_calls(), 0);
    assert_eq!(
        presence.last_intent(),
        Some(SessionPresenceIntent {
            unlock: true,
            inhibit_lock: true,
            inhibit_suspend: true,
        })
    );

    tokio::time::sleep(Duration::from_millis(8)).await;
    daemon.release_idle_session_presence_if_needed().await;
    daemon.release_idle_session_presence_if_needed().await;

    assert_eq!(presence.ensure_calls(), 1);
    assert_eq!(presence.release_calls(), 1);
    assert_eq!(presence.last_relock(), Some(true));

    let action = request(ActionName::Click, json!({"x": 3.0, "y": 4.0}));
    let _ = daemon
        .handle(ServiceRequest::ExecuteAction {
            request: Box::new(action),
        })
        .await;
    assert_eq!(presence.ensure_calls(), 2);
}

#[tokio::test]
async fn explicit_session_presence_requests_are_rejected_when_disabled() {
    let presence = Arc::new(PresenceRecorder::default());
    let daemon = daemon_with_backend_and_presence_config(
        Box::new(FakeBackend {
            snapshot: snapshot(None, Vec::new()),
            outcome: success_outcome(),
            presence: Some(presence.clone()),
        }),
        SessionPresenceConfig::disabled(),
    );

    for action in [
        SessionPresenceAction::Ensure(SessionPresenceIntent {
            unlock: true,
            inhibit_lock: true,
            inhibit_suspend: true,
        }),
        SessionPresenceAction::Release { relock: false },
    ] {
        match daemon
            .handle(ServiceRequest::SessionPresence { action })
            .await
        {
            ServiceResponse::Error { code, .. } => {
                assert_eq!(code, "ActionUnsupportedForEnvironment");
            }
            other => panic!("expected a disabled-gate error, got: {other:?}"),
        }
    }
    assert_eq!(presence.ensure_calls(), 0);
    assert_eq!(presence.release_calls(), 0);
    assert!(!*daemon.session_presence_held.lock().await);

    // Status stays available and reports honestly while disabled.
    match daemon
        .handle(ServiceRequest::SessionPresence {
            action: SessionPresenceAction::Status,
        })
        .await
    {
        ServiceResponse::SessionPresence { .. } => {}
        other => panic!("status should not be gated: {other:?}"),
    }
}

#[test]
fn if_changed_reuses_previous_identical_model_capture_path() {
    let dir = unique_temp_dir("if-changed");
    let previous_path = dir.join("previous.jpg");
    let current_path = dir.join("current.jpg");
    std::fs::write(&previous_path, b"same model image").expect("write previous");
    std::fs::write(&current_path, b"same model image").expect("write current");

    let previous = snapshot(Some(capture_with_path(&previous_path)), Vec::new());
    let mut current = snapshot(Some(capture_with_path(&current_path)), Vec::new());

    assert!(reuse_unchanged_capture(&mut current, Some(&previous)));
    assert_eq!(
        current
            .capture
            .expect("capture")
            .screenshot_path
            .expect("path"),
        previous_path.display().to_string()
    );
}

#[test]
fn if_changed_reuses_previous_cursor_capture_when_raw_sibling_matches() {
    let dir = unique_temp_dir("if-changed-agent-cursor");
    let raw_previous_path = dir.join("capture.png");
    let previous_path = dir.join("capture.agent-cursor.png");
    let current_path = dir.join("current.png");
    std::fs::write(&raw_previous_path, b"same raw model image").expect("write raw previous");
    std::fs::write(&previous_path, b"same raw model image plus cursor").expect("write previous");
    std::fs::write(&current_path, b"same raw model image").expect("write current");

    let previous = snapshot(Some(capture_with_path(&previous_path)), Vec::new());
    let mut current = snapshot(Some(capture_with_path(&current_path)), Vec::new());

    assert!(reuse_unchanged_capture(&mut current, Some(&previous)));
    assert_eq!(
        current
            .capture
            .expect("capture")
            .screenshot_path
            .expect("path"),
        previous_path.display().to_string()
    );
}

#[test]
fn if_changed_reuse_keeps_current_original_screenshot_path() {
    let dir = unique_temp_dir("if-changed-original-path");
    let previous_path = dir.join("previous.jpg");
    let previous_original_path = dir.join("previous-original.jpg");
    let current_path = dir.join("current.jpg");
    let current_original_path = dir.join("current-original.jpg");
    std::fs::write(&previous_path, b"same model image").expect("write previous");
    std::fs::write(&current_path, b"same model image").expect("write current");
    let mut previous_capture = capture_with_path(&previous_path);
    previous_capture.original_screenshot_path = Some(previous_original_path.display().to_string());
    let mut current_capture = capture_with_path(&current_path);
    current_capture.original_screenshot_path = Some(current_original_path.display().to_string());
    let previous = snapshot(Some(previous_capture), Vec::new());
    let mut current = snapshot(Some(current_capture), Vec::new());

    assert!(reuse_unchanged_capture(&mut current, Some(&previous)));

    let capture = current.capture.expect("capture");
    assert_eq!(
        capture.screenshot_path.as_deref(),
        Some(previous_path.to_str().expect("utf-8 previous path"))
    );
    assert_eq!(
        capture.original_screenshot_path.as_deref(),
        Some(current_original_path.to_str().expect("utf-8 current path"))
    );
}

#[test]
fn if_changed_keeps_current_capture_when_image_changed() {
    let dir = unique_temp_dir("if-changed-different");
    let previous_path = dir.join("previous.jpg");
    let current_path = dir.join("current.jpg");
    std::fs::write(&previous_path, b"old model image").expect("write previous");
    std::fs::write(&current_path, b"new model image").expect("write current");

    let previous = snapshot(Some(capture_with_path(&previous_path)), Vec::new());
    let mut current = snapshot(Some(capture_with_path(&current_path)), Vec::new());

    assert!(!reuse_unchanged_capture(&mut current, Some(&previous)));
    assert_eq!(
        current
            .capture
            .expect("capture")
            .screenshot_path
            .expect("path"),
        current_path.display().to_string()
    );
}

fn window_info(window_id: &str, title: Option<&str>, pid: Option<u32>) -> WindowInfo {
    WindowInfo {
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

#[test]
fn select_scrcpy_window_prefers_pid_over_title() {
    let windows = vec![
        window_info("w-other", Some("sky-cua-phone-dev1"), Some(10)),
        window_info("w-target", Some("some-other-title"), Some(42)),
    ];
    // pid 42 wins even though a different window carries the matching title.
    let picked = super::phone::select_scrcpy_window(&windows, Some(42), "sky-cua-phone-dev1")
        .expect("pid match");
    assert_eq!(picked.window_id, "w-target");
}

#[test]
fn select_scrcpy_window_falls_back_to_title() {
    let windows = vec![
        window_info("w-a", Some("unrelated"), Some(1)),
        window_info("w-b", Some("sky-cua-phone-dev1"), Some(2)),
    ];
    // No pid (or a pid that matches nothing) -> exact title match.
    let by_no_pid = super::phone::select_scrcpy_window(&windows, None, "sky-cua-phone-dev1")
        .expect("title match");
    assert_eq!(by_no_pid.window_id, "w-b");
    let by_missing_pid =
        super::phone::select_scrcpy_window(&windows, Some(999), "sky-cua-phone-dev1")
            .expect("title fallback when pid matches nothing");
    assert_eq!(by_missing_pid.window_id, "w-b");
}

#[test]
fn select_scrcpy_window_returns_none_without_a_match() {
    let windows = vec![window_info("w-a", Some("unrelated"), Some(1))];
    assert!(super::phone::select_scrcpy_window(&windows, Some(42), "sky-cua-phone-dev1").is_none());
    assert!(super::phone::select_scrcpy_window(&[], Some(42), "sky-cua-phone-dev1").is_none());
}

fn daemon_with(snapshot: AppStateSnapshot, outcome: ActionOutcome) -> ServiceDaemon {
    daemon_with_backend(Box::new(FakeBackend {
        snapshot,
        outcome,
        presence: None,
    }))
}

fn daemon_with_backend(backend: Box<dyn DesktopBackend>) -> ServiceDaemon {
    daemon_with_backend_and_presence_config(backend, SessionPresenceConfig::disabled())
}

fn daemon_with_backend_and_presence_config(
    backend: Box<dyn DesktopBackend>,
    session_presence_config: SessionPresenceConfig,
) -> ServiceDaemon {
    daemon_with_phone(backend, session_presence_config, test_phone_manager())
}

fn daemon_with_phone(
    backend: Box<dyn DesktopBackend>,
    session_presence_config: SessionPresenceConfig,
    phone: crate::phone::PhoneManager,
) -> ServiceDaemon {
    ServiceDaemon {
        backend,
        sessions: SessionStore::new(),
        snapshots: tokio::sync::Mutex::new(SnapshotManager::new(8)),
        overlay: tokio::sync::Mutex::new(OverlayController::new_for_tests()),
        phone: tokio::sync::Mutex::new(phone),
        session_presence_config,
        session_presence_held: tokio::sync::Mutex::new(false),
        desktop_lane: tokio::sync::Mutex::new(()),
        browser_eval_enabled: false,
        socket_path: PathBuf::from("/tmp/sky-cua-test.sock"),
    }
}

/// A `PhoneManager` backed by a deterministic, unscripted `FakeCommandRunner`,
/// so the daemon's phone routing tests never shell out to a real `adb` that
/// may or may not exist on the test host.
fn test_phone_manager() -> crate::phone::PhoneManager {
    crate::phone::PhoneManager::with_fake_runner_for_tests()
}

impl BlockingBackend {
    fn new(snapshot: AppStateSnapshot) -> Self {
        Self {
            snapshot,
            outcome: success_outcome(),
            execute_calls: Arc::new(AtomicUsize::new(0)),
            first_execute_started: Arc::new(Notify::new()),
            second_execute_started: Arc::new(Notify::new()),
            release_first_execute: Arc::new(Notify::new()),
        }
    }
}

impl PresenceRecorder {
    fn record_ensure(&self, intent: SessionPresenceIntent) {
        self.ensure_calls.fetch_add(1, Ordering::SeqCst);
        *self.last_intent.lock().expect("last intent lock") = Some(intent);
    }

    fn record_release(&self, relock: bool) {
        self.release_calls.fetch_add(1, Ordering::SeqCst);
        *self.last_relock.lock().expect("last relock lock") = Some(relock);
    }

    fn ensure_calls(&self) -> usize {
        self.ensure_calls.load(Ordering::SeqCst)
    }

    fn release_calls(&self) -> usize {
        self.release_calls.load(Ordering::SeqCst)
    }

    fn last_intent(&self) -> Option<SessionPresenceIntent> {
        *self.last_intent.lock().expect("last intent lock")
    }

    fn last_relock(&self) -> Option<bool> {
        *self.last_relock.lock().expect("last relock lock")
    }
}

fn success_outcome() -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: "ok".to_string(),
        code: "Ok".to_string(),
        diagnostics: Vec::new(),
        agent_cursor: None,
    }
}

fn snapshot(capture: Option<CaptureInfo>, elements: Vec<ElementNode>) -> AppStateSnapshot {
    AppStateSnapshot {
        snapshot_id: "snap".to_string(),
        created_at: chrono::Utc::now(),
        environment: environment(),
        capabilities: available_capabilities(),
        focused_app: None,
        capture,
        elements,
        diagnostics: Vec::new(),
        app_guidance: None,
        doctor_report: None,
        agent_cursor: None,
    }
}

fn environment() -> EnvironmentInfo {
    EnvironmentInfo {
        session_kind: SessionKind::Wayland,
        compositor: Some("KWin".to_string()),
        desktop_environment: Some("KDE".to_string()),
        capture_backend: CaptureBackendKind::PortalPipeWire,
        input_backend: InputBackendKind::PortalRemoteDesktop,
        semantic_backend: SemanticBackendKind::Atspi,
        portal_capabilities: PortalCapabilities {
            screencast_version: Some(5),
            remote_desktop_version: Some(2),
            screenshot_version: Some(1),
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        },
        xdg_session_type: Some("wayland".to_string()),
        display: None,
        wayland_display: Some("wayland-0".to_string()),
        displays: Vec::new(),
    }
}

fn capture_with_rect() -> CaptureInfo {
    CaptureInfo {
        backend: CaptureBackendKind::PortalPipeWire,
        image_backend: Some(CaptureBackendKind::PortalPipeWire),
        capture_scope: CaptureScope::Unknown,
        display: None,
        coordinate_space: Some(CoordinateSpace::StreamPixels),
        stream_id: Some("stream".to_string()),
        source_type: Some(1),
        mapping_id: Some("mapping".to_string()),
        source_logical_rect: None,
        logical_rect: Some(RectF {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 200.0,
            space: CoordinateSpace::DesktopLogical,
        }),
        pixel_size: Some(PixelSize {
            width: 400,
            height: 200,
        }),
        original_pixel_size: None,
        logical_to_pixel_scale: None,
        screenshot_path: None,
        original_screenshot_path: None,
        model_image_format: Some(ModelImageFormat::Jpeg),
        model_image_quality: Some(85),
        model_image_bytes: None,
        model_image_encode_ms: None,
    }
}

fn capture_with_path(path: &Path) -> CaptureInfo {
    let mut capture = capture_with_rect();
    capture.screenshot_path = Some(path.display().to_string());
    capture.model_image_format = None;
    capture
}

fn available_capabilities() -> ToolCapabilities {
    let available = || ToolAvailability {
        available: true,
        reason: None,
    };
    ToolCapabilities {
        list_apps: available(),
        get_app_state: available(),
        focus_element: available(),
        activate_element: available(),
        select_element: available(),
        expand_element: available(),
        collapse_element: available(),
        toggle_element: available(),
        click: available(),
        perform_action: available(),
        perform_secondary_action: available(),
        scroll: available(),
        supported_scroll_directions: vec![
            sky_cua_platform::model::ScrollDirection::Up,
            sky_cua_platform::model::ScrollDirection::Down,
        ],
        drag: available(),
        type_text: available(),
        press_key: available(),
        set_value: available(),
    }
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sky-cua-daemon-agent-cursor-{name}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}
