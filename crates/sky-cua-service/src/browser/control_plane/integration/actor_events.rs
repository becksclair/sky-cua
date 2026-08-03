use super::*;
use crate::browser::control_plane::SETTLEMENT_DEADLINE_MS;
use sky_cua_platform::model::BrowserControlEventKind;

const HANDLED_SETTLEMENT_LIMIT: usize = 2_048;

pub(in crate::browser::control_plane) fn spawn_actor_events(
    actor: BridgeActor,
    shared: Arc<Shared>,
    control: ControlPlane,
) {
    spawn_actor_event_receiver(actor.clone(), actor.subscribe(), shared, control);
}

fn spawn_actor_event_receiver(
    actor: BridgeActor,
    mut events: tokio::sync::broadcast::Receiver<BridgeActorEvent>,
    shared: Arc<Shared>,
    control: ControlPlane,
) {
    tokio::spawn(async move {
        let mut blocked_until_epoch: u64 = 0;
        loop {
            let event = match tokio::select! {
                biased;
                _ = actor.wait_closed() => break,
                event = events.recv() => event,
            } {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    let epoch = actor.health().connection_epoch;
                    control.events.record(
                        BrowserControlEventKind::MigrationDiagnostic {
                            code: format!("actor_event_lagged_force_reconnect:{skipped}"),
                        },
                        super::super::introspection::EventContext::default(),
                    );
                    if actor
                        .request_reconnect(epoch, format!("actor_event_lagged_force_reconnect"))
                        .await
                    {
                        blocked_until_epoch = epoch.wrapping_add(1);
                    }
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            record_actor_event(&control, &event);
            match event {
                BridgeActorEvent::Extension(message) => {
                    if let Some(browser_id) = actor.health().browser_instance_id {
                        route_extension_message(&actor, &shared, &browser_id, message).await;
                    }
                }
                BridgeActorEvent::Settlement(message) => {
                    if let Some(browser_id) = actor.health().browser_instance_id
                        && settle_actor_message(
                            &control,
                            &shared,
                            &browser_id,
                            message.clone(),
                            false,
                        )
                        .await
                    {
                        let epoch = actor.health().connection_epoch;
                        let barrier_cleared = epoch > blocked_until_epoch
                            && actor.health().state == BridgeActorState::Ready;
                        if barrier_cleared {
                            if let Err(error) = actor.acknowledge_settlement(&message).await {
                                control.events.record(
                                    BrowserControlEventKind::MigrationDiagnostic {
                                        code: format!("settlement_ack_failed:{epoch}:{:?}", error),
                                    },
                                    super::super::introspection::EventContext::default(),
                                );
                                if actor
                                    .request_reconnect(epoch, "settlement_ack_failed".to_owned())
                                    .await
                                {
                                    blocked_until_epoch = epoch.wrapping_add(1);
                                }
                                continue;
                            }
                        }
                    }
                }
                BridgeActorEvent::SettlementUnknown(message) => {
                    if let Some(browser_id) = actor.health().browser_instance_id
                        && settle_actor_message(
                            &control,
                            &shared,
                            &browser_id,
                            message.clone(),
                            true,
                        )
                        .await
                    {
                        let epoch = actor.health().connection_epoch;
                        let barrier_cleared = epoch > blocked_until_epoch
                            && actor.health().state == BridgeActorState::Ready;
                        if barrier_cleared {
                            if let Err(error) = actor.acknowledge_settlement(&message).await {
                                control.events.record(
                                    BrowserControlEventKind::MigrationDiagnostic {
                                        code: format!("settlement_ack_failed:{epoch}:{:?}", error),
                                    },
                                    super::super::introspection::EventContext::default(),
                                );
                                if actor
                                    .request_reconnect(epoch, "settlement_ack_failed".to_owned())
                                    .await
                                {
                                    blocked_until_epoch = epoch.wrapping_add(1);
                                }
                                continue;
                            }
                        }
                    }
                }
                BridgeActorEvent::BrowserLost {
                    browser_instance_id,
                    ..
                } => {
                    shared
                        .tab_owners
                        .lock()
                        .await
                        .retain(|tab, _| tab.browser_instance_id.0 != browser_instance_id);
                    let affected = shared
                        .operation_browsers
                        .lock()
                        .await
                        .iter()
                        .filter(|(_, browser)| browser.0 == browser_instance_id)
                        .map(|(operation, _)| operation.clone())
                        .collect::<Vec<_>>();
                    for operation in affected {
                        let result = control
                            .settle(
                                operation.clone(),
                                SettlementOutcome::BrowserLost(BrowserInstanceId(
                                    browser_instance_id.clone(),
                                )),
                            )
                            .await;
                        if let SettlementResult::Settled(completion) = result {
                            settle_operation_reservation(&shared, &control, &completion).await;
                            clear_operation_correlations(&shared, &operation).await;
                        }
                    }
                    control
                        .browser_lost(BrowserInstanceId(browser_instance_id))
                        .await;
                }
                BridgeActorEvent::LateResponse {
                    operation_id: Some(child_id),
                    response,
                    ..
                } => {
                    settle_late_response(&control, &shared, child_id, response).await;
                }
                BridgeActorEvent::State(_)
                | BridgeActorEvent::LateResponse { .. }
                | BridgeActorEvent::Failover { .. } => {}
            }
        }
    });
}

pub(in crate::browser::control_plane) async fn settle_late_response(
    control: &ControlPlane,
    shared: &Shared,
    child_id: String,
    response: Value,
) {
    let child = OperationId(child_id);
    let operation = shared
        .settlement_parents
        .lock()
        .await
        .get(&child)
        .cloned()
        .unwrap_or(child);
    let outcome = if let Some(error) = response.get("error") {
        SettlementOutcome::Error(error.to_string())
    } else {
        SettlementOutcome::DefinitiveSuccess(
            response.get("result").unwrap_or(&response).to_string(),
        )
    };
    if let SettlementResult::Settled(completion) = control.settle(operation.clone(), outcome).await
    {
        settle_operation_reservation(shared, control, &completion).await;
        remember_terminal_settlement_operation(shared, &operation).await;
        clear_operation_correlations(shared, &operation).await;
    }
}

#[cfg(test)]
pub(in crate::browser::control_plane) fn spawn_actor_event_receiver_for_test(
    actor: BridgeActor,
    events: tokio::sync::broadcast::Receiver<BridgeActorEvent>,
    shared: Arc<Shared>,
    control: ControlPlane,
) {
    spawn_actor_event_receiver(actor, events, shared, control);
}

async fn route_extension_message(
    actor: &BridgeActor,
    shared: &Shared,
    browser_id: &str,
    message: Value,
) {
    let associated = shared
        .codex_by_browser
        .lock()
        .await
        .get(browser_id)
        .cloned()
        .unwrap_or_default();
    let connections = shared.connections.lock().await;
    let mut live = associated
        .into_iter()
        .filter_map(|connection_id| {
            connections
                .get(&connection_id)
                .map(|entry| (connection_id, entry.1.clone()))
        })
        .collect::<Vec<_>>();
    drop(connections);
    if let Some(session_id) = server_request_session_id(&message) {
        let sessions = shared.codex_connection_sessions.lock().await;
        live.retain(|(connection_id, _)| {
            sessions
                .get(connection_id)
                .is_some_and(|known| known.contains(session_id))
        });
    }
    live.sort_by(|left, right| left.0.cmp(&right.0));

    if message.get("method").and_then(Value::as_str).is_some()
        && let Some(request_id) = message.get("id").cloned()
    {
        let Some(route_id) = ServerRequestId::from_value(&request_id) else {
            reject_server_request(
                actor,
                request_id,
                "server request id must be a string or number",
            )
            .await;
            return;
        };
        let [(connection_id, outbound)] = live.as_slice() else {
            reject_server_request(
                actor,
                request_id,
                "server request has no unambiguous live Codex connection",
            )
            .await;
            return;
        };
        let route_key = (connection_id.clone(), route_id);
        let mut routes = shared.server_request_routes.lock().await;
        if routes.contains_key(&route_key) {
            drop(routes);
            reject_server_request(
                actor,
                request_id,
                "server request id is already pending for this Codex connection",
            )
            .await;
            return;
        }
        routes.insert(route_key.clone(), actor.clone());
        drop(routes);
        if outbound.try_send(message).is_err() {
            shared.server_request_routes.lock().await.remove(&route_key);
            reject_server_request(actor, request_id, "server request Codex connection closed")
                .await;
        }
        return;
    }

    for (_, outbound) in live {
        let _ = outbound.try_send(message.clone());
    }
}

fn server_request_session_id(message: &Value) -> Option<&str> {
    [
        "/params/metadata/codexSessionId",
        "/params/codexSessionId",
        "/metadata/codexSessionId",
    ]
    .into_iter()
    .find_map(|pointer| message.pointer(pointer).and_then(Value::as_str))
    .filter(|session_id| !session_id.is_empty())
}

async fn reject_server_request(actor: &BridgeActor, id: Value, message: &str) {
    let _ = actor
        .send_server_message(json!({
            "jsonrpc":"2.0",
            "id":id,
            "error":{"code":RUNTIME_ERROR_CODE,"message":message},
        }))
        .await;
}

fn record_actor_event(control: &ControlPlane, event: &BridgeActorEvent) {
    let (kind, operation_id) = match event {
        BridgeActorEvent::State(health) => {
            control.events.record(
                BrowserControlEventKind::BridgeState {
                    state: super::bridge_state(health.state),
                },
                super::super::introspection::EventContext::default(),
            );
            let Some(rtt_ms) = health.last_heartbeat_rtt_ms else {
                return;
            };
            (BrowserControlEventKind::Heartbeat { rtt_ms }, None)
        }
        BridgeActorEvent::Settlement(message) => (
            BrowserControlEventKind::Settlement {
                state: "settlement_received".to_owned(),
            },
            message
                .pointer("/params/operation_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
        BridgeActorEvent::SettlementUnknown(message) => (
            BrowserControlEventKind::Settlement {
                state: "settlement_unknown_received".to_owned(),
            },
            message
                .pointer("/params/operation_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
        BridgeActorEvent::LateResponse { operation_id, .. } => (
            BrowserControlEventKind::OperationState {
                state: "late_response".to_owned(),
            },
            operation_id.clone(),
        ),
        BridgeActorEvent::BrowserLost {
            stable_recovery, ..
        } => (
            BrowserControlEventKind::Failover {
                state: if *stable_recovery {
                    "browser_lost_stable_identity"
                } else {
                    "browser_lost_connection_identity"
                }
                .to_owned(),
            },
            None,
        ),
        BridgeActorEvent::Failover { state, .. } => (
            BrowserControlEventKind::Failover {
                state: state.clone(),
            },
            None,
        ),
        BridgeActorEvent::Extension(_) => return,
    };
    control.events.record(
        kind,
        super::super::introspection::EventContext {
            operation_id,
            ..Default::default()
        },
    );
}

pub(in crate::browser::control_plane) async fn settle_actor_message(
    control: &ControlPlane,
    shared: &Shared,
    browser_id: &str,
    message: Value,
    unknown: bool,
) -> bool {
    let Some(identity) = settlement_ack_identity(&message) else {
        return false;
    };
    let child = OperationId(identity.operation_id.clone());
    let operation = shared
        .settlement_parents
        .lock()
        .await
        .get(&child)
        .cloned()
        .unwrap_or(child);
    let fence = shared
        .settlement_fences
        .lock()
        .await
        .get(&operation)
        .cloned();
    let Some(fence) = fence else {
        // Only this daemon's exact, previously reconciled wire identity may be
        // acknowledged after its fence has been cleared. A fresh daemon must
        // leave retained settlements from a prior generation on the host.
        if shared.handled_settlements.lock().await.contains(&identity) {
            return true;
        }
        let key = TerminalSettlementOperation {
            operation_id: operation.clone(),
            daemon_generation: identity.daemon_generation.clone(),
        };
        let mut terminal = shared.terminal_settlement_operations.lock().await;
        let Some(index) = terminal.iter().position(|candidate| candidate == &key) else {
            return false;
        };
        terminal.remove(index);
        drop(terminal);
        remember_handled_settlement(shared, identity).await;
        return true;
    };
    if !settlement_matches(&fence, browser_id, &message) {
        return false;
    }
    if unknown {
        let result = control
            .settle(
                operation.clone(),
                SettlementOutcome::Error("settlement_unknown".to_owned()),
            )
            .await;
        if let SettlementResult::Settled(completion) = result {
            settle_operation_reservation(shared, control, &completion).await;
            clear_operation_correlations(shared, &operation).await;
        }
        control
            .tick(now_ms().saturating_add(SETTLEMENT_DEADLINE_MS))
            .await;
        remember_handled_settlement(shared, identity).await;
        return true;
    }
    let completion = message
        .pointer("/params/completion")
        .cloned()
        .unwrap_or(Value::Null);
    let outcome = if let Some(error) = completion.get("error") {
        SettlementOutcome::Error(error.to_string())
    } else {
        SettlementOutcome::DefinitiveSuccess(
            completion.get("result").unwrap_or(&completion).to_string(),
        )
    };
    match control.settle(operation.clone(), outcome).await {
        SettlementResult::Settled(completion) => {
            settle_operation_reservation(shared, control, &completion).await;
            clear_operation_correlations(shared, &operation).await;
        }
        SettlementResult::RemainsAmbiguous => return false,
        SettlementResult::Ignored => {
            clear_operation_correlations(shared, &operation).await;
        }
    }
    remember_handled_settlement(shared, identity).await;
    true
}

fn settlement_ack_identity(message: &Value) -> Option<SettlementAckIdentity> {
    let params = message.get("params").and_then(Value::as_object)?;
    let operation_id = params.get("operation_id")?.as_str()?.to_owned();
    let daemon_generation = params.get("daemon_generation")?.as_str()?.to_owned();
    let actor_generation = params.get("actor_generation")?.clone();
    let chrome_request_id = params.get("chrome_request_id")?.as_str()?.to_owned();
    if operation_id.is_empty()
        || daemon_generation.is_empty()
        || chrome_request_id.is_empty()
        || !(actor_generation.is_string() || actor_generation.is_number())
    {
        return None;
    }
    Some(SettlementAckIdentity {
        operation_id,
        daemon_generation,
        actor_generation,
        chrome_request_id,
    })
}

async fn remember_handled_settlement(shared: &Shared, identity: SettlementAckIdentity) {
    let mut handled = shared.handled_settlements.lock().await;
    if handled.contains(&identity) {
        return;
    }
    handled.push_back(identity);
    while handled.len() > HANDLED_SETTLEMENT_LIMIT {
        handled.pop_front();
    }
}

fn settlement_matches(fence: &SettlementFence, browser_id: &str, message: &Value) -> bool {
    let params = match message.get("params") {
        Some(Value::Object(params)) => params,
        _ => return false,
    };
    fence.browser_instance_id.0 == browser_id
        && params.get("daemon_generation").and_then(Value::as_str)
            == Some(fence.daemon_generation.as_str())
        && params.get("actor_generation") == Some(&fence.actor_generation)
        && params.get("target_lifetime_key") == Some(&fence.target_lifetime_key)
        && params.get("operation_class").and_then(Value::as_str) == Some(fence.operation_class)
}
