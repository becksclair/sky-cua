use std::collections::BTreeMap;
use std::path::PathBuf;

use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::model::{ActionName, ActionRequest, ServiceRequest, ServiceResponse};

use crate::action_router::route_action;
use crate::approval_store::ApprovalStore;
use crate::backend_factory::create_backend;
use crate::diagnostics::error_response;
use crate::element_resolver::{resolve_action_element, resolve_target_element};
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
                            },
                        }
                    }
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

fn desktop_env_values_present() -> BTreeMap<String, String> {
    [
        "DBUS_SESSION_BUS_ADDRESS",
        "DESKTOP_SESSION",
        "DISPLAY",
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
}
