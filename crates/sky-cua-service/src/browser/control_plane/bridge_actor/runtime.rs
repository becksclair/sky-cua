use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::wire::requires_settlement;
use super::{
    BridgeActorConfig, BridgeActorError, BridgeActorRequest, BridgeRequestSize, OperationClass,
};
use crate::browser::protocol::CONTROL_PLANE_REQUEST_ID_PREFIX;

const ORDINARY_WIDTH: usize = 2;
const TOMBSTONE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_TOMBSTONES: usize = 2_048;

pub(super) struct QueuedRequest {
    pub(super) request: BridgeActorRequest,
    pub(super) reply: oneshot::Sender<Result<Value, BridgeActorError>>,
}

pub(super) struct PendingRequest {
    pub(super) reply: oneshot::Sender<Result<Value, BridgeActorError>>,
    pub(super) operation_class: OperationClass,
    pub(super) operation_id: String,
    pub(super) deadline: Instant,
    pub(super) size: BridgeRequestSize,
}

pub(super) struct Tombstone {
    pub(super) created_at: Instant,
    pub(super) operation_id: Option<String>,
    pub(super) operation_class: Option<OperationClass>,
}

pub(super) struct Runtime {
    next_request_id: u64,
    pub(super) actor_generation: u64,
    pub(super) pending: HashMap<String, PendingRequest>,
    pub(super) queued: VecDeque<QueuedRequest>,
    pub(super) tombstones: HashMap<String, Tombstone>,
    tombstone_order: VecDeque<String>,
    pub(super) heartbeat: Option<(String, Instant)>,
}

impl Runtime {
    pub(super) fn new(actor_generation: u64) -> Self {
        Self {
            next_request_id: 0,
            actor_generation,
            pending: HashMap::new(),
            queued: VecDeque::new(),
            tombstones: HashMap::new(),
            tombstone_order: VecDeque::new(),
            heartbeat: None,
        }
    }

    pub(super) fn allocate_request_id(&mut self, config: &BridgeActorConfig) -> String {
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .expect("bridge request id space exhausted");
        format!(
            "{CONTROL_PLANE_REQUEST_ID_PREFIX}{}-{:020}-{:020}",
            config.daemon_generation, self.actor_generation, self.next_request_id
        )
    }

    pub(super) fn advance_actor_generation(&mut self) {
        self.actor_generation = self
            .actor_generation
            .checked_add(1)
            .expect("bridge actor generation exhausted");
    }

    pub(super) fn can_dispatch(&self, size: BridgeRequestSize) -> bool {
        let ordinary = self
            .pending
            .values()
            .filter(|pending| pending.size == BridgeRequestSize::Ordinary)
            .count();
        let large = self
            .pending
            .values()
            .any(|pending| pending.size == BridgeRequestSize::LargeFrame);
        match size {
            BridgeRequestSize::Ordinary => !large && ordinary < ORDINARY_WIDTH,
            BridgeRequestSize::LargeFrame => !large && ordinary == 0,
        }
    }

    pub(super) fn tombstone(&mut self, request_id: String, now: Instant) {
        self.tombstones.insert(
            request_id.clone(),
            Tombstone {
                created_at: now,
                operation_id: None,
                operation_class: None,
            },
        );
        self.tombstone_order.push_back(request_id);
        self.prune_tombstones(now);
    }

    pub(super) fn tombstone_pending(
        &mut self,
        request_id: String,
        now: Instant,
        operation_id: String,
        operation_class: OperationClass,
    ) {
        self.tombstones.insert(
            request_id.clone(),
            Tombstone {
                created_at: now,
                operation_id: Some(operation_id),
                operation_class: Some(operation_class),
            },
        );
        self.tombstone_order.push_back(request_id);
        self.prune_tombstones(now);
    }

    pub(super) fn prune_tombstones(&mut self, now: Instant) {
        while let Some(oldest) = self.tombstone_order.front() {
            let expired = self
                .tombstones
                .get(oldest)
                .is_none_or(|tombstone| now.duration_since(tombstone.created_at) >= TOMBSTONE_TTL);
            if !expired && self.tombstones.len() <= MAX_TOMBSTONES {
                break;
            }
            let oldest = self.tombstone_order.pop_front().expect("front exists");
            self.tombstones.remove(&oldest);
        }
    }

    pub(super) fn has_unresolved_settlements(&self) -> bool {
        self.tombstones
            .values()
            .any(|tombstone| tombstone.operation_class.is_some_and(requires_settlement))
    }

    pub(super) fn resolve_settlement(&mut self, operation_id: &str) {
        self.tombstones.retain(|_, tombstone| {
            tombstone.operation_id.as_deref() != Some(operation_id)
                || !tombstone.operation_class.is_some_and(requires_settlement)
        });
        self.tombstone_order
            .retain(|request_id| self.tombstones.contains_key(request_id));
    }

    pub(super) fn fail_dispatched(&mut self) {
        let now = Instant::now();
        for (request_id, pending) in self.pending.drain() {
            let result = if requires_settlement(pending.operation_class) {
                Err(BridgeActorError::Ambiguous)
            } else {
                Err(BridgeActorError::Disconnected)
            };
            self.tombstones.insert(
                request_id.clone(),
                Tombstone {
                    created_at: now,
                    operation_id: Some(pending.operation_id.clone()),
                    operation_class: Some(pending.operation_class),
                },
            );
            let _ = pending.reply.send(result);
            self.tombstone_order.push_back(request_id);
        }
        self.heartbeat = None;
        self.prune_tombstones(now);
    }

    pub(super) fn fail_queued(&mut self, error: BridgeActorError) {
        for queued in self.queued.drain(..) {
            let _ = queued.reply.send(Err(error.clone()));
        }
    }
}
