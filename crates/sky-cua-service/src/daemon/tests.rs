use super::capture_reuse::reuse_unchanged_capture;
use super::desktop::action_requires_snapshot_context;
use super::session_presence::request_should_hold_presence;
use super::{
    CuaScreenshotCoordinatePlane, OverlayController, ServiceDaemon, SessionPresenceConfig,
    SessionStore, SnapshotManager,
};
use image::{ImageBuffer, Rgba};
use serde_json::json;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AgentCursorPoint, AgentCursorState, AppInfo,
    AppSelector, AppShotActionSnapshot, AppShotCapture, AppShotCaptureFlags, AppShotConsistency,
    AppShotCoverage, AppShotEnvelope, AppShotTrigger, AppStateSnapshot,
    BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT, BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserCallerKind,
    BrowserCallerProvenance, BrowserLogicalIdentity, BrowserOperationIdentity,
    BrowserProvenanceSource, BrowserRequest, BrowserRequestContext, BrowserResponse,
    BrowserTargetKind, CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode,
    ContentPersistence, ContentRef, ContentSource, CoordinateSpace, CuaActionRequest,
    CuaBackendResponse, CuaCancellation, CuaRequestContext, DiagnosticEntry, DisplayTarget,
    ElementNode, EnvironmentInfo, FocusedApp, InputBackendKind, ModelImageFormat,
    PhoneAppListRequest, PhoneAppResponseKind, PhoneCallerProvenance, PhoneConnectRequest,
    PhoneListDevicesRequest, PhoneMcpClientInfo, PhoneRequest, PhoneRequestContext, PhoneResponse,
    PhoneStatusRequest, PhoneTapRequest, PixelSize, PortalCapabilities, RectF, SemanticBackendKind,
    ServiceRequest, ServiceResponse, SessionKind, SessionPresenceAction, SessionPresenceIntent,
    SessionPresenceStatus, ToolAvailability, ToolCapabilities, WindowInfo, WindowTarget,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Debug, Clone)]
struct FakeBackend {
    snapshot: AppStateSnapshot,
    outcome: ActionOutcome,
    presence: Option<Arc<PresenceRecorder>>,
    recorded_action: Option<Arc<std::sync::Mutex<Option<ActionRequest>>>>,
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

#[derive(Debug, Clone)]
struct ArrivalCheckingBackend {
    snapshot: AppStateSnapshot,
    arrival_marker: PathBuf,
    dispatched_after_arrival: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
struct CuaBlockingBackend {
    snapshot: AppStateSnapshot,
    started: Arc<Notify>,
}

#[derive(Debug, Clone)]
struct CuaCleanupBackend {
    snapshot: AppStateSnapshot,
    input_down: Arc<Notify>,
    released: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl DesktopBackend for FakeBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        Ok(self.snapshot.environment.clone())
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(Vec::new())
    }

    async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError> {
        Ok(vec![fake_window()])
    }

    async fn resolve_window_target(
        &self,
        target: &WindowTarget,
    ) -> Result<WindowInfo, BackendError> {
        let window = fake_window();
        if target.window_id.as_deref() == Some(window.window_id.as_str()) {
            Ok(window)
        } else {
            Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "unknown fake window",
            ))
        }
    }

    async fn get_app_state(
        &self,
        _selector: Option<AppSelector>,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        Ok(self.snapshot.clone())
    }

    async fn get_app_state_for_window(
        &self,
        _window: &WindowInfo,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        Ok(self.snapshot.clone())
    }

    async fn screenshot(
        &self,
        _target: Option<WindowTarget>,
        _display_target: Option<DisplayTarget>,
    ) -> Result<AppStateSnapshot, BackendError> {
        Ok(self.snapshot.clone())
    }

    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        if let Some(recorded_action) = &self.recorded_action {
            *recorded_action.lock().expect("recorded action lock") = Some(request);
        }
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
impl DesktopBackend for CuaBlockingBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        Ok(self.snapshot.environment.clone())
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(Vec::new())
    }

    async fn focused_window(&self) -> Result<Option<WindowInfo>, BackendError> {
        Ok(Some(fake_window()))
    }

    async fn get_app_state(
        &self,
        _selector: Option<AppSelector>,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        Ok(self.snapshot.clone())
    }

    async fn execute_action(&self, _request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        Ok(success_outcome())
    }

    async fn execute_cua_action(
        &self,
        _request: CuaActionRequest,
        cancellation: CuaCancellation,
    ) -> Result<CuaBackendResponse, BackendError> {
        self.started.notify_one();
        loop {
            if cancellation.is_cancelled() {
                return Err(BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "the CUA turn was cancelled",
                ));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

#[async_trait::async_trait]
impl DesktopBackend for CuaCleanupBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        Ok(self.snapshot.environment.clone())
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(Vec::new())
    }

    async fn focused_window(&self) -> Result<Option<WindowInfo>, BackendError> {
        Ok(Some(fake_window()))
    }

    async fn get_app_state(
        &self,
        _selector: Option<AppSelector>,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        Ok(self.snapshot.clone())
    }

    async fn execute_action(&self, _request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        Ok(success_outcome())
    }

    async fn execute_cua_action(
        &self,
        _request: CuaActionRequest,
        cancellation: CuaCancellation,
    ) -> Result<CuaBackendResponse, BackendError> {
        self.input_down.notify_one();
        while !cancellation.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        self.released.store(true, Ordering::SeqCst);
        Err(BackendError::new(
            BackendErrorCode::CuaActionOutcomeUnknown,
            "input was cancelled after pointer-down; release completed",
        ))
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

    async fn focused_window(&self) -> Result<Option<WindowInfo>, BackendError> {
        Ok(Some(fake_window()))
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

#[async_trait::async_trait]
impl DesktopBackend for ArrivalCheckingBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        Ok(self.snapshot.environment.clone())
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(Vec::new())
    }

    async fn focused_window(&self) -> Result<Option<WindowInfo>, BackendError> {
        Ok(Some(fake_window()))
    }

    async fn get_app_state(
        &self,
        _selector: Option<AppSelector>,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        Ok(self.snapshot.clone())
    }

    async fn execute_action(&self, _request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        self.dispatched_after_arrival
            .store(self.arrival_marker.exists(), Ordering::SeqCst);
        Ok(success_outcome())
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
    probe_calls: Arc<AtomicUsize>,
    probe_started: Arc<Notify>,
}

#[async_trait::async_trait]
impl DesktopBackend for HangingBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        self.probe_calls.fetch_add(1, Ordering::SeqCst);
        self.probe_started.notify_one();
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

#[derive(Debug, Clone)]
enum HealthProbeResponse {
    Environment(EnvironmentInfo),
    Error,
    Pending,
}

#[derive(Debug, Clone)]
struct HealthProbeBackend {
    response: Arc<std::sync::Mutex<HealthProbeResponse>>,
    probe_calls: Arc<AtomicUsize>,
}

impl HealthProbeBackend {
    fn new(response: HealthProbeResponse) -> Self {
        Self {
            response: Arc::new(std::sync::Mutex::new(response)),
            probe_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn set_response(&self, response: HealthProbeResponse) {
        *self
            .response
            .lock()
            .expect("health probe response poisoned") = response;
    }
}

#[async_trait::async_trait]
impl DesktopBackend for HealthProbeBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        self.probe_calls.fetch_add(1, Ordering::SeqCst);
        let response = self
            .response
            .lock()
            .expect("health probe response poisoned")
            .clone();
        match response {
            HealthProbeResponse::Environment(environment) => Ok(environment),
            HealthProbeResponse::Error => Err(BackendError::new(
                BackendErrorCode::AccessibilityUnavailable,
                "scripted health capability refresh failure",
            )),
            HealthProbeResponse::Pending => {
                std::future::pending::<()>().await;
                unreachable!("pending health capability refresh must be deadline-bounded")
            }
        }
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(Vec::new())
    }

    async fn get_app_state(
        &self,
        _selector: Option<AppSelector>,
        _capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::Internal,
            "HealthProbeBackend does not support get_app_state",
        ))
    }

    async fn execute_action(&self, _request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::Internal,
            "HealthProbeBackend does not support execute_action",
        ))
    }
}

fn request(action: ActionName, arguments: serde_json::Value) -> ActionRequest {
    ActionRequest {
        action,
        appshot_id: None,
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

async fn authorize_desktop_appshot(daemon: &ServiceDaemon, session_id: Option<&str>) -> String {
    let appshot_id = format!(
        "test-appshot-{}",
        sky_cua_platform::snapshot::new_snapshot_id()
    );
    let mut snapshot = snapshot(Some(capture_with_rect()), Vec::new());
    snapshot.focused_app = Some(FocusedApp {
        app_id: "fake.app".to_string(),
        name: "Fake app".to_string(),
        pid: Some(42),
        desktop_file_id: Some("fake.app.desktop".to_string()),
        app_user_model_id: None,
        window_handle: Some("fake-window".to_string()),
        toolkit_guess: Some("XWayland".to_string()),
        window_title: Some("Fake window".to_string()),
        display: None,
    });
    let appshot = AppShotEnvelope {
        appshot_id: appshot_id.clone(),
        trigger: AppShotTrigger::Observe,
        captured_at: chrono::Utc::now(),
        consistency: AppShotConsistency::Stable,
        capture: AppShotCapture::Desktop {
            app_id: "fake.app".to_string(),
            window_id: "fake-window".to_string(),
            title: Some("Fake window".to_string()),
            bounds: fake_window().bounds.expect("fake window bounds"),
            semantic_projection: json!({}),
        },
        image: ContentRef {
            content_id: format!("content-{appshot_id}"),
            device_id: None,
            link_epoch: None,
            mime_type: "image/png".to_string(),
            filename: None,
            size_bytes: 0,
            sha256: "00".repeat(32),
            source: ContentSource::Screenshot,
            expires_at_ms: None,
            persistence: ContentPersistence::Temporary,
        },
        action_snapshot: AppShotActionSnapshot {
            snapshot_id: snapshot.snapshot_id.clone(),
            session_id: session_id.map(str::to_string),
            subject_generation: None,
        },
        coverage: AppShotCoverage {
            pixels_complete: true,
            semantics_complete: true,
            secure_regions_redacted: false,
            projection_truncated: false,
            total_semantic_nodes: Some(0),
            projected_semantic_nodes: Some(0),
        },
        capability_profile_id: "desktop:test".to_string(),
        diagnostics: Vec::new(),
    };
    let mut snapshots = daemon.snapshots.lock().await;
    snapshots.store(snapshot);
    snapshots.store_appshot(appshot);
    appshot_id
}

async fn authorized_action_request(
    daemon: &ServiceDaemon,
    action: ActionName,
    arguments: serde_json::Value,
) -> ActionRequest {
    let mut request = request(action, arguments);
    request.appshot_id = Some(authorize_desktop_appshot(daemon, None).await);
    request.snapshot_id = Some("snap".to_string());
    request
}

#[tokio::test]
async fn desktop_observe_registers_appshot_for_the_next_action() {
    let dir = unique_temp_dir("desktop-observe-action-fence");
    let source = dir.join("window.png");
    std::fs::write(&source, b"deterministic-window-image").expect("write source image");
    let mut capture = capture_with_path(&source);
    capture.capture_scope = CaptureScope::Window;
    let daemon = daemon_with(snapshot(Some(capture), Vec::new()), success_outcome());

    let observed = daemon
        .handle(ServiceRequest::AppShotCapture {
            request_id: "desktop-observe-1".to_string(),
            target: Some(WindowTarget {
                window_id: Some("fake-window".to_string()),
                ..Default::default()
            }),
            frontmost: false,
            flags: AppShotCaptureFlags::default(),
        })
        .await;
    let (appshot_id, artifact_path) = match observed {
        ServiceResponse::AppShotCapture { result } => {
            let appshot = result.appshot.expect("canonical AppShot");
            (appshot.appshot_id, result.image.path)
        }
        other => panic!("expected AppShot capture, got {other:?}"),
    };

    let mut action = request(ActionName::Click, json!({"x": 12.0, "y": 18.0}));
    action.appshot_id = Some(appshot_id);
    let response = daemon
        .handle(ServiceRequest::ExecuteAction {
            request: Box::new(action),
        })
        .await;
    match response {
        ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
        other => panic!("registered AppShot should authorize action, got {other:?}"),
    }

    let _ = std::fs::remove_file(artifact_path);
    let _ = std::fs::remove_dir_all(dir);
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
        identity: None,
        context: None,
    }));
    assert!(request_should_hold_presence(&ServiceRequest::Browser {
        identity: None,
        context: None,
        request: BrowserRequest::Click {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "tab".to_string(),
            appshot_id: None,
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
        context: None,
    }));
    assert!(!request_should_hold_presence(&ServiceRequest::Phone {
        request: PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
        context: None,
    }));
    assert!(request_should_hold_presence(&ServiceRequest::Phone {
        request: PhoneRequest::Tap(PhoneTapRequest {
            session: Default::default(),
            phone_snapshot_id: None,
            x: 1.0,
            y: 2.0,
            use_device_coordinates: true,
        }),
        context: None,
    }));
}

#[tokio::test]
async fn activate_window_rejects_invalid_optional_request_context() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    let response = daemon
        .handle(ServiceRequest::ActivateWindow {
            target: WindowTarget {
                window_id: Some("fixture-window".to_string()),
                ..Default::default()
            },
            context: Some(CuaRequestContext {
                session_id: "window-session".to_string(),
                appshot_id: None,
                turn_id: "window-turn".to_string(),
                deadline_ms: Some(0),
            }),
        })
        .await;

    assert!(matches!(
        response,
        ServiceResponse::Error {
            ref code,
            ref session_id,
            ref turn_id,
            ref retry,
            ..
        } if code == "SKY_CUA_INVALID_CONTEXT"
            && session_id.as_deref() == Some("window-session")
            && turn_id.as_deref() == Some("window-turn")
            && retry.as_deref() == Some("never")
    ));
}

#[tokio::test]
async fn cua_cancel_turn_interrupts_an_action_over_the_control_path() {
    let started = Arc::new(Notify::new());
    let daemon = Arc::new(daemon_with_backend(Box::new(CuaBlockingBackend {
        snapshot: snapshot(None, Vec::new()),
        started: started.clone(),
    })));
    let appshot_id = authorize_desktop_appshot(&daemon, Some("session-cancel")).await;
    let context = CuaRequestContext {
        session_id: "session-cancel".to_string(),
        appshot_id: Some(appshot_id),
        turn_id: "turn-cancel".to_string(),
        deadline_ms: Some(30_000),
    };
    let action_daemon = daemon.clone();
    let action = tokio::spawn(async move {
        action_daemon
            .handle(ServiceRequest::Click {
                context,
                x: 10.0,
                y: 20.0,
                mouse_button: None,
                click_count: None,
                key: None,
                post_action_sleep_ms: Some(0),
            })
            .await
    });
    started.notified().await;

    let cancel = daemon
        .handle(ServiceRequest::CancelTurn {
            session_id: "session-cancel".to_string(),
            turn_id: "turn-cancel".to_string(),
            reason: "caller stopped the turn".to_string(),
        })
        .await;
    assert!(matches!(
        cancel,
        ServiceResponse::CancelTurn {
            status: sky_cua_platform::model::CuaCancelStatus::CancelRequested,
            ..
        }
    ));

    let response = tokio::time::timeout(Duration::from_secs(1), action)
        .await
        .expect("cancelled CUA action should finish")
        .expect("CUA action task should not panic");
    assert!(matches!(
        response,
        ServiceResponse::Error { ref code, .. } if code == "SKY_CUA_TURN_CANCELLED"
    ));

    let repeat = daemon
        .handle(ServiceRequest::CancelTurn {
            session_id: "session-cancel".to_string(),
            turn_id: "turn-cancel".to_string(),
            reason: "repeat".to_string(),
        })
        .await;
    assert!(matches!(
        repeat,
        ServiceResponse::CancelTurn {
            status: sky_cua_platform::model::CuaCancelStatus::NotFound,
            ..
        }
    ));
}

#[tokio::test]
async fn cua_duplicate_active_turn_is_rejected_without_sharing_cancellation() {
    let started = Arc::new(Notify::new());
    let daemon = Arc::new(daemon_with_backend(Box::new(CuaBlockingBackend {
        snapshot: snapshot(None, Vec::new()),
        started: started.clone(),
    })));
    let appshot_id = authorize_desktop_appshot(&daemon, Some("duplicate-session")).await;
    let context = CuaRequestContext {
        session_id: "duplicate-session".to_string(),
        appshot_id: Some(appshot_id),
        turn_id: "duplicate-turn".to_string(),
        deadline_ms: Some(30_000),
    };
    let action_daemon = daemon.clone();
    let first_context = context.clone();
    let first = tokio::spawn(async move {
        action_daemon
            .handle(ServiceRequest::Move {
                context: first_context,
                x: 1.0,
                y: 2.0,
                key: None,
                post_action_sleep_ms: Some(0),
            })
            .await
    });
    started.notified().await;
    let duplicate = daemon
        .handle(ServiceRequest::Move {
            context,
            x: 3.0,
            y: 4.0,
            key: None,
            post_action_sleep_ms: Some(0),
        })
        .await;
    assert!(matches!(
        duplicate,
        ServiceResponse::Error { ref code, .. } if code == "SKY_CUA_DUPLICATE_ACTIVE_TURN"
    ));
    daemon
        .handle(ServiceRequest::CancelTurn {
            session_id: "duplicate-session".to_string(),
            turn_id: "duplicate-turn".to_string(),
            reason: "finish test".to_string(),
        })
        .await;
    let _ = first.await.expect("first action should finish");
}

#[tokio::test]
async fn cua_action_and_screenshot_deadlines_include_desktop_queue_wait() {
    let backend = BlockingBackend {
        snapshot: snapshot(None, Vec::new()),
        outcome: success_outcome(),
        execute_calls: Arc::new(AtomicUsize::new(0)),
        first_execute_started: Arc::new(Notify::new()),
        second_execute_started: Arc::new(Notify::new()),
        release_first_execute: Arc::new(Notify::new()),
    };
    let started = backend.first_execute_started.clone();
    let release = backend.release_first_execute.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));
    let blocker_request =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 1.0, "y": 2.0})).await;
    let queued_appshot = authorize_desktop_appshot(&daemon, Some("queued-action")).await;
    let blocking_daemon = daemon.clone();
    let blocker = tokio::spawn(async move {
        blocking_daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(blocker_request),
            })
            .await
    });
    started.notified().await;

    let action = daemon
        .handle(ServiceRequest::Move {
            context: CuaRequestContext {
                session_id: "queued-action".to_string(),
                appshot_id: Some(queued_appshot),
                turn_id: "turn".to_string(),
                deadline_ms: Some(5),
            },
            x: 1.0,
            y: 2.0,
            key: None,
            post_action_sleep_ms: Some(0),
        })
        .await;
    assert!(matches!(
        action,
        ServiceResponse::Error { ref code, .. } if code == "SKY_CUA_DEADLINE_EXCEEDED"
    ));

    let screenshot = daemon
        .handle(ServiceRequest::GetScreenshot {
            context: Some(CuaRequestContext {
                session_id: "queued-shot".to_string(),
                appshot_id: None,
                turn_id: "turn".to_string(),
                deadline_ms: Some(5),
            }),
            mouse_size_px: Some(0),
        })
        .await;
    assert!(matches!(
        screenshot,
        ServiceResponse::Error { ref code, .. } if code == "SKY_CUA_DEADLINE_EXCEEDED"
    ));

    release.notify_one();
    let _ = blocker.await.expect("blocking action should finish");
}

#[tokio::test]
async fn cua_cancel_waits_for_backend_release_after_input_down() {
    let input_down = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let daemon = Arc::new(daemon_with_backend(Box::new(CuaCleanupBackend {
        snapshot: snapshot(None, Vec::new()),
        input_down: input_down.clone(),
        released: released.clone(),
    })));
    let appshot_id = authorize_desktop_appshot(&daemon, Some("cleanup-session")).await;
    let action_daemon = daemon.clone();
    let action = tokio::spawn(async move {
        action_daemon
            .handle(ServiceRequest::Drag {
                context: CuaRequestContext {
                    session_id: "cleanup-session".to_string(),
                    appshot_id: Some(appshot_id),
                    turn_id: "cleanup-turn".to_string(),
                    deadline_ms: Some(30_000),
                },
                from_x: 1.0,
                from_y: 2.0,
                to_x: 3.0,
                to_y: 4.0,
                key: Some("Ctrl".to_string()),
                post_action_sleep_ms: Some(0),
            })
            .await
    });
    input_down.notified().await;
    daemon
        .handle(ServiceRequest::CancelTurn {
            session_id: "cleanup-session".to_string(),
            turn_id: "cleanup-turn".to_string(),
            reason: "test cleanup".to_string(),
        })
        .await;
    let response = action.await.expect("action task should finish");
    assert!(released.load(Ordering::SeqCst));
    assert!(matches!(
        response,
        ServiceResponse::Error { ref code, .. } if code == "SKY_CUA_ACTION_OUTCOME_UNKNOWN"
    ));
}

#[tokio::test]
async fn cua_deadline_waits_for_backend_release_after_input_down() {
    let released = Arc::new(AtomicBool::new(false));
    let daemon = daemon_with_backend(Box::new(CuaCleanupBackend {
        snapshot: snapshot(None, Vec::new()),
        input_down: Arc::new(Notify::new()),
        released: released.clone(),
    }));
    let appshot_id = authorize_desktop_appshot(&daemon, Some("deadline-session")).await;
    let response = daemon
        .handle(ServiceRequest::Click {
            context: CuaRequestContext {
                session_id: "deadline-session".to_string(),
                appshot_id: Some(appshot_id),
                turn_id: "deadline-turn".to_string(),
                deadline_ms: Some(1),
            },
            x: 1.0,
            y: 2.0,
            mouse_button: None,
            click_count: None,
            key: None,
            post_action_sleep_ms: Some(0),
        })
        .await;
    assert!(released.load(Ordering::SeqCst));
    assert!(matches!(
        response,
        ServiceResponse::Error { ref code, .. } if code == "SKY_CUA_ACTION_OUTCOME_UNKNOWN"
    ));
}

#[test]
fn cua_screenshot_pixels_map_to_fractional_wayland_desktop_geometry() {
    let plane = CuaScreenshotCoordinatePlane {
        desktop_rect: RectF {
            x: -320.0,
            y: 180.0,
            width: 1706.67,
            height: 1066.67,
            space: CoordinateSpace::DesktopLogical,
        },
        width: 1440,
        height: 900,
    };

    assert_eq!(plane.to_desktop(0.0, 0.0), (-320.0, 180.0));
    let center = plane.to_desktop(720.0, 450.0);
    assert!((center.0 - 533.335).abs() < 0.000_001);
    assert!((center.1 - 713.335).abs() < 0.000_001);
    let far_corner = plane.to_desktop(1440.0, 900.0);
    assert!((far_corner.0 - 1386.67).abs() < 0.000_001);
    assert!((far_corner.1 - 1246.67).abs() < 0.000_001);
}

#[tokio::test]
async fn cua_invalid_empty_context_is_omitted_from_error_serialization() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    let response = daemon
        .handle(ServiceRequest::Click {
            context: CuaRequestContext {
                session_id: "  ".to_string(),
                appshot_id: None,
                turn_id: String::new(),
                deadline_ms: None,
            },
            x: 1.0,
            y: 2.0,
            mouse_button: None,
            click_count: None,
            key: None,
            post_action_sleep_ms: None,
        })
        .await;
    let json = serde_json::to_value(response).expect("error response should serialize");
    assert_eq!(json["code"], "SKY_CUA_INVALID_CONTEXT");
    assert!(json.get("session_id").is_none());
    assert!(json.get("turn_id").is_none());

    let cancel = daemon
        .handle(ServiceRequest::CancelTurn {
            session_id: String::new(),
            turn_id: " ".to_string(),
            reason: "cancel".to_string(),
        })
        .await;
    let cancel_json = serde_json::to_value(cancel).expect("cancel error should serialize");
    assert!(cancel_json.get("session_id").is_none());
    assert!(cancel_json.get("turn_id").is_none());
}

#[tokio::test]
async fn phone_status_routes_through_manager_to_status_response() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::Status(PhoneStatusRequest::default()),
            context: None,
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
}

#[tokio::test]
async fn phone_list_devices_routes_through_manager_to_devices_response() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
            context: None,
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
}

#[tokio::test]
async fn phone_connect_without_device_does_not_fabricate_session() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::Connect(PhoneConnectRequest::default()),
            context: None,
        })
        .await
    {
        ServiceResponse::Phone {
            response: PhoneResponse::Status(report),
        } => assert!(report.sessions.is_empty()),
        other => panic!("connect must not fabricate a session: {other:?}"),
    }
}

#[tokio::test]
async fn phone_tap_without_session_returns_structured_action_failure() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::Tap(PhoneTapRequest {
                session: Default::default(),
                phone_snapshot_id: Some("snap".to_string()),
                x: 5.0,
                y: 6.0,
                use_device_coordinates: false,
            }),
            context: None,
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
}

#[tokio::test]
async fn phone_app_list_routes_through_manager_to_app_response() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    match daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::AppList(PhoneAppListRequest::default()),
            context: None,
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
async fn phone_request_context_is_recorded_without_requiring_legacy_callers_to_supply_it() {
    let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
    assert!(daemon.last_phone_request_context_for_tests().is_none());

    daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::Status(PhoneStatusRequest::default()),
            context: None,
        })
        .await;
    assert!(daemon.last_phone_request_context_for_tests().is_none());

    let context = PhoneRequestContext {
        session_id: Some("session-openclaw".to_string()),
        turn_id: Some("turn-7".to_string()),
        caller_provenance: Some(PhoneCallerProvenance::OpenClaw),
        identity_synthetic: Some(true),
        client_info: Some(PhoneMcpClientInfo {
            name: "openclaw".to_string(),
            version: "7.1".to_string(),
            title: Some("OpenClaw".to_string()),
        }),
    };
    daemon
        .handle(ServiceRequest::Phone {
            request: PhoneRequest::Status(PhoneStatusRequest::default()),
            context: Some(context.clone()),
        })
        .await;
    assert_eq!(daemon.last_phone_request_context_for_tests(), Some(context));
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
        ServiceResponse::Error { code, message, .. } => {
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

#[tokio::test]
async fn fresh_appshot_authorizes_snapshotless_physical_coordinates() {
    let recorded_action = Arc::new(std::sync::Mutex::new(None));
    let daemon = daemon_with_backend(Box::new(FakeBackend {
        snapshot: snapshot(Some(capture_with_rect()), Vec::new()),
        outcome: success_outcome(),
        presence: None,
        recorded_action: Some(recorded_action.clone()),
    }));
    let mut click = request(ActionName::Click, json!({"x": 1454.5, "y": 252.0}));
    click.appshot_id = Some(authorize_desktop_appshot(&daemon, None).await);

    match daemon
        .handle(ServiceRequest::ExecuteAction {
            request: Box::new(click),
        })
        .await
    {
        ServiceResponse::ExecuteAction { outcome } => {
            assert!(outcome.success);
        }
        other => panic!("fresh AppShot should authorize raw physical coordinates, got {other:?}"),
    }
    let recorded = recorded_action
        .lock()
        .expect("recorded action lock")
        .clone()
        .expect("backend action");
    assert_eq!(recorded.snapshot_id, None);
    assert_eq!(recorded.resolved_capture, None);
    assert_eq!(
        recorded
            .resolved_focused_app
            .as_ref()
            .and_then(|app| app.window_handle.as_deref()),
        Some("fake-window")
    );
    assert_eq!(recorded.environment, Some(environment()));
    assert_eq!(recorded.arguments["x"], 1454.5);
    assert_eq!(recorded.arguments["y"], 252.0);
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

    let click =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 42.0, "y": 24.0})).await;

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

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_action_waits_for_cursor_arrival_before_backend_dispatch() {
    if std::process::Command::new("python3")
        .arg("--version")
        .status()
        .is_err()
    {
        return;
    }

    let dir = unique_temp_dir("action-visual-arrival");
    let host_path = dir.join("fake-overlay-host.py");
    let socket_path = dir.join("agent-cursor.sock");
    let arrival_marker = PathBuf::from(format!("{}.arrived", socket_path.display()));
    let request_log = PathBuf::from(format!("{}.requests", socket_path.display()));
    crate::overlay::test_support::write_fake_overlay_host(&host_path);
    let dispatched_after_arrival = Arc::new(AtomicBool::new(false));
    let backend = ArrivalCheckingBackend {
        snapshot: snapshot(None, Vec::new()),
        arrival_marker,
        dispatched_after_arrival: dispatched_after_arrival.clone(),
    };
    let daemon = daemon_with_backend_and_overlay(
        Box::new(backend),
        OverlayController::new_for_tests_with_host(host_path, socket_path),
    );
    let click =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 42.0, "y": 24.0})).await;

    let outcome = match daemon
        .handle(ServiceRequest::ExecuteAction {
            request: Box::new(click),
        })
        .await
    {
        ServiceResponse::ExecuteAction { outcome } => outcome,
        other => panic!("unexpected response: {other:?}"),
    };
    assert!(outcome.success);
    assert!(
        dispatched_after_arrival.load(Ordering::SeqCst),
        "backend input dispatch must not run before the overlay reports arrival; diagnostics={:?}",
        outcome.diagnostics
    );
    assert_eq!(
        std::fs::read_to_string(request_log)
            .expect("arrival request log")
            .lines()
            .collect::<Vec<_>>(),
        vec!["set_cursor", "animate_gesture", "wait_for_arrival"]
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_arrival_wait_fails_open_within_absolute_deadline() {
    if std::process::Command::new("python3")
        .arg("--version")
        .status()
        .is_err()
    {
        return;
    }

    let dir = unique_temp_dir("action-visual-arrival-stalled");
    let host_path = dir.join("stalled-overlay-host.py");
    let socket_path = dir.join("a.sock");
    let request_log = PathBuf::from(format!("{}.requests", socket_path.display()));
    crate::overlay::test_support::write_stalled_overlay_host(&host_path);
    let backend = FakeBackend {
        snapshot: snapshot(None, Vec::new()),
        outcome: success_outcome(),
        presence: None,
        recorded_action: None,
    };
    let daemon = daemon_with_backend_and_overlay(
        Box::new(backend),
        OverlayController::new_for_tests_with_host(host_path, socket_path),
    );
    let click =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 42.0, "y": 24.0})).await;

    let started = tokio::time::Instant::now();
    let outcome = match daemon
        .handle(ServiceRequest::ExecuteAction {
            request: Box::new(click),
        })
        .await
    {
        ServiceResponse::ExecuteAction { outcome } => outcome,
        other => panic!("unexpected response: {other:?}"),
    };

    assert!(outcome.success);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "arrival wait exceeded bounded fail-open latency: {:?}",
        started.elapsed()
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|entry| entry.code == "AgentCursorArrivalTimeout")
    );
    assert_eq!(
        std::fs::read_to_string(request_log)
            .expect("stalled arrival request log")
            .lines()
            .collect::<Vec<_>>(),
        vec!["set_cursor", "animate_gesture", "wait_for_arrival"]
    );
}

#[tokio::test]
async fn service_runtime_health_bypasses_blocked_desktop_request() {
    let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
    let first_started = backend.first_execute_started.clone();
    let release_first = backend.release_first_execute.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));
    let action =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 42.0, "y": 24.0})).await;

    let action_daemon = daemon.clone();
    let action_task = tokio::spawn(async move {
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
async fn service_runtime_health_never_probes_the_backend_inline() {
    let backend = HangingBackend::default();
    let probe_calls = backend.probe_calls.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let daemon = daemon.clone();
        tasks.push(tokio::spawn(async move {
            daemon.handle(ServiceRequest::Health).await
        }));
    }

    let responses = tokio::time::timeout(Duration::from_millis(100), async {
        let mut responses = Vec::new();
        for task in tasks {
            responses.push(task.await.expect("health task"));
        }
        responses
    })
    .await
    .expect("health must not wait for the desktop backend");

    for response in responses {
        match response {
            ServiceResponse::Health {
                ok,
                capabilities,
                desktop_env: _,
                browser_env: _,
                ..
            } => {
                assert!(ok);
                assert!(capabilities.iter().any(|capability| {
                    capability == sky_cua_platform::model::BROWSER_CONTROL_CAPABILITY_V1
                }));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }
    assert_eq!(
        probe_calls.load(Ordering::SeqCst),
        0,
        "Health must only read the service-owned capability snapshot"
    );
}

#[tokio::test]
async fn health_capability_refresher_is_single_and_caller_independent() {
    let backend = HangingBackend::default();
    let probe_calls = backend.probe_calls.clone();
    let probe_started = backend.probe_started.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));
    let started = probe_started.notified();
    let refresher = daemon.spawn_health_capability_refresher();

    tokio::time::timeout(Duration::from_millis(100), started)
        .await
        .expect("the daemon-owned refresher should start immediately");
    assert_eq!(probe_calls.load(Ordering::SeqCst), 1);

    for _ in 0..100 {
        assert!(matches!(
            daemon.handle(ServiceRequest::Health).await,
            ServiceResponse::Health { ok: true, .. }
        ));
    }
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert_eq!(
        probe_calls.load(Ordering::SeqCst),
        1,
        "Health callers must neither fan out nor restart the pending refresh"
    );
    refresher.abort();
}

#[tokio::test]
async fn health_capability_refresh_updates_downgrades_and_recovers() {
    let backend = HealthProbeBackend::new(HealthProbeResponse::Environment(environment()));
    let backend_control = backend.clone();
    let daemon = daemon_with_backend(Box::new(backend));

    assert!(daemon.refresh_health_capability_snapshot().await);
    let ServiceResponse::Health { capabilities, .. } = daemon.handle(ServiceRequest::Health).await
    else {
        panic!("expected Health response");
    };
    assert!(
        capabilities
            .iter()
            .any(|value| value == "linux.scroll.pixels")
    );

    backend_control.set_response(HealthProbeResponse::Error);
    assert!(!daemon.refresh_health_capability_snapshot().await);
    let ServiceResponse::Health { capabilities, .. } = daemon.handle(ServiceRequest::Health).await
    else {
        panic!("expected Health response");
    };
    assert!(
        !capabilities
            .iter()
            .any(|value| value.starts_with("linux.scroll."))
    );
    assert!(
        capabilities
            .iter()
            .any(|value| { value == sky_cua_platform::model::BROWSER_CONTROL_CAPABILITY_V1 })
    );

    backend_control.set_response(HealthProbeResponse::Environment(environment()));
    assert!(daemon.refresh_health_capability_snapshot().await);
    backend_control.set_response(HealthProbeResponse::Pending);
    assert!(!daemon.refresh_health_capability_snapshot().await);
    let ServiceResponse::Health { capabilities, .. } = daemon.handle(ServiceRequest::Health).await
    else {
        panic!("expected Health response");
    };
    assert!(
        !capabilities
            .iter()
            .any(|value| value.starts_with("linux.scroll."))
    );

    let mut degraded_input = environment();
    degraded_input.input_backend = InputBackendKind::XTest;
    degraded_input.semantic_backend = SemanticBackendKind::None;
    backend_control.set_response(HealthProbeResponse::Environment(degraded_input));
    assert!(!daemon.refresh_health_capability_snapshot().await);
    let ServiceResponse::Health { capabilities, .. } = daemon.handle(ServiceRequest::Health).await
    else {
        panic!("expected Health response");
    };
    assert!(
        capabilities
            .iter()
            .any(|value| value == "linux.scroll.direction")
    );
    assert!(
        !capabilities
            .iter()
            .any(|value| value == "linux.scroll.pixels")
    );

    backend_control.set_response(HealthProbeResponse::Environment(environment()));
    assert!(daemon.refresh_health_capability_snapshot().await);
    assert_eq!(backend_control.probe_calls.load(Ordering::SeqCst), 6);
}

#[test]
fn health_capability_refresh_backoff_is_bounded() {
    assert_eq!(
        super::health_capability_refresh_delay(0),
        Duration::from_secs(30)
    );
    assert_eq!(
        super::health_capability_refresh_delay(1),
        Duration::from_secs(30)
    );
    assert_eq!(
        super::health_capability_refresh_delay(2),
        Duration::from_secs(60)
    );
    assert_eq!(
        super::health_capability_refresh_delay(3),
        Duration::from_secs(120)
    );
    assert_eq!(
        super::health_capability_refresh_delay(4),
        Duration::from_secs(240)
    );
    assert_eq!(
        super::health_capability_refresh_delay(5),
        Duration::from_secs(300)
    );
    assert_eq!(
        super::health_capability_refresh_delay(u32::MAX),
        Duration::from_secs(300)
    );
}

#[tokio::test]
async fn service_runtime_browser_open_bypasses_blocked_desktop_request() {
    let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
    let first_started = backend.first_execute_started.clone();
    let release_first = backend.release_first_execute.clone();
    let daemon = Arc::new(daemon_with_backend(Box::new(backend)));
    let action =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 42.0, "y": 24.0})).await;

    let action_daemon = daemon.clone();
    let action_task = tokio::spawn(async move {
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
                identity: None,
                context: Some(BrowserRequestContext {
                    provenance: BrowserCallerProvenance {
                        caller: BrowserCallerKind::DirectMcp,
                        source: BrowserProvenanceSource::ClientInfoInference,
                        connection_id: "blocked-desktop-browser-open".to_string(),
                        declared_caller: None,
                        client_info: None,
                    },
                    logical_identity: BrowserLogicalIdentity {
                        session_id: "blocked-desktop-browser-open".to_string(),
                        thread_id: None,
                        turn_id: None,
                    },
                    operation_identity: BrowserOperationIdentity {
                        operation_id: "blocked-desktop-browser-open".to_string(),
                        request_id_fingerprint: "blocked-desktop-browser-open".to_string(),
                    },
                }),
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
    let action =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 42.0, "y": 24.0})).await;

    let action_daemon = daemon.clone();
    let action_task = tokio::spawn(async move {
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
                identity: None,
                context: None,
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
async fn service_runtime_browser_status_defers_a_hung_doctor_probe() {
    let daemon = daemon_with_backend(Box::new(HangingBackend::default()));
    let response = tokio::time::timeout(
        Duration::from_millis(500),
        daemon.handle(ServiceRequest::Browser {
            request: BrowserRequest::Status,
            identity: None,
            context: None,
        }),
    )
    .await
    .expect("browser status must abandon a hung doctor probe");

    match response {
        ServiceResponse::Browser {
            response: BrowserResponse::Status { report },
        } => {
            let diagnostic = report
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == "BrowserIntegrationDeferred")
                .expect("timed-out doctor should produce a deferred diagnostic");
            assert!(diagnostic.message.contains("status deadline"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert!(
        daemon.desktop_lane.try_lock().is_ok(),
        "browser status timeout must release the desktop lane"
    );
}

#[tokio::test]
async fn hybrid_browser_status_reports_codex_ingress_bind_degradation() {
    let backend = FakeBackend {
        snapshot: snapshot(Some(capture_with_rect()), Vec::new()),
        outcome: success_outcome(),
        presence: None,
        recorded_action: None,
    };
    let mut daemon = daemon_with_backend(Box::new(backend));
    daemon.browser_control_mode = Ok(crate::browser::BrowserControlMode::Hybrid);
    daemon.browser_control_runtime = Some(crate::browser::BrowserControlRuntime::new());
    daemon.record_browser_control_startup_diagnostic(DiagnosticEntry {
        code: "CodexBrowserIngressUnavailable".to_owned(),
        message: "fixture".to_owned(),
        details: Some("bind failed".to_owned()),
    });

    let response = daemon
        .handle(ServiceRequest::Browser {
            request: BrowserRequest::Status,
            identity: None,
            context: Some(BrowserRequestContext {
                provenance: BrowserCallerProvenance {
                    caller: BrowserCallerKind::DirectMcp,
                    source: BrowserProvenanceSource::ClientInfoInference,
                    connection_id: "status-test".to_owned(),
                    declared_caller: None,
                    client_info: None,
                },
                logical_identity: BrowserLogicalIdentity {
                    session_id: "status-test".to_owned(),
                    thread_id: None,
                    turn_id: None,
                },
                operation_identity: BrowserOperationIdentity {
                    operation_id: "status-test-operation".to_owned(),
                    request_id_fingerprint: "status-test".to_owned(),
                },
            }),
        })
        .await;
    let ServiceResponse::Browser {
        response: BrowserResponse::Status { report },
    } = response
    else {
        panic!("unexpected response: {response:?}");
    };
    assert!(
        report
            .diagnostics
            .iter()
            .any(|entry| entry.code == "CodexBrowserIngressUnavailable")
    );
}

#[tokio::test]
async fn browser_snapshot_rejects_oversized_text_limit_at_service_boundary() {
    let daemon = daemon_with(
        snapshot(Some(capture_with_rect()), Vec::new()),
        success_outcome(),
    );

    match daemon
        .handle(ServiceRequest::Browser {
            identity: None,
            context: None,
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
        ServiceResponse::Error { code, message, .. } => {
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
            identity: None,
            context: None,
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
        ServiceResponse::Error { code, message, .. } => {
            assert_eq!(code, "InvalidRequest");
            assert!(message.contains(&BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT.to_string()));
        }
        other => panic!("expected invalid request response, got: {other:?}"),
    }
}

#[tokio::test]
async fn browser_observe_rejects_oversized_projection_limits_at_service_boundary() {
    let daemon = daemon_with(
        snapshot(Some(capture_with_rect()), Vec::new()),
        success_outcome(),
    );
    let requests = [
        BrowserRequest::ObserveAppShot {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "tab-1".to_string(),
            text_limit: Some(BROWSER_SNAPSHOT_MAX_TEXT_LIMIT + 1),
            element_offset: None,
            element_limit: None,
            element_query: None,
            capture_timeout_ms: None,
            include_image_data: false,
        },
        BrowserRequest::ObserveAppShot {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "tab-1".to_string(),
            text_limit: None,
            element_offset: None,
            element_limit: Some(BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT + 1),
            element_query: None,
            capture_timeout_ms: None,
            include_image_data: false,
        },
    ];

    for request in requests {
        match daemon
            .handle(ServiceRequest::Browser {
                identity: None,
                context: None,
                request,
            })
            .await
        {
            ServiceResponse::Error { code, .. } => assert_eq!(code, "InvalidRequest"),
            other => panic!("expected invalid request response, got: {other:?}"),
        }
    }
}

#[tokio::test]
async fn browser_observe_rejects_invalid_capture_timeout_at_service_boundary() {
    let daemon = daemon_with(
        snapshot(Some(capture_with_rect()), Vec::new()),
        success_outcome(),
    );

    match daemon
        .handle(ServiceRequest::Browser {
            identity: None,
            context: None,
            request: BrowserRequest::ObserveAppShot {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "tab-1".to_string(),
                text_limit: None,
                element_offset: None,
                element_limit: None,
                element_query: None,
                capture_timeout_ms: Some(
                    sky_cua_platform::model::BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS - 1,
                ),
                include_image_data: false,
            },
        })
        .await
    {
        ServiceResponse::Error { code, message, .. } => {
            assert_eq!(code, "InvalidRequest");
            assert!(message.contains(
                &sky_cua_platform::model::BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS.to_string()
            ));
            assert!(message.contains(
                &sky_cua_platform::model::BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS.to_string()
            ));
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
    let first_action =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 1.0, "y": 2.0})).await;
    let second_action =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 3.0, "y": 4.0})).await;

    let first_daemon = daemon.clone();
    let first_task = tokio::spawn(async move {
        first_daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(first_action),
            })
            .await
    });
    first_started.notified().await;

    let second_daemon = daemon.clone();
    let second_task = tokio::spawn(async move {
        second_daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(second_action),
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
            recorded_action: None,
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
        let action =
            authorized_action_request(&daemon, ActionName::Click, json!({"x": 1.0, "y": 2.0}))
                .await;
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

    let action =
        authorized_action_request(&daemon, ActionName::Click, json!({"x": 3.0, "y": 4.0})).await;
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
            recorded_action: None,
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

fn fake_window() -> WindowInfo {
    WindowInfo {
        window_id: "fake-window".to_string(),
        title: Some("Fake window".to_string()),
        app_id: Some("fake.app".to_string()),
        wm_class: Some("FakeApp".to_string()),
        pid: Some(42),
        bounds: Some(RectF {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 200.0,
            space: CoordinateSpace::DesktopLogical,
        }),
        display: None,
        display_intersections: Vec::new(),
        workspace: None,
        focused: true,
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
        recorded_action: None,
    }))
}

fn daemon_with_backend(backend: Box<dyn DesktopBackend>) -> ServiceDaemon {
    daemon_with_backend_and_presence_config(backend, SessionPresenceConfig::disabled())
}

fn daemon_with_backend_and_overlay(
    backend: Box<dyn DesktopBackend>,
    overlay: OverlayController,
) -> ServiceDaemon {
    daemon_with_phone_and_overlay(
        backend,
        SessionPresenceConfig::disabled(),
        test_phone_manager(),
        overlay,
    )
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
    daemon_with_phone_and_overlay(
        backend,
        session_presence_config,
        phone,
        OverlayController::new_for_tests(),
    )
}

fn daemon_with_phone_and_overlay(
    backend: Box<dyn DesktopBackend>,
    session_presence_config: SessionPresenceConfig,
    phone: crate::phone::PhoneManager,
    overlay: OverlayController,
) -> ServiceDaemon {
    ServiceDaemon {
        backend,
        sessions: SessionStore::new(),
        snapshots: tokio::sync::Mutex::new(SnapshotManager::new(8)),
        overlay: tokio::sync::Mutex::new(overlay),
        phone: tokio::sync::Mutex::new(phone),
        phone_direct: tokio::sync::Mutex::new(None),
        last_phone_request_context: std::sync::Mutex::new(None),
        session_presence_config,
        session_presence_held: tokio::sync::Mutex::new(false),
        desktop_lane: tokio::sync::Mutex::new(()),
        browser_eval_enabled: false,
        socket_path: PathBuf::from("/tmp/sky-cua-test.sock"),
        cua_cancellations: std::sync::Mutex::new(std::collections::HashMap::new()),
        cua_screenshot_planes: std::sync::Mutex::new(std::collections::HashMap::new()),
        browser_control_mode: Ok(crate::browser::BrowserControlMode::Legacy),
        browser_control_runtime: None,
        browser_control_startup_diagnostics: std::sync::Mutex::new(Vec::new()),
        health_capability_snapshot: std::sync::RwLock::new(
            super::HealthCapabilitySnapshot::default(),
        ),
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

#[tokio::test]
async fn daemon_phone_manager_exposes_installed_direct_runtime_provider() {
    let mut phone = test_phone_manager();
    phone.set_direct_runtime(Some(crate::phone::DirectRuntimeHandle::new()));
    let daemon = daemon_with_phone(
        Box::new(FakeBackend {
            snapshot: snapshot(None, Vec::new()),
            outcome: success_outcome(),
            presence: None,
            recorded_action: None,
        }),
        SessionPresenceConfig::disabled(),
        phone,
    );
    assert!(daemon.phone.lock().await.direct_provider().is_some());
}
