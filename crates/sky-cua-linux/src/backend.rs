use atspi::AccessibilityConnection;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AppSelector, AppStateSnapshot, CaptureBackendKind,
    CaptureInfo, CoordinateSpace, DiagnosticEntry, ElementNode, EnvironmentInfo, FocusedApp,
    InputBackendKind, ModelImageFormat, PixelSize, RectF, SemanticBackendKind, ToolAvailability,
    ToolCapabilities,
};
use sky_cua_platform::{AppInfo, SetValueFallbackMode, SetValueRouting, new_snapshot_id};

use crate::app_policy::{AppActionPolicies, ResolvedSetValueFallbackPolicy};
use crate::apps::discovery::{DiscoveredApp, discover_apps};
use crate::atspi::{actions as atspi_actions, connect, snapshot::snapshot_for_app};
use crate::coords::{center_of, desktop_to_stream};
use crate::env_probe::probe_environment;
use crate::focus::pick_focused_app;
use crate::kwin::{self, KWinWindowInfo};
use crate::portal::remote_desktop::{
    MouseButton, PortalLifecycleEvent, PortalTokenResetOutcome, RemoteDesktopSessionManager,
};
use crate::portal::screenshot;
use crate::x11::capture as x11_capture;
use crate::x11::input_xtest::{self, X11MouseButton};
use crate::x11::windowing::{self, X11WindowInfo};
use tokio::sync::Mutex;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct LinuxDesktopBackend {
    portal: RemoteDesktopSessionManager,
    atspi: std::sync::Arc<Mutex<Option<AccessibilityConnection>>>,
    app_policies: AppActionPolicies,
}

impl Default for LinuxDesktopBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxDesktopBackend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            portal: RemoteDesktopSessionManager::new(),
            atspi: std::sync::Arc::new(Mutex::new(None)),
            app_policies: AppActionPolicies::load_from_repo().unwrap_or_else(|error| {
                warn!(
                    message = %error,
                    "failed to load app action policies; heuristics-driven set_value fallback will stay disabled"
                );
                AppActionPolicies::default()
            }),
        }
    }

    async fn accessibility_connection(&self) -> Result<AccessibilityConnection, BackendError> {
        let mut guard = self.atspi.lock().await;
        if let Some(connection) = guard.as_ref() {
            return Ok(connection.clone());
        }

        let connection = connect().await?;
        *guard = Some(connection.clone());
        Ok(connection)
    }

    async fn reset_accessibility_connection(&self) {
        let mut guard = self.atspi.lock().await;
        *guard = None;
    }

    async fn discover_accessible_apps(
        &self,
    ) -> Result<(AccessibilityConnection, Vec<DiscoveredApp>), BackendError> {
        let connection = self.accessibility_connection().await?;
        match discover_apps(&connection).await {
            Ok(apps) => Ok((connection, apps)),
            Err(error) if is_retryable_accessibility_error(&error) => {
                self.reset_accessibility_connection().await;
                let connection = self.accessibility_connection().await?;
                let apps = discover_apps(&connection).await?;
                Ok((connection, apps))
            }
            Err(error) => Err(error),
        }
    }

    fn capabilities(environment: &EnvironmentInfo) -> ToolCapabilities {
        let semantic_ready = environment.semantic_backend == SemanticBackendKind::Atspi;
        let x11_listing_ready = environment.session_kind
            == sky_cua_platform::model::SessionKind::X11
            && windowing::x11_window_query_available();
        let kwin_listing_ready = kwin::kwin_window_query_available(environment);
        let physical_ready = environment.input_backend != InputBackendKind::None;

        ToolCapabilities {
            list_apps: ToolAvailability {
                available: semantic_ready || x11_listing_ready || kwin_listing_ready,
                reason: (!(semantic_ready || x11_listing_ready || kwin_listing_ready))
                    .then(|| "Neither AT-SPI nor a window-query fallback is available".to_string()),
            },
            get_app_state: ToolAvailability {
                available: semantic_ready || x11_listing_ready || kwin_listing_ready,
                reason: (!(semantic_ready || x11_listing_ready || kwin_listing_ready))
                    .then(|| "Neither AT-SPI nor a window-query fallback is available".to_string()),
            },
            click: ToolAvailability {
                available: semantic_ready || physical_ready,
                reason: (!(semantic_ready || physical_ready))
                    .then(|| "No semantic or physical input backend is available".to_string()),
            },
            perform_secondary_action: ToolAvailability {
                available: physical_ready || semantic_ready,
                reason: (!(physical_ready || semantic_ready))
                    .then(|| "No semantic or physical input backend is available".to_string()),
            },
            scroll: ToolAvailability {
                available: physical_ready,
                reason: (!physical_ready)
                    .then(|| "No physical input backend is available".to_string()),
            },
            drag: ToolAvailability {
                available: physical_ready,
                reason: (!physical_ready)
                    .then(|| "No physical input backend is available".to_string()),
            },
            type_text: ToolAvailability {
                available: physical_ready,
                reason: (!physical_ready)
                    .then(|| "No physical input backend is available".to_string()),
            },
            press_key: ToolAvailability {
                available: physical_ready,
                reason: (!physical_ready)
                    .then(|| "No physical input backend is available".to_string()),
            },
            set_value: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready)
                    .then(|| "AT-SPI semantic editing interfaces are unavailable".to_string()),
            },
        }
    }

    fn focused_from_app(app: &AppInfo) -> FocusedApp {
        FocusedApp {
            app_id: app.app_id.clone(),
            name: app.name.clone(),
            pid: app.pid,
            desktop_file_id: app.desktop_file_id.clone(),
            toolkit_guess: app.toolkit_guess.clone(),
            window_title: app.window_title.clone(),
        }
    }

    pub async fn reset_portal_tokens(&self) -> Result<PortalTokenResetOutcome, BackendError> {
        self.portal.reset_persisted_tokens().await
    }
}

#[async_trait::async_trait]
impl DesktopBackend for LinuxDesktopBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        let mut environment = probe_environment().await?;
        environment.semantic_backend = if self.accessibility_connection().await.is_ok() {
            SemanticBackendKind::Atspi
        } else {
            SemanticBackendKind::None
        };
        Ok(environment)
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        let environment = self.probe_environment().await?;
        let x11_windows = windowing::discover_windows().unwrap_or_default();
        let kwin_windows = kwin::discover_windows(&environment).unwrap_or_default();
        let mut atspi_apps = match self.discover_accessible_apps().await {
            Ok((_, apps)) => apps,
            Err(error) => {
                if x11_windows.is_empty() && kwin_windows.is_empty() {
                    return Err(error);
                }
                Vec::new()
            }
        };
        enrich_accessible_apps_from_x11(&mut atspi_apps, &x11_windows);
        enrich_accessible_apps_from_kwin(&mut atspi_apps, &kwin_windows);
        Ok(merge_app_lists(&atspi_apps, &x11_windows, &kwin_windows))
    }

    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
    ) -> Result<AppStateSnapshot, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        let snapshot_id = new_snapshot_id();
        let environment = self.probe_environment().await?;
        let capabilities = Self::capabilities(&environment);
        let mut diagnostics = DiagnosticBuilder::new();
        let mut portal_session_error: Option<BackendError> = None;
        let mut capture_error: Option<BackendError> = None;
        let x11_windows = windowing::discover_windows().unwrap_or_default();
        let kwin_windows = kwin::discover_windows(&environment).unwrap_or_default();

        let mut capture = (environment.capture_backend != CaptureBackendKind::None).then_some(
            sky_cua_platform::model::CaptureInfo {
                backend: environment.capture_backend.clone(),
                image_backend: None,
                coordinate_space: None,
                stream_id: None,
                source_type: None,
                mapping_id: None,
                logical_rect: None,
                pixel_size: None,
                original_pixel_size: None,
                logical_to_pixel_scale: None,
                screenshot_path: None,
                original_screenshot_path: None,
                model_image_format: None,
                model_image_quality: None,
                model_image_bytes: None,
                model_image_encode_ms: None,
            },
        );

        if environment.input_backend == InputBackendKind::PortalRemoteDesktop {
            match self.portal.ensure_started().await {
                Ok(Some(stream)) => {
                    if let Some(capture_info) = capture.as_mut() {
                        capture_info.stream_id = Some(
                            stream
                                .stream_id
                                .unwrap_or_else(|| stream.node_id.to_string()),
                        );
                        capture_info.source_type = stream.source_type;
                        capture_info.mapping_id = stream.mapping_id;
                        capture_info.logical_rect = stream.logical_rect;
                    }
                }
                Ok(None) => diagnostics.push(
                    BackendErrorCode::PortalCapabilityMissing,
                    "RemoteDesktop started without an associated screencast stream",
                    None,
                ),
                Err(error) => portal_session_error = Some(error),
            }
        }

        if environment.capture_backend == CaptureBackendKind::PortalPipeWire
            && environment.input_backend == InputBackendKind::PortalRemoteDesktop
            && portal_session_error.is_none()
        {
            match self.portal.capture_frame(&snapshot_id).await {
                Ok(frame) => {
                    if let Some(capture_info) = capture.as_mut() {
                        capture_info.image_backend = Some(CaptureBackendKind::PortalPipeWire);
                        apply_model_capture(
                            capture_info,
                            &snapshot_id,
                            &frame.path,
                            frame.pixel_size,
                        )?;
                    }
                }
                Err(error) => {
                    capture_error = Some(error);
                }
            }
        } else if environment.capture_backend == CaptureBackendKind::X11 {
            match x11_capture::capture_still(&snapshot_id).await {
                Ok(frame) => {
                    if let Some(capture_info) = capture.as_mut() {
                        capture_info.image_backend = Some(CaptureBackendKind::X11);
                        apply_model_capture(
                            capture_info,
                            &snapshot_id,
                            &frame.path,
                            frame.pixel_size,
                        )?;
                    }
                }
                Err(error) => diagnostics.push(
                    BackendErrorCode::Internal,
                    "X11 capture failed while building the app-state snapshot",
                    Some(error.message),
                ),
            }
        }

        let should_fallback_to_screenshot = capture
            .as_ref()
            .is_some_and(|capture_info| capture_info.screenshot_path.is_none())
            && environment.portal_capabilities.screenshot_version.is_some()
            && !portal_approval_pending(portal_session_error.as_ref())
            && !portal_approval_pending(capture_error.as_ref())
            && matches!(
                environment.session_kind,
                sky_cua_platform::model::SessionKind::Wayland
            );

        if should_fallback_to_screenshot {
            match screenshot::capture_still(&snapshot_id).await {
                Ok(path) => {
                    if let Some(capture_info) = capture.as_mut() {
                        capture_info.image_backend = Some(CaptureBackendKind::PortalScreenshot);
                        let original_pixel_size = screenshot::pixel_size_from_path(&path);
                        apply_model_capture(
                            capture_info,
                            &snapshot_id,
                            &path,
                            original_pixel_size,
                        )?;
                    }
                }
                Err(error) => diagnostics.push(
                    BackendErrorCode::PortalRequestDenied,
                    "Still capture fallback through the Screenshot portal failed",
                    Some(error.message),
                ),
            }
        }
        let portal_lifecycle_events = self.portal.take_lifecycle_events().await;

        let (connection, mut apps) = match self.discover_accessible_apps().await {
            Ok(result) => result,
            Err(error) => {
                diagnostics.push(
                    BackendErrorCode::AccessibilityUnavailable,
                    error.message.clone(),
                    None,
                );
                let fallback_window = selector
                    .as_ref()
                    .and_then(|selector| {
                        select_x11_window(&x11_windows, selector).map(FallbackWindow::X11)
                    })
                    .or_else(|| {
                        selector
                            .as_ref()
                            .and_then(|selector| select_kwin_window(&kwin_windows, selector))
                    })
                    .or_else(|| preferred_x11_window(&x11_windows).map(FallbackWindow::X11))
                    .or_else(|| preferred_kwin_window(&kwin_windows).map(FallbackWindow::KWin));
                push_capture_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);
                if let Some(window) = fallback_window {
                    let summary = selector_or_window_summary(selector.as_ref(), window.app());
                    match window {
                        FallbackWindow::X11(window) => {
                            diagnostics.push(
                                BackendErrorCode::AccessibilityCoverageLimited,
                                "The selected X11/XWayland window is visible through X11, but no AT-SPI application tree was available for it",
                                Some(summary),
                            );
                            return Ok(x11_fallback_snapshot(
                                snapshot_id,
                                environment,
                                capabilities,
                                capture,
                                diagnostics,
                                window,
                            ));
                        }
                        FallbackWindow::KWin(window) => {
                            diagnostics.push(
                                BackendErrorCode::AccessibilityCoverageLimited,
                                "The active Wayland window is visible through KWin, but no AT-SPI application tree was available for it",
                                Some(summary),
                            );
                            return Ok(kwin_fallback_snapshot(
                                snapshot_id,
                                environment,
                                capabilities,
                                capture,
                                diagnostics,
                                window,
                            ));
                        }
                    }
                }
                return Ok(AppStateSnapshot {
                    snapshot_id,
                    created_at: chrono::Utc::now(),
                    environment,
                    capabilities,
                    focused_app: None,
                    capture,
                    elements: Vec::new(),
                    diagnostics: diagnostics.finish(),
                    app_guidance: None,
                });
            }
        };

        enrich_accessible_apps_from_x11(&mut apps, &x11_windows);
        enrich_accessible_apps_from_kwin(&mut apps, &kwin_windows);
        if apps.is_empty() {
            diagnostics.push(
                BackendErrorCode::AccessibilityCoverageLimited,
                "AT-SPI returned no accessible applications",
                None,
            );
            push_capture_diagnostics(
                &environment,
                capture.as_ref(),
                portal_session_error.as_ref(),
                capture_error.as_ref(),
                &mut diagnostics,
            );
            push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);
            if let Some(window) = selector
                .as_ref()
                .and_then(|selector| {
                    select_x11_window(&x11_windows, selector).map(FallbackWindow::X11)
                })
                .or_else(|| {
                    selector
                        .as_ref()
                        .and_then(|selector| select_kwin_window(&kwin_windows, selector))
                })
                .or_else(|| preferred_x11_window(&x11_windows).map(FallbackWindow::X11))
                .or_else(|| preferred_kwin_window(&kwin_windows).map(FallbackWindow::KWin))
            {
                let summary = selector_or_window_summary(selector.as_ref(), window.app());
                match window {
                    FallbackWindow::X11(window) => {
                        diagnostics.push(
                            BackendErrorCode::AccessibilityCoverageLimited,
                            "The selected X11/XWayland window is visible through X11, but no accessible AT-SPI application tree matched it",
                            Some(summary),
                        );
                        return Ok(x11_fallback_snapshot(
                            snapshot_id,
                            environment,
                            capabilities,
                            capture,
                            diagnostics,
                            window,
                        ));
                    }
                    FallbackWindow::KWin(window) => {
                        diagnostics.push(
                            BackendErrorCode::AccessibilityCoverageLimited,
                            "The active Wayland window is visible through KWin, but no accessible AT-SPI application tree matched it",
                            Some(summary),
                        );
                        return Ok(kwin_fallback_snapshot(
                            snapshot_id,
                            environment,
                            capabilities,
                            capture,
                            diagnostics,
                            window,
                        ));
                    }
                }
            }
            return Ok(AppStateSnapshot {
                snapshot_id,
                created_at: chrono::Utc::now(),
                environment,
                capabilities,
                focused_app: None,
                capture,
                elements: Vec::new(),
                diagnostics: diagnostics.finish(),
                app_guidance: None,
            });
        }

        let focused = pick_focused_app(&connection, &apps)
            .await
            .unwrap_or_else(|error| {
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    error.message.clone(),
                    None,
                );
                None
            });

        let chosen_app: DiscoveredApp = if let Some(selector) = selector.as_ref() {
            if let Some(app) = select_app(&apps, selector) {
                app
            } else if let Some(window) = select_x11_window(&x11_windows, selector)
                .map(FallbackWindow::X11)
                .or_else(|| select_kwin_window(&kwin_windows, selector))
            {
                push_capture_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);
                let summary = selector_or_window_summary(Some(selector), window.app());
                match window {
                    FallbackWindow::X11(window) => {
                        diagnostics.push(
                            BackendErrorCode::AccessibilityCoverageLimited,
                            "The selected X11/XWayland window is visible through X11, but no accessible AT-SPI application tree matched it",
                            Some(summary),
                        );
                        return Ok(x11_fallback_snapshot(
                            snapshot_id,
                            environment,
                            capabilities,
                            capture,
                            diagnostics,
                            window,
                        ));
                    }
                    FallbackWindow::KWin(window) => {
                        diagnostics.push(
                            BackendErrorCode::AccessibilityCoverageLimited,
                            "The active Wayland window is visible through KWin, but no accessible AT-SPI application tree matched it",
                            Some(summary),
                        );
                        return Ok(kwin_fallback_snapshot(
                            snapshot_id,
                            environment,
                            capabilities,
                            capture,
                            diagnostics,
                            window,
                        ));
                    }
                }
            } else {
                return Err(BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "no accessible application matched selector {}",
                        selector_summary(selector)
                    ),
                ));
            }
        } else {
            if let Some(window) = preferred_x11_window(&x11_windows)
                && !apps
                    .iter()
                    .any(|app| x11_window_matches_app(&window, &app.info))
            {
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    "The focused X11/XWayland window is visible through X11, but no accessible AT-SPI application tree matched it",
                    Some(window_summary(&window.app)),
                );
                push_capture_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);
                return Ok(x11_fallback_snapshot(
                    snapshot_id,
                    environment,
                    capabilities,
                    capture,
                    diagnostics,
                    window,
                ));
            }

            if let Some(window) = preferred_kwin_window(&kwin_windows)
                && !apps
                    .iter()
                    .any(|app| kwin_window_matches_app(&window, &app.info))
            {
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    "The active Wayland window is visible through KWin, but no accessible AT-SPI application tree matched it",
                    Some(window_summary(&window.app)),
                );
                push_capture_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);
                return Ok(kwin_fallback_snapshot(
                    snapshot_id,
                    environment,
                    capabilities,
                    capture,
                    diagnostics,
                    window,
                ));
            }

            focused.unwrap_or_else(|| apps[0].clone())
        };
        let focused_app = Some(Self::focused_from_app(&chosen_app.info));

        let (elements, snapshot_diags) = snapshot_for_app(&connection, &chosen_app).await?;
        for entry in snapshot_diags {
            diagnostics.push(
                BackendErrorCode::AccessibilityCoverageLimited,
                entry.message,
                entry.details,
            );
        }

        push_capture_diagnostics(
            &environment,
            capture.as_ref(),
            portal_session_error.as_ref(),
            capture_error.as_ref(),
            &mut diagnostics,
        );
        push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);

        Ok(AppStateSnapshot {
            snapshot_id,
            created_at: chrono::Utc::now(),
            environment,
            capabilities,
            focused_app,
            capture,
            elements,
            diagnostics: diagnostics.finish(),
            app_guidance: None,
        })
    }

    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        match request.action {
            ActionName::Click => self.click(request).await,
            ActionName::PerformSecondaryAction => self.secondary_click(request).await,
            ActionName::Scroll => self.scroll(request).await,
            ActionName::Drag => self.drag(request).await,
            ActionName::TypeText => self.type_text(request).await,
            ActionName::PressKey => self.press_key(request).await,
            ActionName::SetValue => self.set_value(request).await,
        }
    }
}

fn is_retryable_accessibility_error(error: &BackendError) -> bool {
    error.code == BackendErrorCode::AccessibilityUnavailable.as_str()
        && error.message.contains("Resource temporarily unavailable")
}

impl LinuxDesktopBackend {
    async fn click(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
        {
            let connection = self.accessibility_connection().await?;
            if atspi_actions::invoke_default_action(&connection, backend_ref)
                .await
                .unwrap_or(false)
            {
                return Ok(success("Invoked the element semantically through AT-SPI."));
            }
        }

        let (x, y) = action_point(&request)?;
        match input_backend_for(&request) {
            InputBackendKind::PortalRemoteDesktop => {
                self.portal.pointer_move_absolute(x, y).await?;
                self.portal.click(MouseButton::Left).await?;
                Ok(success_with_diagnostics(
                    "Clicked the target through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                input_xtest::pointer_move_absolute(x, y)?;
                input_xtest::click(X11MouseButton::Left)?;
                Ok(success(
                    "Clicked the target through the X11 input fallback.",
                ))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for click fallback",
            )),
        }
    }

    async fn secondary_click(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
        {
            let connection = self.accessibility_connection().await?;
            if atspi_actions::invoke_secondary_action(&connection, backend_ref)
                .await
                .unwrap_or(false)
            {
                return Ok(success(
                    "Performed the secondary action semantically through AT-SPI.",
                ));
            }
        }

        let (x, y) = action_point(&request)?;
        match input_backend_for(&request) {
            InputBackendKind::PortalRemoteDesktop => {
                self.portal.pointer_move_absolute(x, y).await?;
                self.portal.click(MouseButton::Right).await?;
                Ok(success_with_diagnostics(
                    "Performed the secondary click through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                input_xtest::pointer_move_absolute(x, y)?;
                input_xtest::click(X11MouseButton::Right)?;
                Ok(success(
                    "Performed the secondary click through the X11 input fallback.",
                ))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for secondary click fallback",
            )),
        }
    }

    async fn scroll(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        if let Ok((x, y)) = action_point(&request) {
            match input_backend_for(&request) {
                InputBackendKind::PortalRemoteDesktop => {
                    self.portal.pointer_move_absolute(x, y).await?
                }
                InputBackendKind::XTest => input_xtest::pointer_move_absolute(x, y)?,
                InputBackendKind::None => {}
            }
        }

        let delta_y = request
            .arguments
            .get("delta_y")
            .and_then(serde_json::Value::as_f64);
        let steps = request
            .arguments
            .get("steps")
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(-1);

        match input_backend_for(&request) {
            InputBackendKind::PortalRemoteDesktop => {
                if let Some(delta_y) = delta_y {
                    self.portal.scroll_vertical_smooth(delta_y).await?;
                } else {
                    self.portal.scroll_vertical_discrete(steps).await?;
                }
                Ok(success_with_diagnostics(
                    "Scrolled through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                input_xtest::scroll_vertical(delta_y, Some(steps))?;
                Ok(success("Scrolled through the X11 input fallback."))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for scroll",
            )),
        }
    }

    async fn drag(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let from = drag_from_point(&request)?;
        let to = if let Some(element) = request.resolved_target_element.as_ref() {
            point_for_element(element, request.resolved_capture.as_ref())?
        } else {
            drag_to_point(&request).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "drag requires either to_element_index or explicit to_x/to_y coordinates",
                )
            })?
        };

        match input_backend_for(&request) {
            InputBackendKind::PortalRemoteDesktop => {
                self.portal.pointer_move_absolute(from.0, from.1).await?;
                self.portal.pointer_button(MouseButton::Left, true).await?;
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                self.portal.pointer_move_absolute(to.0, to.1).await?;
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                self.portal.pointer_button(MouseButton::Left, false).await?;
                Ok(success_with_diagnostics(
                    "Dragged through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                input_xtest::pointer_move_absolute(from.0, from.1)?;
                input_xtest::pointer_button(X11MouseButton::Left, true)?;
                std::thread::sleep(std::time::Duration::from_millis(40));
                input_xtest::pointer_move_absolute(to.0, to.1)?;
                std::thread::sleep(std::time::Duration::from_millis(40));
                input_xtest::pointer_button(X11MouseButton::Left, false)?;
                Ok(success("Dragged through the X11 input fallback."))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for drag",
            )),
        }
    }

    async fn type_text(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
        {
            let connection = self.accessibility_connection().await?;
            let _ = atspi_actions::grab_focus(&connection, backend_ref).await;
        }

        let text = request
            .arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "type_text requires a text argument",
                )
            })?;
        let x11_window = matched_x11_window_for_request(&request);
        match effective_keyboard_input_backend(&request, x11_window.as_ref()) {
            InputBackendKind::PortalRemoteDesktop => {
                self.portal.send_text(text).await?;
                Ok(success_with_diagnostics(
                    "Typed text through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                activate_x11_window(x11_window.as_ref())?;
                input_xtest::send_text_to_target(
                    x11_window.as_ref().map(|window| window.window_id.as_str()),
                    text,
                )?;
                Ok(success("Typed text through the X11 input fallback."))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for type_text",
            )),
        }
    }

    async fn press_key(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
        {
            let connection = self.accessibility_connection().await?;
            let _ = atspi_actions::grab_focus(&connection, backend_ref).await;
        }

        let keys = parse_key_sequence(&request.arguments).ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "press_key requires a key string or keys array",
            )
        })?;
        let x11_window = matched_x11_window_for_request(&request);
        match effective_keyboard_input_backend(&request, x11_window.as_ref()) {
            InputBackendKind::PortalRemoteDesktop => {
                self.portal.press_key_sequence(&keys).await?;
                Ok(success_with_diagnostics(
                    "Pressed the key sequence through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                activate_x11_window(x11_window.as_ref())?;
                input_xtest::press_key_sequence_to_target(
                    x11_window.as_ref().map(|window| window.window_id.as_str()),
                    &keys,
                )?;
                Ok(success(
                    "Pressed the key sequence through the X11 input fallback.",
                ))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for press_key",
            )),
        }
    }

    async fn set_value(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let element = request.resolved_element.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "set_value requires element_index so the service can resolve a semantic target",
            )
        })?;
        let backend_ref = element.backend_ref.as_deref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "set_value target did not include a backend_ref",
            )
        })?;
        let value = request
            .arguments
            .get("value")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "set_value requires a string value argument",
                )
            })?;

        let policy = self
            .app_policies
            .resolve_set_value_fallback(request.resolved_focused_app.as_ref());
        if matches!(
            policy.as_ref().map(|policy| policy.routing),
            Some(SetValueRouting::PreferPhysicalFallback)
        ) {
            return self
                .set_value_with_fallback_policy(
                    &request,
                    value,
                    policy.as_ref().expect("checked above"),
                )
                .await;
        }

        let connection = self.accessibility_connection().await?;
        let _ = atspi_actions::grab_focus(&connection, backend_ref).await;
        match atspi_actions::set_value(&connection, backend_ref, value).await {
            Ok(true) => return Ok(success("Set the value semantically through AT-SPI.")),
            Ok(false) => {}
            Err(error) if error.code == BackendErrorCode::ActionRequiresPhysicalInput.as_str() => {
                if let Some(policy) = policy.as_ref() {
                    return self
                        .set_value_with_fallback_policy(&request, value, policy)
                        .await;
                }
            }
            Err(error) => return Err(error),
        }

        Err(BackendError::new(
            BackendErrorCode::ActionRequiresPhysicalInput,
            "semantic set_value failed and no physical fallback is enabled for set_value",
        ))
    }

    async fn set_value_with_fallback_policy(
        &self,
        request: &ActionRequest,
        value: &str,
        policy: &ResolvedSetValueFallbackPolicy,
    ) -> Result<ActionOutcome, BackendError> {
        match policy.mode {
            SetValueFallbackMode::FocusClickSelectAllType => {
                let x11_window = matched_x11_window_for_request(request);
                let physical_backend =
                    effective_keyboard_input_backend(request, x11_window.as_ref());
                let (x, y) = action_point_for_backend(request, physical_backend.clone())?;
                let select_all = vec!["Ctrl".to_string(), "A".to_string()];
                let mut diagnostics = vec![DiagnosticEntry {
                    code: "HeuristicSetValueFallbackUsed".to_string(),
                    message: match policy.routing {
                        SetValueRouting::PreferSemantic =>
                            "Used a heuristics-backed physical fallback for set_value after semantic editing was unavailable"
                                .to_string(),
                        SetValueRouting::PreferPhysicalFallback =>
                            "Used a heuristics-backed physical set_value path because this app prefers keyboard-driven replacement"
                                .to_string(),
                    },
                    details: Some(format!(
                        "policy_key={} mode=focus_click_select_all_type routing={}",
                        policy.key,
                        match policy.routing {
                            SetValueRouting::PreferSemantic => "prefer_semantic",
                            SetValueRouting::PreferPhysicalFallback => "prefer_physical_fallback",
                        }
                    )),
                }];

                match physical_backend {
                    InputBackendKind::PortalRemoteDesktop => {
                        self.portal.pointer_move_absolute(x, y).await?;
                        self.portal.click(MouseButton::Left).await?;
                        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                        self.portal.press_key_sequence(&select_all).await?;
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                        self.portal.send_text(value).await?;
                        diagnostics.extend(portal_lifecycle_diagnostics(
                            &self.portal.take_lifecycle_events().await,
                        ));
                        Ok(success_with_diagnostics(
                            "Set the value through a heuristics-backed physical typing fallback.",
                            diagnostics,
                        ))
                    }
                    InputBackendKind::XTest => {
                        activate_x11_window(x11_window.as_ref())?;
                        input_xtest::pointer_move_absolute(x, y)?;
                        input_xtest::click(X11MouseButton::Left)?;
                        std::thread::sleep(std::time::Duration::from_millis(40));
                        input_xtest::press_key_sequence_to_target(
                            x11_window.as_ref().map(|window| window.window_id.as_str()),
                            &select_all,
                        )?;
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        input_xtest::send_text_to_target(
                            x11_window.as_ref().map(|window| window.window_id.as_str()),
                            value,
                        )?;
                        Ok(success_with_diagnostics(
                            "Set the value through a heuristics-backed physical typing fallback.",
                            diagnostics,
                        ))
                    }
                    InputBackendKind::None => Err(BackendError::new(
                        BackendErrorCode::ActionUnsupportedForEnvironment,
                        "heuristics allowed a physical set_value fallback, but no physical input backend is available",
                    )),
                }
            }
        }
    }
}

fn success(message: impl Into<String>) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: message.into(),
        code: "Ok".to_string(),
        diagnostics: Vec::new(),
    }
}

fn success_with_diagnostics(
    message: impl Into<String>,
    diagnostics: Vec<sky_cua_platform::model::DiagnosticEntry>,
) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: message.into(),
        code: "Ok".to_string(),
        diagnostics,
    }
}

fn input_backend_for(request: &ActionRequest) -> InputBackendKind {
    request
        .environment
        .as_ref()
        .map(|environment| environment.input_backend.clone())
        .unwrap_or(InputBackendKind::None)
}

fn effective_keyboard_input_backend(
    request: &ActionRequest,
    x11_window: Option<&X11WindowInfo>,
) -> InputBackendKind {
    let backend = input_backend_for(request);
    if backend == InputBackendKind::PortalRemoteDesktop
        && x11_window.is_some()
        && input_xtest::xtest_is_available()
    {
        return InputBackendKind::XTest;
    }
    backend
}

fn activate_x11_window(window: Option<&X11WindowInfo>) -> Result<(), BackendError> {
    if let Some(window) = window {
        input_xtest::window_activate(&window.window_id)?;
    }
    Ok(())
}

fn matched_x11_window_for_request(request: &ActionRequest) -> Option<X11WindowInfo> {
    let app = request.resolved_focused_app.as_ref()?;
    if !windowing::x11_window_query_available() {
        return None;
    }

    let windows = windowing::discover_windows().ok()?;
    let app = AppInfo {
        app_id: app.app_id.clone(),
        name: app.name.clone(),
        pid: app.pid,
        executable: None,
        desktop_file_id: app.desktop_file_id.clone(),
        toolkit_guess: app.toolkit_guess.clone(),
        window_title: app.window_title.clone(),
        is_focused_candidate: true,
    };
    best_x11_window_match(&windows, &app).cloned()
}

fn action_point_for_backend(
    request: &ActionRequest,
    backend: InputBackendKind,
) -> Result<(f64, f64), BackendError> {
    if let Some(point) = explicit_point(&request.arguments) {
        return Ok(point_from_screenshot_pixels(
            point,
            request.resolved_capture.as_ref(),
            backend,
        ));
    }
    let element = request.resolved_element.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "this action requires either explicit x/y coordinates or a resolved element target",
        )
    })?;

    let capture = match backend {
        InputBackendKind::PortalRemoteDesktop => request.resolved_capture.as_ref(),
        InputBackendKind::XTest | InputBackendKind::None => None,
    };
    point_for_element(element, capture)
}

fn action_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    action_point_for_backend(request, input_backend_for(request))
}

fn point_for_element(
    element: &ElementNode,
    capture: Option<&sky_cua_platform::model::CaptureInfo>,
) -> Result<(f64, f64), BackendError> {
    let bounds = element.bounds.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!(
                "element {} did not include bounds, so a physical action target cannot be derived",
                element.element_index
            ),
        )
    })?;
    let center = center_of(bounds);
    if let Some(logical_rect) = capture.and_then(|capture| capture.logical_rect.as_ref())
        && let Some(stream_point) = desktop_to_stream(center, logical_rect)
    {
        return Ok(stream_point);
    }
    Ok(center)
}

fn explicit_point(arguments: &serde_json::Value) -> Option<(f64, f64)> {
    point_from_fields(arguments, "x", "y")
}

fn drag_from_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    if let Some(point) = point_from_fields(&request.arguments, "from_x", "from_y")
        .or_else(|| explicit_point(&request.arguments))
    {
        return Ok(point_from_screenshot_pixels(
            point,
            request.resolved_capture.as_ref(),
            input_backend_for(request),
        ));
    }

    let element = request.resolved_element.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "drag requires either element_index or explicit from_x/from_y coordinates",
        )
    })?;
    point_for_element(element, request.resolved_capture.as_ref())
}

fn drag_to_point(request: &ActionRequest) -> Option<(f64, f64)> {
    point_from_fields(&request.arguments, "to_x", "to_y").map(|point| {
        point_from_screenshot_pixels(
            point,
            request.resolved_capture.as_ref(),
            input_backend_for(request),
        )
    })
}

fn point_from_fields(
    arguments: &serde_json::Value,
    x_field: &str,
    y_field: &str,
) -> Option<(f64, f64)> {
    let x = arguments.get(x_field).and_then(serde_json::Value::as_f64)?;
    let y = arguments.get(y_field).and_then(serde_json::Value::as_f64)?;
    Some((x, y))
}

fn apply_model_capture(
    capture_info: &mut CaptureInfo,
    snapshot_id: &str,
    raw_path: &std::path::Path,
    raw_pixel_size: Option<PixelSize>,
) -> Result<(), BackendError> {
    let model_capture = screenshot::prepare_model_capture(snapshot_id, raw_path)?;
    capture_info.coordinate_space = Some(CoordinateSpace::StreamPixels);
    capture_info.screenshot_path = Some(model_capture.path.display().to_string());
    capture_info.pixel_size = model_capture.pixel_size;
    capture_info.original_screenshot_path = model_capture
        .original_path
        .map(|path| path.display().to_string());
    capture_info.original_pixel_size = model_capture.original_pixel_size.or(raw_pixel_size);
    capture_info.model_image_format = Some(match model_capture.format {
        screenshot::ModelScreenshotFormat::Jpeg => ModelImageFormat::Jpeg,
        screenshot::ModelScreenshotFormat::Webp => ModelImageFormat::Webp,
    });
    capture_info.model_image_quality = Some(model_capture.quality);
    capture_info.model_image_bytes = model_capture.bytes;
    capture_info.model_image_encode_ms = Some(model_capture.encode_ms);
    update_model_capture_scale(capture_info);
    Ok(())
}

fn update_model_capture_scale(capture_info: &mut CaptureInfo) {
    capture_info.logical_to_pixel_scale = None;
    if let (Some(pixel_size), Some(logical_rect)) = (
        capture_info.pixel_size.as_ref(),
        capture_info.logical_rect.as_ref(),
    ) && logical_rect.width > 0.0
    {
        capture_info.logical_to_pixel_scale =
            Some(f64::from(pixel_size.width) / logical_rect.width);
    }
}

fn point_from_screenshot_pixels(
    point: (f64, f64),
    capture: Option<&CaptureInfo>,
    backend: InputBackendKind,
) -> (f64, f64) {
    let Some(capture) = capture else {
        return point;
    };
    let Some(pixel_size) = capture.pixel_size.as_ref() else {
        return point;
    };
    if pixel_size.width == 0 || pixel_size.height == 0 {
        return point;
    }

    let rel_x = point.0 / f64::from(pixel_size.width);
    let rel_y = point.1 / f64::from(pixel_size.height);

    match backend {
        InputBackendKind::PortalRemoteDesktop => {
            if let Some(logical_rect) = capture.logical_rect.as_ref()
                && logical_rect.width > 0.0
                && logical_rect.height > 0.0
            {
                return (rel_x * logical_rect.width, rel_y * logical_rect.height);
            }
            point
        }
        InputBackendKind::XTest => {
            if let Some(original_pixel_size) = capture.original_pixel_size.as_ref() {
                return (
                    rel_x * f64::from(original_pixel_size.width),
                    rel_y * f64::from(original_pixel_size.height),
                );
            }
            point
        }
        InputBackendKind::None => point,
    }
}

fn portal_lifecycle_diagnostics(
    events: &[PortalLifecycleEvent],
) -> Vec<sky_cua_platform::model::DiagnosticEntry> {
    events
        .iter()
        .map(|event| sky_cua_platform::model::DiagnosticEntry {
            code: event.code.to_string(),
            message: event.message.clone(),
            details: event.details.clone(),
        })
        .collect()
}

fn push_portal_lifecycle_diagnostics(
    events: &[PortalLifecycleEvent],
    diagnostics: &mut DiagnosticBuilder,
) {
    for event in events {
        diagnostics.push_code(event.code, event.message.clone(), event.details.clone());
    }
}

fn parse_key_sequence(arguments: &serde_json::Value) -> Option<Vec<String>> {
    if let Some(keys) = arguments.get("keys").and_then(serde_json::Value::as_array) {
        let parsed = keys
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }

    if let Some(key) = arguments.get("key").and_then(serde_json::Value::as_str) {
        let parsed = key
            .split('+')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }

    None
}

fn select_app(apps: &[DiscoveredApp], selector: &AppSelector) -> Option<DiscoveredApp> {
    apps.iter()
        .filter_map(|app| selector_match_score(&app.info, selector).map(|score| (score, app)))
        .max_by_key(|(score, app)| (*score, app.info.is_focused_candidate))
        .map(|(_, app)| app.clone())
}

#[derive(Debug, Clone)]
enum FallbackWindow {
    X11(X11WindowInfo),
    KWin(KWinWindowInfo),
}

impl FallbackWindow {
    fn app(&self) -> &AppInfo {
        match self {
            Self::X11(window) => &window.app,
            Self::KWin(window) => &window.app,
        }
    }
}

fn select_x11_window(windows: &[X11WindowInfo], selector: &AppSelector) -> Option<X11WindowInfo> {
    windows
        .iter()
        .filter_map(|window| {
            selector_match_score(&window.app, selector).map(|score| (score, window))
        })
        .max_by_key(|(score, window)| (*score, window.app.is_focused_candidate))
        .map(|(_, window)| window.clone())
}

fn select_kwin_window(
    windows: &[KWinWindowInfo],
    selector: &AppSelector,
) -> Option<FallbackWindow> {
    windows
        .iter()
        .filter_map(|window| {
            selector_match_score(&window.app, selector).map(|score| (score, window))
        })
        .max_by_key(|(score, window)| (*score, window.app.is_focused_candidate))
        .map(|(_, window)| FallbackWindow::KWin(window.clone()))
}

fn preferred_x11_window(windows: &[X11WindowInfo]) -> Option<X11WindowInfo> {
    windows
        .iter()
        .find(|window| window.app.is_focused_candidate)
        .cloned()
        .or_else(|| windows.first().cloned())
}

fn preferred_kwin_window(windows: &[KWinWindowInfo]) -> Option<KWinWindowInfo> {
    windows
        .iter()
        .find(|window| window.app.is_focused_candidate)
        .cloned()
        .or_else(|| windows.first().cloned())
}

fn enrich_accessible_apps_from_x11(apps: &mut [DiscoveredApp], x11_windows: &[X11WindowInfo]) {
    for app in apps {
        let Some(window) = best_x11_window_match(x11_windows, &app.info) else {
            continue;
        };

        if app.info.pid.is_none() {
            app.info.pid = window.app.pid;
        }
        if app.info.executable.is_none() {
            app.info.executable = window.app.executable.clone();
        }
        if app.info.desktop_file_id.is_none() {
            app.info.desktop_file_id = window.app.desktop_file_id.clone();
        }
        if app.info.toolkit_guess.is_none() {
            app.info.toolkit_guess = window.app.toolkit_guess.clone();
        }
        if app.info.window_title.is_none() {
            app.info.window_title = window.app.window_title.clone();
        }
        if app.info.name.eq_ignore_ascii_case("Unnamed") {
            app.info.name = window.app.name.clone();
        }
        if !app.info.is_focused_candidate && window.app.is_focused_candidate {
            app.info.is_focused_candidate = true;
        }
    }
}

fn enrich_accessible_apps_from_kwin(apps: &mut [DiscoveredApp], windows: &[KWinWindowInfo]) {
    for app in apps {
        let Some(window) = best_kwin_window_match(windows, &app.info) else {
            continue;
        };
        if app.info.desktop_file_id.is_none() {
            app.info.desktop_file_id = window.app.desktop_file_id.clone();
        }
        if app.info.window_title.is_none() {
            app.info.window_title = window.app.window_title.clone();
        }
        if app.info.executable.is_none() {
            app.info.executable = window.app.executable.clone();
        }
        if app.info.toolkit_guess.is_none() {
            app.info.toolkit_guess = window.app.toolkit_guess.clone();
        }
        if !app.info.is_focused_candidate && window.app.is_focused_candidate {
            app.info.is_focused_candidate = true;
        }
    }
}

fn merge_app_lists(
    apps: &[DiscoveredApp],
    x11_windows: &[X11WindowInfo],
    kwin_windows: &[KWinWindowInfo],
) -> Vec<AppInfo> {
    let mut merged = apps.iter().map(|app| app.info.clone()).collect::<Vec<_>>();
    for window in x11_windows {
        if !merged.iter().any(|app| x11_window_matches_app(window, app)) {
            merged.push(window.app.clone());
        }
    }
    for window in kwin_windows {
        if !merged
            .iter()
            .any(|app| kwin_window_matches_app(window, app))
        {
            merged.push(window.app.clone());
        }
    }
    merged
}

fn best_kwin_window_match<'a>(
    windows: &'a [KWinWindowInfo],
    app: &AppInfo,
) -> Option<&'a KWinWindowInfo> {
    windows
        .iter()
        .filter_map(|window| kwin_match_score(window, app).map(|score| (score, window)))
        .max_by_key(|(score, window)| (*score, window.app.is_focused_candidate))
        .map(|(_, window)| window)
}

fn kwin_window_matches_app(window: &KWinWindowInfo, app: &AppInfo) -> bool {
    kwin_match_score(window, app).is_some()
}

fn kwin_match_score(window: &KWinWindowInfo, app: &AppInfo) -> Option<i32> {
    if app.app_id == window.app.app_id {
        return Some(1_000);
    }

    let mut score = 0i32;
    let mut identity_signals = 0u8;

    let window_title = window
        .app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let app_title = app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let window_name = normalize_match_key(&window.app.name);
    let app_name = normalize_match_key(&app.name);
    let window_executable = window.app.executable.as_deref().map(normalize_match_key);
    let app_executable = app.executable.as_deref().map(normalize_match_key);
    let window_desktop = window
        .app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let app_desktop = app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let window_resource_name = window.resource_name.as_deref().map(normalize_match_key);
    let window_resource_class = window.resource_class.as_deref().map(normalize_match_key);

    if !window_title.is_empty()
        && !app_title.is_empty()
        && normalize_match_key(&window_title) == normalize_match_key(&app_title)
    {
        score += 400;
    }

    if window_desktop.is_some() && window_desktop == app_desktop {
        score += 240;
        identity_signals += 1;
    }

    if window_executable.is_some() && window_executable == app_executable {
        score += 220;
        identity_signals += 1;
    }

    if window_name == app_name {
        score += 180;
        identity_signals += 1;
    }

    if window_resource_name.as_ref().is_some_and(|resource| {
        resource == &app_name
            || app_executable
                .as_ref()
                .is_some_and(|executable| executable == resource)
            || app_desktop
                .as_ref()
                .is_some_and(|desktop| desktop == resource)
    }) {
        score += 170;
        identity_signals += 1;
    }

    if window_resource_class.as_ref().is_some_and(|resource| {
        resource == &app_name
            || app_executable
                .as_ref()
                .is_some_and(|executable| executable == resource)
            || app_desktop
                .as_ref()
                .is_some_and(|desktop| desktop == resource)
    }) {
        score += 170;
        identity_signals += 1;
    }

    if !window_title.is_empty()
        && !app_title.is_empty()
        && (window_title.contains(&app_title) || app_title.contains(&window_title))
    {
        score += 120;
    }

    if window.app.is_focused_candidate {
        score += 5;
    }

    (identity_signals > 0 && score > 0).then_some(score)
}

fn x11_window_matches_app(window: &X11WindowInfo, app: &AppInfo) -> bool {
    x11_match_score(window, app).is_some()
}

fn best_x11_window_match<'a>(
    windows: &'a [X11WindowInfo],
    app: &AppInfo,
) -> Option<&'a X11WindowInfo> {
    windows
        .iter()
        .filter_map(|window| x11_match_score(window, app).map(|score| (score, window)))
        .max_by_key(|(score, window)| (*score, window.app.is_focused_candidate))
        .map(|(_, window)| window)
}

fn x11_match_score(window: &X11WindowInfo, app: &AppInfo) -> Option<i32> {
    if app.app_id == window.app.app_id {
        return Some(1_000);
    }

    if let (Some(window_pid), Some(app_pid)) = (window.app.pid, app.pid)
        && window_pid == app_pid
    {
        return Some(900);
    }

    let window_title = window
        .app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let app_title = app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let window_name = normalize_match_key(&window.app.name);
    let app_name = normalize_match_key(&app.name);
    let window_executable = window.app.executable.as_deref().map(normalize_match_key);
    let app_executable = app.executable.as_deref().map(normalize_match_key);
    let window_desktop = window
        .app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let app_desktop = app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let window_instance = window.instance_name.as_deref().map(normalize_match_key);
    let window_class = window.class_name.as_deref().map(normalize_match_key);

    let mut score = 0i32;
    let mut identity_signals = 0u8;
    if let (Some(window_title), Some(app_title)) =
        (window.app.window_title.as_ref(), app.window_title.as_ref())
        && normalize_match_key(window_title) == normalize_match_key(app_title)
    {
        score += 400;
    }

    if let (Some(window_desktop_file_id), Some(app_desktop_file_id)) = (
        window.app.desktop_file_id.as_ref(),
        app.desktop_file_id.as_ref(),
    ) && normalize_match_key(window_desktop_file_id) == normalize_match_key(app_desktop_file_id)
        && normalize_match_key(&window.app.name) == normalize_match_key(&app.name)
    {
        score += 260;
        identity_signals += 1;
    }

    if window_desktop.is_some() && window_desktop == app_desktop {
        score += 240;
        identity_signals += 1;
    }

    if window_executable.is_some() && window_executable == app_executable {
        score += 220;
        identity_signals += 1;
    }

    if window_name == app_name {
        score += 180;
        identity_signals += 1;
    }

    if window_instance.as_ref().is_some_and(|instance| {
        instance == &app_name
            || app_executable
                .as_ref()
                .is_some_and(|executable| executable == instance)
            || app_desktop
                .as_ref()
                .is_some_and(|desktop| desktop == instance)
    }) {
        score += 170;
        identity_signals += 1;
    }

    if window_class.as_ref().is_some_and(|class_name| {
        class_name == &app_name
            || app_executable
                .as_ref()
                .is_some_and(|executable| executable == class_name)
            || app_desktop
                .as_ref()
                .is_some_and(|desktop| desktop == class_name)
    }) {
        score += 170;
        identity_signals += 1;
    }

    if !window_title.is_empty()
        && !app_title.is_empty()
        && (window_title.contains(&app_title) || app_title.contains(&window_title))
    {
        score += 120;
    }

    if window.app.is_focused_candidate {
        score += 5;
    }

    if identity_signals == 0 {
        return None;
    }

    if obvious_service_app(app) {
        score -= 40;
    }

    (score > 0).then_some(score)
}

fn obvious_service_app(app: &AppInfo) -> bool {
    [
        app.executable.as_deref(),
        app.desktop_file_id.as_deref(),
        Some(app.name.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(normalize_match_key)
    .any(|value| {
        [
            "service",
            "proxy",
            "menu",
            "portal",
            "daemon",
            "ksmserver",
            "kaccess",
            "kglobalaccel",
            "kded",
            "xembedsniproxy",
            "gmenudbusmenuproxy",
        ]
        .into_iter()
        .any(|needle| value.contains(needle))
    })
}

fn normalize_match_key(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_desktop_id_stem(value: &str) -> String {
    normalize_match_key(value.trim_end_matches(".desktop"))
}

fn push_capture_diagnostics(
    environment: &EnvironmentInfo,
    capture: Option<&sky_cua_platform::model::CaptureInfo>,
    portal_session_error: Option<&BackendError>,
    capture_error: Option<&BackendError>,
    diagnostics: &mut DiagnosticBuilder,
) {
    if let Some(error) = portal_session_error {
        if portal_approval_pending(Some(error)) {
            diagnostics.push_code(
                error.code,
                "Waiting on portal approval before live screen control can start",
                Some(error.message.clone()),
            );
        } else {
            diagnostics.push_code(
                error.code,
                "Combined RemoteDesktop and ScreenCast session could not be started",
                Some(error.message.clone()),
            );
        }
    }

    if let Some(error) = capture_error {
        if portal_approval_pending(Some(error)) {
            diagnostics.push_code(
                error.code,
                "Waiting on portal approval before a live frame can be captured for this snapshot",
                Some(error.message.clone()),
            );
            return;
        }
        let used_screenshot_fallback = capture.is_some_and(|capture_info| {
            capture_info.image_backend == Some(CaptureBackendKind::PortalScreenshot)
        });
        diagnostics.push(
            BackendErrorCode::PipeWireStreamFailed,
            if used_screenshot_fallback {
                "Live PipeWire frame capture failed before the snapshot image was produced"
            } else {
                "Live PipeWire frame capture failed and no fallback image was produced"
            },
            Some(error.message.clone()),
        );
        if used_screenshot_fallback {
            diagnostics.push(
                BackendErrorCode::CaptureBackendDowngraded,
                "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback",
                Some(
                    "primary_backend=portal_pipe_wire image_backend=portal_screenshot".to_string(),
                ),
            );
        }
    } else if portal_session_error.is_none()
        && environment.capture_backend == CaptureBackendKind::PortalPipeWire
    {
        let image_backend = capture.and_then(|capture_info| capture_info.image_backend.as_ref());
        if image_backend == Some(&CaptureBackendKind::PortalScreenshot) {
            diagnostics.push(
                BackendErrorCode::PipeWireStreamFailed,
                "Live PipeWire frame capture did not produce the snapshot image",
                Some("no PipeWire frame image was available for this snapshot".to_string()),
            );
            diagnostics.push(
                BackendErrorCode::CaptureBackendDowngraded,
                "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback",
                Some(
                    "primary_backend=portal_pipe_wire image_backend=portal_screenshot".to_string(),
                ),
            );
        } else if capture.is_some_and(|capture_info| capture_info.screenshot_path.is_none()) {
            diagnostics.push(
                BackendErrorCode::PipeWireStreamFailed,
                "ScreenCast metadata is live, but no frame image could be produced for this snapshot",
                None,
            );
        }
    }
}

fn portal_approval_pending(error: Option<&BackendError>) -> bool {
    error.is_some_and(|error| error.code == BackendErrorCode::PortalApprovalPending.as_str())
}

fn x11_fallback_snapshot(
    snapshot_id: String,
    environment: EnvironmentInfo,
    capabilities: ToolCapabilities,
    capture: Option<sky_cua_platform::model::CaptureInfo>,
    diagnostics: DiagnosticBuilder,
    window: X11WindowInfo,
) -> AppStateSnapshot {
    let elements = x11_window_elements(&window);
    AppStateSnapshot {
        snapshot_id,
        created_at: chrono::Utc::now(),
        environment,
        capabilities,
        focused_app: Some(LinuxDesktopBackend::focused_from_app(&window.app)),
        capture,
        elements,
        diagnostics: diagnostics.finish(),
        app_guidance: None,
    }
}

fn kwin_fallback_snapshot(
    snapshot_id: String,
    environment: EnvironmentInfo,
    capabilities: ToolCapabilities,
    capture: Option<sky_cua_platform::model::CaptureInfo>,
    diagnostics: DiagnosticBuilder,
    window: KWinWindowInfo,
) -> AppStateSnapshot {
    AppStateSnapshot {
        snapshot_id,
        created_at: chrono::Utc::now(),
        environment,
        capabilities,
        focused_app: Some(LinuxDesktopBackend::focused_from_app(&window.app)),
        capture,
        elements: kwin_window_elements(&window),
        diagnostics: diagnostics.finish(),
        app_guidance: None,
    }
}

fn kwin_window_elements(window: &KWinWindowInfo) -> Vec<ElementNode> {
    let Some(bounds) = window.bounds.clone() else {
        return Vec::new();
    };

    let mut root_state_flags = vec![
        "kwin_fallback".to_string(),
        "physical_target".to_string(),
        "vision_anchor".to_string(),
        "container".to_string(),
        "content_like".to_string(),
    ];
    if window.app.is_focused_candidate {
        root_state_flags.push("focused".to_string());
        root_state_flags.push("active".to_string());
    }

    let mut elements = vec![ElementNode {
        element_index: 0,
        parent_index: None,
        role: "window".to_string(),
        name: window
            .app
            .window_title
            .clone()
            .or_else(|| Some(window.app.name.clone())),
        description: Some(
            "Wayland window surfaced from KWin without a matching AT-SPI tree. The child regions below are geometric anchors only: use them to narrow the search space, then confirm the real target on the screenshot before clicking, dragging, or typing."
                .to_string(),
        ),
        value: None,
        state_flags: root_state_flags,
        semantic_actions: Vec::new(),
        bounds: Some(bounds.clone()),
        backend_ref: None,
    }];

    if bounds.width < 220.0 || bounds.height < 180.0 {
        return elements;
    }

    let top_band_height = (bounds.height * 0.13).clamp(44.0, 96.0);
    let content_y = bounds.y + top_band_height;
    let content_height = (bounds.height - top_band_height).max(40.0);
    let sidebar_width = if bounds.width >= 520.0 {
        (bounds.width * 0.23).clamp(140.0, 320.0)
    } else {
        0.0
    };
    let space = bounds.space.clone();
    let main_x = bounds.x + sidebar_width;
    let main_width = (bounds.width - sidebar_width).max(120.0);
    let header_bounds = RectF {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: top_band_height,
        space: space.clone(),
    };
    let search_width = (bounds.width * 0.38)
        .clamp(180.0, 480.0)
        .min(bounds.width - 32.0);
    let search_height = (top_band_height * 0.62).clamp(28.0, 52.0);
    let search_bounds = RectF {
        x: bounds.x + ((bounds.width - search_width) / 2.0),
        y: bounds.y + ((top_band_height - search_height) / 2.0),
        width: search_width,
        height: search_height,
        space: space.clone(),
    };
    let toolbar_width = (bounds.width * 0.18).clamp(120.0, 260.0);
    let toolbar_bounds = RectF {
        x: bounds.x + bounds.width - toolbar_width - 12.0,
        y: bounds.y,
        width: toolbar_width,
        height: top_band_height,
        space: space.clone(),
    };

    push_kwin_anchor(
        &mut elements,
        0,
        "wayland_header_band",
        Some("Top band".to_string()),
        "Heuristic top band derived from the KWin window bounds. It often contains app navigation, tabs, or a search bar, but it is not a semantic tree node. Use the screenshot to verify the real control before acting.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "container",
            "action_like",
        ],
        header_bounds,
    );

    push_kwin_anchor(
        &mut elements,
        1,
        "wayland_search_candidate",
        Some("Search candidate".to_string()),
        "Heuristic search/text-entry candidate carved out of the top band. Treat it as a likely text-target anchor only; confirm the visible search field on the screenshot before clicking or typing.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "leaf",
            "search_like",
            "text_like",
        ],
        search_bounds,
    );

    push_kwin_anchor(
        &mut elements,
        1,
        "wayland_toolbar_candidate",
        Some("Action strip candidate".to_string()),
        "Heuristic action strip on the right side of the top band. It often contains buttons or profile controls, but the screenshot must confirm the actual target.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "leaf",
            "action_like",
        ],
        toolbar_bounds,
    );

    if sidebar_width > 0.0 {
        let sidebar_bounds = RectF {
            x: bounds.x,
            y: content_y,
            width: sidebar_width,
            height: content_height,
            space: space.clone(),
        };
        let sidebar_index = push_kwin_anchor(
            &mut elements,
            0,
            "wayland_sidebar_region",
            Some("Sidebar candidate".to_string()),
            "Heuristic left-side navigation rail derived from the window geometry. It is useful for orienting around libraries, playlists, or side panels, but the screenshot should confirm the visible list or rail before interaction.",
            vec![
                "kwin_fallback",
                "physical_target",
                "vision_anchor",
                "container",
                "navigation_like",
                "list_like",
            ],
            sidebar_bounds.clone(),
        );
        let sidebar_list_height =
            (sidebar_bounds.height * 0.72).clamp(120.0, sidebar_bounds.height);
        let sidebar_list_bounds = RectF {
            x: sidebar_bounds.x,
            y: sidebar_bounds.y + 8.0,
            width: sidebar_bounds.width,
            height: sidebar_list_height - 8.0,
            space: space.clone(),
        };
        let sidebar_list_index = push_kwin_anchor(
            &mut elements,
            sidebar_index,
            "wayland_list_candidate",
            Some("Sidebar list candidate".to_string()),
            "Heuristic list-like region inside the sidebar. Use it as a structural hint for playlist or library rails, then verify the actual rows on the screenshot before clicking or scrolling.",
            vec![
                "kwin_fallback",
                "physical_target",
                "vision_anchor",
                "leaf",
                "list_like",
                "navigation_like",
            ],
            sidebar_list_bounds.clone(),
        );
        let sidebar_row_band_count =
            ((sidebar_list_bounds.height / 96.0).floor() as usize).clamp(2, 5);
        let sidebar_row_band_height = (sidebar_list_bounds.height * 0.16)
            .clamp(46.0, 104.0)
            .min((sidebar_list_bounds.height - 12.0).max(40.0));
        let sidebar_row_band_gap = ((sidebar_list_bounds.height
            - sidebar_row_band_height * sidebar_row_band_count as f64)
            / (sidebar_row_band_count as f64 + 1.0))
            .clamp(6.0, 28.0);
        for band in 0..sidebar_row_band_count {
            let row_y = sidebar_list_bounds.y
                + sidebar_row_band_gap
                + band as f64 * (sidebar_row_band_height + sidebar_row_band_gap);
            push_kwin_anchor(
                &mut elements,
                sidebar_list_index,
                "wayland_row_band_candidate",
                Some(format!("Sidebar row band candidate {}", band + 1)),
                "Heuristic visible row band inside the sidebar list. It often contains playlist or library rows with text and context actions. Verify the row text on the screenshot before clicking, right-clicking, or dragging.",
                vec![
                    "kwin_fallback",
                    "physical_target",
                    "vision_anchor",
                    "leaf",
                    "list_like",
                    "row_like",
                    "text_like",
                ],
                RectF {
                    x: sidebar_list_bounds.x + 4.0,
                    y: row_y,
                    width: (sidebar_list_bounds.width - 8.0).max(40.0),
                    height: sidebar_row_band_height,
                    space: sidebar_list_bounds.space.clone(),
                },
            );
        }
    }

    let main_bounds = RectF {
        x: main_x,
        y: content_y,
        width: main_width,
        height: content_height,
        space: space.clone(),
    };
    let main_index = push_kwin_anchor(
        &mut elements,
        0,
        "wayland_main_region",
        Some("Main content candidate".to_string()),
        "Heuristic main content region. This usually contains the primary page or detail view. Use it to orient the screenshot search, not as a promise about semantics.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "container",
            "content_like",
        ],
        main_bounds.clone(),
    );
    let list_candidate_height = (main_bounds.height * 0.68).clamp(140.0, main_bounds.height);
    let main_list_bounds = RectF {
        x: main_bounds.x + 8.0,
        y: main_bounds.y + 8.0,
        width: (main_bounds.width - 16.0).max(40.0),
        height: (list_candidate_height - 8.0).max(40.0),
        space,
    };
    let main_list_index = push_kwin_anchor(
        &mut elements,
        main_index,
        "wayland_list_candidate",
        Some("Main list candidate".to_string()),
        "Heuristic list/grid region inside the main content area. This is often where search results, playlists, or tracks appear. Confirm the visible rows or tiles on the screenshot before clicking, scrolling, or dragging.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "leaf",
            "list_like",
            "content_like",
        ],
        main_list_bounds.clone(),
    );
    let main_row_band_count = ((main_list_bounds.height / 108.0).floor() as usize).clamp(2, 5);
    let visible_row_band_height = (main_list_bounds.height * 0.11)
        .clamp(44.0, 104.0)
        .min((main_list_bounds.height - 12.0).max(40.0));
    let row_band_width = (main_list_bounds.width - 8.0).max(56.0);
    let main_row_band_gap = ((main_list_bounds.height
        - visible_row_band_height * main_row_band_count as f64)
        / (main_row_band_count as f64 + 1.0))
        .clamp(8.0, 32.0);
    for band in 0..main_row_band_count {
        let row_y = main_list_bounds.y
            + main_row_band_gap
            + band as f64 * (visible_row_band_height + main_row_band_gap);
        push_kwin_anchor(
            &mut elements,
            main_list_index,
            "wayland_row_band_candidate",
            Some(format!("Main row band candidate {}", band + 1)),
            "Heuristic visible row band inside the main list/grid area. It often contains track or result rows with text and row-level actions. Use the screenshot to confirm the real row before clicking or opening a context menu.",
            vec![
                "kwin_fallback",
                "physical_target",
                "vision_anchor",
                "leaf",
                "list_like",
                "row_like",
                "text_like",
            ],
            RectF {
                x: main_list_bounds.x,
                y: row_y,
                width: row_band_width,
                height: visible_row_band_height,
                space: main_bounds.space.clone(),
            },
        );
    }
    let action_cluster_width = (main_bounds.width * 0.18).clamp(64.0, 180.0);
    let action_cluster_height = (visible_row_band_height * 1.25).clamp(44.0, 120.0);
    for band in 0..main_row_band_count {
        let row_y = main_list_bounds.y
            + main_row_band_gap
            + band as f64 * (visible_row_band_height + main_row_band_gap);
        push_kwin_anchor(
            &mut elements,
            main_list_index,
            "wayland_action_cluster_candidate",
            Some(format!("Main action cluster candidate {}", band + 1)),
            "Heuristic right-edge action cluster inside a visible row area. This often lines up with overflow buttons, kebab menus, or row-level actions. Confirm the visible affordance on the screenshot before clicking or right-clicking.",
            vec![
                "kwin_fallback",
                "physical_target",
                "vision_anchor",
                "leaf",
                "action_like",
                "menu_like",
            ],
            RectF {
                x: (main_bounds.x + main_bounds.width - action_cluster_width - 12.0)
                    .max(main_bounds.x + 8.0),
                y: (row_y - 2.0).max(main_list_bounds.y),
                width: action_cluster_width,
                height: action_cluster_height
                    .min((main_list_bounds.y + main_list_bounds.height - row_y).max(32.0)),
                space: main_bounds.space.clone(),
            },
        );
    }

    elements
}

fn push_kwin_anchor(
    elements: &mut Vec<ElementNode>,
    parent_index: usize,
    role: &str,
    name: Option<String>,
    description: &str,
    state_flags: Vec<&str>,
    bounds: RectF,
) -> usize {
    let element_index = elements.len();
    elements.push(ElementNode {
        element_index,
        parent_index: Some(parent_index),
        role: role.to_string(),
        name,
        description: Some(description.to_string()),
        value: None,
        state_flags: state_flags
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect(),
        semantic_actions: Vec::new(),
        bounds: Some(bounds),
        backend_ref: None,
    });
    element_index
}

fn x11_window_elements(window: &X11WindowInfo) -> Vec<ElementNode> {
    let Some(bounds) = window.bounds.clone() else {
        return Vec::new();
    };

    let mut state_flags = Vec::new();
    if window.app.is_focused_candidate {
        state_flags.push("focused".to_string());
        state_flags.push("active".to_string());
    }

    let mut elements = vec![ElementNode {
        element_index: 0,
        parent_index: None,
        role: "window".to_string(),
        name: window
            .app
            .window_title
            .clone()
            .or_else(|| Some(window.app.name.clone())),
        description: Some(
            "X11/XWayland window surfaced without a matching AT-SPI tree; physical actions can still target its bounds"
                .to_string(),
        ),
        value: None,
        state_flags,
        semantic_actions: Vec::new(),
        bounds: Some(bounds.clone()),
        backend_ref: None,
    }];

    let child_counts = window.child_regions.iter().fold(
        std::collections::HashMap::<String, usize>::new(),
        |mut counts, region| {
            if let Some(parent_window_id) = region.parent_window_id.as_ref() {
                *counts.entry(parent_window_id.clone()).or_default() += 1;
            }
            counts
        },
    );
    let mut index_by_window_id =
        std::collections::HashMap::from([(window.window_id.clone(), 0usize)]);
    for region in &window.child_regions {
        if region.bounds.width < 8.0 || region.bounds.height < 8.0 {
            continue;
        }

        let parent_index = region
            .parent_window_id
            .as_ref()
            .and_then(|window_id| index_by_window_id.get(window_id).copied())
            .or(Some(0));
        let element_index = elements.len();
        let has_children = child_counts
            .get(&region.window_id)
            .copied()
            .unwrap_or_default()
            > 0;
        let role = x11_region_role(region, has_children, &bounds);
        let mut state_flags = vec!["x11_fallback".to_string(), "physical_target".to_string()];
        if has_children {
            state_flags.push("container".to_string());
        } else {
            state_flags.push("leaf".to_string());
        }
        if role == "x11_action_region" {
            state_flags.push("action_like".to_string());
        }
        elements.push(ElementNode {
            element_index,
            parent_index,
            role: role.to_string(),
            name: region.name.clone(),
            description: Some(x11_region_description(region, role)),
            value: None,
            state_flags,
            semantic_actions: Vec::new(),
            bounds: Some(region.bounds.clone()),
            backend_ref: None,
        });
        index_by_window_id.insert(region.window_id.clone(), element_index);
    }

    elements
}

fn x11_region_role(
    region: &crate::x11::windowing::X11WindowRegion,
    has_children: bool,
    root_bounds: &sky_cua_platform::model::RectF,
) -> &'static str {
    if has_children {
        return "x11_container";
    }

    let center_y = region.bounds.y + (region.bounds.height / 2.0);
    let root_mid_y = root_bounds.y + (root_bounds.height / 2.0);
    let small_relative_width = region.bounds.width <= root_bounds.width * 0.4;
    let small_relative_height = region.bounds.height <= root_bounds.height * 0.5;
    if center_y >= root_mid_y && small_relative_width && small_relative_height {
        "x11_action_region"
    } else {
        "x11_leaf_region"
    }
}

fn x11_region_description(region: &crate::x11::windowing::X11WindowRegion, role: &str) -> String {
    let role_hint = match role {
        "x11_container" => "container-like region",
        "x11_action_region" => "small lower leaf region that may behave like an actionable control",
        _ => "leaf region",
    };
    format!(
        "Recovered from the X11 window tree at depth {} as a {}; physical actions can target this region, but no semantic AT-SPI interface is available",
        region.depth, role_hint
    )
}

fn window_summary(app: &AppInfo) -> String {
    selector_summary(&AppSelector {
        app_id: Some(app.app_id.clone()),
        desktop_file_id: app.desktop_file_id.clone(),
        window_title: app.window_title.clone(),
        name: Some(app.name.clone()),
    })
}

fn selector_or_window_summary(selector: Option<&AppSelector>, app: &AppInfo) -> String {
    match selector {
        Some(selector) => format!(
            "{}, matched_x11_window={}",
            selector_summary(selector),
            window_summary(app)
        ),
        None => window_summary(app),
    }
}

fn matches_selector(app: &AppInfo, selector: &AppSelector) -> bool {
    selector
        .app_id
        .as_ref()
        .is_none_or(|wanted| &app.app_id == wanted)
        && selector
            .desktop_file_id
            .as_ref()
            .is_none_or(|wanted| app.desktop_file_id.as_ref() == Some(wanted))
        && selector.window_title.as_ref().is_none_or(|wanted| {
            app.window_title.as_ref().is_some_and(|title| {
                title
                    .to_ascii_lowercase()
                    .contains(&wanted.to_ascii_lowercase())
            })
        })
        && selector.name.as_ref().is_none_or(|wanted| {
            app.name
                .to_ascii_lowercase()
                .contains(&wanted.to_ascii_lowercase())
        })
}

fn selector_match_score(app: &AppInfo, selector: &AppSelector) -> Option<i32> {
    if !matches_selector(app, selector) {
        return None;
    }

    let mut score = 0i32;

    if let Some(app_id) = selector.app_id.as_ref()
        && &app.app_id == app_id
    {
        score += 10_000;
    }

    if let Some(desktop_file_id) = selector.desktop_file_id.as_ref()
        && app.desktop_file_id.as_ref() == Some(desktop_file_id)
    {
        score += 2_000;
    }

    if let Some(window_title) = selector.window_title.as_ref() {
        let wanted = normalize_match_key(window_title);
        let actual = app
            .window_title
            .as_deref()
            .map(normalize_match_key)
            .unwrap_or_default();
        if actual == wanted {
            score += 1_500;
        } else if actual.contains(&wanted) {
            score += 800;
        }
    }

    if let Some(name) = selector.name.as_ref() {
        let wanted = normalize_match_key(name);
        let actual = normalize_match_key(&app.name);
        if actual == wanted {
            score += 1_000;
        } else if actual.contains(&wanted) {
            score += 500;
        }
    }

    if app.is_focused_candidate {
        score += 25;
    }

    if app
        .window_title
        .as_ref()
        .is_some_and(|title| !title.trim().is_empty())
    {
        score += 5;
    }

    Some(score)
}

fn selector_summary(selector: &AppSelector) -> String {
    let mut parts = Vec::new();
    if let Some(app_id) = selector.app_id.as_ref() {
        parts.push(format!("app_id={app_id}"));
    }
    if let Some(desktop_file_id) = selector.desktop_file_id.as_ref() {
        parts.push(format!("desktop_file_id={desktop_file_id}"));
    }
    if let Some(window_title) = selector.window_title.as_ref() {
        parts.push(format!("window_title={window_title}"));
    }
    if let Some(name) = selector.name.as_ref() {
        parts.push(format!("name={name}"));
    }
    if parts.is_empty() {
        "<empty selector>".to_string()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        AppInfo, AppSelector, best_x11_window_match, drag_to_point, explicit_point,
        kwin_window_elements, matches_selector, parse_key_sequence, point_from_screenshot_pixels,
        push_capture_diagnostics, select_x11_window, selector_summary, x11_window_elements,
        x11_window_matches_app,
    };
    use crate::kwin::KWinWindowInfo;
    use crate::x11::windowing::{X11WindowInfo, X11WindowRegion};
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
    use sky_cua_platform::model::{
        ActionName, ActionRequest, CaptureBackendKind, CaptureInfo, CoordinateSpace,
        EnvironmentInfo, InputBackendKind, ModelImageFormat, PixelSize, PortalCapabilities, RectF,
        SemanticBackendKind, SessionKind,
    };

    fn wayland_pipewire_environment() -> EnvironmentInfo {
        EnvironmentInfo {
            session_kind: SessionKind::Wayland,
            compositor: Some("kde-kwin-wayland".to_string()),
            desktop_environment: Some("KDE".to_string()),
            capture_backend: CaptureBackendKind::PortalPipeWire,
            input_backend: InputBackendKind::PortalRemoteDesktop,
            semantic_backend: SemanticBackendKind::Atspi,
            portal_capabilities: PortalCapabilities {
                screencast_version: Some(5),
                remote_desktop_version: Some(2),
                screenshot_version: Some(2),
                available_source_types: None,
                available_cursor_modes: None,
                available_device_types: None,
            },
            xdg_session_type: Some("wayland".to_string()),
            display: None,
            wayland_display: Some("wayland-0".to_string()),
        }
    }

    #[test]
    fn parses_key_chord_string() {
        assert_eq!(
            parse_key_sequence(&json!({"key": "Ctrl+L"})),
            Some(vec!["Ctrl".to_string(), "L".to_string()])
        );
    }

    #[test]
    fn matches_app_selector_by_window_title() {
        let app = AppInfo {
            app_id: "app-1".to_string(),
            name: "zenity".to_string(),
            pid: Some(123),
            executable: Some("zenity".to_string()),
            desktop_file_id: Some("zenity.desktop".to_string()),
            toolkit_guess: Some("GTK".to_string()),
            window_title: Some("sky-cua zenity smoke".to_string()),
            is_focused_candidate: false,
        };
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: Some("zenity.desktop".to_string()),
            window_title: Some("zenity smoke".to_string()),
            name: None,
        };
        assert!(matches_selector(&app, &selector));
    }

    #[test]
    fn parses_explicit_drag_destination_coordinates() {
        let request = ActionRequest {
            action: ActionName::Drag,
            snapshot_id: None,
            element_index: None,
            arguments: json!({"to_x": 320.0, "to_y": 240.0}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: None,
        };
        assert_eq!(drag_to_point(&request), Some((320.0, 240.0)));
        let request_without_to = ActionRequest {
            arguments: json!({"x": 1.0, "y": 2.0}),
            ..request
        };
        assert_eq!(drag_to_point(&request_without_to), None);
    }

    #[test]
    fn parses_explicit_action_coordinates() {
        assert_eq!(
            explicit_point(&json!({"x": 640.0, "y": 360.0})),
            Some((640.0, 360.0))
        );
    }

    #[test]
    fn maps_screenshot_pixels_to_portal_stream_coordinates() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 2560.0,
                height: 1440.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            original_pixel_size: Some(PixelSize {
                width: 2560,
                height: 1440,
            }),
            logical_to_pixel_scale: Some(0.75),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        assert_eq!(
            point_from_screenshot_pixels(
                (960.0, 540.0),
                Some(&capture),
                InputBackendKind::PortalRemoteDesktop
            ),
            (1280.0, 720.0)
        );
    }

    #[test]
    fn maps_screenshot_pixels_to_original_x11_pixels() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::X11,
            image_backend: Some(CaptureBackendKind::X11),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: None,
            pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            original_pixel_size: Some(PixelSize {
                width: 2560,
                height: 1440,
            }),
            logical_to_pixel_scale: None,
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        assert_eq!(
            point_from_screenshot_pixels((960.0, 540.0), Some(&capture), InputBackendKind::XTest),
            (1280.0, 720.0)
        );
    }

    #[test]
    fn matches_selector_case_insensitively_for_titles_and_names() {
        let app = AppInfo {
            app_id: "app-1".to_string(),
            name: "Zenity".to_string(),
            pid: Some(123),
            executable: Some("zenity".to_string()),
            desktop_file_id: Some("zenity.desktop".to_string()),
            toolkit_guess: Some("GTK".to_string()),
            window_title: Some("Sky-CUA Pointer Smoke".to_string()),
            is_focused_candidate: false,
        };
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: None,
            window_title: Some("pointer smoke".to_string()),
            name: Some("zenity".to_string()),
        };
        assert!(matches_selector(&app, &selector));
    }

    #[test]
    fn summarizes_selector_fields() {
        let selector = AppSelector {
            app_id: Some("app-1".to_string()),
            desktop_file_id: None,
            window_title: Some("demo".to_string()),
            name: None,
        };
        assert_eq!(
            selector_summary(&selector),
            "app_id=app-1, window_title=demo"
        );
    }

    #[test]
    fn matches_x11_window_to_accessible_app_by_pid() {
        let app = AppInfo {
            app_id: "accessible-1".to_string(),
            name: "Discord".to_string(),
            pid: Some(1234),
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            toolkit_guess: Some("Electron".to_string()),
            window_title: Some("@Sky - Discord".to_string()),
            is_focused_candidate: false,
        };
        let window = X11WindowInfo {
            window_id: "0x2400006".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x2400006".to_string(),
                name: "discord".to_string(),
                pid: Some(1234),
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("@Sky - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            child_regions: Vec::new(),
        };
        assert!(x11_window_matches_app(&window, &app));
    }

    #[test]
    fn creates_a_synthetic_root_element_for_x11_fallback_windows() {
        let window = X11WindowInfo {
            window_id: "0x3800030".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: AppInfo {
                app_id: "x11:0x3800030".to_string(),
                name: "Xmessage".to_string(),
                pid: None,
                executable: None,
                desktop_file_id: Some("xmessage.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua xmessage probe".to_string()),
                is_focused_candidate: true,
            },
            bounds: Some(RectF {
                x: 100.0,
                y: 200.0,
                width: 320.0,
                height: 180.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            child_regions: vec![
                X11WindowRegion {
                    window_id: "0x3800031".to_string(),
                    parent_window_id: None,
                    depth: 1,
                    name: None,
                    bounds: RectF {
                        x: 100.0,
                        y: 200.0,
                        width: 320.0,
                        height: 180.0,
                        space: CoordinateSpace::DesktopLogical,
                    },
                },
                X11WindowRegion {
                    window_id: "0x3800032".to_string(),
                    parent_window_id: Some("0x3800031".to_string()),
                    depth: 2,
                    name: Some("OK".to_string()),
                    bounds: RectF {
                        x: 180.0,
                        y: 330.0,
                        width: 48.0,
                        height: 24.0,
                        space: CoordinateSpace::DesktopLogical,
                    },
                },
            ],
        };

        let elements = x11_window_elements(&window);
        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0].role, "window");
        assert_eq!(
            elements[0].bounds.as_ref().map(|rect| rect.width),
            Some(320.0)
        );
        assert!(elements[0].state_flags.iter().any(|flag| flag == "focused"));
        assert_eq!(elements[1].role, "x11_container");
        assert!(
            elements[1]
                .state_flags
                .iter()
                .any(|flag| flag == "container")
        );
        assert_eq!(elements[2].role, "x11_action_region");
        assert_eq!(elements[2].parent_index, Some(1));
        assert!(
            elements[2]
                .state_flags
                .iter()
                .any(|flag| flag == "action_like")
        );
    }

    #[test]
    fn creates_structural_anchor_regions_for_kwin_fallback_windows() {
        let window = KWinWindowInfo {
            window_id: "{tidal-window}".to_string(),
            resource_name: Some("tidal".to_string()),
            resource_class: Some("TIDAL".to_string()),
            app: AppInfo {
                app_id: "kwin:{tidal-window}".to_string(),
                name: "TIDAL".to_string(),
                pid: Some(4242),
                executable: Some("TIDAL".to_string()),
                desktop_file_id: Some("tidal-hifi.desktop".to_string()),
                toolkit_guess: Some("Qt".to_string()),
                window_title: Some("TIDAL Hi-Fi".to_string()),
                is_focused_candidate: true,
            },
            bounds: Some(RectF {
                x: 100.0,
                y: 80.0,
                width: 1280.0,
                height: 820.0,
                space: CoordinateSpace::DesktopLogical,
            }),
        };

        let elements = kwin_window_elements(&window);
        assert!(elements.len() >= 8);
        assert_eq!(elements[0].role, "window");
        assert!(
            elements[0]
                .state_flags
                .iter()
                .any(|flag| flag == "vision_anchor")
        );
        assert_eq!(elements[1].role, "wayland_header_band");
        assert_eq!(elements[1].parent_index, Some(0));
        assert!(
            elements[1]
                .description
                .as_deref()
                .is_some_and(|description| description.contains("screenshot"))
        );
        assert!(elements.iter().any(|element| {
            element.role == "wayland_search_candidate"
                && element.state_flags.iter().any(|flag| flag == "search_like")
                && element.state_flags.iter().any(|flag| flag == "text_like")
        }));
        assert!(elements.iter().any(|element| {
            element.role == "wayland_sidebar_region"
                && element
                    .state_flags
                    .iter()
                    .any(|flag| flag == "navigation_like")
        }));
        assert!(elements.iter().any(|element| {
            element.role == "wayland_list_candidate"
                && element.state_flags.iter().any(|flag| flag == "list_like")
                && element
                    .state_flags
                    .iter()
                    .any(|flag| flag == "vision_anchor")
        }));
        assert!(elements.iter().any(|element| {
            element.role == "wayland_row_band_candidate"
                && element.state_flags.iter().any(|flag| flag == "row_like")
                && element.state_flags.iter().any(|flag| flag == "text_like")
        }));
        assert!(
            elements
                .iter()
                .filter(|element| element.role == "wayland_row_band_candidate")
                .count()
                >= 4
        );
        assert!(elements.iter().any(|element| {
            element.role == "wayland_action_cluster_candidate"
                && element.state_flags.iter().any(|flag| flag == "action_like")
                && element.state_flags.iter().any(|flag| flag == "menu_like")
        }));
        assert!(
            elements
                .iter()
                .filter(|element| element.role == "wayland_action_cluster_candidate")
                .count()
                >= 2
        );
        assert!(
            elements
                .iter()
                .any(|element| element.role == "wayland_main_region")
        );
    }

    #[test]
    fn capture_diagnostics_surface_downgrade_when_screenshot_fallback_is_used() {
        let environment = wayland_pipewire_environment();
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalScreenshot),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            logical_rect: None,
            pixel_size: None,
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: Some("/tmp/fallback.png".to_string()),
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        };
        let error = BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            "remote fd closed unexpectedly",
        );
        let mut diagnostics = DiagnosticBuilder::new();

        push_capture_diagnostics(
            &environment,
            Some(&capture),
            None,
            Some(&error),
            &mut diagnostics,
        );

        let entries = diagnostics.finish();
        assert!(
            entries
                .iter()
                .any(|entry| entry.code == "PipeWireStreamFailed")
        );
        let downgrade = entries
            .iter()
            .find(|entry| entry.code == "CaptureBackendDowngraded")
            .expect("expected a capture downgrade diagnostic");
        assert!(downgrade.message.contains("downgraded from PipeWire"));
        assert_eq!(
            downgrade.details.as_deref(),
            Some("primary_backend=portal_pipe_wire image_backend=portal_screenshot")
        );
    }

    #[test]
    fn capture_diagnostics_do_not_claim_downgrade_without_a_fallback_image() {
        let environment = wayland_pipewire_environment();
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: None,
            coordinate_space: None,
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            logical_rect: None,
            pixel_size: None,
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        };
        let error = BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            "capture timed out on cached stream",
        );
        let mut diagnostics = DiagnosticBuilder::new();

        push_capture_diagnostics(
            &environment,
            Some(&capture),
            None,
            Some(&error),
            &mut diagnostics,
        );

        let entries = diagnostics.finish();
        let pipewire = entries
            .iter()
            .find(|entry| entry.code == "PipeWireStreamFailed")
            .expect("expected a PipeWire failure diagnostic");
        assert!(pipewire.message.contains("no fallback image was produced"));
        assert!(
            !entries
                .iter()
                .any(|entry| entry.code == "CaptureBackendDowngraded")
        );
    }

    #[test]
    fn matches_x11_window_to_accessible_app_by_class_when_titles_do_not_help() {
        let app = AppInfo {
            app_id: "accessible-2".to_string(),
            name: "Code".to_string(),
            pid: None,
            executable: Some("code".to_string()),
            desktop_file_id: Some("code.desktop".to_string()),
            toolkit_guess: Some("Electron".to_string()),
            window_title: Some("workspace-a".to_string()),
            is_focused_candidate: false,
        };
        let window = X11WindowInfo {
            window_id: "0x500001".to_string(),
            instance_name: Some("code".to_string()),
            class_name: Some("Code".to_string()),
            app: AppInfo {
                app_id: "x11:0x500001".to_string(),
                name: "Code".to_string(),
                pid: None,
                executable: None,
                desktop_file_id: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("totally different title".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            child_regions: Vec::new(),
        };

        assert!(x11_window_matches_app(&window, &app));
    }

    #[test]
    fn does_not_match_an_x11_window_by_title_alone() {
        let app = AppInfo {
            app_id: "accessible-2b".to_string(),
            name: "kaccess".to_string(),
            pid: None,
            executable: Some("kaccess".to_string()),
            desktop_file_id: Some("kaccess.desktop".to_string()),
            toolkit_guess: Some("Qt".to_string()),
            window_title: Some("sky-cua xmessage probe".to_string()),
            is_focused_candidate: false,
        };
        let window = X11WindowInfo {
            window_id: "0x500002".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: AppInfo {
                app_id: "x11:0x500002".to_string(),
                name: "Xmessage".to_string(),
                pid: None,
                executable: Some("xmessage".to_string()),
                desktop_file_id: Some("xmessage.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua xmessage probe".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            child_regions: Vec::new(),
        };

        assert!(!x11_window_matches_app(&window, &app));
    }

    #[test]
    fn selector_prefers_exact_window_title_over_broader_desktop_match() {
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: Some("xmessage.desktop".to_string()),
            window_title: Some("selector beta".to_string()),
            name: None,
        };
        let alpha = X11WindowInfo {
            window_id: "0x500010".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: AppInfo {
                app_id: "x11:0x500010".to_string(),
                name: "Xmessage".to_string(),
                pid: None,
                executable: Some("xmessage".to_string()),
                desktop_file_id: Some("xmessage.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua selector alpha".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            child_regions: Vec::new(),
        };
        let beta = X11WindowInfo {
            window_id: "0x500011".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: AppInfo {
                app_id: "x11:0x500011".to_string(),
                name: "Xmessage".to_string(),
                pid: None,
                executable: Some("xmessage".to_string()),
                desktop_file_id: Some("xmessage.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua selector beta".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            child_regions: Vec::new(),
        };

        let matched =
            select_x11_window(&[alpha, beta.clone()], &selector).expect("selector should match");
        assert_eq!(matched.window_id, beta.window_id);
    }

    #[test]
    fn selector_prefers_focused_x11_window_when_only_desktop_id_is_given() {
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: Some("discord.desktop".to_string()),
            window_title: None,
            name: None,
        };
        let background = X11WindowInfo {
            window_id: "0x500012".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x500012".to_string(),
                name: "discord".to_string(),
                pid: None,
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Friends - Discord".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            child_regions: Vec::new(),
        };
        let focused = X11WindowInfo {
            window_id: "0x500013".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x500013".to_string(),
                name: "discord".to_string(),
                pid: None,
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Project Foxglove - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            child_regions: Vec::new(),
        };

        let matched = select_x11_window(&[background, focused.clone()], &selector)
            .expect("selector should match");
        assert_eq!(matched.window_id, focused.window_id);
    }

    #[test]
    fn prefers_the_best_x11_window_match_when_multiple_windows_share_a_process() {
        let app = AppInfo {
            app_id: "accessible-3".to_string(),
            name: "Discord".to_string(),
            pid: Some(4321),
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            toolkit_guess: Some("Electron".to_string()),
            window_title: Some("Project Foxglove - Discord".to_string()),
            is_focused_candidate: false,
        };
        let weaker = X11WindowInfo {
            window_id: "0x600001".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x600001".to_string(),
                name: "discord".to_string(),
                pid: Some(4321),
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Friends - Discord".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            child_regions: Vec::new(),
        };
        let stronger = X11WindowInfo {
            window_id: "0x600002".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x600002".to_string(),
                name: "discord".to_string(),
                pid: Some(4321),
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Project Foxglove - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            child_regions: Vec::new(),
        };

        let windows = [weaker.clone(), stronger.clone()];
        let matched = best_x11_window_match(&windows, &app).expect("a best match should be found");
        assert_eq!(matched.window_id, stronger.window_id);
    }
}
