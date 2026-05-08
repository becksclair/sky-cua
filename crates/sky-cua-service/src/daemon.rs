use std::path::PathBuf;

use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::model::{
    ActionName, ActionRequest, AppStateSnapshot, ElementNode, ServiceRequest, ServiceResponse,
};

use crate::action_router::route_action;
use crate::approval_store::ApprovalStore;
use crate::backend_factory::create_backend;
use crate::diagnostics::error_response;
use crate::session_store::SessionStore;
use crate::snapshot_manager::SnapshotManager;
use tracing::debug;

pub struct ServiceDaemon {
    backend: Box<dyn DesktopBackend>,
    sessions: SessionStore,
    snapshots: SnapshotManager,
    socket_path: PathBuf,
}

impl ServiceDaemon {
    pub fn new(socket_path: PathBuf) -> std::io::Result<Self> {
        ApprovalStore::initialize()?;
        Ok(Self {
            backend: create_backend(),
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
                let outcome = route_action(self.backend.as_ref(), request).await;
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

fn action_requires_snapshot_context(request: &ActionRequest) -> bool {
    matches!(
        request.action,
        ActionName::FocusElement
            | ActionName::ActivateElement
            | ActionName::SelectElement
            | ActionName::ExpandElement
            | ActionName::CollapseElement
            | ActionName::ToggleElement
            | ActionName::SetValue
    ) || request.element_index.is_some()
        || request.arguments.get("to_element_index").is_some()
}

#[cfg(test)]
mod tests {
    use super::action_requires_snapshot_context;
    use serde_json::json;
    use sky_cua_platform::model::{ActionName, ActionRequest};

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
}
