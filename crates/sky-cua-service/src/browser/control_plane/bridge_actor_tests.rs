use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::net::{UnixListener, UnixStream};

use super::bridge_actor::{
    BridgeOwnerMode, exercise_stalled_write, exercise_write_failure, request_frame_for_test,
};
use super::{
    BridgeActor, BridgeActorConfig, BridgeActorError, BridgeActorEvent, BridgeActorRequest,
    BridgeActorState, BridgeRequestSize, OperationClass, fixed_width_daemon_generation,
};
use crate::browser::protocol::{HOST_HELLO_METHOD, MAX_FRAME_SIZE, read_frame, write_frame};
use crate::browser::sockets::{
    BrowserSocketSelection, find_bridge_sockets, record_persistent_actor_health,
    reset_socket_inventory_for_tests,
};

const CAPABILITIES: &[&str] = &[
    "control_plane",
    "heartbeat",
    "extension_events",
    "private_param_stripping",
    "settlements",
    "settlement_ack",
    "side_panel_requests",
    "owner_release",
];

struct SocketFixture {
    dir: PathBuf,
    path: PathBuf,
}

impl SocketFixture {
    fn new(name: &str) -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-bridge-actor-{name}-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("extension-123-actor.sock");
        Self { dir, path }
    }
}

impl Drop for SocketFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn config(path: &Path, generation: u64) -> BridgeActorConfig {
    BridgeActorConfig {
        socket_path: path.to_path_buf(),
        daemon_generation: fixed_width_daemon_generation(),
        actor_generation: generation,
        owner_mode: BridgeOwnerMode::Hybrid,
        connect_timeout: Duration::from_millis(200),
        handshake_timeout: Duration::from_millis(200),
        write_timeout: Duration::from_millis(200),
        heartbeat_interval: Duration::from_secs(10),
        reconnect_min: Duration::from_millis(10),
        reconnect_max: Duration::from_millis(25),
    }
}

#[test]
fn mutating_host_mapping_outlives_actor_timeout_through_settlement_window() {
    let config = config(Path::new("/tmp/unused-settlement-frame.sock"), 1);
    let mut request = BridgeActorRequest::new(
        "executeCdp",
        json!({"method":"Page.navigate"}),
        "settlement-window-operation",
        OperationClass::Mutation,
    );
    request.timeout = Duration::from_millis(250);
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let frame = request_frame_for_test(&config, &request);
    let deadline = frame["params"]["_sky_cua_host_request"]["settlement_deadline_ms"]
        .as_u64()
        .unwrap();
    assert!(deadline >= before + 250 + super::SETTLEMENT_DEADLINE_MS);
}

async fn accept_hello(listener: &UnixListener, host: &str) -> UnixStream {
    accept_hello_with_owner_mode(
        listener,
        host,
        BridgeOwnerMode::Hybrid,
        BridgeOwnerMode::Hybrid,
    )
    .await
}

async fn accept_stable_hello(listener: &UnixListener, host: &str, browser: &str) -> UnixStream {
    let (mut stream, _) = listener.accept().await.unwrap();
    let hello = read_frame(&mut stream).await.unwrap().unwrap();
    write_frame(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": hello["id"],
            "result": {
                "protocol_version": 1,
                "host_instance_id": host,
                "browser_instance_id": browser,
                "browser_instance_stability": "stable",
                "browser_family": "test",
                "mode": "hybrid",
                "owner_mode": "hybrid",
                "capabilities": CAPABILITIES,
            }
        }),
    )
    .await
    .unwrap();
    stream
}

async fn accept_connection_only_hello(
    listener: &UnixListener,
    host: &str,
    browser: &str,
) -> UnixStream {
    let (mut stream, _) = listener.accept().await.unwrap();
    let hello = read_frame(&mut stream).await.unwrap().unwrap();
    write_frame(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": hello["id"],
            "result": {
                "protocol_version": 1,
                "host_instance_id": host,
                "browser_instance_id": browser,
                "browser_instance_stability": "connection_only",
                "browser_family": "test",
                "mode": "hybrid",
                "owner_mode": "hybrid",
                "capabilities": CAPABILITIES,
            }
        }),
    )
    .await
    .unwrap();
    stream
}

async fn accept_hello_with_owner_mode(
    listener: &UnixListener,
    host: &str,
    requested_owner_mode: BridgeOwnerMode,
    returned_owner_mode: BridgeOwnerMode,
) -> UnixStream {
    let (mut stream, _) = listener.accept().await.unwrap();
    let hello = read_frame(&mut stream).await.unwrap().unwrap();
    assert_eq!(hello["method"], HOST_HELLO_METHOD);
    assert_eq!(hello["params"]["client_role"], "control_plane");
    assert_eq!(hello["params"]["owner_mode"], requested_owner_mode.as_str());
    assert_eq!(hello["params"]["capabilities"], json!(CAPABILITIES));
    let generation = hello["params"]["daemon_generation"].as_str().unwrap();
    assert_eq!(generation.len(), 50);
    write_frame(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": hello["id"],
            "result": {
                "protocol_version": 1,
                "host_instance_id": host,
                "browser_instance_id": null,
                "browser_instance_stability": "unavailable",
                "browser_family": "test",
                "mode": returned_owner_mode.as_str(),
                "owner_mode": returned_owner_mode.as_str(),
                "capabilities": CAPABILITIES,
            }
        }),
    )
    .await
    .unwrap();
    stream
}

async fn assert_owner_mode_is_accepted(owner_mode: BridgeOwnerMode, generation: u64) {
    let fixture = SocketFixture::new(owner_mode.as_str());
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream =
            accept_hello_with_owner_mode(&listener, "host-owner-mode", owner_mode, owner_mode)
                .await;
        if owner_mode == BridgeOwnerMode::Strict {
            let release = read_operation(&mut stream).await;
            assert_eq!(release["method"], "skyCuaHost/release");
            assert_eq!(release["params"]["owner_mode"], "hybrid");
            write_frame(
                &mut stream,
                &json!({
                    "jsonrpc": "2.0",
                    "id": release["id"],
                    "result": { "released": true, "owner_mode": "hybrid" },
                }),
            )
            .await
            .unwrap();
        } else {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    });
    let mut actor_config = config(&fixture.path, generation);
    actor_config.owner_mode = owner_mode;
    let mut actor = BridgeActor::spawn(actor_config);
    actor.wait_until_ready().await.unwrap();
    actor.shutdown().await;
    server.await.unwrap();
}

async fn read_operation(stream: &mut UnixStream) -> Value {
    loop {
        let frame = read_frame(stream).await.unwrap().unwrap();
        if frame["method"] == "ping" {
            write_frame(
                stream,
                &json!({"jsonrpc":"2.0", "id":frame["id"], "result":"pong"}),
            )
            .await
            .unwrap();
            continue;
        }
        return frame;
    }
}

fn request(name: &str) -> BridgeActorRequest {
    BridgeActorRequest::new(
        name,
        json!({"value": name}),
        format!("operation-{name}"),
        OperationClass::ReadOnly,
    )
}

async fn ready_actor(path: &Path, generation: u64) -> BridgeActor {
    let mut actor = BridgeActor::spawn(config(path, generation));
    let health = tokio::time::timeout(Duration::from_secs(1), actor.wait_until_ready())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(health.state, BridgeActorState::Ready);
    assert_eq!(
        health.browser_instance_stability,
        sky_cua_platform::model::BrowserInstanceStability::ConnectionOnly
    );
    assert!(health.peer_pid.is_some());
    assert!(health.peer_start_ticks.is_some());
    assert!(health.boot_id.is_some());
    actor
}

#[test]
fn daemon_generation_is_fixed_width_and_time_ordered() {
    let first = fixed_width_daemon_generation();
    let second = fixed_width_daemon_generation();
    assert_eq!(first.len(), 50);
    assert_eq!(second.len(), 50);
    assert!(second > first);
}

#[test]
fn bridge_actor_config_defaults_to_hybrid_owner_mode() {
    let config = BridgeActorConfig::new(PathBuf::from("bridge.sock"), 1);
    assert_eq!(config.owner_mode, BridgeOwnerMode::Hybrid);
}

#[test]
fn host_edge_frame_bound_matches_native_host_without_changing_image_payloads() {
    assert_eq!(MAX_FRAME_SIZE, 100 * 1024 * 1024);
}

#[tokio::test]
async fn host_owner_mode_contract_accepts_both_modes_and_quarantines_mismatch() {
    assert_owner_mode_is_accepted(BridgeOwnerMode::Hybrid, 20).await;
    assert_owner_mode_is_accepted(BridgeOwnerMode::Strict, 21).await;

    let fixture = SocketFixture::new("owner-mode-mismatch");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let _stream = accept_hello_with_owner_mode(
            &listener,
            "host-owner-mode-mismatch",
            BridgeOwnerMode::Strict,
            BridgeOwnerMode::Hybrid,
        )
        .await;
        tokio::time::sleep(Duration::from_millis(25)).await;
    });
    let mut actor_config = config(&fixture.path, 22);
    actor_config.owner_mode = BridgeOwnerMode::Strict;
    let mut actor = BridgeActor::spawn(actor_config);
    let error = actor.wait_until_ready().await.unwrap_err();
    assert!(matches!(
        error,
        BridgeActorError::Unavailable(reason)
            if reason.contains(
                "native host owner mode mismatch: requested strict, negotiated hybrid"
            )
    ));
    assert_eq!(actor.health().state, BridgeActorState::Quarantined);
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn strict_shutdown_does_not_release_with_unresolved_mutation_tombstone() {
    let fixture = SocketFixture::new("strict-unresolved-tombstone");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello_with_owner_mode(
            &listener,
            "host-strict-unresolved",
            BridgeOwnerMode::Strict,
            BridgeOwnerMode::Strict,
        )
        .await;
        let mutation = read_operation(&mut stream).await;
        assert_eq!(mutation["method"], "mutate");
        let after_shutdown =
            tokio::time::timeout(Duration::from_millis(200), read_frame(&mut stream)).await;
        assert!(
            matches!(after_shutdown, Ok(Ok(None)) | Ok(Err(_))),
            "strict owner release must not be sent with an unresolved mutation tombstone: {after_shutdown:?}"
        );
    });
    let mut actor_config = config(&fixture.path, 27);
    actor_config.owner_mode = BridgeOwnerMode::Strict;
    let mut actor = BridgeActor::spawn(actor_config);
    actor.wait_until_ready().await.unwrap();
    let mut mutation = BridgeActorRequest::new(
        "mutate",
        json!({}),
        "strict-unresolved-operation",
        OperationClass::Mutation,
    );
    mutation.timeout = Duration::from_millis(25);
    assert_eq!(
        actor.request(mutation).await.unwrap_err(),
        BridgeActorError::Ambiguous
    );
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn healthy_actor_socket_is_preferred_over_merely_newer_candidate() {
    let fixture = SocketFixture::new("healthy-preference");
    let older = fixture.dir.join("extension-111-older.sock");
    let newer = fixture.dir.join("extension-222-newer.sock");
    let _older_listener = UnixListener::bind(&older).unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    let _newer_listener = UnixListener::bind(&newer).unwrap();
    reset_socket_inventory_for_tests();
    // SAFETY: nextest runs this test in its own process.
    unsafe { std::env::set_var("SKY_CUA_BROWSER_USE_SOCKET_DIR", &fixture.dir) };
    record_persistent_actor_health(&older, true);

    let candidates = find_bridge_sockets(BrowserSocketSelection::All);
    assert_eq!(candidates.first(), Some(&older));

    record_persistent_actor_health(&older, false);
    reset_socket_inventory_for_tests();
    // SAFETY: nextest runs this test in its own process.
    unsafe { std::env::remove_var("SKY_CUA_BROWSER_USE_SOCKET_DIR") };
}

#[tokio::test]
async fn canonical_hello_requires_side_panel_requests_capability() {
    let fixture = SocketFixture::new("hello-mismatch");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let hello = read_frame(&mut stream).await.unwrap().unwrap();
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc":"2.0",
                "id": hello["id"],
                "result": {
                    "protocol_version":1,
                    "mode":"hybrid",
                    "host_instance_id":"host-mismatch",
                    "browser_instance_id":null,
                    "browser_instance_stability":"unavailable",
                    "capabilities":[
                        "control_plane",
                        "heartbeat",
                        "extension_events",
                        "private_param_stripping",
                        "settlements",
                        "settlement_ack",
                        "owner_release"
                    ]
                }
            }),
        )
        .await
        .unwrap();
    });
    let mut actor = BridgeActor::spawn(config(&fixture.path, 1));
    let error = actor.wait_until_ready().await.unwrap_err();
    assert!(matches!(
        error,
        BridgeActorError::Unavailable(reason)
            if reason.contains("native host capability mismatch; missing side_panel_requests")
    ));
    assert_eq!(actor.health().state, BridgeActorState::Quarantined);
    actor.shutdown().await;
    server.await.unwrap();
}

async fn assert_write_failure_settlement(
    operation_class: OperationClass,
    bytes_before_failure: usize,
    expected: BridgeActorError,
) {
    let outcome = exercise_write_failure(
        &config(Path::new("unused.sock"), 23),
        BridgeActorRequest::new(
            "dispatch",
            json!({"payload": "test"}),
            "uncertain-operation",
            operation_class,
        ),
        bytes_before_failure,
    )
    .await;

    assert_eq!(outcome.0, expected);
    assert_eq!(outcome.1, 1, "work must be pending before write");
    assert_eq!(outcome.2.as_deref(), Some("uncertain-operation"));
    assert_eq!(outcome.3, Some(operation_class));
    assert_eq!(outcome.4, bytes_before_failure);
}

#[tokio::test]
async fn failed_and_partial_writes_settle_dispatched_work_without_replay() {
    assert_write_failure_settlement(OperationClass::Mutation, 0, BridgeActorError::Ambiguous).await;
    // Four header bytes plus one body byte prove that the partial-frame path
    // receives the same ambiguous treatment for global work.
    assert_write_failure_settlement(
        OperationClass::BrowserGlobal,
        5,
        BridgeActorError::Ambiguous,
    )
    .await;
    assert_write_failure_settlement(OperationClass::ReadOnly, 5, BridgeActorError::Disconnected)
        .await;
}

#[tokio::test]
async fn stalled_dispatch_write_times_out_and_preserves_ambiguous_mutation() {
    let mut actor_config = config(Path::new("unused.sock"), 24);
    actor_config.write_timeout = Duration::from_millis(20);
    let error = exercise_stalled_write(
        &actor_config,
        BridgeActorRequest::new(
            "dispatch",
            json!({"payload": "test"}),
            "stalled-mutation",
            OperationClass::Mutation,
        ),
    )
    .await;

    assert_eq!(error, BridgeActorError::Ambiguous);
}

#[tokio::test]
async fn settlement_ack_copies_retained_identity_and_uses_selected_daemon_generation() {
    let fixture = SocketFixture::new("settlement-ack");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let actor_config = config(&fixture.path, 26);
    let expected_generation = actor_config.daemon_generation.clone();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-settlement-ack").await;
        let ack = read_operation(&mut stream).await;
        assert_eq!(ack["method"], "skyCuaHost/settlementAck");
        assert_eq!(ack["params"]["operation_id"], "operation-retained");
        assert_eq!(ack["params"]["daemon_generation"], "daemon-old");
        assert_eq!(ack["params"]["actor_generation"], 7);
        assert_eq!(ack["params"]["chrome_request_id"], "chrome-retained");
        assert_eq!(
            ack["params"]["acknowledging_daemon_generation"],
            expected_generation
        );
    });
    let mut actor = BridgeActor::spawn(actor_config);
    actor.wait_until_ready().await.unwrap();
    actor
        .acknowledge_settlement(&json!({
            "jsonrpc":"2.0",
            "method":"skyCuaHost/settlement",
            "params":{
                "operation_id":"operation-retained",
                "daemon_generation":"daemon-old",
                "actor_generation":7,
                "chrome_request_id":"chrome-retained",
            }
        }))
        .await
        .unwrap();
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn out_of_order_replies_correlate_to_unique_monotonic_ids() {
    let fixture = SocketFixture::new("out-of-order");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-order").await;
        let first = read_operation(&mut stream).await;
        let second = read_operation(&mut stream).await;
        assert_ne!(first["id"], second["id"]);
        assert!(first["id"].as_str().unwrap() < second["id"].as_str().unwrap());
        assert_eq!(first["params"]["session_id"], "sky-cua-control-plane-v1");
        assert_eq!(first["params"]["turn_id"], "control-plane-lease-v1");
        assert_eq!(
            first["params"]["_sky_cua_host_request"]["operation_id"],
            "operation-first"
        );
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":second["id"], "result":"second-result"}),
        )
        .await
        .unwrap();
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":first["id"], "result":"first-result"}),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(25)).await;
    });
    let actor = ready_actor(&fixture.path, 2).await;
    let first_actor = actor.clone();
    let first = tokio::spawn(async move { first_actor.request(request("first")).await });
    let second_actor = actor.clone();
    let second = tokio::spawn(async move { second_actor.request(request("second")).await });
    assert_eq!(first.await.unwrap().unwrap()["result"], "first-result");
    assert_eq!(second.await.unwrap().unwrap()["result"], "second-result");
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn permits_two_ordinary_requests_before_either_completes() {
    let fixture = SocketFixture::new("two-ordinary");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-two").await;
        let first = read_operation(&mut stream).await;
        let second = tokio::time::timeout(Duration::from_millis(100), read_operation(&mut stream))
            .await
            .expect("second ordinary request should overlap");
        for frame in [first, second] {
            write_frame(
                &mut stream,
                &json!({"jsonrpc":"2.0", "id":frame["id"], "result":true}),
            )
            .await
            .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    });
    let actor = ready_actor(&fixture.path, 3).await;
    let a = tokio::spawn({
        let actor = actor.clone();
        async move { actor.request(request("a")).await }
    });
    let b = tokio::spawn({
        let actor = actor.clone();
        async move { actor.request(request("b")).await }
    });
    a.await.unwrap().unwrap();
    b.await.unwrap().unwrap();
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn large_frame_request_runs_exclusively() {
    let fixture = SocketFixture::new("large-exclusive");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-large").await;
        let large = read_operation(&mut stream).await;
        assert_eq!(large["method"], "large");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), read_operation(&mut stream))
                .await
                .is_err()
        );
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":large["id"], "result":"large"}),
        )
        .await
        .unwrap();
        let ordinary = read_operation(&mut stream).await;
        assert_eq!(ordinary["method"], "ordinary");
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":ordinary["id"], "result":"ordinary"}),
        )
        .await
        .unwrap();
    });
    let actor = ready_actor(&fixture.path, 4).await;
    let large = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .request(request("large").with_size(BridgeRequestSize::LargeFrame))
                .await
        }
    });
    tokio::task::yield_now().await;
    let ordinary = tokio::spawn({
        let actor = actor.clone();
        async move { actor.request(request("ordinary")).await }
    });
    large.await.unwrap().unwrap();
    ordinary.await.unwrap().unwrap();
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn extension_ping_is_answered_while_operation_is_pending() {
    let fixture = SocketFixture::new("ping-pending");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-ping").await;
        let pending = read_operation(&mut stream).await;
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":"extension-ping", "method":"ping"}),
        )
        .await
        .unwrap();
        let pong = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(pong["id"], "extension-ping");
        assert_eq!(pong["result"], "pong");
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":pending["id"], "result":true}),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(25)).await;
    });
    let actor = ready_actor(&fixture.path, 5).await;
    actor.request(request("pending")).await.unwrap();
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn eof_makes_dispatched_mutation_ambiguous_without_replay() {
    let fixture = SocketFixture::new("eof");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-eof").await;
        let mutation = read_operation(&mut stream).await;
        assert_eq!(mutation["method"], "mutate");
    });
    let actor = ready_actor(&fixture.path, 6).await;
    let result = actor
        .request(BridgeActorRequest::new(
            "mutate",
            json!({}),
            "mutation-1",
            OperationClass::Mutation,
        ))
        .await;
    assert_eq!(result.unwrap_err(), BridgeActorError::Ambiguous);
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn late_reply_hits_tombstone_and_stream_remains_usable() {
    let fixture = SocketFixture::new("late-reply");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-late").await;
        let late = read_operation(&mut stream).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":late["id"], "result":"late"}),
        )
        .await
        .unwrap();
        let next = read_operation(&mut stream).await;
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":next["id"], "result":"next"}),
        )
        .await
        .unwrap();
    });
    let actor = ready_actor(&fixture.path, 7).await;
    let mut events = actor.subscribe();
    let mut late = request("late");
    late.timeout = Duration::from_millis(25);
    assert_eq!(
        actor.request(late).await.unwrap_err(),
        BridgeActorError::TimedOut
    );
    loop {
        if matches!(
            events.recv().await.unwrap(),
            BridgeActorEvent::LateResponse { .. }
        ) {
            break;
        }
    }
    assert_eq!(
        actor.request(request("next")).await.unwrap()["result"],
        "next"
    );
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn settlement_and_settlement_unknown_are_routed_as_events() {
    let fixture = SocketFixture::new("settlement");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-settlement").await;
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "method":"skyCuaHost/settlement", "params":{"status":"completed"}}),
        )
        .await
        .unwrap();
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "method":"skyCuaHost/settlement", "params":{"status":"settlement_unknown"}}),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    let mut actor = BridgeActor::spawn(config(&fixture.path, 8));
    let mut events = actor.subscribe();
    actor.wait_until_ready().await.unwrap();
    let mut settlement = false;
    let mut unknown = false;
    while !settlement || !unknown {
        match events.recv().await.unwrap() {
            BridgeActorEvent::Settlement(_) => settlement = true,
            BridgeActorEvent::SettlementUnknown(_) => unknown = true,
            _ => {}
        }
    }
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn detached_waiter_does_not_close_or_stop_draining_stream() {
    let fixture = SocketFixture::new("detach");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "host-detach").await;
        let detached = read_operation(&mut stream).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":detached["id"], "result":true}),
        )
        .await
        .unwrap();
        let next = read_operation(&mut stream).await;
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":next["id"], "result":"still-open"}),
        )
        .await
        .unwrap();
    });
    let actor = ready_actor(&fixture.path, 9).await;
    let detached = tokio::spawn({
        let actor = actor.clone();
        async move { actor.request(request("detached")).await }
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    detached.abort();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        actor.request(request("after-detach")).await.unwrap()["result"],
        "still-open"
    );
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn multiple_bridge_candidates_each_get_one_persistent_actor() {
    let first_fixture = SocketFixture::new("multiple-first");
    let second_fixture = SocketFixture::new("multiple-second");
    let first_listener = UnixListener::bind(&first_fixture.path).unwrap();
    let second_listener = UnixListener::bind(&second_fixture.path).unwrap();
    let first_server = tokio::spawn(async move {
        let mut stream = accept_hello(&first_listener, "host-first").await;
        let request = read_operation(&mut stream).await;
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":request["id"], "result":"first"}),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    let second_server = tokio::spawn(async move {
        let mut stream = accept_hello(&second_listener, "host-second").await;
        let request = read_operation(&mut stream).await;
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0", "id":request["id"], "result":"second"}),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    let first = ready_actor(&first_fixture.path, 10).await;
    let second = ready_actor(&second_fixture.path, 11).await;
    assert_eq!(
        first.request(request("first")).await.unwrap()["result"],
        "first"
    );
    assert_eq!(
        second.request(request("second")).await.unwrap()["result"],
        "second"
    );
    first.shutdown().await;
    second.shutdown().await;
    first_server.await.unwrap();
    second_server.await.unwrap();
}

#[tokio::test]
async fn host_restart_with_connection_only_identity_emits_browser_lost() {
    let fixture = SocketFixture::new("restart-loss");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let first = accept_hello(&listener, "host-before").await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(first);
        let _second = accept_hello(&listener, "host-after").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let actor = ready_actor(&fixture.path, 12).await;
    let initial_instance = actor.health().browser_instance_id.unwrap();
    let mut events = actor.subscribe();
    let lost = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let BridgeActorEvent::BrowserLost {
                stable_recovery, ..
            } = events.recv().await.unwrap()
            {
                break stable_recovery;
            }
        }
    })
    .await
    .unwrap();
    assert!(!lost);
    tokio::time::timeout(Duration::from_secs(1), async {
        while actor.health().actor_generation == 12 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(actor.health().actor_generation, 13);
    assert_ne!(
        actor.health().browser_instance_id.as_deref(),
        Some(initial_instance.as_str())
    );
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn repeated_connection_only_browser_id_is_never_treated_as_stable() {
    let fixture = SocketFixture::new("repeated-connection-only-id");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let first = accept_connection_only_hello(&listener, "same-host", "repeated-browser").await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(first);
        let _second =
            accept_connection_only_hello(&listener, "same-host", "repeated-browser").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let actor = ready_actor(&fixture.path, 13).await;
    let initial_instance = actor.health().browser_instance_id.unwrap();
    assert_ne!(initial_instance, "repeated-browser");
    tokio::time::timeout(Duration::from_secs(1), async {
        while actor.health().actor_generation == 13 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_ne!(
        actor.health().browser_instance_id.as_deref(),
        Some(initial_instance.as_str())
    );
    assert_ne!(
        actor.health().browser_instance_id.as_deref(),
        Some("repeated-browser")
    );
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn queued_target_lifetime_action_fails_before_reconnect_dispatch() {
    let fixture = SocketFixture::new("queued-reconnect");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let (two_dispatched, two_dispatched_rx) = tokio::sync::oneshot::channel();
    let (disconnect, disconnect_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut first = accept_stable_hello(&listener, "stable-host", "browser-before").await;
        let _first = read_operation(&mut first).await;
        let _second = read_operation(&mut first).await;
        two_dispatched.send(()).unwrap();
        disconnect_rx.await.unwrap();
        drop(first);

        let mut second = accept_stable_hello(&listener, "stable-host", "browser-after").await;
        let frame = tokio::time::timeout(Duration::from_millis(75), read_frame(&mut second)).await;
        assert!(
            matches!(frame, Err(_) | Ok(Ok(None))),
            "queued old-browser action must not dispatch after reconnect: {frame:?}"
        );
    });
    let mut actor = BridgeActor::spawn(config(&fixture.path, 25));
    actor.wait_until_ready().await.unwrap();
    let first = actor
        .enqueue_request_for_test(request("occupy-first"))
        .await;
    let second = actor
        .enqueue_request_for_test(request("occupy-second"))
        .await;
    two_dispatched_rx.await.unwrap();

    let mut queued = BridgeActorRequest::new(
        "old-browser-mutation",
        json!({}),
        "queued-old-browser",
        OperationClass::Mutation,
    );
    queued.target_lifetime_key = Some(json!({ "browser_instance_id": "browser-before" }));
    let queued = actor.enqueue_request_for_test(queued).await;
    actor.barrier_for_test().await;
    disconnect.send(()).unwrap();

    assert_eq!(
        queued.await.unwrap().unwrap_err(),
        BridgeActorError::Disconnected
    );
    assert_eq!(
        first.await.unwrap().unwrap_err(),
        BridgeActorError::Disconnected
    );
    assert_eq!(
        second.await.unwrap().unwrap_err(),
        BridgeActorError::Disconnected
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while actor.health().browser_instance_id.as_deref() != Some("browser-after") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    actor.shutdown().await;
    server.await.unwrap();
}

#[tokio::test]
async fn stable_reconnect_with_changed_browser_identity_emits_previous_browser_lost() {
    let fixture = SocketFixture::new("stable-identity-change");
    let listener = UnixListener::bind(&fixture.path).unwrap();
    let server = tokio::spawn(async move {
        let first = accept_stable_hello(&listener, "stable-host", "browser-before").await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(first);
        let _second = accept_stable_hello(&listener, "stable-host", "browser-after").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let mut actor = BridgeActor::spawn(config(&fixture.path, 14));
    tokio::time::timeout(Duration::from_secs(1), actor.wait_until_ready())
        .await
        .unwrap()
        .unwrap();
    let mut events = actor.subscribe();
    let lost = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let BridgeActorEvent::BrowserLost {
                browser_instance_id,
                reason,
                stable_recovery,
            } = events.recv().await.unwrap()
            {
                break (browser_instance_id, reason, stable_recovery);
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(lost.0, "browser-before");
    assert_eq!(lost.1, "browser instance changed across reconnect");
    assert!(!lost.2);
    tokio::time::timeout(Duration::from_secs(1), async {
        while actor.health().browser_instance_id.as_deref() != Some("browser-after") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    actor.shutdown().await;
    server.await.unwrap();
}
