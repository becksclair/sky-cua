use super::*;
use std::cmp::Ordering;

pub(super) const SKY_CUA_HOST_HELLO_METHOD: &str = "skyCuaHost/hello";
pub(super) const SKY_CUA_HOST_RELEASE_METHOD: &str = "skyCuaHost/release";
pub(super) const SKY_CUA_HOST_PROTOCOL_VERSION: u64 = 1;
pub(super) const CONTROL_PLANE_ROLE: &str = "control_plane";
pub(super) const CONTROL_PLANE_CAPABILITY: &str = "control_plane";
pub(super) const EXTENSION_EVENTS_CAPABILITY: &str = "extension_events";
pub(super) const HEARTBEAT_CAPABILITY: &str = "heartbeat";
pub(super) const PRIVATE_PARAM_STRIPPING_CAPABILITY: &str = "private_param_stripping";
pub(super) const SIDE_PANEL_REQUESTS_CAPABILITY: &str = "side_panel_requests";
pub(super) const OWNER_RELEASE_CAPABILITY: &str = "owner_release";
pub(super) const SETTLEMENTS_CAPABILITY: &str = "settlements";
pub(super) const SETTLEMENT_ACK_CAPABILITY: &str = "settlement_ack";
pub(super) const SKY_CUA_HOST_REQUEST_PARAM: &str = "_sky_cua_host_request";
pub(super) const SKY_CUA_OPERATION_ID_PARAM: &str = "_sky_cua_operation_id";
pub(super) const SKY_CUA_DAEMON_GENERATION_PARAM: &str = "_sky_cua_daemon_generation";
pub(super) const SKY_CUA_ACTOR_GENERATION_PARAM: &str = "_sky_cua_actor_generation";
pub(super) const SKY_CUA_TARGET_LIFETIME_KEY_PARAM: &str = "_sky_cua_target_lifetime_key";
pub(super) const SKY_CUA_OPERATION_CLASS_PARAM: &str = "_sky_cua_operation_class";
pub(super) const SKY_CUA_SETTLEMENT_DEADLINE_MS_PARAM: &str = "_sky_cua_settlement_deadline_ms";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnerMode {
    Hybrid,
    Strict,
}

impl OwnerMode {
    fn parse(value: Option<&Value>) -> Option<Self> {
        match value {
            None => Some(Self::Hybrid),
            Some(Value::String(value)) if value == "hybrid" => Some(Self::Hybrid),
            Some(Value::String(value)) if value == "strict" => Some(Self::Strict),
            Some(_) => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::Strict => "strict",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChromeRequestRoute {
    Ping,
    SidePanel,
}

pub(super) struct HostHelloOutcome {
    pub(super) response: Value,
    pub(super) fenced_clients: Vec<(usize, Client)>,
    pub(super) rejected_legacy_clients: Vec<(usize, Client)>,
}

impl HostState {
    pub(super) fn handle_owner_release(&mut self, client_id: usize, message: &Value) -> Value {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let error = |error_type: &str, message: &str, data: Value| {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32001,
                    "message": message,
                    "data": {
                        "type": error_type,
                        "details": data,
                    }
                }
            })
        };

        let Some(client) = self.clients.get(&client_id) else {
            return error(
                "sky_cua_host_client_gone",
                "Native-host client disconnected before owner release",
                json!({}),
            );
        };
        if client.role != ClientRole::ControlPlane {
            return error(
                "sky_cua_host_owner_release_forbidden",
                "Only the active control plane can release strict ownership",
                json!({ "selected_role": client_role_name(client.role) }),
            );
        }
        let requested_generation = message
            .pointer("/params/daemon_generation")
            .and_then(Value::as_str);
        if requested_generation != client.daemon_generation.as_deref()
            || requested_generation != self.owner_daemon_generation.as_deref()
        {
            return error(
                "sky_cua_host_generation_mismatch",
                "owner release daemon_generation does not match the active strict owner",
                json!({
                    "active_daemon_generation": self.owner_daemon_generation,
                    "requested_daemon_generation": requested_generation,
                }),
            );
        }
        if self.owner_mode != OwnerMode::Strict {
            return error(
                "sky_cua_host_owner_release_not_strict",
                "Native host is not under strict ownership",
                json!({ "owner_mode": self.owner_mode.name() }),
            );
        }
        if message
            .pointer("/params/owner_mode")
            .and_then(Value::as_str)
            != Some("hybrid")
        {
            return error(
                "sky_cua_host_invalid_owner_mode",
                "owner release must transition to hybrid mode",
                json!({ "requested_owner_mode": message.pointer("/params/owner_mode") }),
            );
        }
        if !self.pending_chrome_requests.is_empty()
            || !self.pending_client_requests.is_empty()
            || !self.queued_settlements.is_empty()
            || self.settlement_delivery_in_progress
        {
            return error(
                "sky_cua_host_mode_transition_unsafe",
                "Cannot release strict ownership while requests or settlements are unsettled",
                json!({
                    "active_browser_requests": self.pending_chrome_requests.len(),
                    "active_extension_requests": self.pending_client_requests.len(),
                    "queued_settlements": self.queued_settlements.len(),
                    "settlement_delivery_in_progress": self.settlement_delivery_in_progress,
                }),
            );
        }

        self.owner_mode = OwnerMode::Hybrid;
        self.owner_daemon_generation = None;
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "released": true,
                "owner_mode": "hybrid",
            }
        })
    }
}

impl HostState {
    pub(super) fn handle_host_hello(
        &mut self,
        client_id: usize,
        message: &Value,
    ) -> HostHelloOutcome {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let error = |error_type: &str, message: &str, data: Value| HostHelloOutcome {
            response: json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32001,
                    "message": message,
                    "data": {
                        "type": error_type,
                        "host_protocol_version": SKY_CUA_HOST_PROTOCOL_VERSION,
                        "details": data,
                    }
                }
            }),
            fenced_clients: Vec::new(),
            rejected_legacy_clients: Vec::new(),
        };

        let Some(client) = self.clients.get(&client_id) else {
            return error(
                "sky_cua_host_client_gone",
                "Native-host client disconnected before hello",
                json!({}),
            );
        };
        if client.role != ClientRole::Unknown {
            return error(
                "sky_cua_host_role_immutable",
                "Native-host client role is immutable after it is selected",
                json!({ "selected_role": client_role_name(client.role) }),
            );
        }

        let Some(params) = message.get("params").and_then(Value::as_object) else {
            return error(
                "sky_cua_host_invalid_hello",
                "skyCuaHost/hello requires object params",
                json!({ "field": "params" }),
            );
        };
        let protocol_version = params.get("protocol_version").and_then(Value::as_u64);
        if protocol_version != Some(SKY_CUA_HOST_PROTOCOL_VERSION) {
            return error(
                "sky_cua_host_unsupported_protocol",
                "Unsupported native-host control protocol version",
                json!({ "requested_protocol_version": protocol_version }),
            );
        }

        let declared_role = params
            .get("client_role")
            .or_else(|| params.get("role"))
            .or_else(|| params.get(SKY_CUA_CLIENT_ROLE_PARAM))
            .and_then(Value::as_str);
        if declared_role != Some(CONTROL_PLANE_ROLE) {
            return error(
                "sky_cua_host_unsupported_role",
                "skyCuaHost/hello supports only the control_plane role",
                json!({ "requested_role": declared_role }),
            );
        }
        let mut declared_roles = ["client_role", "role", SKY_CUA_CLIENT_ROLE_PARAM]
            .into_iter()
            .filter_map(|field| params.get(field).and_then(Value::as_str))
            .collect::<Vec<_>>();
        declared_roles.sort_unstable();
        declared_roles.dedup();
        if declared_roles.len() > 1 {
            return error(
                "sky_cua_host_conflicting_role",
                "skyCuaHost/hello role declarations conflict",
                json!({ "declared_roles": declared_roles }),
            );
        }

        let Some(daemon_generation) = params
            .get("daemon_generation")
            .and_then(Value::as_str)
            .filter(|generation| !generation.trim().is_empty())
            .map(str::to_string)
        else {
            return error(
                "sky_cua_host_invalid_generation",
                "control_plane hello requires a non-empty string daemon_generation",
                json!({ "field": "daemon_generation" }),
            );
        };
        let Some(advertised_capabilities) = params.get("capabilities").and_then(Value::as_array)
        else {
            return error(
                "sky_cua_host_invalid_capabilities",
                "control_plane hello requires a capability array",
                json!({ "field": "capabilities" }),
            );
        };
        let Some(advertised_capabilities) = advertised_capabilities
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
        else {
            return error(
                "sky_cua_host_invalid_capabilities",
                "control_plane capabilities must all be strings",
                json!({ "field": "capabilities" }),
            );
        };
        if !advertised_capabilities.contains(&CONTROL_PLANE_CAPABILITY) {
            return error(
                "sky_cua_host_missing_capability",
                "control_plane hello must advertise the control_plane capability",
                json!({ "required_capability": CONTROL_PLANE_CAPABILITY }),
            );
        }

        let Some(requested_mode) = OwnerMode::parse(params.get("owner_mode")) else {
            return error(
                "sky_cua_host_invalid_owner_mode",
                "control_plane owner_mode must be hybrid or strict",
                json!({ "requested_owner_mode": params.get("owner_mode") }),
            );
        };

        let active_control_plane = self.clients.iter().find_map(|(id, client)| {
            (client.role == ClientRole::ControlPlane)
                .then_some((*id, client.daemon_generation.as_deref().unwrap_or_default()))
        });
        if let Some((active_id, active_generation)) = active_control_plane
            && compare_daemon_generations(&daemon_generation, active_generation)
                != Ordering::Greater
        {
            return error(
                "sky_cua_host_stale_generation",
                "A control plane from the same or a newer daemon generation is already active",
                json!({
                    "active_client_id": active_id,
                    "active_daemon_generation": active_generation,
                    "requested_daemon_generation": daemon_generation,
                }),
            );
        }
        if active_control_plane.is_none()
            && self.owner_mode == OwnerMode::Strict
            && self
                .owner_daemon_generation
                .as_deref()
                .is_some_and(|generation| {
                    compare_daemon_generations(&daemon_generation, generation) == Ordering::Less
                })
        {
            return error(
                "sky_cua_host_stale_generation",
                "A control plane from a newer daemon generation previously owned strict mode",
                json!({
                    "active_daemon_generation": self.owner_daemon_generation,
                    "requested_daemon_generation": daemon_generation,
                }),
            );
        }
        if self.owner_mode == OwnerMode::Strict
            && requested_mode == OwnerMode::Hybrid
            && (!self.pending_chrome_requests.is_empty()
                || !self.pending_client_requests.is_empty())
        {
            return error(
                "sky_cua_host_mode_transition_unsafe",
                "Cannot roll back strict ownership while browser requests are unsettled",
                json!({
                    "active_browser_requests": self.pending_chrome_requests.len(),
                    "active_extension_requests": self.pending_client_requests.len(),
                }),
            );
        }

        let supported = host_control_plane_capabilities();
        let negotiated_capabilities = advertised_capabilities
            .iter()
            .filter(|capability| supported.contains(**capability))
            .map(|capability| (*capability).to_string())
            .collect::<HashSet<_>>();
        let mut unsupported_capabilities = advertised_capabilities
            .iter()
            .filter(|capability| !supported.contains(**capability))
            .map(|capability| (*capability).to_string())
            .collect::<Vec<_>>();
        unsupported_capabilities.sort();

        let evict_ids = self
            .clients
            .iter()
            .filter_map(|(id, client)| {
                (*id != client_id && client.role == ClientRole::ControlPlane).then_some(*id)
            })
            .collect::<Vec<_>>();
        let mut fenced_clients = Vec::new();
        for evict_id in evict_ids {
            if let Some(client) = self.clients.remove(&evict_id) {
                self.cleanup_pending_for_removed_client(evict_id, client.role);
                fenced_clients.push((evict_id, client));
            }
        }

        let legacy_evict_ids = if requested_mode == OwnerMode::Strict {
            self.clients
                .iter()
                .filter_map(|(id, client)| {
                    (*id != client_id
                        && matches!(
                            client.role,
                            ClientRole::Primary | ClientRole::Heartbeat | ClientRole::Ephemeral
                        ))
                    .then_some(*id)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut rejected_legacy_clients = Vec::new();
        for legacy_id in legacy_evict_ids {
            if let Some(client) = self.clients.remove(&legacy_id) {
                self.cleanup_pending_for_removed_client(legacy_id, client.role);
                rejected_legacy_clients.push((legacy_id, client));
            }
        }
        self.strict_legacy_clients_evicted = self
            .strict_legacy_clients_evicted
            .saturating_add(rejected_legacy_clients.len() as u64);

        let client = self
            .clients
            .get_mut(&client_id)
            .expect("hello client remains registered");
        client.role = ClientRole::ControlPlane;
        client.daemon_generation = Some(daemon_generation.clone());
        client.capabilities = negotiated_capabilities.clone();
        self.owner_mode = requested_mode;
        self.owner_daemon_generation = Some(daemon_generation.clone());
        self.supersede_prior_generation_settlements(&daemon_generation);

        let mut negotiated_capabilities = negotiated_capabilities.into_iter().collect::<Vec<_>>();
        negotiated_capabilities.sort();
        HostHelloOutcome {
            response: json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocol_version": SKY_CUA_HOST_PROTOCOL_VERSION,
                    "host_instance_id": self.host_instance_id,
                    "browser_instance_id": null,
                    "browser_instance_stability": "unavailable",
                    "browser_family": "unknown",
                    "role": CONTROL_PLANE_ROLE,
                    "mode": requested_mode.name(),
                    "owner_mode": requested_mode.name(),
                    "daemon_generation": daemon_generation,
                    "capabilities": negotiated_capabilities,
                    "unsupported_capabilities": unsupported_capabilities,
                    "migration_telemetry": {
                        "legacy_clients_evicted": self.strict_legacy_clients_evicted,
                        "legacy_requests_rejected": self.strict_legacy_requests_rejected,
                    },
                }
            }),
            fenced_clients,
            rejected_legacy_clients,
        }
    }
}

fn host_control_plane_capabilities() -> HashSet<&'static str> {
    HashSet::from([
        CONTROL_PLANE_CAPABILITY,
        EXTENSION_EVENTS_CAPABILITY,
        HEARTBEAT_CAPABILITY,
        PRIVATE_PARAM_STRIPPING_CAPABILITY,
        SIDE_PANEL_REQUESTS_CAPABILITY,
        SETTLEMENTS_CAPABILITY,
        SETTLEMENT_ACK_CAPABILITY,
        OWNER_RELEASE_CAPABILITY,
    ])
}

pub(super) fn compare_daemon_generations(candidate: &str, active: &str) -> Ordering {
    match (candidate.parse::<u128>(), active.parse::<u128>()) {
        (Ok(candidate), Ok(active)) => return candidate.cmp(&active),
        (Ok(_), Err(_)) | (Err(_), Ok(_)) => return candidate.cmp(active),
        (Err(_), Err(_)) => {}
    }

    match (
        split_numeric_suffix(candidate),
        split_numeric_suffix(active),
    ) {
        (Some((candidate_prefix, candidate_number)), Some((active_prefix, active_number)))
            if candidate_prefix == active_prefix =>
        {
            candidate_number.cmp(&active_number)
        }
        _ => candidate.cmp(active),
    }
}

fn split_numeric_suffix(value: &str) -> Option<(&str, u128)> {
    let prefix_len = value
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_ascii_digit())
        .map_or(0, |(index, character)| index + character.len_utf8());
    let (prefix, suffix) = value.split_at(prefix_len);
    (!suffix.is_empty())
        .then(|| suffix.parse::<u128>().ok().map(|number| (prefix, number)))
        .flatten()
}

fn client_role_name(role: ClientRole) -> &'static str {
    match role {
        ClientRole::Unknown => "unknown",
        ClientRole::Primary => "primary",
        ClientRole::ControlPlane => CONTROL_PLANE_ROLE,
        ClientRole::Heartbeat => "heartbeat",
        ClientRole::Ephemeral => "ephemeral",
    }
}

pub(super) fn reject_control_plane_request(
    state: &SharedState,
    client_id: usize,
    id: &Value,
    error_type: &str,
    error_message: &str,
) {
    let Some((writer, host_name)) = ({
        let state = state.lock().expect("host state mutex poisoned");
        state
            .client_writer(client_id)
            .map(|writer| (writer, state.host_name.clone()))
    }) else {
        return;
    };
    let _ = write_client_frame(
        &writer,
        &host_name,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32001,
                "message": error_message,
                "data": { "type": error_type }
            }
        }),
    );
}

fn select_legacy_primary_client_id(
    clients: &HashMap<usize, Client>,
) -> std::result::Result<usize, ChromeClientRouteError> {
    let mut primary_client_id = None;
    let mut unknown_client_id = None;
    let mut unknown_count = 0;
    for (id, client) in clients {
        match client.role {
            ClientRole::Primary => {
                if primary_client_id.is_some() {
                    return Err(ChromeClientRouteError::MultipleClients);
                }
                primary_client_id = Some(*id);
            }
            ClientRole::Unknown => {
                unknown_count += 1;
                unknown_client_id = Some(*id);
            }
            ClientRole::ControlPlane | ClientRole::Heartbeat | ClientRole::Ephemeral => {}
        }
    }
    if let Some(primary_client_id) = primary_client_id {
        return Ok(primary_client_id);
    }
    if unknown_count > 1 {
        return Err(ChromeClientRouteError::MultipleClients);
    }
    unknown_client_id.ok_or(ChromeClientRouteError::NoClients)
}

pub(super) fn select_chrome_request_client_id(
    clients: &HashMap<usize, Client>,
    route: ChromeRequestRoute,
    owner_mode: OwnerMode,
) -> std::result::Result<usize, ChromeClientRouteError> {
    let control_plane = clients
        .iter()
        .find(|(_, client)| client.role == ClientRole::ControlPlane);
    match route {
        ChromeRequestRoute::Ping => {
            if let Some((id, _)) = control_plane {
                return Ok(*id);
            }
            if owner_mode == OwnerMode::Strict {
                return Err(ChromeClientRouteError::NoClients);
            }
            let heartbeat = clients
                .iter()
                .filter(|(_, client)| client.role == ClientRole::Heartbeat)
                .max_by_key(|(id, client)| (client.connected_at, *id));
            if let Some((id, _)) = heartbeat {
                return Ok(*id);
            }
        }
        ChromeRequestRoute::SidePanel => {
            if let Some((id, client)) = control_plane
                && client.capabilities.contains(SIDE_PANEL_REQUESTS_CAPABILITY)
            {
                return Ok(*id);
            }
            if owner_mode == OwnerMode::Strict {
                return Err(ChromeClientRouteError::NoClients);
            }
            return select_legacy_primary_client_id(clients);
        }
    }
    select_primary_client_id(clients)
}

pub(super) fn strict_legacy_request_error(id: Value, rejected_count: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32001,
            "message": "Strict control-plane ownership rejects direct browser-operation requests",
            "data": {
                "type": "sky_cua_host_strict_owner_required",
                "owner_mode": "strict",
                "required_role": CONTROL_PLANE_ROLE,
                "legacy_requests_rejected": rejected_count,
            }
        }
    })
}

pub(super) fn strip_host_private_params(mut message: Value) -> Value {
    if let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) {
        params.remove(SKY_CUA_CLIENT_ROLE_PARAM);
        params.remove(SKY_CUA_OBSERVE_TURNS_PARAM);
        params.remove(SKY_CUA_HOST_REQUEST_PARAM);
        params.remove(SKY_CUA_OPERATION_ID_PARAM);
        params.remove(SKY_CUA_DAEMON_GENERATION_PARAM);
        params.remove(SKY_CUA_ACTOR_GENERATION_PARAM);
        params.remove(SKY_CUA_TARGET_LIFETIME_KEY_PARAM);
        params.remove(SKY_CUA_OPERATION_CLASS_PARAM);
        params.remove(SKY_CUA_SETTLEMENT_DEADLINE_MS_PARAM);
    }
    message
}

#[cfg(test)]
mod tests;
