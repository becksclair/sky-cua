use std::collections::{HashSet, VecDeque};

use super::{OperationState, State};
use crate::browser::control_plane::{
    group::GroupError,
    operation::{
        BrowserInstanceId, Completion, CompletionDisposition, DispatchOperation, ExecutorOutcome,
        GroupId, OperationClass, OperationId, OperationScope, Principal, SETTLEMENT_DEADLINE_MS,
        SettlementOutcome, SettlementResult, SettlementState, TabKey,
    },
};

impl State {
    pub(super) fn dispatch_ready(&mut self) -> Vec<DispatchOperation> {
        let mut ready = Vec::new();
        while let Some(operation_id) = self.next_ready() {
            let Some(record) = self.operations.get(&operation_id) else {
                continue;
            };
            let validation = match (&record.dispatch.scope, &record.dispatch.group_id) {
                (OperationScope::Tab(tab), _) => {
                    record
                        .lease
                        .as_ref()
                        .map_or(Err(GroupError::AdmissionClosed), |proof| {
                            self.groups.validate(
                                proof,
                                &record.dispatch.principal,
                                Some(tab),
                                self.now_ms,
                            )
                        })
                }
                (OperationScope::BridgeGlobal(browser), Some(group_id)) => {
                    self.groups.validate_bridge_global(
                        group_id,
                        &record.dispatch.principal,
                        browser,
                        self.now_ms,
                    )
                }
                (OperationScope::BridgeGlobal(_), None) | (OperationScope::DaemonGlobal, _) => {
                    Ok(())
                }
            };
            if let Err(error) = validation {
                self.reject_before_dispatch(&operation_id, error);
                continue;
            }
            self.mark_dispatched(&operation_id);
            ready.push(self.operations[&operation_id].dispatch.clone());
        }
        ready
    }

    fn next_ready(&mut self) -> Option<OperationId> {
        if self.daemon_global_in_flight {
            return None;
        }
        if !self.daemon_globals.is_empty() && self.daemon_tab_round.is_none() {
            if self.any_in_flight() {
                return None;
            }
            self.daemon_global_in_flight = true;
            return self.daemon_globals.pop_front();
        }

        let tabs: Vec<_> = self.tab_queues.keys().cloned().collect();
        for tab in tabs {
            if self.tab_in_flight.contains(&tab)
                || self
                    .bridge_dispatch_in_flight
                    .get(&tab.browser_instance_id)
                    .copied()
                    .unwrap_or(0)
                    >= self.limits.per_bridge_dispatch.max(1)
                || self.tab_queues.get(&tab).is_none_or(VecDeque::is_empty)
            {
                continue;
            }
            let head_is_settlement_blocked = self
                .tab_queues
                .get(&tab)
                .and_then(|queue| queue.front())
                .and_then(|operation_id| self.operations.get(operation_id))
                .and_then(|record| record.dispatch.group_id.as_ref())
                .is_some_and(|group_id| self.groups.has_unresolved_settlement(group_id));
            if head_is_settlement_blocked {
                continue;
            }
            if !self.daemon_globals.is_empty()
                && self
                    .daemon_tab_round
                    .as_ref()
                    .is_none_or(|round| !round.contains(&tab))
            {
                continue;
            }
            let bridge = self
                .bridges
                .entry(tab.browser_instance_id.clone())
                .or_default();
            if bridge.global_in_flight {
                continue;
            }
            if !bridge.globals.is_empty()
                && bridge
                    .tab_round
                    .as_ref()
                    .is_none_or(|round| !round.contains(&tab))
            {
                continue;
            }
            if let Some(round) = bridge.tab_round.as_mut() {
                round.remove(&tab);
            }
            if let Some(round) = self.daemon_tab_round.as_mut() {
                round.remove(&tab);
            }
            self.tab_in_flight.insert(tab.clone());
            return self.tab_queues.get_mut(&tab).and_then(VecDeque::pop_front);
        }

        if self.daemon_globals.is_empty()
            || (self
                .daemon_tab_round
                .as_ref()
                .is_some_and(HashSet::is_empty)
                && !self.any_tab_in_flight())
        {
            self.daemon_tab_round = None;
        }

        let browsers: Vec<_> = self.bridges.keys().cloned().collect();
        for browser in browsers {
            let active_for_bridge = self
                .tab_in_flight
                .iter()
                .any(|tab| tab.browser_instance_id == browser);
            let bridge = self.bridges.get_mut(&browser).expect("known bridge");
            if bridge.global_in_flight || bridge.globals.is_empty() {
                continue;
            }
            if bridge.tab_round.as_ref().is_some_and(HashSet::is_empty) && !active_for_bridge {
                bridge.tab_round = None;
            }
            if bridge.tab_round.is_none() && !active_for_bridge {
                bridge.global_in_flight = true;
                return bridge.globals.pop_front();
            }
        }
        None
    }

    fn mark_dispatched(&mut self, operation_id: &OperationId) {
        let (client, group, lease, principal, admitted_at_ms) = {
            let record = self
                .operations
                .get_mut(operation_id)
                .expect("queued operation exists");
            record.state = OperationState::Dispatched;
            (
                record.dispatch.client_id.clone(),
                record.dispatch.group_id.clone(),
                record.lease.clone(),
                record.dispatch.principal.clone(),
                record.admitted_at_ms,
            )
        };
        if let Some(depth) = self.queued_by_client.get_mut(&client) {
            *depth = depth.saturating_sub(1);
        }
        if let Some(group) = &group {
            *self.in_flight_by_group.entry(group.clone()).or_default() += 1;
        }
        if let Some(browser) = operation_browser(&self.operations[operation_id].dispatch.scope) {
            *self
                .bridge_dispatch_in_flight
                .entry(browser.clone())
                .or_default() += 1;
        }
        self.record_operation_event(operation_id, "in_flight");
        self.record_queue_depth();
        if let Some(proof) = lease {
            let _ = self
                .groups
                .renew(&proof, &principal, self.now_ms.max(admitted_at_ms));
        }
    }

    fn reject_before_dispatch(&mut self, operation_id: &OperationId, error: GroupError) {
        let completion = Completion {
            operation_id: operation_id.clone(),
            certainty: super::super::operation::CompletionCertainty::PreDispatchRejected,
            disposition: CompletionDisposition::Failure,
            detail: format!("pre-dispatch lease rejection: {error:?}"),
        };
        let rejected_scope = self
            .operations
            .get(operation_id)
            .map(|record| record.dispatch.scope.clone());
        if let Some(record) = self.operations.get_mut(operation_id) {
            record.state = OperationState::Terminal;
            record.completion = Some(completion.clone());
            for waiter in record.waiters.drain(..) {
                let _ = waiter.send(Ok(completion.clone()));
            }
        }
        match rejected_scope {
            Some(OperationScope::Tab(tab)) => {
                self.tab_in_flight.remove(&tab);
            }
            Some(OperationScope::BridgeGlobal(browser)) => {
                if let Some(bridge) = self.bridges.get_mut(&browser) {
                    bridge.global_in_flight = false;
                }
            }
            Some(OperationScope::DaemonGlobal) => self.daemon_global_in_flight = false,
            None => {}
        }
        self.remove_from_queues(operation_id);
        self.remember(operation_id.clone());
    }

    pub(super) fn complete(&mut self, operation_id: &OperationId, outcome: ExecutorOutcome) {
        let Some(record) = self.operations.get_mut(operation_id) else {
            return;
        };
        if record.state != OperationState::Dispatched {
            return;
        }
        let ambiguous_mutation = matches!(outcome, ExecutorOutcome::Ambiguous(_))
            && matches!(
                record.dispatch.class,
                OperationClass::Mutation | OperationClass::BrowserGlobal
            );
        if ambiguous_mutation {
            let detail = match outcome {
                ExecutorOutcome::Ambiguous(detail) => detail,
                _ => unreachable!(),
            };
            let mut completion = Completion::detached(operation_id.clone());
            completion.detail = detail;
            let scope = record.dispatch.scope.clone();
            let group = record.dispatch.group_id.clone();
            record.state = OperationState::SettlementPending {
                deadline_ms: self.now_ms.saturating_add(SETTLEMENT_DEADLINE_MS),
            };
            record.completion = Some(completion.clone());
            self.record_operation_event(operation_id, "settlement_pending");
            self.record_settlement_event(operation_id, "pending");
            self.release_executor_capacity(&scope, group.as_ref());
            if let Some(group_id) = group {
                self.groups
                    .begin_settlement(&group_id, operation_id.clone());
            }
            if let Some(early) = self.early_settlements.remove(operation_id)
                && let SettlementResult::Settled(settled) = self.settle(operation_id, early)
            {
                if let Some(record) = self.operations.get_mut(operation_id) {
                    for waiter in record.waiters.drain(..) {
                        let _ = waiter.send(Ok(settled.clone()));
                    }
                }
                return;
            }
            if let Some(record) = self.operations.get_mut(operation_id) {
                for waiter in record.waiters.drain(..) {
                    let _ = waiter.send(Ok(completion.clone()));
                }
            }
            return;
        }
        let completion = Completion::from_executor(operation_id.clone(), outcome);
        let scope = record.dispatch.scope.clone();
        let group = record.dispatch.group_id.clone();
        record.state = OperationState::Terminal;
        record.completion = Some(completion.clone());
        for waiter in record.waiters.drain(..) {
            let _ = waiter.send(Ok(completion.clone()));
        }
        self.release_executor_capacity(&scope, group.as_ref());
        if let Some(group_id) = group {
            self.groups.finish_execution(&group_id);
        }
        self.remember(operation_id.clone());
        self.early_settlements.remove(operation_id);
        self.record_operation_event(operation_id, "terminal");
    }

    fn release_executor_capacity(&mut self, scope: &OperationScope, group: Option<&GroupId>) {
        if let Some(browser) = operation_browser(scope)
            && let Some(count) = self.bridge_dispatch_in_flight.get_mut(browser)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.bridge_dispatch_in_flight.remove(browser);
            }
        }
        match scope {
            OperationScope::Tab(tab) => {
                self.tab_in_flight.remove(tab);
            }
            OperationScope::BridgeGlobal(browser) => {
                let waiting = self.waiting_tabs(Some(browser));
                let bridge = self.bridges.entry(browser.clone()).or_default();
                bridge.global_in_flight = false;
                bridge.tab_round = (!waiting.is_empty()).then_some(waiting);
            }
            OperationScope::DaemonGlobal => {
                self.daemon_global_in_flight = false;
                let waiting = self.waiting_tabs(None);
                self.daemon_tab_round = (!waiting.is_empty()).then_some(waiting);
            }
        }
        if let Some(group_id) = group
            && let Some(count) = self.in_flight_by_group.get_mut(group_id)
        {
            *count = count.saturating_sub(1);
        }
    }

    pub(super) fn settlement_state(&self, operation_id: &OperationId) -> Option<SettlementState> {
        match self.operations.get(operation_id)?.state {
            OperationState::SettlementPending { deadline_ms } => {
                Some(SettlementState::Pending { deadline_ms })
            }
            OperationState::SettlementUnknown { deadline_ms } => {
                Some(SettlementState::Unknown { deadline_ms })
            }
            _ => None,
        }
    }

    pub(super) fn settle(
        &mut self,
        operation_id: &OperationId,
        outcome: SettlementOutcome,
    ) -> SettlementResult {
        let Some(record) = self.operations.get(operation_id) else {
            return SettlementResult::Ignored;
        };
        if record.state == OperationState::Dispatched
            && matches!(
                record.dispatch.class,
                OperationClass::Mutation | OperationClass::BrowserGlobal
            )
        {
            self.early_settlements.insert(operation_id.clone(), outcome);
            return SettlementResult::RemainsAmbiguous;
        }
        if !matches!(
            record.state,
            OperationState::SettlementPending { .. } | OperationState::SettlementUnknown { .. }
        ) {
            return SettlementResult::Ignored;
        }
        let group_id = record.dispatch.group_id.clone();
        let completion = match outcome {
            SettlementOutcome::DefinitiveSuccess(detail) => {
                Completion::settlement_success(operation_id.clone(), detail)
            }
            SettlementOutcome::ProvenPreDispatchFailure(detail) => {
                Completion::settlement_pre_dispatch_failure(operation_id.clone(), detail)
            }
            SettlementOutcome::Error(_) => return SettlementResult::RemainsAmbiguous,
            SettlementOutcome::TargetLost(tab) => {
                if !matches!(&record.dispatch.scope, OperationScope::Tab(expected) if expected == &tab)
                {
                    return SettlementResult::Ignored;
                }
                if let Some(group_id) = &group_id {
                    self.groups.remove_target(group_id, &tab);
                }
                Completion::target_lost(operation_id.clone(), "target_lost".into())
            }
            SettlementOutcome::BrowserLost(browser) => {
                let expected = match &record.dispatch.scope {
                    OperationScope::Tab(tab) => &tab.browser_instance_id,
                    OperationScope::BridgeGlobal(expected) => expected,
                    OperationScope::DaemonGlobal => return SettlementResult::Ignored,
                };
                if expected != &browser {
                    return SettlementResult::Ignored;
                }
                if let Some(group_id) = &group_id {
                    self.groups.remove_browser_targets(group_id, &browser);
                }
                Completion::target_lost(operation_id.clone(), "target_lost".into())
            }
        };
        let record = self
            .operations
            .get_mut(operation_id)
            .expect("settlement operation still exists");
        record.state = OperationState::Terminal;
        record.completion = Some(completion.clone());
        if let Some(group_id) = group_id {
            self.groups.finish_settlement(&group_id, operation_id);
        }
        self.remember(operation_id.clone());
        self.record_operation_event(operation_id, "settled");
        self.record_settlement_event(operation_id, "resolved");
        SettlementResult::Settled(completion)
    }

    pub(super) fn expire_settlements(&mut self) {
        let expired: Vec<_> = self
            .operations
            .iter()
            .filter_map(|(id, record)| match record.state {
                OperationState::SettlementPending { deadline_ms } if self.now_ms >= deadline_ms => {
                    Some((id.clone(), deadline_ms, record.dispatch.group_id.clone()))
                }
                _ => None,
            })
            .collect();
        for (operation_id, deadline_ms, group_id) in expired {
            if let Some(record) = self.operations.get_mut(&operation_id) {
                record.state = OperationState::SettlementUnknown { deadline_ms };
            }
            self.record_operation_event(&operation_id, "settlement_unknown");
            self.record_settlement_event(&operation_id, "unknown");
            if let Some(group_id) = group_id {
                self.groups
                    .mark_settlement_unknown(&group_id, &operation_id);
            }
        }
    }

    fn waiting_tabs(&self, browser: Option<&BrowserInstanceId>) -> HashSet<TabKey> {
        self.tab_queues
            .iter()
            .filter(|(tab, queue)| {
                !queue.is_empty()
                    && browser.is_none_or(|browser| &tab.browser_instance_id == browser)
            })
            .map(|(tab, _)| tab.clone())
            .collect()
    }

    fn any_in_flight(&self) -> bool {
        self.daemon_global_in_flight
            || !self.tab_in_flight.is_empty()
            || self.bridges.values().any(|bridge| bridge.global_in_flight)
    }

    fn any_tab_in_flight(&self) -> bool {
        !self.tab_in_flight.is_empty()
    }

    pub(super) fn group_in_flight(&self, group_id: &GroupId) -> usize {
        self.in_flight_by_group.get(group_id).copied().unwrap_or(0)
    }

    pub(super) fn advance_time(&mut self, now_ms: u64) {
        self.now_ms = self.now_ms.max(now_ms);
    }

    pub(super) fn cancel_queued_for_principal(&mut self, principal: &Principal) {
        let ids: Vec<_> = self
            .operations
            .iter()
            .filter(|(_, record)| {
                record.state == OperationState::Queued && record.dispatch.principal == *principal
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.cancel(&id, None);
        }
    }

    pub(super) fn browser_lost(&mut self, browser: &BrowserInstanceId) -> Vec<GroupId> {
        let queued = self
            .operations
            .iter()
            .filter(|(_, record)| {
                record.state == OperationState::Queued
                    && match &record.dispatch.scope {
                        OperationScope::Tab(tab) => &tab.browser_instance_id == browser,
                        OperationScope::BridgeGlobal(candidate) => candidate == browser,
                        OperationScope::DaemonGlobal => false,
                    }
            })
            .map(|(operation_id, _)| operation_id.clone())
            .collect::<Vec<_>>();
        for operation_id in queued {
            self.reject_before_dispatch(&operation_id, GroupError::AdmissionClosed);
        }
        self.groups.browser_lost(browser)
    }

    pub(super) fn detach_in_flight_for_principal(&mut self, principal: &Principal) {
        let ids: Vec<_> = self
            .operations
            .iter()
            .filter(|(_, record)| {
                record.state == OperationState::Dispatched
                    && record.dispatch.principal == *principal
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.cancel(&id, None);
        }
    }

    pub(super) fn cancel_queued_for_group(&mut self, group_id: &GroupId) {
        let ids: Vec<_> = self
            .operations
            .iter()
            .filter(|(_, record)| {
                record.state == OperationState::Queued
                    && record.dispatch.group_id.as_ref() == Some(group_id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.cancel(&id, None);
        }
    }
}

fn operation_browser(scope: &OperationScope) -> Option<&BrowserInstanceId> {
    match scope {
        OperationScope::Tab(tab) => Some(&tab.browser_instance_id),
        OperationScope::BridgeGlobal(browser) => Some(browser),
        OperationScope::DaemonGlobal => None,
    }
}
