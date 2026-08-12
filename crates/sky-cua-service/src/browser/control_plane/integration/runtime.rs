use super::*;

impl BrowserControlRuntime {
    pub(crate) async fn shutdown(&self) {
        let actors = self
            .shared
            .actors
            .read()
            .expect("actor registry poisoned")
            .values()
            .map(|entry| entry.actor.clone())
            .collect::<Vec<_>>();
        for actor in &actors {
            actor.shutdown().await;
        }
        for actor in actors {
            if tokio::time::timeout(Duration::from_secs(5), actor.wait_closed())
                .await
                .is_err()
            {
                tracing::warn!(
                    socket = %actor.health().socket_path.display(),
                    "timed out waiting for browser bridge actor shutdown"
                );
            }
        }
    }

    pub(crate) fn new() -> Arc<Self> {
        Self::new_with_owner_mode(BridgeOwnerMode::Hybrid)
    }

    pub(crate) fn new_with_mode(mode: crate::browser::BrowserControlMode) -> Arc<Self> {
        let owner_mode = match mode {
            crate::browser::BrowserControlMode::Legacy
            | crate::browser::BrowserControlMode::Hybrid => BridgeOwnerMode::Hybrid,
            crate::browser::BrowserControlMode::Strict => BridgeOwnerMode::Strict,
        };
        Self::new_with_owner_mode(owner_mode)
    }

    fn new_with_owner_mode(owner_mode: BridgeOwnerMode) -> Arc<Self> {
        Self::new_with_owner_mode_and_limits(
            owner_mode,
            QueueLimits::default(),
            default_recovery_path(),
        )
    }

    #[cfg(test)]
    pub(in crate::browser::control_plane) fn new_with_limits(limits: QueueLimits) -> Arc<Self> {
        Self::new_with_owner_mode_and_limits(BridgeOwnerMode::Hybrid, limits, None)
    }

    #[cfg(test)]
    pub(in crate::browser::control_plane) fn new_with_recovery_path(path: PathBuf) -> Arc<Self> {
        Self::new_with_owner_mode_and_limits(
            BridgeOwnerMode::Hybrid,
            QueueLimits::default(),
            Some(Ok(path)),
        )
    }

    fn new_with_owner_mode_and_limits(
        owner_mode: BridgeOwnerMode,
        limits: QueueLimits,
        recovery_path: Option<std::io::Result<PathBuf>>,
    ) -> Arc<Self> {
        let generation = fixed_width_daemon_generation();
        let shared = Arc::new(Shared::default());
        let executor = Arc::new(IntegrationExecutor {
            shared: Arc::clone(&shared),
        });
        let (control, path_failure) = match recovery_path {
            Some(Ok(path)) => (
                ControlPlane::recover_persistent(generation.clone(), executor, limits, path),
                None,
            ),
            Some(Err(error)) => (
                ControlPlane::start(generation.clone(), executor, limits),
                Some(error),
            ),
            None => (
                ControlPlane::start(generation.clone(), executor, limits),
                None,
            ),
        };
        if let Some(error) = path_failure {
            control.events.record(
                BrowserControlEventKind::Recovery {
                    state: "recovery_journal_path_unavailable".to_owned(),
                },
                super::introspection::EventContext::default(),
            );
            tracing::warn!(detail = %error, "browser recovery journal path unavailable");
        }
        control.events.record(
            BrowserControlEventKind::MigrationDiagnostic {
                code: format!("{}_mode_active", owner_mode.as_str()),
            },
            super::introspection::EventContext::default(),
        );
        let _ = shared.control.set(control.clone());
        let runtime = Arc::new(Self {
            generation,
            owner_mode,
            control,
            shared,
        });
        Self::spawn_lease_tick(&runtime);
        runtime
    }

    fn spawn_lease_tick(runtime: &Arc<Self>) {
        let runtime = Arc::downgrade(runtime);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(LEASE_TICK_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(runtime) = runtime.upgrade() else {
                    break;
                };
                runtime.control.tick(now_ms()).await;
                runtime.prune_released_groups().await;
            }
        });
    }

    pub(crate) async fn high_level(
        &self,
        request: BrowserRequest,
        context: BrowserRequestContext,
    ) -> Result<BrowserResponse, DiagnosticEntry> {
        if matches!(request, BrowserRequest::Status) {
            return Err(runtime_diagnostic(
                "BrowserControlStatusLocal",
                "browser status is handled by the daemon without bridge dispatch",
            ));
        }
        let principal = principal_from_mcp(&context);
        let connection_id = context.provenance.connection_id.clone();
        if !self
            .begin_mcp_request(&connection_id, principal.clone())
            .await
        {
            return Err(runtime_diagnostic(
                "BrowserClientDisconnected",
                "browser request arrived after its MCP connection closed",
            ));
        }
        let result = async {
        self.record_mcp_client(&context.provenance);
        self.initialize_ownership_indexes().await;
        let logical_group = logical_group_key(
            &context.logical_identity.session_id,
            context.logical_identity.thread_id.as_deref(),
        );
        let client_id = ClientId(context.provenance.connection_id.clone());
        let actors = self.ready_actors().await?;
        let (scope, browser, tab) = self
            .resolve_high_level_scope(&request, &principal, &actors)
            .await?;
        let (group_id, lease) = if let Some(browser) = &browser {
            if let Some(tab) = &tab
                && !matches!(request, BrowserRequest::ClaimTab { .. })
            {
                let expected = self
                    .default_group(&principal, &logical_group, browser)
                    .await;
                let group = self
                    .group_for_tab(&principal, tab, Some(&expected.group_id))
                    .await?;
                (Some(group.group_id.clone()), Some(group.lease.proof()))
            } else {
                let group = self
                    .default_group(&principal, &logical_group, browser)
                    .await;
                (Some(group.group_id.clone()), None)
            }
        } else {
            (None, None)
        };
        let reserved_tab = if matches!(request, BrowserRequest::ClaimTab { .. }) {
            if let (Some(tab), Some(group_id)) = (&tab, &group_id) {
                let mut owners = self.shared.tab_owners.lock().await;
                if let Some(owner) = owners.get(tab)
                    && owner != group_id
                {
                    return Err(runtime_diagnostic(
                        "BrowserOwnershipRejected",
                        "tab belongs to another logical browser group; explicit handoff is required",
                    ));
                }
                if owners.contains_key(tab) {
                    None
                } else {
                    owners.insert(tab.clone(), group_id.clone());
                    Some(tab.clone())
                }
            } else {
                None
            }
        } else {
            None
        };
        let identity = BrowserSessionIdentity {
            session_id: context.logical_identity.session_id.clone(),
            thread_id: context.logical_identity.thread_id.clone(),
            turn_id: context
                .logical_identity
                .turn_id
                .clone()
                .unwrap_or_else(|| "browser-control".to_owned()),
        };
        let class = high_level_class(&request);
        let operation_id = OperationId(context.operation_identity.operation_id.clone());
        if let (Some(tab), Some(group_id)) = (&reserved_tab, &group_id) {
            remember_operation_reservation(
                &self.shared,
                operation_id.clone(),
                tab.clone(),
                group_id.clone(),
                principal.clone(),
            )
            .await;
        }
        self.track_operation(
            operation_id.clone(),
            context.provenance.connection_id.clone(),
            browser.as_ref(),
            &scope,
            class,
            &actors,
        )
        .await;
        let payload = serde_json::to_string(&IntegrationPayload::HighLevel {
            request: request.clone(),
            identity,
        })
        .expect("browser request serializes");
        let completion_result = self
            .control
            .submit(SubmitOperation {
                operation_id: Some(operation_id.clone()),
                canonical_fingerprint: context.operation_identity.request_id_fingerprint.clone(),
                upstream: UpstreamCorrelation {
                    ingress: "mcp".to_owned(),
                    request_id: Some(context.operation_identity.operation_id.clone()),
                },
                client_id,
                principal: principal.clone(),
                group_id: group_id.clone(),
                lease,
                scope,
                class,
                payload,
                now_ms: now_ms(),
            })
            .await;
        let completion = match completion_result {
            Ok(completion) => completion,
            Err(error) => {
                self.clear_operation_correlations(&operation_id).await;
                release_operation_reservation(&self.shared, &operation_id).await;
                return Err(admission_diagnostic(error));
            }
        };
        let certainty = completion.certainty.clone();
        let ambiguous = certainty == CompletionCertainty::Ambiguous;
        if !ambiguous {
            if matches!(class, OperationClass::Mutation | OperationClass::BrowserGlobal) {
                remember_terminal_settlement_operation(&self.shared, &operation_id).await;
            }
            self.clear_operation_correlations(&operation_id).await;
        }
        let mut response = match completion_response::<BrowserResponse>(completion) {
            Ok(response) => response,
            Err(error) => {
                release_operation_reservation_if_definitive(
                    &self.shared,
                    &operation_id,
                    &certainty,
                )
                .await;
                return Err(error);
            }
        };
        if context.provenance.caller == BrowserCallerKind::LegacyUnknown
            && let Some(declared) = &context.provenance.declared_caller
        {
            append_response_diagnostic(
                &mut response,
                runtime_diagnostic(
                    "BrowserProvenanceNormalized",
                    &format!(
                        "Unrecognized declared browser caller {declared:?} was normalized to legacy_unknown."
                    ),
                ),
            );
        }
        if matches!(
            request,
            BrowserRequest::Open { .. } | BrowserRequest::ClaimTab { .. }
        ) && let (Some(browser), Some(group_id), Some(tab_id)) =
            (browser, group_id, returned_tab_id(&response))
        {
            let tab = TabKey::new(browser, tab_id);
            self.control
                .add_member(group_id.clone(), principal, tab.clone())
                .await
                .map_err(group_diagnostic)?;
            self.shared.tab_owners.lock().await.insert(tab, group_id);
        }
        commit_operation_reservation(&self.shared, &operation_id).await;
        Ok(response)
        }
        .await;
        self.end_mcp_request(&connection_id).await;
        result
    }

    pub(crate) async fn status_report(
        &self,
        integration: Option<BrowserIntegrationReport>,
        deferred_reason: Option<&str>,
    ) -> BrowserStatusReport {
        let mut diagnostics = Vec::new();
        let bridge_ready = match self.ready_actors().await {
            Ok(_) => true,
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                false
            }
        };
        if let Some(reason) = deferred_reason {
            diagnostics.push(runtime_diagnostic("BrowserIntegrationDeferred", reason));
        } else if integration.is_none() {
            diagnostics.push(runtime_diagnostic(
                "BrowserIntegrationUnavailable",
                "The active backend did not report Chrome-family browser integration checks.",
            ));
        }
        BrowserStatusReport {
            enabled: true,
            available_targets: vec![persistent_target_availability(
                bridge_ready,
                integration.as_ref(),
            )],
            tabs_known: None,
            browser_integration: integration,
            control_plane: Some(Box::new(self.control_plane_snapshot().await)),
            diagnostics,
        }
    }

    pub(crate) async fn cancel_mcp_operation(
        &self,
        connection_id: &str,
        operation_id: &str,
    ) -> Result<CancelResult, DiagnosticEntry> {
        let operation = OperationId(operation_id.to_owned());
        let owner = self
            .shared
            .operation_clients
            .lock()
            .await
            .get(&operation)
            .cloned();
        if owner.as_deref().is_some_and(|owner| owner != connection_id) {
            return Err(runtime_diagnostic(
                "BrowserCancellationRejected",
                "browser operation does not belong to this MCP connection",
            ));
        }
        Ok(self
            .control
            .cancel_for_client(operation, ClientId(connection_id.to_owned()))
            .await)
    }

    pub(crate) async fn mcp_client_disconnected(&self, connection_id: &str) {
        self.record_client_closed(connection_id);
        let release_now = {
            let mut lifecycle = self.shared.mcp_connections.lock().await;
            lifecycle.closed.insert(connection_id.to_owned());
            lifecycle
                .active_requests
                .get(connection_id)
                .copied()
                .unwrap_or(0)
                == 0
        };
        if release_now {
            self.release_connection_principals(connection_id).await;
        }
    }

    pub(in crate::browser::control_plane) async fn begin_mcp_request(
        &self,
        connection_id: &str,
        principal: Principal,
    ) -> bool {
        let mut lifecycle = self.shared.mcp_connections.lock().await;
        if lifecycle.closed.contains(connection_id) {
            return false;
        }
        *lifecycle
            .active_requests
            .entry(connection_id.to_owned())
            .or_default() += 1;
        self.register_principal_connection(connection_id, principal)
            .await;
        true
    }

    pub(in crate::browser::control_plane) async fn end_mcp_request(&self, connection_id: &str) {
        let release_now = {
            let mut lifecycle = self.shared.mcp_connections.lock().await;
            let remaining = lifecycle
                .active_requests
                .get(connection_id)
                .copied()
                .unwrap_or(0)
                .saturating_sub(1);
            if remaining == 0 {
                lifecycle.active_requests.remove(connection_id);
            } else {
                lifecycle
                    .active_requests
                    .insert(connection_id.to_owned(), remaining);
            }
            remaining == 0 && lifecycle.closed.contains(connection_id)
        };
        if release_now {
            self.release_connection_principals(connection_id).await;
        }
    }

    pub(in crate::browser::control_plane) async fn track_operation(
        &self,
        operation_id: OperationId,
        connection_id: String,
        browser: Option<&BrowserInstanceId>,
        scope: &OperationScope,
        class: OperationClass,
        actors: &[ActorEntry],
    ) {
        self.shared
            .operation_clients
            .lock()
            .await
            .insert(operation_id.clone(), connection_id);
        let Some(browser) = browser else {
            return;
        };
        self.shared
            .operation_browsers
            .lock()
            .await
            .insert(operation_id.clone(), browser.clone());
        let actor_generation = actors
            .iter()
            .find(|entry| entry.browser_id == browser.0)
            .map(|entry| Value::from(entry.actor.health().actor_generation));
        if let Some(actor_generation) = actor_generation {
            self.shared.settlement_fences.lock().await.insert(
                operation_id,
                SettlementFence {
                    daemon_generation: self.generation.clone(),
                    actor_generation,
                    browser_instance_id: browser.clone(),
                    target_lifetime_key: operation_target(scope).unwrap_or(Value::Null),
                    operation_class: operation_class_name(class),
                },
            );
        }
    }

    pub(in crate::browser::control_plane) async fn clear_operation_correlations(
        &self,
        operation_id: &OperationId,
    ) {
        clear_operation_correlations(&self.shared, operation_id).await;
    }

    pub(in crate::browser::control_plane) async fn control_plane_snapshot(
        &self,
    ) -> BrowserControlPlaneSnapshot {
        let scheduler = self.control.snapshot().await;
        let entries = self
            .shared
            .actors
            .read()
            .expect("actor registry poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let canonical_sockets = canonical_ready_actors(entries.clone())
            .into_iter()
            .map(|entry| entry.socket)
            .collect::<HashSet<_>>();
        let mut actors = entries
            .into_iter()
            .map(|entry| {
                let health = entry.actor.health();
                let protocol_capable = health.host_instance_id.is_some();
                let canonical = canonical_sockets.contains(&entry.socket);
                BrowserControlActorSnapshot {
                    state: bridge_state(health.state),
                    transport: BrowserBridgeTransport::ExtensionNativeHost,
                    socket_path: entry.socket.to_string_lossy().into_owned(),
                    bridge_connection_id: health.bridge_connection_id,
                    browser_instance_id: health.browser_instance_id,
                    browser_instance_stability: health.browser_instance_stability,
                    host_instance_id: health.host_instance_id,
                    peer_pid: health.peer_pid,
                    peer_start_ticks: health.peer_start_ticks,
                    actor_generation: health.actor_generation,
                    protocol_capable,
                    selected: health.state == BridgeActorState::Ready,
                    canonical,
                    last_heartbeat_rtt_ms: health.last_heartbeat_rtt_ms,
                    reconnect_count: health.reconnect_count,
                    quarantine_reason: health.quarantine_reason,
                }
            })
            .collect::<Vec<_>>();
        actors.sort_by(|left, right| left.socket_path.cmp(&right.socket_path));
        let ready = actors.iter().any(|actor| actor.canonical);
        let actors_omitted =
            u32::try_from(actors.len().saturating_sub(ACTOR_RESULT_LIMIT)).unwrap_or(u32::MAX);
        actors.truncate(ACTOR_RESULT_LIMIT);
        let mut clients = self
            .shared
            .clients
            .read()
            .expect("client registry poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        clients.sort_by(|left, right| left.connection_id.cmp(&right.connection_id));
        let client_count = u32::try_from(clients.len()).unwrap_or(u32::MAX);
        let clients_omitted =
            u32::try_from(clients.len().saturating_sub(CLIENT_RESULT_LIMIT)).unwrap_or(u32::MAX);
        clients.truncate(CLIENT_RESULT_LIMIT);
        BrowserControlPlaneSnapshot {
            protocol_version: BROWSER_CONTROL_PROTOCOL_VERSION,
            daemon_generation: self.generation.clone(),
            migration_mode: match self.owner_mode {
                BridgeOwnerMode::Hybrid => BrowserMigrationMode::Hybrid,
                BridgeOwnerMode::Strict => BrowserMigrationMode::Strict,
            },
            ready,
            client_count,
            clients,
            clients_omitted,
            actors,
            actors_omitted,
            scheduler,
            events: self.control.events.snapshot(),
        }
    }

    pub(in crate::browser::control_plane) fn record_mcp_client(
        &self,
        provenance: &BrowserCallerProvenance,
    ) {
        let client_info_label = provenance
            .client_info
            .as_ref()
            .map(|info| bounded_label(&format!("{}/{}", info.name, info.version)));
        let client_info = provenance
            .client_info
            .as_ref()
            .map(|info| BrowserMcpClientInfo {
                name: bounded_label(&info.name),
                version: bounded_label(&info.version),
                title: info.title.as_deref().map(bounded_label),
            });
        let result = self.record_client_open(BrowserControlClientSummary {
            connection_id: provenance.connection_id.clone(),
            ingress: "mcp".to_owned(),
            surface: BrowserClientSurface::McpTools,
            caller: provenance.caller,
            provenance_source: provenance.source,
            declared_label: provenance.declared_caller.as_deref().map(bounded_label),
            client_info_label,
            client_info,
        });
        if result.is_err() {
            tracing::warn!(
                connection_id = %provenance.connection_id,
                "MCP browser connection attempted to change caller provenance"
            );
        }
    }

    pub(crate) fn observe_mcp_client(&self, provenance: &BrowserCallerProvenance) {
        self.record_mcp_client(provenance);
    }

    pub(in crate::browser::control_plane) fn record_raw_client_open(
        &self,
        provenance: &BrowserCallerProvenance,
    ) -> Result<(), String> {
        let client_info_label = provenance
            .client_info
            .as_ref()
            .map(|info| bounded_label(&format!("{}/{}", info.name, info.version)));
        let client = BrowserControlClientSummary {
            connection_id: provenance.connection_id.clone(),
            ingress: "raw_native_pipe".to_owned(),
            surface: BrowserClientSurface::NodeReplBrowserApi,
            caller: provenance.caller,
            provenance_source: provenance.source,
            declared_label: provenance.declared_caller.as_deref().map(bounded_label),
            client_info_label,
            client_info: provenance
                .client_info
                .as_ref()
                .map(|info| BrowserMcpClientInfo {
                    name: bounded_label(&info.name),
                    version: bounded_label(&info.version),
                    title: info.title.as_deref().map(bounded_label),
                }),
        };
        self.record_client_open(client)
            .map(|_| ())
            .map_err(|()| "raw browser connection attempted to change caller provenance".to_owned())
    }

    fn record_client_open(&self, client: BrowserControlClientSummary) -> Result<bool, ()> {
        let ingress = client.ingress.clone();
        let inserted = {
            let mut clients = self
                .shared
                .clients
                .write()
                .expect("client registry poisoned");
            match clients.get(&client.connection_id) {
                Some(existing) if existing == &client => false,
                Some(_) => return Err(()),
                None => {
                    clients.insert(client.connection_id.clone(), client.clone());
                    true
                }
            }
        };
        if inserted {
            self.control.events.record(
                BrowserControlEventKind::ClientState {
                    state: format!("{ingress}_connected"),
                    client,
                },
                super::introspection::EventContext::default(),
            );
        }
        Ok(inserted)
    }

    pub(in crate::browser::control_plane) fn record_client_closed(&self, connection_id: &str) {
        if let Some(client) = self
            .shared
            .clients
            .write()
            .expect("client registry poisoned")
            .remove(connection_id)
        {
            let ingress = client.ingress.clone();
            self.control.events.record(
                BrowserControlEventKind::ClientState {
                    state: format!("{ingress}_disconnected"),
                    client,
                },
                super::introspection::EventContext::default(),
            );
        }
    }

    pub(in crate::browser::control_plane) async fn ready_actors(
        &self,
    ) -> Result<Vec<ActorEntry>, DiagnosticEntry> {
        let selection = browser_socket_selection_from_env()?;
        let sockets = find_bridge_sockets(selection);
        let retired = {
            let selected = sockets.iter().cloned().collect::<HashSet<_>>();
            let mut actors = self.shared.actors.write().expect("actor registry poisoned");
            let retired = actors
                .iter()
                .filter(|(socket, _)| !selected.contains(*socket) || !socket.exists())
                .map(|(_, entry)| entry.actor.clone())
                .collect::<Vec<_>>();
            actors.retain(|socket, _| selected.contains(socket) && socket.exists());
            retired
        };
        for actor in retired {
            actor.shutdown().await;
        }
        if sockets.is_empty() {
            return Err(runtime_diagnostic(
                "BrowserControlUnavailable",
                "no native-host browser sockets are available",
            ));
        }
        let mut spawned = Vec::new();
        let mut new_actors = Vec::new();
        {
            let mut entries = self.shared.actors.write().expect("actor registry poisoned");
            for socket in sockets {
                let entry = entries.entry(socket.clone()).or_insert_with(|| {
                    let mut config = BridgeActorConfig::new(socket.clone(), 1);
                    config.daemon_generation = self.generation.clone();
                    config.owner_mode = self.owner_mode;
                    let actor = BridgeActor::spawn(config);
                    new_actors.push(actor.clone());
                    ActorEntry {
                        actor,
                        socket,
                        browser_id: String::new(),
                    }
                });
                spawned.push(entry.clone());
            }
        }
        for actor in new_actors {
            spawn_actor_events(actor, Arc::clone(&self.shared), self.control.clone());
        }
        let mut ready = Vec::new();
        for mut entry in spawned {
            let mut actor = entry.actor.clone();
            if tokio::time::timeout(Duration::from_secs(4), actor.wait_until_ready())
                .await
                .is_ok_and(|result| result.is_ok())
            {
                let health = actor.health();
                if health.state == BridgeActorState::Ready
                    && let Some(browser_id) = health.browser_instance_id
                {
                    entry.browser_id = browser_id;
                    ready.push(entry);
                }
            }
        }
        if ready.is_empty() {
            return Err(runtime_diagnostic(
                "BrowserControlUnavailable",
                "persistent native-host actors are not ready",
            ));
        }
        {
            let mut entries = self.shared.actors.write().expect("actor registry poisoned");
            for entry in &ready {
                entries.insert(entry.socket.clone(), entry.clone());
            }
        }
        Ok(canonical_ready_actors(ready))
    }

    async fn resolve_high_level_scope(
        &self,
        request: &BrowserRequest,
        principal: &Principal,
        actors: &[ActorEntry],
    ) -> Result<(OperationScope, Option<BrowserInstanceId>, Option<TabKey>), DiagnosticEntry> {
        if matches!(
            request,
            BrowserRequest::Status | BrowserRequest::ListTabs { .. }
        ) {
            return Ok((OperationScope::DaemonGlobal, None, None));
        }
        if matches!(request, BrowserRequest::Open { .. }) {
            let actor = one_actor(actors)?;
            let browser = BrowserInstanceId(actor.browser_id.clone());
            return Ok((
                OperationScope::BridgeGlobal(browser.clone()),
                Some(browser),
                None,
            ));
        }
        let tab_id = high_level_tab_id(request).ok_or_else(|| {
            runtime_diagnostic(
                "BrowserControlInvalidScope",
                "browser request has no tab target",
            )
        })?;
        let tab = self.resolve_tab(tab_id, principal, actors).await?;
        let browser = tab.browser_instance_id.clone();
        let scope = if matches!(request, BrowserRequest::ClaimTab { .. }) {
            OperationScope::BridgeGlobal(browser.clone())
        } else {
            OperationScope::Tab(tab.clone())
        };
        Ok((scope, Some(browser), Some(tab)))
    }

    pub(in crate::browser::control_plane) async fn initialize_ownership_indexes(&self) {
        let mut initialized = self.shared.ownership_indexes_initialized.lock().await;
        if *initialized {
            return;
        }
        self.reconcile_tab_owners().await;
        *initialized = true;
    }

    pub(in crate::browser::control_plane) async fn reconcile_tab_owners(&self) {
        let groups = self.control.groups().await;
        let active_groups = groups
            .iter()
            .filter(|group| !matches!(group.admission, super::GroupAdmission::Released))
            .map(|group| group.group_id.clone())
            .collect::<HashSet<_>>();
        let authoritative = authoritative_tab_owners(groups);
        let mut owners = self.shared.tab_owners.lock().await;
        // Keep in-flight claim/open reservations for active groups even before
        // the returned tab has been committed to group membership. Admission
        // and completion paths remove failed reservations explicitly.
        owners.retain(|_, group_id| active_groups.contains(group_id));
        owners.extend(authoritative);
    }

    pub(in crate::browser::control_plane) async fn prune_released_groups(&self) {
        let pruned = self.control.prune_released().await;
        if !pruned.is_empty() {
            let pruned = pruned.into_iter().collect::<HashSet<_>>();
            self.shared
                .groups
                .lock()
                .await
                .retain(|_, group_id| !pruned.contains(group_id));
        }
        self.reconcile_tab_owners().await;
    }

    #[cfg(test)]
    pub(in crate::browser::control_plane) async fn has_logical_group_index(
        &self,
        group_id: &GroupId,
    ) -> bool {
        self.shared
            .groups
            .lock()
            .await
            .values()
            .any(|indexed| indexed == group_id)
    }

    pub(in crate::browser::control_plane) async fn register_principal_connection(
        &self,
        connection_id: &str,
        principal: Principal,
    ) {
        self.shared
            .connection_principals
            .lock()
            .await
            .entry(connection_id.to_owned())
            .or_default()
            .insert(principal.id.clone(), principal.clone());
        self.shared
            .principal_connections
            .lock()
            .await
            .entry(principal.id.clone())
            .or_default()
            .insert(connection_id.to_owned());
    }

    pub(in crate::browser::control_plane) async fn release_connection_principals(
        &self,
        connection_id: &str,
    ) {
        let principals = self
            .shared
            .connection_principals
            .lock()
            .await
            .remove(connection_id)
            .unwrap_or_default();
        for (principal_id, principal) in principals {
            let should_disconnect = {
                let mut references = self.shared.principal_connections.lock().await;
                let Some(connections) = references.get_mut(&principal_id) else {
                    continue;
                };
                connections.remove(connection_id);
                if connections.is_empty() {
                    references.remove(&principal_id);
                    true
                } else {
                    false
                }
            };
            if should_disconnect {
                self.control.disconnect(principal, now_ms()).await;
            }
        }
    }

    async fn resolve_tab(
        &self,
        tab_id: &str,
        _principal: &Principal,
        actors: &[ActorEntry],
    ) -> Result<TabKey, DiagnosticEntry> {
        let owners = self.shared.tab_owners.lock().await;
        let eligible_browsers = actors
            .iter()
            .map(|actor| actor.browser_id.as_str())
            .collect::<HashSet<_>>();
        let known = owners
            .keys()
            .filter(|tab| {
                tab.tab_id == tab_id
                    && eligible_browsers.contains(tab.browser_instance_id.0.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        drop(owners);
        if known.len() > 1 {
            return Err(runtime_diagnostic(
                "BrowserInstanceAmbiguous",
                "bare tab id exists in multiple browser instances",
            ));
        }
        if let Some(tab) = known.first() {
            return Ok(tab.clone());
        }
        let mut matches = Vec::new();
        for actor in actors {
            let response = actor
                .actor
                .request(BridgeActorRequest::new(
                    "getUserTabs",
                    json!({}),
                    format!("discover-{tab_id}"),
                    OperationClass::ReadOnly,
                ))
                .await;
            if response
                .ok()
                .and_then(|value| value.get("result").cloned())
                .is_some_and(|result| contains_tab(&result, tab_id))
            {
                matches.push(TabKey::new(actor.browser_id.as_str(), tab_id));
            }
        }
        match matches.as_slice() {
            [tab] => Ok(tab.clone()),
            [] => Err(runtime_diagnostic(
                "BrowserTabNotFound",
                "tab is not present in any eligible browser instance",
            )),
            _ => Err(runtime_diagnostic(
                "BrowserInstanceAmbiguous",
                "bare tab id exists in multiple browser instances",
            )),
        }
    }

    pub(in crate::browser::control_plane) async fn default_group(
        &self,
        principal: &Principal,
        logical_group: &str,
        browser: &BrowserInstanceId,
    ) -> super::GroupSnapshot {
        let key = (
            principal.id.clone(),
            logical_group.to_owned(),
            browser.0.clone(),
        );
        let existing = self.shared.groups.lock().await.get(&key).cloned();
        if let Some(group_id) = existing
            && let Ok(group) = self.control.group(group_id).await
        {
            if matches!(group.admission, super::GroupAdmission::Released) {
                return self
                    .control
                    .create_group(group.group_id, browser.clone(), principal.clone(), now_ms())
                    .await;
            }
            if let Ok(lease) = self
                .control
                .renew(group.lease.proof(), principal.clone(), now_ms())
                .await
            {
                return super::GroupSnapshot { lease, ..group };
            }
            return group;
        }
        let group_id = GroupId(format!(
            "default:{}:{}:{}",
            principal.id, logical_group, browser.0
        ));
        if let Ok(group) = self.control.group(group_id.clone()).await {
            self.shared
                .groups
                .lock()
                .await
                .insert(key, group_id.clone());
            if matches!(group.admission, super::GroupAdmission::Released) {
                return self
                    .control
                    .create_group(group_id, browser.clone(), principal.clone(), now_ms())
                    .await;
            }
            if let Ok(lease) = self
                .control
                .renew(group.lease.proof(), principal.clone(), now_ms())
                .await
            {
                return super::GroupSnapshot { lease, ..group };
            }
            return group;
        }
        let group = self
            .control
            .create_group(
                group_id.clone(),
                browser.clone(),
                principal.clone(),
                now_ms(),
            )
            .await;
        self.shared.groups.lock().await.insert(key, group_id);
        group
    }

    pub(in crate::browser::control_plane) async fn group_for_tab(
        &self,
        principal: &Principal,
        tab: &TabKey,
        expected_group: Option<&GroupId>,
    ) -> Result<super::GroupSnapshot, DiagnosticEntry> {
        let group_id = self
            .shared
            .tab_owners
            .lock()
            .await
            .get(tab)
            .cloned()
            .ok_or_else(|| {
                runtime_diagnostic(
                    "BrowserOwnershipRequired",
                    "tab must be opened or claimed by this caller before use",
                )
            })?;
        let group = self
            .control
            .group(group_id)
            .await
            .map_err(group_diagnostic)?;
        if expected_group.is_some_and(|expected| expected != &group.group_id) {
            return Err(runtime_diagnostic(
                "BrowserOwnershipRejected",
                "tab belongs to another logical browser group; explicit handoff is required",
            ));
        }
        if group.lease.principal != *principal {
            return Err(runtime_diagnostic(
                "BrowserOwnershipRejected",
                "tab belongs to another browser principal",
            ));
        }
        Ok(group)
    }
}
