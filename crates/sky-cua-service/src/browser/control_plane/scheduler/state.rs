use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use tokio::sync::mpsc;

use sky_cua_platform::model::{
    BrowserCompletionCertainty, BrowserControlEventKind, BrowserControlOperationSummary,
    BrowserControlSchedulerSnapshot, BrowserOperationClass, BrowserTabKey,
};

use super::super::{
    control::{AdmissionError, CancelResult, Command, QueueLimits, Reply, SubmitOperation},
    group::{GroupError, GroupRegistry},
    introspection::{
        EventContext, EventRecorder, GROUP_MEMBER_LIMIT, GROUP_RESULT_LIMIT, RECENT_OPERATION_LIMIT,
    },
    lease::LeaseProof,
    operation::{
        BrowserInstanceId, ClientId, Completion, CompletionDisposition, DispatchOperation,
        Executor, GroupId, OperationClass, OperationId, OperationIdentity, OperationScope,
        Principal, SETTLEMENT_DEADLINE_MS, SettlementOutcome, SettlementResult, SettlementState,
        TabKey,
    },
    persistence::{JournalWriter, RecoveryJournal},
};

struct OperationRecord {
    dispatch: DispatchOperation,
    lease: Option<LeaseProof>,
    state: OperationState,
    waiters: Vec<Reply<Result<Completion, AdmissionError>>>,
    completion: Option<Completion>,
    admitted_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationState {
    Queued,
    Dispatched,
    SettlementPending { deadline_ms: u64 },
    SettlementUnknown { deadline_ms: u64 },
    Terminal,
}

#[derive(Default)]
struct BridgeSchedule {
    globals: VecDeque<OperationId>,
    global_in_flight: bool,
    tab_round: Option<HashSet<TabKey>>,
}

struct State {
    generation: String,
    next_operation: u64,
    limits: QueueLimits,
    operations: HashMap<OperationId, OperationRecord>,
    recent: VecDeque<OperationId>,
    tab_queues: HashMap<TabKey, VecDeque<OperationId>>,
    tab_in_flight: HashSet<TabKey>,
    bridge_dispatch_in_flight: HashMap<BrowserInstanceId, usize>,
    bridges: HashMap<BrowserInstanceId, BridgeSchedule>,
    daemon_globals: VecDeque<OperationId>,
    daemon_global_in_flight: bool,
    daemon_tab_round: Option<HashSet<TabKey>>,
    queued_by_client: HashMap<ClientId, usize>,
    groups: GroupRegistry,
    in_flight_by_group: HashMap<GroupId, usize>,
    early_settlements: HashMap<OperationId, SettlementOutcome>,
    now_ms: u64,
    events: EventRecorder,
    persistence: Option<JournalWriter>,
    last_journal: Option<RecoveryJournal>,
}

impl State {
    fn new(
        generation: String,
        limits: QueueLimits,
        groups: GroupRegistry,
        events: EventRecorder,
        persistence: Option<JournalWriter>,
    ) -> Self {
        Self {
            generation,
            next_operation: 0,
            limits,
            operations: HashMap::new(),
            recent: VecDeque::new(),
            tab_queues: HashMap::new(),
            tab_in_flight: HashSet::new(),
            bridge_dispatch_in_flight: HashMap::new(),
            bridges: HashMap::new(),
            daemon_globals: VecDeque::new(),
            daemon_global_in_flight: false,
            daemon_tab_round: None,
            queued_by_client: HashMap::new(),
            groups,
            in_flight_by_group: HashMap::new(),
            early_settlements: HashMap::new(),
            now_ms: 0,
            events,
            persistence,
            last_journal: None,
        }
    }

    fn persist_recovery_hints(&mut self) {
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

    fn snapshot(&self) -> BrowserControlSchedulerSnapshot {
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

    fn record_operation_event(&self, operation_id: &OperationId, state: &str) {
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

    fn record_settlement_event(&self, operation_id: &OperationId, state: &str) {
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

    fn record_queue_depth(&self) {
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

    fn record_lifecycle(&self, group_id: Option<&GroupId>, state: impl Into<String>) {
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

    fn admit(
        &mut self,
        mut request: SubmitOperation,
        waiter: Reply<Result<Completion, AdmissionError>>,
    ) -> Result<(), (AdmissionError, Reply<Result<Completion, AdmissionError>>)> {
        let operation_id = request.operation_id.take().unwrap_or_else(|| {
            loop {
                self.next_operation += 1;
                let candidate = OperationId(format!(
                    "daemon-op-{}-{}",
                    self.generation, self.next_operation
                ));
                if !self.operations.contains_key(&candidate) {
                    break candidate;
                }
            }
        });
        if let Some(record) = self.operations.get_mut(&operation_id) {
            if record.dispatch.identity.daemon_generation != self.generation {
                return Err((AdmissionError::StaleGeneration, waiter));
            }
            if record.dispatch.identity.canonical_fingerprint != request.canonical_fingerprint {
                return Err((AdmissionError::OperationIdCollision, waiter));
            }
            if let Some(completion) = &record.completion {
                let _ = waiter.send(Ok(completion.clone()));
            } else {
                record.waiters.push(waiter);
            }
            return Ok(());
        }
        if operation_id.0.starts_with("daemon-op-")
            && !operation_id
                .0
                .starts_with(&format!("daemon-op-{}-", self.generation))
        {
            return Err((AdmissionError::StaleGeneration, waiter));
        }
        let client_depth = self
            .queued_by_client
            .get(&request.client_id)
            .copied()
            .unwrap_or(0);
        if client_depth >= self.limits.per_client {
            return Err((AdmissionError::Backpressure, waiter));
        }
        if let OperationScope::Tab(tab) = &request.scope {
            let depth = self.tab_queues.get(tab).map_or(0, VecDeque::len);
            if depth >= self.limits.per_tab {
                return Err((AdmissionError::Backpressure, waiter));
            }
            let Some(proof) = request.lease.as_ref() else {
                return Err((AdmissionError::LeaseRequired, waiter));
            };
            if request.group_id.as_ref() != Some(&proof.group_id) {
                return Err((AdmissionError::LeaseRequired, waiter));
            }
            if let Err(error) =
                self.groups
                    .validate(proof, &request.principal, Some(tab), request.now_ms)
            {
                return Err((AdmissionError::Group(error), waiter));
            }
        }
        if let (OperationScope::BridgeGlobal(browser), Some(group_id)) =
            (&request.scope, &request.group_id)
            && let Err(error) = self.groups.validate_bridge_global(
                group_id,
                &request.principal,
                browser,
                request.now_ms,
            )
        {
            return Err((AdmissionError::Group(error), waiter));
        }
        let identity = OperationIdentity {
            operation_id: operation_id.clone(),
            daemon_generation: self.generation.clone(),
            canonical_fingerprint: request.canonical_fingerprint,
            upstream: request.upstream,
        };
        let dispatch = DispatchOperation {
            identity,
            client_id: request.client_id.clone(),
            principal: request.principal,
            group_id: request.group_id,
            scope: request.scope.clone(),
            class: request.class,
            payload: request.payload,
        };
        self.operations.insert(
            operation_id.clone(),
            OperationRecord {
                dispatch,
                lease: request.lease,
                state: OperationState::Queued,
                waiters: vec![waiter],
                completion: None,
                admitted_at_ms: request.now_ms,
            },
        );
        self.record_operation_event(&operation_id, "queued");
        self.record_queue_depth();
        *self.queued_by_client.entry(request.client_id).or_default() += 1;
        match request.scope {
            OperationScope::Tab(tab) => self
                .tab_queues
                .entry(tab)
                .or_default()
                .push_back(operation_id),
            OperationScope::BridgeGlobal(browser) => self
                .bridges
                .entry(browser)
                .or_default()
                .globals
                .push_back(operation_id),
            OperationScope::DaemonGlobal => self.daemon_globals.push_back(operation_id),
        }
        Ok(())
    }

    fn cancel(&mut self, operation_id: &OperationId) -> CancelResult {
        let Some(record) = self.operations.get_mut(operation_id) else {
            return CancelResult::UnknownOperation;
        };
        if let Some(completion) = &record.completion {
            return CancelResult::AlreadyTerminal(completion.clone());
        }
        match record.state {
            OperationState::Queued => {
                let completion = Completion::cancelled(operation_id.clone());
                record.state = OperationState::Terminal;
                record.completion = Some(completion.clone());
                for waiter in record.waiters.drain(..) {
                    let _ = waiter.send(Ok(completion.clone()));
                }
                self.remove_from_queues(operation_id);
                self.remember(operation_id.clone());
                self.record_operation_event(operation_id, "cancelled_before_dispatch");
                self.record_queue_depth();
                CancelResult::CancelledBeforeDispatch
            }
            OperationState::Dispatched => {
                let detached = Completion::detached(operation_id.clone());
                for waiter in record.waiters.drain(..) {
                    let _ = waiter.send(Ok(detached.clone()));
                }
                self.record_operation_event(operation_id, "waiter_detached");
                CancelResult::WaiterDetached
            }
            OperationState::SettlementPending { .. } | OperationState::SettlementUnknown { .. } => {
                CancelResult::AlreadyTerminal(
                    record
                        .completion
                        .clone()
                        .expect("settlement operation has caller completion"),
                )
            }
            OperationState::Terminal => CancelResult::AlreadyTerminal(
                record
                    .completion
                    .clone()
                    .expect("terminal operation has completion"),
            ),
        }
    }

    fn remove_from_queues(&mut self, operation_id: &OperationId) {
        for queue in self.tab_queues.values_mut() {
            queue.retain(|queued| queued != operation_id);
        }
        for bridge in self.bridges.values_mut() {
            bridge.globals.retain(|queued| queued != operation_id);
        }
        self.daemon_globals.retain(|queued| queued != operation_id);
        self.decrement_queued_client(operation_id);
    }

    fn decrement_queued_client(&mut self, operation_id: &OperationId) {
        let Some(record) = self.operations.get(operation_id) else {
            return;
        };
        if let Some(depth) = self.queued_by_client.get_mut(&record.dispatch.client_id) {
            *depth = depth.saturating_sub(1);
        }
    }

    fn remember(&mut self, operation_id: OperationId) {
        self.recent.push_back(operation_id);
        while self.recent.len() > self.limits.recent_operations {
            if let Some(old) = self.recent.pop_front()
                && self
                    .operations
                    .get(&old)
                    .is_some_and(|record| record.state == OperationState::Terminal)
            {
                self.operations.remove(&old);
            }
        }
    }

    fn dispatch_ready(&mut self) -> Vec<DispatchOperation> {
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
            certainty: super::operation::CompletionCertainty::PreDispatchRejected,
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

    fn complete(&mut self, operation_id: &OperationId, outcome: super::operation::ExecutorOutcome) {
        let Some(record) = self.operations.get_mut(operation_id) else {
            return;
        };
        if record.state != OperationState::Dispatched {
            return;
        }
        let ambiguous_mutation = matches!(outcome, super::operation::ExecutorOutcome::Ambiguous(_))
            && matches!(
                record.dispatch.class,
                OperationClass::Mutation | OperationClass::BrowserGlobal
            );
        if ambiguous_mutation {
            let detail = match outcome {
                super::operation::ExecutorOutcome::Ambiguous(detail) => detail,
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

    fn settlement_state(&self, operation_id: &OperationId) -> Option<SettlementState> {
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

    fn settle(
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

    fn expire_settlements(&mut self) {
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

    fn group_in_flight(&self, group_id: &GroupId) -> usize {
        self.in_flight_by_group.get(group_id).copied().unwrap_or(0)
    }

    fn advance_time(&mut self, now_ms: u64) {
        self.now_ms = self.now_ms.max(now_ms);
    }

    fn cancel_queued_for_principal(&mut self, principal: &Principal) {
        let ids: Vec<_> = self
            .operations
            .iter()
            .filter(|(_, record)| {
                record.state == OperationState::Queued && record.dispatch.principal == *principal
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.cancel(&id);
        }
    }

    fn browser_lost(&mut self, browser: &BrowserInstanceId) -> Vec<GroupId> {
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

    fn detach_in_flight_for_principal(&mut self, principal: &Principal) {
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
            self.cancel(&id);
        }
    }

    fn cancel_queued_for_group(&mut self, group_id: &GroupId) {
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
            self.cancel(&id);
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
                super::operation::CompletionCertainty::PreDispatchRejected => {
                    BrowserCompletionCertainty::PreDispatchRejected
                }
                super::operation::CompletionCertainty::Ambiguous => {
                    BrowserCompletionCertainty::AmbiguousCompletion
                }
                super::operation::CompletionCertainty::Definitive => {
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

fn lifecycle_result<T>(success: &str, failure: &str, result: &Result<T, GroupError>) -> String {
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

fn admission_error_code(error: &AdmissionError) -> &'static str {
    match error {
        AdmissionError::Backpressure => "backpressure",
        AdmissionError::OperationIdCollision => "operation_id_collision",
        AdmissionError::StaleGeneration => "stale_generation",
        AdmissionError::Group(error) => group_error_code(error),
        AdmissionError::LeaseRequired => "lease_required",
        AdmissionError::ActorStopped => "actor_stopped",
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_actor(
    mut receiver: mpsc::UnboundedReceiver<Command>,
    sender: mpsc::UnboundedSender<Command>,
    executor: Arc<dyn Executor>,
    generation: String,
    limits: QueueLimits,
    groups: GroupRegistry,
    events: EventRecorder,
    persistence: Option<JournalWriter>,
) {
    let mut state = State::new(generation, limits, groups, events, persistence);
    while let Some(command) = receiver.recv().await {
        match command {
            Command::Submit(request, reply) => {
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
            Command::Cancel(operation_id, reply) => {
                let result = state.cancel(&operation_id);
                if result == CancelResult::UnknownOperation {
                    state.record_lifecycle(None, "cancel_failed:unknown_operation");
                }
                let _ = reply.send(result);
            }
            Command::Executed(operation_id, outcome) => {
                state.complete(&operation_id, outcome);
            }
            Command::Settle(operation_id, outcome, reply) => {
                let result = state.settle(&operation_id, outcome);
                let event_state = match &result {
                    SettlementResult::Settled(_) => "settled",
                    SettlementResult::RemainsAmbiguous => "remains_ambiguous",
                    SettlementResult::Ignored => "ignored",
                };
                state.record_settlement_event(&operation_id, event_state);
                let _ = reply.send(result);
            }
            Command::SettlementState(operation_id, reply) => {
                let _ = reply.send(state.settlement_state(&operation_id));
            }
            Command::CreateGroup {
                group_id,
                browser,
                principal,
                now_ms,
                reply,
            } => {
                state.advance_time(now_ms);
                let group = state.groups.create(group_id, browser, principal, now_ms);
                state.record_lifecycle(Some(&group.group_id), "group_created");
                let _ = reply.send(group);
            }
            Command::AddMember {
                group_id,
                principal,
                tab,
                reply,
            } => {
                let result = state.groups.add_member(&group_id, &principal, tab);
                state.record_lifecycle(
                    Some(&group_id),
                    lifecycle_result("member_added", "member_add_failed", &result),
                );
                let _ = reply.send(result);
            }
            Command::Group(group_id, reply) => {
                let result = state.groups.get(&group_id).cloned();
                let _ = reply.send(result);
            }
            Command::Groups(reply) => {
                let _ = reply.send(state.groups.all().cloned().collect());
            }
            Command::BrowserLost(browser, reply) => {
                let affected = state.browser_lost(&browser);
                for group_id in &affected {
                    state.record_lifecycle(Some(group_id), "browser_lost");
                }
                let _ = reply.send(affected);
            }
            Command::Renew(proof, principal, now_ms, reply) => {
                state.advance_time(now_ms);
                let result = state.groups.renew(&proof, &principal, now_ms);
                state.record_lifecycle(
                    Some(&proof.group_id),
                    lifecycle_result("lease_renewed", "lease_renew_failed", &result),
                );
                let _ = reply.send(result);
            }
            Command::Offer {
                group_id,
                principal,
                target,
                revision,
                reply,
            } => {
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
            Command::Accept {
                group_id,
                target,
                revision,
                now_ms,
                reply,
            } => {
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
            Command::Force {
                group_id,
                requester,
                target,
                revision,
                now_ms,
                reply,
            } => {
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
            Command::Disconnect(principal, now_ms, reply) => {
                state.advance_time(now_ms);
                state.cancel_queued_for_principal(&principal);
                state.detach_in_flight_for_principal(&principal);
                state.groups.mark_disconnected(&principal, now_ms);
                state.record_lifecycle(None, "principal_disconnected");
                let _ = reply.send(());
            }
            Command::EndGroup(group_id, principal, reply) => {
                state.cancel_queued_for_group(&group_id);
                let in_flight = state.group_in_flight(&group_id);
                let result = state.groups.end_lifecycle(&group_id, &principal, in_flight);
                state.record_lifecycle(
                    Some(&group_id),
                    lifecycle_result("group_ended", "group_end_failed", &result),
                );
                let _ = reply.send(result);
            }
            Command::Tick(now_ms, reply) => {
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
            Command::ResumeRecovered {
                group_id,
                browser,
                principal,
                members,
                revision,
                now_ms,
                reply,
            } => {
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
            Command::Snapshot(reply) => {
                let _ = reply.send(state.snapshot());
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
