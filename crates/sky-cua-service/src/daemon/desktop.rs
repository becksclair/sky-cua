use super::agent_cursor::{AgentCursorResponseKind, agent_cursor_status_response};
use super::capture_reuse::reuse_unchanged_capture;
use super::session_presence::session_presence_disabled_response;
use super::*;

const ACTION_VISUAL_ARRIVAL_POLL_INTERVAL: Duration = Duration::from_millis(16);
const ACTION_VISUAL_ARRIVAL_TIMEOUT: Duration = Duration::from_secs(8);

impl ServiceDaemon {
    pub(super) async fn handle_desktop_request(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::Health => unreachable!("health bypasses the desktop request lane"),
            ServiceRequest::Browser { .. } => {
                unreachable!("browser requests bypass the desktop request lane")
            }
            ServiceRequest::Phone { .. } => {
                unreachable!("phone requests bypass the desktop request lane")
            }
            ServiceRequest::SessionPresence { action } => match action {
                SessionPresenceAction::Ensure(intent) => {
                    if !self.session_presence_config.enabled {
                        return session_presence_disabled_response();
                    }
                    match self.backend.ensure_session_presence(intent).await {
                        Ok(status) => {
                            let mut held = self.session_presence_held.lock().await;
                            *held = true;
                            ServiceResponse::SessionPresence { status }
                        }
                        Err(error) => error_response(error.code, error.message),
                    }
                }
                SessionPresenceAction::Release { relock } => {
                    if !self.session_presence_config.enabled {
                        return session_presence_disabled_response();
                    }
                    match self.backend.release_session_presence(relock).await {
                        Ok(status) => {
                            let mut held = self.session_presence_held.lock().await;
                            *held = false;
                            ServiceResponse::SessionPresence { status }
                        }
                        Err(error) => error_response(error.code, error.message),
                    }
                }
                SessionPresenceAction::Status => ServiceResponse::SessionPresence {
                    status: self.backend.session_presence_status().await,
                },
            },
            ServiceRequest::Doctor => {
                match self.with_desktop_deadline(self.backend.doctor()).await {
                    Ok(report) => ServiceResponse::Doctor {
                        report: Box::new(report),
                    },
                    Err(error) => error_response(error.code, error.message),
                }
            }
            ServiceRequest::SetupAccessibility => {
                match self
                    .with_desktop_deadline(self.backend.setup_accessibility())
                    .await
                {
                    Ok(report) => ServiceResponse::SetupAccessibility {
                        report: Box::new(report),
                    },
                    Err(error) => error_response(error.code, error.message),
                }
            }
            ServiceRequest::SetupWindowTargeting => {
                match self
                    .with_desktop_deadline(self.backend.setup_window_targeting())
                    .await
                {
                    Ok(report) => ServiceResponse::SetupWindowTargeting {
                        report: Box::new(report),
                    },
                    Err(error) => error_response(error.code, error.message),
                }
            }
            ServiceRequest::LaunchApplication { command, args } => {
                // Isolation gating lives at the client: the `desktop_launch_app`
                // MCP tool refuses unless the session is isolated. The daemon is
                // intentionally ignorant of xpra and launches into whatever
                // environment it was spawned with, so leak-safety rests on the
                // client gate plus the isolated daemon's sandbox spawn env —
                // consistent with the "client orchestrates, daemon ignorant"
                // design and the project's non-security-hardening posture.
                debug!(command = %command, "handling launch_application request");
                match self.backend.launch_application(&command, &args).await {
                    Ok(launched) => ServiceResponse::LaunchApplication { pid: launched.pid },
                    Err(error) => error_response(error.code, error.message),
                }
            }
            ServiceRequest::ListApps => {
                debug!("handling list_apps request");
                let environment = match self
                    .with_desktop_deadline(self.backend.probe_environment())
                    .await
                {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self.with_desktop_deadline(self.backend.list_apps()).await {
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
                let environment = match self
                    .with_desktop_deadline(self.backend.probe_environment())
                    .await
                {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self
                    .with_desktop_deadline(self.backend.list_windows())
                    .await
                {
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
                let environment = match self
                    .with_desktop_deadline(self.backend.probe_environment())
                    .await
                {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self
                    .with_desktop_deadline(self.backend.focused_window())
                    .await
                {
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
                let capture_guard = if capture_screen != CaptureScreenMode::Never {
                    Some(self.overlay.lock().await.prepare_for_capture())
                } else {
                    None
                };
                match self
                    .with_desktop_deadline(self.backend.get_app_state(selector, capture_screen))
                    .await
                {
                    Ok(mut snapshot) => {
                        let reused_capture = if capture_screen == CaptureScreenMode::IfChanged {
                            let snapshots = self.snapshots.lock().await;
                            reuse_unchanged_capture(&mut snapshot, snapshots.latest())
                        } else {
                            false
                        };
                        if reused_capture {
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
                        {
                            let mut overlay = self.overlay.lock().await;
                            overlay.apply_to_snapshot(&mut snapshot);
                            if let Some(capture_guard) = capture_guard {
                                snapshot
                                    .diagnostics
                                    .extend(overlay.restore_after_capture(capture_guard));
                            }
                        }
                        self.snapshots.lock().await.store(snapshot.clone());
                        ServiceResponse::GetAppState {
                            snapshot: Box::new(snapshot),
                        }
                    }
                    Err(error) => {
                        if let Some(capture_guard) = capture_guard {
                            let _ = self
                                .overlay
                                .lock()
                                .await
                                .restore_after_capture(capture_guard);
                        }
                        error_response(error.code, error.message)
                    }
                }
            }
            ServiceRequest::Screenshot {
                target,
                display_target,
            } => {
                if screenshot_selector_count(target.as_ref(), display_target.as_ref()) > 1 {
                    return error_response(
                        BackendErrorCode::InvalidRequest.as_str(),
                        "screenshot accepts exactly one capture selector: window target or display target",
                    );
                }
                debug!(
                    target = ?target,
                    display_target = ?display_target,
                    "handling screenshot request"
                );
                let capture_guard = Some(self.overlay.lock().await.prepare_for_capture());
                match self
                    .with_desktop_deadline(self.backend.screenshot(target, display_target))
                    .await
                {
                    Ok(mut snapshot) => {
                        if let Some(capture_guard) = capture_guard.as_ref() {
                            snapshot
                                .diagnostics
                                .extend(capture_guard.diagnostics.iter().cloned());
                        }
                        {
                            let mut overlay = self.overlay.lock().await;
                            overlay.apply_to_snapshot(&mut snapshot);
                            if let Some(capture_guard) = capture_guard {
                                snapshot
                                    .diagnostics
                                    .extend(overlay.restore_after_capture(capture_guard));
                            }
                        }
                        self.snapshots.lock().await.store(snapshot.clone());
                        ServiceResponse::Screenshot {
                            snapshot: Box::new(snapshot),
                        }
                    }
                    Err(error) => {
                        if let Some(capture_guard) = capture_guard {
                            let _ = self
                                .overlay
                                .lock()
                                .await
                                .restore_after_capture(capture_guard);
                        }
                        error_response(error.code, error.message)
                    }
                }
            }
            ServiceRequest::AgentCursorStatus => {
                let status = self.overlay.lock().await.status();
                agent_cursor_status_response(status, AgentCursorResponseKind::Status)
            }
            ServiceRequest::SetAgentCursor { state } => {
                let status = self.overlay.lock().await.set_state(state);
                agent_cursor_status_response(status, AgentCursorResponseKind::Set)
            }
            ServiceRequest::HideAgentCursor { reason } => {
                let status = self.overlay.lock().await.hide(reason);
                agent_cursor_status_response(status, AgentCursorResponseKind::Hide)
            }
            ServiceRequest::ShowAgentCursor => {
                let status = self.overlay.lock().await.show();
                agent_cursor_status_response(status, AgentCursorResponseKind::Show)
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
                // Publish the target and gesture first, then hold physical
                // input until the host confirms that arrival-gated feedback
                // has begun. This keeps the real click/drag from overtaking
                // the visible agent cursor.
                let preparation = {
                    let mut overlay = self.overlay.lock().await;
                    overlay.prepare_action_visual(&request)
                };
                let mut pre_dispatch_diagnostics = preparation.diagnostics;
                if preparation.wait_for_arrival {
                    let started_at = tokio::time::Instant::now();
                    loop {
                        tokio::time::sleep(ACTION_VISUAL_ARRIVAL_POLL_INTERVAL).await;
                        let arrival = self.overlay.lock().await.poll_action_visual_arrival();
                        pre_dispatch_diagnostics.extend(arrival.diagnostics);
                        if arrival.arrived {
                            break;
                        }
                        if started_at.elapsed() >= ACTION_VISUAL_ARRIVAL_TIMEOUT {
                            pre_dispatch_diagnostics.push(DiagnosticEntry {
                                code: "AgentCursorArrivalTimeout".to_string(),
                                message: "Agent cursor did not reach the action target before the visual arrival timeout; input dispatch continued.".to_string(),
                                details: Some(format!(
                                    "timeout_ms={}",
                                    ACTION_VISUAL_ARRIVAL_TIMEOUT.as_millis()
                                )),
                            });
                            break;
                        }
                    }
                }
                let mut outcome = route_action(self.backend.as_ref(), request.clone()).await;
                outcome.diagnostics.extend(pre_dispatch_diagnostics);
                let cursor_diagnostics = self
                    .overlay
                    .lock()
                    .await
                    .update_from_action(&request, &mut outcome);
                outcome.diagnostics.extend(cursor_diagnostics);
                ServiceResponse::ExecuteAction { outcome }
            }
        }
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
        let snapshot = {
            let snapshots = self.snapshots.lock().await;
            if let Some(snapshot) = snapshots.get_if_latest(snapshot_id) {
                snapshot.clone()
            } else if snapshots.get(snapshot_id).is_some() {
                return Err((
                    "SnapshotStale",
                    format!(
                        "snapshot {snapshot_id} is no longer the latest app state. Re-run get_app_state and retry with the current snapshot_id."
                    ),
                ));
            } else {
                return Err((
                    "SnapshotStale",
                    format!("snapshot {snapshot_id} is not present in the service cache"),
                ));
            }
        };

        request.environment = Some(snapshot.environment.clone());
        request.resolved_capture = snapshot.capture.clone();
        request.resolved_focused_app = snapshot.focused_app.clone();

        request.resolved_element = resolve_action_element(
            &snapshot,
            &request.action,
            request.element_index,
            &request.arguments,
        )?;
        request.resolved_target_element = resolve_target_element(&snapshot, &request.arguments)?;

        Ok(request)
    }
}

pub(super) fn action_requires_snapshot_context(request: &ActionRequest) -> bool {
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

fn screenshot_selector_count(
    target: Option<&WindowTarget>,
    display_target: Option<&DisplayTarget>,
) -> usize {
    usize::from(target.is_some()) + usize::from(display_target.is_some())
}
