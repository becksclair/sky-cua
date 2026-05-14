use atspi::AccessibilityConnection;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AppSelector, AppStateSnapshot, CaptureBackendKind,
    CaptureInfo, CoordinateSpace, DiagnosticEntry, DoctorReport, ElementNode, EnvironmentInfo,
    FocusedApp, InputBackendKind, ModelImageFormat, PixelSize, RectF, SemanticBackendKind,
    ToolAvailability, ToolCapabilities,
};
use sky_cua_platform::{AppInfo, SetValueFallbackMode, SetValueRouting, new_snapshot_id};

use crate::app_policy::{AppActionPolicies, ResolvedSetValueFallbackPolicy};
use crate::apps::discovery::{DiscoveredApp, discover_apps};
use crate::atspi::{
    actions as atspi_actions, connect, normalize_action, snapshot::snapshot_for_app,
};
use crate::coords::{center_of, desktop_to_stream, logical_to_pixel};
use crate::env_probe::{probe_environment, require_supported_environment};
use crate::focus::pick_focused_app;
use crate::portal::remote_desktop::{
    MouseButton, PortalLifecycleEvent, RemoteDesktopSessionManager,
};
use crate::portal::screenshot;
use crate::windowing as linux_windowing;
use crate::x11::capture as x11_capture;
use crate::x11::input_xtest::{self, X11MouseButton};
use crate::x11::windowing::{self, X11WindowInfo};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::warn;
use zbus::Proxy;

#[derive(Debug, Clone)]
pub struct LinuxDesktopBackend {
    portal: RemoteDesktopSessionManager,
    atspi: std::sync::Arc<Mutex<Option<AccessibilityConnection>>>,
    app_policies: AppActionPolicies,
}

#[derive(Debug, Clone, Copy)]
enum SemanticAtspiAction {
    Activate,
    Select,
    Expand,
    Collapse,
    Toggle,
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

    async fn focus_window_target_for_keyboard(
        &self,
        request: &ActionRequest,
    ) -> Result<Option<linux_windowing::LinuxWindowInfo>, BackendError> {
        let Some(target) = window_target_from_arguments(&request.arguments)? else {
            return Ok(None);
        };
        let environment = match request.environment.clone() {
            Some(environment) => environment,
            None => self.probe_environment().await?,
        };
        require_supported_environment(&environment)?;
        let windows = linux_windowing::discover_activation_windows(&environment).await?;
        let target_window = linux_windowing::resolve_window_target(&windows, &target.into())?;

        if target_window.backend == "x11" {
            if !input_xtest::xtest_is_available() {
                return Err(BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    "targeted X11 keyboard input requires XTest/xdotool window activation",
                ));
            }
            input_xtest::window_activate(&target_window.window_id)?;
            return Ok(Some(target_window.clone()));
        }

        if environment.input_backend == InputBackendKind::None {
            return Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!(
                    "matched {} window {}, but no session input backend is available after native activation",
                    target_window.backend, target_window.window_id
                ),
            ));
        }

        linux_windowing::activate_window(target_window).await?;
        let _ = linux_windowing::verify_window_focused(&environment, target_window).await?;
        Ok(Some(target_window.clone()))
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
        let window_listing_ready = linux_windowing::probe_backends(environment)
            .iter()
            .any(|probe| probe.can_list_windows);
        let physical_ready = environment.input_backend != InputBackendKind::None;

        ToolCapabilities {
            list_apps: ToolAvailability {
                available: semantic_ready || window_listing_ready,
                reason: (!(semantic_ready || window_listing_ready))
                    .then(|| "Neither AT-SPI nor a window-query fallback is available".to_string()),
            },
            get_app_state: ToolAvailability {
                available: semantic_ready || window_listing_ready,
                reason: (!(semantic_ready || window_listing_ready))
                    .then(|| "Neither AT-SPI nor a window-query fallback is available".to_string()),
            },
            focus_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            activate_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            select_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            expand_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            collapse_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            toggle_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            click: ToolAvailability {
                available: semantic_ready || physical_ready,
                reason: (!(semantic_ready || physical_ready))
                    .then(|| "No semantic or physical input backend is available".to_string()),
            },
            perform_action: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
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
            app_user_model_id: app.app_user_model_id.clone(),
            window_handle: app.window_handle.clone(),
            toolkit_guess: app.toolkit_guess.clone(),
            window_title: app.window_title.clone(),
        }
    }
}

#[async_trait::async_trait]
impl DesktopBackend for LinuxDesktopBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        let mut environment = probe_environment().await?;
        environment.semantic_backend = if require_supported_environment(&environment).is_ok()
            && self.accessibility_connection().await.is_ok()
        {
            SemanticBackendKind::Atspi
        } else {
            SemanticBackendKind::None
        };
        Ok(environment)
    }

    async fn doctor(&self) -> Result<sky_cua_platform::model::DoctorReport, BackendError> {
        let environment = self.probe_environment().await?;
        Ok(crate::doctor::build_doctor_report(environment))
    }

    async fn setup_accessibility(
        &self,
    ) -> Result<sky_cua_platform::model::AccessibilitySetupReport, BackendError> {
        crate::setup::setup_accessibility_report(|| async { self.doctor().await }).await
    }

    async fn setup_window_targeting(
        &self,
    ) -> Result<sky_cua_platform::model::WindowTargetingSetupReport, BackendError> {
        Ok(crate::setup::setup_window_targeting_report().await)
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        let registry_windows = linux_windowing::discover_app_windows(&environment)
            .await
            .unwrap_or_default();
        let mut atspi_apps = match self.discover_accessible_apps().await {
            Ok((_, apps)) => apps,
            Err(error) => {
                if registry_windows.is_empty() {
                    return Err(error);
                }
                Vec::new()
            }
        };
        enrich_accessible_apps_from_windows(&mut atspi_apps, &registry_windows);
        Ok(merge_app_lists(&atspi_apps, &registry_windows))
    }

    async fn list_windows(&self) -> Result<Vec<sky_cua_platform::model::WindowInfo>, BackendError> {
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        linux_windowing::discover_windows(&environment)
            .await
            .map(|windows| windows.into_iter().map(Into::into).collect())
    }

    async fn focused_window(
        &self,
    ) -> Result<Option<sky_cua_platform::model::WindowInfo>, BackendError> {
        if let Some(window) = linux_windowing::focused_window_override() {
            return Ok(Some(window.into()));
        }
        Ok(self
            .list_windows()
            .await?
            .into_iter()
            .find(|window| window.focused))
    }

    async fn activate_window(
        &self,
        target: sky_cua_platform::model::WindowTarget,
    ) -> Result<ActionOutcome, BackendError> {
        if !target.has_target() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "activate_window requires one of window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd",
            ));
        }
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        let windows = linux_windowing::discover_activation_windows(&environment).await?;
        let window = linux_windowing::resolve_window_target(&windows, &target.into())?;
        linux_windowing::activate_window(window).await?;
        let focused = linux_windowing::verify_window_focused(&environment, window).await?;
        Ok(success_with_diagnostics(
            format!("Activated {} window {}.", window.backend, window.window_id),
            vec![DiagnosticEntry {
                code: "WindowFocusVerified".to_string(),
                message: format!(
                    "Focus verification matched {} window {}.",
                    focused.backend, focused.window_id
                ),
                details: None,
            }],
        ))
    }

    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
    ) -> Result<AppStateSnapshot, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        let snapshot_id = new_snapshot_id();
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        let capabilities = Self::capabilities(&environment);
        let doctor_report = crate::doctor::build_doctor_report(environment.clone());
        let mut diagnostics = DiagnosticBuilder::new();
        if !doctor_report.readiness.can_build_accessibility_tree {
            diagnostics.push(
                BackendErrorCode::AccessibilityUnavailable,
                "Semantic accessibility is unavailable; Computer Use will fall back to window and screenshot anchors where possible.",
                Some(doctor_report.readiness.recommended_next_step.clone()),
            );
        }
        let mut portal_session_error: Option<BackendError> = None;
        let mut capture_error: Option<BackendError> = None;
        let registry_windows = linux_windowing::discover_app_windows(&environment)
            .await
            .unwrap_or_default();

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
                    .and_then(|selector| select_linux_window(&registry_windows, selector))
                    .or_else(|| preferred_linux_window(&registry_windows));
                push_capture_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);
                if let Some(window) = fallback_window {
                    let app = app_from_linux_window(&window);
                    diagnostics.push(
                        BackendErrorCode::AccessibilityCoverageLimited,
                        format!(
                            "The selected {} window is visible through the window registry, but no AT-SPI application tree was available for it",
                            window.backend
                        ),
                        Some(selector_or_window_summary(selector.as_ref(), &app)),
                    );
                    return Ok(linux_fallback_snapshot(
                        snapshot_id,
                        environment,
                        capabilities,
                        capture,
                        diagnostics,
                        Some(doctor_report),
                        window,
                    ));
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
                    doctor_report: Some(doctor_report),
                });
            }
        };

        enrich_accessible_apps_from_windows(&mut apps, &registry_windows);
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
                .and_then(|selector| select_linux_window(&registry_windows, selector))
                .or_else(|| preferred_linux_window(&registry_windows))
            {
                let app = app_from_linux_window(&window);
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!(
                        "The selected {} window is visible through the window registry, but no accessible AT-SPI application tree matched it",
                        window.backend
                    ),
                    Some(selector_or_window_summary(selector.as_ref(), &app)),
                );
                return Ok(linux_fallback_snapshot(
                    snapshot_id,
                    environment,
                    capabilities,
                    capture,
                    diagnostics,
                    Some(doctor_report),
                    window,
                ));
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
                doctor_report: Some(doctor_report),
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
            } else if let Some(window) = select_linux_window(&registry_windows, selector) {
                push_capture_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);
                let app = app_from_linux_window(&window);
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!(
                        "The selected {} window is visible through the window registry, but no accessible AT-SPI application tree matched it",
                        window.backend
                    ),
                    Some(selector_or_window_summary(Some(selector), &app)),
                );
                return Ok(linux_fallback_snapshot(
                    snapshot_id,
                    environment,
                    capabilities,
                    capture,
                    diagnostics,
                    Some(doctor_report),
                    window,
                ));
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
            if let Some(window) = preferred_linux_window(&registry_windows)
                && !apps
                    .iter()
                    .any(|app| linux_window_matches_app(&window, &app.info))
            {
                let app = app_from_linux_window(&window);
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!(
                        "The focused {} window is visible through the window registry, but no accessible AT-SPI application tree matched it",
                        window.backend
                    ),
                    Some(window_summary(&app)),
                );
                push_capture_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&portal_lifecycle_events, &mut diagnostics);
                return Ok(linux_fallback_snapshot(
                    snapshot_id,
                    environment,
                    capabilities,
                    capture,
                    diagnostics,
                    Some(doctor_report),
                    window,
                ));
            }

            focused.unwrap_or_else(|| {
                apps.first()
                    .cloned()
                    .expect("apps should not be empty at this point")
            })
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
            doctor_report: Some(doctor_report),
        })
    }

    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        match request.action {
            ActionName::FocusElement => self.focus_element(request).await,
            ActionName::ActivateElement => self.activate_element(request).await,
            ActionName::SelectElement => self.select_element(request).await,
            ActionName::ExpandElement => self.expand_element(request).await,
            ActionName::CollapseElement => self.collapse_element(request).await,
            ActionName::ToggleElement => self.toggle_element(request).await,
            ActionName::Click => self.click(request).await,
            ActionName::PerformAction => self.perform_action(request).await,
            ActionName::PerformSecondaryAction => self.secondary_click(request).await,
            ActionName::Scroll => self.scroll(request).await,
            ActionName::Drag => self.drag(request).await,
            ActionName::TypeText => self.type_text(request).await,
            ActionName::PressKey => self.press_key(request).await,
            ActionName::SetValue => self.set_value(request).await,
        }
    }

    async fn reset_portal_tokens(
        &self,
    ) -> Result<sky_cua_platform::model::PortalTokenResetOutcome, BackendError> {
        self.portal.reset_persisted_tokens().await
    }
}

fn semantic_backend_ref<'a>(
    request: &'a ActionRequest,
    tool_name: &str,
) -> Result<&'a str, BackendError> {
    if let Some(backend_ref) = direct_backend_ref(&request.arguments) {
        return Ok(backend_ref);
    }
    let element = request.resolved_element.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!(
                "{tool_name} requires element_index, element_identifier, or a semantic selector so the service can resolve a semantic target"
            ),
        )
    })?;
    element.backend_ref.as_deref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("{tool_name} target did not include a backend_ref"),
        )
    })
}

fn direct_backend_ref(arguments: &serde_json::Value) -> Option<&str> {
    arguments
        .get("element_identifier")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_retryable_accessibility_error(error: &BackendError) -> bool {
    error.code == BackendErrorCode::AccessibilityUnavailable.as_str()
        && error.message.contains("Resource temporarily unavailable")
}

impl LinuxDesktopBackend {
    async fn focus_element(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let backend_ref = semantic_backend_ref(&request, "focus_element")?;
        let connection = self.accessibility_connection().await?;
        if atspi_actions::grab_focus(&connection, backend_ref).await? {
            return Ok(success("Focused the element semantically through AT-SPI."));
        }
        Err(BackendError::new(
            BackendErrorCode::ActionRequiresPhysicalInput,
            format!("AT-SPI focus was unavailable for element {backend_ref}"),
        ))
    }

    async fn activate_element(
        &self,
        request: ActionRequest,
    ) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "activate_element",
            "Activated the element semantically through AT-SPI.",
            SemanticAtspiAction::Activate,
        )
        .await
    }

    async fn select_element(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "select_element",
            "Selected the element semantically through AT-SPI.",
            SemanticAtspiAction::Select,
        )
        .await
    }

    async fn expand_element(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "expand_element",
            "Expanded the element semantically through AT-SPI.",
            SemanticAtspiAction::Expand,
        )
        .await
    }

    async fn collapse_element(
        &self,
        request: ActionRequest,
    ) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "collapse_element",
            "Collapsed the element semantically through AT-SPI.",
            SemanticAtspiAction::Collapse,
        )
        .await
    }

    async fn toggle_element(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "toggle_element",
            "Toggled the element semantically through AT-SPI.",
            SemanticAtspiAction::Toggle,
        )
        .await
    }

    async fn semantic_atspi_action(
        &self,
        request: &ActionRequest,
        tool_name: &str,
        success_message: &str,
        action: SemanticAtspiAction,
    ) -> Result<ActionOutcome, BackendError> {
        let backend_ref = semantic_backend_ref(request, tool_name)?;
        let connection = self.accessibility_connection().await?;
        let performed = match action {
            SemanticAtspiAction::Activate => {
                atspi_actions::activate(&connection, backend_ref).await
            }
            SemanticAtspiAction::Select => atspi_actions::select(&connection, backend_ref).await,
            SemanticAtspiAction::Expand => atspi_actions::expand(&connection, backend_ref).await,
            SemanticAtspiAction::Collapse => {
                atspi_actions::collapse(&connection, backend_ref).await
            }
            SemanticAtspiAction::Toggle => atspi_actions::toggle(&connection, backend_ref).await,
        }?;
        if performed {
            return Ok(success(success_message));
        }
        Err(BackendError::new(
            BackendErrorCode::ActionRequiresPhysicalInput,
            format!("AT-SPI {tool_name} was unavailable for element {backend_ref}"),
        ))
    }

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

        let input_backend = effective_pointer_input_backend_for_target(&request);
        let (x, y) = action_point_for_backend(&request, input_backend.clone())?;
        match input_backend {
            InputBackendKind::PortalRemoteDesktop => {
                self.portal.pointer_move_absolute(x, y).await?;
                self.portal.click(MouseButton::Left).await?;
                Ok(success_with_diagnostics(
                    "Clicked the target through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                let x11_window = matched_x11_window_for_request(&request);
                activate_x11_window(x11_window.as_ref());
                input_xtest::pointer_move_absolute(x, y)?;
                input_xtest::click(X11MouseButton::Left)?;
                Ok(success(
                    "Clicked the target through the X11 input fallback.",
                ))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("click fallback"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for click fallback",
            )),
        }
    }

    async fn perform_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let backend_ref = semantic_backend_ref(&request, "perform_action")?;
        let connection = self.accessibility_connection().await?;
        let action_index = self
            .resolve_requested_action_index(&connection, backend_ref, &request)
            .await?;
        let invocation =
            atspi_actions::invoke_action_by_index(&connection, backend_ref, action_index).await?;
        if invocation.ok {
            Ok(success(format!(
                "Invoked AT-SPI action {} ({}).",
                invocation.action_index,
                invocation
                    .action_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("unnamed")
            )))
        } else {
            Err(BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!(
                    "AT-SPI action {} ({}) returned false for element {backend_ref}",
                    invocation.action_index,
                    invocation
                        .action_name
                        .as_deref()
                        .filter(|name| !name.is_empty())
                        .unwrap_or("unnamed")
                ),
            ))
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

        let input_backend = effective_pointer_input_backend_for_target(&request);
        let (x, y) = action_point_for_backend(&request, input_backend.clone())?;
        match input_backend {
            InputBackendKind::PortalRemoteDesktop => {
                self.portal.pointer_move_absolute(x, y).await?;
                self.portal.click(MouseButton::Right).await?;
                Ok(success_with_diagnostics(
                    "Performed the secondary click through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                let x11_window = matched_x11_window_for_request(&request);
                activate_x11_window(x11_window.as_ref());
                input_xtest::pointer_move_absolute(x, y)?;
                input_xtest::click(X11MouseButton::Right)?;
                Ok(success(
                    "Performed the secondary click through the X11 input fallback.",
                ))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("secondary click fallback"))
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
                InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {}
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
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("scroll"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for scroll",
            )),
        }
    }

    async fn drag(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let input_backend = effective_pointer_input_backend_for_target(&request);
        let from = drag_from_point(&request, input_backend.clone())?;
        let to = if let Some(element) = request.resolved_target_element.as_ref() {
            point_for_element_for_backend(
                element,
                request.resolved_capture.as_ref(),
                input_backend.clone(),
            )?
        } else {
            drag_to_point(&request, input_backend.clone()).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "drag requires either to_element_index or explicit to_x/to_y coordinates",
                )
            })?
        };

        match input_backend {
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
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                input_xtest::pointer_move_absolute(to.0, to.1)?;
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                input_xtest::pointer_button(X11MouseButton::Left, false)?;
                Ok(success("Dragged through the X11 input fallback."))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("drag"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for drag",
            )),
        }
    }

    async fn type_text(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let text = request
            .arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "type_text requires a text argument",
                )
            })?;
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
        {
            let connection = self.accessibility_connection().await?;
            let _ = atspi_actions::grab_focus(&connection, backend_ref).await;
        }
        let target_window = self.focus_window_target_for_keyboard(&request).await?;
        let x11_window = if target_window.is_none() {
            matched_x11_window_for_request(&request)
        } else {
            None
        };
        let x11_window_id = target_window
            .as_ref()
            .filter(|window| window.backend == "x11")
            .map(|window| window.window_id.as_str())
            .or_else(|| x11_window.as_ref().map(|window| window.window_id.as_str()));
        let input_backend = effective_keyboard_input_backend_for_target(
            &request,
            x11_window.as_ref(),
            x11_window_id,
        );
        match input_backend {
            InputBackendKind::PortalRemoteDesktop => {
                if should_prefer_kde_clipboard_text_backend(&request) {
                    match run_kde_clipboard_paste_text(&self.portal, &text).await {
                        Ok(message) => {
                            return Ok(success_with_diagnostics(
                                message,
                                portal_lifecycle_diagnostics(
                                    &self.portal.take_lifecycle_events().await,
                                ),
                            ));
                        }
                        Err(error) => {
                            if error.clear_portal_session {
                                self.portal.reset_session().await;
                            }
                            if !error.can_fallback_to_portal_keysym {
                                return Err(BackendError::new(
                                    BackendErrorCode::ActionUnsupportedForEnvironment,
                                    error.message,
                                ));
                            }
                        }
                    }
                }
                self.portal.send_text(&text).await?;
                Ok(success_with_diagnostics(
                    "Typed text through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                activate_x11_window(x11_window.as_ref());
                input_xtest::send_text_to_target(x11_window_id, &text)?;
                Ok(success("Typed text through the X11 input fallback."))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("type_text"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for type_text",
            )),
        }
    }

    async fn press_key(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let keys = parse_key_sequence(&request.arguments).ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "press_key requires a key string or keys array",
            )
        })?;
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
        {
            let connection = self.accessibility_connection().await?;
            let _ = atspi_actions::grab_focus(&connection, backend_ref).await;
        }

        let target_window = self.focus_window_target_for_keyboard(&request).await?;
        let x11_window = if target_window.is_none() {
            matched_x11_window_for_request(&request)
        } else {
            None
        };
        let x11_window_id = target_window
            .as_ref()
            .filter(|window| window.backend == "x11")
            .map(|window| window.window_id.as_str())
            .or_else(|| x11_window.as_ref().map(|window| window.window_id.as_str()));
        match effective_keyboard_input_backend_for_target(
            &request,
            x11_window.as_ref(),
            x11_window_id,
        ) {
            InputBackendKind::PortalRemoteDesktop => {
                self.portal.press_key_sequence(&keys).await?;
                Ok(success_with_diagnostics(
                    "Pressed the key sequence through the RemoteDesktop portal.",
                    portal_lifecycle_diagnostics(&self.portal.take_lifecycle_events().await),
                ))
            }
            InputBackendKind::XTest => {
                activate_x11_window(x11_window.as_ref());
                input_xtest::press_key_sequence_to_target(x11_window_id, &keys)?;
                Ok(success(
                    "Pressed the key sequence through the X11 input fallback.",
                ))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("press_key"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for press_key",
            )),
        }
    }

    async fn set_value(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let backend_ref = semantic_backend_ref(&request, "set_value")?;
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
            Ok(atspi_actions::SetValueResult::EditableText) => {
                return Ok(success("Set editable text semantically through AT-SPI."));
            }
            Ok(atspi_actions::SetValueResult::Numeric { value }) => {
                return Ok(success(format!(
                    "Set numeric value to {value} semantically through AT-SPI."
                )));
            }
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

    async fn resolve_requested_action_index(
        &self,
        connection: &AccessibilityConnection,
        backend_ref: &str,
        request: &ActionRequest,
    ) -> Result<i32, BackendError> {
        if let Some(index) = request
            .arguments
            .get("action_index")
            .and_then(serde_json::Value::as_i64)
            .or_else(|| {
                request
                    .arguments
                    .get("action_index")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| value.trim().parse::<i64>().ok())
            })
            .or_else(|| {
                request
                    .arguments
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| value.trim().parse::<i64>().ok())
            })
        {
            return i32::try_from(index).map_err(|error| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("action_index {index} is not a valid AT-SPI action index: {error}"),
                )
            });
        }

        let Some(action_name) = request
            .arguments
            .get("action_name")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                request
                    .arguments
                    .get("action")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(0);
        };

        let actions = atspi_actions::available_actions(connection, backend_ref).await?;
        actions
            .iter()
            .position(|candidate| action_name_matches(candidate, action_name))
            .and_then(|index| i32::try_from(index).ok())
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "element {backend_ref} exposes actions [{}], but none matched requested action_name {action_name:?}",
                        actions.join(", ")
                    ),
                )
            })
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
                        activate_x11_window(x11_window.as_ref());
                        input_xtest::pointer_move_absolute(x, y)?;
                        input_xtest::click(X11MouseButton::Left)?;
                        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                        input_xtest::press_key_sequence_to_target(
                            x11_window.as_ref().map(|window| window.window_id.as_str()),
                            &select_all,
                        )?;
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                        input_xtest::send_text_to_target(
                            x11_window.as_ref().map(|window| window.window_id.as_str()),
                            value,
                        )?;
                        Ok(success_with_diagnostics(
                            "Set the value through a heuristics-backed physical typing fallback.",
                            diagnostics,
                        ))
                    }
                    InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                        Err(windows_input_backend_error("set_value fallback"))
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

fn windows_input_backend_error(action: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!("Windows input backends are unavailable in the Linux backend for {action}"),
    )
}

fn input_backend_for(request: &ActionRequest) -> InputBackendKind {
    request
        .environment
        .as_ref()
        .map(|environment| environment.input_backend.clone())
        .unwrap_or(InputBackendKind::None)
}

fn effective_pointer_input_backend_for_target(request: &ActionRequest) -> InputBackendKind {
    input_backend_for(request)
}

fn element_is_x11_fallback(element: &ElementNode) -> bool {
    element.role.starts_with("x11_")
        || element
            .state_flags
            .iter()
            .any(|flag| flag == "x11_fallback")
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

fn effective_keyboard_input_backend_for_target(
    request: &ActionRequest,
    x11_window: Option<&X11WindowInfo>,
    target_window_id: Option<&str>,
) -> InputBackendKind {
    if target_window_id.is_some() {
        if input_xtest::xtest_is_available() {
            InputBackendKind::XTest
        } else {
            InputBackendKind::None
        }
    } else {
        effective_keyboard_input_backend(request, x11_window)
    }
}

const EVDEV_KEY_LEFTCTRL: i32 = 29;
const EVDEV_KEY_V: i32 = 47;
const KDE_CLIPBOARD_RESTORE_DELAY_MS: u64 = 500;
const WL_COPY_STARTUP_GRACE_MS: u64 = 50;
const WL_COPY_PASTE_ONCE_TIMEOUT_MS: u64 = 2_000;
const PLAIN_TEXT_CLIPBOARD_MIME_TYPES: &[&str] = &["text/plain", "utf8_string", "string", "text"];

#[derive(Debug)]
struct KdeClipboardPasteError {
    message: String,
    can_fallback_to_portal_keysym: bool,
    clear_portal_session: bool,
}

impl KdeClipboardPasteError {
    fn before_text_input(message: String) -> Self {
        Self {
            message,
            can_fallback_to_portal_keysym: true,
            clear_portal_session: false,
        }
    }

    fn after_portal_input(message: String) -> Self {
        Self {
            message,
            can_fallback_to_portal_keysym: false,
            clear_portal_session: true,
        }
    }
}

fn should_prefer_kde_clipboard_text_backend(request: &ActionRequest) -> bool {
    let Some(environment) = request.environment.as_ref() else {
        return false;
    };
    if environment.input_backend != InputBackendKind::PortalRemoteDesktop {
        return false;
    }
    environment
        .desktop_environment
        .as_deref()
        .is_some_and(|desktop| desktop.to_ascii_lowercase().contains("kde"))
}

async fn run_kde_clipboard_paste_text(
    portal: &RemoteDesktopSessionManager,
    text: &str,
) -> Result<String, KdeClipboardPasteError> {
    ensure_clipboard_is_plain_text_only()
        .await
        .map_err(KdeClipboardPasteError::before_text_input)?;
    let previous = kde_clipboard_contents()
        .await
        .map_err(KdeClipboardPasteError::before_text_input)?;
    wl_copy_sensitive_paste_once(text)
        .await
        .map_err(KdeClipboardPasteError::before_text_input)?;

    let paste_result = portal
        .press_keycode_chord(&[EVDEV_KEY_LEFTCTRL], EVDEV_KEY_V)
        .await
        .map_err(|error| error.message);

    tokio::time::sleep(Duration::from_millis(KDE_CLIPBOARD_RESTORE_DELAY_MS)).await;
    let restore_result = kde_set_clipboard_contents(&previous).await;

    match (paste_result, restore_result) {
        (Ok(()), Ok(())) => Ok("Typed text through the KDE clipboard portal fallback.".to_string()),
        (Err(error), Ok(())) => Err(KdeClipboardPasteError::after_portal_input(error)),
        (Ok(()), Err(restore_error)) => Ok(format!(
            "Typed text through the KDE clipboard portal fallback. Warning: previous KDE clipboard contents could not be restored: {restore_error}"
        )),
        (Err(error), Err(restore_error)) => {
            Err(KdeClipboardPasteError::after_portal_input(format!(
                "{error}; previous KDE clipboard contents could not be restored: {restore_error}"
            )))
        }
    }
}

async fn ensure_clipboard_is_plain_text_only() -> Result<(), String> {
    let mime_types = wl_paste_mime_types().await?;
    if clipboard_mime_types_are_plain_text_only(&mime_types) {
        return Ok(());
    }
    Err(format!(
        "KDE clipboard paste fallback refused to overwrite non-text clipboard contents: {}",
        mime_types.join(", ")
    ))
}

fn clipboard_mime_types_are_plain_text_only(mime_types: &[String]) -> bool {
    mime_types.iter().all(|mime_type| {
        let normalized = mime_type.trim().to_ascii_lowercase();
        normalized.is_empty()
            || PLAIN_TEXT_CLIPBOARD_MIME_TYPES
                .iter()
                .any(|plain| normalized == *plain || normalized.starts_with(&format!("{plain};")))
    })
}

async fn kde_clipboard_contents() -> Result<String, String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| format!("failed to connect to session bus: {error}"))?;
    let proxy = kde_klipper_proxy(&connection).await?;
    proxy
        .call("getClipboardContents", &())
        .await
        .map_err(|error| format!("failed to read KDE clipboard contents: {error}"))
}

async fn kde_set_clipboard_contents(text: &str) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| format!("failed to connect to session bus: {error}"))?;
    let proxy = kde_klipper_proxy(&connection).await?;
    proxy
        .call("setClipboardContents", &(text))
        .await
        .map_err(|error| format!("failed to set KDE clipboard contents: {error}"))
}

async fn wl_paste_mime_types() -> Result<Vec<String>, String> {
    if !command_exists("wl-paste") {
        return Err(
            "wl-paste is required to safely inspect current clipboard MIME types".to_string(),
        );
    }
    let output = Command::new("wl-paste")
        .args(["--list-types"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("failed to run wl-paste --list-types: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("wl-paste --list-types exited with {}", output.status)
        } else {
            format!(
                "wl-paste --list-types exited with {}: {stderr}",
                output.status
            )
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

async fn wl_copy_sensitive_paste_once(text: &str) -> Result<(), String> {
    if !command_exists("wl-copy") {
        return Err("wl-copy is required for KDE clipboard paste fallback".to_string());
    }
    let mut child = Command::new("wl-copy")
        .args([
            "--foreground",
            "--paste-once",
            "--sensitive",
            "--type",
            "text/plain;charset=utf-8",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to run wl-copy: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open wl-copy stdin".to_string())?;
    stdin
        .write_all(text.as_bytes())
        .await
        .map_err(|error| format!("failed to write text to wl-copy stdin: {error}"))?;
    drop(stdin);

    tokio::time::sleep(Duration::from_millis(WL_COPY_STARTUP_GRACE_MS)).await;
    if let Some(status) = child
        .try_wait()
        .map_err(|error| format!("failed to inspect wl-copy status: {error}"))?
    {
        return Err(format!(
            "wl-copy exited before the paste request with {status}"
        ));
    }

    tokio::spawn(async move {
        if tokio::time::timeout(
            Duration::from_millis(WL_COPY_PASTE_ONCE_TIMEOUT_MS),
            child.wait(),
        )
        .await
        .is_err()
        {
            let _ = child.kill().await;
        }
    });

    Ok(())
}

async fn kde_klipper_proxy(connection: &zbus::Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(
        connection,
        "org.kde.klipper",
        "/klipper",
        "org.kde.klipper.klipper",
    )
    .await
    .map_err(|error| format!("failed to create KDE Klipper proxy: {error}"))
}

fn command_exists(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(command).is_file())
}

fn action_name_matches(candidate: &str, requested: &str) -> bool {
    let candidate = normalize_action(candidate);
    let requested = normalize_action(requested);
    if candidate == requested {
        return true;
    }
    canonical_action_aliases(&requested)
        .iter()
        .any(|alias| candidate == *alias)
}

fn canonical_action_aliases(action: &str) -> &'static [&'static str] {
    match action {
        "activate" => &["press", "click", "open", "jump", "invoke"],
        "select" => &["choose"],
        "expand" => &["open"],
        "collapse" => &["close"],
        "toggle" => &["check", "uncheck"],
        _ => &[],
    }
}

fn activate_x11_window(window: Option<&X11WindowInfo>) {
    if let Some(window) = window
        && let Err(error) = input_xtest::window_activate(&window.window_id)
    {
        warn!(
            "X11 window activation failed before input fallback; continuing with pointer injection: {}",
            error.message
        );
    }
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
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: app.toolkit_guess.clone(),
        window_title: app.window_title.clone(),
        is_focused_candidate: true,
    };
    best_x11_window_match(&windows, &app).cloned()
}

fn window_target_from_arguments(
    arguments: &serde_json::Value,
) -> Result<Option<sky_cua_platform::model::WindowTarget>, BackendError> {
    const TARGET_FIELDS: &[&str] = &[
        "window_id",
        "pid",
        "tty",
        "terminal_pid",
        "terminal_command",
        "terminal_cwd",
        "app_id",
        "wm_class",
        "title",
    ];
    let has_target = TARGET_FIELDS
        .iter()
        .any(|field| arguments.get(*field).is_some_and(value_is_present));
    if !has_target {
        return Ok(None);
    }

    serde_json::from_value(arguments.clone())
        .map(Some)
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!("invalid window target arguments: {error}"),
            )
        })
}

fn value_is_present(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        _ => true,
    }
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

    match backend {
        InputBackendKind::PortalRemoteDesktop if element_is_x11_fallback(element) => {
            point_for_x11_element_through_portal(element, request.resolved_capture.as_ref())
        }
        InputBackendKind::PortalRemoteDesktop => {
            point_for_element(element, request.resolved_capture.as_ref())
        }
        InputBackendKind::XTest => {
            point_for_x11_element(element, request.resolved_capture.as_ref())
        }
        InputBackendKind::SendInput
        | InputBackendKind::WindowsMessages
        | InputBackendKind::None => point_for_element(element, None),
    }
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

fn point_for_x11_element(
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
    if bounds.space == CoordinateSpace::DesktopLogical
        && let Some(capture) = capture
        && let (Some(logical_rect), Some(original_pixel_size)) = (
            capture.logical_rect.as_ref(),
            capture.original_pixel_size.as_ref(),
        )
        && let Some(pixel_point) = logical_to_pixel(center, logical_rect, original_pixel_size)
    {
        return Ok(pixel_point);
    }
    Ok(center)
}

fn point_for_x11_element_through_portal(
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
    if let Some(capture) = capture
        && let (Some(logical_rect), Some(original_pixel_size)) = (
            capture.logical_rect.as_ref(),
            capture.original_pixel_size.as_ref(),
        )
        && logical_rect.width > 0.0
        && logical_rect.height > 0.0
        && original_pixel_size.width > 0
        && original_pixel_size.height > 0
    {
        let rel_x = center.0 / f64::from(original_pixel_size.width);
        let rel_y = center.1 / f64::from(original_pixel_size.height);
        return Ok((rel_x * logical_rect.width, rel_y * logical_rect.height));
    }
    Ok(center)
}

fn point_for_element_for_backend(
    element: &ElementNode,
    capture: Option<&CaptureInfo>,
    backend: InputBackendKind,
) -> Result<(f64, f64), BackendError> {
    match backend {
        InputBackendKind::PortalRemoteDesktop if element_is_x11_fallback(element) => {
            point_for_x11_element_through_portal(element, capture)
        }
        InputBackendKind::PortalRemoteDesktop => point_for_element(element, capture),
        InputBackendKind::XTest => point_for_x11_element(element, capture),
        InputBackendKind::SendInput
        | InputBackendKind::WindowsMessages
        | InputBackendKind::None => point_for_element(element, None),
    }
}

fn explicit_point(arguments: &serde_json::Value) -> Option<(f64, f64)> {
    point_from_fields(arguments, "x", "y")
}

fn drag_from_point(
    request: &ActionRequest,
    backend: InputBackendKind,
) -> Result<(f64, f64), BackendError> {
    if let Some(point) = point_from_fields(&request.arguments, "from_x", "from_y")
        .or_else(|| explicit_point(&request.arguments))
    {
        return Ok(point_from_screenshot_pixels(
            point,
            request.resolved_capture.as_ref(),
            backend,
        ));
    }

    let element = request.resolved_element.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "drag requires either element_index or explicit from_x/from_y coordinates",
        )
    })?;
    point_for_element_for_backend(element, request.resolved_capture.as_ref(), backend)
}

fn drag_to_point(request: &ActionRequest, backend: InputBackendKind) -> Option<(f64, f64)> {
    point_from_fields(&request.arguments, "to_x", "to_y").map(|point| {
        point_from_screenshot_pixels(point, request.resolved_capture.as_ref(), backend)
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
        InputBackendKind::SendInput | InputBackendKind::WindowsMessages => point,
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

fn select_linux_window(
    windows: &[linux_windowing::LinuxWindowInfo],
    selector: &AppSelector,
) -> Option<linux_windowing::LinuxWindowInfo> {
    windows
        .iter()
        .filter_map(|window| {
            let app = app_from_linux_window(window);
            selector_match_score(&app, selector).map(|score| (score, window))
        })
        .max_by_key(|(score, window)| (*score, window.focused))
        .map(|(_, window)| window.clone())
}

fn preferred_linux_window(
    windows: &[linux_windowing::LinuxWindowInfo],
) -> Option<linux_windowing::LinuxWindowInfo> {
    windows
        .iter()
        .find(|window| window.focused)
        .cloned()
        .or_else(|| windows.first().cloned())
}

fn enrich_accessible_apps_from_windows(
    apps: &mut [DiscoveredApp],
    windows: &[linux_windowing::LinuxWindowInfo],
) {
    for app in apps {
        let Some(window) = best_linux_window_match(windows, &app.info) else {
            continue;
        };
        let window_app = app_from_linux_window(window);

        if app.info.pid.is_none() {
            app.info.pid = window_app.pid;
        }
        if app.info.executable.is_none() {
            app.info.executable = window_app.executable.clone();
        }
        if app.info.desktop_file_id.is_none() {
            app.info.desktop_file_id = window_app.desktop_file_id.clone();
        }
        if app.info.toolkit_guess.is_none() {
            app.info.toolkit_guess = window_app.toolkit_guess.clone();
        }
        if app.info.window_title.is_none() {
            app.info.window_title = window_app.window_title.clone();
        }
        if app.info.name.eq_ignore_ascii_case("Unnamed") {
            app.info.name = window_app.name.clone();
        }
        if !app.info.is_focused_candidate && window_app.is_focused_candidate {
            app.info.is_focused_candidate = true;
        }
    }
}

fn merge_app_lists(
    apps: &[DiscoveredApp],
    windows: &[linux_windowing::LinuxWindowInfo],
) -> Vec<AppInfo> {
    let mut merged = apps.iter().map(|app| app.info.clone()).collect::<Vec<_>>();
    for window in windows {
        if !merged
            .iter()
            .any(|app| linux_window_matches_app(window, app))
        {
            merged.push(app_from_linux_window(window));
        }
    }
    merged
}

fn best_linux_window_match<'a>(
    windows: &'a [linux_windowing::LinuxWindowInfo],
    app: &AppInfo,
) -> Option<&'a linux_windowing::LinuxWindowInfo> {
    windows
        .iter()
        .filter_map(|window| linux_window_match_score(window, app).map(|score| (score, window)))
        .max_by_key(|(score, window)| (*score, window.focused))
        .map(|(_, window)| window)
}

fn linux_window_matches_app(window: &linux_windowing::LinuxWindowInfo, app: &AppInfo) -> bool {
    linux_window_match_score(window, app).is_some()
}

fn linux_window_match_score(
    window: &linux_windowing::LinuxWindowInfo,
    app: &AppInfo,
) -> Option<i32> {
    let window_app = app_from_linux_window(window);
    if app.app_id == window_app.app_id {
        return Some(1_000);
    }
    if let (Some(window_pid), Some(app_pid)) = (window.pid, app.pid)
        && window_pid == app_pid
    {
        return Some(900);
    }

    let mut score = 0i32;
    let mut identity_signals = 0u8;

    let window_title = window_app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let app_title = app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let window_name = normalize_match_key(&window_app.name);
    let app_name = normalize_match_key(&app.name);
    let window_executable = window_app.executable.as_deref().map(normalize_match_key);
    let app_executable = app.executable.as_deref().map(normalize_match_key);
    let window_desktop = window_app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let app_desktop = app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let window_resource_name = window.app_id.as_deref().map(normalize_match_key);
    let window_resource_class = window.wm_class.as_deref().map(normalize_match_key);

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

    if window.focused {
        score += 5;
    }

    (identity_signals > 0 && score > 0).then_some(score)
}

fn app_from_linux_window(window: &linux_windowing::LinuxWindowInfo) -> AppInfo {
    let name = window
        .app_id
        .as_deref()
        .or(window.wm_class.as_deref())
        .or(window.title.as_deref())
        .unwrap_or("Window")
        .to_string();
    AppInfo {
        app_id: window
            .app_id
            .clone()
            .unwrap_or_else(|| format!("{}:{}", window.backend, window.window_id)),
        name,
        pid: window.pid,
        executable: None,
        desktop_file_id: window
            .app_id
            .as_ref()
            .filter(|value| value.ends_with(".desktop"))
            .cloned(),
        app_user_model_id: None,
        window_handle: Some(window.window_id.clone()),
        toolkit_guess: window.client_type.clone(),
        window_title: window.title.clone(),
        is_focused_candidate: window.focused,
    }
}

#[cfg(test)]
fn select_x11_window(windows: &[X11WindowInfo], selector: &AppSelector) -> Option<X11WindowInfo> {
    windows
        .iter()
        .filter_map(|window| {
            selector_match_score(&window.app, selector).map(|score| (score, window))
        })
        .max_by_key(|(score, window)| (*score, window.app.is_focused_candidate))
        .map(|(_, window)| window.clone())
}

#[cfg(test)]
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

fn linux_fallback_snapshot(
    snapshot_id: String,
    environment: EnvironmentInfo,
    capabilities: ToolCapabilities,
    capture: Option<sky_cua_platform::model::CaptureInfo>,
    diagnostics: DiagnosticBuilder,
    doctor_report: Option<DoctorReport>,
    window: linux_windowing::LinuxWindowInfo,
) -> AppStateSnapshot {
    let app = app_from_linux_window(&window);
    AppStateSnapshot {
        snapshot_id,
        created_at: chrono::Utc::now(),
        environment,
        capabilities,
        focused_app: Some(LinuxDesktopBackend::focused_from_app(&app)),
        capture,
        elements: fallback_window_elements(&window),
        diagnostics: diagnostics.finish(),
        app_guidance: None,
        doctor_report,
    }
}

fn fallback_window_elements(window: &linux_windowing::LinuxWindowInfo) -> Vec<ElementNode> {
    let x11_window = refreshed_x11_window_for_linux_window(window);
    fallback_window_elements_with_x11_detail(window, x11_window.as_ref())
}

fn fallback_window_elements_with_x11_detail(
    window: &linux_windowing::LinuxWindowInfo,
    x11_window: Option<&X11WindowInfo>,
) -> Vec<ElementNode> {
    x11_window
        .map(x11_window_elements)
        .filter(|elements| !elements.is_empty())
        .unwrap_or_else(|| linux_window_elements(window))
}

fn refreshed_x11_window_for_linux_window(
    window: &linux_windowing::LinuxWindowInfo,
) -> Option<X11WindowInfo> {
    if window.backend != "x11" {
        return None;
    }
    windowing::discover_windows()
        .ok()?
        .into_iter()
        .find(|candidate| candidate.window_id == window.window_id)
}

fn linux_window_elements(window: &linux_windowing::LinuxWindowInfo) -> Vec<ElementNode> {
    let Some(bounds) = window.bounds.clone() else {
        return Vec::new();
    };

    let mut root_state_flags = vec![
        "native_window_fallback".to_string(),
        "physical_target".to_string(),
        "vision_anchor".to_string(),
        "container".to_string(),
        "content_like".to_string(),
    ];
    if window.focused {
        root_state_flags.push("focused".to_string());
        root_state_flags.push("active".to_string());
    }
    let app = app_from_linux_window(window);

    let mut elements = vec![ElementNode {
        element_index: 0,
        parent_index: None,
        role: "window".to_string(),
        name: app.window_title.clone().or_else(|| Some(app.name.clone())),
        description: Some(format!(
            "{} window surfaced from the window registry without a matching AT-SPI tree. The child regions below are geometric anchors only: use them to narrow the search space, then confirm the real target on the screenshot before clicking, dragging, or typing.",
            window.backend
        )),
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
    state_flags.push("native_window_fallback".to_string());
    state_flags.push("x11_fallback".to_string());
    state_flags.push("physical_target".to_string());

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
        AppInfo, AppSelector, KdeClipboardPasteError, LinuxDesktopBackend, action_name_matches,
        app_from_linux_window, best_x11_window_match, clipboard_mime_types_are_plain_text_only,
        drag_from_point, drag_to_point, effective_pointer_input_backend_for_target, explicit_point,
        fallback_window_elements_with_x11_detail, linux_fallback_snapshot, linux_window_elements,
        matches_selector, parse_key_sequence, point_for_x11_element_through_portal,
        point_from_screenshot_pixels, push_capture_diagnostics, select_x11_window,
        selector_summary, should_prefer_kde_clipboard_text_backend, x11_window_elements,
        x11_window_matches_app,
    };
    use crate::windowing::LinuxWindowInfo;
    use crate::x11::windowing::{X11WindowInfo, X11WindowRegion};
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
    use sky_cua_platform::model::{
        ActionName, ActionRequest, CaptureBackendKind, CaptureInfo, CoordinateSpace, ElementNode,
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
    fn kde_clipboard_text_backend_is_kde_portal_only() {
        let request = ActionRequest {
            action: ActionName::TypeText,
            snapshot_id: None,
            element_index: None,
            arguments: json!({"text": "hello"}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };
        assert!(should_prefer_kde_clipboard_text_backend(&request));

        let non_kde = ActionRequest {
            environment: Some(EnvironmentInfo {
                desktop_environment: Some("GNOME".to_string()),
                ..wayland_pipewire_environment()
            }),
            ..request.clone()
        };
        assert!(!should_prefer_kde_clipboard_text_backend(&non_kde));

        let non_portal = ActionRequest {
            environment: Some(EnvironmentInfo {
                input_backend: InputBackendKind::XTest,
                ..wayland_pipewire_environment()
            }),
            ..request
        };
        assert!(!should_prefer_kde_clipboard_text_backend(&non_portal));
    }

    #[test]
    fn kde_clipboard_error_contract_only_falls_back_before_text_input() {
        let before = KdeClipboardPasteError::before_text_input("missing qdbus".to_string());
        assert!(before.can_fallback_to_portal_keysym);
        assert!(!before.clear_portal_session);
        assert_eq!(before.message, "missing qdbus");

        let after = KdeClipboardPasteError::after_portal_input("paste failed".to_string());
        assert!(!after.can_fallback_to_portal_keysym);
        assert!(after.clear_portal_session);
        assert_eq!(after.message, "paste failed");
    }

    #[test]
    fn xwayland_fallback_elements_stay_on_portal_pointer_backend() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: Some(1),
            arguments: json!({}),
            resolved_element: Some(ElementNode {
                element_index: 1,
                parent_index: Some(0),
                role: "x11_action_region".to_string(),
                name: None,
                description: None,
                value: None,
                state_flags: vec!["x11_fallback".to_string(), "physical_target".to_string()],
                semantic_actions: Vec::new(),
                bounds: Some(RectF {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                backend_ref: None,
            }),
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        assert_eq!(
            effective_pointer_input_backend_for_target(&request),
            InputBackendKind::PortalRemoteDesktop
        );
    }

    #[test]
    fn kde_clipboard_plain_text_guard_rejects_rich_clipboard_types() {
        assert!(clipboard_mime_types_are_plain_text_only(&[]));
        assert!(clipboard_mime_types_are_plain_text_only(&[
            "text/plain;charset=utf-8".to_string(),
            "UTF8_STRING".to_string(),
        ]));
        assert!(!clipboard_mime_types_are_plain_text_only(&[
            "text/plain".to_string(),
            "text/html".to_string(),
        ]));
        assert!(!clipboard_mime_types_are_plain_text_only(&[
            "image/png".to_string()
        ]));
    }

    #[test]
    fn action_name_matching_accepts_advertised_canonical_aliases() {
        assert!(action_name_matches("Press", "activate"));
        assert!(action_name_matches("choose", "select"));
        assert!(action_name_matches("close", "collapse"));
        assert!(!action_name_matches("scroll", "activate"));
    }

    #[test]
    fn matches_app_selector_by_window_title() {
        let app = AppInfo {
            app_id: "app-1".to_string(),
            name: "zenity".to_string(),
            pid: Some(123),
            executable: Some("zenity".to_string()),
            desktop_file_id: Some("zenity.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
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
        assert_eq!(
            drag_to_point(&request, InputBackendKind::PortalRemoteDesktop),
            Some((320.0, 240.0))
        );
        let request_without_to = ActionRequest {
            arguments: json!({"x": 1.0, "y": 2.0}),
            ..request
        };
        assert_eq!(
            drag_to_point(&request_without_to, InputBackendKind::PortalRemoteDesktop),
            None
        );
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
    fn maps_xwayland_x11_pixels_to_portal_logical_coordinates() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("166".to_string()),
            source_type: Some(1),
            mapping_id: None,
            logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 1536.0,
                height: 864.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1440,
                height: 810,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            logical_to_pixel_scale: Some(0.9375),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };
        let element = ElementNode {
            element_index: 4,
            parent_index: Some(1),
            role: "x11_action_region".to_string(),
            name: None,
            description: None,
            value: None,
            state_flags: vec!["x11_fallback".to_string(), "physical_target".to_string()],
            semantic_actions: Vec::new(),
            bounds: Some(RectF {
                x: 896.0,
                y: 552.0,
                width: 32.0,
                height: 24.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            backend_ref: None,
        };

        let point = point_for_x11_element_through_portal(&element, Some(&capture)).unwrap();

        assert!((point.0 - 729.6).abs() < 0.000_001);
        assert!((point.1 - 451.2).abs() < 0.000_001);
    }

    #[test]
    fn maps_xwayland_drag_element_start_through_same_portal_scaling() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("166".to_string()),
            source_type: Some(1),
            mapping_id: None,
            logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 1536.0,
                height: 864.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1440,
                height: 810,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            logical_to_pixel_scale: Some(0.9375),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };
        let request = ActionRequest {
            action: ActionName::Drag,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: Some(4),
            arguments: json!({"to_x": 640.0, "to_y": 480.0}),
            resolved_element: Some(ElementNode {
                element_index: 4,
                parent_index: Some(1),
                role: "x11_action_region".to_string(),
                name: None,
                description: None,
                value: None,
                state_flags: vec!["x11_fallback".to_string(), "physical_target".to_string()],
                semantic_actions: Vec::new(),
                bounds: Some(RectF {
                    x: 896.0,
                    y: 552.0,
                    width: 32.0,
                    height: 24.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                backend_ref: None,
            }),
            resolved_target_element: None,
            resolved_capture: Some(capture),
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        let point = drag_from_point(&request, InputBackendKind::PortalRemoteDesktop).unwrap();

        assert!((point.0 - 729.6).abs() < 0.000_001);
        assert!((point.1 - 451.2).abs() < 0.000_001);
    }

    #[test]
    fn matches_selector_case_insensitively_for_titles_and_names() {
        let app = AppInfo {
            app_id: "app-1".to_string(),
            name: "Zenity".to_string(),
            pid: Some(123),
            executable: Some("zenity".to_string()),
            desktop_file_id: Some("zenity.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
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
            app_user_model_id: None,
            window_handle: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("@Sky - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
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
                app_user_model_id: None,
                window_handle: None,
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
            workspace: None,
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
    fn registry_fallback_prefers_refreshed_x11_child_regions() {
        let linux_window = LinuxWindowInfo {
            window_id: "0x3800030".to_string(),
            title: Some("sky-cua xmessage probe".to_string()),
            app_id: Some("xmessage.desktop".to_string()),
            wm_class: Some("Xmessage".to_string()),
            pid: None,
            bounds: Some(RectF {
                x: 100.0,
                y: 200.0,
                width: 320.0,
                height: 180.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: Some("xwayland".to_string()),
            backend: "x11".to_string(),
            terminal: None,
        };
        let x11_window = X11WindowInfo {
            window_id: "0x3800030".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: app_from_linux_window(&linux_window),
            bounds: linux_window.bounds.clone(),
            workspace: None,
            child_regions: vec![X11WindowRegion {
                window_id: "0x3800032".to_string(),
                parent_window_id: Some("0x3800030".to_string()),
                depth: 1,
                name: Some("OK".to_string()),
                bounds: RectF {
                    x: 180.0,
                    y: 330.0,
                    width: 48.0,
                    height: 24.0,
                    space: CoordinateSpace::DesktopLogical,
                },
            }],
        };

        let elements = fallback_window_elements_with_x11_detail(&linux_window, Some(&x11_window));

        assert_eq!(elements.len(), 2);
        assert_eq!(elements[1].role, "x11_action_region");
        assert_eq!(elements[1].parent_index, Some(0));
    }

    #[test]
    fn creates_structural_anchor_regions_for_kwin_fallback_windows() {
        let window = LinuxWindowInfo {
            window_id: "kwin:{tidal-window}".to_string(),
            title: Some("TIDAL Hi-Fi".to_string()),
            app_id: Some("tidal-hifi.desktop".to_string()),
            wm_class: Some("TIDAL".to_string()),
            pid: Some(4242),
            bounds: Some(RectF {
                x: 100.0,
                y: 80.0,
                width: 1280.0,
                height: 820.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "kwin".to_string(),
            terminal: None,
        };

        let elements = linux_window_elements(&window);
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
    fn linux_fallback_snapshot_preserves_doctor_report() {
        let environment = wayland_pipewire_environment();
        let capabilities = LinuxDesktopBackend::capabilities(&environment);
        let report = crate::doctor::build_doctor_report(environment.clone());
        let window = LinuxWindowInfo {
            window_id: "kwin:{tidal-window}".to_string(),
            title: Some("TIDAL Hi-Fi".to_string()),
            app_id: Some("tidal-hifi.desktop".to_string()),
            wm_class: Some("TIDAL".to_string()),
            pid: Some(4242),
            bounds: Some(RectF {
                x: 100.0,
                y: 80.0,
                width: 1280.0,
                height: 820.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "kwin".to_string(),
            terminal: None,
        };

        let snapshot = linux_fallback_snapshot(
            "snap-1".to_string(),
            environment,
            capabilities,
            None,
            DiagnosticBuilder::new(),
            Some(report.clone()),
            window,
        );

        assert_eq!(snapshot.doctor_report, Some(report));
    }

    #[test]
    fn registry_window_app_does_not_invent_executable() {
        let app = app_from_linux_window(&LinuxWindowInfo {
            window_id: "kwin:{tidal-window}".to_string(),
            title: Some("TIDAL Hi-Fi".to_string()),
            app_id: Some("tidal-hifi.desktop".to_string()),
            wm_class: Some("TIDAL".to_string()),
            pid: Some(4242),
            bounds: None,
            workspace: None,
            focused: true,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "kwin".to_string(),
            terminal: None,
        });

        assert_eq!(app.desktop_file_id.as_deref(), Some("tidal-hifi.desktop"));
        assert_eq!(app.executable, None);
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
            app_user_model_id: None,
            window_handle: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("totally different title".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            workspace: None,
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
            app_user_model_id: None,
            window_handle: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua xmessage probe".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua selector alpha".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua selector beta".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            workspace: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Friends - Discord".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            workspace: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Project Foxglove - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
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
            app_user_model_id: None,
            window_handle: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Friends - Discord".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            workspace: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Project Foxglove - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };

        let windows = [weaker.clone(), stronger.clone()];
        let matched = best_x11_window_match(&windows, &app).expect("a best match should be found");
        assert_eq!(matched.window_id, stronger.window_id);
    }
}
