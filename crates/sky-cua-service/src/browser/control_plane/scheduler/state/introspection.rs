use super::{OperationRecord, OperationState, State};
use crate::browser::control_plane::{
    control::AdmissionError,
    group::GroupError,
    introspection::{EventContext, GROUP_MEMBER_LIMIT, GROUP_RESULT_LIMIT, RECENT_OPERATION_LIMIT},
    operation::{
        CompletionCertainty, CompletionDisposition, GroupId, OperationClass, OperationId,
        OperationScope,
    },
    persistence::RecoveryJournal,
};
use sky_cua_platform::model::{
    BrowserCompletionCertainty, BrowserControlEventKind, BrowserControlOperationSummary,
    BrowserControlSchedulerSnapshot, BrowserOperationClass, BrowserTabKey,
};

impl State {
    pub(super) fn persist_recovery_hints(&mut self) {
        let Some(writer) = &self.persistence else {
            return;
        };
        let unresolved = self
            .operations
            .values()
            .filter(|record| {
                matches!(
                    record.dispatch.class,
                    OperationClass::Mutation | OperationClass::BrowserGlobal
                ) && matches!(
                    record.state,
                    OperationState::Dispatched
                        | OperationState::SettlementPending { .. }
                        | OperationState::SettlementUnknown { .. }
                )
            })
            .filter_map(|record| record.dispatch.group_id.clone())
            .collect();
        let journal = RecoveryJournal::capture(&self.groups, &unresolved);
        if self.last_journal.as_ref() != Some(&journal) {
            writer.enqueue(journal.clone());
            self.last_journal = Some(journal);
        }
    }

    pub(super) fn snapshot(&self) -> BrowserControlSchedulerSnapshot {
        let queued_count = self
            .operations
            .values()
            .filter(|record| record.state == OperationState::Queued)
            .count();
        let in_flight_count = self
            .operations
            .values()
            .filter(|record| record.state == OperationState::Dispatched)
            .count();
        let (settlement_pending_count, settlement_unknown_count) = self.groups.settlement_counts();
        let (groups, groups_omitted) =
            self.groups
                .introspection_summaries(GROUP_RESULT_LIMIT, GROUP_MEMBER_LIMIT, |group| {
                    self.group_in_flight(group)
                });
        let mut operations = self.operations.values().collect::<Vec<_>>();
        operations.sort_by(|left, right| {
            right
                .admitted_at_ms
                .cmp(&left.admitted_at_ms)
                .then_with(|| {
                    left.dispatch
                        .identity
                        .operation_id
                        .0
                        .cmp(&right.dispatch.identity.operation_id.0)
                })
        });
        let recent_operations_omitted =
            bounded_u32(operations.len().saturating_sub(RECENT_OPERATION_LIMIT));
        let recent_operations = operations
            .into_iter()
            .take(RECENT_OPERATION_LIMIT)
            .map(operation_summary)
            .collect();
        BrowserControlSchedulerSnapshot {
            queued_count: bounded_u32(queued_count),
            in_flight_count: bounded_u32(in_flight_count),
            settlement_pending_count: bounded_u32(settlement_pending_count),
            settlement_unknown_count: bounded_u32(settlement_unknown_count),
            queued_client_count: bounded_u32(
                self.queued_by_client
                    .values()
                    .filter(|depth| **depth != 0)
                    .count(),
            ),
            groups,
            groups_omitted,
            recent_operations,
            recent_operations_omitted,
        }
    }

    pub(super) fn record_operation_event(&self, operation_id: &OperationId, state: &str) {
        let Some(record) = self.operations.get(operation_id) else {
            return;
        };
        let tab_key = match &record.dispatch.scope {
            OperationScope::Tab(tab) => Some(BrowserTabKey {
                browser_instance_id: tab.browser_instance_id.0.clone(),
                extension_tab_id: tab.tab_id.clone(),
            }),
            OperationScope::BridgeGlobal(_) | OperationScope::DaemonGlobal => None,
        };
        self.events.record(
            BrowserControlEventKind::OperationState {
                state: state.to_owned(),
            },
            EventContext {
                principal_id: None,
                group_id: record
                    .dispatch
                    .group_id
                    .as_ref()
                    .map(|group| group.0.clone()),
                tab_key,
                operation_id: Some(operation_id.0.clone()),
            },
        );
    }

    pub(super) fn record_settlement_event(&self, operation_id: &OperationId, state: &str) {
        self.events.record(
            BrowserControlEventKind::Settlement {
                state: state.to_owned(),
            },
            EventContext {
                group_id: self
                    .operations
                    .get(operation_id)
                    .and_then(|record| record.dispatch.group_id.as_ref())
                    .map(|group| group.0.clone()),
                operation_id: Some(operation_id.0.clone()),
                ..Default::default()
            },
        );
    }

    pub(super) fn record_queue_depth(&self) {
        self.events.record(
            BrowserControlEventKind::QueueState {
                depth: bounded_u32(
                    self.operations
                        .values()
                        .filter(|record| record.state == OperationState::Queued)
                        .count(),
                ),
            },
            EventContext::default(),
        );
    }

    pub(super) fn record_lifecycle(&self, group_id: Option<&GroupId>, state: impl Into<String>) {
        self.events.record(
            BrowserControlEventKind::Lifecycle {
                state: state.into(),
            },
            EventContext {
                group_id: group_id.map(|group| group.0.clone()),
                ..Default::default()
            },
        );
    }
}

fn operation_summary(record: &OperationRecord) -> BrowserControlOperationSummary {
    let tab_key = match &record.dispatch.scope {
        OperationScope::Tab(tab) => Some(BrowserTabKey {
            browser_instance_id: tab.browser_instance_id.0.clone(),
            extension_tab_id: tab.tab_id.clone(),
        }),
        OperationScope::BridgeGlobal(_) | OperationScope::DaemonGlobal => None,
    };
    BrowserControlOperationSummary {
        operation_id: record.dispatch.identity.operation_id.0.clone(),
        client_id: record.dispatch.client_id.0.clone(),
        class: match record.dispatch.class {
            OperationClass::ReadOnly => BrowserOperationClass::ReadOnly,
            OperationClass::AbsoluteSet => BrowserOperationClass::AbsoluteSet,
            OperationClass::Mutation | OperationClass::BrowserGlobal => {
                BrowserOperationClass::Mutation
            }
        },
        state: operation_state(record.state).to_owned(),
        admitted_at_ms: record.admitted_at_ms,
        group_id: record
            .dispatch
            .group_id
            .as_ref()
            .map(|group| group.0.clone()),
        tab_key,
        completion: record
            .completion
            .as_ref()
            .map(|completion| match completion.certainty {
                CompletionCertainty::PreDispatchRejected => {
                    BrowserCompletionCertainty::PreDispatchRejected
                }
                CompletionCertainty::Ambiguous => BrowserCompletionCertainty::AmbiguousCompletion,
                CompletionCertainty::Definitive => {
                    if completion.disposition == CompletionDisposition::Success {
                        BrowserCompletionCertainty::DefinitiveSuccess
                    } else {
                        BrowserCompletionCertainty::DefinitiveFailure
                    }
                }
            }),
    }
}

fn operation_state(state: OperationState) -> &'static str {
    match state {
        OperationState::Queued => "queued",
        OperationState::Dispatched => "in_flight",
        OperationState::SettlementPending { .. } => "settlement_pending",
        OperationState::SettlementUnknown { .. } => "settlement_unknown",
        OperationState::Terminal => "terminal",
    }
}

fn bounded_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(super) fn lifecycle_result<T>(
    success: &str,
    failure: &str,
    result: &Result<T, GroupError>,
) -> String {
    match result {
        Ok(_) => success.to_owned(),
        Err(error) => format!("{failure}:{}", group_error_code(error)),
    }
}

fn group_error_code(error: &GroupError) -> &'static str {
    match error {
        GroupError::UnknownGroup => "unknown_group",
        GroupError::WrongBrowserInstance => "wrong_browser_instance",
        GroupError::WrongPrincipal => "wrong_principal",
        GroupError::StaleFence => "stale_fence",
        GroupError::StaleMembershipRevision => "stale_membership_revision",
        GroupError::AdmissionClosed => "admission_closed",
        GroupError::DifferentUid => "different_uid",
        GroupError::SettlementRequired => "settlement_required",
        GroupError::InFlight => "in_flight",
        GroupError::NoHandoffOffer => "no_handoff_offer",
        GroupError::WrongHandoffTarget => "wrong_handoff_target",
        GroupError::RecoveryIdentityMismatch => "recovery_identity_mismatch",
    }
}

pub(super) fn admission_error_code(error: &AdmissionError) -> &'static str {
    match error {
        AdmissionError::Backpressure => "backpressure",
        AdmissionError::OperationIdCollision => "operation_id_collision",
        AdmissionError::StaleGeneration => "stale_generation",
        AdmissionError::Group(error) => group_error_code(error),
        AdmissionError::LeaseRequired => "lease_required",
        AdmissionError::ActorStopped => "actor_stopped",
    }
}
