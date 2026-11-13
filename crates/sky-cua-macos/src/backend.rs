//! `MacosDesktopBackend`: the `DesktopBackend` implementation for macOS.
//!
//! Mirrors the Windows backend method by method. CoreGraphics supplies the
//! window and display lists, ScreenCaptureKit the screenshots, the AX runtime
//! the semantic tree and actions, and CGEvent the physical input. Every
//! backend kind reported in `EnvironmentInfo` follows the live TCC grant
//! state, never an assumption.

mod actions;
mod app_state;
mod focus;

use async_trait::async_trait;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    AccessibilitySetupReport, ActionOutcome, ActionRequest, AppInfo, AppSelector, AppStateSnapshot,
    CaptureBackendKind, CaptureScope, CaptureScreenMode, DiagnosticEntry, DisplayInfo,
    DisplayTarget, DoctorReport, EnvironmentInfo, InputBackendKind, PortalCapabilities,
    ScrollDirection, SemanticBackendKind, SessionKind, ToolAvailability, ToolCapabilities,
    WindowInfo, WindowTarget,
};
use sky_cua_platform::new_snapshot_id;

use self::actions::ActionPlan;
use crate::ax::{self, AxRuntime};
use crate::capture::{self, CaptureSource};
use crate::displays;
use crate::permissions::{self, Permission, PermissionStatus};
use crate::windows;

/// Reported in `EnvironmentInfo.compositor`.
const COMPOSITOR: &str = "macos-window-server";

#[derive(Debug, Default)]
pub struct MacosDesktopBackend {
    ax: AxRuntime,
}

impl MacosDesktopBackend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ax: AxRuntime::new(),
        }
    }

    /// The environment and the grant state it was derived from, read once.
    fn probe() -> (EnvironmentInfo, PermissionStatus) {
        let status = permissions::status();
        (
            environment_from(status, displays::enumerate_displays()),
            status,
        )
    }
}

fn environment_from(status: PermissionStatus, displays: Vec<DisplayInfo>) -> EnvironmentInfo {
    EnvironmentInfo {
        session_kind: SessionKind::Macos,
        compositor: Some(COMPOSITOR.to_string()),
        desktop_environment: None,
        capture_backend: if status.screen_capture {
            CaptureBackendKind::ScreenCaptureKit
        } else {
            CaptureBackendKind::None
        },
        input_backend: if status.post_event {
            InputBackendKind::CgEvent
        } else {
            InputBackendKind::None
        },
        semantic_backend: if status.accessibility {
            SemanticBackendKind::MacosAccessibility
        } else {
            SemanticBackendKind::None
        },
        portal_capabilities: PortalCapabilities {
            screencast_version: None,
            remote_desktop_version: None,
            screenshot_version: None,
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        },
        xdg_session_type: None,
        display: None,
        wayland_display: None,
        displays,
    }
}

/// What the environment can actually do; unavailable tools carry the System
/// Settings guidance for the missing grant.
fn capabilities(environment: &EnvironmentInfo) -> ToolCapabilities {
    let semantic_ready = environment.semantic_backend == SemanticBackendKind::MacosAccessibility;
    let physical_ready = environment.input_backend == InputBackendKind::CgEvent;
    let listing_ready = environment.session_kind == SessionKind::Macos;
    let semantic_reason = permissions::missing_permission_error(Permission::Accessibility).message;
    let input_reason = permissions::missing_permission_error(Permission::PostEvent).message;
    let semantic = || availability(semantic_ready, &semantic_reason);
    let physical = || availability(physical_ready, &input_reason);
    let either = || {
        availability(
            semantic_ready || physical_ready,
            &format!("{semantic_reason} {input_reason}"),
        )
    };

    ToolCapabilities {
        list_apps: availability(listing_ready, "macOS window listing is unavailable"),
        get_app_state: availability(listing_ready, "macOS window listing is unavailable"),
        focus_element: semantic(),
        activate_element: semantic(),
        select_element: semantic(),
        expand_element: semantic(),
        collapse_element: semantic(),
        toggle_element: semantic(),
        click: either(),
        perform_action: semantic(),
        perform_secondary_action: either(),
        scroll: physical(),
        supported_scroll_directions: vec![
            ScrollDirection::Up,
            ScrollDirection::Down,
            ScrollDirection::Left,
            ScrollDirection::Right,
        ],
        drag: physical(),
        type_text: physical(),
        press_key: physical(),
        set_value: either(),
    }
}

fn availability(available: bool, reason: &str) -> ToolAvailability {
    ToolAvailability {
        available,
        reason: (!available).then(|| reason.to_string()),
    }
}

/// The resolved AX `backend_ref` of a request whose semantic attempt fell
/// through, for the physical-fallback diagnostic.
fn ax_backend_ref_for_fallback(request: &ActionRequest) -> Option<&str> {
    request
        .resolved_element
        .as_ref()
        .and_then(|element| element.backend_ref.as_deref())
        .filter(|backend_ref| ax::is_ax_backend_ref(backend_ref))
        .filter(|_| actions::has_semantic_equivalent(&request.action))
}

fn join_error(error: tokio::task::JoinError) -> BackendError {
    BackendError::new(
        BackendErrorCode::Internal,
        format!("macOS input task failed to join cleanly: {error}"),
    )
}

#[async_trait]
impl DesktopBackend for MacosDesktopBackend {
    /// Runs once at daemon startup. It never prompts: a routine start must
    /// not raise consent dialogs or open System Settings. Missing grants are
    /// logged with their System Settings guidance; `setup_accessibility` is
    /// the explicit path that requests them.
    async fn prepare_automation_permissions(&self) -> Result<(), BackendError> {
        let missing = permissions::status().missing();
        for permission in &missing {
            let error = permissions::missing_permission_error(*permission);
            tracing::warn!(
                code = error.code,
                message = %error.message,
                "macOS privacy permission is not granted"
            );
        }
        match missing.first() {
            Some(permission) => Err(permissions::missing_permission_error(*permission)),
            None => Ok(()),
        }
    }

    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        Ok(Self::probe().0)
    }

    async fn doctor(&self) -> Result<DoctorReport, BackendError> {
        let (environment, status) = Self::probe();
        Ok(permissions::doctor_report(environment, status))
    }

    /// Explicit, user-initiated setup: requests every missing grant (which
    /// registers the responsible application in System Settings, shows the
    /// consent prompt, and opens the pane) and reports doctor state around
    /// it. The requests run on a blocking thread and never wait for the user.
    async fn setup_accessibility(&self) -> Result<AccessibilitySetupReport, BackendError> {
        let before = self.doctor().await?;
        let outcome = tokio::task::spawn_blocking(permissions::request_missing)
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!("macOS permission request task failed to join cleanly: {error}"),
                )
            })?;
        let after = self.doctor().await?;
        Ok(permissions::setup_report(before, &outcome, after))
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        let windows_front_to_back = windows::enumerate_windows();
        let mut seen_pids = Vec::new();
        Ok(windows_front_to_back
            .iter()
            .filter(|window| {
                let first = !seen_pids.contains(&window.pid);
                if first {
                    seen_pids.push(window.pid);
                }
                first
            })
            .map(windows::window_to_app)
            .collect())
    }

    async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError> {
        let displays = displays::enumerate_displays();
        Ok(windows::enumerate_windows()
            .iter()
            .map(|window| windows::window_to_model(window, &displays))
            .collect())
    }

    async fn focused_window(&self) -> Result<Option<WindowInfo>, BackendError> {
        Ok(windows::enumerate_windows()
            .iter()
            .find(|window| window.focused)
            .map(|window| windows::window_to_model(window, &displays::enumerate_displays())))
    }

    async fn resolve_window_target(
        &self,
        target: &WindowTarget,
    ) -> Result<WindowInfo, BackendError> {
        let window = windows::resolve_window_target(&windows::enumerate_windows(), target)?;
        Ok(windows::window_to_model(
            &window,
            &displays::enumerate_displays(),
        ))
    }

    async fn list_displays(&self) -> Result<Vec<DisplayInfo>, BackendError> {
        Ok(displays::enumerate_displays())
    }

    async fn activate_window(&self, target: WindowTarget) -> Result<ActionOutcome, BackendError> {
        let windows_front_to_back = windows::enumerate_windows();
        let window = windows::resolve_window_target(&windows_front_to_back, &target)?;
        let report = focus::focus_window(&self.ax, &window, &windows_front_to_back).await?;
        if !report.focused
            && let Some(error) = report.raise_error
        {
            return Err(error);
        }
        let mut outcome =
            actions::success(&format!("Activated macOS window {}.", window.window_id));
        outcome.diagnostics = report.diagnostics();
        Ok(outcome)
    }

    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        self.app_state(selector, capture_screen).await
    }

    async fn get_app_state_for_window(
        &self,
        window: &WindowInfo,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        self.app_state_for_window(window, capture_screen).await
    }

    async fn screenshot(
        &self,
        target: Option<WindowTarget>,
        display_target: Option<DisplayTarget>,
    ) -> Result<AppStateSnapshot, BackendError> {
        let snapshot_id = new_snapshot_id();
        let (environment, status) = Self::probe();
        let capabilities = capabilities(&environment);
        let mut diagnostics = DiagnosticBuilder::new();
        if !status.screen_capture {
            return Err(permissions::missing_permission_error(
                Permission::ScreenCapture,
            ));
        }

        let mut focused_app = None;
        let (source, scope) = if let Some(target) = target {
            let windows_front_to_back = windows::enumerate_windows();
            let window = windows::resolve_window_target(&windows_front_to_back, &target)?;
            if window.focused {
                // ScreenCaptureKit captures the window wherever it is, so a
                // window that is already focused is not re-activated.
                diagnostics.push_code(
                    "WindowFocusVerified",
                    format!(
                        "macOS window {} was already focused before screenshot capture.",
                        window.window_id
                    ),
                    None,
                );
            } else {
                diagnostics.push_code(
                    "WindowFocusRequested",
                    format!(
                        "Requested foreground focus for macOS window {}.",
                        window.window_id
                    ),
                    None,
                );
                match focus::focus_window(&self.ax, &window, &windows_front_to_back).await {
                    Ok(report) => {
                        for diagnostic in report.diagnostics() {
                            diagnostics.push_code(
                                diagnostic.code,
                                diagnostic.message,
                                diagnostic.details,
                            );
                        }
                    }
                    // ScreenCaptureKit captures the window without focus, so
                    // a declined activation degrades the snapshot, not fails it.
                    Err(error) => diagnostics.push_code(
                        error.code,
                        format!(
                            "macOS window {} could not be brought to the front; capturing it in place",
                            window.window_id
                        ),
                        Some(error.message),
                    ),
                }
            }
            focused_app = Some(windows::window_to_focused_app(
                &window,
                &environment.displays,
            ));
            (CaptureSource::Window(window), CaptureScope::Window)
        } else if let Some(display_target) = display_target {
            let display = displays::resolve_display_target(&environment.displays, &display_target)?;
            (CaptureSource::Display(display), CaptureScope::Display)
        } else if let Some(display) = displays::primary_display(&environment.displays) {
            (
                CaptureSource::Display(display),
                CaptureScope::PrimaryDisplay,
            )
        } else {
            return Err(BackendError::new(
                BackendErrorCode::CaptureSourceGeometryMissing,
                "no active macOS display is available to capture",
            ));
        };

        let capture = capture::capture(&snapshot_id, source, scope).await?;

        Ok(AppStateSnapshot {
            snapshot_id,
            created_at: chrono::Utc::now(),
            environment,
            capabilities,
            focused_app,
            capture: Some(capture),
            elements: Vec::new(),
            diagnostics: diagnostics.finish(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        })
    }

    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let ax_fallback_ref = ax_backend_ref_for_fallback(&request).map(ToOwned::to_owned);
        if let Some(outcome) = self.ax.try_semantic_action(&request).await? {
            return Ok(outcome);
        }

        let status = permissions::status();
        let input = match actions::plan_action(&request)? {
            ActionPlan::Physical(input) => input,
            ActionPlan::RequiresAccessibility => {
                return Err(if status.accessibility {
                    BackendError::new(
                        BackendErrorCode::ActionUnsupportedForEnvironment,
                        "this semantic automation primitive requires a macOS Accessibility element from a fresh snapshot",
                    )
                } else {
                    permissions::missing_permission_error(Permission::Accessibility)
                });
            }
        };
        if !status.post_event {
            return Err(permissions::missing_permission_error(Permission::PostEvent));
        }

        let focus_diagnostics =
            focus::focus_request_window(&self.ax, &request, input.uses_snapshot_point()).await?;
        let mut outcome = tokio::task::spawn_blocking(move || actions::perform(input))
            .await
            .map_err(join_error)??;
        let mut diagnostics = focus_diagnostics;
        diagnostics.append(&mut outcome.diagnostics);
        if let Some(reference) = ax_fallback_ref {
            diagnostics.push(DiagnosticEntry {
                code: BackendErrorCode::AccessibilityCoverageLimited
                    .as_str()
                    .to_string(),
                message: "macOS Accessibility semantic action was unavailable; physical input fallback was used".to_string(),
                details: Some(reference),
            });
        }
        outcome.diagnostics = diagnostics;
        Ok(outcome)
    }

    async fn reset_desktop_session_state(&self) {
        self.ax.reset().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(accessibility: bool, screen_capture: bool, post_event: bool) -> PermissionStatus {
        PermissionStatus {
            accessibility,
            screen_capture,
            post_event,
        }
    }

    #[test]
    fn environment_reports_backends_from_grants() {
        let granted = environment_from(status(true, true, true), Vec::new());
        assert_eq!(granted.session_kind, SessionKind::Macos);
        assert_eq!(
            granted.capture_backend,
            CaptureBackendKind::ScreenCaptureKit
        );
        assert_eq!(granted.input_backend, InputBackendKind::CgEvent);
        assert_eq!(
            granted.semantic_backend,
            SemanticBackendKind::MacosAccessibility
        );
        assert_eq!(granted.portal_capabilities.screencast_version, None);

        let denied = environment_from(status(false, false, false), Vec::new());
        assert_eq!(denied.capture_backend, CaptureBackendKind::None);
        assert_eq!(denied.input_backend, InputBackendKind::None);
        assert_eq!(denied.semantic_backend, SemanticBackendKind::None);
    }

    #[test]
    fn capabilities_follow_the_environment() {
        let semantic_only = capabilities(&environment_from(status(true, true, false), Vec::new()));
        assert!(semantic_only.click.available);
        assert!(semantic_only.perform_action.available);
        assert!(semantic_only.set_value.available);
        assert!(semantic_only.perform_secondary_action.available);
        assert!(!semantic_only.scroll.available);
        assert!(semantic_only.scroll.reason.is_some());
        assert!(!semantic_only.press_key.available);

        let physical_only = capabilities(&environment_from(status(false, true, true), Vec::new()));
        assert!(physical_only.click.available);
        assert!(!physical_only.focus_element.available);
        assert!(physical_only.drag.available);
        assert!(physical_only.perform_secondary_action.available);
        assert!(physical_only.list_apps.available);

        let none = capabilities(&environment_from(status(false, false, false), Vec::new()));
        assert!(!none.click.available);
        assert!(!none.set_value.available);
        assert!(!none.perform_secondary_action.available);
        assert!(none.get_app_state.available);
    }
}
