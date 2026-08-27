use std::cmp::Ordering;
use std::sync::TryLockError;

use super::*;
use crate::frame::write_frame_until;

pub(super) const SKY_CUA_HOST_SETTLEMENT_METHOD: &str = "skyCuaHost/settlement";
pub(super) const SKY_CUA_HOST_SETTLEMENT_ACK_METHOD: &str = "skyCuaHost/settlementAck";
const MAX_RETAINED_SETTLEMENTS: usize = 100;
const MAX_SETTLEMENT_EVICTIONS_PER_TICK: usize = 32;
const TOMBSTONE_TTL: Duration = Duration::from_secs(10 * 60);
const SETTLEMENT_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);

pub(super) use super::timing::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingRequestState {
    Active,
    OrphanedPending,
}

#[derive(Debug, Clone)]
pub(super) struct SettlementMetadata {
    pub(super) operation_id: String,
    pub(super) daemon_generation: String,
    pub(super) actor_generation: Value,
    pub(super) target_lifetime_key: Option<Value>,
    pub(super) operation_class: OperationClass,
    pub(super) settlement_deadline_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationClass {
    ReadOnly,
    AbsoluteSet,
    Mutation,
    BrowserGlobal,
}

impl OperationClass {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "read_only" => Some(Self::ReadOnly),
            "absolute_set" => Some(Self::AbsoluteSet),
            "mutation" => Some(Self::Mutation),
            "browser_global" => Some(Self::BrowserGlobal),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::AbsoluteSet => "absolute_set",
            Self::Mutation => "mutation",
            Self::BrowserGlobal => "browser_global",
        }
    }

    pub(super) fn requires_settlement(self) -> bool {
        matches!(self, Self::Mutation | Self::BrowserGlobal)
    }
}

/// A settlement retained in the host queue. Identity is parsed once at
/// admission (from the structured metadata plus the wire chrome request id),
/// and absolute phase deadlines are captured at enqueue time so an entry
/// blocked behind the front ages toward its own recovery deadlines rather
/// than a fixed per-entry timeout applied serially.
pub(super) struct QueuedSettlement {
    metadata: SettlementMetadata,
    message: Value,
    entered_at: Instant,
    phase: SettlementPhase,
    /// Incremented on every Original -> Unknown conversion so an in-flight
    /// delivery lease taken before the conversion cannot mark the converted
    /// entry as delivered.
    phase_revision: u64,
}

#[derive(Debug, Clone, Copy)]
enum SettlementPhase {
    Original {
        convert_at: Instant,
        hard_evict_at: Instant,
    },
    Unknown {
        first_delivered_at: Option<Instant>,
        hard_evict_at: Instant,
    },
}

/// Identity of a retained settlement, parsed once at admission from the
/// structured metadata plus the wire chrome request id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettlementIdentity {
    operation_id: String,
    daemon_generation: String,
    actor_generation: Value,
    chrome_request_id: String,
}

/// A claim on the front settlement during an out-of-state-lock delivery.
/// Carries the identity and phase revision observed at `begin`, so a stale
/// completion (concurrent conversion or ack-pop) is inert.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeliveryLease {
    client_id: usize,
    identity: SettlementIdentity,
    phase_revision: u64,
    armed: bool,
    delivery_seq: u64,
}

/// RAII safety net that clears only the transient
/// `settlement_delivery_in_progress` flag. Never touches phase timestamps.
/// Guards against clearing a newer delivery's flag via `delivery_seq`.
struct DeliveryGuard {
    state: Arc<Mutex<HostState>>,
    delivery_seq: u64,
}

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.delivery_seq == self.delivery_seq {
            state.settlement_delivery_in_progress = false;
        }
    }
}

impl QueuedSettlement {
    /// Compare an acknowledgement's identity fields against this entry's
    /// stored metadata and wire chrome request id.
    fn identity_matches(&self, ack: &Value) -> bool {
        wire_metadata_matches(&self.metadata, ack)
            && ack.pointer("/params/chrome_request_id")
                == self.message.pointer("/params/chrome_request_id")
    }

    fn identity(&self) -> SettlementIdentity {
        SettlementIdentity {
            operation_id: self.metadata.operation_id.clone(),
            daemon_generation: self.metadata.daemon_generation.clone(),
            actor_generation: self.metadata.actor_generation.clone(),
            chrome_request_id: self
                .message
                .pointer("/params/chrome_request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }
    }

    /// The eviction reason for the front entry at `now`, or `None` while the
    /// entry still has budget. For `Unknown`, the hard cap truncates a late
    /// post-delivery grace: `effective = min(first_delivered_at + 4s,
    /// hard_evict_at)`.
    fn effective_eviction_reason(&self, now: Instant) -> Option<&'static str> {
        match &self.phase {
            SettlementPhase::Original { hard_evict_at, .. } => {
                (now >= *hard_evict_at).then_some("unknown_absolute_cap_exceeded")
            }
            SettlementPhase::Unknown {
                first_delivered_at,
                hard_evict_at,
            } => match first_delivered_at {
                Some(delivered) => {
                    let grace_expired = delivered
                        .checked_add(SETTLEMENT_UNKNOWN_MAX_POST_DELIVERY_GRACE)
                        .unwrap_or(*hard_evict_at);
                    if grace_expired < *hard_evict_at && now >= grace_expired {
                        Some("unknown_ack_grace_exceeded")
                    } else if now >= *hard_evict_at {
                        Some("unknown_absolute_cap_exceeded")
                    } else {
                        None
                    }
                }
                None if now >= *hard_evict_at => Some("unknown_absolute_cap_exceeded"),
                None => None,
            },
        }
    }
}

/// Convert an Original settlement to Unknown in place, preserving identity.
/// The rebuilt wire message keeps the same chrome request id and original
/// request id; the hard eviction deadline is retained.
fn convert_original_to_unknown(
    entry: &mut QueuedSettlement,
    active_generation: Option<&str>,
    queue_len: usize,
    phase_age_ms: u128,
) {
    let SettlementPhase::Original { hard_evict_at, .. } = entry.phase else {
        return;
    };
    let chrome_request_id = entry
        .message
        .pointer("/params/chrome_request_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let original_request_id = entry
        .message
        .pointer("/params/original_request_id")
        .cloned()
        .unwrap_or(Value::Null);
    entry.message = settlement_message(
        "settlement_unknown",
        &chrome_request_id,
        &original_request_id,
        &entry.metadata,
        None,
    );
    debug_assert!(wire_metadata_matches(&entry.metadata, &entry.message));
    entry.phase = SettlementPhase::Unknown {
        first_delivered_at: None,
        hard_evict_at,
    };
    entry.phase_revision += 1;
    diagnostics::settlement_delivery_converted_to_unknown(
        &entry.metadata.operation_id,
        &chrome_request_id,
        &entry.metadata.daemon_generation,
        active_generation,
        queue_len,
        phase_age_ms,
    );
}

fn wire_metadata_matches(metadata: &SettlementMetadata, message: &Value) -> bool {
    message
        .pointer("/params/operation_id")
        .and_then(Value::as_str)
        == Some(metadata.operation_id.as_str())
        && message
            .pointer("/params/daemon_generation")
            .and_then(Value::as_str)
            == Some(metadata.daemon_generation.as_str())
        && message.pointer("/params/actor_generation") == Some(&metadata.actor_generation)
}

impl HostState {
    pub(super) fn cleanup_pending_for_removed_client(
        &mut self,
        client_id: usize,
        role: ClientRole,
    ) {
        if self.settlement_delivered_to == Some(client_id) {
            self.settlement_delivered_to = None;
            self.settlement_delivered_at = None;
        }
        let removed_ids = self
            .pending_chrome_requests
            .iter_mut()
            .filter_map(|(id, pending)| {
                if pending.client_id != client_id {
                    return None;
                }
                if role == ClientRole::ControlPlane
                    && pending
                        .settlement
                        .as_ref()
                        .is_some_and(|metadata| metadata.operation_class.requires_settlement())
                {
                    pending.state = PendingRequestState::OrphanedPending;
                    None
                } else {
                    Some(id.clone())
                }
            })
            .collect::<Vec<_>>();
        for id in removed_ids {
            self.pending_chrome_requests.remove(&id);
            self.tombstone_pending_id(id);
        }
        self.pending_client_requests
            .retain(|_, pending| pending.client_id != client_id);
    }

    pub(super) fn cleanup_old_requests(&mut self) {
        let now = Instant::now();
        self.evict_settlements_at(now);
        self.fence_unresponsive_control_plane_at(now);
        let now_epoch_ms = unix_epoch_ms();
        self.pending_id_tombstones
            .retain(|_, tombstoned_at| now.duration_since(*tombstoned_at) < TOMBSTONE_TTL);

        let expired = self
            .pending_chrome_requests
            .iter()
            .filter_map(|(id, request)| {
                let expired = request.settlement.as_ref().is_some_and(|metadata| {
                    metadata.operation_class.requires_settlement()
                        && metadata.settlement_deadline_ms <= now_epoch_ms
                }) || request
                    .settlement
                    .as_ref()
                    .is_none_or(|metadata| !metadata.operation_class.requires_settlement())
                    && now.duration_since(request.created_at) >= REQUEST_TIMEOUT;
                expired.then_some(id.clone())
            })
            .collect::<Vec<_>>();
        for id in expired {
            if let Some(request) = self.pending_chrome_requests.remove(&id) {
                self.tombstone_pending_id(id.clone());
                if let Some(metadata) = request
                    .settlement
                    .filter(|metadata| metadata.operation_class.requires_settlement())
                    && let Err(reason) = self.queue_settlement(
                        metadata.clone(),
                        settlement_message(
                            "settlement_unknown",
                            &id,
                            &request.client_request_id,
                            &metadata,
                            None,
                        ),
                    )
                {
                    diagnostics::settlement_metadata_rejected(reason);
                }
            }
        }
        self.pending_client_requests
            .retain(|_, req| now.duration_since(req.created_at) < REQUEST_TIMEOUT);
        while self.pending_chrome_requests.len() > MAX_PENDING_REQUESTS {
            if let Some(oldest) = self
                .pending_chrome_requests
                .iter()
                .filter(|(_, request)| {
                    request
                        .settlement
                        .as_ref()
                        .is_none_or(|metadata| !metadata.operation_class.requires_settlement())
                })
                .min_by_key(|(_, req)| req.created_at)
                .map(|(id, _)| id.clone())
            {
                self.pending_chrome_requests.remove(&oldest);
                self.tombstone_pending_id(oldest);
            } else {
                break;
            }
        }
        while self.pending_client_requests.len() > MAX_PENDING_REQUESTS {
            if let Some(oldest) = self
                .pending_client_requests
                .iter()
                .min_by_key(|(_, req)| req.created_at)
                .map(|(id, _)| id.clone())
            {
                self.pending_client_requests.remove(&oldest);
            }
        }
    }

    /// Convert aged Originals to Unknown and drain the front by its absolute
    /// deadlines. Runs first in the maintenance pass and skips entirely while a
    /// delivery is in progress so the front is never popped mid-write. All
    /// timestamps use the injected `now` captured once per pass.
    fn evict_settlements_at(&mut self, now: Instant) {
        if self.settlement_delivery_in_progress {
            return;
        }
        let active_generation = self.clients.iter().find_map(|(_, client)| {
            (client.role == ClientRole::ControlPlane
                && client.capabilities.contains(SETTLEMENT_ACK_CAPABILITY))
            .then(|| client.daemon_generation.clone().unwrap_or_default())
        });
        let queue_len = self.queued_settlements.len();

        // Convert every aged Original anywhere in the queue: entries blocked
        // behind the front age by their own absolute `convert_at`, so the whole
        // queue converts rather than waiting one serial front timeout per slot.
        for entry in self.queued_settlements.iter_mut() {
            let SettlementPhase::Original { convert_at, .. } = entry.phase else {
                continue;
            };
            if now < convert_at {
                continue;
            }
            let phase_age_ms = now.saturating_duration_since(entry.entered_at).as_millis();
            convert_original_to_unknown(
                entry,
                active_generation.as_deref(),
                queue_len,
                phase_age_ms,
            );
        }

        // Drain the front by its effective deadline (hard cap truncates a late
        // post-delivery grace). Bounded removals per tick to keep the pass cheap.
        let mut removals = 0;
        while removals < MAX_SETTLEMENT_EVICTIONS_PER_TICK {
            let Some(reason) = self
                .queued_settlements
                .front()
                .and_then(|front| front.effective_eviction_reason(now))
            else {
                break;
            };
            self.pop_front_settlement(reason);
            removals += 1;
        }
    }

    fn pop_front_settlement(&mut self, reason: &'static str) {
        let (operation_id, chrome_request_id, generation) = self
            .queued_settlements
            .front()
            .map(|front| {
                (
                    front.metadata.operation_id.clone(),
                    front
                        .message
                        .pointer("/params/chrome_request_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    front.metadata.daemon_generation.clone(),
                )
            })
            .expect("pop_front_settlement requires a non-empty queue");
        self.queued_settlements.pop_front();
        self.settlement_delivered_to = None;
        self.settlement_delivered_at = None;
        diagnostics::settlement_delivery_evicted(
            &operation_id,
            &chrome_request_id,
            &generation,
            self.queued_settlements.len(),
            reason,
        );
    }

    /// Convert every queued Original from a strictly-older daemon generation to
    /// Unknown immediately when a newer generation is promoted, so prior-
    /// generation queues drain without a per-entry 15s conversion delay.
    pub(super) fn supersede_prior_generation_settlements(&mut self, new_generation: &str) {
        let now = Instant::now();
        let queue_len = self.queued_settlements.len();
        for entry in self.queued_settlements.iter_mut() {
            if entry.metadata.daemon_generation == new_generation {
                continue;
            }
            if compare_daemon_generations(new_generation, &entry.metadata.daemon_generation)
                != Ordering::Greater
            {
                continue;
            }
            if !matches!(entry.phase, SettlementPhase::Original { .. }) {
                continue;
            }
            let phase_age_ms = now.saturating_duration_since(entry.entered_at).as_millis();
            convert_original_to_unknown(entry, Some(new_generation), queue_len, phase_age_ms);
        }
    }

    /// Fence a control-plane client that has stopped sending messages while
    /// the settlement queue is still non-empty. Guards: not mid-write, queue
    /// non-empty, an acknowledged control plane is registered, the client has
    /// not already been requested to close, and the silent period exceeds the
    /// liveness deadline.
    fn fence_unresponsive_control_plane_at(&mut self, now: Instant) {
        if self.settlement_delivery_in_progress {
            return;
        }
        if self.queued_settlements.is_empty() {
            return;
        }
        let Some((client_id, last_seen_at)) = self.clients.iter().find_map(|(id, client)| {
            (client.role == ClientRole::ControlPlane
                && client.capabilities.contains(SETTLEMENT_ACK_CAPABILITY))
            .then_some((*id, client.last_seen_at))
        }) else {
            return;
        };
        if self
            .clients
            .get(&client_id)
            .is_some_and(|client| client.close_requested)
        {
            return;
        }
        if now.saturating_duration_since(last_seen_at) < CONTROL_PLANE_LIVENESS_DEADLINE {
            return;
        }
        let last_seen_age_ms = now.saturating_duration_since(last_seen_at).as_millis();
        diagnostics::control_plane_fenced_unresponsive(
            client_id,
            last_seen_age_ms,
            self.queued_settlements.len(),
        );
        self.request_client_close(client_id, "liveness_deadline_exceeded");
    }

    pub(super) fn tombstone_pending_id(&mut self, id: String) {
        self.pending_id_tombstones.insert(id, Instant::now());
    }

    pub(super) fn queue_settlement(
        &mut self,
        metadata: SettlementMetadata,
        message: Value,
    ) -> std::result::Result<(), &'static str> {
        // Admission reserves this space before mutating work is dispatched.
        // Reaching the bound is reported to the producer rather than silently
        // discarding an ambiguous settlement or panicking.
        if self.queued_settlements.len() >= MAX_RETAINED_SETTLEMENTS {
            return Err("settlement retention admission capacity exceeded");
        }
        // Identity is parsed once at admission: the structured metadata must
        // match the identity fields of the wire message. Reject without
        // consuming queue capacity.
        if !wire_metadata_matches(&metadata, &message) {
            return Err("settlement wire message identity does not match its structured metadata");
        }
        let entered_at = Instant::now();
        let phase = if message.pointer("/params/status").and_then(Value::as_str)
            == Some("settlement_unknown")
        {
            SettlementPhase::Unknown {
                first_delivered_at: None,
                hard_evict_at: entered_at + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP,
            }
        } else {
            SettlementPhase::Original {
                convert_at: entered_at + SETTLEMENT_ORIGINAL_TO_UNKNOWN,
                hard_evict_at: entered_at + SETTLEMENT_ENQUEUE_HARD_EVICT,
            }
        };
        self.queued_settlements.push_back(QueuedSettlement {
            metadata,
            message,
            entered_at,
            phase,
            phase_revision: 0,
        });
        Ok(())
    }

    pub(super) fn settlement_capacity_available(&self) -> bool {
        let retained_pending = self
            .pending_chrome_requests
            .values()
            .filter(|request| {
                request
                    .settlement
                    .as_ref()
                    .is_some_and(|metadata| metadata.operation_class.requires_settlement())
            })
            .count();
        retained_pending + self.queued_settlements.len() < MAX_RETAINED_SETTLEMENTS
    }

    pub(super) fn allocate_chrome_id(&mut self) -> String {
        loop {
            let id = format!("linux-{}-{}", process::id(), self.next_chrome_id);
            self.next_chrome_id = self.next_chrome_id.wrapping_add(1);
            if !self.pending_chrome_requests.contains_key(&id)
                && !self.pending_id_tombstones.contains_key(&id)
            {
                return id;
            }
        }
    }

    fn active_control_plane(&self) -> Option<(usize, SharedClientWriter)> {
        self.clients.iter().find_map(|(client_id, client)| {
            (client.role == ClientRole::ControlPlane
                && client.capabilities.contains(SETTLEMENT_ACK_CAPABILITY))
            .then(|| (*client_id, Arc::clone(&client.writer)))
        })
    }

    fn begin_settlement_delivery(&mut self) -> Option<(DeliveryLease, SharedClientWriter, Value)> {
        if self.settlement_delivery_in_progress {
            return None;
        }
        let (client_id, writer) = self.active_control_plane()?;
        if self.settlement_delivered_to == Some(client_id)
            && self
                .settlement_delivered_at
                .is_some_and(|delivered_at| delivered_at.elapsed() < SETTLEMENT_ACK_RETRY_INTERVAL)
        {
            return None;
        }
        let front = self.queued_settlements.front()?;
        let message = front.message.clone();
        let identity = front.identity();
        let phase_revision = front.phase_revision;
        self.delivery_seq += 1;
        self.settlement_delivery_in_progress = true;
        Some((
            DeliveryLease {
                client_id,
                identity,
                phase_revision,
                armed: true,
                delivery_seq: self.delivery_seq,
            },
            writer,
            message,
        ))
    }

    fn finish_delivery(&mut self, lease: DeliveryLease, delivered: bool) {
        assert!(
            self.settlement_delivery_in_progress,
            "settlement delivery completed without an active claim"
        );
        if delivered {
            // Throttle state and phase timestamps change only when the full
            // frame was delivered AND the front still matches this lease. A
            // stale completion (concurrent conversion or ack-pop) cannot
            // re-arm the throttle or mark the next front as delivered.
            if let Some(front) = self.queued_settlements.front_mut()
                && front.phase_revision == lease.phase_revision
                && front.identity() == lease.identity
            {
                self.settlement_delivered_to = Some(lease.client_id);
                self.settlement_delivered_at = Some(Instant::now());
                if let SettlementPhase::Unknown {
                    first_delivered_at, ..
                } = &mut front.phase
                {
                    *first_delivered_at = Some(Instant::now());
                }
            }
        } else {
            self.settlement_delivered_to = None;
            self.settlement_delivered_at = None;
        }
        self.settlement_delivery_in_progress = false;
    }

    /// Atomically request a client close. Returns whether this caller
    /// initiated the request; repeated callers (liveness fence, write-error
    /// path) are suppressed. Breaks a stuck blocked writer via the shared
    /// shutdown handle, independent of the writer mutex.
    pub(super) fn request_client_close(&mut self, client_id: usize, reason: &'static str) -> bool {
        let Some(client) = self.clients.get_mut(&client_id) else {
            return false;
        };
        if client.close_requested {
            return false;
        }
        client.close_requested = true;
        trace(
            &self.host_name,
            &format!("requesting client {client_id} close: {reason}"),
        );
        if let Some(handle) = &client.shutdown_handle {
            let _ = handle.shutdown(Shutdown::Both);
        } else if let Ok(writer) = client.writer.lock() {
            let _ = writer.shutdown(Shutdown::Both);
        }
        true
    }

    pub(super) fn acknowledge_settlement(&mut self, client_id: usize, message: &Value) -> bool {
        let Some(client) = self.clients.get(&client_id) else {
            return false;
        };
        if client.role != ClientRole::ControlPlane
            || !client.capabilities.contains(SETTLEMENT_ACK_CAPABILITY)
            || self.settlement_delivered_to != Some(client_id)
            || message
                .pointer("/params/acknowledging_daemon_generation")
                .and_then(Value::as_str)
                != client.daemon_generation.as_deref()
        {
            return false;
        }
        let Some(entry) = self.queued_settlements.front() else {
            return false;
        };
        if !entry.identity_matches(message) {
            return false;
        }
        self.queued_settlements.pop_front();
        self.settlement_delivered_to = None;
        self.settlement_delivered_at = None;
        true
    }
}

pub(super) fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(super) fn settlement_message(
    status: &str,
    chrome_request_id: &str,
    original_request_id: &Value,
    metadata: &SettlementMetadata,
    completion: Option<Value>,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": SKY_CUA_HOST_SETTLEMENT_METHOD,
        "params": {
            "status": status,
            "operation_id": metadata.operation_id,
            "daemon_generation": metadata.daemon_generation,
            "actor_generation": metadata.actor_generation,
            "target_lifetime_key": metadata.target_lifetime_key,
            "operation_class": metadata.operation_class.name(),
            "settlement_deadline_ms": metadata.settlement_deadline_ms,
            "original_request_id": original_request_id,
            "chrome_request_id": chrome_request_id,
            "completion": completion,
        }
    })
}

pub(super) fn settlement_metadata(
    message: &Value,
) -> std::result::Result<SettlementMetadata, &'static str> {
    let params = message
        .get("params")
        .and_then(Value::as_object)
        .ok_or("control-plane extension requests require object params")?;
    let private = params
        .get(SKY_CUA_HOST_REQUEST_PARAM)
        .and_then(Value::as_object);
    let field = |canonical: &str, flat: &str| {
        private
            .and_then(|metadata| metadata.get(canonical))
            .or_else(|| params.get(flat))
    };
    let operation_id = field("operation_id", SKY_CUA_OPERATION_ID_PARAM)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("control-plane request requires a non-empty operation_id")?
        .to_string();
    let daemon_generation = field("daemon_generation", SKY_CUA_DAEMON_GENERATION_PARAM)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("control-plane request requires a non-empty daemon_generation")?
        .to_string();
    let actor_generation = field("actor_generation", SKY_CUA_ACTOR_GENERATION_PARAM)
        .filter(|value| value.is_string() || value.is_number())
        .cloned()
        .ok_or("control-plane request requires a string or number actor_generation")?;
    let operation_class = field("operation_class", SKY_CUA_OPERATION_CLASS_PARAM)
        .and_then(Value::as_str)
        .and_then(OperationClass::parse)
        .ok_or("control-plane request has an invalid operation_class")?;
    let settlement_deadline_ms = field(
        "settlement_deadline_ms",
        SKY_CUA_SETTLEMENT_DEADLINE_MS_PARAM,
    )
    .and_then(Value::as_u64)
    .ok_or("control-plane request requires a u64 settlement_deadline_ms")?;
    if operation_class.requires_settlement() && settlement_deadline_ms <= unix_epoch_ms() {
        return Err("control-plane mutating request settlement deadline has expired");
    }
    let target_lifetime_key = field("target_lifetime_key", SKY_CUA_TARGET_LIFETIME_KEY_PARAM)
        .filter(|value| !value.is_null())
        .cloned();
    Ok(SettlementMetadata {
        operation_id,
        daemon_generation,
        actor_generation,
        target_lifetime_key,
        operation_class,
        settlement_deadline_ms,
    })
}

pub(super) fn settlement_maintenance_loop(state: SharedState) {
    loop {
        thread::sleep(SETTLEMENT_MAINTENANCE_INTERVAL);
        state
            .lock()
            .expect("host state mutex poisoned")
            .cleanup_old_requests();
        deliver_queued_settlements(&state);
    }
}

pub(super) fn deliver_queued_settlements(state: &SharedState) {
    let delivery = {
        let mut state = state.lock().expect("host state mutex poisoned");
        state
            .begin_settlement_delivery()
            .map(|(lease, writer, message)| (lease, writer, state.host_name.clone(), message))
    };
    let Some((lease, writer, host_name, message)) = delivery else {
        return;
    };
    let _guard = DeliveryGuard {
        state: Arc::clone(state),
        delivery_seq: lease.delivery_seq,
    };
    let delivered = deliver_settlement_frame(state, &lease, &writer, &host_name, &message);
    state
        .lock()
        .expect("host state mutex poisoned")
        .finish_delivery(lease, delivered);
}

fn deliver_settlement_frame(
    state: &SharedState,
    lease: &DeliveryLease,
    writer: &SharedClientWriter,
    host_name: &str,
    message: &Value,
) -> bool {
    let mut writer = match writer.try_lock() {
        Ok(writer) => writer,
        Err(TryLockError::WouldBlock) => {
            trace(
                host_name,
                "settlement delivery deferred: client writer contended",
            );
            return false;
        }
        Err(TryLockError::Poisoned(_)) => {
            diagnostics::control_plane_socket_closed_delivery_failure(
                lease.client_id,
                "writer_mutex_poisoned",
                "try_lock",
            );
            let initiated = {
                let mut state = state.lock().expect("host state mutex poisoned");
                state.request_client_close(lease.client_id, "writer_mutex_poisoned")
            };
            if initiated {
                trace(
                    host_name,
                    "settlement delivery aborted: client writer mutex poisoned",
                );
            }
            return false;
        }
    };
    let deadline = Instant::now() + CONTROL_PLANE_FRAME_WRITE_DEADLINE;
    match write_frame_until(&mut writer, message, deadline) {
        Ok(()) => true,
        Err(error) => {
            diagnostics::control_plane_socket_closed_delivery_failure(
                lease.client_id,
                &format!("{:?}", error.kind()),
                "write",
            );
            let initiated = {
                let mut state = state.lock().expect("host state mutex poisoned");
                state.request_client_close(lease.client_id, "write_failure")
            };
            if initiated {
                trace(
                    host_name,
                    "settlement delivery failed: closing control-plane client",
                );
            }
            false
        }
    }
}

#[cfg(test)]
mod tests;
