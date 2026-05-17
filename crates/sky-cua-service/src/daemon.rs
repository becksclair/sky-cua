use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::model::{
    ActionName, ActionRequest, AppStateSnapshot, CaptureInfo, CaptureScreenMode, DiagnosticEntry,
    ServiceRequest, ServiceResponse,
};

use crate::action_router::route_action;
use crate::approval_store::ApprovalStore;
use crate::backend_factory::create_backend;
use crate::diagnostics::error_response;
use crate::element_resolver::{resolve_action_element, resolve_target_element};
use crate::overlay::{AgentCursorStatus, OverlayController};
use crate::session_store::SessionStore;
use crate::snapshot_manager::SnapshotManager;
use tracing::debug;

pub struct ServiceDaemon {
    backend: Box<dyn DesktopBackend>,
    sessions: SessionStore,
    snapshots: SnapshotManager,
    overlay: OverlayController,
    socket_path: PathBuf,
}

impl ServiceDaemon {
    pub async fn new(socket_path: PathBuf) -> std::io::Result<Self> {
        ApprovalStore::initialize()?;
        let backend = create_backend();
        if let Err(error) = backend.prepare_automation_permissions().await {
            debug!(
                code = error.code,
                message = error.message,
                "desktop backend automation permission preparation did not complete"
            );
        }
        Ok(Self {
            backend,
            sessions: SessionStore::new(),
            snapshots: SnapshotManager::new(8),
            overlay: OverlayController::new(&socket_path),
            socket_path,
        })
    }

    pub async fn handle(&mut self, request: ServiceRequest) -> ServiceResponse {
        self.sessions.touch().await;
        match request {
            ServiceRequest::Health => ServiceResponse::Health {
                ok: true,
                service_socket: self.socket_path.display().to_string(),
                desktop_env: desktop_env_values_present(),
            },
            ServiceRequest::Doctor => match self.backend.doctor().await {
                Ok(report) => ServiceResponse::Doctor {
                    report: Box::new(report),
                },
                Err(error) => error_response(error.code, error.message),
            },
            ServiceRequest::SetupAccessibility => match self.backend.setup_accessibility().await {
                Ok(report) => ServiceResponse::SetupAccessibility {
                    report: Box::new(report),
                },
                Err(error) => error_response(error.code, error.message),
            },
            ServiceRequest::SetupWindowTargeting => {
                match self.backend.setup_window_targeting().await {
                    Ok(report) => ServiceResponse::SetupWindowTargeting {
                        report: Box::new(report),
                    },
                    Err(error) => error_response(error.code, error.message),
                }
            }
            ServiceRequest::ListApps => {
                debug!("handling list_apps request");
                let environment = match self.backend.probe_environment().await {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self.backend.list_apps().await {
                    Ok(apps) => {
                        let diagnostics = self.backend.session_env_diagnostics();
                        ServiceResponse::ListApps {
                            environment,
                            apps,
                            diagnostics,
                        }
                    }
                    Err(error) => ServiceResponse::ListApps {
                        environment,
                        apps: Vec::new(),
                        diagnostics: {
                            let mut diagnostics = self.backend.session_env_diagnostics();
                            diagnostics.push(error.diagnostic());
                            diagnostics
                        },
                    },
                }
            }
            ServiceRequest::ListWindows => {
                debug!("handling list_windows request");
                let environment = match self.backend.probe_environment().await {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self.backend.list_windows().await {
                    Ok(windows) => ServiceResponse::ListWindows {
                        environment,
                        windows,
                        diagnostics: Vec::new(),
                    },
                    Err(error) => ServiceResponse::ListWindows {
                        environment,
                        windows: Vec::new(),
                        diagnostics: vec![error.diagnostic()],
                    },
                }
            }
            ServiceRequest::FocusedWindow => {
                debug!("handling focused_window request");
                let environment = match self.backend.probe_environment().await {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self.backend.focused_window().await {
                    Ok(window) => ServiceResponse::FocusedWindow {
                        environment,
                        window: window.map(Box::new),
                        diagnostics: Vec::new(),
                    },
                    Err(error) => ServiceResponse::FocusedWindow {
                        environment,
                        window: None,
                        diagnostics: vec![error.diagnostic()],
                    },
                }
            }
            ServiceRequest::ActivateWindow { target } => {
                debug!(target = ?target, "handling activate_window request");
                match self.backend.activate_window(target).await {
                    Ok(outcome) => ServiceResponse::ActivateWindow { outcome },
                    Err(error) => {
                        let diagnostic = error.diagnostic();
                        ServiceResponse::ActivateWindow {
                            outcome: sky_cua_platform::model::ActionOutcome {
                                success: false,
                                message: error.message.clone(),
                                code: error.code.to_string(),
                                diagnostics: vec![diagnostic],
                                agent_cursor: None,
                            },
                        }
                    }
                }
            }
            ServiceRequest::GetAppState {
                selector,
                capture_screen,
            } => {
                debug!(selector = ?selector, ?capture_screen, "handling get_app_state request");
                let capture_guard = (capture_screen != CaptureScreenMode::Never)
                    .then(|| self.overlay.prepare_for_capture());
                match self.backend.get_app_state(selector, capture_screen).await {
                    Ok(mut snapshot) => {
                        if capture_screen == CaptureScreenMode::IfChanged
                            && reuse_unchanged_capture(&mut snapshot, self.snapshots.latest())
                        {
                            snapshot.diagnostics.push(DiagnosticEntry {
                                code: "CaptureScreenUnchanged".to_string(),
                                message: "Screen capture matched the previous model-facing image; reusing the previous screenshot path.".to_string(),
                                details: None,
                            });
                        }
                        if let Some(capture_guard) = capture_guard.as_ref() {
                            snapshot
                                .diagnostics
                                .extend(capture_guard.diagnostics.iter().cloned());
                        }
                        self.overlay.apply_to_snapshot(&mut snapshot);
                        if let Some(capture_guard) = capture_guard {
                            snapshot
                                .diagnostics
                                .extend(self.overlay.restore_after_capture(capture_guard));
                        }
                        self.snapshots.store(snapshot.clone());
                        ServiceResponse::GetAppState {
                            snapshot: Box::new(snapshot),
                        }
                    }
                    Err(error) => {
                        if let Some(capture_guard) = capture_guard {
                            let _ = self.overlay.restore_after_capture(capture_guard);
                        }
                        error_response(error.code, error.message)
                    }
                }
            }
            ServiceRequest::AgentCursorStatus => {
                agent_cursor_status_response(self.overlay.status(), AgentCursorResponseKind::Status)
            }
            ServiceRequest::SetAgentCursor { state } => agent_cursor_status_response(
                self.overlay.set_state(state),
                AgentCursorResponseKind::Set,
            ),
            ServiceRequest::HideAgentCursor { reason } => agent_cursor_status_response(
                self.overlay.hide(reason),
                AgentCursorResponseKind::Hide,
            ),
            ServiceRequest::ShowAgentCursor => {
                agent_cursor_status_response(self.overlay.show(), AgentCursorResponseKind::Show)
            }
            ServiceRequest::ResetPortalTokens => {
                debug!("handling reset_portal_tokens request");
                match self.backend.reset_portal_tokens().await {
                    Ok(outcome) => ServiceResponse::ResetPortalTokens {
                        cleared: outcome.cleared,
                        token_path: outcome.token_path,
                        dropped_cached_session: outcome.dropped_cached_session,
                    },
                    Err(error) => error_response(error.code, error.message),
                }
            }
            ServiceRequest::ExecuteAction { request } => {
                let request = match self.enrich_action_request(*request).await {
                    Ok(request) => request,
                    Err((code, message)) => return error_response(code, message),
                };
                let mut outcome = route_action(self.backend.as_ref(), request.clone()).await;
                let cursor_diagnostics = self.overlay.update_from_action(&request, &mut outcome);
                outcome.diagnostics.extend(cursor_diagnostics);
                ServiceResponse::ExecuteAction { outcome }
            }
        }
    }

    pub async fn idle_for(&self) -> std::time::Duration {
        self.sessions.idle_for().await
    }

    async fn enrich_action_request(
        &self,
        mut request: ActionRequest,
    ) -> Result<ActionRequest, (&'static str, String)> {
        let Some(snapshot_id) = request.snapshot_id.as_deref() else {
            if action_requires_snapshot_context(&request) {
                return Err((
                    "ComputerUseInactive",
                    "Element-targeted actions require a current snapshot_id from get_app_state."
                        .to_string(),
                ));
            }
            request.environment = Some(
                self.backend
                    .probe_environment()
                    .await
                    .map_err(|error| (error.code, error.message))?,
            );
            return Ok(request);
        };
        let snapshot = self.snapshots.get_if_latest(snapshot_id).ok_or_else(|| {
            if self.snapshots.get(snapshot_id).is_some() {
                (
                    "SnapshotStale",
                    format!(
                        "snapshot {snapshot_id} is no longer the latest app state. Re-run get_app_state and retry with the current snapshot_id."
                    ),
                )
            } else {
                (
                    "SnapshotStale",
                    format!("snapshot {snapshot_id} is not present in the service cache"),
                )
            }
        })?;

        request.environment = Some(snapshot.environment.clone());
        request.resolved_capture = snapshot.capture.clone();
        request.resolved_focused_app = snapshot.focused_app.clone();

        request.resolved_element = resolve_action_element(
            snapshot,
            &request.action,
            request.element_index,
            &request.arguments,
        )?;
        request.resolved_target_element = resolve_target_element(snapshot, &request.arguments)?;

        Ok(request)
    }
}

fn reuse_unchanged_capture(
    snapshot: &mut AppStateSnapshot,
    previous: Option<&AppStateSnapshot>,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    let (Some(current_capture), Some(previous_capture)) =
        (snapshot.capture.as_mut(), previous.capture.as_ref())
    else {
        return false;
    };
    if !capture_metadata_compatible_for_reuse(current_capture, previous_capture) {
        return false;
    }
    let (Some(current_path), Some(previous_path)) = (
        current_capture.screenshot_path.as_deref(),
        previous_capture.screenshot_path.as_deref(),
    ) else {
        return false;
    };
    let Ok(current_bytes) = fs::read(current_path) else {
        return false;
    };
    let Ok(previous_bytes) = fs::read(previous_path) else {
        return false;
    };
    if current_bytes != previous_bytes {
        return false;
    }

    current_capture.screenshot_path = previous_capture.screenshot_path.clone();
    current_capture.original_screenshot_path = previous_capture.original_screenshot_path.clone();
    current_capture.model_image_bytes = previous_capture.model_image_bytes;
    current_capture.model_image_encode_ms = previous_capture.model_image_encode_ms;
    true
}

fn capture_metadata_compatible_for_reuse(current: &CaptureInfo, previous: &CaptureInfo) -> bool {
    current.backend == previous.backend
        && current.image_backend == previous.image_backend
        && current.coordinate_space == previous.coordinate_space
        && current.pixel_size == previous.pixel_size
        && current.original_pixel_size == previous.original_pixel_size
        && current.logical_to_pixel_scale == previous.logical_to_pixel_scale
        && current.logical_rect == previous.logical_rect
        && current.model_image_format == previous.model_image_format
        && current.model_image_quality == previous.model_image_quality
}

enum AgentCursorResponseKind {
    Status,
    Set,
    Hide,
    Show,
}

fn agent_cursor_status_response(
    status: AgentCursorStatus,
    kind: AgentCursorResponseKind,
) -> ServiceResponse {
    match kind {
        AgentCursorResponseKind::Status => ServiceResponse::AgentCursorStatus {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Set => ServiceResponse::SetAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Hide => ServiceResponse::HideAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Show => ServiceResponse::ShowAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
    }
}

fn desktop_env_values_present() -> BTreeMap<String, String> {
    [
        "DBUS_SESSION_BUS_ADDRESS",
        "DESKTOP_SESSION",
        "DISPLAY",
        "PATH",
        "WAYLAND_DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
    ]
    .into_iter()
    .filter_map(|key| {
        std::env::var(key)
            .ok()
            .filter(|value| !value.is_empty())
            .map(|value| (key.to_string(), value))
    })
    .collect()
}

fn action_requires_snapshot_context(request: &ActionRequest) -> bool {
    let has_snapshot_target = request.arguments.get("to_element_index").is_some();
    let has_semantic_selector = request.arguments.get("role").is_some()
        || request.arguments.get("name").is_some()
        || (request.arguments.get("text").is_some() && request.action != ActionName::TypeText)
        || request.arguments.get("states").is_some();

    if request
        .arguments
        .get("element_identifier")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return has_snapshot_target;
    }

    matches!(
        request.action,
        ActionName::FocusElement
            | ActionName::ActivateElement
            | ActionName::SelectElement
            | ActionName::ExpandElement
            | ActionName::CollapseElement
            | ActionName::ToggleElement
            | ActionName::PerformAction
            | ActionName::SetValue
    ) || request.element_index.is_some()
        || has_snapshot_target
        || has_semantic_selector
}

#[cfg(test)]
mod tests {
    use super::{
        OverlayController, ServiceDaemon, SessionStore, SnapshotManager,
        action_requires_snapshot_context, reuse_unchanged_capture,
    };
    use image::{ImageBuffer, Rgba};
    use serde_json::json;
    use sky_cua_platform::backend::DesktopBackend;
    use sky_cua_platform::diagnostics::BackendError;
    use sky_cua_platform::model::{
        ActionName, ActionOutcome, ActionRequest, AgentCursorPoint, AgentCursorState, AppInfo,
        AppSelector, AppStateSnapshot, CaptureBackendKind, CaptureInfo, CaptureScreenMode,
        CoordinateSpace, ElementNode, EnvironmentInfo, InputBackendKind, ModelImageFormat,
        PixelSize, PortalCapabilities, RectF, SemanticBackendKind, ServiceRequest, ServiceResponse,
        SessionKind, ToolAvailability, ToolCapabilities,
    };
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone)]
    struct FakeBackend {
        snapshot: AppStateSnapshot,
        outcome: ActionOutcome,
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

        async fn execute_action(
            &self,
            _request: ActionRequest,
        ) -> Result<ActionOutcome, BackendError> {
            Ok(self.outcome.clone())
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
        let mut daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
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
        let mut daemon = daemon_with(
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
        let mut daemon = daemon_with(
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

    fn daemon_with(snapshot: AppStateSnapshot, outcome: ActionOutcome) -> ServiceDaemon {
        ServiceDaemon {
            backend: Box::new(FakeBackend { snapshot, outcome }),
            sessions: SessionStore::new(),
            snapshots: SnapshotManager::new(8),
            overlay: OverlayController::new_for_tests(),
            socket_path: PathBuf::from("/tmp/sky-cua-test.sock"),
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
        }
    }

    fn capture_with_rect() -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("stream".to_string()),
            source_type: Some(1),
            mapping_id: Some("mapping".to_string()),
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
}
