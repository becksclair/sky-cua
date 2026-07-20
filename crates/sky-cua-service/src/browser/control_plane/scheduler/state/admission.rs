use std::collections::VecDeque;

use super::{OperationRecord, OperationState, State};
use crate::browser::control_plane::{
    control::{AdmissionError, CancelResult, Reply, SubmitOperation},
    operation::{
        ClientId, Completion, DispatchOperation, OperationId, OperationIdentity, OperationScope,
    },
};

impl State {
    pub(super) fn admit(
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
        let cancellation_intent = self.cancellation_intents.remove(&operation_id);
        if cancellation_intent.is_some() {
            self.cancellation_intent_order
                .retain(|pending| pending != &operation_id);
        }
        if cancellation_intent.is_some_and(|expected| {
            expected.is_none() || expected.as_ref() == Some(&request.client_id)
        }) {
            let completion = Completion::cancelled(operation_id.clone());
            let _ = waiter.send(Ok(completion.clone()));
            let identity = OperationIdentity {
                operation_id: operation_id.clone(),
                daemon_generation: self.generation.clone(),
                canonical_fingerprint: request.canonical_fingerprint,
                upstream: request.upstream,
            };
            self.operations.insert(
                operation_id.clone(),
                OperationRecord {
                    dispatch: DispatchOperation {
                        identity,
                        client_id: request.client_id,
                        principal: request.principal,
                        group_id: request.group_id,
                        scope: request.scope,
                        class: request.class,
                        payload: request.payload,
                    },
                    lease: request.lease,
                    state: OperationState::Terminal,
                    waiters: Vec::new(),
                    completion: Some(completion),
                    admitted_at_ms: request.now_ms,
                },
            );
            self.remember(operation_id.clone());
            self.record_operation_event(&operation_id, "cancelled_before_registration");
            return Ok(());
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

    pub(super) fn cancel(
        &mut self,
        operation_id: &OperationId,
        client_id: Option<&ClientId>,
    ) -> CancelResult {
        let Some(record) = self.operations.get_mut(operation_id) else {
            self.remember_cancellation_intent(operation_id.clone(), client_id.cloned());
            return CancelResult::UnknownOperation;
        };
        if client_id.is_some_and(|client| client != &record.dispatch.client_id) {
            return CancelResult::UnknownOperation;
        }
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

    fn remember_cancellation_intent(
        &mut self,
        operation_id: OperationId,
        client_id: Option<ClientId>,
    ) {
        if let Some(existing) = self.cancellation_intents.get_mut(&operation_id) {
            if client_id.is_none() {
                *existing = None;
            }
            return;
        }
        self.cancellation_intents
            .insert(operation_id.clone(), client_id);
        self.cancellation_intent_order.push_back(operation_id);
        let limit = self.limits.recent_operations.max(1);
        while self.cancellation_intent_order.len() > limit {
            if let Some(expired) = self.cancellation_intent_order.pop_front() {
                self.cancellation_intents.remove(&expired);
            }
        }
    }

    pub(super) fn remove_from_queues(&mut self, operation_id: &OperationId) {
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

    pub(super) fn remember(&mut self, operation_id: OperationId) {
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
}
