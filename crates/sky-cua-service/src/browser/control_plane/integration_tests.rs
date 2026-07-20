use std::{path::PathBuf, sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::{
    net::UnixListener,
    sync::{Notify, mpsc},
};

use super::{
    BridgeActor, BridgeActorConfig, BridgeActorRequest, BrowserInstanceId, ClientId, ControlPlane,
    Executor, ExecutorOutcome, OperationClass, OperationId, OperationScope, Principal, QueueLimits,
    SubmitOperation, TabKey, UpstreamCorrelation,
    persistent_proxy::{self, ChildTracker, ProxyContext},
};
use crate::browser::protocol::{read_frame, write_frame};

#[derive(Clone)]
struct AmbiguousExecutor;

impl Executor for AmbiguousExecutor {
    fn execute(
        &self,
        _operation: super::DispatchOperation,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ExecutorOutcome> + Send + 'static>>
    {
        Box::pin(async { ExecutorOutcome::Ambiguous("test ambiguity".to_owned()) })
    }
}

#[tokio::test]
async fn production_runtime_constructor_shape_recovers_only_suspended_hints() {
    use std::{
        collections::BTreeSet,
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::integration::BrowserControlRuntime;
    use super::{BrowserInstanceId, GroupAdmission, GroupId, LeaseState, Principal, TabKey};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir()
        .join(format!("sky-cua-runtime-recovery-{nonce}"))
        .join(super::persistence::RECOVERY_JOURNAL_FILE);
    let owner = Principal::new("owner", 1000);
    let browser = BrowserInstanceId::from("browser-a");
    let group_id = GroupId::from("group-a");
    let tab = TabKey::new(browser.clone(), "tab-a");

    let first = BrowserControlRuntime::new_with_recovery_path(path.clone());
    first
        .control
        .create_group(group_id.clone(), browser.clone(), owner.clone(), 0)
        .await;
    let active = first
        .control
        .add_member(group_id.clone(), owner.clone(), tab.clone())
        .await
        .unwrap();
    first.control.flush_persistence();

    let restarted = BrowserControlRuntime::new_with_recovery_path(path.clone());
    let recovered = restarted.control.group(group_id.clone()).await.unwrap();
    assert_eq!(recovered.admission, GroupAdmission::Suspended);
    assert_eq!(recovered.lease.state, LeaseState::Suspended);
    assert_eq!(recovered.lease.fence, active.lease.fence + 1);
    assert_eq!(recovered.members, BTreeSet::from([tab.clone()]));
    assert!(
        restarted
            .control
            .snapshot()
            .await
            .recent_operations
            .is_empty()
    );
    restarted.initialize_ownership_indexes().await;
    assert_eq!(
        restarted.shared.tab_owners.lock().await.get(&tab),
        Some(&group_id)
    );
    restarted.control.flush_persistence();
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn ambiguous_claim_reservations_survive_until_definitive_settlement() {
    use super::integration::{
        Shared, release_operation_reservation_if_definitive, remember_operation_reservation,
        settle_operation_reservation,
    };
    use super::{
        Completion, CompletionCertainty, ExecutorOutcome, GroupId, OperationId, Principal, TabKey,
    };

    let shared = Shared::default();
    let control = ControlPlane::start(
        "reservation-generation",
        Arc::new(AmbiguousExecutor),
        QueueLimits::default(),
    );
    let principal = Principal::new("reservation-owner", 1000);
    let browser = BrowserInstanceId::from("reservation-browser");

    let mcp_group = GroupId::from("mcp-group");
    let mcp_tab = TabKey::new(browser.clone(), "mcp-tab");
    let mcp_operation = OperationId::from("mcp-claim");
    control
        .create_group(mcp_group.clone(), browser.clone(), principal.clone(), 1)
        .await;
    shared
        .tab_owners
        .lock()
        .await
        .insert(mcp_tab.clone(), mcp_group.clone());
    remember_operation_reservation(
        &shared,
        mcp_operation.clone(),
        mcp_tab.clone(),
        mcp_group.clone(),
        principal.clone(),
    )
    .await;
    release_operation_reservation_if_definitive(
        &shared,
        &mcp_operation,
        &CompletionCertainty::Ambiguous,
    )
    .await;
    assert_eq!(
        shared.tab_owners.lock().await.get(&mcp_tab),
        Some(&mcp_group)
    );
    settle_operation_reservation(
        &shared,
        &control,
        &Completion::settlement_success(mcp_operation.clone(), "claimed".to_owned()),
    )
    .await;
    assert!(
        control
            .group(mcp_group)
            .await
            .unwrap()
            .members
            .contains(&mcp_tab)
    );

    let codex_group = GroupId::from("codex-group");
    let codex_tab = TabKey::new(browser.clone(), "codex-tab");
    let codex_operation = OperationId::from("codex-claim");
    control
        .create_group(codex_group.clone(), browser, principal.clone(), 1)
        .await;
    shared
        .tab_owners
        .lock()
        .await
        .insert(codex_tab.clone(), codex_group.clone());
    remember_operation_reservation(
        &shared,
        codex_operation.clone(),
        codex_tab.clone(),
        codex_group,
        principal,
    )
    .await;
    release_operation_reservation_if_definitive(
        &shared,
        &codex_operation,
        &CompletionCertainty::Ambiguous,
    )
    .await;
    assert!(shared.tab_owners.lock().await.contains_key(&codex_tab));
    settle_operation_reservation(
        &shared,
        &control,
        &Completion::from_executor(
            codex_operation.clone(),
            ExecutorOutcome::DefinitiveFailure("claim rejected".to_owned()),
        ),
    )
    .await;
    assert!(!shared.tab_owners.lock().await.contains_key(&codex_tab));
    assert!(shared.operation_reservations.lock().await.is_empty());
}

#[tokio::test]
async fn mcp_disconnect_orders_cleanup_after_started_requests_and_rejects_late_requests() {
    use super::integration::BrowserControlRuntime;
    use super::{GroupAdmission, GroupId, LeaseState, Principal};
    use sky_cua_platform::model::{
        BrowserCallerKind, BrowserCallerProvenance, BrowserLogicalIdentity,
        BrowserOperationIdentity, BrowserProvenanceSource, BrowserRequest, BrowserRequestContext,
    };

    let runtime = BrowserControlRuntime::new_with_limits(QueueLimits::default());
    let connection_id = "mcp-disconnect-race";
    let principal = Principal::new("mcp-session:disconnect-race", 1000);
    let group_id = GroupId::from("mcp-disconnect-group");
    let browser = BrowserInstanceId::from("mcp-disconnect-browser");

    assert!(
        runtime
            .begin_mcp_request(connection_id, principal.clone())
            .await
    );
    runtime
        .control
        .create_group(group_id.clone(), browser, principal.clone(), 1)
        .await;
    runtime.mcp_client_disconnected(connection_id).await;
    let before_completion = runtime.control.group(group_id.clone()).await.unwrap();
    assert_eq!(before_completion.admission, GroupAdmission::Open);
    assert_eq!(before_completion.lease.state, LeaseState::Active);
    assert!(
        !runtime
            .begin_mcp_request(connection_id, principal.clone())
            .await
    );
    let rejected = runtime
        .high_level(
            BrowserRequest::ListTabs { target: None },
            BrowserRequestContext {
                provenance: BrowserCallerProvenance {
                    caller: BrowserCallerKind::DirectMcp,
                    source: BrowserProvenanceSource::InstallerDeclaration,
                    connection_id: connection_id.to_owned(),
                    declared_caller: None,
                    client_info: None,
                },
                logical_identity: BrowserLogicalIdentity {
                    session_id: "disconnect-race".to_owned(),
                    thread_id: None,
                    turn_id: Some("late-turn".to_owned()),
                },
                operation_identity: BrowserOperationIdentity {
                    operation_id: "late-operation".to_owned(),
                    request_id_fingerprint: "late-fingerprint".to_owned(),
                },
            },
        )
        .await
        .unwrap_err();
    assert_eq!(rejected.code, "BrowserClientDisconnected");

    runtime.end_mcp_request(connection_id).await;
    let after_completion = runtime.control.group(group_id).await.unwrap();
    assert!(matches!(
        after_completion.lease.state,
        LeaseState::OrphanedGrace { .. }
    ));
    assert!(
        !runtime
            .shared
            .connection_principals
            .lock()
            .await
            .contains_key(connection_id)
    );
}

struct SocketFixture(PathBuf);

impl SocketFixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("sky-cua-integration-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.join(format!("extension-{name}.sock")))
    }
}

impl Drop for SocketFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

async fn accept_hello(listener: &UnixListener, browser_id: &str) -> tokio::net::UnixStream {
    let (mut stream, _) = listener.accept().await.unwrap();
    let hello = read_frame(&mut stream).await.unwrap().unwrap();
    let owner_mode = hello["params"]["owner_mode"]
        .as_str()
        .expect("actor hello carries owner mode");
    write_frame(&mut stream, &json!({
        "jsonrpc":"2.0", "id":hello["id"], "result":{
            "protocol_version":1,
            "mode":owner_mode,
            "owner_mode":owner_mode,
            "host_instance_id":"host-integration",
            "browser_instance_id":browser_id,
            "browser_instance_stability":"stable",
            "browser_family":"brave",
            "capabilities":["control_plane","heartbeat","extension_events","private_param_stripping","settlements","settlement_ack","side_panel_requests","owner_release"]
        }
    })).await.unwrap();
    stream
}

fn raw_dispatch_operation(
    operation_id: &str,
    browser_id: &str,
    tab_id: &str,
    timeout_ms: u64,
) -> super::DispatchOperation {
    use super::operation::OperationIdentity;

    super::DispatchOperation {
        identity: OperationIdentity {
            operation_id: OperationId::from(operation_id),
            daemon_generation: "test-generation".to_owned(),
            canonical_fingerprint: format!("fingerprint-{operation_id}"),
            upstream: UpstreamCorrelation {
                ingress: "test".to_owned(),
                request_id: None,
            },
        },
        client_id: ClientId::from("test-client"),
        principal: Principal::new("test-principal", unsafe { libc::geteuid() }),
        group_id: None,
        scope: OperationScope::Tab(TabKey::new(browser_id, tab_id)),
        class: OperationClass::Mutation,
        payload: serde_json::to_string(&json!({
            "kind":"raw",
            "method":"executeCdp",
            "params":{
                "tabId":tab_id,
                "method":"Runtime.evaluate",
                "params":{"expression":"window.testMutation = 1"}
            },
            "timeout_ms":timeout_ms,
            "identity":{
                "session_id":"test-session",
                "thread_id":"test-thread",
                "turn_id":"test-turn"
            }
        }))
        .unwrap(),
    }
}

#[tokio::test]
async fn canonical_ready_actors_deduplicate_stable_browser_sockets_deterministically() {
    use super::integration::{ActorEntry, BrowserControlRuntime, canonical_ready_actors};

    let first = SocketFixture::new("canonical-a");
    let second = SocketFixture::new("canonical-b");
    let first_listener = UnixListener::bind(&first.0).unwrap();
    let second_listener = UnixListener::bind(&second.0).unwrap();
    let first_server = tokio::spawn(async move {
        let _stream = accept_hello(&first_listener, "browser-duplicate").await;
        std::future::pending::<()>().await;
    });
    let second_server = tokio::spawn(async move {
        let _stream = accept_hello(&second_listener, "browser-duplicate").await;
        std::future::pending::<()>().await;
    });
    let mut first_actor = BridgeActor::spawn(BridgeActorConfig::new(first.0.clone(), 1));
    let mut second_actor = BridgeActor::spawn(BridgeActorConfig::new(second.0.clone(), 1));
    first_actor.wait_until_ready().await.unwrap();
    second_actor.wait_until_ready().await.unwrap();
    let entries = vec![
        ActorEntry {
            actor: second_actor.clone(),
            socket: second.0.clone(),
            browser_id: "browser-duplicate".to_owned(),
        },
        ActorEntry {
            actor: first_actor.clone(),
            socket: first.0.clone(),
            browser_id: "browser-duplicate".to_owned(),
        },
    ];

    let canonical = canonical_ready_actors(entries.clone());
    assert_eq!(canonical.len(), 1);
    let expected_socket = std::cmp::min(first.0.clone(), second.0.clone());
    assert_eq!(canonical[0].socket, expected_socket);

    let runtime = BrowserControlRuntime::new();
    runtime.shared.actors.write().unwrap().extend(
        entries
            .into_iter()
            .map(|entry| (entry.socket.clone(), entry)),
    );
    let snapshot = runtime.control_plane_snapshot().await;
    assert_eq!(
        snapshot
            .actors
            .iter()
            .filter(|actor| actor.canonical)
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .actors
            .iter()
            .find(|actor| actor.canonical)
            .unwrap()
            .socket_path,
        canonical[0].socket.to_string_lossy()
    );

    first_actor.shutdown().await;
    second_actor.shutdown().await;
    first_server.abort();
    second_server.abort();
}

#[tokio::test]
async fn canonical_ready_actors_preserve_distinct_browser_instances_and_require_selection() {
    use super::integration::{ActorEntry, canonical_ready_actors, one_actor};

    let first = SocketFixture::new("distinct-a");
    let second = SocketFixture::new("distinct-b");
    let first_listener = UnixListener::bind(&first.0).unwrap();
    let second_listener = UnixListener::bind(&second.0).unwrap();
    let first_server = tokio::spawn(async move {
        let _stream = accept_hello(&first_listener, "browser-distinct-a").await;
        std::future::pending::<()>().await;
    });
    let second_server = tokio::spawn(async move {
        let _stream = accept_hello(&second_listener, "browser-distinct-b").await;
        std::future::pending::<()>().await;
    });
    let mut first_actor = BridgeActor::spawn(BridgeActorConfig::new(first.0.clone(), 1));
    let mut second_actor = BridgeActor::spawn(BridgeActorConfig::new(second.0.clone(), 1));
    first_actor.wait_until_ready().await.unwrap();
    second_actor.wait_until_ready().await.unwrap();

    let canonical = canonical_ready_actors([
        ActorEntry {
            actor: first_actor.clone(),
            socket: first.0.clone(),
            browser_id: "browser-distinct-a".to_owned(),
        },
        ActorEntry {
            actor: second_actor.clone(),
            socket: second.0.clone(),
            browser_id: "browser-distinct-b".to_owned(),
        },
    ]);
    assert_eq!(canonical.len(), 2);
    let Err(error) = one_actor(&canonical) else {
        panic!("instance-less selection must reject distinct browsers");
    };
    assert_eq!(error.code, "BrowserInstanceAmbiguous");

    first_actor.shutdown().await;
    second_actor.shutdown().await;
    first_server.abort();
    second_server.abort();
}

#[tokio::test]
async fn raw_codex_execute_cdp_recovers_and_replays_only_upfront_unattached_failure() {
    use super::integration::{ActorEntry, IntegrationExecutor, Shared};

    let fixture = SocketFixture::new("raw-codex-upfront-recovery");
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "browser-raw-recovery").await;
        let original = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(original["method"], "executeCdp");
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0","id":original["id"],"error":{"code":1,"message":"Debugger unattached"}}),
        )
        .await
        .unwrap();

        let mut recovery_methods = Vec::new();
        for _ in 0..4 {
            let request = read_frame(&mut stream).await.unwrap().unwrap();
            recovery_methods.push(request["method"].as_str().unwrap().to_owned());
            if request["method"] == "executeCdp" {
                assert_eq!(request["params"]["method"], "Page.enable");
            }
            write_frame(
                &mut stream,
                &json!({"jsonrpc":"2.0","id":request["id"],"result":{}}),
            )
            .await
            .unwrap();
        }
        assert_eq!(
            recovery_methods,
            ["claimUserTab", "detach", "attach", "executeCdp"]
        );
        let replay = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(replay["method"], "executeCdp");
        assert_eq!(replay["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0","id":replay["id"],"result":{"replayed":true}}),
        )
        .await
        .unwrap();
    });
    let mut config = BridgeActorConfig::new(fixture.0.clone(), 1);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let shared = Arc::new(Shared::default());
    shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor: actor.clone(),
            socket: fixture.0.clone(),
            browser_id: "browser-raw-recovery".to_owned(),
        },
    );
    let executor = IntegrationExecutor {
        shared: Arc::clone(&shared),
    };
    let control = ControlPlane::start(
        "raw-recovery-generation",
        Arc::new(executor.clone()),
        QueueLimits::default(),
    );
    shared.control.set(control).ok().unwrap();

    let outcome = executor
        .execute(raw_dispatch_operation(
            "raw-recovery-operation",
            "browser-raw-recovery",
            "515",
            2_000,
        ))
        .await;
    assert_eq!(
        outcome,
        ExecutorOutcome::DefinitiveSuccess("{\"replayed\":true}".to_owned())
    );
    server.await.unwrap();
    actor.shutdown().await;
}

#[tokio::test]
async fn raw_codex_execute_cdp_ambiguous_timeout_is_never_replayed() {
    use super::integration::{ActorEntry, IntegrationExecutor, Shared};

    let fixture = SocketFixture::new("raw-codex-ambiguous-no-replay");
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "browser-raw-ambiguous").await;
        let original = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(original["method"], "executeCdp");
        let second =
            tokio::time::timeout(Duration::from_millis(250), read_frame(&mut stream)).await;
        assert!(second.is_err(), "ambiguous command must not be replayed");
    });
    let mut config = BridgeActorConfig::new(fixture.0.clone(), 1);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let shared = Arc::new(Shared::default());
    shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor: actor.clone(),
            socket: fixture.0.clone(),
            browser_id: "browser-raw-ambiguous".to_owned(),
        },
    );
    let executor = IntegrationExecutor { shared };
    let outcome = executor
        .execute(raw_dispatch_operation(
            "raw-ambiguous-operation",
            "browser-raw-ambiguous",
            "515",
            50,
        ))
        .await;
    assert!(matches!(outcome, ExecutorOutcome::Ambiguous(_)));
    server.await.unwrap();
    actor.shutdown().await;
}

#[tokio::test]
async fn proxy_rewrites_static_ids_per_subrequest_and_drop_only_detaches() {
    let fixture = SocketFixture::new("proxy");
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "browser-proxy").await;
        let mut host_operations = Vec::new();
        for result in ["one", "two", "after-drop"] {
            let request = read_frame(&mut stream).await.unwrap().unwrap();
            host_operations.push(
                request
                    .pointer("/params/_sky_cua_host_request/operation_id")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_owned(),
            );
            write_frame(
                &mut stream,
                &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
            )
            .await
            .unwrap();
        }
        host_operations
    });
    let mut config = BridgeActorConfig::new(fixture.0.clone(), 1);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let parents = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let context = ProxyContext::new(
        [(fixture.0.clone(), actor.clone())],
        "parent-operation".to_owned(),
        OperationClass::Mutation,
        Duration::from_secs(5),
        None,
        Arc::clone(&parents),
        ChildTracker::new(
            OperationId("parent-operation".to_owned()),
            ControlPlane::start(
                "proxy-test",
                Arc::new(actor.clone()),
                QueueLimits::default(),
            ),
        ),
    );
    persistent_proxy::scope(context, async {
        let mut proxy = super::connect_persistent_proxy(&fixture.0).await.unwrap().unwrap();
        for expected in ["one", "two"] {
            write_frame(&mut proxy, &json!({"jsonrpc":"2.0","id":"static-id","method":"executeCdp","params":{"method":"Runtime.evaluate"}})).await.unwrap();
            let response = read_frame(&mut proxy).await.unwrap().unwrap();
            assert_eq!(response["id"], "static-id");
            assert_eq!(response["result"], expected);
        }
    }).await;
    assert_eq!(
        actor
            .request(BridgeActorRequest::new(
                "getInfo",
                json!({}),
                "after-drop",
                OperationClass::ReadOnly
            ))
            .await
            .unwrap()["result"],
        "after-drop"
    );
    let operations = server.await.unwrap();
    assert_ne!(operations[0], operations[1]);
    assert!(operations[0].starts_with("parent-operation:bridge-subrequest:"));
    assert!(operations[1].starts_with("parent-operation:bridge-subrequest:"));
    assert_eq!(operations[2], "after-drop");
    assert!(parents.lock().await.is_empty());
}

#[tokio::test]
async fn extension_server_messages_round_trip_through_the_selected_codex_connection_exactly() {
    use super::integration::{ActorEntry, BrowserControlRuntime, spawn_actor_events};
    use crate::codex_browser_compat::CodexBrowserBackend;

    let fixture = SocketFixture::new("server-message");
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let release = Arc::new(Notify::new());
    let server_release = Arc::clone(&release);
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "browser-server-message").await;
        server_release.notified().await;
        let request = json!({"jsonrpc":"2.0","id":"host-request-77","method":"codex/serverRequest","params":{"exact":true}});
        write_frame(&mut stream, &request).await.unwrap();
        let response = read_frame(&mut stream).await.unwrap().unwrap();
        (request, response)
    });
    let mut config = BridgeActorConfig::new(fixture.0.clone(), 1);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let runtime = BrowserControlRuntime::new();
    runtime.shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor: actor.clone(),
            socket: fixture.0.clone(),
            browser_id: "browser-server-message".to_owned(),
        },
    );
    let (outbound, mut outbound_rx) = crate::codex_browser_compat::CodexOutbound::channel();
    let (other_outbound, mut other_outbound_rx) =
        crate::codex_browser_compat::CodexOutbound::channel();
    let principal = super::Principal::new("codex:connection-1", unsafe { libc::geteuid() });
    runtime
        .shared
        .connections
        .lock()
        .await
        .insert("connection-1".to_owned(), (principal, outbound));
    runtime.shared.connections.lock().await.insert(
        "connection-2".to_owned(),
        (
            super::Principal::new("codex:connection-2", unsafe { libc::geteuid() }),
            other_outbound,
        ),
    );
    runtime.shared.codex_by_browser.lock().await.insert(
        "browser-server-message".to_owned(),
        ["connection-1".to_owned()].into_iter().collect(),
    );
    spawn_actor_events(actor, Arc::clone(&runtime.shared), runtime.control.clone());
    release.notify_one();
    let forwarded = tokio::time::timeout(Duration::from_secs(2), outbound_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let response = json!({"jsonrpc":"2.0","id":"host-request-77","result":{"exact":"response"}});
    runtime
        .client_message("connection-1", response.clone())
        .await;
    let (original, received) = server.await.unwrap();
    assert_eq!(forwarded, original);
    assert_eq!(received, response);
    assert!(other_outbound_rx.try_recv().is_err());
}

#[tokio::test]
async fn codex_fetch_continuation_reenters_the_in_flight_tab_operation() {
    use super::integration::{ActorEntry, BrowserControlRuntime, spawn_actor_events};
    use crate::codex_browser_compat::{
        CodexBackendReply, CodexBrowserBackend, CodexConnectionContext, CodexLogicalIdentity,
        CodexNormalizedRequest, CodexOperationClass, CodexOperationScope,
    };
    use sky_cua_platform::model::{
        BrowserCallerKind, BrowserCallerProvenance, BrowserProvenanceSource,
    };

    fn request(
        connection: &CodexConnectionContext,
        operation_id: &str,
        upstream_id: u64,
        method: &str,
        params: Value,
        scope: CodexOperationScope,
    ) -> CodexNormalizedRequest {
        CodexNormalizedRequest {
            operation_id: operation_id.to_owned(),
            upstream_id,
            method: method.to_owned(),
            raw_request: json!({
                "jsonrpc":"2.0",
                "id":upstream_id,
                "method":method,
                "params":params,
            }),
            params,
            connection: connection.clone(),
            logical_identity: CodexLogicalIdentity {
                session_id: Some("codex-reentrant-session".to_owned()),
                thread_id: Some("codex-reentrant-thread".to_owned()),
                turn_id: Some("codex-reentrant-turn".to_owned()),
            },
            caller_provenance: BrowserCallerProvenance {
                caller: BrowserCallerKind::CodexDesktop,
                source: BrowserProvenanceSource::HostProvidedIab,
                connection_id: connection.connection_id.clone(),
                declared_caller: None,
                client_info: None,
            },
            identity_synthetic: false,
            class: CodexOperationClass::Mutation,
            scope,
            canonical_fingerprint: format!("fingerprint-{operation_id}"),
            deadline: Duration::from_secs(2),
        }
    }

    let fixture = SocketFixture::new("codex-reentrant-fetch");
    let socket_dir = fixture.0.parent().unwrap().to_path_buf();
    unsafe {
        std::env::set_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV, &socket_dir);
        std::env::set_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV, "all");
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "browser-codex-reentrant").await;
        let create = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(create["method"], "createTab");
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0","id":create["id"],"result":{"id":44,"url":"about:blank"}}),
        )
        .await
        .unwrap();

        let navigate = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(navigate["method"], "executeCdp");
        assert_eq!(navigate["params"]["method"], "Page.navigate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc":"2.0",
                "method":"onCDPEvent",
                "params":{
                    "method":"Fetch.requestPaused",
                    "params":{"requestId":"interception-job-test"},
                    "source":{"tabId":44}
                }
            }),
        )
        .await
        .unwrap();

        // This continuation must reach the actor before Page.navigate has a
        // response. Queuing it in the same-tab FIFO deadlocks the two calls.
        let continuation = tokio::time::timeout(Duration::from_secs(1), read_frame(&mut stream))
            .await
            .expect("reentrant continuation reached bridge")
            .unwrap()
            .unwrap();
        assert_eq!(continuation["method"], "executeCdp");
        assert_eq!(continuation["params"]["method"], "Fetch.continueResponse");
        assert_eq!(
            continuation["params"]["commandParams"]["requestId"],
            "interception-job-test"
        );
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0","id":continuation["id"],"result":{}}),
        )
        .await
        .unwrap();
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0","id":navigate["id"],"result":{"frameId":"frame-44"}}),
        )
        .await
        .unwrap();
    });

    let mut config = BridgeActorConfig::new(fixture.0.clone(), 1);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let runtime = BrowserControlRuntime::new();
    runtime.shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor: actor.clone(),
            socket: fixture.0.clone(),
            browser_id: "browser-codex-reentrant".to_owned(),
        },
    );
    spawn_actor_events(actor, Arc::clone(&runtime.shared), runtime.control.clone());
    let connection = CodexConnectionContext {
        connection_id: "codex-reentrant-connection".to_owned(),
        ingress: "test",
        peer_uid: unsafe { libc::geteuid() },
        codex_app_build_flavor: Some("prod".to_owned()),
        daemon_generation: runtime.daemon_generation(),
    };
    let (outbound, mut outbound_rx) = crate::codex_browser_compat::CodexOutbound::channel();
    runtime
        .connection_opened(connection.clone(), outbound)
        .await
        .unwrap();

    let created = runtime
        .request(request(
            &connection,
            "codex-create",
            1,
            "createTab",
            json!({}),
            CodexOperationScope::Bridge,
        ))
        .await;
    assert!(matches!(created, CodexBackendReply::Result(_)));

    let parent_runtime = Arc::clone(&runtime);
    let parent_connection = connection.clone();
    let parent = tokio::spawn(async move {
        parent_runtime
            .request(request(
                &parent_connection,
                "codex-navigate",
                2,
                "executeCdp",
                json!({
                    "target":{"tabId":44},
                    "method":"Page.navigate",
                    "commandParams":{"url":"http://127.0.0.1/"}
                }),
                CodexOperationScope::Tab("44".to_owned()),
            ))
            .await
    });
    let event = tokio::time::timeout(Duration::from_secs(2), outbound_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event["params"]["method"], "Fetch.requestPaused");

    let continuation = runtime
        .request(request(
            &connection,
            "codex-fetch-continue",
            3,
            "executeCdp",
            json!({
                "target":{"tabId":44},
                "method":"Fetch.continueResponse",
                "commandParams":{"requestId":"interception-job-test"}
            }),
            CodexOperationScope::Tab("44".to_owned()),
        ))
        .await;
    assert_eq!(continuation, CodexBackendReply::Result(json!({})));
    assert_eq!(
        parent.await.unwrap(),
        CodexBackendReply::Result(json!({"frameId":"frame-44"}))
    );
    assert!(runtime.shared.raw_tab_parents.lock().await.is_empty());
    assert_eq!(
        super::integration::correlation_counts(&runtime.shared)
            .await
            .3,
        0
    );
    server.await.unwrap();
    unsafe {
        std::env::remove_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV);
        std::env::remove_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV);
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
}

#[tokio::test]
async fn ambiguous_server_requests_are_rejected_and_notifications_fan_out_exactly() {
    use super::integration::{ActorEntry, BrowserControlRuntime, spawn_actor_events};
    use crate::codex_browser_compat::CodexBrowserBackend;

    let fixture = SocketFixture::new("ambiguous-server-message");
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let release = Arc::new(Notify::new());
    let server_release = Arc::clone(&release);
    let request =
        json!({"jsonrpc":"2.0","id":91,"method":"codex/serverRequest","params":{"exact":true}});
    let notification =
        json!({"jsonrpc":"2.0","method":"codex/serverNotification","params":{"exact":true}});
    let targeted_request = json!({
        "jsonrpc":"2.0",
        "id":"session-targeted",
        "method":"codex/serverRequest",
        "params":{"metadata":{"codexSessionId":"session-two"}}
    });
    let sent_request = request.clone();
    let sent_notification = notification.clone();
    let sent_targeted_request = targeted_request.clone();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "browser-ambiguous-server-message").await;
        server_release.notified().await;
        write_frame(&mut stream, &sent_request).await.unwrap();
        let rejection = read_frame(&mut stream).await.unwrap().unwrap();
        write_frame(&mut stream, &sent_notification).await.unwrap();
        write_frame(&mut stream, &sent_targeted_request)
            .await
            .unwrap();
        let targeted_response = read_frame(&mut stream).await.unwrap().unwrap();
        (rejection, targeted_response)
    });
    let mut config = BridgeActorConfig::new(fixture.0.clone(), 1);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let runtime = BrowserControlRuntime::new();
    runtime.shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor: actor.clone(),
            socket: fixture.0.clone(),
            browser_id: "browser-ambiguous-server-message".to_owned(),
        },
    );
    let (first_tx, mut first_rx) = crate::codex_browser_compat::CodexOutbound::channel();
    let (second_tx, mut second_rx) = crate::codex_browser_compat::CodexOutbound::channel();
    let uid = unsafe { libc::geteuid() };
    runtime.shared.connections.lock().await.extend([
        (
            "connection-1".to_owned(),
            (Principal::new("codex:connection-1", uid), first_tx),
        ),
        (
            "connection-2".to_owned(),
            (Principal::new("codex:connection-2", uid), second_tx),
        ),
    ]);
    runtime.shared.codex_by_browser.lock().await.insert(
        "browser-ambiguous-server-message".to_owned(),
        ["connection-1".to_owned(), "connection-2".to_owned()]
            .into_iter()
            .collect(),
    );
    runtime
        .shared
        .codex_connection_sessions
        .lock()
        .await
        .extend([
            (
                "connection-1".to_owned(),
                ["session-one".to_owned()].into_iter().collect(),
            ),
            (
                "connection-2".to_owned(),
                ["session-two".to_owned()].into_iter().collect(),
            ),
        ]);
    spawn_actor_events(actor, Arc::clone(&runtime.shared), runtime.control.clone());
    release.notify_one();

    let first = tokio::time::timeout(Duration::from_secs(2), first_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(2), second_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first, notification);
    assert_eq!(second, notification);
    let targeted = tokio::time::timeout(Duration::from_secs(2), second_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(targeted, targeted_request);
    assert!(first_rx.try_recv().is_err());
    let targeted_response =
        json!({"jsonrpc":"2.0","id":"session-targeted","result":{"selected":true}});
    runtime
        .client_message("connection-2", targeted_response.clone())
        .await;
    let (rejection, received_targeted_response) = server.await.unwrap();
    assert_eq!(rejection["id"], request["id"]);
    assert_eq!(rejection["error"]["code"], -32072);
    assert!(
        rejection["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unambiguous")
    );
    assert_eq!(received_targeted_response, targeted_response);
}

#[tokio::test]
async fn ambiguous_mutating_child_is_not_masked_by_high_level_diagnostics() {
    use super::integration::{ActorEntry, IntegrationExecutor, Shared};
    use super::operation::{DispatchOperation, OperationIdentity};

    let fixture = SocketFixture::new("ambiguous-child");
    let socket_dir = fixture.0.parent().unwrap().to_path_buf();
    unsafe {
        std::env::set_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV, &socket_dir);
        std::env::set_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV, "all");
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "browser-ambiguous").await;
        let probe = read_frame(&mut stream).await.unwrap().unwrap();
        write_frame(
            &mut stream,
            &json!({"jsonrpc":"2.0","id":probe["id"],"result":{"ready":true}}),
        )
        .await
        .unwrap();
        let mutation = loop {
            let request = read_frame(&mut stream).await.unwrap().unwrap();
            if request["method"] == "executeCdp" {
                break request;
            }
            assert!(matches!(
                request["method"].as_str(),
                Some("moveMouse" | "getInfo")
            ));
            write_frame(
                &mut stream,
                &json!({"jsonrpc":"2.0","id":request["id"],"result":true}),
            )
            .await
            .unwrap();
        };
        assert_eq!(mutation["method"], "executeCdp");
        assert_eq!(
            mutation["params"]["_sky_cua_host_request"]["operation_class"],
            "mutation"
        );
        drop(stream);
    });
    let mut config = BridgeActorConfig::new(fixture.0.clone(), 1);
    config.heartbeat_interval = Duration::from_secs(30);
    config.reconnect_min = Duration::from_secs(5);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let shared = Arc::new(Shared::default());
    shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor,
            socket: fixture.0.clone(),
            browser_id: "browser-ambiguous".to_owned(),
        },
    );
    let executor = Arc::new(IntegrationExecutor {
        shared: Arc::clone(&shared),
    });
    let control = ControlPlane::start("integration-test", executor.clone(), QueueLimits::default());
    assert!(shared.control.set(control).is_ok());
    let operation = DispatchOperation {
        identity: OperationIdentity {
            operation_id: OperationId("parent-ambiguous".to_owned()),
            daemon_generation: "generation".to_owned(),
            canonical_fingerprint: "fingerprint".to_owned(),
            upstream: UpstreamCorrelation {
                ingress: "test".to_owned(),
                request_id: None,
            },
        },
        client_id: ClientId("client".to_owned()),
        principal: Principal::new("principal", unsafe { libc::geteuid() }),
        group_id: None,
        scope: OperationScope::Tab(TabKey::new(
            BrowserInstanceId("browser-ambiguous".to_owned()),
            "7",
        )),
        class: OperationClass::Mutation,
        payload: json!({
            "kind":"high_level",
            "request":{"type":"click","tab_id":"7","x":10.0,"y":20.0},
            "identity":{"session_id":"test","turn_id":"turn"}
        })
        .to_string(),
    };
    let outcome = tokio::time::timeout(Duration::from_secs(2), executor.execute(operation))
        .await
        .unwrap();
    assert!(
        matches!(outcome, super::ExecutorOutcome::Ambiguous(_)),
        "unexpected outcome: {outcome:?}"
    );
    server.await.unwrap();
    unsafe {
        std::env::remove_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV);
        std::env::remove_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV);
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
}

#[tokio::test]
async fn one_connection_keeps_logical_session_groups_and_tab_ownership_isolated() {
    use super::integration::BrowserControlRuntime;

    let runtime = BrowserControlRuntime::new();
    let principal = Principal::new("codex:shared-connection", 1000);
    let browser = BrowserInstanceId("browser-logical".to_owned());
    let first = runtime
        .default_group(&principal, "session:first:thread:a", &browser)
        .await;
    let second = runtime
        .default_group(&principal, "session:second:thread:b", &browser)
        .await;
    assert_ne!(first.group_id, second.group_id);

    let tab = TabKey::new(browser, "91");
    runtime
        .control
        .add_member(first.group_id.clone(), principal.clone(), tab.clone())
        .await
        .unwrap();
    runtime
        .shared
        .tab_owners
        .lock()
        .await
        .insert(tab.clone(), first.group_id);
    let error = runtime
        .group_for_tab(&principal, &tab, Some(&second.group_id))
        .await
        .unwrap_err();
    assert_eq!(error.code, "BrowserOwnershipRejected");
}

#[tokio::test]
async fn stale_settlement_metadata_cannot_resolve_another_operation() {
    use super::SettlementState;
    use super::integration::{
        SettlementFence, Shared, correlation_counts, install_test_operation_correlations,
        settle_actor_message,
    };

    let shared = Arc::new(Shared::default());
    let control = ControlPlane::start(
        "daemon-current",
        Arc::new(AmbiguousExecutor),
        QueueLimits::default(),
    );
    let operation = OperationId("reused-operation".to_owned());
    let browser = BrowserInstanceId("browser-settlement".to_owned());
    let completion = control
        .submit(SubmitOperation {
            operation_id: Some(operation.clone()),
            canonical_fingerprint: "current-request".to_owned(),
            upstream: UpstreamCorrelation {
                ingress: "test".to_owned(),
                request_id: Some("1".to_owned()),
            },
            client_id: ClientId("current-client".to_owned()),
            principal: Principal::new("current-principal", 1000),
            group_id: None,
            lease: None,
            scope: OperationScope::BridgeGlobal(browser.clone()),
            class: OperationClass::Mutation,
            payload: "ignored".to_owned(),
            now_ms: 1,
        })
        .await
        .unwrap();
    assert_eq!(completion.operation_id, operation);
    assert!(control.settlement_state(operation.clone()).await.is_some());
    let target = json!({"browser_instance_id":browser.0});
    install_test_operation_correlations(
        &shared,
        operation.clone(),
        "current-client",
        SettlementFence {
            daemon_generation: "daemon-current".to_owned(),
            actor_generation: Value::from(4),
            browser_instance_id: browser.clone(),
            target_lifetime_key: target.clone(),
            operation_class: "mutation",
        },
    )
    .await;

    let settlement = |daemon: &str, actor: u64, browser_target: Value| {
        json!({
            "jsonrpc":"2.0",
            "method":"skyCuaHost/settlement",
            "params":{
                "operation_id":"reused-operation",
                "daemon_generation":daemon,
                "actor_generation":actor,
                "chrome_request_id":"chrome-reused-operation",
                "target_lifetime_key":browser_target,
                "operation_class":"mutation",
                "completion":{"result":{"done":true}}
            }
        })
    };
    settle_actor_message(
        &control,
        &shared,
        "browser-settlement",
        settlement("daemon-stale", 4, target.clone()),
        false,
    )
    .await;
    settle_actor_message(
        &control,
        &shared,
        "browser-settlement",
        settlement("daemon-current", 3, target.clone()),
        false,
    )
    .await;
    settle_actor_message(
        &control,
        &shared,
        "browser-other",
        settlement("daemon-current", 4, target.clone()),
        false,
    )
    .await;
    assert!(matches!(
        control.settlement_state(operation.clone()).await,
        Some(SettlementState::Pending { .. })
    ));
    assert_eq!(correlation_counts(&shared).await, (1, 1, 1, 0, 0, 0));

    settle_actor_message(
        &control,
        &shared,
        "browser-settlement",
        settlement("daemon-current", 4, target),
        false,
    )
    .await;
    assert!(control.settlement_state(operation).await.is_none());
    assert_eq!(correlation_counts(&shared).await, (0, 0, 0, 0, 0, 0));
}

#[tokio::test]
async fn retained_settlement_uses_recorded_operation_generation_not_current_daemon_generation() {
    use super::integration::{
        SettlementFence, Shared, correlation_counts, install_test_operation_correlations,
        settle_actor_message,
    };

    let shared = Arc::new(Shared::default());
    let control = ControlPlane::start(
        "daemon-new",
        Arc::new(AmbiguousExecutor),
        QueueLimits::default(),
    );
    let operation = OperationId("retained-operation".to_owned());
    control
        .submit(SubmitOperation {
            operation_id: Some(operation.clone()),
            canonical_fingerprint: "retained-request".to_owned(),
            upstream: UpstreamCorrelation {
                ingress: "test".to_owned(),
                request_id: None,
            },
            client_id: ClientId("retained-client".to_owned()),
            principal: Principal::new("retained-principal", 1000),
            group_id: None,
            lease: None,
            scope: OperationScope::BridgeGlobal(BrowserInstanceId("browser-retained".to_owned())),
            class: OperationClass::Mutation,
            payload: "ignored".to_owned(),
            now_ms: 1,
        })
        .await
        .unwrap();
    let target = json!({"browser_instance_id":"browser-retained"});
    install_test_operation_correlations(
        &shared,
        operation.clone(),
        "retained-client",
        SettlementFence {
            daemon_generation: "daemon-old".to_owned(),
            actor_generation: Value::from(7),
            browser_instance_id: BrowserInstanceId("browser-retained".to_owned()),
            target_lifetime_key: target.clone(),
            operation_class: "mutation",
        },
    )
    .await;
    let settlement = json!({
        "jsonrpc":"2.0",
        "method":"skyCuaHost/settlement",
        "params":{
            "operation_id":"retained-operation",
            "daemon_generation":"daemon-old",
            "actor_generation":7,
            "chrome_request_id":"chrome-retained-operation",
            "target_lifetime_key":target,
            "operation_class":"mutation",
            "completion":{"result":true}
        }
    });
    assert!(
        settle_actor_message(
            &control,
            &shared,
            "browser-retained",
            settlement.clone(),
            false,
        )
        .await
    );
    assert!(control.settlement_state(operation).await.is_none());
    assert_eq!(correlation_counts(&shared).await, (0, 0, 0, 0, 0, 0));
    assert!(settle_actor_message(&control, &shared, "browser-retained", settlement, false,).await);
}

#[tokio::test]
async fn retained_settlement_unknown_to_this_daemon_is_not_acknowledged() {
    use super::integration::{Shared, settle_actor_message};

    let shared = Shared::default();
    let control = ControlPlane::start(
        "daemon-new",
        Arc::new(AmbiguousExecutor),
        QueueLimits::default(),
    );
    let acknowledged = settle_actor_message(
        &control,
        &shared,
        "browser-retained",
        json!({
            "jsonrpc":"2.0",
            "method":"skyCuaHost/settlement",
            "params":{
                "operation_id":"operation-from-dead-daemon",
                "daemon_generation":"daemon-old",
                "actor_generation":7,
                "chrome_request_id":"chrome-from-dead-daemon",
                "target_lifetime_key":{"browser_instance_id":"browser-retained"},
                "operation_class":"mutation",
                "completion":{"result":true}
            }
        }),
        false,
    )
    .await;

    assert!(!acknowledged);
}

#[tokio::test]
async fn direct_terminal_mutation_accepts_its_retained_host_settlement() {
    use super::integration::{Shared, TerminalSettlementOperation, settle_actor_message};

    let shared = Shared::default();
    shared
        .terminal_settlement_operations
        .lock()
        .await
        .push_back(TerminalSettlementOperation {
            operation_id: OperationId("direct-terminal-operation".to_owned()),
            daemon_generation: "daemon-current".to_owned(),
        });
    let control = ControlPlane::start(
        "daemon-current",
        Arc::new(AmbiguousExecutor),
        QueueLimits::default(),
    );
    let settlement = json!({
        "jsonrpc":"2.0",
        "method":"skyCuaHost/settlement",
        "params":{
            "operation_id":"direct-terminal-operation",
            "daemon_generation":"daemon-current",
            "actor_generation":9,
            "chrome_request_id":"chrome-direct-terminal",
            "target_lifetime_key":{"browser_instance_id":"browser-direct"},
            "operation_class":"mutation",
            "completion":{"result":true}
        }
    });

    assert!(
        settle_actor_message(
            &control,
            &shared,
            "browser-direct",
            settlement.clone(),
            false,
        )
        .await
    );
    assert!(
        settle_actor_message(&control, &shared, "browser-direct", settlement, false).await,
        "an ack replay of the exact handled identity remains idempotent"
    );
    assert!(
        shared
            .terminal_settlement_operations
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn late_mutation_response_preserves_identity_for_following_settlement_ack() {
    use super::integration::{
        SettlementFence, Shared, correlation_counts, install_test_operation_correlations,
        settle_actor_message, settle_late_response,
    };

    let shared = Shared::default();
    let control = ControlPlane::start(
        "daemon-current",
        Arc::new(AmbiguousExecutor),
        QueueLimits::default(),
    );
    let operation = OperationId("late-terminal-operation".to_owned());
    control
        .submit(SubmitOperation {
            operation_id: Some(operation.clone()),
            canonical_fingerprint: "late-terminal-request".to_owned(),
            upstream: UpstreamCorrelation {
                ingress: "test".to_owned(),
                request_id: None,
            },
            client_id: ClientId("late-terminal-client".to_owned()),
            principal: Principal::new("late-terminal-principal", 1000),
            group_id: None,
            lease: None,
            scope: OperationScope::BridgeGlobal(BrowserInstanceId(
                "browser-late-terminal".to_owned(),
            )),
            class: OperationClass::Mutation,
            payload: "ignored".to_owned(),
            now_ms: 1,
        })
        .await
        .unwrap();
    let target = json!({"browser_instance_id":"browser-late-terminal"});
    install_test_operation_correlations(
        &shared,
        operation.clone(),
        "late-terminal-client",
        SettlementFence {
            daemon_generation: "daemon-current".to_owned(),
            actor_generation: Value::from(11),
            browser_instance_id: BrowserInstanceId("browser-late-terminal".to_owned()),
            target_lifetime_key: target.clone(),
            operation_class: "mutation",
        },
    )
    .await;

    settle_late_response(
        &control,
        &shared,
        operation.0.clone(),
        json!({"jsonrpc":"2.0", "id":"actor-request", "result":{"done":true}}),
    )
    .await;
    assert!(control.settlement_state(operation.clone()).await.is_none());
    assert_eq!(correlation_counts(&shared).await, (0, 0, 0, 0, 0, 0));
    assert_eq!(shared.terminal_settlement_operations.lock().await.len(), 1);

    assert!(
        settle_actor_message(
            &control,
            &shared,
            "browser-late-terminal",
            json!({
                "jsonrpc":"2.0",
                "method":"skyCuaHost/settlement",
                "params":{
                    "operation_id":"late-terminal-operation",
                    "daemon_generation":"daemon-current",
                    "actor_generation":11,
                    "chrome_request_id":"chrome-late-terminal",
                    "target_lifetime_key":target,
                    "operation_class":"mutation",
                    "completion":{"result":{"done":true}}
                }
            }),
            false,
        )
        .await
    );
    assert!(
        shared
            .terminal_settlement_operations
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn admission_rejections_leave_no_operation_correlation_leaks() {
    use super::integration::{ActorEntry, BrowserControlRuntime, correlation_counts};
    use crate::codex_browser_compat::{
        CodexBackendReply, CodexBrowserBackend, CodexConnectionContext, CodexLogicalIdentity,
        CodexNormalizedRequest, CodexOperationClass, CodexOperationScope,
    };
    use sky_cua_platform::model::{
        BrowserCallerKind, BrowserCallerProvenance, BrowserLogicalIdentity,
        BrowserOperationIdentity, BrowserProvenanceSource, BrowserRequest, BrowserRequestContext,
    };

    let fixture = SocketFixture::new("admission-cleanup");
    let socket_dir = fixture.0.parent().unwrap().to_path_buf();
    unsafe {
        std::env::set_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV, &socket_dir);
        std::env::set_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV, "all");
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let release = Arc::new(Notify::new());
    let server_release = Arc::clone(&release);
    let server = tokio::spawn(async move {
        let _stream = accept_hello(&listener, "browser-admission-cleanup").await;
        server_release.notified().await;
    });
    let mut config = BridgeActorConfig::new(fixture.0.clone(), 2);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let runtime = BrowserControlRuntime::new_with_limits(QueueLimits {
        per_client: 0,
        ..QueueLimits::default()
    });
    runtime.shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor,
            socket: fixture.0.clone(),
            browser_id: "browser-admission-cleanup".to_owned(),
        },
    );

    let mcp_error = runtime
        .high_level(
            BrowserRequest::ListTabs { target: None },
            BrowserRequestContext {
                provenance: BrowserCallerProvenance {
                    caller: BrowserCallerKind::LegacyUnknown,
                    source: BrowserProvenanceSource::LegacyFallback,
                    connection_id: "mcp-rejected".to_owned(),
                    declared_caller: None,
                    client_info: None,
                },
                logical_identity: BrowserLogicalIdentity {
                    session_id: "session".to_owned(),
                    thread_id: None,
                    turn_id: None,
                },
                operation_identity: BrowserOperationIdentity {
                    operation_id: "mcp-operation".to_owned(),
                    request_id_fingerprint: "mcp-fingerprint".to_owned(),
                },
            },
        )
        .await
        .unwrap_err();
    assert_eq!(mcp_error.code, "BrowserControlAdmissionRejected");
    assert_eq!(
        correlation_counts(&runtime.shared).await,
        (0, 0, 0, 0, 0, 0)
    );

    let generation = runtime.daemon_generation();
    let connection = CodexConnectionContext {
        connection_id: "codex-rejected".to_owned(),
        ingress: "test",
        peer_uid: unsafe { libc::geteuid() },
        codex_app_build_flavor: None,
        daemon_generation: generation,
    };
    let (outbound, _outbound_rx) = crate::codex_browser_compat::CodexOutbound::channel();
    runtime
        .connection_opened(connection.clone(), outbound)
        .await
        .unwrap();
    let reply = runtime
        .request(CodexNormalizedRequest {
            operation_id: "codex-operation".to_owned(),
            upstream_id: 42,
            method: "createTab".to_owned(),
            params: json!({}),
            raw_request: json!({"jsonrpc":"2.0","id":42,"method":"createTab","params":{}}),
            connection,
            logical_identity: CodexLogicalIdentity::default(),
            caller_provenance: BrowserCallerProvenance {
                caller: BrowserCallerKind::CodexDesktop,
                source: BrowserProvenanceSource::HostProvidedIab,
                connection_id: "codex-rejected".to_owned(),
                declared_caller: None,
                client_info: None,
            },
            identity_synthetic: false,
            class: CodexOperationClass::Mutation,
            scope: CodexOperationScope::Bridge,
            canonical_fingerprint: "codex-fingerprint".to_owned(),
            deadline: Duration::from_secs(1),
        })
        .await;
    assert!(matches!(reply, CodexBackendReply::Error(_)));
    assert!(crate::browser::browser_session_lingering());
    assert_eq!(
        correlation_counts(&runtime.shared).await,
        (0, 0, 0, 0, 0, 0)
    );

    release.notify_one();
    server.await.unwrap();
    unsafe {
        std::env::remove_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV);
        std::env::remove_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV);
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
}

#[tokio::test]
async fn released_owner_cleanup_waits_for_actual_release() {
    use super::integration::BrowserControlRuntime;
    use super::{GroupAdmission, GroupId, LeaseState};

    let runtime = BrowserControlRuntime::new();
    let principal = Principal::new("cleanup-principal", unsafe { libc::geteuid() });
    let group_id = GroupId::from("cleanup-group");
    let tab = TabKey::new("cleanup-browser", "cleanup-tab");
    runtime
        .control
        .create_group(
            group_id.clone(),
            tab.browser_instance_id.clone(),
            principal.clone(),
            0,
        )
        .await;
    let active = runtime
        .control
        .add_member(group_id.clone(), principal.clone(), tab.clone())
        .await
        .unwrap();
    runtime
        .shared
        .tab_owners
        .lock()
        .await
        .insert(tab.clone(), group_id.clone());

    for admission in [
        GroupAdmission::SettlementPending,
        GroupAdmission::ExpiryPending,
    ] {
        let mut pending = active.clone();
        pending.admission = admission;
        pending.lease.state = LeaseState::ExpiryPending;
        runtime
            .remove_released_group_ownership(&group_id, &pending)
            .await;
        assert_eq!(
            runtime.shared.tab_owners.lock().await.get(&tab),
            Some(&group_id)
        );
    }

    let released = runtime
        .control
        .end_group(group_id.clone(), principal)
        .await
        .unwrap();
    assert_eq!(released.admission, GroupAdmission::Released);
    runtime
        .remove_released_group_ownership(&group_id, &released)
        .await;
    assert!(!runtime.shared.tab_owners.lock().await.contains_key(&tab));
}

#[tokio::test]
async fn ownership_reconciliation_keeps_pending_and_drops_released_groups() {
    use super::integration::{BrowserControlRuntime, authoritative_tab_owners};
    use super::{GroupAdmission, GroupId, LeaseState};

    let runtime = BrowserControlRuntime::new();
    let principal = Principal::new("reconcile-principal", unsafe { libc::geteuid() });
    let pending_id = GroupId::from("reconcile-pending");
    let released_id = GroupId::from("reconcile-released");
    let pending_tab = TabKey::new("reconcile-browser", "pending");
    let released_tab = TabKey::new("reconcile-browser", "released");
    for (group_id, tab) in [
        (pending_id.clone(), pending_tab.clone()),
        (released_id.clone(), released_tab.clone()),
    ] {
        runtime
            .control
            .create_group(
                group_id.clone(),
                tab.browser_instance_id.clone(),
                principal.clone(),
                0,
            )
            .await;
        runtime
            .control
            .add_member(group_id, principal.clone(), tab)
            .await
            .unwrap();
    }
    runtime.control.disconnect(principal.clone(), 0).await;
    runtime
        .control
        .tick(super::lease::DISCONNECT_GRACE_MS)
        .await;
    let released_idle = runtime.control.group(pending_id.clone()).await.unwrap();
    assert!(matches!(released_idle.lease.state, LeaseState::Released));
    let released_lifecycle = runtime
        .control
        .end_group(released_id.clone(), principal)
        .await
        .unwrap();
    let mut settlement_pending = released_idle.clone();
    settlement_pending.group_id = GroupId::from("still-settlement-pending");
    settlement_pending.admission = GroupAdmission::SettlementPending;
    settlement_pending.lease.state = LeaseState::ExpiryPending;
    settlement_pending.members = [pending_tab.clone()].into_iter().collect();
    let mut expiry_pending = settlement_pending.clone();
    expiry_pending.group_id = GroupId::from("still-expiry-pending");
    expiry_pending.admission = GroupAdmission::ExpiryPending;
    expiry_pending.members = [released_tab.clone()].into_iter().collect();
    let owners = authoritative_tab_owners(vec![
        released_idle,
        released_lifecycle,
        settlement_pending.clone(),
        expiry_pending.clone(),
    ]);
    assert_eq!(owners.get(&pending_tab), Some(&settlement_pending.group_id));
    assert_eq!(owners.get(&released_tab), Some(&expiry_pending.group_id));

    let reservation_principal = Principal::new("reservation-principal", unsafe { libc::geteuid() });
    let reservation_group = GroupId::from("active-reservation");
    let reservation_tab = TabKey::new("reconcile-browser", "reserved-before-membership");
    runtime
        .control
        .create_group(
            reservation_group.clone(),
            reservation_tab.browser_instance_id.clone(),
            reservation_principal,
            1,
        )
        .await;
    runtime
        .shared
        .tab_owners
        .lock()
        .await
        .insert(reservation_tab.clone(), reservation_group.clone());

    runtime.reconcile_tab_owners().await;
    assert!(
        !runtime
            .shared
            .tab_owners
            .lock()
            .await
            .contains_key(&pending_tab)
    );
    assert!(
        !runtime
            .shared
            .tab_owners
            .lock()
            .await
            .contains_key(&released_tab)
    );
    assert_eq!(
        runtime.shared.tab_owners.lock().await.get(&reservation_tab),
        Some(&reservation_group)
    );
}

#[tokio::test]
async fn production_runtime_periodically_expires_leases_and_reconciles_owners() {
    use super::GroupId;
    use super::integration::BrowserControlRuntime;

    let runtime = BrowserControlRuntime::new();
    let principal = Principal::new("periodic-principal", unsafe { libc::geteuid() });
    let group_id = GroupId::from("periodic-group");
    let tab = TabKey::new("periodic-browser", "periodic-tab");
    runtime
        .control
        .create_group(
            group_id.clone(),
            tab.browser_instance_id.clone(),
            principal.clone(),
            0,
        )
        .await;
    runtime
        .control
        .add_member(group_id.clone(), principal, tab.clone())
        .await
        .unwrap();
    runtime.reconcile_tab_owners().await;
    assert!(runtime.shared.tab_owners.lock().await.contains_key(&tab));

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if runtime.control.group(group_id.clone()).await.is_err()
                && !runtime.shared.tab_owners.lock().await.contains_key(&tab)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("periodic lease tick released the idle group and reconciled ownership");
}

#[tokio::test]
async fn released_group_pruning_removes_scheduler_and_logical_group_indexes() {
    use super::integration::BrowserControlRuntime;
    use super::{GroupError, GroupId};

    let runtime = BrowserControlRuntime::new();
    let principal = Principal::new("pruned-default-owner", unsafe { libc::geteuid() });
    let browser = BrowserInstanceId::from("pruned-default-browser");
    let group = runtime
        .default_group(&principal, "session:pruned", &browser)
        .await;
    runtime
        .control
        .end_group(group.group_id.clone(), principal)
        .await
        .unwrap();

    runtime.prune_released_groups().await;

    assert_eq!(
        runtime.control.group(group.group_id.clone()).await,
        Err(GroupError::UnknownGroup)
    );
    assert!(!runtime.has_logical_group_index(&group.group_id).await);
}

#[tokio::test]
async fn closed_codex_connection_cleanup_reorphanes_a_late_group_renewal() {
    use super::LeaseState;
    use super::integration::BrowserControlRuntime;
    use crate::codex_browser_compat::{CodexBrowserBackend, CodexConnectionContext};

    let runtime = BrowserControlRuntime::new();
    let connection_id = "closed-renewal-race";
    let peer_uid = unsafe { libc::geteuid() };
    let connection = CodexConnectionContext {
        connection_id: connection_id.to_owned(),
        ingress: "test",
        peer_uid,
        codex_app_build_flavor: None,
        daemon_generation: runtime.daemon_generation(),
    };
    let (outbound, _outbound_rx) = crate::codex_browser_compat::CodexOutbound::channel();
    runtime
        .connection_opened(connection, outbound)
        .await
        .unwrap();
    let principal = Principal::new(format!("codex:{connection_id}"), peer_uid);
    runtime
        .register_principal_connection(connection_id, principal.clone())
        .await;
    let browser = BrowserInstanceId::from("closed-renewal-browser");
    let group = runtime
        .default_group(&principal, "session:closed-renewal", &browser)
        .await;

    runtime.connection_closed(connection_id).await;
    let orphaned = runtime.control.group(group.group_id.clone()).await.unwrap();
    assert!(matches!(
        orphaned.lease.state,
        LeaseState::OrphanedGrace { .. }
    ));

    let renewed_after_close = runtime
        .default_group(&principal, "session:closed-renewal", &browser)
        .await;
    assert_eq!(renewed_after_close.lease.state, LeaseState::Active);
    runtime
        .abort_closed_codex_request(connection_id, &principal)
        .await;

    let reorphaned = runtime.control.group(group.group_id).await.unwrap();
    assert!(matches!(
        reorphaned.lease.state,
        LeaseState::OrphanedGrace { .. }
    ));
}

#[tokio::test]
async fn normalized_session_principal_survives_reconnect_and_is_reference_counted() {
    use super::LeaseState;
    use super::integration::{BrowserControlRuntime, principal_from_mcp};
    use sky_cua_platform::model::{
        BrowserCallerKind, BrowserCallerProvenance, BrowserLogicalIdentity,
        BrowserOperationIdentity, BrowserProvenanceSource, BrowserRequestContext,
    };

    let context = |connection_id: &str, session_id: &str| BrowserRequestContext {
        provenance: BrowserCallerProvenance {
            caller: BrowserCallerKind::OpenCode,
            source: BrowserProvenanceSource::InstallerDeclaration,
            connection_id: connection_id.to_owned(),
            declared_caller: Some("opencode".to_owned()),
            client_info: None,
        },
        logical_identity: BrowserLogicalIdentity {
            session_id: session_id.to_owned(),
            thread_id: Some("thread-a".to_owned()),
            turn_id: None,
        },
        operation_identity: BrowserOperationIdentity {
            operation_id: format!("operation-{connection_id}"),
            request_id_fingerprint: format!("fingerprint-{connection_id}"),
        },
    };
    let first = context("connection-a", "session-a");
    let second = context("connection-b", "session-a");
    let different = context("connection-c", "session-b");
    let principal = principal_from_mcp(&first);
    assert_eq!(principal, principal_from_mcp(&second));
    assert_ne!(principal, principal_from_mcp(&different));

    let runtime = BrowserControlRuntime::new();
    runtime
        .register_principal_connection("connection-a", principal.clone())
        .await;
    runtime
        .register_principal_connection("connection-b", principal.clone())
        .await;
    let browser = BrowserInstanceId::from("reference-browser");
    let logical = "session:session-a:thread:thread-a";
    let group = runtime.default_group(&principal, logical, &browser).await;
    runtime.release_connection_principals("connection-a").await;
    assert_eq!(
        runtime
            .control
            .group(group.group_id.clone())
            .await
            .unwrap()
            .lease
            .state,
        LeaseState::Active
    );
    runtime.release_connection_principals("connection-b").await;
    assert!(matches!(
        runtime
            .control
            .group(group.group_id.clone())
            .await
            .unwrap()
            .lease
            .state,
        LeaseState::OrphanedGrace { .. }
    ));
    runtime
        .register_principal_connection("connection-c", principal.clone())
        .await;
    let resumed = runtime.default_group(&principal, logical, &browser).await;
    assert_eq!(resumed.lease.state, LeaseState::Active);
    assert_eq!(resumed.group_id, group.group_id);
}

#[tokio::test]
async fn caller_lanes_with_the_same_session_own_distinct_groups_on_one_browser() {
    use super::integration::{BrowserControlRuntime, principal_from_mcp};
    use sky_cua_platform::model::{
        BrowserCallerKind, BrowserCallerProvenance, BrowserLogicalIdentity,
        BrowserOperationIdentity, BrowserProvenanceSource, BrowserRequestContext,
    };

    let runtime = BrowserControlRuntime::new();
    let browser = BrowserInstanceId::from("shared-extension-actor");
    let logical = "session:shared-session:thread:shared-thread";
    let callers = [
        BrowserCallerKind::CodexDesktop,
        BrowserCallerKind::OpenClaw,
        BrowserCallerKind::OpenCode,
        BrowserCallerKind::DirectMcp,
    ];
    let mut principals = Vec::new();
    let mut groups = Vec::new();
    for (index, caller) in callers.into_iter().enumerate() {
        let connection_id = format!("connection-{index}");
        let context = BrowserRequestContext {
            provenance: BrowserCallerProvenance {
                caller,
                source: BrowserProvenanceSource::InstallerDeclaration,
                connection_id: connection_id.clone(),
                declared_caller: None,
                client_info: None,
            },
            logical_identity: BrowserLogicalIdentity {
                session_id: "shared-session".to_owned(),
                thread_id: Some("shared-thread".to_owned()),
                turn_id: None,
            },
            operation_identity: BrowserOperationIdentity {
                operation_id: format!("operation-{index}"),
                request_id_fingerprint: format!("fingerprint-{index}"),
            },
        };
        let principal = principal_from_mcp(&context);
        groups.push(
            runtime
                .default_group(&principal, logical, &browser)
                .await
                .group_id,
        );
        principals.push(principal.id);
    }

    principals.sort();
    principals.dedup();
    groups.sort();
    groups.dedup();
    assert_eq!(principals.len(), 4);
    assert_eq!(groups.len(), 4);
}

#[tokio::test]
async fn actor_event_lag_records_diagnostic_and_continues_processing() {
    use super::integration::{BrowserControlRuntime, spawn_actor_event_receiver_for_test};
    use sky_cua_platform::model::BrowserControlEventKind;
    use tokio::sync::broadcast;

    let runtime = BrowserControlRuntime::new();
    let actor = BridgeActor::spawn(BridgeActorConfig::new(
        std::env::temp_dir().join("missing-lag-test.sock"),
        1,
    ));
    let (sender, receiver) = broadcast::channel(1);
    let health = actor.health();
    sender
        .send(super::BridgeActorEvent::State(health.clone()))
        .unwrap();
    sender.send(super::BridgeActorEvent::State(health)).unwrap();
    spawn_actor_event_receiver_for_test(
        actor,
        receiver,
        Arc::clone(&runtime.shared),
        runtime.control.clone(),
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    let events = runtime.control.events.snapshot().events;
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        BrowserControlEventKind::MigrationDiagnostic { code }
            if code.starts_with("actor_event_lagged_fail_closed:")
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(&event.kind, BrowserControlEventKind::BridgeState { .. }))
    );
}

#[tokio::test]
async fn missing_socket_prunes_stale_actor_registry_entry() {
    use super::integration::{ActorEntry, BrowserControlRuntime};

    let dir = std::env::temp_dir().join(format!("sky-cua-stale-actor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    unsafe {
        std::env::set_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV, &dir);
        std::env::set_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV, "all");
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
    let missing = dir.join("extension-stale.sock");
    let actor = BridgeActor::spawn(BridgeActorConfig::new(missing.clone(), 1));
    let actor_probe = actor.clone();
    let runtime = BrowserControlRuntime::new();
    runtime.shared.actors.write().unwrap().insert(
        missing.clone(),
        ActorEntry {
            actor,
            socket: missing,
            browser_id: "stale-browser".to_owned(),
        },
    );
    assert!(runtime.ready_actors().await.is_err());
    assert!(runtime.shared.actors.read().unwrap().is_empty());
    tokio::time::timeout(Duration::from_secs(1), actor_probe.wait_closed())
        .await
        .expect("retired actor task stops");
    unsafe {
        std::env::remove_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV);
        std::env::remove_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV);
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn integration_executor_ignores_selected_actor_until_it_is_ready() {
    use super::integration::{ActorEntry, IntegrationExecutor, Shared};
    use super::operation::{DispatchOperation, OperationIdentity};

    let path = std::env::temp_dir().join(format!(
        "sky-cua-unready-actor-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, b"not-a-socket").unwrap();
    let actor = BridgeActor::spawn(BridgeActorConfig::new(path.clone(), 1));
    let shared = Arc::new(Shared::default());
    shared.actors.write().unwrap().insert(
        path.clone(),
        ActorEntry {
            actor,
            socket: path.clone(),
            browser_id: "unready-browser".to_owned(),
        },
    );
    let executor = IntegrationExecutor { shared };
    let outcome = executor
        .execute(DispatchOperation {
            identity: OperationIdentity {
                operation_id: OperationId::from("unready-operation"),
                daemon_generation: "generation".to_owned(),
                canonical_fingerprint: "fingerprint".to_owned(),
                upstream: UpstreamCorrelation {
                    ingress: "test".to_owned(),
                    request_id: None,
                },
            },
            client_id: ClientId::from("client"),
            principal: Principal::new("principal", unsafe { libc::geteuid() }),
            group_id: None,
            scope: OperationScope::BridgeGlobal(BrowserInstanceId::from("unready-browser")),
            class: OperationClass::ReadOnly,
            payload: serde_json::to_string(&json!({
                "kind":"raw",
                "method":"getInfo",
                "params":{},
                "timeout_ms":10,
                "identity":{
                    "session_id":"test-session",
                    "turn_id":"test-turn"
                }
            }))
            .unwrap(),
        })
        .await;
    assert_eq!(
        outcome,
        ExecutorOutcome::DefinitiveFailure("persistent bridge unavailable".to_owned())
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn control_plane_snapshot_aggregates_actor_health_and_normalized_clients() {
    use super::integration::{ActorEntry, BrowserControlRuntime};
    use sky_cua_platform::model::{
        BrowserCallerKind, BrowserCallerProvenance, BrowserMcpClientInfo, BrowserProvenanceSource,
    };

    let fixture = SocketFixture::new("introspection-health");
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let release = Arc::new(Notify::new());
    let server_release = Arc::clone(&release);
    let server = tokio::spawn(async move {
        let _stream = accept_hello(&listener, "browser-health").await;
        server_release.notified().await;
    });
    let mut config = BridgeActorConfig::new(fixture.0.clone(), 7);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    let runtime = BrowserControlRuntime::new();
    runtime.shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor,
            socket: fixture.0.clone(),
            browser_id: "browser-health".to_owned(),
        },
    );
    runtime.record_mcp_client(&BrowserCallerProvenance {
        caller: BrowserCallerKind::LegacyUnknown,
        source: BrowserProvenanceSource::LegacyFallback,
        connection_id: "mcp-legacy".to_owned(),
        declared_caller: Some("unexpected-host".to_owned()),
        client_info: Some(BrowserMcpClientInfo {
            name: "third-party".to_owned(),
            version: "1".to_owned(),
            title: Some("not exposed".to_owned()),
        }),
    });
    runtime
        .record_raw_client_open(&BrowserCallerProvenance {
            caller: BrowserCallerKind::CodexDesktop,
            source: BrowserProvenanceSource::RequestMetadataDeclaration,
            connection_id: "codex-1".to_owned(),
            declared_caller: Some("codex_desktop".to_owned()),
            client_info: None,
        })
        .unwrap();
    assert!(
        runtime
            .record_raw_client_open(&BrowserCallerProvenance {
                caller: BrowserCallerKind::OpenCode,
                source: BrowserProvenanceSource::RequestMetadataDeclaration,
                connection_id: "codex-1".to_owned(),
                declared_caller: Some("opencode".to_owned()),
                client_info: None,
            })
            .is_err(),
        "one raw connection must not switch caller ownership lanes"
    );

    let snapshot = runtime.control_plane_snapshot().await;
    assert!(snapshot.ready);
    assert_eq!(snapshot.actors.len(), 1);
    assert_eq!(
        snapshot.actors[0].browser_instance_id.as_deref(),
        Some("browser-health")
    );
    assert_eq!(
        snapshot.actors[0].host_instance_id.as_deref(),
        Some("host-integration")
    );
    assert_eq!(snapshot.actors[0].actor_generation, 7);
    assert_eq!(
        snapshot.actors[0].transport,
        sky_cua_platform::model::BrowserBridgeTransport::ExtensionNativeHost
    );
    assert!(snapshot.actors[0].protocol_capable);
    assert!(snapshot.actors[0].canonical);
    assert_eq!(snapshot.client_count, 2);
    assert_eq!(snapshot.clients.len(), 2);
    assert!(snapshot.clients.iter().any(|client| {
        client.connection_id == "mcp-legacy"
            && client.caller == BrowserCallerKind::LegacyUnknown
            && client.provenance_source == BrowserProvenanceSource::LegacyFallback
            && client.surface == sky_cua_platform::model::BrowserClientSurface::McpTools
            && client.declared_label.as_deref() == Some("unexpected-host")
            && client.client_info_label.as_deref() == Some("third-party/1")
            && client.client_info.as_ref().is_some_and(|info| {
                info.name == "third-party"
                    && info.version == "1"
                    && info.title.as_deref() == Some("not exposed")
            })
    }));
    assert!(snapshot.clients.iter().any(|client| {
        client.connection_id == "codex-1"
            && client.caller == BrowserCallerKind::CodexDesktop
            && client.surface == sky_cua_platform::model::BrowserClientSurface::HostProvidedIab
            && client.provenance_source == BrowserProvenanceSource::RequestMetadataDeclaration
    }));

    let client_events = snapshot
        .events
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            sky_cua_platform::model::BrowserControlEventKind::ClientState { state, client } => {
                Some((state.as_str(), client))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(client_events.iter().any(|(state, client)| {
        *state == "mcp_connected"
            && client.connection_id == "mcp-legacy"
            && client.declared_label.as_deref() == Some("unexpected-host")
            && client.client_info_label.as_deref() == Some("third-party/1")
    }));

    runtime.record_client_closed("codex-1");
    let after_close = runtime.control_plane_snapshot().await;
    assert!(after_close.events.events.iter().any(|event| matches!(
        &event.kind,
        sky_cua_platform::model::BrowserControlEventKind::ClientState { state, client }
            if state == "raw_native_pipe_disconnected"
                && client.connection_id == "codex-1"
                && client.surface
                    == sky_cua_platform::model::BrowserClientSurface::HostProvidedIab
    )));
    assert!(client_events.iter().any(|(state, client)| {
        *state == "raw_native_pipe_connected"
            && client.connection_id == "codex-1"
            && client.surface == sky_cua_platform::model::BrowserClientSurface::HostProvidedIab
    }));

    release.notify_one();
    server.await.unwrap();
}

#[test]
fn installed_manifest_without_ready_actor_is_not_operational() {
    use super::integration::persistent_target_availability;
    use sky_cua_platform::model::{BrowserIntegrationReport, DoctorCheck};

    let check = |name: &str, detail: &str| DoctorCheck {
        name: name.to_owned(),
        ok: true,
        detail: detail.to_owned(),
    };
    let integration = BrowserIntegrationReport {
        chrome: check("chrome", "/usr/bin/google-chrome"),
        chromium: check("chromium", "missing"),
        brave: check("brave", "missing"),
        native_host_manifest: check("native-host", "installed"),
    };

    let availability = persistent_target_availability(false, Some(&integration));
    assert!(!availability.available);
    assert!(
        availability
            .detail
            .contains("No canonical browser actor is ready")
    );
    assert!(availability.detail.contains("installed"));
}
