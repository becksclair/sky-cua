use super::agent_cursor::{AgentCursorResponseKind, agent_cursor_status_response};
use super::capture_reuse::reuse_unchanged_capture;
use super::session_presence::session_presence_disabled_response;
use super::*;
use sky_cua_overlay_host::OverlayArrivalOutcome;
use sky_cua_platform::model::{
    AppShotCapture, AppShotRejectionReason, AppShotRequired, AppShotTrigger,
};

#[cfg(not(test))]
const ACTION_VISUAL_ARRIVAL_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(test)]
const ACTION_VISUAL_ARRIVAL_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(not(test))]
const ACTION_VISUAL_REPLY_GRACE: Duration = Duration::from_millis(50);
#[cfg(test)]
const ACTION_VISUAL_REPLY_GRACE: Duration = Duration::from_millis(25);

impl ServiceDaemon {
    pub(super) async fn handle_desktop_request(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::Health => unreachable!("health bypasses the desktop request lane"),
            ServiceRequest::Click { .. }
            | ServiceRequest::Drag { .. }
            | ServiceRequest::GetScreenshot { .. }
            | ServiceRequest::Move { .. }
            | ServiceRequest::PressKey { .. }
            | ServiceRequest::Scroll { .. }
            | ServiceRequest::TypeText { .. }
            | ServiceRequest::CancelTurn { .. } => {
                unreachable!("CUA requests bypass the legacy desktop request lane")
            }
            ServiceRequest::Browser { .. } => {
                unreachable!("browser requests bypass the desktop request lane")
            }
            ServiceRequest::CancelBrowserOperation { .. }
            | ServiceRequest::BrowserClientDisconnected { .. } => {
                unreachable!("browser lifecycle requests bypass the desktop request lane")
            }
            ServiceRequest::Phone { .. } | ServiceRequest::PhoneDirectCreateEnrollment => {
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
                    Ok(launched) => {
                        let destination_appshot = self
                            .capture_destination_appshot(Some(WindowTarget {
                                pid: Some(launched.pid),
                                ..Default::default()
                            }))
                            .await;
                        ServiceResponse::LaunchApplication {
                            pid: launched.pid,
                            diagnostics: if destination_appshot.is_none() {
                                vec![DiagnosticEntry {
                                    code: "DestinationAppShotUnavailable".into(),
                                    message: "application launched, but the exact destination window AppShot was unavailable".into(),
                                    details: None,
                                }]
                            } else {
                                vec![]
                            },
                            destination_appshot,
                        }
                    }
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
            ServiceRequest::ActivateWindow { target, context } => {
                if let Some(context) = context.as_ref()
                    && let Err(message) = context.validate()
                {
                    return cua_error_response(
                        "SKY_CUA_INVALID_CONTEXT",
                        message,
                        Some(context),
                        Some("never"),
                    );
                }
                debug!(
                    target = ?target,
                    session_id = context.as_ref().map(|context| context.session_id.as_str()),
                    turn_id = context.as_ref().map(|context| context.turn_id.as_str()),
                    deadline_ms = context.as_ref().map(CuaRequestContext::deadline_ms),
                    "handling activate_window request"
                );
                match self.backend.activate_window(target.clone()).await {
                    Ok(mut outcome) => {
                        let destination_appshot = if outcome.success {
                            self.capture_destination_appshot(Some(target)).await
                        } else {
                            None
                        };
                        if outcome.success && destination_appshot.is_none() {
                            outcome.diagnostics.push(DiagnosticEntry {
                                code: "DestinationAppShotUnavailable".to_string(),
                                message: "window activation succeeded, but an exact-window destination AppShot could not be captured".to_string(),
                                details: None,
                            });
                        }
                        ServiceResponse::ActivateWindow {
                            outcome,
                            destination_appshot,
                        }
                    }
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
                            destination_appshot: None,
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
            ServiceRequest::AppShotCapture {
                request_id,
                target,
                frontmost,
                flags,
            } => {
                let response = self
                    .handle_appshot_capture(request_id, target, frontmost, flags)
                    .await;
                if let ServiceResponse::AppShotCapture { result } = &response
                    && let Some(appshot) = result.appshot.as_ref()
                {
                    self.snapshots
                        .lock()
                        .await
                        .store_appshot((**appshot).clone());
                }
                response
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
                let mut request = *request;
                if request.snapshot_id.is_none() {
                    request.snapshot_id = self
                        .snapshots
                        .lock()
                        .await
                        .appshot(request.appshot_id.as_deref().unwrap_or_default())
                        .map(|appshot| appshot.action_snapshot.snapshot_id.clone());
                }
                if let Some(response) = self.validate_action_appshot(&request).await {
                    return response;
                }
                let arrival_deadline = tokio::time::Instant::now() + ACTION_VISUAL_ARRIVAL_TIMEOUT;
                let request = match self.enrich_action_request(request).await {
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
                    let remaining =
                        arrival_deadline.saturating_duration_since(tokio::time::Instant::now());
                    let host_timeout = remaining.saturating_sub(ACTION_VISUAL_REPLY_GRACE);
                    let arrival = if host_timeout.is_zero() {
                        None
                    } else {
                        Some(
                            tokio::time::timeout_at(arrival_deadline, async {
                                self.overlay
                                    .lock()
                                    .await
                                    .wait_for_action_visual_arrival(host_timeout)
                                    .await
                            })
                            .await,
                        )
                    };
                    match arrival {
                        Some(Ok(arrival)) => {
                            pre_dispatch_diagnostics.extend(arrival.diagnostics);
                            match arrival.outcome {
                                OverlayArrivalOutcome::Arrived => {}
                                OverlayArrivalOutcome::DeadlineElapsed => {
                                    pre_dispatch_diagnostics.push(DiagnosticEntry {
                                        code: "AgentCursorArrivalTimeout".to_string(),
                                        message: "Agent cursor did not reach the action target before the visual arrival timeout; input dispatch continued.".to_string(),
                                        details: Some(format!(
                                            "timeout_ms={} source=host_deadline",
                                            ACTION_VISUAL_ARRIVAL_TIMEOUT.as_millis()
                                        )),
                                    });
                                }
                                OverlayArrivalOutcome::Superseded => {
                                    pre_dispatch_diagnostics.push(DiagnosticEntry {
                                        code: "AgentCursorArrivalSuperseded".to_string(),
                                        message: "Agent cursor arrival wait was superseded; input dispatch continued.".to_string(),
                                        details: None,
                                    });
                                }
                                OverlayArrivalOutcome::Unavailable => {
                                    pre_dispatch_diagnostics.push(DiagnosticEntry {
                                        code: "AgentCursorArrivalUnavailable".to_string(),
                                        message: "Agent cursor arrival could not be confirmed; input dispatch continued.".to_string(),
                                        details: None,
                                    });
                                }
                            }
                        }
                        Some(Err(_)) | None => {
                            pre_dispatch_diagnostics.push(DiagnosticEntry {
                                code: "AgentCursorArrivalTimeout".to_string(),
                                message: "Agent cursor did not reach the action target before the visual arrival timeout; input dispatch continued.".to_string(),
                                details: Some(format!(
                                    "timeout_ms={} source=service_deadline",
                                    ACTION_VISUAL_ARRIVAL_TIMEOUT.as_millis()
                                )),
                            });
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

    async fn validate_action_appshot(&self, request: &ActionRequest) -> Option<ServiceResponse> {
        let appshot_id = request.appshot_id.as_deref();
        let (reason, target) = {
            let snapshots = self.snapshots.lock().await;
            match appshot_id.and_then(|id| snapshots.appshot(id)) {
                None if appshot_id.is_none() => (AppShotRejectionReason::Missing, None),
                None => (AppShotRejectionReason::Stale, None),
                Some(appshot) => {
                    let target = match &appshot.capture {
                        AppShotCapture::Desktop { window_id, .. } => Some(WindowTarget {
                            window_id: Some(window_id.clone()),
                            ..Default::default()
                        }),
                        _ => None,
                    };
                    let snapshot_ok = request
                        .snapshot_id
                        .as_deref()
                        .is_some_and(|id| id == appshot.action_snapshot.snapshot_id)
                        && snapshots.is_latest(&appshot.action_snapshot.snapshot_id);
                    if !snapshot_ok {
                        (AppShotRejectionReason::Stale, target)
                    } else {
                        (AppShotRejectionReason::WrongTarget, target)
                    }
                }
            }
        };

        // A valid appshot is accepted only when its snapshot is still latest;
        // target verification is performed against the focused window below.
        if reason == AppShotRejectionReason::WrongTarget {
            let target_id = target.as_ref().and_then(|t| t.window_id.as_deref());
            if let Ok(Some(focused)) = self.backend.focused_window().await
                && target_id == Some(focused.window_id.as_str())
            {
                return None;
            }
        }

        let capture_target = target;
        let frontmost = capture_target.is_none();
        let request_id = format!("recovery-{}", sky_cua_platform::snapshot::new_snapshot_id());
        let response = self
            .handle_appshot_capture(request_id, capture_target, frontmost, Default::default())
            .await;
        let fresh_appshot = match response {
            ServiceResponse::AppShotCapture { result } => {
                let mut appshot = result.appshot?;
                appshot.trigger = AppShotTrigger::Recovery;
                self.snapshots
                    .lock()
                    .await
                    .store_appshot((*appshot).clone());
                Some(appshot)
            }
            _ => None,
        };
        let Some(fresh_appshot) = fresh_appshot else {
            return Some(error_response(
                "AppShotRequired",
                "desktop action requires a fresh exact-window AppShot; capture was unavailable",
            ));
        };
        Some(ServiceResponse::AppShotRequired {
            rejection: Box::new(AppShotRequired {
                code: "AppShotRequired".to_string(),
                reason,
                message: "desktop state-changing actions require a present, fresh AppShot for the exact window and snapshot".to_string(),
                fresh_appshot,
            }),
        })
    }

    async fn capture_destination_appshot(
        &self,
        target: Option<WindowTarget>,
    ) -> Option<Box<sky_cua_platform::model::AppShotEnvelope>> {
        let request_id = format!(
            "destination-{}",
            sky_cua_platform::snapshot::new_snapshot_id()
        );
        let response = self
            .handle_appshot_capture(request_id, target, false, Default::default())
            .await;
        match response {
            ServiceResponse::AppShotCapture { result } => {
                let mut appshot = result.appshot?;
                appshot.trigger = AppShotTrigger::DesktopActivation;
                self.snapshots
                    .lock()
                    .await
                    .store_appshot((*appshot).clone());
                Some(appshot)
            }
            _ => None,
        }
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
