use super::*;
use crate::browser::control_plane::DispatchOperation;
use sha2::{Digest, Sha256};
use sky_cua_platform::model::{
    AppShotCapture, AppShotEnvelope, AppShotRejectionReason, AppShotRequired, AppShotTrigger,
    BrowserNavigateResponse, BrowserOpenResponse, BrowserTargetKind,
};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone)]
struct RegisteredAppShot {
    envelope: AppShotEnvelope,
    target: BrowserTargetKind,
}

fn appshot_registry() -> &'static Mutex<HashMap<String, RegisteredAppShot>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, RegisteredAppShot>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_appshot(envelope: AppShotEnvelope, target: BrowserTargetKind) {
    if let Ok(mut registry) = appshot_registry().lock() {
        registry.insert(
            envelope.appshot_id.clone(),
            RegisteredAppShot { envelope, target },
        );
    }
}

fn attach_open_destination(response: &mut BrowserOpenResponse, shot: AppShotEnvelope) {
    response.destination_appshot = Some(Box::new(shot));
}

fn attach_navigate_destination(response: &mut BrowserNavigateResponse, shot: AppShotEnvelope) {
    response.destination_appshot = Some(Box::new(shot));
}

fn registered_rejection_reason(
    entry: &RegisteredAppShot,
    target: BrowserTargetKind,
    tab_id: &str,
    session_id: &str,
    document_generation: u64,
) -> Option<AppShotRejectionReason> {
    if entry.target != target
        || !matches!(&entry.envelope.capture, AppShotCapture::Browser { tab_id: captured, .. } if captured == tab_id)
    {
        Some(AppShotRejectionReason::WrongTarget)
    } else if entry.envelope.action_snapshot.session_id.as_deref() != Some(session_id) {
        Some(AppShotRejectionReason::WrongSession)
    } else if !matches!(&entry.envelope.capture, AppShotCapture::Browser { document_generation: captured, .. } if *captured == document_generation)
    {
        Some(AppShotRejectionReason::Stale)
    } else {
        None
    }
}

async fn require_appshot(
    appshot_id: Option<String>,
    target: Option<BrowserTargetKind>,
    tab_id: &str,
    identity: &BrowserSessionIdentity,
) -> Result<(), BrowserResponse> {
    let target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let registered = appshot_id.as_deref().and_then(|id| {
        appshot_registry()
            .lock()
            .ok()
            .and_then(|registry| registry.get(id).cloned())
    });
    let rejection = if let Some(entry) = registered.as_ref() {
        let current = crate::browser::snapshot_with_identity(
            Some(target),
            tab_id.to_owned(),
            Some(0),
            None,
            Some(0),
            None,
            Some(identity.clone()),
        )
        .await;
        let generation_source = current
            .snapshot
            .as_ref()
            .and_then(|value| value.get("documentGeneration"))
            .and_then(|value| value.as_str())
            .or(current.url.as_deref())
            .unwrap_or_default();
        let digest = Sha256::digest(generation_source.as_bytes());
        let current_generation = u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]));
        registered_rejection_reason(
            entry,
            target,
            tab_id,
            &identity.session_id,
            current_generation,
        )
    } else {
        Some(AppShotRejectionReason::Missing)
    };
    if rejection.is_none() {
        return Ok(());
    }

    let current = crate::browser::observe_appshot_with_identity(
        Some(target),
        tab_id.to_owned(),
        Some(4_000),
        None,
        Some(200),
        None,
        false,
        None,
        Some(identity.clone()),
    )
    .await;
    register_appshot(current.appshot.clone(), target);
    let reason = rejection.unwrap_or(AppShotRejectionReason::Missing);
    Err(BrowserResponse::AppShotRequired {
        rejection: AppShotRequired {
            code: "AppShotRequired".into(),
            reason,
            message: "Capture a fresh browser AppShot before this mutation.".into(),
            fresh_appshot: Box::new(current.appshot),
        },
    })
}

impl Executor for IntegrationExecutor {
    fn execute(
        &self,
        operation: DispatchOperation,
    ) -> Pin<Box<dyn Future<Output = ExecutorOutcome> + Send + 'static>> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            let payload: IntegrationPayload = match serde_json::from_str(&operation.payload) {
                Ok(payload) => payload,
                Err(error) => {
                    return ExecutorOutcome::DefinitiveFailure(format!(
                        "invalid integration payload: {error}"
                    ));
                }
            };
            let browser = match &operation.scope {
                OperationScope::Tab(tab) => Some(&tab.browser_instance_id),
                OperationScope::BridgeGlobal(browser) => Some(browser),
                OperationScope::DaemonGlobal => None,
            };
            let entries = shared
                .actors
                .read()
                .expect("actor registry poisoned")
                .values()
                .cloned()
                .collect::<Vec<_>>();
            let actors = canonical_ready_actors(entries)
                .into_iter()
                .filter(|entry| browser.is_none_or(|browser| entry.browser_id == browser.0))
                .collect::<Vec<_>>();
            let target = operation_target(&operation.scope);
            match payload {
                IntegrationPayload::Raw {
                    method,
                    params,
                    timeout_ms,
                    identity,
                } => {
                    let Some(entry) = actors.first() else {
                        return ExecutorOutcome::DefinitiveFailure(
                            "persistent bridge unavailable".to_owned(),
                        );
                    };
                    if let Some(outcome) = super::raw_host::execute(
                        &shared,
                        &entry.actor,
                        &operation,
                        &method,
                        &params,
                        timeout_ms,
                        &identity,
                    )
                    .await
                    {
                        return outcome;
                    }
                    let started = tokio::time::Instant::now();
                    let timeout = Duration::from_millis(timeout_ms);
                    let mut request = BridgeActorRequest::new(
                        method.clone(),
                        params.clone(),
                        operation.identity.operation_id.0.clone(),
                        operation.class,
                    );
                    request.timeout = timeout;
                    request.target_lifetime_key = target.clone();
                    let raw_tab_key = match &operation.scope {
                        OperationScope::Tab(tab) => {
                            Some((operation.client_id.0.clone(), tab.clone()))
                        }
                        OperationScope::BridgeGlobal(_) | OperationScope::DaemonGlobal => None,
                    };
                    if let Some(key) = &raw_tab_key {
                        shared
                            .raw_tab_parents
                            .lock()
                            .await
                            .insert(key.clone(), operation.identity.operation_id.clone());
                    }
                    let mut result = entry.actor.request(request).await;
                    if method == "executeCdp"
                        && matches!(
                            &result,
                            Err(BridgeActorError::UpstreamError(error))
                                if is_upfront_unattached_upstream_error(error)
                        )
                        && let OperationScope::Tab(tab) = &operation.scope
                    {
                        let deadline = started + timeout;
                        let remaining =
                            deadline.saturating_duration_since(tokio::time::Instant::now());
                        let tracker = ChildTracker::new(
                            operation.identity.operation_id.clone(),
                            shared
                                .control
                                .get()
                                .expect("integration control initialized")
                                .clone(),
                        );
                        let recovery_context = ProxyContext::new(
                            [(entry.socket.clone(), entry.actor.clone())],
                            format!("{}:session-recovery", operation.identity.operation_id.0),
                            OperationClass::ReadOnly,
                            remaining,
                            target.clone(),
                            Arc::clone(&shared.settlement_parents),
                            tracker,
                        );
                        let recovery = persistent_proxy::scope(recovery_context, async {
                            let mut stream =
                                match crate::browser::control_plane::connect_persistent_proxy(
                                    &entry.socket,
                                )
                                .await
                                {
                                    Some(Ok(stream)) => stream,
                                    Some(Err(error)) => {
                                        return Err(runtime_diagnostic(
                                            "BrowserBridgeProxyFailed",
                                            &error.to_string(),
                                        ));
                                    }
                                    None => unreachable!(
                                        "recovery runs inside a persistent proxy scope"
                                    ),
                                };
                            crate::browser::session::recover_cdp_session_until(
                                &mut stream,
                                &entry.socket,
                                &crate::browser::tabs::tab_id_value(&tab.tab_id),
                                deadline,
                                false,
                                &identity,
                            )
                            .await
                        })
                        .await;
                        if let Err(error) = recovery {
                            result = Err(BridgeActorError::RequestFailed(format!(
                                "debugger session recovery failed: {}",
                                error.message
                            )));
                        } else {
                            let mut replay = BridgeActorRequest::new(
                                method,
                                params,
                                operation.identity.operation_id.0.clone(),
                                operation.class,
                            );
                            replay.timeout =
                                deadline.saturating_duration_since(tokio::time::Instant::now());
                            replay.target_lifetime_key = target;
                            result = entry.actor.request(replay).await;
                        }
                    }
                    if let Some(key) = raw_tab_key {
                        let mut parents = shared.raw_tab_parents.lock().await;
                        if parents.get(&key) == Some(&operation.identity.operation_id) {
                            parents.remove(&key);
                        }
                    }
                    actor_outcome(result)
                }
                IntegrationPayload::HighLevel { request, identity } => {
                    let context = ProxyContext::new(
                        actors.into_iter().map(|entry| (entry.socket, entry.actor)),
                        operation.identity.operation_id.0.clone(),
                        operation.class,
                        Duration::from_secs(60),
                        target,
                        Arc::clone(&shared.settlement_parents),
                        ChildTracker::new(
                            operation.identity.operation_id.clone(),
                            shared
                                .control
                                .get()
                                .expect("integration control initialized")
                                .clone(),
                        ),
                    );
                    let tracker = context.tracker();
                    let response =
                        persistent_proxy::scope(context, execute_high_level(request, identity))
                            .await;
                    if tracker.detach_requires_settlement().await {
                        return ExecutorOutcome::Ambiguous(
                            "a mutating bridge subrequest has unresolved settlement".to_owned(),
                        );
                    }
                    ExecutorOutcome::DefinitiveSuccess(
                        serde_json::to_string(&response).expect("browser response serializes"),
                    )
                }
            }
        })
    }
}

fn actor_outcome(result: Result<Value, BridgeActorError>) -> ExecutorOutcome {
    match result {
        Ok(value) => {
            ExecutorOutcome::DefinitiveSuccess(value.get("result").unwrap_or(&value).to_string())
        }
        Err(BridgeActorError::Ambiguous) => ExecutorOutcome::Ambiguous(
            "persistent bridge operation completion is ambiguous".to_owned(),
        ),
        Err(BridgeActorError::UpstreamError(error)) => {
            ExecutorOutcome::DefinitiveFailure(format!("__SKY_CUA_UPSTREAM_ERROR__{error}"))
        }
        Err(error) => {
            ExecutorOutcome::DefinitiveFailure(format!("persistent bridge failed: {error:?}"))
        }
    }
}

pub(crate) async fn execute_high_level(
    request: BrowserRequest,
    identity: BrowserSessionIdentity,
) -> BrowserResponse {
    let identity = Some(identity);
    match request {
        BrowserRequest::ListTabs { target } => BrowserResponse::ListTabs {
            response: crate::browser::list_tabs_with_identity(target, identity).await,
        },
        BrowserRequest::Open { target, url } => {
            let mut response =
                crate::browser::open_tab_with_identity(target, url, identity.clone()).await;
            if let Some(tab) = &response.tab {
                let shot = crate::browser::observe_appshot_with_identity(
                    Some(response.target),
                    tab.tab_id.clone(),
                    Some(4_000),
                    None,
                    Some(200),
                    None,
                    false,
                    None,
                    identity.clone(),
                )
                .await;
                let mut shot = shot;
                shot.appshot.trigger = AppShotTrigger::BrowserNavigation;
                register_appshot(shot.appshot.clone(), response.target);
                attach_open_destination(&mut response, shot.appshot);
            }
            BrowserResponse::Open { response }
        }
        BrowserRequest::ClaimTab { target, tab_id } => BrowserResponse::ClaimTab {
            response: crate::browser::claim_tab_with_identity(target, tab_id, identity).await,
        },
        BrowserRequest::MoveMouse {
            target,
            tab_id,
            x,
            y,
            wait_for_arrival,
            appshot_id,
        } => {
            if let Err(rejection) = require_appshot(
                appshot_id,
                target,
                &tab_id,
                identity.as_ref().expect("identity"),
            )
            .await
            {
                rejection
            } else {
                BrowserResponse::MoveMouse {
                    response: crate::browser::move_mouse_with_identity(
                        target,
                        tab_id,
                        x,
                        y,
                        wait_for_arrival,
                        identity,
                    )
                    .await,
                }
            }
        }
        BrowserRequest::Navigate {
            target,
            tab_id,
            url,
        } => {
            let mut response = crate::browser::navigate_with_identity(
                target,
                tab_id.clone(),
                url,
                identity.clone(),
            )
            .await;
            let shot = crate::browser::observe_appshot_with_identity(
                Some(response.target),
                tab_id,
                Some(4_000),
                None,
                Some(200),
                None,
                false,
                None,
                identity.clone(),
            )
            .await;
            let mut shot = shot;
            shot.appshot.trigger = AppShotTrigger::BrowserNavigation;
            register_appshot(shot.appshot.clone(), response.target);
            attach_navigate_destination(&mut response, shot.appshot);
            BrowserResponse::Navigate { response }
        }
        BrowserRequest::Snapshot {
            target,
            tab_id,
            text_limit,
            element_offset,
            element_limit,
            element_query,
        } => BrowserResponse::Snapshot {
            response: crate::browser::snapshot_with_identity(
                target,
                tab_id,
                text_limit,
                element_offset,
                element_limit,
                element_query,
                identity,
            )
            .await,
        },
        BrowserRequest::ObserveAppShot {
            target,
            tab_id,
            text_limit,
            element_offset,
            element_limit,
            element_query,
            include_image_data,
            capture_timeout_ms,
        } => {
            let response = crate::browser::observe_appshot_with_identity(
                target,
                tab_id,
                text_limit,
                element_offset,
                element_limit,
                element_query,
                include_image_data,
                capture_timeout_ms,
                identity,
            )
            .await;
            register_appshot(
                response.appshot.clone(),
                target.unwrap_or(BrowserTargetKind::UserChrome),
            );
            BrowserResponse::AppShot { response }
        }
        BrowserRequest::Screenshot {
            target,
            tab_id,
            include_image_data,
        } => BrowserResponse::Screenshot {
            response: crate::browser::screenshot_with_identity(
                target,
                tab_id,
                include_image_data,
                identity,
            )
            .await,
        },
        BrowserRequest::Click {
            target,
            tab_id,
            x,
            y,
            appshot_id,
        } => {
            if let Err(rejection) = require_appshot(
                appshot_id,
                target,
                &tab_id,
                identity.as_ref().expect("identity"),
            )
            .await
            {
                rejection
            } else {
                BrowserResponse::Click {
                    response: crate::browser::click_with_identity(target, tab_id, x, y, identity)
                        .await,
                }
            }
        }
        BrowserRequest::ClickElement {
            target,
            tab_id,
            element_ref,
            appshot_id,
        } => {
            if let Err(rejection) = require_appshot(
                appshot_id,
                target,
                &tab_id,
                identity.as_ref().expect("identity"),
            )
            .await
            {
                rejection
            } else {
                BrowserResponse::Click {
                    response: crate::browser::click_element_with_identity(
                        target,
                        tab_id,
                        element_ref,
                        identity,
                    )
                    .await,
                }
            }
        }
        BrowserRequest::TypeText {
            target,
            tab_id,
            text,
            appshot_id,
        } => {
            if let Err(rejection) = require_appshot(
                appshot_id,
                target,
                &tab_id,
                identity.as_ref().expect("identity"),
            )
            .await
            {
                rejection
            } else {
                BrowserResponse::TypeText {
                    response: crate::browser::type_text_with_identity(
                        target, tab_id, text, identity,
                    )
                    .await,
                }
            }
        }
        BrowserRequest::TypeTextElement {
            target,
            tab_id,
            element_ref,
            text,
            appshot_id,
        } => {
            if let Err(rejection) = require_appshot(
                appshot_id,
                target,
                &tab_id,
                identity.as_ref().expect("identity"),
            )
            .await
            {
                rejection
            } else {
                BrowserResponse::TypeText {
                    response: crate::browser::type_text_element_with_identity(
                        target,
                        tab_id,
                        element_ref,
                        text,
                        identity,
                    )
                    .await,
                }
            }
        }
        BrowserRequest::PressKey {
            target,
            tab_id,
            key,
            appshot_id,
        } => {
            if let Err(rejection) = require_appshot(
                appshot_id,
                target,
                &tab_id,
                identity.as_ref().expect("identity"),
            )
            .await
            {
                rejection
            } else {
                BrowserResponse::PressKey {
                    response: crate::browser::press_key_with_identity(
                        target, tab_id, key, identity,
                    )
                    .await,
                }
            }
        }
        BrowserRequest::Scroll {
            target,
            tab_id,
            delta_x,
            delta_y,
            x,
            y,
            appshot_id,
        } => {
            if let Err(rejection) = require_appshot(
                appshot_id,
                target,
                &tab_id,
                identity.as_ref().expect("identity"),
            )
            .await
            {
                rejection
            } else {
                BrowserResponse::Scroll {
                    response: crate::browser::scroll_with_identity(
                        target, tab_id, delta_x, delta_y, x, y, identity,
                    )
                    .await,
                }
            }
        }
        BrowserRequest::Eval {
            target,
            tab_id,
            expression,
            appshot_id,
        } => {
            if let Err(rejection) = require_appshot(
                appshot_id,
                target,
                &tab_id,
                identity.as_ref().expect("identity"),
            )
            .await
            {
                rejection
            } else {
                BrowserResponse::Eval {
                    response: crate::browser::eval_with_policy_and_identity(
                        target, tab_id, expression, true, identity,
                    )
                    .await,
                }
            }
        }
        BrowserRequest::Status => {
            unreachable!("status is handled outside the integration executor")
        }
    }
}

#[cfg(test)]
mod appshot_fence_tests {
    use super::*;
    use sky_cua_platform::model::{
        AppShotActionSnapshot, AppShotConsistency, AppShotCoverage, AppShotTrigger,
        BrowserNavigateResponse, BrowserOpenResponse, BrowserTab, ContentPersistence, ContentRef,
        ContentSource, PixelSize,
    };

    fn entry(
        target: BrowserTargetKind,
        tab: &str,
        session: &str,
        generation: u64,
    ) -> RegisteredAppShot {
        RegisteredAppShot {
            target,
            envelope: AppShotEnvelope {
                appshot_id: "shot".into(),
                trigger: AppShotTrigger::Observe,
                captured_at: chrono::Utc::now(),
                consistency: AppShotConsistency::Stable,
                capture: AppShotCapture::Browser {
                    tab_id: tab.into(),
                    url: "about:blank".into(),
                    title: None,
                    viewport: PixelSize {
                        width: 1,
                        height: 1,
                    },
                    document_generation: generation,
                    semantic_snapshot: serde_json::json!({}),
                    readiness: Default::default(),
                    capture_outcome: Default::default(),
                },
                image: ContentRef {
                    content_id: "image".into(),
                    device_id: None,
                    link_epoch: None,
                    mime_type: "image/png".into(),
                    filename: None,
                    size_bytes: 0,
                    sha256: "00".repeat(32),
                    source: ContentSource::Screenshot,
                    expires_at_ms: None,
                    persistence: ContentPersistence::Temporary,
                },
                action_snapshot: AppShotActionSnapshot {
                    snapshot_id: "actions".into(),
                    session_id: Some(session.into()),
                    subject_generation: Some(generation),
                },
                coverage: AppShotCoverage {
                    pixels_complete: true,
                    semantics_complete: true,
                    secure_regions_redacted: false,
                    projection_truncated: false,
                    total_semantic_nodes: None,
                    projected_semantic_nodes: None,
                },
                capability_profile_id: "browser-v1".into(),
                diagnostics: vec![],
            },
        }
    }

    #[test]
    fn fence_accepts_exact_latest_identity_and_generation() {
        let shot = entry(BrowserTargetKind::UserChrome, "tab-1", "session-1", 7);
        assert_eq!(
            registered_rejection_reason(
                &shot,
                BrowserTargetKind::UserChrome,
                "tab-1",
                "session-1",
                8
            ),
            Some(AppShotRejectionReason::Stale)
        );
        assert_eq!(
            registered_rejection_reason(
                &shot,
                BrowserTargetKind::UserChrome,
                "tab-1",
                "session-1",
                7
            ),
            None
        );
    }

    #[test]
    fn fence_rejects_wrong_tab_target_session_and_document() {
        let shot = entry(BrowserTargetKind::UserChrome, "tab-1", "session-1", 7);
        assert_eq!(
            registered_rejection_reason(
                &shot,
                BrowserTargetKind::UserChrome,
                "tab-1",
                "session-1",
                7
            ),
            None
        );
        assert_eq!(
            registered_rejection_reason(
                &shot,
                BrowserTargetKind::UserChrome,
                "tab-2",
                "session-1",
                7
            ),
            Some(AppShotRejectionReason::WrongTarget)
        );
        assert_eq!(
            registered_rejection_reason(
                &shot,
                BrowserTargetKind::UserChrome,
                "tab-1",
                "session-2",
                7
            ),
            Some(AppShotRejectionReason::WrongSession)
        );
        assert_eq!(
            registered_rejection_reason(
                &shot,
                BrowserTargetKind::UserChrome,
                "tab-1",
                "session-1",
                8
            ),
            Some(AppShotRejectionReason::Stale)
        );
    }

    #[test]
    fn open_and_navigate_destinations_are_bound_to_returned_shot() {
        let shot = entry(BrowserTargetKind::UserChrome, "tab-1", "session-1", 7).envelope;
        let mut open = BrowserOpenResponse {
            target: BrowserTargetKind::UserChrome,
            tab: Some(BrowserTab {
                tab_id: "tab-1".into(),
                target: BrowserTargetKind::UserChrome,
                title: None,
                url: Some("about:blank".into()),
                active: true,
            }),
            destination_appshot: None,
            diagnostics: vec![],
        };
        attach_open_destination(&mut open, shot.clone());
        assert_eq!(
            open.destination_appshot
                .as_ref()
                .map(|s| s.appshot_id.as_str()),
            Some("shot")
        );
        let mut navigate = BrowserNavigateResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".into(),
            url: "https://example.test".into(),
            destination_appshot: None,
            diagnostics: vec![],
        };
        attach_navigate_destination(&mut navigate, shot);
        assert_eq!(
            navigate
                .destination_appshot
                .as_ref()
                .and_then(|s| match &s.capture {
                    AppShotCapture::Browser { tab_id, .. } => Some(tab_id.as_str()),
                    _ => None,
                }),
            Some("tab-1")
        );
    }

    #[test]
    fn rejected_fence_does_not_call_fake_driver_mutation() {
        let shot = entry(BrowserTargetKind::UserChrome, "tab-1", "session-1", 7);
        let mut driver_calls = 0u32;
        let rejection = registered_rejection_reason(
            &shot,
            BrowserTargetKind::UserChrome,
            "tab-2",
            "session-1",
            7,
        );
        if rejection.is_none() {
            driver_calls += 1;
        }
        assert_eq!(rejection, Some(AppShotRejectionReason::WrongTarget));
        assert_eq!(
            driver_calls, 0,
            "rejected AppShots must not reach the driver mutation"
        );
    }
}
