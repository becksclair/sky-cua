use std::path::PathBuf;

use sky_cua_linux::LinuxDesktopBackend;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::model::{
    ActionRequest, AppStateSnapshot, ElementNode, ServiceRequest, ServiceResponse,
};

use crate::action_router::route_action;
use crate::approval_store::ApprovalStore;
use crate::diagnostics::error_response;
use crate::session_store::SessionStore;
use crate::snapshot_manager::SnapshotManager;
use tracing::debug;

pub struct ServiceDaemon {
    backend: LinuxDesktopBackend,
    sessions: SessionStore,
    snapshots: SnapshotManager,
    socket_path: PathBuf,
}

impl ServiceDaemon {
    pub fn new(socket_path: PathBuf) -> std::io::Result<Self> {
        ApprovalStore::initialize()?;
        Ok(Self {
            backend: LinuxDesktopBackend::new(),
            sessions: SessionStore::new(),
            snapshots: SnapshotManager::new(8),
            socket_path,
        })
    }

    pub async fn handle(&mut self, request: ServiceRequest) -> ServiceResponse {
        self.sessions.touch().await;
        match request {
            ServiceRequest::Health => ServiceResponse::Health {
                ok: true,
                service_socket: self.socket_path.display().to_string(),
            },
            ServiceRequest::ListApps => {
                debug!("handling list_apps request");
                let environment = match self.backend.probe_environment().await {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self.backend.list_apps().await {
                    Ok(apps) => ServiceResponse::ListApps {
                        environment,
                        apps,
                        diagnostics: Vec::new(),
                    },
                    Err(error) => ServiceResponse::ListApps {
                        environment,
                        apps: Vec::new(),
                        diagnostics: vec![error.diagnostic()],
                    },
                }
            }
            ServiceRequest::GetAppState { selector } => {
                debug!(selector = ?selector, "handling get_app_state request");
                match self.backend.get_app_state(selector).await {
                    Ok(snapshot) => {
                        self.snapshots.store(snapshot.clone());
                        ServiceResponse::GetAppState {
                            snapshot: Box::new(snapshot),
                        }
                    }
                    Err(error) => error_response(error.code, error.message),
                }
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
                let outcome = route_action(&self.backend, request).await;
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
            return Err((
                "ComputerUseInactive",
                "Computer Use is not active for this action. Call get_app_state first and pass the current snapshot_id with the action.".to_string(),
            ));
        };
        let snapshot = self.snapshots.get(snapshot_id).ok_or_else(|| {
            (
                "SnapshotStale",
                format!("snapshot {snapshot_id} is not present in the service cache"),
            )
        })?;
        if !self.snapshots.is_latest(snapshot_id) {
            return Err((
                "SnapshotStale",
                format!(
                    "snapshot {snapshot_id} is no longer the latest app state. Re-run get_app_state and retry with the current snapshot_id."
                ),
            ));
        }

        request.environment = Some(snapshot.environment.clone());
        request.resolved_capture = snapshot.capture.clone();
        request.resolved_focused_app = snapshot.focused_app.clone();

        if let Some(index) = request.element_index {
            request.resolved_element = Some(resolve_element(snapshot, index)?);
        }

        if let Some(target_index) = request
            .arguments
            .get("to_element_index")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
        {
            request.resolved_target_element = Some(resolve_element(snapshot, target_index)?);
        }

        Ok(request)
    }
}

fn resolve_element(
    snapshot: &AppStateSnapshot,
    index: usize,
) -> Result<ElementNode, (&'static str, String)> {
    snapshot.elements.get(index).cloned().ok_or_else(|| {
        (
            "InvalidRequest",
            format!(
                "element_index {index} is out of range for snapshot {}",
                snapshot.snapshot_id
            ),
        )
    })
}
