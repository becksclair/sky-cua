use super::*;
use crate::browser::control_plane::DispatchOperation;

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

async fn execute_high_level(
    request: BrowserRequest,
    identity: BrowserSessionIdentity,
) -> BrowserResponse {
    let identity = Some(identity);
    match request {
        BrowserRequest::ListTabs { target } => BrowserResponse::ListTabs {
            response: crate::browser::list_tabs_with_identity(target, identity).await,
        },
        BrowserRequest::Open { target, url } => BrowserResponse::Open {
            response: crate::browser::open_tab_with_identity(target, url, identity).await,
        },
        BrowserRequest::ClaimTab { target, tab_id } => BrowserResponse::ClaimTab {
            response: crate::browser::claim_tab_with_identity(target, tab_id, identity).await,
        },
        BrowserRequest::MoveMouse {
            target,
            tab_id,
            x,
            y,
            wait_for_arrival,
        } => BrowserResponse::MoveMouse {
            response: crate::browser::move_mouse_with_identity(
                target,
                tab_id,
                x,
                y,
                wait_for_arrival,
                identity,
            )
            .await,
        },
        BrowserRequest::Navigate {
            target,
            tab_id,
            url,
        } => BrowserResponse::Navigate {
            response: crate::browser::navigate_with_identity(target, tab_id, url, identity).await,
        },
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
        } => BrowserResponse::Click {
            response: crate::browser::click_with_identity(target, tab_id, x, y, identity).await,
        },
        BrowserRequest::ClickElement {
            target,
            tab_id,
            element_ref,
        } => BrowserResponse::Click {
            response: crate::browser::click_element_with_identity(
                target,
                tab_id,
                element_ref,
                identity,
            )
            .await,
        },
        BrowserRequest::TypeText {
            target,
            tab_id,
            text,
        } => BrowserResponse::TypeText {
            response: crate::browser::type_text_with_identity(target, tab_id, text, identity).await,
        },
        BrowserRequest::TypeTextElement {
            target,
            tab_id,
            element_ref,
            text,
        } => BrowserResponse::TypeText {
            response: crate::browser::type_text_element_with_identity(
                target,
                tab_id,
                element_ref,
                text,
                identity,
            )
            .await,
        },
        BrowserRequest::PressKey {
            target,
            tab_id,
            key,
        } => BrowserResponse::PressKey {
            response: crate::browser::press_key_with_identity(target, tab_id, key, identity).await,
        },
        BrowserRequest::Scroll {
            target,
            tab_id,
            delta_x,
            delta_y,
            x,
            y,
        } => BrowserResponse::Scroll {
            response: crate::browser::scroll_with_identity(
                target, tab_id, delta_x, delta_y, x, y, identity,
            )
            .await,
        },
        BrowserRequest::Eval {
            target,
            tab_id,
            expression,
        } => BrowserResponse::Eval {
            response: crate::browser::eval_with_policy_and_identity(
                target, tab_id, expression, true, identity,
            )
            .await,
        },
        BrowserRequest::Status => {
            unreachable!("status is handled outside the integration executor")
        }
    }
}
