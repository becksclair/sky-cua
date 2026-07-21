use super::*;

#[async_trait]
impl CodexBrowserBackend for BrowserControlRuntime {
    fn daemon_generation(&self) -> String {
        self.generation.clone()
    }

    async fn connection_opened(
        &self,
        connection: CodexConnectionContext,
        outbound: crate::codex_browser_compat::CodexOutbound,
    ) -> Result<(), CodexBackendReply> {
        let principal = Principal::new(
            format!("codex:{}", connection.connection_id),
            connection.peer_uid,
        );
        self.shared
            .connections
            .lock()
            .await
            .insert(connection.connection_id, (principal, outbound));
        Ok(())
    }

    async fn request(&self, request: CodexNormalizedRequest) -> CodexBackendReply {
        match self.raw_request(request).await {
            Ok(value) => CodexBackendReply::Result(value),
            Err(message) => match message.strip_prefix("__SKY_CUA_UPSTREAM_ERROR__") {
                Some(encoded) => serde_json::from_str(encoded)
                    .map(CodexBackendReply::Error)
                    .unwrap_or_else(|_| backend_error(message)),
                None => backend_error(message),
            },
        }
    }

    async fn caller_lifecycle(
        &self,
        lifecycle: CodexCallerLifecycle,
        request: CodexNormalizedRequest,
    ) -> CodexBackendReply {
        let connections = self.shared.connections.lock().await;
        if !connections.contains_key(&request.connection.connection_id) {
            return backend_error("unknown raw browser connection".to_owned());
        }
        if let Err(message) = self.record_raw_client_open(&request.caller_provenance) {
            return backend_error(message);
        }
        drop(connections);
        self.control.events.record(
            BrowserControlEventKind::Lifecycle {
                state: format!("raw_lifecycle_{lifecycle:?}").to_lowercase(),
            },
            super::super::introspection::EventContext::default(),
        );
        // Caller lifecycle is logical-group state only. The canonical
        // extension session is never finalized, and the connection lease is
        // preserved until actual socket EOF invokes `connection_closed`.
        let principal = self
            .shared
            .connections
            .lock()
            .await
            .get(&request.connection.connection_id)
            .map(|_| principal_from_raw(&request));
        if let Some(principal) = principal {
            self.register_principal_connection(
                &request.connection.connection_id,
                principal.clone(),
            )
            .await;
            let logical_group = logical_group_key(
                request
                    .logical_identity
                    .session_id
                    .as_deref()
                    .unwrap_or(&request.connection.connection_id),
                request.logical_identity.thread_id.as_deref(),
            );
            let group_ids = self
                .shared
                .groups
                .lock()
                .await
                .iter()
                .filter(|((owner, logical, _), _)| {
                    owner == &principal.id && logical == &logical_group
                })
                .map(|(_, group)| group.clone())
                .collect::<Vec<_>>();
            for group_id in group_ids {
                if let Ok(group) = self
                    .control
                    .end_group(group_id.clone(), principal.clone())
                    .await
                {
                    self.remove_released_group_ownership(&group_id, &group)
                        .await;
                }
            }
        }
        CodexBackendReply::Result(Value::Bool(true))
    }

    async fn client_message(&self, connection_id: &str, message: Value) {
        if let Some(request_id) = message.get("id").and_then(ServerRequestId::from_value) {
            let actor = self
                .shared
                .server_request_routes
                .lock()
                .await
                .remove(&(connection_id.to_owned(), request_id));
            if let Some(actor) = actor {
                let _ = actor.send_server_message(message).await;
            }
            return;
        }

        let browser_ids = self
            .shared
            .codex_by_browser
            .lock()
            .await
            .iter()
            .filter(|(_, connections)| connections.contains(connection_id))
            .map(|(browser, _)| browser.clone())
            .collect::<Vec<_>>();
        for browser_id in browser_ids {
            if let Some(actor) = actor_for_browser(&self.shared, &browser_id) {
                let _ = actor.send_server_message(message.clone()).await;
            }
        }
    }

    async fn cancel_or_detach(&self, connection_id: &str, operation_id: &str) {
        let _ = self
            .control
            .cancel_for_client(
                OperationId(operation_id.to_owned()),
                ClientId(connection_id.to_owned()),
            )
            .await;
    }

    async fn connection_closed(&self, connection_id: &str) {
        self.record_client_closed(connection_id);
        self.shared.connections.lock().await.remove(connection_id);
        self.shared
            .codex_connection_sessions
            .lock()
            .await
            .remove(connection_id);
        self.release_connection_principals(connection_id).await;
        self.shared
            .codex_by_browser
            .lock()
            .await
            .retain(|_, connections| {
                connections.remove(connection_id);
                !connections.is_empty()
            });
        self.shared
            .server_request_routes
            .lock()
            .await
            .retain(|(connection, _), _| connection != connection_id);
    }
}

impl BrowserControlRuntime {
    pub(in crate::browser::control_plane) async fn remove_released_group_ownership(
        &self,
        group_id: &GroupId,
        group: &super::super::GroupSnapshot,
    ) {
        if matches!(group.admission, super::super::GroupAdmission::Released) {
            self.shared
                .tab_owners
                .lock()
                .await
                .retain(|_, owner| owner != group_id);
        }
    }

    async fn raw_request(&self, request: CodexNormalizedRequest) -> Result<Value, String> {
        // Codex compatibility requests bypass ServiceDaemon::handle_browser,
        // so mark them explicitly. Otherwise an active canonical bridge can
        // hit the daemon's five-minute idle exit during Browser discovery.
        crate::browser::mark_bridge_activity();
        let connection_id = request.connection.connection_id.clone();
        let connections = self.shared.connections.lock().await;
        if !connections.contains_key(&connection_id) {
            return Err("unknown raw browser connection".to_owned());
        }
        self.record_raw_client_open(&request.caller_provenance)?;
        drop(connections);
        self.initialize_ownership_indexes().await;
        let actors = self.ready_actors().await.map_err(|error| error.message)?;
        let actor = one_actor(&actors).map_err(|error| error.message)?;
        let browser = BrowserInstanceId(actor.browser_id.clone());
        let connections = self.shared.connections.lock().await;
        if !connections.contains_key(&connection_id) {
            return Err("raw browser connection closed before ownership".to_owned());
        }
        let principal = principal_from_raw(&request);
        self.register_principal_connection(&connection_id, principal.clone())
            .await;
        if let Some(session_id) = request
            .logical_identity
            .session_id
            .as_deref()
            .filter(|session_id| !session_id.is_empty())
        {
            self.shared
                .codex_connection_sessions
                .lock()
                .await
                .entry(connection_id.clone())
                .or_default()
                .insert(session_id.to_owned());
        }
        drop(connections);
        let tab = match &request.scope {
            CodexOperationScope::Tab(id) => Some(TabKey::new(browser.clone(), id)),
            _ => None,
        };
        if let Some(tab) = &tab
            && is_reentrant_cdp_continuation(&request)
            && let Some(parent_operation) = self
                .shared
                .raw_tab_parents
                .lock()
                .await
                .get(&(connection_id.clone(), tab.clone()))
                .cloned()
        {
            return self
                .execute_reentrant_codex_subrequest(&actor.actor, &request, tab, parent_operation)
                .await;
        }
        let discovery = matches!(
            request.method.as_str(),
            "getInfo" | "getTabs" | "getUserTabs"
        );
        let membership_add = matches!(
            request.method.as_str(),
            "create"
                | "createTab"
                | "open"
                | "openTab"
                | "claim"
                | "claimTab"
                | "claimUserTab"
                | "attach"
        );
        let logical_group = logical_group_key(
            request
                .logical_identity
                .session_id
                .as_deref()
                .unwrap_or(&connection_id),
            request.logical_identity.thread_id.as_deref(),
        );
        let group = self
            .default_group(&principal, &logical_group, &browser)
            .await;
        if !self
            .shared
            .connections
            .lock()
            .await
            .contains_key(&connection_id)
        {
            self.abort_closed_codex_request(&connection_id, &principal)
                .await;
            return Err("raw browser connection closed before dispatch".to_owned());
        }
        let reserved_tab = if membership_add {
            if let Some(tab) = &tab {
                let mut owners = self.shared.tab_owners.lock().await;
                if let Some(owner) = owners.get(tab)
                    && owner != &group.group_id
                {
                    return Err(
                        "tab belongs to another logical browser group; explicit handoff is required"
                            .to_owned(),
                    );
                }
                if owners.contains_key(tab) {
                    None
                } else {
                    owners.insert(tab.clone(), group.group_id.clone());
                    Some(tab.clone())
                }
            } else {
                None
            }
        } else {
            None
        };
        let lease = if let Some(tab) = &tab {
            if membership_add {
                None
            } else {
                Some(
                    self.group_for_tab(&principal, tab, Some(&group.group_id))
                        .await
                        .map_err(|error| error.message)?
                        .lease
                        .proof(),
                )
            }
        } else {
            None
        };
        let scope = if discovery || membership_add {
            OperationScope::BridgeGlobal(browser.clone())
        } else {
            match tab.clone() {
                Some(tab) => OperationScope::Tab(tab),
                None => OperationScope::BridgeGlobal(browser.clone()),
            }
        };
        let operation_id = OperationId(request.operation_id.clone());
        if let Some(tab) = &reserved_tab {
            remember_operation_reservation(
                &self.shared,
                operation_id.clone(),
                tab.clone(),
                group.group_id.clone(),
                principal.clone(),
            )
            .await;
        }
        let newly_associated = self
            .shared
            .codex_by_browser
            .lock()
            .await
            .entry(actor.browser_id.clone())
            .or_default()
            .insert(connection_id.clone());
        self.track_operation(
            operation_id.clone(),
            connection_id.clone(),
            Some(&browser),
            &scope,
            codex_class(request.class),
            &actors,
        )
        .await;
        let payload = serde_json::to_string(&IntegrationPayload::Raw {
            method: request.method.clone(),
            params: request.params.clone(),
            timeout_ms: u64::try_from(request.deadline.as_millis()).unwrap_or(u64::MAX),
            identity: BrowserSessionIdentity {
                session_id: request
                    .logical_identity
                    .session_id
                    .clone()
                    .unwrap_or_else(|| connection_id.clone()),
                thread_id: request.logical_identity.thread_id.clone(),
                turn_id: request
                    .logical_identity
                    .turn_id
                    .clone()
                    .unwrap_or_else(|| request.operation_id.clone()),
            },
        })
        .expect("raw payload serializes");
        let completion_result = self
            .control
            .submit(SubmitOperation {
                operation_id: Some(operation_id.clone()),
                canonical_fingerprint: request.canonical_fingerprint,
                upstream: UpstreamCorrelation {
                    ingress: "codex".to_owned(),
                    request_id: Some(request.upstream_id.to_string()),
                },
                client_id: ClientId(connection_id.clone()),
                principal: principal.clone(),
                group_id: Some(group.group_id.clone()),
                lease,
                scope,
                class: codex_class(request.class),
                payload,
                now_ms: now_ms(),
            })
            .await;
        let completion = match completion_result {
            Ok(completion) => completion,
            Err(error) => {
                self.clear_operation_correlations(&operation_id).await;
                if newly_associated {
                    remove_browser_connection_association(
                        &self.shared,
                        &actor.browser_id,
                        &connection_id,
                    )
                    .await;
                }
                release_operation_reservation(&self.shared, &operation_id).await;
                return Err(format!("browser admission rejected: {error:?}"));
            }
        };
        let certainty = completion.certainty.clone();
        let ambiguous = certainty == CompletionCertainty::Ambiguous;
        if !ambiguous {
            if request.class == CodexOperationClass::Mutation {
                remember_terminal_settlement_operation(&self.shared, &operation_id).await;
            }
            self.clear_operation_correlations(&operation_id).await;
        }
        let mut value: Value = match completion_response(completion) {
            Ok(value) => value,
            Err(error) => {
                release_operation_reservation_if_definitive(
                    &self.shared,
                    &operation_id,
                    &certainty,
                )
                .await;
                return Err(error.message);
            }
        };
        if request.method == "getInfo" {
            enrich_codex_get_info(
                &mut value,
                request.logical_identity.session_id.as_deref(),
                request.connection.codex_app_build_flavor.as_deref(),
                request.caller_provenance.caller,
                request.identity_synthetic,
            )?;
        }
        if membership_add
            && let Some(tab_id) =
                raw_returned_tab_id(&value).or_else(|| tab.as_ref().map(|tab| tab.tab_id.clone()))
        {
            let tab = TabKey::new(browser, tab_id);
            self.control
                .add_member(group.group_id.clone(), principal, tab.clone())
                .await
                .map_err(|error| format!("tab ownership failed: {error:?}"))?;
            self.shared
                .tab_owners
                .lock()
                .await
                .insert(tab, group.group_id);
        }
        commit_operation_reservation(&self.shared, &operation_id).await;
        Ok(value)
    }

    async fn execute_reentrant_codex_subrequest(
        &self,
        actor: &BridgeActor,
        request: &CodexNormalizedRequest,
        tab: &TabKey,
        parent_operation: OperationId,
    ) -> Result<Value, String> {
        let child_operation = OperationId(request.operation_id.clone());
        self.shared
            .settlement_parents
            .lock()
            .await
            .insert(child_operation.clone(), parent_operation.clone());
        self.control.events.record(
            BrowserControlEventKind::OperationState {
                state: "reentrant_bridge_subrequest".to_owned(),
            },
            super::super::introspection::EventContext {
                operation_id: Some(child_operation.0.clone()),
                ..Default::default()
            },
        );
        let mut bridge_request = BridgeActorRequest::new(
            request.method.clone(),
            request.params.clone(),
            child_operation.0.clone(),
            codex_class(request.class),
        );
        bridge_request.timeout = request.deadline;
        bridge_request.target_lifetime_key = operation_target(&OperationScope::Tab(tab.clone()));
        let result = actor.request(bridge_request).await;
        if !matches!(result, Err(BridgeActorError::Ambiguous)) {
            self.shared
                .settlement_parents
                .lock()
                .await
                .remove(&child_operation);
        }
        match result {
            Ok(response) => Ok(response.get("result").unwrap_or(&response).clone()),
            Err(BridgeActorError::UpstreamError(error)) => {
                Err(format!("__SKY_CUA_UPSTREAM_ERROR__{error}"))
            }
            Err(BridgeActorError::Ambiguous) => Err(format!(
                "reentrant bridge subrequest completion is ambiguous for parent {}",
                parent_operation.0
            )),
            Err(error) => Err(format!("persistent bridge failed: {error:?}")),
        }
    }

    pub(in crate::browser::control_plane) async fn abort_closed_codex_request(
        &self,
        connection_id: &str,
        principal: &Principal,
    ) {
        self.release_connection_principals(connection_id).await;
        let still_referenced = self
            .shared
            .principal_connections
            .lock()
            .await
            .get(&principal.id)
            .is_some_and(|connections| !connections.is_empty());
        if !still_referenced {
            // A close can race between the default-group lookup and renewal.
            // Re-apply disconnect after the liveness check so that late
            // renewal cannot reactivate an orphan lease.
            self.control.disconnect(principal.clone(), now_ms()).await;
        }
    }
}

fn principal_from_raw(request: &CodexNormalizedRequest) -> Principal {
    let session_id = request
        .logical_identity
        .session_id
        .as_deref()
        .filter(|session_id| !session_id.is_empty())
        .unwrap_or(&request.connection.connection_id);
    Principal::new(
        format!(
            "raw:{}:session:{session_id}",
            caller_name(request.caller_provenance.caller)
        ),
        request.connection.peer_uid,
    )
}

fn is_reentrant_cdp_continuation(request: &CodexNormalizedRequest) -> bool {
    if request.method != "executeCdp" {
        return false;
    }
    matches!(
        request.params.get("method").and_then(Value::as_str),
        Some(
            "Fetch.continueRequest"
                | "Fetch.continueResponse"
                | "Fetch.continueWithAuth"
                | "Fetch.failRequest"
                | "Fetch.fulfillRequest"
                | "Runtime.runIfWaitingForDebugger"
        )
    )
}

fn enrich_codex_get_info(
    value: &mut Value,
    session_id: Option<&str>,
    codex_app_build_flavor: Option<&str>,
    caller: BrowserCallerKind,
    identity_synthetic: bool,
) -> Result<(), String> {
    let Some(result) = value.as_object_mut() else {
        return Err("getInfo host result must be an object".to_owned());
    };
    let Some(session_id) = session_id.filter(|session_id| !session_id.is_empty()) else {
        return Err("getInfo requires a non-empty logical session".to_owned());
    };
    let metadata = result
        .entry("metadata")
        .or_insert_with(|| Value::Object(Default::default()));
    if !metadata.is_object() {
        return Err("getInfo host metadata must be an object".to_owned());
    }
    let capabilities = result.entry("capabilities").or_insert_with(|| json!({}));
    let Some(capabilities) = capabilities.as_object_mut() else {
        return Err("getInfo host capabilities must be an object".to_owned());
    };
    let tab_capabilities = capabilities.entry("tab").or_insert_with(|| json!([]));
    let Some(tab_capabilities) = tab_capabilities.as_array_mut() else {
        return Err("getInfo host tab capabilities must be an array".to_owned());
    };
    for (id, description) in [
        (
            "botDetection",
            "Report detected anti-bot challenges through the sky-cua daemon",
        ),
        (
            "browserAuth",
            "Request a sky-cua daemon browser-authentication handoff",
        ),
    ] {
        if !tab_capabilities
            .iter()
            .any(|capability| capability.get("id").and_then(Value::as_str) == Some(id))
        {
            tab_capabilities.push(json!({"id": id, "description": description}));
        }
    }
    let bridge_type = result
        .get("type")
        .and_then(Value::as_str)
        .filter(|bridge_type| !bridge_type.is_empty())
        .map(str::to_owned);
    let metadata = result["metadata"]
        .as_object_mut()
        .expect("metadata object was validated");
    metadata.insert(
        "codexSessionId".to_owned(),
        Value::String(session_id.to_owned()),
    );
    metadata.insert(
        "skyCuaBridgeTransport".to_owned(),
        Value::String("extension_native_host".to_owned()),
    );
    metadata.insert(
        "skyCuaCallerProvenance".to_owned(),
        Value::String(caller_name(caller).to_owned()),
    );
    metadata.insert(
        "skyCuaIdentitySynthetic".to_owned(),
        Value::Bool(identity_synthetic),
    );
    if let Some(bridge_type) = bridge_type {
        metadata.insert("skyCuaBridgeType".to_owned(), Value::String(bridge_type));
    }
    if let Some(flavor) = codex_app_build_flavor {
        metadata.insert(
            "codexAppBuildFlavor".to_owned(),
            Value::String(flavor.to_owned()),
        );
    }
    Ok(())
}

fn actor_for_browser(shared: &Shared, browser_id: &str) -> Option<BridgeActor> {
    let entries = shared
        .actors
        .read()
        .expect("actor registry poisoned")
        .values()
        .cloned()
        .collect::<Vec<_>>();
    canonical_ready_actors(entries)
        .into_iter()
        .find(|entry| entry.browser_id == browser_id)
        .map(|entry| entry.actor.clone())
}

async fn remove_browser_connection_association(
    shared: &Shared,
    browser_id: &str,
    connection_id: &str,
) {
    let mut associations = shared.codex_by_browser.lock().await;
    if let Some(connections) = associations.get_mut(browser_id) {
        connections.remove(connection_id);
        if connections.is_empty() {
            associations.remove(browser_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::enrich_codex_get_info;
    use serde_json::json;
    use sky_cua_platform::model::BrowserCallerKind;

    #[test]
    fn get_info_metadata_preserves_truthful_extension_identity_for_codex() {
        let mut value = json!({
            "name": "Chrome",
            "type": "extension",
            "metadata": {"extensionId": "extension-1"},
            "capabilities": {
                "tab": [{"id": "cdp", "description": "Extension CDP"}]
            }
        });
        enrich_codex_get_info(
            &mut value,
            Some("session-1"),
            Some("production-linux"),
            BrowserCallerKind::CodexDesktop,
            false,
        )
        .unwrap();
        assert_eq!(value["type"], json!("extension"));
        assert_eq!(value["metadata"]["extensionId"], json!("extension-1"));
        assert_eq!(value["metadata"]["skyCuaBridgeType"], json!("extension"));
        assert_eq!(
            value["metadata"]["skyCuaBridgeTransport"],
            json!("extension_native_host")
        );
        assert_eq!(value["metadata"]["codexSessionId"], json!("session-1"));
        assert_eq!(
            value["metadata"]["skyCuaCallerProvenance"],
            json!("codex_desktop")
        );
        assert_eq!(value["metadata"]["skyCuaIdentitySynthetic"], json!(false));
        assert_eq!(
            value["metadata"]["codexAppBuildFlavor"],
            json!("production-linux")
        );
        assert_eq!(
            value["capabilities"]["tab"],
            json!([
                {"id": "cdp", "description": "Extension CDP"},
                {
                    "id": "botDetection",
                    "description": "Report detected anti-bot challenges through the sky-cua daemon"
                },
                {
                    "id": "browserAuth",
                    "description": "Request a sky-cua daemon browser-authentication handoff"
                }
            ])
        );
    }

    #[test]
    fn get_info_requires_session_and_rejects_non_object_metadata() {
        let mut no_session = json!({"type":"extension","metadata":{"preserved":true}});
        assert_eq!(
            enrich_codex_get_info(
                &mut no_session,
                Some(""),
                None,
                BrowserCallerKind::CodexDesktop,
                false,
            ),
            Err("getInfo requires a non-empty logical session".to_owned())
        );
        assert_eq!(no_session["type"], json!("extension"));
        assert_eq!(no_session["metadata"]["preserved"], json!(true));
        assert!(no_session["metadata"].get("codexSessionId").is_none());

        let mut invalid = json!({"type":"extension","metadata":[]});
        assert_eq!(
            enrich_codex_get_info(
                &mut invalid,
                Some("session-1"),
                None,
                BrowserCallerKind::CodexDesktop,
                false,
            ),
            Err("getInfo host metadata must be an object".to_owned())
        );
        assert_eq!(invalid["type"], json!("extension"));

        let mut no_flavor = json!({"type":"extension","metadata":{"preserved":true}});
        enrich_codex_get_info(
            &mut no_flavor,
            Some("session-1"),
            None,
            BrowserCallerKind::CodexDesktop,
            false,
        )
        .unwrap();
        assert_eq!(no_flavor["type"], json!("extension"));
        assert_eq!(no_flavor["metadata"]["preserved"], json!(true));
        assert!(no_flavor["metadata"].get("codexAppBuildFlavor").is_none());

        let mut node_repl = json!({"type":"extension","metadata":{}});
        enrich_codex_get_info(
            &mut node_repl,
            Some("openclaw-session"),
            None,
            BrowserCallerKind::OpenClaw,
            true,
        )
        .unwrap();
        assert_eq!(node_repl["type"], json!("extension"));
        assert_eq!(
            node_repl["metadata"]["skyCuaCallerProvenance"],
            json!("openclaw")
        );
        assert_eq!(
            node_repl["metadata"]["skyCuaIdentitySynthetic"],
            json!(true)
        );

        let mut invalid_capabilities = json!({"type":"extension","metadata":{},"capabilities":[]});
        assert_eq!(
            enrich_codex_get_info(
                &mut invalid_capabilities,
                Some("session-1"),
                None,
                BrowserCallerKind::OpenCode,
                false,
            ),
            Err("getInfo host capabilities must be an object".to_owned())
        );

        let mut invalid_tab_capabilities =
            json!({"type":"extension","metadata":{},"capabilities":{"tab":{}}});
        assert_eq!(
            enrich_codex_get_info(
                &mut invalid_tab_capabilities,
                Some("session-1"),
                None,
                BrowserCallerKind::OpenCode,
                false,
            ),
            Err("getInfo host tab capabilities must be an array".to_owned())
        );
    }
}
