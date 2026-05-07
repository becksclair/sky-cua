use async_trait::async_trait;

use crate::diagnostics::BackendError;
use crate::model::{
    ActionOutcome, ActionRequest, AppInfo, AppSelector, AppStateSnapshot, EnvironmentInfo,
    HeuristicMatch, PortalTokenResetOutcome,
};

#[async_trait]
pub trait DesktopBackend: Send + Sync {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError>;
    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError>;
    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
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
