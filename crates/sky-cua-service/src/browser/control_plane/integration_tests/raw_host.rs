use super::*;

#[tokio::test]
async fn raw_host_seams_enforce_ownership_provenance_validation_and_no_replay() {
    use super::super::integration::{ActorEntry, BrowserControlRuntime, spawn_actor_events};
    use crate::codex_browser_compat::{
        CodexBackendReply, CodexBrowserBackend, CodexConnectionContext, CodexLogicalIdentity,
        CodexNormalizedRequest, CodexOperationClass, CodexOperationScope,
    };
    use sky_cua_platform::model::{
        BrowserCallerKind, BrowserCallerProvenance, BrowserMcpClientInfo, BrowserProvenanceSource,
    };

    fn request(
        connection: &CodexConnectionContext,
        operation_id: &str,
        method: &str,
        params: Value,
        caller: BrowserCallerKind,
        logical_identity: (&str, &str),
        scope: CodexOperationScope,
    ) -> CodexNormalizedRequest {
        let (session, thread) = logical_identity;
        CodexNormalizedRequest {
            operation_id: operation_id.to_owned(),
            upstream_id: operation_id.len() as u64,
            method: method.to_owned(),
            raw_request: json!({"jsonrpc":"2.0","id":operation_id.len(),"method":method,"params":params}),
            params,
            connection: connection.clone(),
            logical_identity: CodexLogicalIdentity {
                session_id: Some(session.to_owned()),
                thread_id: Some(thread.to_owned()),
                turn_id: Some(format!("turn-{operation_id}")),
            },
            caller_provenance: BrowserCallerProvenance {
                caller,
                source: BrowserProvenanceSource::RequestMetadataDeclaration,
                connection_id: connection.connection_id.clone(),
                declared_caller: Some(
                    match caller {
                        BrowserCallerKind::OpenCode => "opencode",
                        BrowserCallerKind::OpenClaw => "openclaw",
                        _ => "direct_mcp",
                    }
                    .to_owned(),
                ),
                client_info: Some(BrowserMcpClientInfo {
                    name: "host-seam-test".to_owned(),
                    version: "1".to_owned(),
                    title: None,
                }),
            },
            identity_synthetic: true,
            class: CodexOperationClass::Mutation,
            scope,
            canonical_fingerprint: format!("fingerprint-{operation_id}"),
            deadline: Duration::from_secs(2),
        }
    }

    fn auth_params(origin: &str, selectors_valid_case: &str) -> Value {
        json!({
            "tabId":44,
            "origin":origin,
            "reason":format!("Sign in ({selectors_valid_case})"),
            "expires_at":(chrono::Utc::now() + chrono::Duration::minutes(2)).to_rfc3339(),
            "fields":[{
                "id":"username",
                "label":"Email",
                "type":"email",
                "autocomplete":"username",
                "required":true,
                "selector":"input[name=email]"
            }],
            "submit":{"selector":"button[type=submit]","action":"click"}
        })
    }

    let fixture = SocketFixture::new("raw-host-seams");
    let socket_dir = fixture.0.parent().unwrap().to_path_buf();
    unsafe {
        std::env::set_var(crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV, &socket_dir);
        std::env::set_var(crate::browser::sockets::SKY_CUA_BROWSER_ENV, "all");
    }
    crate::browser::sockets::reset_socket_inventory_for_tests();
    let listener = UnixListener::bind(&fixture.0).unwrap();
    let late_settlement = Arc::new(Notify::new());
    let server_settlement = Arc::clone(&late_settlement);
    let runtime = BrowserControlRuntime::new_with_limits(QueueLimits::default());
    let daemon_generation = runtime.daemon_generation();
    let server_generation = daemon_generation.clone();
    let server = tokio::spawn(async move {
        let mut stream = accept_hello(&listener, "browser-raw-host-seams").await;
        server_settlement.notified().await;
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc":"2.0",
                "method":"skyCuaHost/settlement",
                "params":{
                    "operation_id":"report-operation",
                    "daemon_generation":server_generation,
                    "actor_generation":1,
                    "chrome_request_id":"late-report-settlement",
                    "target_lifetime_key":{
                        "browser_instance_id":"browser-raw-host-seams",
                        "tab_id":"44"
                    },
                    "operation_class":"mutation",
                    "completion":{"result":{"status":"reported"}}
                }
            }),
        )
        .await
        .unwrap();
        let ack = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(ack["method"], "skyCuaHost/settlementAck");
        assert_eq!(ack["params"]["operation_id"], "report-operation");

        for (expected_origin, selectors_valid) in [
            ("https://example.test", true),
            ("https://changed.test", true),
            ("https://example.test", false),
        ] {
            let preflight = read_frame(&mut stream).await.unwrap().unwrap();
            assert_eq!(preflight["method"], "executeCdp");
            assert_eq!(preflight["params"]["method"], "Runtime.evaluate");
            assert_eq!(preflight["params"]["target"]["tabId"], 44);
            assert_eq!(
                preflight["params"]["_sky_cua_host_request"]["operation_class"],
                "read_only"
            );
            let encoded = preflight["params"]["commandParams"]["expression"]
                .as_str()
                .unwrap();
            assert!(!encoded.contains("credential"));
            assert!(!encoded.contains("password"));
            write_frame(
                &mut stream,
                &json!({
                    "jsonrpc":"2.0",
                    "id":preflight["id"],
                    "result":{"result":{"value":{
                        "origin":expected_origin,
                        "selectorsValid":selectors_valid
                    }}}
                }),
            )
            .await
            .unwrap();
        }

        assert!(
            tokio::time::timeout(Duration::from_millis(150), read_frame(&mut stream))
                .await
                .is_err(),
            "local host seams and rejected requests must not be forwarded or replayed"
        );
    });

    let mut config = BridgeActorConfig::new(fixture.0.clone(), 1);
    config.heartbeat_interval = Duration::from_secs(30);
    let mut actor = BridgeActor::spawn(config);
    actor.wait_until_ready().await.unwrap();
    runtime.shared.actors.write().unwrap().insert(
        fixture.0.clone(),
        ActorEntry {
            actor: actor.clone(),
            socket: fixture.0.clone(),
            browser_id: "browser-raw-host-seams".to_owned(),
        },
    );
    spawn_actor_events(
        actor.clone(),
        Arc::clone(&runtime.shared),
        runtime.control.clone(),
    );

    let primary = CodexConnectionContext {
        connection_id: "opencode-host-seam".to_owned(),
        ingress: "raw_native_pipe",
        peer_uid: unsafe { libc::geteuid() },
        codex_app_build_flavor: None,
        daemon_generation: daemon_generation.clone(),
    };
    let (outbound, _outbound_rx) = crate::codex_browser_compat::CodexOutbound::channel();
    runtime
        .connection_opened(primary.clone(), outbound)
        .await
        .unwrap();
    let owner = Principal::new("raw:opencode:session:session-a", unsafe { libc::geteuid() });
    let browser = BrowserInstanceId::from("browser-raw-host-seams");
    let tab = TabKey::new(browser.clone(), "44");
    let logical_group = super::super::integration::logical_group_key("session-a", Some("thread-a"));
    let group_id = runtime
        .default_group(&owner, &logical_group, &browser)
        .await
        .group_id;
    runtime
        .control
        .add_member(group_id.clone(), owner.clone(), tab.clone())
        .await
        .unwrap();
    runtime
        .shared
        .tab_owners
        .lock()
        .await
        .insert(tab, group_id.clone());
    runtime.initialize_ownership_indexes().await;
    assert_eq!(
        runtime
            .shared
            .tab_owners
            .lock()
            .await
            .get(&TabKey::new("browser-raw-host-seams", "44")),
        Some(&group_id)
    );

    let report = runtime
        .request(request(
            &primary,
            "report-operation",
            "reportBotDetection",
            json!({"tabId":44,"reason":"challenge_loop","hostname":"example.test"}),
            BrowserCallerKind::OpenCode,
            ("session-a", "thread-a"),
            CodexOperationScope::Tab("44".to_owned()),
        ))
        .await;
    assert_eq!(
        report,
        CodexBackendReply::Result(json!({"status":"reported","hostname":"example.test"}))
    );

    let snapshot = runtime.control_plane_snapshot().await;
    assert!(snapshot.clients.iter().any(|client| {
        client.connection_id == primary.connection_id
            && client.caller == BrowserCallerKind::OpenCode
            && client.provenance_source == BrowserProvenanceSource::RequestMetadataDeclaration
    }));
    let report_event = snapshot
        .events
        .events
        .iter()
        .find(|event| {
            matches!(
                &event.kind,
                sky_cua_platform::model::BrowserControlEventKind::OperationState { state }
                    if state == "bot_detection_reported:challenge_loop:example.test"
            )
        })
        .expect("bot report is observable");
    assert_eq!(
        report_event.operation_id.as_deref(),
        Some("report-operation")
    );
    assert_eq!(
        report_event.principal_id.as_deref(),
        Some("raw:opencode:session:session-a")
    );
    assert!(
        report_event
            .group_id
            .as_deref()
            .is_some_and(|group| group.contains("session:session-a:thread:thread-a"))
    );
    assert_eq!(
        report_event
            .tab_key
            .as_ref()
            .map(|tab| tab.extension_tab_id.as_str()),
        Some("44")
    );

    late_settlement.notify_one();
    assert_eq!(
        runtime
            .request(request(
                &primary,
                "auth-unavailable",
                "browserAuthHandoff",
                auth_params("https://example.test", "unavailable"),
                BrowserCallerKind::OpenCode,
                ("session-a", "thread-a"),
                CodexOperationScope::Tab("44".to_owned()),
            ))
            .await,
        CodexBackendReply::Result(json!({"status":"unavailable"}))
    );
    assert_eq!(
        runtime
            .request(request(
                &primary,
                "auth-origin-changed",
                "browserAuthHandoff",
                auth_params("https://example.test", "origin"),
                BrowserCallerKind::OpenCode,
                ("session-a", "thread-a"),
                CodexOperationScope::Tab("44".to_owned()),
            ))
            .await,
        CodexBackendReply::Result(json!({"status":"origin_changed"}))
    );
    assert_eq!(
        runtime
            .request(request(
                &primary,
                "auth-locator-invalid",
                "browserAuthHandoff",
                auth_params("https://example.test", "locator"),
                BrowserCallerKind::OpenCode,
                ("session-a", "thread-a"),
                CodexOperationScope::Tab("44".to_owned()),
            ))
            .await,
        CodexBackendReply::Result(json!({"status":"locator_invalid"}))
    );

    let mut expired = auth_params("https://example.test", "expired");
    expired["expires_at"] = json!((chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339());
    assert_eq!(
        runtime
            .request(request(
                &primary,
                "auth-expired",
                "browserAuthHandoff",
                expired,
                BrowserCallerKind::OpenCode,
                ("session-a", "thread-a"),
                CodexOperationScope::Tab("44".to_owned()),
            ))
            .await,
        CodexBackendReply::Result(json!({"status":"expired"}))
    );
    let mut credential = auth_params("https://example.test", "credential");
    credential["fields"][0]["value"] = json!("must-not-escape");
    let rejected = runtime
        .request(request(
            &primary,
            "auth-credential-rejected",
            "browserAuthHandoff",
            credential,
            BrowserCallerKind::OpenCode,
            ("session-a", "thread-a"),
            CodexOperationScope::Tab("44".to_owned()),
        ))
        .await;
    let CodexBackendReply::Error(error) = rejected else {
        panic!("credential-bearing request must be rejected")
    };
    assert!(!error.to_string().contains("must-not-escape"));

    let secondary = CodexConnectionContext {
        connection_id: "openclaw-host-seam".to_owned(),
        ingress: "raw_native_pipe",
        peer_uid: unsafe { libc::geteuid() },
        codex_app_build_flavor: None,
        daemon_generation,
    };
    let (secondary_outbound, _secondary_rx) = crate::codex_browser_compat::CodexOutbound::channel();
    runtime
        .connection_opened(secondary.clone(), secondary_outbound)
        .await
        .unwrap();
    let ownership_rejection = runtime
        .request(request(
            &secondary,
            "foreign-report",
            "reportBotDetection",
            json!({"tabId":44,"reason":"access_denied","hostname":"example.test"}),
            BrowserCallerKind::OpenClaw,
            ("session-b", "thread-b"),
            CodexOperationScope::Tab("44".to_owned()),
        ))
        .await;
    assert!(matches!(ownership_rejection, CodexBackendReply::Error(_)));

    let bad_reason = runtime
        .request(request(
            &primary,
            "bad-report",
            "reportBotDetection",
            json!({"tabId":44,"reason":"free_form","hostname":"example.test"}),
            BrowserCallerKind::OpenCode,
            ("session-a", "thread-a"),
            CodexOperationScope::Tab("44".to_owned()),
        ))
        .await;
    assert!(matches!(bad_reason, CodexBackendReply::Error(_)));

    server.await.unwrap();
    actor.shutdown().await;
}
