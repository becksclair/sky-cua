use super::*;

pub(super) const SKY_CUA_HOST_SETTLEMENT_METHOD: &str = "skyCuaHost/settlement";
pub(super) const SKY_CUA_HOST_SETTLEMENT_ACK_METHOD: &str = "skyCuaHost/settlementAck";
const MAX_RETAINED_SETTLEMENTS: usize = 100;
const TOMBSTONE_TTL: Duration = Duration::from_secs(10 * 60);
const SETTLEMENT_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);
const SETTLEMENT_ACK_RETRY_INTERVAL: Duration = Duration::from_secs(1);

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
                {
                    self.queue_settlement(settlement_message(
                        "settlement_unknown",
                        &id,
                        &request.client_request_id,
                        &metadata,
                        None,
                    ));
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

    pub(super) fn tombstone_pending_id(&mut self, id: String) {
        self.pending_id_tombstones.insert(id, Instant::now());
    }

    pub(super) fn queue_settlement(&mut self, message: Value) {
        // Admission reserves this space before mutating work is dispatched, so
        // reaching the bound indicates a programming error rather than a reason
        // to silently discard an ambiguous settlement.
        assert!(
            self.queued_settlements.len() < MAX_RETAINED_SETTLEMENTS,
            "settlement retention admission invariant violated"
        );
        self.queued_settlements.push_back(message);
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

    fn begin_settlement_delivery(&mut self) -> Option<(usize, SharedClientWriter, Value)> {
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
        let message = self.queued_settlements.front()?.clone();
        self.settlement_delivery_in_progress = true;
        Some((client_id, writer, message))
    }

    fn finish_settlement_delivery(&mut self, client_id: usize, delivered: bool) {
        assert!(
            self.settlement_delivery_in_progress,
            "settlement delivery completed without an active claim"
        );
        if delivered {
            self.settlement_delivered_to = Some(client_id);
            self.settlement_delivered_at = Some(Instant::now());
        } else {
            self.settlement_delivered_to = None;
            self.settlement_delivered_at = None;
        }
        self.settlement_delivery_in_progress = false;
    }

    pub(super) fn acknowledge_settlement(&mut self, client_id: usize, message: &Value) -> bool {
        let Some(client) = self.clients.get(&client_id) else {
            return false;
        };
        if client.role != ClientRole::ControlPlane
            || self.settlement_delivered_to != Some(client_id)
            || message
                .pointer("/params/acknowledging_daemon_generation")
                .and_then(Value::as_str)
                != client.daemon_generation.as_deref()
        {
            return false;
        }
        let Some(settlement) = self.queued_settlements.front() else {
            return false;
        };
        let identity_matches = [
            "operation_id",
            "daemon_generation",
            "actor_generation",
            "chrome_request_id",
        ]
        .into_iter()
        .all(|field| {
            message.pointer(&format!("/params/{field}"))
                == settlement.pointer(&format!("/params/{field}"))
        });
        if !identity_matches {
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
            .map(|(client_id, writer, message)| {
                (client_id, writer, state.host_name.clone(), message)
            })
    };
    let Some((client_id, writer, host_name, message)) = delivery else {
        return;
    };
    let delivered = write_client_frame(&writer, &host_name, &message);
    state
        .lock()
        .expect("host state mutex poisoned")
        .finish_settlement_delivery(client_id, delivered);
}

#[cfg(test)]
mod tests;
