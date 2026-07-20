use super::super::*;
use super::*;
use crate::frame::read_frame;
use std::cmp::Ordering;

#[test]
fn canonical_extension_session_does_not_infer_control_plane_role() {
    let message = json!({
        "jsonrpc": "2.0",
        "id": "get-info",
        "method": "getInfo",
        "params": {
            "session_id": "sky-cua-control-plane-v1",
            "turn_id": "control-plane-lease-v1"
        }
    });

    assert_eq!(client_role_for_message(&message), ClientRole::Primary);
}

#[test]
fn host_hello_negotiates_control_plane_and_reports_capability_downgrade() {
    let mut state = test_host_state();
    let client_id = state.add_client(test_client().writer.clone());

    let outcome = state.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello-1",
            "daemon-7",
            &[CONTROL_PLANE_CAPABILITY, "future_capability"],
        ),
    );

    assert!(outcome.fenced_clients.is_empty());
    assert_eq!(outcome.response["result"]["protocol_version"], json!(1));
    assert!(
        outcome.response["result"]["host_instance_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(outcome.response["result"]["browser_instance_id"].is_null());
    assert_eq!(
        outcome.response["result"]["browser_instance_stability"],
        json!("unavailable")
    );
    assert_eq!(outcome.response["result"]["role"], json!("control_plane"));
    assert_eq!(outcome.response["result"]["mode"], json!("hybrid"));
    assert_eq!(outcome.response["result"]["owner_mode"], json!("hybrid"));
    assert_eq!(
        outcome.response["result"]["capabilities"],
        json!([CONTROL_PLANE_CAPABILITY])
    );
    assert_eq!(
        outcome.response["result"]["unsupported_capabilities"],
        json!(["future_capability"])
    );
    assert_eq!(state.clients[&client_id].role, ClientRole::ControlPlane);
    assert_eq!(
        state.clients[&client_id].daemon_generation.as_deref(),
        Some("daemon-7")
    );
}

#[test]
fn strict_hello_evicts_preexisting_legacy_operation_clients_and_reports_counts() {
    let mut state = test_host_state();
    let primary_id = state.add_client(test_client().writer.clone());
    state.update_client_role_for_message(
        primary_id,
        &json!({ "id": 1, "method": "getInfo", "params": { "session_id": "legacy" } }),
    );
    let ephemeral_id = state.add_client(test_client().writer.clone());
    state.update_client_role_for_message(
        ephemeral_id,
        &json!({
            "id": 2,
            "method": "getTabs",
            "params": { "_sky_cua_client_role": "ephemeral" }
        }),
    );
    let heartbeat_id = state.add_client(test_client().writer.clone());
    state.update_client_role_for_message(
        heartbeat_id,
        &json!({
            "id": 3,
            "method": "ping",
            "params": { "_sky_cua_client_role": "heartbeat" }
        }),
    );
    state.pending_chrome_requests.insert(
        "legacy-request".to_string(),
        PendingChromeRequest {
            client_id: primary_id,
            client_request_id: json!(1),
            created_at: Instant::now(),
            settlement: None,
            state: PendingRequestState::Active,
        },
    );

    let control_plane_id = state.add_client(test_client().writer.clone());
    let outcome = state.handle_host_hello(
        control_plane_id,
        &control_plane_hello_with_mode(
            "strict",
            "daemon-10",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );

    let rejected_ids = outcome
        .rejected_legacy_clients
        .iter()
        .map(|(id, _)| *id)
        .collect::<HashSet<_>>();
    assert_eq!(
        rejected_ids,
        HashSet::from([primary_id, ephemeral_id, heartbeat_id])
    );
    assert!(!state.clients.contains_key(&primary_id));
    assert!(!state.clients.contains_key(&ephemeral_id));
    assert!(!state.clients.contains_key(&heartbeat_id));
    assert!(state.pending_chrome_requests.is_empty());
    assert_eq!(state.owner_mode, OwnerMode::Strict);
    assert_eq!(outcome.response["result"]["mode"], json!("strict"));
    assert_eq!(
        outcome.response["result"]["migration_telemetry"]["legacy_clients_evicted"],
        json!(3)
    );
}

#[test]
fn strict_rejects_new_non_control_plane_operation_with_stable_diagnostic() {
    let state = Arc::new(Mutex::new(test_host_state()));
    let control_plane_id = state
        .lock()
        .unwrap()
        .add_client(test_client().writer.clone());
    state.lock().unwrap().handle_host_hello(
        control_plane_id,
        &control_plane_hello_with_mode(
            "strict",
            "daemon-10",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );
    let (mut peer, writer_stream) = UnixStream::pair().unwrap();
    let legacy_id = state
        .lock()
        .unwrap()
        .add_client(Arc::new(Mutex::new(writer_stream)));

    handle_client_message(
        &state,
        legacy_id,
        json!({ "jsonrpc": "2.0", "id": "legacy", "method": "getTabs" }),
    );

    let response = read_frame(&mut peer).unwrap().unwrap();
    assert_eq!(
        response["error"]["data"]["type"],
        json!("sky_cua_host_strict_owner_required")
    );
    assert_eq!(response["error"]["data"]["owner_mode"], json!("strict"));
    let state = state.lock().unwrap();
    assert_eq!(state.clients[&legacy_id].role, ClientRole::Unknown);
    assert!(state.pending_chrome_requests.is_empty());
    assert_eq!(state.strict_legacy_requests_rejected, 1);
}

#[test]
fn strict_does_not_forward_non_control_plane_operation_notifications() {
    let state = Arc::new(Mutex::new(test_host_state()));
    let control_plane_id = state
        .lock()
        .unwrap()
        .add_client(test_client().writer.clone());
    state.lock().unwrap().handle_host_hello(
        control_plane_id,
        &control_plane_hello_with_mode(
            "strict",
            "daemon-10",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );
    let (mut peer, writer_stream) = UnixStream::pair().unwrap();
    let legacy_id = state
        .lock()
        .unwrap()
        .add_client(Arc::new(Mutex::new(writer_stream)));

    handle_client_message(
        &state,
        legacy_id,
        json!({ "jsonrpc": "2.0", "method": "legacyMutation" }),
    );

    let response = read_frame(&mut peer).unwrap().unwrap();
    assert!(response["id"].is_null());
    assert_eq!(
        response["error"]["data"]["type"],
        json!("sky_cua_host_strict_owner_required")
    );
    assert_eq!(state.lock().unwrap().next_chrome_id, 1);
}

#[test]
fn strict_routes_side_panel_only_to_capable_control_plane() {
    let mut clients = HashMap::new();
    clients.insert(1, test_client_with_role(ClientRole::Primary));
    clients.insert(2, test_client_with_role(ClientRole::ControlPlane));

    assert_eq!(
        select_chrome_request_client_id(&clients, ChromeRequestRoute::SidePanel, OwnerMode::Strict,),
        Err(ChromeClientRouteError::NoClients)
    );
    clients
        .get_mut(&2)
        .unwrap()
        .capabilities
        .insert(SIDE_PANEL_REQUESTS_CAPABILITY.to_string());
    assert_eq!(
        select_chrome_request_client_id(&clients, ChromeRequestRoute::SidePanel, OwnerMode::Strict,),
        Ok(2)
    );
}

#[test]
fn strict_does_not_restore_the_legacy_heartbeat_route_without_a_control_plane() {
    let mut clients = HashMap::new();
    clients.insert(1, test_client_with_role(ClientRole::Heartbeat));

    assert_eq!(
        select_chrome_request_client_id(&clients, ChromeRequestRoute::Ping, OwnerMode::Strict),
        Err(ChromeClientRouteError::NoClients)
    );
}

#[test]
fn strict_to_hybrid_rollback_requires_an_idle_request_ledger() {
    let mut state = test_host_state();
    let strict_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        strict_id,
        &control_plane_hello_with_mode(
            "strict",
            "daemon-10",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );
    state.pending_chrome_requests.insert(
        "in-flight".to_string(),
        PendingChromeRequest {
            client_id: strict_id,
            client_request_id: json!("operation"),
            created_at: Instant::now(),
            settlement: None,
            state: PendingRequestState::Active,
        },
    );
    let hybrid_id = state.add_client(test_client().writer.clone());
    let hybrid_hello = control_plane_hello_with_mode(
        "hybrid",
        "daemon-11",
        &[CONTROL_PLANE_CAPABILITY],
        Some("hybrid"),
    );

    let blocked = state.handle_host_hello(hybrid_id, &hybrid_hello);
    assert_eq!(
        blocked.response["error"]["data"]["type"],
        json!("sky_cua_host_mode_transition_unsafe")
    );
    assert_eq!(state.owner_mode, OwnerMode::Strict);
    assert_eq!(state.clients[&strict_id].role, ClientRole::ControlPlane);

    state.pending_chrome_requests.clear();
    let accepted = state.handle_host_hello(hybrid_id, &hybrid_hello);
    assert_eq!(accepted.response["result"]["mode"], json!("hybrid"));
    assert_eq!(accepted.fenced_clients.len(), 1);
    assert_eq!(state.owner_mode, OwnerMode::Hybrid);
}

#[test]
fn acknowledged_strict_release_restores_legacy_on_surviving_host_after_drain() {
    let state = Arc::new(Mutex::new(test_host_state()));
    let (mut control_peer, control_writer) = UnixStream::pair().unwrap();
    let control_plane_id = state
        .lock()
        .unwrap()
        .add_client(Arc::new(Mutex::new(control_writer)));
    state.lock().unwrap().handle_host_hello(
        control_plane_id,
        &control_plane_hello_with_mode(
            "strict",
            "daemon-10",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );
    let (mut legacy_peer, legacy_writer) = UnixStream::pair().unwrap();
    let legacy_id = state
        .lock()
        .unwrap()
        .add_client(Arc::new(Mutex::new(legacy_writer)));

    state
        .lock()
        .unwrap()
        .queued_settlements
        .push_back(json!({ "method": SKY_CUA_HOST_SETTLEMENT_METHOD }));
    handle_client_message(
        &state,
        control_plane_id,
        owner_release("blocked-release", "daemon-10"),
    );
    let blocked = read_frame(&mut control_peer).unwrap().unwrap();
    assert_eq!(
        blocked["error"]["data"]["type"],
        json!("sky_cua_host_mode_transition_unsafe")
    );
    assert_eq!(state.lock().unwrap().owner_mode, OwnerMode::Strict);

    state.lock().unwrap().queued_settlements.clear();
    handle_client_message(
        &state,
        control_plane_id,
        owner_release("release", "daemon-10"),
    );
    let released = read_frame(&mut control_peer).unwrap().unwrap();
    assert_eq!(released["result"]["owner_mode"], json!("hybrid"));
    assert_eq!(state.lock().unwrap().owner_mode, OwnerMode::Hybrid);

    state.lock().unwrap().remove_client(control_plane_id);
    handle_client_message(
        &state,
        legacy_id,
        json!({ "jsonrpc": "2.0", "id": "legacy-ping", "method": "ping" }),
    );
    let legacy_response = read_frame(&mut legacy_peer).unwrap().unwrap();
    assert_eq!(legacy_response["result"], json!("pong"));
}

#[test]
fn strict_mode_keeps_app_server_controls_local() {
    let mut state = test_host_state();
    state.owner_mode = OwnerMode::Strict;

    for method in [
        "ensureCodexAppServer",
        "codexRuntime/hello",
        "codexRuntime/ensure",
        "codexRuntime/restart",
    ] {
        assert!(is_app_server_local_method(method));
    }
    assert!(state.clients.is_empty());
}

#[test]
fn host_hello_rejects_invalid_protocol_and_missing_control_plane_capability() {
    let mut state = test_host_state();
    let client_id = state.add_client(test_client().writer.clone());
    let mut wrong_version =
        control_plane_hello("hello-version", "daemon-7", &[CONTROL_PLANE_CAPABILITY]);
    wrong_version["params"]["protocol_version"] = json!(2);

    let version_outcome = state.handle_host_hello(client_id, &wrong_version);
    assert_eq!(
        version_outcome.response["error"]["data"]["type"],
        json!("sky_cua_host_unsupported_protocol")
    );
    assert_eq!(state.clients[&client_id].role, ClientRole::Unknown);

    let capability_outcome = state.handle_host_hello(
        client_id,
        &control_plane_hello("hello-capability", "daemon-7", &[HEARTBEAT_CAPABILITY]),
    );
    assert_eq!(
        capability_outcome.response["error"]["data"]["type"],
        json!("sky_cua_host_missing_capability")
    );
    assert_eq!(state.clients[&client_id].role, ClientRole::Unknown);

    let invalid_mode = state.handle_host_hello(
        client_id,
        &control_plane_hello_with_mode(
            "hello-mode",
            "daemon-7",
            &[CONTROL_PLANE_CAPABILITY],
            Some("legacy"),
        ),
    );
    assert_eq!(
        invalid_mode.response["error"]["data"]["type"],
        json!("sky_cua_host_invalid_owner_mode")
    );
    assert_eq!(state.clients[&client_id].role, ClientRole::Unknown);
}

#[test]
fn control_plane_role_is_immutable_after_hello() {
    let mut state = test_host_state();
    let client_id = state.add_client(test_client().writer.clone());
    let first = state.handle_host_hello(
        client_id,
        &control_plane_hello("hello-1", "daemon-7", &[CONTROL_PLANE_CAPABILITY]),
    );
    assert!(first.response.get("result").is_some());

    state.update_client_role_for_message(
        client_id,
        &json!({
            "jsonrpc": "2.0",
            "id": "legacy-request",
            "method": "getInfo",
            "params": { "session_id": "legacy-primary" }
        }),
    );
    assert_eq!(state.clients[&client_id].role, ClientRole::ControlPlane);

    let repeated = state.handle_host_hello(
        client_id,
        &control_plane_hello("hello-2", "daemon-8", &[CONTROL_PLANE_CAPABILITY]),
    );
    assert_eq!(
        repeated.response["error"]["data"]["type"],
        json!("sky_cua_host_role_immutable")
    );
    assert_eq!(
        state.clients[&client_id].daemon_generation.as_deref(),
        Some("daemon-7")
    );
}

#[test]
fn legacy_role_selection_prevents_late_control_plane_hello() {
    let mut state = test_host_state();
    let client_id = state.add_client(test_client().writer.clone());
    state.update_client_role_for_message(
        client_id,
        &json!({ "id": 1, "method": "getInfo", "params": { "session_id": "legacy" } }),
    );

    let outcome = state.handle_host_hello(
        client_id,
        &control_plane_hello("late-hello", "daemon-7", &[CONTROL_PLANE_CAPABILITY]),
    );

    assert_eq!(
        outcome.response["error"]["data"]["type"],
        json!("sky_cua_host_role_immutable")
    );
    assert_eq!(state.clients[&client_id].role, ClientRole::Primary);
}

#[test]
fn control_plane_marker_without_hello_is_rejected_without_role_inference() {
    let (mut peer, writer_stream) = UnixStream::pair().unwrap();
    let state = Arc::new(Mutex::new(test_host_state()));
    let client_id = state
        .lock()
        .unwrap()
        .add_client(Arc::new(Mutex::new(writer_stream)));

    handle_client_message(
        &state,
        client_id,
        json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "method": "getInfo",
            "params": { "_sky_cua_client_role": "control_plane" }
        }),
    );

    let response = read_frame(&mut peer).unwrap().unwrap();
    assert_eq!(
        response["error"]["data"]["type"],
        json!("sky_cua_host_hello_required")
    );
    assert_eq!(
        state.lock().unwrap().clients[&client_id].role,
        ClientRole::Unknown
    );
}

#[test]
fn control_plane_marker_after_hello_is_accepted() {
    let state = Arc::new(Mutex::new(test_host_state()));
    let (mut peer, writer_stream) = UnixStream::pair().unwrap();
    let client_id = state
        .lock()
        .unwrap()
        .add_client(Arc::new(Mutex::new(writer_stream)));
    state.lock().unwrap().handle_host_hello(
        client_id,
        &control_plane_hello("hello", "daemon-10", &[CONTROL_PLANE_CAPABILITY]),
    );

    handle_client_message(
        &state,
        client_id,
        json!({
            "jsonrpc": "2.0",
            "id": "request-after-hello",
            "method": "ping",
            "params": { "_sky_cua_client_role": "control_plane" }
        }),
    );

    let response = read_frame(&mut peer).unwrap().unwrap();
    assert_eq!(response["result"], json!("pong"));
    let state = state.lock().unwrap();
    assert_eq!(state.clients[&client_id].role, ClientRole::ControlPlane);
    assert!(state.pending_chrome_requests.is_empty());
}

#[test]
fn newer_control_plane_generation_fences_older_without_touching_primary() {
    let mut state = test_host_state();
    let primary_id = state.add_client(test_client().writer.clone());
    state.update_client_role_for_message(
        primary_id,
        &json!({ "id": 1, "method": "getInfo", "params": { "session_id": "legacy" } }),
    );
    let old_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        old_id,
        &control_plane_hello("old", "daemon-10", &[CONTROL_PLANE_CAPABILITY]),
    );
    state.pending_chrome_requests.insert(
        "old-extension-request".to_string(),
        PendingChromeRequest {
            client_id: old_id,
            client_request_id: json!("old-client-request"),
            created_at: Instant::now(),
            settlement: None,
            state: PendingRequestState::Active,
        },
    );
    state.pending_client_requests.insert(
        "old-client-request".to_string(),
        PendingClientRequest {
            client_id: old_id,
            chrome_request_id: json!("old-extension-request"),
            created_at: Instant::now(),
        },
    );

    let new_id = state.add_client(test_client().writer.clone());
    let outcome = state.handle_host_hello(
        new_id,
        &control_plane_hello("new", "daemon-11", &[CONTROL_PLANE_CAPABILITY]),
    );

    assert_eq!(outcome.fenced_clients.len(), 1);
    assert_eq!(outcome.fenced_clients[0].0, old_id);
    assert!(!state.clients.contains_key(&old_id));
    assert_eq!(state.clients[&new_id].role, ClientRole::ControlPlane);
    assert_eq!(state.clients[&primary_id].role, ClientRole::Primary);
    assert!(state.pending_chrome_requests.is_empty());
    assert!(state.pending_client_requests.is_empty());
}

#[test]
fn stale_or_duplicate_control_plane_generation_is_rejected_without_eviction() {
    let mut state = test_host_state();
    let active_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        active_id,
        &control_plane_hello("active", "daemon-10", &[CONTROL_PLANE_CAPABILITY]),
    );

    for generation in ["daemon-9", "daemon-10"] {
        let candidate_id = state.add_client(test_client().writer.clone());
        let outcome = state.handle_host_hello(
            candidate_id,
            &control_plane_hello("candidate", generation, &[CONTROL_PLANE_CAPABILITY]),
        );
        assert!(outcome.fenced_clients.is_empty());
        assert_eq!(
            outcome.response["error"]["data"]["type"],
            json!("sky_cua_host_stale_generation")
        );
        assert_eq!(state.clients[&candidate_id].role, ClientRole::Unknown);
        assert_eq!(state.clients[&active_id].role, ClientRole::ControlPlane);
    }
}

#[test]
fn strict_allows_same_generation_reconnect_after_active_client_removal() {
    let mut state = test_host_state();
    let active_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        active_id,
        &control_plane_hello_with_mode(
            "active",
            "daemon-10",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );
    state.remove_client(active_id);

    let reconnect_id = state.add_client(test_client().writer.clone());
    let outcome = state.handle_host_hello(
        reconnect_id,
        &control_plane_hello_with_mode(
            "reconnect",
            "daemon-10",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );

    assert!(outcome.response.get("result").is_some());
    assert!(outcome.fenced_clients.is_empty());
    assert_eq!(state.clients[&reconnect_id].role, ClientRole::ControlPlane);
    assert_eq!(state.owner_mode, OwnerMode::Strict);
    assert_eq!(state.owner_daemon_generation.as_deref(), Some("daemon-10"));
}

#[test]
fn strict_rejects_older_generation_after_active_client_removal() {
    let mut state = test_host_state();
    let active_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        active_id,
        &control_plane_hello_with_mode(
            "active",
            "daemon-10",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );
    state.remove_client(active_id);

    let stale_id = state.add_client(test_client().writer.clone());
    let outcome = state.handle_host_hello(
        stale_id,
        &control_plane_hello_with_mode(
            "stale",
            "daemon-9",
            &[CONTROL_PLANE_CAPABILITY],
            Some("strict"),
        ),
    );

    assert_eq!(
        outcome.response["error"]["data"]["type"],
        json!("sky_cua_host_stale_generation")
    );
    assert!(outcome.fenced_clients.is_empty());
    assert_eq!(state.clients[&stale_id].role, ClientRole::Unknown);
    assert_eq!(state.owner_mode, OwnerMode::Strict);
    assert_eq!(state.owner_daemon_generation.as_deref(), Some("daemon-10"));
}

#[test]
fn control_plane_is_non_prunable_during_ephemeral_client_churn() {
    let mut state = test_host_state();
    let control_plane_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        control_plane_id,
        &control_plane_hello("control", "daemon-10", &[CONTROL_PLANE_CAPABILITY]),
    );

    let mut evicted = Vec::new();
    for _ in 0..(MAX_NON_PRIMARY_CLIENTS + 2) {
        let (_, removed) = state.accept_client(test_client().writer.clone());
        evicted.extend(removed);
    }

    assert!(state.clients.contains_key(&control_plane_id));
    assert!(
        evicted
            .iter()
            .all(|(_, client)| client.role != ClientRole::ControlPlane)
    );
}

#[test]
fn chrome_routes_ping_to_control_plane_then_heartbeat_fallback() {
    let mut clients = HashMap::new();
    clients.insert(1, test_client_with_role(ClientRole::Primary));
    clients.insert(2, test_client_with_role(ClientRole::Heartbeat));
    assert_eq!(
        select_chrome_request_client_id(&clients, ChromeRequestRoute::Ping, OwnerMode::Hybrid),
        Ok(2)
    );

    clients.insert(3, test_client_with_role(ClientRole::ControlPlane));
    assert_eq!(
        select_chrome_request_client_id(&clients, ChromeRequestRoute::Ping, OwnerMode::Hybrid),
        Ok(3)
    );
}

#[test]
fn side_panel_requests_stay_primary_without_control_plane_capability() {
    let mut clients = HashMap::new();
    clients.insert(1, test_client_with_role(ClientRole::Primary));
    clients.insert(2, test_client_with_role(ClientRole::ControlPlane));
    clients.insert(3, test_client_with_role(ClientRole::Heartbeat));
    assert_eq!(
        select_chrome_request_client_id(&clients, ChromeRequestRoute::SidePanel, OwnerMode::Hybrid,),
        Ok(1)
    );

    clients
        .get_mut(&2)
        .unwrap()
        .capabilities
        .insert(SIDE_PANEL_REQUESTS_CAPABILITY.to_string());
    assert_eq!(
        select_chrome_request_client_id(&clients, ChromeRequestRoute::SidePanel, OwnerMode::Hybrid,),
        Ok(2)
    );

    clients.get_mut(&2).unwrap().capabilities.clear();
    clients.remove(&1);
    assert_eq!(
        select_chrome_request_client_id(&clients, ChromeRequestRoute::SidePanel, OwnerMode::Hybrid,),
        Err(ChromeClientRouteError::NoClients)
    );
}

#[test]
fn notifications_are_broadcast_to_control_plane_and_legacy_primary() {
    let mut state = test_host_state();
    state
        .clients
        .insert(1, test_client_with_role(ClientRole::Primary));
    state
        .clients
        .insert(2, test_client_with_role(ClientRole::ControlPlane));
    state
        .clients
        .insert(3, test_client_with_role(ClientRole::Heartbeat));

    assert_eq!(state.notification_client_writers().len(), 2);
}

#[test]
fn extension_bound_params_strip_only_host_private_fields() {
    let message = json!({
        "jsonrpc": "2.0",
        "id": "request-1",
        "method": "getTabs",
        "params": {
            "session_id": "sky-cua-control-plane-v1",
            "turn_id": "control-plane-lease-v1",
            "_sky_cua_client_role": "control_plane",
            "_sky_cua_observe_turns": false,
            "_sky_cua_host_request": {
                "operation_id": "operation-1",
                "daemon_generation": "daemon-1",
                "actor_generation": 7,
                "target_lifetime_key": { "browser_instance_id": "browser-1", "tab_id": 42 },
                "operation_class": "mutation",
                "settlement_deadline_ms": unix_epoch_ms() + 60_000
            },
            "tabId": 42
        }
    });

    assert_eq!(
        strip_host_private_params(message),
        json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "method": "getTabs",
            "params": {
                "session_id": "sky-cua-control-plane-v1",
                "turn_id": "control-plane-lease-v1",
                "tabId": 42
            }
        })
    );
}

#[test]
fn parses_control_plane_settlement_metadata_without_chrome_exposure() {
    let deadline = unix_epoch_ms() + 60_000;
    let message = json!({
        "id": "actor-request",
        "method": "executeCdp",
        "params": {
            "tabId": 42,
            "_sky_cua_host_request": {
                "operation_id": "operation-1",
                "daemon_generation": "daemon-9",
                "actor_generation": "actor-3",
                "target_lifetime_key": {
                    "browser_instance_id": "browser-1",
                    "tab_id": 42,
                    "target_lifetime": "target-4"
                },
                "operation_class": "mutation",
                "settlement_deadline_ms": deadline
            }
        }
    });

    let metadata = settlement_metadata(&message).unwrap();
    assert_eq!(metadata.operation_id, "operation-1");
    assert_eq!(metadata.daemon_generation, "daemon-9");
    assert_eq!(metadata.actor_generation, json!("actor-3"));
    assert_eq!(metadata.operation_class, OperationClass::Mutation);
    assert_eq!(metadata.settlement_deadline_ms, deadline);
    assert_eq!(
        metadata.target_lifetime_key,
        Some(json!({
            "browser_instance_id": "browser-1",
            "tab_id": 42,
            "target_lifetime": "target-4"
        }))
    );
    assert!(
        strip_host_private_params(message)["params"]
            .get(SKY_CUA_HOST_REQUEST_PARAM)
            .is_none()
    );
}

#[test]
fn daemon_generation_order_handles_numeric_suffixes_and_sortable_ids() {
    assert_eq!(
        compare_daemon_generations("daemon-10", "daemon-9"),
        Ordering::Greater
    );
    assert_eq!(
        compare_daemon_generations("019f-b", "019f-a"),
        Ordering::Greater
    );
    assert_eq!(compare_daemon_generations("10", "9"), Ordering::Greater);
}

fn test_client() -> Client {
    test_client_with_role(ClientRole::Unknown)
}

fn test_client_with_role(role: ClientRole) -> Client {
    let (stream, _peer) = UnixStream::pair().unwrap();
    Client {
        writer: Arc::new(Mutex::new(stream)),
        role,
        daemon_generation: (role == ClientRole::ControlPlane).then_some("daemon-1".to_string()),
        capabilities: HashSet::new(),
        connected_at: Instant::now(),
    }
}

fn control_plane_hello(id: &str, daemon_generation: impl ToString, capabilities: &[&str]) -> Value {
    control_plane_hello_with_mode(id, daemon_generation, capabilities, None)
}

fn control_plane_hello_with_mode(
    id: &str,
    daemon_generation: impl ToString,
    capabilities: &[&str],
    owner_mode: Option<&str>,
) -> Value {
    let mut params = json!({
        "protocol_version": SKY_CUA_HOST_PROTOCOL_VERSION,
        "client_role": CONTROL_PLANE_ROLE,
        "daemon_generation": daemon_generation.to_string(),
        "capabilities": capabilities,
    });
    if let Some(owner_mode) = owner_mode {
        params["owner_mode"] = json!(owner_mode);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": SKY_CUA_HOST_HELLO_METHOD,
        "params": params,
    })
}

fn owner_release(id: &str, daemon_generation: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": SKY_CUA_HOST_RELEASE_METHOD,
        "params": {
            "daemon_generation": daemon_generation,
            "owner_mode": "hybrid",
        }
    })
}

fn test_host_state() -> HostState {
    let stdout = Arc::new(Mutex::new(io::stdout()));
    HostState::new(
        "com.openai.codexextension",
        Arc::clone(&stdout),
        RolloutTracker::without_worker("com.openai.codexextension".to_string(), stdout, None),
    )
}
