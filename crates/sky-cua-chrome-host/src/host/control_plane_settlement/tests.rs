use super::super::*;
use super::*;
use crate::frame::read_frame;

#[test]
fn control_plane_normal_completion_returns_to_original_actor() {
    let (mut peer, writer_stream) = UnixStream::pair().unwrap();
    let state = Arc::new(Mutex::new(test_host_state()));
    {
        let mut state = state.lock().unwrap();
        let client_id = state.add_client(Arc::new(Mutex::new(writer_stream)));
        state.handle_host_hello(
            client_id,
            &control_plane_hello(
                "hello",
                "daemon-1",
                &[CONTROL_PLANE_CAPABILITY, SETTLEMENTS_CAPABILITY],
            ),
        );
        insert_control_plane_pending(&mut state, "chrome-normal", client_id, "operation-1");
    }

    handle_chrome_message(
        &state,
        json!({ "jsonrpc": "2.0", "id": "chrome-normal", "result": {"ok": true} }),
    );

    let response = read_frame(&mut peer).unwrap().unwrap();
    assert_eq!(response["id"], json!("actor-request-operation-1"));
    assert_eq!(response["result"]["ok"], json!(true));
    let state = state.lock().unwrap();
    assert!(state.pending_chrome_requests.is_empty());
    assert!(state.pending_id_tombstones.contains_key("chrome-normal"));
    assert!(state.queued_settlements.is_empty());
}

#[test]
fn control_plane_eof_retains_orphan_and_late_completion() {
    let mut state = test_host_state();
    let client_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        client_id,
        &control_plane_hello("hello", "daemon-1", &[CONTROL_PLANE_CAPABILITY]),
    );
    insert_control_plane_pending(&mut state, "chrome-late", client_id, "operation-late");

    state.remove_client(client_id);
    assert_eq!(
        state.pending_chrome_requests["chrome-late"].state,
        PendingRequestState::OrphanedPending
    );
    let state = Arc::new(Mutex::new(state));
    handle_chrome_message(
        &state,
        json!({ "jsonrpc": "2.0", "id": "chrome-late", "result": {"mutated": true} }),
    );

    let state = state.lock().unwrap();
    assert!(state.pending_chrome_requests.is_empty());
    assert_eq!(state.queued_settlements.len(), 1);
    assert_eq!(
        state.queued_settlements[0]["params"]["operation_id"],
        json!("operation-late")
    );
    assert_eq!(
        state.queued_settlements[0]["params"]["completion"]["result"]["mutated"],
        json!(true)
    );
}

#[test]
fn higher_generation_receives_old_actor_late_completion() {
    let mut host = test_host_state();
    let old_id = host.add_client(test_client().writer.clone());
    host.handle_host_hello(
        old_id,
        &control_plane_hello("old", "daemon-1", &[CONTROL_PLANE_CAPABILITY]),
    );
    insert_control_plane_pending(&mut host, "chrome-old", old_id, "operation-old");

    let (mut new_peer, new_writer) = UnixStream::pair().unwrap();
    let new_id = host.add_client(Arc::new(Mutex::new(new_writer)));
    let outcome = host.handle_host_hello(
        new_id,
        &control_plane_hello("new", "daemon-2", &[CONTROL_PLANE_CAPABILITY]),
    );
    assert_eq!(outcome.fenced_clients.len(), 1);
    assert_eq!(
        host.pending_chrome_requests["chrome-old"].state,
        PendingRequestState::OrphanedPending
    );

    let state = Arc::new(Mutex::new(host));
    handle_chrome_message(
        &state,
        json!({ "jsonrpc": "2.0", "id": "chrome-old", "result": {"late": true} }),
    );

    let settlement = read_frame(&mut new_peer).unwrap().unwrap();
    assert_eq!(settlement["method"], json!(SKY_CUA_HOST_SETTLEMENT_METHOD));
    assert_eq!(settlement["params"]["operation_id"], json!("operation-old"));
    assert_eq!(settlement["params"]["daemon_generation"], json!("daemon-1"));
    assert_eq!(settlement["params"]["actor_generation"], json!(7));
    assert_eq!(
        settlement["params"]["completion"]["id"],
        json!("chrome-old")
    );
}

#[test]
fn actor_absent_completion_remains_queued_until_delivery() {
    let mut host = test_host_state();
    let old_id = host.add_client(test_client().writer.clone());
    host.handle_host_hello(
        old_id,
        &control_plane_hello("old", "daemon-1", &[CONTROL_PLANE_CAPABILITY]),
    );
    insert_control_plane_pending(&mut host, "chrome-held", old_id, "operation-held");
    host.remove_client(old_id);
    let state = Arc::new(Mutex::new(host));
    handle_chrome_message(
        &state,
        json!({ "jsonrpc": "2.0", "id": "chrome-held", "result": {"done": true} }),
    );

    let mut host = Arc::try_unwrap(state).ok().unwrap().into_inner().unwrap();
    let (mut new_peer, new_writer) = UnixStream::pair().unwrap();
    let new_id = host.add_client(Arc::new(Mutex::new(new_writer)));
    host.handle_host_hello(
        new_id,
        &control_plane_hello("new", "daemon-2", &[CONTROL_PLANE_CAPABILITY]),
    );

    assert_eq!(host.queued_settlements.len(), 1);
    assert_eq!(
        host.queued_settlements[0]["params"]["operation_id"],
        json!("operation-held")
    );

    let state = Arc::new(Mutex::new(host));
    deliver_queued_settlements(&state);

    let settlement = read_frame(&mut new_peer).unwrap().unwrap();
    assert_eq!(
        settlement["params"]["operation_id"],
        json!("operation-held")
    );
    assert!(state.lock().unwrap().queued_settlements.is_empty());
}

#[test]
fn failed_direct_mutation_completion_is_requeued_as_settlement() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let state = Arc::new(Mutex::new(test_host_state()));
    {
        let mut state = state.lock().unwrap();
        let client_id = state.add_client(Arc::new(Mutex::new(writer_stream)));
        state.handle_host_hello(
            client_id,
            &control_plane_hello("hello", "daemon-1", &[CONTROL_PLANE_CAPABILITY]),
        );
        insert_control_plane_pending(&mut state, "chrome-failed", client_id, "operation-failed");
    }
    drop(peer);

    handle_chrome_message(
        &state,
        json!({ "jsonrpc": "2.0", "id": "chrome-failed", "result": {"mutated": true} }),
    );

    let state = state.lock().unwrap();
    assert!(state.pending_chrome_requests.is_empty());
    assert_eq!(state.queued_settlements.len(), 1);
    assert_eq!(
        state.queued_settlements[0]["params"]["operation_id"],
        json!("operation-failed")
    );
    assert_eq!(
        state.queued_settlements[0]["params"]["completion"]["result"]["mutated"],
        json!(true)
    );
}

#[test]
fn failed_hello_settlement_replay_keeps_ledger_entry() {
    let mut host = test_host_state();
    host.queue_settlement(settlement_message(
        "completed",
        "chrome-replay",
        &json!("actor-request-replay"),
        &test_settlement_metadata("operation-replay"),
        Some(json!({ "jsonrpc": "2.0", "id": "chrome-replay", "result": {"done": true} })),
    ));
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    let outcome = host.handle_host_hello(
        client_id,
        &control_plane_hello("hello", "daemon-1", &[CONTROL_PLANE_CAPABILITY]),
    );
    assert!(outcome.response.get("result").is_some());
    drop(peer);
    let state = Arc::new(Mutex::new(host));

    deliver_queued_settlements(&state);

    let state = state.lock().unwrap();
    assert_eq!(state.queued_settlements.len(), 1);
    assert_eq!(
        state.queued_settlements[0]["params"]["operation_id"],
        json!("operation-replay")
    );
    assert!(!state.settlement_delivery_in_progress);
}

#[test]
fn pending_id_tombstone_prevents_late_response_reuse() {
    let mut state = test_host_state();
    let tombstoned = format!("linux-{}-1", process::id());
    state.tombstone_pending_id(tombstoned.clone());
    state.next_chrome_id = 1;

    let allocated = state.allocate_chrome_id();

    assert_ne!(allocated, tombstoned);
    assert_eq!(allocated, format!("linux-{}-2", process::id()));
}

#[test]
fn mutating_retention_expiry_queues_settlement_unknown() {
    let mut state = test_host_state();
    let client_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        client_id,
        &control_plane_hello("hello", "daemon-1", &[CONTROL_PLANE_CAPABILITY]),
    );
    insert_control_plane_pending(&mut state, "chrome-expired", client_id, "operation-expired");
    state
        .pending_chrome_requests
        .get_mut("chrome-expired")
        .unwrap()
        .settlement
        .as_mut()
        .unwrap()
        .settlement_deadline_ms = 0;

    state.cleanup_old_requests();

    assert!(!state.pending_chrome_requests.contains_key("chrome-expired"));
    assert!(state.pending_id_tombstones.contains_key("chrome-expired"));
    assert_eq!(state.queued_settlements.len(), 1);
    assert_eq!(
        state.queued_settlements[0]["params"]["status"],
        json!("settlement_unknown")
    );
    assert!(state.queued_settlements[0]["params"]["completion"].is_null());
}

#[test]
fn control_plane_nonmutating_eof_uses_eager_cleanup() {
    let mut state = test_host_state();
    let client_id = state.add_client(test_client().writer.clone());
    state.handle_host_hello(
        client_id,
        &control_plane_hello("hello", "daemon-1", &[CONTROL_PLANE_CAPABILITY]),
    );
    insert_control_plane_pending(&mut state, "chrome-read", client_id, "operation-read");
    state
        .pending_chrome_requests
        .get_mut("chrome-read")
        .unwrap()
        .settlement
        .as_mut()
        .unwrap()
        .operation_class = OperationClass::ReadOnly;

    state.remove_client(client_id);

    assert!(!state.pending_chrome_requests.contains_key("chrome-read"));
    assert!(state.pending_id_tombstones.contains_key("chrome-read"));
    assert!(state.queued_settlements.is_empty());
}

fn insert_control_plane_pending(
    state: &mut HostState,
    chrome_request_id: &str,
    client_id: usize,
    operation_id: &str,
) {
    state.pending_chrome_requests.insert(
        chrome_request_id.to_string(),
        PendingChromeRequest {
            client_id,
            client_request_id: json!(format!("actor-request-{operation_id}")),
            created_at: Instant::now(),
            settlement: Some(test_settlement_metadata(operation_id)),
            state: PendingRequestState::Active,
        },
    );
}

fn test_settlement_metadata(operation_id: &str) -> SettlementMetadata {
    SettlementMetadata {
        operation_id: operation_id.to_string(),
        daemon_generation: "daemon-1".to_string(),
        actor_generation: json!(7),
        target_lifetime_key: Some(json!({
            "browser_instance_id": "browser-1",
            "tab_id": 42,
            "target_lifetime": "target-1"
        })),
        operation_class: OperationClass::Mutation,
        settlement_deadline_ms: unix_epoch_ms() + 60_000,
    }
}

fn test_client() -> Client {
    let (stream, _peer) = UnixStream::pair().unwrap();
    Client {
        writer: Arc::new(Mutex::new(stream)),
        role: ClientRole::Unknown,
        daemon_generation: None,
        capabilities: HashSet::new(),
        connected_at: Instant::now(),
    }
}

fn control_plane_hello(id: &str, daemon_generation: impl ToString, capabilities: &[&str]) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": SKY_CUA_HOST_HELLO_METHOD,
        "params": {
            "protocol_version": SKY_CUA_HOST_PROTOCOL_VERSION,
            "client_role": CONTROL_PLANE_ROLE,
            "daemon_generation": daemon_generation.to_string(),
            "capabilities": capabilities,
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
