use super::*;

const TERMINAL_SETTLEMENT_OPERATION_LIMIT: usize = 2_048;

pub(in crate::browser::control_plane) fn authoritative_tab_owners(
    groups: Vec<super::GroupSnapshot>,
) -> HashMap<TabKey, GroupId> {
    let mut authoritative = HashMap::new();
    for group in groups {
        if matches!(group.admission, super::GroupAdmission::Released) {
            continue;
        }
        for tab in group.members {
            authoritative.insert(tab, group.group_id.clone());
        }
    }
    authoritative
}

#[cfg(not(test))]
pub(in crate::browser::control_plane) fn default_recovery_path() -> Option<std::io::Result<PathBuf>>
{
    Some(
        sky_cua_platform::paths::sky_cua_state_dir()
            .map(|path| path.join(super::persistence::RECOVERY_JOURNAL_FILE)),
    )
}

#[cfg(test)]
pub(in crate::browser::control_plane) fn default_recovery_path() -> Option<std::io::Result<PathBuf>>
{
    None
}

pub(in crate::browser::control_plane) async fn clear_operation_correlations(
    shared: &Shared,
    operation_id: &OperationId,
) {
    shared.operation_browsers.lock().await.remove(operation_id);
    shared.operation_clients.lock().await.remove(operation_id);
    shared.settlement_fences.lock().await.remove(operation_id);
    shared
        .settlement_parents
        .lock()
        .await
        .retain(|child, parent| child != operation_id && parent != operation_id);
}

pub(in crate::browser::control_plane) async fn remember_terminal_settlement_operation(
    shared: &Shared,
    operation_id: &OperationId,
) {
    let daemon_generation = shared
        .settlement_fences
        .lock()
        .await
        .get(operation_id)
        .map(|fence| fence.daemon_generation.clone());
    let Some(daemon_generation) = daemon_generation else {
        return;
    };
    let key = TerminalSettlementOperation {
        operation_id: operation_id.clone(),
        daemon_generation,
    };
    let mut terminal = shared.terminal_settlement_operations.lock().await;
    if terminal.contains(&key) {
        return;
    }
    terminal.push_back(key);
    while terminal.len() > TERMINAL_SETTLEMENT_OPERATION_LIMIT {
        terminal.pop_front();
    }
}

pub(in crate::browser::control_plane) async fn remember_operation_reservation(
    shared: &Shared,
    operation_id: OperationId,
    tab: TabKey,
    group_id: GroupId,
    principal: Principal,
) {
    shared.operation_reservations.lock().await.insert(
        operation_id,
        OperationReservation {
            tab,
            group_id,
            principal,
        },
    );
}

pub(in crate::browser::control_plane) async fn commit_operation_reservation(
    shared: &Shared,
    operation_id: &OperationId,
) {
    shared
        .operation_reservations
        .lock()
        .await
        .remove(operation_id);
}

pub(in crate::browser::control_plane) async fn release_operation_reservation(
    shared: &Shared,
    operation_id: &OperationId,
) {
    let reservation = shared
        .operation_reservations
        .lock()
        .await
        .remove(operation_id);
    if let Some(reservation) = reservation {
        shared
            .tab_owners
            .lock()
            .await
            .retain(|tab, owner| tab != &reservation.tab || owner != &reservation.group_id);
    }
}

pub(in crate::browser::control_plane) async fn release_operation_reservation_if_definitive(
    shared: &Shared,
    operation_id: &OperationId,
    certainty: &CompletionCertainty,
) {
    if certainty != &CompletionCertainty::Ambiguous {
        release_operation_reservation(shared, operation_id).await;
    }
}

pub(in crate::browser::control_plane) async fn settle_operation_reservation(
    shared: &Shared,
    control: &ControlPlane,
    completion: &Completion,
) {
    let reservation = shared
        .operation_reservations
        .lock()
        .await
        .remove(&completion.operation_id);
    let Some(reservation) = reservation else {
        return;
    };
    if completion.disposition == CompletionDisposition::Success {
        if control
            .add_member(
                reservation.group_id.clone(),
                reservation.principal,
                reservation.tab.clone(),
            )
            .await
            .is_ok()
        {
            shared
                .tab_owners
                .lock()
                .await
                .insert(reservation.tab, reservation.group_id);
        } else {
            control.events.record(
                BrowserControlEventKind::MigrationDiagnostic {
                    code: "claim_settlement_membership_commit_failed".to_owned(),
                },
                super::super::introspection::EventContext {
                    group_id: Some(reservation.group_id.0),
                    tab_key: Some(BrowserTabKey {
                        browser_instance_id: reservation.tab.browser_instance_id.0,
                        extension_tab_id: reservation.tab.tab_id,
                    }),
                    operation_id: Some(completion.operation_id.0.clone()),
                    ..Default::default()
                },
            );
        }
        return;
    }
    shared
        .tab_owners
        .lock()
        .await
        .retain(|tab, owner| tab != &reservation.tab || owner != &reservation.group_id);
}

#[cfg(test)]
pub(in crate::browser::control_plane) async fn install_test_operation_correlations(
    shared: &Shared,
    operation_id: OperationId,
    connection_id: &str,
    fence: SettlementFence,
) {
    shared
        .operation_clients
        .lock()
        .await
        .insert(operation_id.clone(), connection_id.to_owned());
    shared
        .operation_browsers
        .lock()
        .await
        .insert(operation_id.clone(), fence.browser_instance_id.clone());
    shared
        .settlement_fences
        .lock()
        .await
        .insert(operation_id, fence);
}

#[cfg(test)]
pub(in crate::browser::control_plane) async fn correlation_counts(
    shared: &Shared,
) -> (usize, usize, usize, usize, usize, usize) {
    (
        shared.operation_clients.lock().await.len(),
        shared.operation_browsers.lock().await.len(),
        shared.settlement_fences.lock().await.len(),
        shared.settlement_parents.lock().await.len(),
        shared
            .codex_by_browser
            .lock()
            .await
            .values()
            .map(HashSet::len)
            .sum(),
        shared.server_request_routes.lock().await.len(),
    )
}

pub(in crate::browser::control_plane) fn bridge_state(
    state: BridgeActorState,
) -> BrowserBridgeState {
    match state {
        BridgeActorState::Connecting => BrowserBridgeState::Connecting,
        BridgeActorState::HostHandshake => BrowserBridgeState::HostHandshake,
        BridgeActorState::Ready => BrowserBridgeState::Ready,
        BridgeActorState::Reconnecting => BrowserBridgeState::Reconnecting,
        BridgeActorState::Quarantined => BrowserBridgeState::Quarantined,
        BridgeActorState::Lost => BrowserBridgeState::Lost,
    }
}

pub(in crate::browser::control_plane) fn persistent_target_availability(
    bridge_ready: bool,
    integration: Option<&BrowserIntegrationReport>,
) -> BrowserTargetAvailability {
    BrowserTargetAvailability {
        target: BrowserTargetKind::UserChrome,
        available: bridge_ready,
        detail: if bridge_ready {
            "Persistent Chrome native-host browser actor is responsive.".to_owned()
        } else if let Some(integration) = integration {
            format!(
                "No canonical browser actor is ready. Native host installation: {}",
                integration.native_host_manifest.detail
            )
        } else {
            "No persistent Chrome native-host browser actor is ready.".to_owned()
        },
    }
}

pub(in crate::browser::control_plane) fn bounded_label(value: &str) -> String {
    value.chars().take(CLIENT_LABEL_LIMIT).collect()
}

pub(in crate::browser::control_plane) fn principal_from_mcp(
    context: &BrowserRequestContext,
) -> Principal {
    let session_id = if context.logical_identity.session_id.is_empty() {
        &context.provenance.connection_id
    } else {
        context.logical_identity.session_id.as_str()
    };
    Principal::new(
        format!(
            "mcp:{}:session:{}",
            caller_name(context.provenance.caller),
            session_id,
        ),
        unsafe { libc::geteuid() },
    )
}

pub(in crate::browser::control_plane) fn caller_name(caller: BrowserCallerKind) -> &'static str {
    match caller {
        BrowserCallerKind::CodexDesktop => "codex_desktop",
        BrowserCallerKind::CodexCli => "codex_cli",
        BrowserCallerKind::OpenClaw => "openclaw",
        BrowserCallerKind::OpenCode => "opencode",
        BrowserCallerKind::Pi => "pi",
        BrowserCallerKind::DirectMcp => "direct_mcp",
        BrowserCallerKind::LegacyUnknown => "legacy_unknown",
    }
}
pub(in crate::browser::control_plane) fn logical_group_key(
    session_id: &str,
    thread_id: Option<&str>,
) -> String {
    match thread_id.filter(|thread| !thread.is_empty()) {
        Some(thread) => format!("session:{session_id}:thread:{thread}"),
        None => format!("session:{session_id}"),
    }
}
pub(in crate::browser::control_plane) fn one_actor(
    actors: &[ActorEntry],
) -> Result<&ActorEntry, DiagnosticEntry> {
    match actors {
        [actor] => Ok(actor),
        [] => Err(runtime_diagnostic(
            "BrowserControlUnavailable",
            "no persistent browser instance is ready",
        )),
        _ => Err(runtime_diagnostic(
            "BrowserInstanceAmbiguous",
            "multiple eligible browser instances require an explicit instance-qualified tab",
        )),
    }
}

pub(in crate::browser::control_plane) fn canonical_ready_actors(
    actors: impl IntoIterator<Item = ActorEntry>,
) -> Vec<ActorEntry> {
    let mut actors = actors
        .into_iter()
        .filter(|entry| {
            let health = entry.actor.health();
            entry.socket.exists()
                && health.state == BridgeActorState::Ready
                && health.browser_instance_id.as_deref() == Some(entry.browser_id.as_str())
        })
        .collect::<Vec<_>>();
    actors.sort_by(|left, right| left.socket.cmp(&right.socket));

    let mut stable_browsers = HashSet::new();
    actors.retain(|entry| {
        let health = entry.actor.health();
        health.browser_instance_stability
            != sky_cua_platform::model::BrowserInstanceStability::Stable
            || stable_browsers.insert(entry.browser_id.clone())
    });
    actors
}

pub(in crate::browser::control_plane) fn is_upfront_unattached_upstream_error(
    error: &Value,
) -> bool {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string());
    crate::browser::session::is_upfront_unattached_diagnostic(&DiagnosticEntry {
        code: "BrowserBridgeRequestFailed".to_owned(),
        message,
        details: None,
    })
}
pub(in crate::browser::control_plane) fn high_level_tab_id(
    request: &BrowserRequest,
) -> Option<&str> {
    match request {
        BrowserRequest::ClaimTab { tab_id, .. }
        | BrowserRequest::MoveMouse { tab_id, .. }
        | BrowserRequest::Navigate { tab_id, .. }
        | BrowserRequest::ObserveAppShot { tab_id, .. }
        | BrowserRequest::Snapshot { tab_id, .. }
        | BrowserRequest::Screenshot { tab_id, .. }
        | BrowserRequest::Click { tab_id, .. }
        | BrowserRequest::ClickElement { tab_id, .. }
        | BrowserRequest::TypeText { tab_id, .. }
        | BrowserRequest::TypeTextElement { tab_id, .. }
        | BrowserRequest::PressKey { tab_id, .. }
        | BrowserRequest::Scroll { tab_id, .. }
        | BrowserRequest::Eval { tab_id, .. } => Some(tab_id),
        _ => None,
    }
}
pub(in crate::browser::control_plane) fn high_level_class(
    request: &BrowserRequest,
) -> OperationClass {
    match request {
        BrowserRequest::Status
        | BrowserRequest::ListTabs { .. }
        | BrowserRequest::ObserveAppShot { .. }
        | BrowserRequest::Snapshot { .. }
        | BrowserRequest::Screenshot { .. } => OperationClass::ReadOnly,
        BrowserRequest::MoveMouse { .. } => OperationClass::AbsoluteSet,
        BrowserRequest::Open { .. } | BrowserRequest::ClaimTab { .. } => {
            OperationClass::BrowserGlobal
        }
        _ => OperationClass::Mutation,
    }
}
pub(in crate::browser::control_plane) fn codex_class(class: CodexOperationClass) -> OperationClass {
    match class {
        CodexOperationClass::ReadOnly => OperationClass::ReadOnly,
        CodexOperationClass::AbsoluteSet => OperationClass::AbsoluteSet,
        CodexOperationClass::Mutation => OperationClass::Mutation,
    }
}

pub(in crate::browser::control_plane) fn operation_class_name(
    class: OperationClass,
) -> &'static str {
    match class {
        OperationClass::ReadOnly => "read_only",
        OperationClass::AbsoluteSet => "absolute_set",
        OperationClass::Mutation => "mutation",
        OperationClass::BrowserGlobal => "browser_global",
    }
}

pub(in crate::browser::control_plane) fn operation_target(scope: &OperationScope) -> Option<Value> {
    match scope {
        OperationScope::Tab(tab) => {
            Some(json!({"browser_instance_id":tab.browser_instance_id.0,"tab_id":tab.tab_id}))
        }
        OperationScope::BridgeGlobal(browser) => Some(json!({"browser_instance_id":browser.0})),
        OperationScope::DaemonGlobal => None,
    }
}
pub(in crate::browser::control_plane) fn returned_tab_id(
    response: &BrowserResponse,
) -> Option<String> {
    match response {
        BrowserResponse::Open { response } => response.tab.as_ref().map(|tab| tab.tab_id.clone()),
        BrowserResponse::ClaimTab { response } => {
            response.tab.as_ref().map(|tab| tab.tab_id.clone())
        }
        _ => None,
    }
}
pub(in crate::browser::control_plane) fn append_response_diagnostic(
    response: &mut BrowserResponse,
    diagnostic: DiagnosticEntry,
) {
    match response {
        BrowserResponse::ListTabs { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::Open { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::ClaimTab { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::MoveMouse { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::Navigate { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::Snapshot { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::Screenshot { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::Click { response }
        | BrowserResponse::TypeText { response }
        | BrowserResponse::PressKey { response }
        | BrowserResponse::Scroll { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::Eval { response } => response.diagnostics.push(diagnostic),
        BrowserResponse::AppShot { response } => response.appshot.diagnostics.push(diagnostic),
        BrowserResponse::AppShotRequired { .. } => {}
        BrowserResponse::Status { report } => report.diagnostics.push(diagnostic),
    }
}
pub(in crate::browser::control_plane) fn raw_returned_tab_id(value: &Value) -> Option<String> {
    value
        .pointer("/tab/id")
        .or_else(|| value.get("tabId"))
        .or_else(|| value.get("id"))
        .and_then(|id| {
            id.as_str()
                .map(str::to_owned)
                .or_else(|| id.as_u64().map(|id| id.to_string()))
        })
}
pub(in crate::browser::control_plane) fn contains_tab(value: &Value, id: &str) -> bool {
    value
        .as_array()
        .or_else(|| value.get("tabs").and_then(Value::as_array))
        .is_some_and(|tabs| {
            tabs.iter().any(|tab| {
                tab.get("id")
                    .or_else(|| tab.get("tabId"))
                    .is_some_and(|value| {
                        value.as_str() == Some(id)
                            || value
                                .as_u64()
                                .is_some_and(|number| number.to_string() == id)
                    })
            })
        })
}
pub(in crate::browser::control_plane) fn completion_response<T: serde::de::DeserializeOwned>(
    completion: Completion,
) -> Result<T, DiagnosticEntry> {
    if completion.disposition != CompletionDisposition::Success {
        return Err(runtime_diagnostic(
            "BrowserControlOperationFailed",
            &completion.detail,
        ));
    }
    serde_json::from_str(&completion.detail)
        .map_err(|error| runtime_diagnostic("BrowserControlMalformedResult", &error.to_string()))
}
pub(in crate::browser::control_plane) fn admission_diagnostic(
    error: AdmissionError,
) -> DiagnosticEntry {
    runtime_diagnostic("BrowserControlAdmissionRejected", &format!("{error:?}"))
}
pub(in crate::browser::control_plane) fn group_diagnostic(
    error: super::GroupError,
) -> DiagnosticEntry {
    runtime_diagnostic("BrowserOwnershipRejected", &format!("{error:?}"))
}
pub(in crate::browser::control_plane) fn runtime_diagnostic(
    code: &str,
    message: &str,
) -> DiagnosticEntry {
    DiagnosticEntry {
        code: code.to_owned(),
        message: message.to_owned(),
        details: None,
    }
}
pub(in crate::browser::control_plane) fn backend_error(message: String) -> CodexBackendReply {
    CodexBackendReply::Error(json!({"code":RUNTIME_ERROR_CODE,"message":message}))
}
pub(in crate::browser::control_plane) fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
