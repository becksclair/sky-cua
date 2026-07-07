use async_trait::async_trait;

use crate::diagnostics::{BackendError, BackendErrorCode};
use crate::model::{
    AccessibilitySetupReport, ActionOutcome, ActionRequest, AppInfo, AppSelector, AppStateSnapshot,
    CaptureBackendKind, CaptureScreenMode, DiagnosticEntry, DisplayInfo, DisplayTarget,
    DoctorCheck, DoctorReadiness, DoctorReport, EnvironmentInfo, HeuristicMatch, InputBackendKind,
    LaunchedApplication, PortalTokenResetOutcome, SemanticBackendKind, SessionPresenceIntent,
    SessionPresenceStatus, WindowInfo, WindowTarget, WindowTargetingSetupReport,
};

#[async_trait]
pub trait DesktopBackend: Send + Sync {
    async fn prepare_automation_permissions(&self) -> Result<(), BackendError> {
        Ok(())
    }

    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError>;
    async fn doctor(&self) -> Result<DoctorReport, BackendError> {
        let environment = self.probe_environment().await?;
        let mut checks = Vec::new();
        checks.push(DoctorCheck {
            name: "semantic_backend".to_string(),
            ok: environment.semantic_backend != SemanticBackendKind::None,
            detail: format!("{:?}", environment.semantic_backend),
        });
        checks.push(DoctorCheck {
            name: "capture_backend".to_string(),
            ok: environment.capture_backend != CaptureBackendKind::None,
            detail: format!("{:?}", environment.capture_backend),
        });
        checks.push(DoctorCheck {
            name: "input_backend".to_string(),
            ok: environment.input_backend != InputBackendKind::None,
            detail: format!("{:?}", environment.input_backend),
        });
        let can_build_accessibility_tree =
            environment.semantic_backend != SemanticBackendKind::None;
        let can_capture_screen = environment.capture_backend != CaptureBackendKind::None;
        let can_send_input = environment.input_backend != InputBackendKind::None;
        let mut blockers = Vec::new();
        if !can_build_accessibility_tree {
            blockers.push("AT-SPI semantic accessibility is unavailable".to_string());
        }
        if !can_capture_screen {
            blockers.push("No screenshot/capture backend is available".to_string());
        }
        if !can_send_input {
            blockers.push("No physical input backend is available".to_string());
        }
        let recommended_next_step = if blockers.is_empty() {
            "Computer Use core backends are ready.".to_string()
        } else {
            format!("{}.", blockers.join(". "))
        };

        Ok(DoctorReport {
            environment,
            checks,
            readiness: DoctorReadiness {
                can_register_mcp_tools: true,
                can_build_accessibility_tree,
                can_capture_screen,
                can_send_input,
                can_list_windows: false,
                can_target_windows: false,
                can_inhibit_presence: false,
                can_unlock_session: false,
                recommended_next_step,
                blockers,
            },
            platform: None,
            display_topology: None,
            session_env: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
            session_presence: None,
        })
    }
    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError>;
    fn session_env_diagnostics(&self) -> Vec<DiagnosticEntry> {
        Vec::new()
    }
    async fn setup_accessibility(&self) -> Result<AccessibilitySetupReport, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "accessibility setup is only available on Linux backends",
        ))
    }
    async fn setup_window_targeting(&self) -> Result<WindowTargetingSetupReport, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "window targeting setup is only available on Linux backends",
        ))
    }
    /// Launch an application into the backend's own desktop session.
    ///
    /// The child inherits the daemon's environment unchanged; in the isolated
    /// desktop that inherited environment (`DISPLAY=:N`, sandbox session bus, no
    /// `WAYLAND_DISPLAY`) is what keeps the launched window inside the private
    /// desktop. Backends that cannot launch applications return an unsupported
    /// error.
    async fn launch_application(
        &self,
        _command: &str,
        _args: &[String],
    ) -> Result<LaunchedApplication, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "application launch is only available on Linux backends",
        ))
    }
    async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "window listing is not available for this backend",
        ))
    }
    async fn focused_window(&self) -> Result<Option<WindowInfo>, BackendError> {
        Ok(self
            .list_windows()
            .await?
            .into_iter()
            .find(|window| window.focused))
    }
    /// Enumerate the host display topology (logical rects, pixel sizes, scale).
    ///
    /// Defaults to an empty list for backends without display discovery; callers
    /// that derive geometry from the host displays (e.g. phone-scale scrcpy mirror
    /// sizing) must treat an empty result as "topology unknown" and fall back,
    /// never as "no displays exist".
    async fn list_displays(&self) -> Result<Vec<DisplayInfo>, BackendError> {
        Ok(Vec::new())
    }
    async fn activate_window(&self, _target: WindowTarget) -> Result<ActionOutcome, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "window activation is not available for this backend",
        ))
    }
    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError>;
    async fn screenshot(
        &self,
        target: Option<WindowTarget>,
        display_target: Option<DisplayTarget>,
    ) -> Result<AppStateSnapshot, BackendError> {
        if target.is_some() || display_target.is_some() {
            return Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "targeted screenshots are not available for this backend",
            ));
        }
        self.get_app_state(None, CaptureScreenMode::Always).await
    }
    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError>;
    async fn reset_portal_tokens(&self) -> Result<PortalTokenResetOutcome, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "portal token reset is only available on portal-backed Linux sessions",
        ))
    }
    async fn ensure_session_presence(
        &self,
        _intent: SessionPresenceIntent,
    ) -> Result<SessionPresenceStatus, BackendError> {
        Ok(SessionPresenceStatus::unsupported("none"))
    }
    async fn release_session_presence(
        &self,
        _relock: bool,
    ) -> Result<SessionPresenceStatus, BackendError> {
        Ok(SessionPresenceStatus::unsupported("none"))
    }
    async fn session_presence_status(&self) -> SessionPresenceStatus {
        SessionPresenceStatus::unsupported("none")
    }

    /// Drop cached live desktop-session handles (the AT-SPI connection, the
    /// portal RemoteDesktop/ScreenCast session) after a desktop request was
    /// abandoned server-side for exceeding its deadline. Persisted portal
    /// tokens are NOT touched — this only clears in-memory session handles so
    /// the next request opens a fresh connection instead of reusing one that
    /// may still be wedged on the compositor/portal side. Backends with no
    /// live session cache (e.g. Windows) default to a no-op.
    async fn reset_desktop_session_state(&self) {}
}

#[async_trait]
pub trait SemanticBackend: Send + Sync {
    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError>;
    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
    ) -> Result<AppStateSnapshot, BackendError>;
}

#[async_trait]
pub trait CaptureBackend: Send + Sync {
    async fn capture_summary(&self) -> Result<Option<crate::model::CaptureInfo>, BackendError>;
}

#[async_trait]
pub trait InputBackend: Send + Sync {
    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError>;
}

#[async_trait]
pub trait AppDiscoveryBackend: Send + Sync {
    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError>;
}

#[async_trait]
pub trait FocusTracker: Send + Sync {
    async fn focused_app(&self) -> Result<Option<AppInfo>, BackendError>;
}

#[async_trait]
pub trait HeuristicsResolver: Send + Sync {
    async fn resolve(&self, app: &AppInfo) -> Result<Option<HeuristicMatch>, BackendError>;
}
