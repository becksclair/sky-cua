use async_trait::async_trait;

use crate::diagnostics::BackendError;
use crate::model::{
    AccessibilitySetupReport, ActionOutcome, ActionRequest, AppInfo, AppSelector, AppStateSnapshot,
    CaptureBackendKind, CaptureScreenMode, DoctorCheck, DoctorReadiness, DoctorReport,
    EnvironmentInfo, HeuristicMatch, InputBackendKind, PortalTokenResetOutcome,
    SemanticBackendKind, WindowInfo, WindowTarget, WindowTargetingSetupReport,
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
                recommended_next_step,
                blockers,
            },
            platform: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
        })
    }
    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError>;
    async fn setup_accessibility(&self) -> Result<AccessibilitySetupReport, BackendError> {
        Err(BackendError::new(
            crate::diagnostics::BackendErrorCode::ActionUnsupportedForEnvironment,
            "accessibility setup is only available on Linux backends",
        ))
    }
    async fn setup_window_targeting(&self) -> Result<WindowTargetingSetupReport, BackendError> {
        Err(BackendError::new(
            crate::diagnostics::BackendErrorCode::ActionUnsupportedForEnvironment,
            "window targeting setup is only available on Linux backends",
        ))
    }
    async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError> {
        Err(BackendError::new(
            crate::diagnostics::BackendErrorCode::ActionUnsupportedForEnvironment,
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
    async fn activate_window(&self, _target: WindowTarget) -> Result<ActionOutcome, BackendError> {
        Err(BackendError::new(
            crate::diagnostics::BackendErrorCode::ActionUnsupportedForEnvironment,
            "window activation is not available for this backend",
        ))
    }
    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError>;
    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError>;
    async fn reset_portal_tokens(&self) -> Result<PortalTokenResetOutcome, BackendError> {
        Err(BackendError::new(
            crate::diagnostics::BackendErrorCode::ActionUnsupportedForEnvironment,
            "portal token reset is only available on portal-backed Linux sessions",
        ))
    }
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
