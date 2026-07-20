use std::sync::Arc;

use tokio::sync::mpsc;

use super::{
    State,
    introspection::{admission_error_code, lifecycle_result},
};
use crate::browser::control_plane::{
    control::{CancelResult, Command, QueueLimits, SubmitCommand},
    group::GroupRegistry,
    introspection::{EventContext, EventRecorder},
    operation::{Executor, OperationScope, SettlementResult},
    persistence::JournalWriter,
};
use sky_cua_platform::model::{BrowserControlEventKind, BrowserTabKey};

fn admit_submission(state: &mut State, (request, reply): SubmitCommand) {
    let context = EventContext {
        group_id: request.group_id.as_ref().map(|group| group.0.clone()),
        tab_key: match &request.scope {
            OperationScope::Tab(tab) => Some(BrowserTabKey {
                browser_instance_id: tab.browser_instance_id.0.clone(),
                extension_tab_id: tab.tab_id.clone(),
            }),
            OperationScope::BridgeGlobal(_) | OperationScope::DaemonGlobal => None,
        },
        operation_id: request.operation_id.as_ref().map(|id| id.0.clone()),
        ..Default::default()
    };
    state.advance_time(request.now_ms);
    if let Err((error, reply)) = state.admit(*request, reply) {
        state.events.record(
            BrowserControlEventKind::Lifecycle {
                state: format!("admission_failed:{}", admission_error_code(&error)),
            },
            context,
        );
        let _ = reply.send(Err(error));
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::browser::control_plane::scheduler) async fn run_actor(
    mut receiver: mpsc::UnboundedReceiver<Command>,
    mut submit_receiver: mpsc::Receiver<SubmitCommand>,
    sender: mpsc::UnboundedSender<Command>,
    executor: Arc<dyn Executor>,
    generation: String,
    limits: QueueLimits,
    groups: GroupRegistry,
    events: EventRecorder,
    persistence: Option<JournalWriter>,
) {
    let mut state = State::new(generation, limits, groups, events, persistence);
    loop {
        let command = tokio::select! {
            biased;
            command = receiver.recv() => command.map(ActorInput::Internal),
            submit = submit_receiver.recv() => submit.map(ActorInput::Submit),
        };
        let Some(command) = command else {
            break;
        };
        // A completion must observe submissions already accepted by the
        // bounded ingress so fairness rounds are computed from the same queue
        // state as the former single FIFO mailbox. The batch is finite, so the
        // completion lane remains non-starvable. Cancellation and lifecycle
        // commands intentionally run first when selected.
        if matches!(command, ActorInput::Internal(Command::Executed(_, _))) {
            while let Ok(submit) = submit_receiver.try_recv() {
                admit_submission(&mut state, submit);
            }
        }
        match command {
            ActorInput::Submit(submit) => admit_submission(&mut state, submit),
            ActorInput::Internal(Command::Cancel(operation_id, client_id, reply)) => {
                let result = state.cancel(&operation_id, client_id.as_ref());
                if result == CancelResult::UnknownOperation {
                    state.record_lifecycle(None, "cancel_failed:unknown_operation");
                }
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Executed(operation_id, outcome)) => {
                state.complete(&operation_id, outcome);
            }
            ActorInput::Internal(Command::Settle(operation_id, outcome, reply)) => {
                let result = state.settle(&operation_id, outcome);
                let event_state = match &result {
                    SettlementResult::Settled(_) => "settled",
                    SettlementResult::RemainsAmbiguous => "remains_ambiguous",
                    SettlementResult::Ignored => "ignored",
                };
                state.record_settlement_event(&operation_id, event_state);
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::SettlementState(operation_id, reply)) => {
                let _ = reply.send(state.settlement_state(&operation_id));
            }
            ActorInput::Internal(Command::CreateGroup {
                group_id,
                browser,
                principal,
                now_ms,
                reply,
            }) => {
                state.advance_time(now_ms);
                let group = state.groups.create(group_id, browser, principal, now_ms);
                state.record_lifecycle(Some(&group.group_id), "group_created");
                let _ = reply.send(group);
            }
            ActorInput::Internal(Command::AddMember {
                group_id,
                principal,
                tab,
                reply,
            }) => {
                let result = state.groups.add_member(&group_id, &principal, tab);
                state.record_lifecycle(
                    Some(&group_id),
                    lifecycle_result("member_added", "member_add_failed", &result),
                );
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Group(group_id, reply)) => {
                let result = state.groups.get(&group_id).cloned();
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Groups(reply)) => {
                let _ = reply.send(state.groups.all().cloned().collect());
            }
            ActorInput::Internal(Command::BrowserLost(browser, reply)) => {
                let affected = state.browser_lost(&browser);
                for group_id in &affected {
                    state.record_lifecycle(Some(group_id), "browser_lost");
                }
                let _ = reply.send(affected);
            }
            ActorInput::Internal(Command::Renew(proof, principal, now_ms, reply)) => {
                state.advance_time(now_ms);
                let result = state.groups.renew(&proof, &principal, now_ms);
                state.record_lifecycle(
                    Some(&proof.group_id),
                    lifecycle_result("lease_renewed", "lease_renew_failed", &result),
                );
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Offer {
                group_id,
                principal,
                target,
                revision,
                reply,
            }) => {
                let result = state
                    .groups
                    .offer_handoff(&group_id, &principal, target, revision);
                if result.is_ok() {
                    state.cancel_queued_for_group(&group_id);
                }
                state.record_lifecycle(
                    Some(&group_id),
                    lifecycle_result("handoff_offered", "handoff_offer_failed", &result),
                );
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Accept {
                group_id,
                target,
                revision,
                now_ms,
                reply,
            }) => {
                state.advance_time(now_ms);
                let count = state.group_in_flight(&group_id);
                let result = state
                    .groups
                    .accept_handoff(&group_id, &target, revision, count, now_ms);
                state.record_lifecycle(
                    Some(&group_id),
                    lifecycle_result("handoff_accepted", "handoff_accept_failed", &result),
                );
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Force {
                group_id,
                requester,
                target,
                revision,
                now_ms,
                reply,
            }) => {
                state.advance_time(now_ms);
                let count = state.group_in_flight(&group_id);
                let result = state
                    .groups
                    .force_handoff(&group_id, &requester, target, revision, count, now_ms);
                state.record_lifecycle(
                    Some(&group_id),
                    lifecycle_result("handoff_forced", "handoff_force_failed", &result),
                );
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Disconnect(principal, now_ms, reply)) => {
                state.advance_time(now_ms);
                state.cancel_queued_for_principal(&principal);
                state.detach_in_flight_for_principal(&principal);
                state.groups.mark_disconnected(&principal, now_ms);
                state.record_lifecycle(None, "principal_disconnected");
                let _ = reply.send(());
            }
            ActorInput::Internal(Command::EndGroup(group_id, principal, reply)) => {
                state.cancel_queued_for_group(&group_id);
                let in_flight = state.group_in_flight(&group_id);
                let result = state.groups.end_lifecycle(&group_id, &principal, in_flight);
                state.record_lifecycle(
                    Some(&group_id),
                    lifecycle_result("group_ended", "group_end_failed", &result),
                );
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Tick(now_ms, reply)) => {
                state.advance_time(now_ms);
                state.expire_settlements();
                let in_flight = state.in_flight_by_group.clone();
                let expired = state
                    .groups
                    .expire(now_ms, |group| in_flight.get(group).copied().unwrap_or(0));
                if expired {
                    state.record_lifecycle(None, "lease_expiry_applied");
                }
                let _ = reply.send(());
            }
            ActorInput::Internal(Command::ResumeRecovered {
                group_id,
                browser,
                principal,
                members,
                revision,
                now_ms,
                reply,
            }) => {
                state.advance_time(now_ms);
                let result = state
                    .groups
                    .resume_recovered(&group_id, &browser, &principal, &members, revision, now_ms);
                state.record_lifecycle(
                    Some(&group_id),
                    lifecycle_result("recovery_resumed", "recovery_resume_failed", &result),
                );
                let _ = reply.send(result);
            }
            ActorInput::Internal(Command::Snapshot(reply)) => {
                let _ = reply.send(state.snapshot());
            }
            ActorInput::Internal(Command::PruneReleased(reply)) => {
                let in_flight = state.in_flight_by_group.clone();
                let released = state
                    .groups
                    .prune_released(|group| in_flight.get(group).copied().unwrap_or(0));
                let _ = reply.send(released);
            }
        }

        // State bookkeeping is complete before executor futures are created and
        // spawned. The actor owns state exclusively and never awaits executor I/O.
        let dispatches = state.dispatch_ready();
        // This only hands an authority-free snapshot to a dedicated writer
        // thread. No scheduler/group state is held across filesystem I/O.
        state.persist_recovery_hints();
        for dispatch in dispatches {
            let operation_id = dispatch.identity.operation_id.clone();
            let executor = executor.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let outcome = executor.execute(dispatch).await;
                let _ = sender.send(Command::Executed(operation_id, outcome));
            });
        }
    }
}

enum ActorInput {
    Internal(Command),
    Submit(SubmitCommand),
}
