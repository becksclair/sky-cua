use super::super::*;
use super::*;
use crate::frame::read_frame;

#[test]
fn active_mutation_completion_returns_directly_and_remains_retained_until_ack() {
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
                &[
                    CONTROL_PLANE_CAPABILITY,
                    SETTLEMENTS_CAPABILITY,
                    SETTLEMENT_ACK_CAPABILITY,
                ],
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
    let settlement = read_frame(&mut peer).unwrap().unwrap();
    assert_eq!(settlement["method"], SKY_CUA_HOST_SETTLEMENT_METHOD);
    assert_eq!(settlement["params"]["operation_id"], "operation-1");
    {
        let state = state.lock().unwrap();
        assert!(state.pending_chrome_requests.is_empty());
        assert!(state.pending_id_tombstones.contains_key("chrome-normal"));
        assert_eq!(state.queued_settlements.len(), 1);
    }
    handle_client_message(&state, 1, settlement_ack(&settlement, "daemon-1"));
    assert!(state.lock().unwrap().queued_settlements.is_empty());
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
        state.queued_settlements[0].message["params"]["operation_id"],
        json!("operation-late")
    );
    assert_eq!(
        state.queued_settlements[0].message["params"]["completion"]["result"]["mutated"],
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
        &control_plane_hello(
            "new",
            "daemon-2",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
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
fn actor_absent_completion_remains_retained_until_acknowledged() {
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
        &control_plane_hello(
            "new",
            "daemon-2",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    assert_eq!(host.queued_settlements.len(), 1);
    assert_eq!(
        host.queued_settlements[0].message["params"]["operation_id"],
        json!("operation-held")
    );

    let state = Arc::new(Mutex::new(host));
    deliver_queued_settlements(&state);

    let settlement = read_frame(&mut new_peer).unwrap().unwrap();
    assert_eq!(
        settlement["params"]["operation_id"],
        json!("operation-held")
    );
    assert_eq!(state.lock().unwrap().queued_settlements.len(), 1);

    handle_client_message(&state, new_id, settlement_ack(&settlement, "daemon-2"));
    assert!(state.lock().unwrap().queued_settlements.is_empty());
}

#[test]
fn unacknowledged_settlement_is_retried_to_the_same_client() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-retry");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-retry",
            &json!("actor-request-retry"),
            &metadata,
            Some(json!({"jsonrpc":"2.0", "id":"chrome-retry", "result":true})),
        ),
    )
    .unwrap();
    let client_id = host.add_client(test_client().writer);
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    let first = host.begin_settlement_delivery().unwrap();
    let first_message = first.2.clone();
    host.finish_delivery(first.0, true);
    assert!(host.begin_settlement_delivery().is_none());

    host.settlement_delivered_at = Some(Instant::now() - SETTLEMENT_ACK_RETRY_INTERVAL);
    let retry = host.begin_settlement_delivery().unwrap();
    assert_eq!(retry.0.client_id, client_id);
    assert_eq!(retry.2, first_message);
}

#[test]
fn settlement_write_then_disconnect_replays_and_generation_safe_ack_clears_once() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-retained");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-retained",
            &json!("actor-request-retained"),
            &metadata,
            Some(json!({"jsonrpc":"2.0", "id":"chrome-retained", "result":true})),
        ),
    )
    .unwrap();
    let (mut first_peer, first_writer) = UnixStream::pair().unwrap();
    let first_id = host.add_client(Arc::new(Mutex::new(first_writer)));
    host.handle_host_hello(
        first_id,
        &control_plane_hello(
            "first",
            "daemon-2",
            &[
                CONTROL_PLANE_CAPABILITY,
                SETTLEMENTS_CAPABILITY,
                SETTLEMENT_ACK_CAPABILITY,
            ],
        ),
    );
    let state = Arc::new(Mutex::new(host));
    deliver_queued_settlements(&state);
    let first_delivery = read_frame(&mut first_peer).unwrap().unwrap();
    assert_eq!(state.lock().unwrap().queued_settlements.len(), 1);

    state.lock().unwrap().remove_client(first_id);
    drop(first_peer);
    let (mut second_peer, second_writer) = UnixStream::pair().unwrap();
    let second_id = state
        .lock()
        .unwrap()
        .add_client(Arc::new(Mutex::new(second_writer)));
    state.lock().unwrap().handle_host_hello(
        second_id,
        &control_plane_hello(
            "second",
            "daemon-3",
            &[
                CONTROL_PLANE_CAPABILITY,
                SETTLEMENTS_CAPABILITY,
                SETTLEMENT_ACK_CAPABILITY,
            ],
        ),
    );
    deliver_queued_settlements(&state);
    let replay = read_frame(&mut second_peer).unwrap().unwrap();
    assert_eq!(replay, first_delivery);

    handle_client_message(&state, second_id, settlement_ack(&replay, "daemon-stale"));
    assert_eq!(state.lock().unwrap().queued_settlements.len(), 1);
    let mut wrong_operation = settlement_ack(&replay, "daemon-3");
    wrong_operation["params"]["operation_id"] = json!("other-operation");
    handle_client_message(&state, second_id, wrong_operation);
    assert_eq!(state.lock().unwrap().queued_settlements.len(), 1);

    let ack = settlement_ack(&replay, "daemon-3");
    handle_client_message(&state, second_id, ack.clone());
    assert!(state.lock().unwrap().queued_settlements.is_empty());
    handle_client_message(&state, second_id, ack);
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
        state.queued_settlements[0].message["params"]["operation_id"],
        json!("operation-failed")
    );
    assert_eq!(
        state.queued_settlements[0].message["params"]["completion"]["result"]["mutated"],
        json!(true)
    );
}

#[test]
fn failed_hello_settlement_replay_keeps_ledger_entry() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-replay");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-replay",
            &json!("actor-request-replay"),
            &metadata,
            Some(json!({ "jsonrpc": "2.0", "id": "chrome-replay", "result": {"done": true} })),
        ),
    )
    .unwrap();
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    let outcome = host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    assert!(outcome.response.get("result").is_some());
    drop(peer);
    let state = Arc::new(Mutex::new(host));

    deliver_queued_settlements(&state);

    let state = state.lock().unwrap();
    assert_eq!(state.queued_settlements.len(), 1);
    assert_eq!(
        state.queued_settlements[0].message["params"]["operation_id"],
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
        state.queued_settlements[0].message["params"]["status"],
        json!("settlement_unknown")
    );
    assert!(state.queued_settlements[0].message["params"]["completion"].is_null());
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

#[test]
fn write_frame_obeys_total_deadline_under_partial_progress() {
    use std::io::Read;
    use std::os::unix::io::AsRawFd;

    let (mut peer, mut writer) = UnixStream::pair().unwrap();
    writer
        .set_write_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let small_send_buf = 8 * 1024;
    let set = unsafe {
        libc::setsockopt(
            writer.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            &small_send_buf as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert_eq!(set, 0, "SO_SNDBUF must be constrained for partial progress");

    let body = "x".repeat(512 * 1024);
    let message = json!({ "params": { "payload": body } });
    let start = Instant::now();
    let result = write_frame_until(
        &mut writer,
        &message,
        Instant::now() + Duration::from_millis(100),
    );
    assert!(result.is_err(), "deadline exceeded write must fail");
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "total deadline must bound the write"
    );

    // Prior write timeout restored on every exit path.
    assert_eq!(
        writer.write_timeout().unwrap(),
        Some(Duration::from_millis(20))
    );
    // Partial progress: the peer received a non-empty prefix of the frame.
    let mut buf = [0_u8; 16];
    assert!(peer.read(&mut buf).unwrap() > 0);
}

#[test]
fn writer_lock_contention_does_not_pin_delivery_in_progress() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-lock");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-lock",
            &json!("actor-request-lock"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    let state = Arc::new(Mutex::new(host));
    let held = state.lock().unwrap().clients[&client_id].writer.clone();
    let _held = held.lock().unwrap();
    deliver_queued_settlements(&state);
    drop(_held);
    drop(peer);

    let state = state.lock().unwrap();
    assert!(!state.settlement_delivery_in_progress);
    assert_eq!(state.queued_settlements.len(), 1);
}

#[test]
fn lock_contention_does_not_start_unknown_ack_grace() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-unknown-lock");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-unknown-lock",
            &json!("actor-request-unknown-lock"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    let state = Arc::new(Mutex::new(host));
    let held = state.lock().unwrap().clients[&client_id].writer.clone();
    let _held = held.lock().unwrap();
    deliver_queued_settlements(&state);
    drop(_held);
    drop(peer);

    let state = state.lock().unwrap();
    assert!(!state.settlement_delivery_in_progress);
    let SettlementPhase::Unknown {
        first_delivered_at, ..
    } = &state.queued_settlements[0].phase
    else {
        panic!("preexisting unknown must stay in the Unknown phase");
    };
    assert!(first_delivered_at.is_none());
}

#[test]
fn failed_write_does_not_start_unknown_ack_grace() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-failed-write");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-failed-write",
            &json!("actor-request-failed-write"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    drop(peer);

    let state = Arc::new(Mutex::new(host));
    deliver_queued_settlements(&state);

    let state = state.lock().unwrap();
    assert!(!state.settlement_delivery_in_progress);
    let SettlementPhase::Unknown {
        first_delivered_at, ..
    } = &state.queued_settlements[0].phase
    else {
        panic!("preexisting unknown must stay in the Unknown phase");
    };
    assert!(
        first_delivered_at.is_none(),
        "failed writes never start the ack grace"
    );
}

#[test]
fn phase_conversion_during_write_does_not_mark_unknown_delivered() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-convert-during-write");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-convert-during-write",
            &json!("actor-request-convert-during-write"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    let (lease, _writer, _message) = host.begin_settlement_delivery().unwrap();
    // Simulate a concurrent Original -> Unknown conversion bumping the revision.
    host.queued_settlements[0].phase = SettlementPhase::Unknown {
        first_delivered_at: None,
        hard_evict_at: host.queued_settlements[0].entered_at + SETTLEMENT_ENQUEUE_HARD_EVICT,
    };
    host.queued_settlements[0].phase_revision += 1;
    host.finish_delivery(lease, true);
    drop(peer);

    let SettlementPhase::Unknown {
        first_delivered_at, ..
    } = host.queued_settlements[0].phase
    else {
        panic!("converted entry must be in the Unknown phase");
    };
    assert!(
        first_delivered_at.is_none(),
        "a stale completion cannot mark the converted entry as delivered"
    );
}

#[test]
fn ack_pop_during_write_cannot_mark_next_front_delivered() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let first_metadata = test_settlement_metadata("operation-pop-1");
    let second_metadata = test_settlement_metadata("operation-pop-2");
    host.queue_settlement(
        first_metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-pop-1",
            &json!("actor-request-pop-1"),
            &first_metadata,
            None,
        ),
    )
    .unwrap();
    host.queue_settlement(
        second_metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-pop-2",
            &json!("actor-request-pop-2"),
            &second_metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    // A prior delivery succeeded, arming the acknowledgement path.
    let (first_lease, _, _) = host.begin_settlement_delivery().unwrap();
    host.finish_delivery(first_lease, true);
    host.settlement_delivered_at = Some(Instant::now() - SETTLEMENT_ACK_RETRY_INTERVAL);

    let (retry_lease, _, _) = host.begin_settlement_delivery().unwrap();
    let front_ack = settlement_ack(&host.queued_settlements[0].message, "daemon-1");
    assert!(host.acknowledge_settlement(client_id, &front_ack));
    assert_eq!(host.queued_settlements.len(), 1);
    assert_eq!(
        host.queued_settlements[0].message["params"]["operation_id"],
        json!("operation-pop-2")
    );
    host.finish_delivery(retry_lease, true);
    drop(peer);

    let SettlementPhase::Unknown {
        first_delivered_at, ..
    } = host.queued_settlements[0].phase
    else {
        panic!("second entry must be in the Unknown phase");
    };
    assert!(
        first_delivered_at.is_none(),
        "a stale completion cannot mark the next front as delivered"
    );
}

#[test]
fn delivery_guard_clears_in_progress_on_all_error_paths() {
    let state = Arc::new(Mutex::new(test_host_state()));
    // A newer delivery superseded this one: the stale guard must not clear it.
    state.lock().unwrap().settlement_delivery_in_progress = true;
    state.lock().unwrap().delivery_seq = 1;
    drop(DeliveryGuard {
        state: Arc::clone(&state),
        delivery_seq: 0,
    });
    assert!(state.lock().unwrap().settlement_delivery_in_progress);

    // Same-sequence guard clears the transient flag and nothing else.
    state.lock().unwrap().settlement_delivery_in_progress = true;
    drop(DeliveryGuard {
        state: Arc::clone(&state),
        delivery_seq: 1,
    });
    assert!(!state.lock().unwrap().settlement_delivery_in_progress);
}

#[test]
fn acknowledgement_clears_first_attempted_at() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-ack-clears-throttle");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-ack-clears-throttle",
            &json!("actor-request-ack-clears-throttle"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    let (lease, _, _) = host.begin_settlement_delivery().unwrap();
    host.finish_delivery(lease, true);
    assert_eq!(host.settlement_delivered_to, Some(client_id));

    let ack = settlement_ack(&host.queued_settlements[0].message, "daemon-1");
    assert!(host.acknowledge_settlement(client_id, &ack));
    assert!(host.settlement_delivered_to.is_none());
    assert!(host.settlement_delivered_at.is_none());
    drop(peer);
}

#[test]
fn delivery_deadline_converts_unacked_settlement_to_unknown_after_deadline() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-deadline-convert");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-deadline-convert",
            &json!("actor-request-deadline-convert"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let entered = host.queued_settlements[0].entered_at;

    host.evict_settlements_at(entered + SETTLEMENT_ORIGINAL_TO_UNKNOWN - Duration::from_millis(1));
    let SettlementPhase::Original { .. } = host.queued_settlements[0].phase else {
        panic!("conversion must not fire before the deadline");
    };

    host.evict_settlements_at(entered + SETTLEMENT_ORIGINAL_TO_UNKNOWN + Duration::from_millis(1));
    assert_eq!(host.queued_settlements.len(), 1);
    let SettlementPhase::Unknown {
        first_delivered_at, ..
    } = host.queued_settlements[0].phase
    else {
        panic!("settlement must convert to Unknown after the deadline");
    };
    assert!(first_delivered_at.is_none());
    assert_eq!(
        host.queued_settlements[0].message["params"]["status"],
        json!("settlement_unknown")
    );
    assert_eq!(host.queued_settlements[0].phase_revision, 1);
    assert_eq!(
        host.queued_settlements[0].message["params"]["operation_id"],
        json!("operation-deadline-convert")
    );
}

#[test]
fn delivery_deadline_pops_after_unknown_delivery_deadline() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-deadline-pop");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-deadline-pop",
            &json!("actor-request-deadline-pop"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let entered = host.queued_settlements[0].entered_at;

    host.evict_settlements_at(entered + SETTLEMENT_ENQUEUE_HARD_EVICT + Duration::from_millis(1));
    assert!(host.queued_settlements.is_empty());
    assert!(host.settlement_capacity_available());
}

#[test]
fn delivery_deadline_does_not_fire_within_window() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-deadline-window");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-deadline-window",
            &json!("actor-request-deadline-window"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let entered = host.queued_settlements[0].entered_at;

    host.evict_settlements_at(entered + Duration::from_secs(5));
    assert_eq!(host.queued_settlements.len(), 1);
    let SettlementPhase::Original { .. } = host.queued_settlements[0].phase else {
        panic!("settlement must stay Original within the conversion window");
    };
}

#[test]
fn preexisting_unknown_enters_unknown_phase() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-preexisting-unknown");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-preexisting-unknown",
            &json!("actor-request-preexisting-unknown"),
            &metadata,
            None,
        ),
    )
    .unwrap();

    let SettlementPhase::Unknown {
        first_delivered_at,
        hard_evict_at,
    } = host.queued_settlements[0].phase
    else {
        panic!("preexisting settlement_unknown must enter the Unknown phase");
    };
    assert!(first_delivered_at.is_none());
    assert_eq!(
        hard_evict_at,
        host.queued_settlements[0].entered_at + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP
    );
}

#[test]
fn unknown_timer_is_not_reset_by_repeated_cleanup() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-repeated-cleanup");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-repeated-cleanup",
            &json!("actor-request-repeated-cleanup"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let entered = host.queued_settlements[0].entered_at;
    let now = entered + Duration::from_secs(5);

    host.evict_settlements_at(now);
    host.evict_settlements_at(now);
    host.evict_settlements_at(now);
    assert_eq!(host.queued_settlements.len(), 1);

    host.evict_settlements_at(
        entered + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP + Duration::from_millis(1),
    );
    assert!(host.queued_settlements.is_empty());
}

#[test]
fn unknown_receives_post_delivery_ack_window() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-post-delivery-window");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-post-delivery-window",
            &json!("actor-request-post-delivery-window"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let entered = host.queued_settlements[0].entered_at;
    let delivered_at = entered + Duration::from_millis(100);
    host.queued_settlements[0].phase = SettlementPhase::Unknown {
        first_delivered_at: Some(delivered_at),
        hard_evict_at: entered + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP,
    };

    host.evict_settlements_at(delivered_at + Duration::from_secs(3));
    assert_eq!(
        host.queued_settlements.len(),
        1,
        "the post-delivery ack window must not evict within the grace"
    );

    host.evict_settlements_at(
        delivered_at + SETTLEMENT_UNKNOWN_MAX_POST_DELIVERY_GRACE + Duration::from_millis(1),
    );
    assert!(
        host.queued_settlements.is_empty(),
        "eviction fires once the post-delivery grace expires"
    );
}

#[test]
fn unknown_hard_cap_truncates_late_post_delivery_grace() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-cap-truncates-grace");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-cap-truncates-grace",
            &json!("actor-request-cap-truncates-grace"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let entered = host.queued_settlements[0].entered_at;
    // Delivery lands just before the hard cap; the grace would end at +4s but
    // the absolute cap must evict first.
    let delivered_at =
        entered + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP - Duration::from_millis(100);
    host.queued_settlements[0].phase = SettlementPhase::Unknown {
        first_delivered_at: Some(delivered_at),
        hard_evict_at: entered + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP,
    };

    host.evict_settlements_at(entered + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP);
    assert!(
        host.queued_settlements.is_empty(),
        "the hard cap truncates a late post-delivery grace"
    );
}

#[test]
fn unknown_hard_cap_evicts_when_no_control_plane_connects() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-no-control-plane");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-no-control-plane",
            &json!("actor-request-no-control-plane"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let entered = host.queued_settlements[0].entered_at;

    host.evict_settlements_at(
        entered + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP + Duration::from_millis(1),
    );
    assert!(
        host.queued_settlements.is_empty(),
        "an Unknown must evict by its hard cap even with no control plane"
    );
}

#[test]
fn late_original_ack_during_unknown_phase_is_safe() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-late-ack-safe");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-late-ack-safe",
            &json!("actor-request-late-ack-safe"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    let (lease, _, _) = host.begin_settlement_delivery().unwrap();
    host.finish_delivery(lease, true);

    let entered = host.queued_settlements[0].entered_at;
    host.evict_settlements_at(entered + SETTLEMENT_ORIGINAL_TO_UNKNOWN + Duration::from_millis(1));
    let SettlementPhase::Unknown { .. } = host.queued_settlements[0].phase else {
        panic!("front must have converted to Unknown");
    };

    let ack = settlement_ack(&host.queued_settlements[0].message, "daemon-1");
    assert!(
        host.acknowledge_settlement(client_id, &ack),
        "a late original ack matches the converted front's preserved identity"
    );
    assert!(host.queued_settlements.is_empty());
    drop(peer);
}

#[test]
fn late_ack_after_eviction_cannot_pop_the_next_front() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let first_metadata = test_settlement_metadata("operation-evicted-front");
    let second_metadata = test_settlement_metadata("operation-next-front");
    host.queue_settlement(
        first_metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-evicted-front",
            &json!("actor-request-evicted-front"),
            &first_metadata,
            None,
        ),
    )
    .unwrap();
    host.queue_settlement(
        second_metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-next-front",
            &json!("actor-request-next-front"),
            &second_metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    let (lease, _, _) = host.begin_settlement_delivery().unwrap();
    host.finish_delivery(lease, true);
    let evicted_message = host.queued_settlements[0].message.clone();

    // Keep the second entry well past the eviction window so the pass pops only
    // the expired front.
    host.queued_settlements[1].phase = SettlementPhase::Unknown {
        first_delivered_at: None,
        hard_evict_at: host.queued_settlements[1].entered_at + Duration::from_secs(60),
    };

    let entered = host.queued_settlements[0].entered_at;
    host.evict_settlements_at(
        entered + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP + Duration::from_millis(1),
    );
    assert_eq!(host.queued_settlements.len(), 1);

    let late_ack = settlement_ack(&evicted_message, "daemon-1");
    assert!(!host.acknowledge_settlement(client_id, &late_ack));
    assert_eq!(host.queued_settlements.len(), 1);
    assert_eq!(
        host.queued_settlements[0].message["params"]["operation_id"],
        json!("operation-next-front")
    );
    drop(peer);
}

#[test]
fn wrong_identity_ack_does_not_clear_front_timing_state() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-wrong-identity-ack");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-wrong-identity-ack",
            &json!("actor-request-wrong-identity-ack"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    let (lease, _, _) = host.begin_settlement_delivery().unwrap();
    host.finish_delivery(lease, true);

    let mut wrong_ack = settlement_ack(&host.queued_settlements[0].message, "daemon-1");
    wrong_ack["params"]["operation_id"] = json!("operation-someone-else");
    assert!(!host.acknowledge_settlement(client_id, &wrong_ack));
    assert_eq!(host.queued_settlements.len(), 1);
    let SettlementPhase::Unknown {
        first_delivered_at: _,
        hard_evict_at,
    } = host.queued_settlements[0].phase
    else {
        panic!("front must remain in the Unknown phase");
    };
    let _ = hard_evict_at;
    drop(peer);
}

#[test]
fn new_generation_promotion_converts_all_prior_generation_entries() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    for index in 0..3 {
        let metadata = test_settlement_metadata(&format!("operation-prior-{index}"));
        host.queue_settlement(
            metadata.clone(),
            settlement_message(
                "completed",
                &format!("chrome-prior-{index}"),
                &json!(format!("actor-request-prior-{index}")),
                &metadata,
                None,
            ),
        )
        .unwrap();
    }
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-2",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    for entry in &host.queued_settlements {
        let SettlementPhase::Unknown {
            first_delivered_at, ..
        } = entry.phase
        else {
            panic!("every prior-generation Original must convert on promotion");
        };
        assert!(first_delivered_at.is_none());
        assert_eq!(entry.phase_revision, 1);
        assert_eq!(
            entry.message["params"]["status"],
            json!("settlement_unknown")
        );
    }
    drop(peer);
}

#[test]
fn prior_generation_queue_drains_without_per_entry_15s_delay() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    for index in 0..5 {
        let metadata = test_settlement_metadata(&format!("operation-drain-{index}"));
        host.queue_settlement(
            metadata.clone(),
            settlement_message(
                "completed",
                &format!("chrome-drain-{index}"),
                &json!(format!("actor-request-drain-{index}")),
                &metadata,
                None,
            ),
        )
        .unwrap();
    }
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-2",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    for entry in &host.queued_settlements {
        let SettlementPhase::Unknown { .. } = entry.phase else {
            panic!("supersession must convert the whole queue immediately");
        };
    }

    let entered = host.queued_settlements[0].entered_at;
    host.evict_settlements_at(entered + SETTLEMENT_ENQUEUE_HARD_EVICT + Duration::from_millis(1));
    assert!(
        host.queued_settlements.is_empty(),
        "converted prior-generation entries drain by their absolute caps in one pass"
    );
    drop(peer);
}

#[test]
fn saturated_settlement_queue_restores_capacity_under_persistent_ack_loss() {
    let mut host = test_host_state();
    for index in 0..MAX_RETAINED_SETTLEMENTS {
        let metadata = test_settlement_metadata(&format!("operation-saturation-{index}"));
        host.queue_settlement(
            metadata.clone(),
            settlement_message(
                "completed",
                &format!("chrome-saturation-{index}"),
                &json!(format!("actor-request-saturation-{index}")),
                &metadata,
                None,
            ),
        )
        .unwrap();
    }
    assert_eq!(host.queued_settlements.len(), MAX_RETAINED_SETTLEMENTS);
    assert!(!host.settlement_capacity_available());
    assert!(
        host.queue_settlement(
            test_settlement_metadata("operation-over-capacity"),
            settlement_message(
                "completed",
                "chrome-over-capacity",
                &json!("actor-request-over-capacity"),
                &test_settlement_metadata("operation-over-capacity"),
                None,
            ),
        )
        .is_err()
    );

    let entered = host
        .queued_settlements
        .iter()
        .map(|entry| entry.entered_at)
        .max()
        .expect("saturated queue is non-empty");
    let now = entered + SETTLEMENT_ENQUEUE_HARD_EVICT + Duration::from_millis(1);
    for _ in 0..(MAX_RETAINED_SETTLEMENTS / MAX_SETTLEMENT_EVICTIONS_PER_TICK + 1) {
        host.evict_settlements_at(now);
    }

    assert!(
        host.queued_settlements.is_empty(),
        "all 100 entries must be gone by their individual hard deadlines"
    );
    assert!(host.settlement_capacity_available());
}

#[test]
fn evicted_unknown_does_not_pin_strict_owner_or_settlement_capacity() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    let mut hello = control_plane_hello(
        "strict",
        "daemon-1",
        &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
    );
    hello["params"]["owner_mode"] = json!("strict");
    host.handle_host_hello(client_id, &hello);
    assert_eq!(host.owner_mode, OwnerMode::Strict);

    let metadata = test_settlement_metadata("operation-strict-evict");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "settlement_unknown",
            "chrome-strict-evict",
            &json!("actor-request-strict-evict"),
            &metadata,
            None,
        ),
    )
    .unwrap();

    let entered = host.queued_settlements[0].entered_at;
    host.evict_settlements_at(
        entered + SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP + Duration::from_millis(1),
    );

    assert!(host.queued_settlements.is_empty());
    assert!(host.settlement_capacity_available());
    let release = host.handle_owner_release(client_id, &owner_release("release", "daemon-1"));
    assert!(
        release.get("result").is_some(),
        "evicted unknowns must not pin the strict owner release"
    );
    drop(peer);
}

#[test]
fn fence_unresponsive_control_plane_drops_active_control_plane_when_queue_nonempty() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-fence");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-fence",
            &json!("actor-request-fence"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    let now = host.clients[&client_id].last_seen_at
        + CONTROL_PLANE_LIVENESS_DEADLINE
        + Duration::from_millis(1);

    host.fence_unresponsive_control_plane_at(now);

    assert!(host.clients[&client_id].close_requested);
    assert_eq!(host.queued_settlements.len(), 1);
    drop(peer);
}

#[test]
fn fence_unresponsive_control_plane_no_op_when_queue_empty() {
    let mut host = test_host_state();
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    let now = host.clients[&client_id].last_seen_at
        + CONTROL_PLANE_LIVENESS_DEADLINE
        + Duration::from_millis(1);

    host.fence_unresponsive_control_plane_at(now);

    assert!(!host.clients[&client_id].close_requested);
    drop(peer);
}

#[test]
fn fence_unresponsive_control_plane_no_op_when_last_seen_fresh() {
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-fence-fresh");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-fence-fresh",
            &json!("actor-request-fence-fresh"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    host.fence_unresponsive_control_plane_at(Instant::now());

    assert!(!host.clients[&client_id].close_requested);
    drop(peer);
}

#[test]
fn fence_then_reader_eof_then_same_generation_hello_accepts() {
    let (peer, writer_stream) = UnixStream::pair().unwrap();
    let mut host = test_host_state();
    let metadata = test_settlement_metadata("operation-fence-reconnect");
    host.queue_settlement(
        metadata.clone(),
        settlement_message(
            "completed",
            "chrome-fence-reconnect",
            &json!("actor-request-fence-reconnect"),
            &metadata,
            None,
        ),
    )
    .unwrap();
    let client_id = host.add_client(Arc::new(Mutex::new(writer_stream)));
    host.handle_host_hello(
        client_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );

    // Fence fires: the control plane is close-requested.
    let now = host.clients[&client_id].last_seen_at
        + CONTROL_PLANE_LIVENESS_DEADLINE
        + Duration::from_millis(1);
    host.fence_unresponsive_control_plane_at(now);
    assert!(host.clients[&client_id].close_requested);

    // Simulate the reader thread observing EOF and running remove_client.
    host.remove_client(client_id);
    assert!(host.active_control_plane().is_none());

    // A new client connects with the same daemon generation.
    let (new_peer, new_writer) = UnixStream::pair().unwrap();
    let new_id = host.add_client(Arc::new(Mutex::new(new_writer)));
    let outcome = host.handle_host_hello(
        new_id,
        &control_plane_hello(
            "hello",
            "daemon-1",
            &[CONTROL_PLANE_CAPABILITY, SETTLEMENT_ACK_CAPABILITY],
        ),
    );
    assert!(
        outcome.response.get("result").is_some(),
        "same-generation reconnect must succeed after the old client is removed"
    );
    assert_eq!(
        host.clients[&new_id].daemon_generation.as_deref(),
        Some("daemon-1")
    );
    drop(peer);
    drop(new_peer);
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
        shutdown_handle: stream.try_clone().ok().map(Arc::new),
        writer: Arc::new(Mutex::new(stream)),
        role: ClientRole::Unknown,
        daemon_generation: None,
        capabilities: HashSet::new(),
        connected_at: Instant::now(),
        last_seen_at: Instant::now(),
        close_requested: false,
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

fn settlement_ack(settlement: &Value, acknowledging_daemon_generation: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": SKY_CUA_HOST_SETTLEMENT_ACK_METHOD,
        "params": {
            "operation_id": settlement["params"]["operation_id"],
            "daemon_generation": settlement["params"]["daemon_generation"],
            "actor_generation": settlement["params"]["actor_generation"],
            "chrome_request_id": settlement["params"]["chrome_request_id"],
            "acknowledging_daemon_generation": acknowledging_daemon_generation,
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
