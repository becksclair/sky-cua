//! Session ownership and claim/reclaim tests; CDP action recovery lives in
//! `session_recovery.rs`.

use std::time::Duration;

use serde_json::{Value, json};
use sky_cua_platform::model::BrowserTargetKind;
use tokio::net::UnixListener;

use super::helpers::*;
use crate::browser::bridge::{claim_tab, move_mouse};
use crate::browser::protocol::{read_frame, write_frame};
use crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV;

#[tokio::test]
async fn claim_tab_adopts_existing_user_chrome_tab() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-claim-tab");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, claim) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            claim.get("method").and_then(Value::as_str),
            Some("claimUserTab")
        );
        assert_eq!(claim["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(claim["params"]["tabId"], 515);
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": claim["id"],
                "result": {
                    "id": 515,
                    "title": "Claimed Tab",
                    "url": "https://example.test/claimed",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();
        reply_to_attach_and_enable(&mut stream, 515).await;
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = claim_tab(Some(BrowserTargetKind::UserChrome), "515".to_string()).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    let tab = response.tab.expect("claimed tab should be returned");
    assert_eq!(tab.tab_id, "515");
    assert_eq!(tab.title.as_deref(), Some("Claimed Tab"));
    assert_eq!(tab.url.as_deref(), Some("https://example.test/claimed"));
    assert!(tab.active);
}

#[tokio::test]
async fn claim_tab_releases_stale_sky_cua_session_and_retries() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-claim-tab-reclaim");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;
        let claim = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            claim.get("method").and_then(Value::as_str),
            Some("claimUserTab")
        );
        assert_eq!(claim["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(claim["params"]["tabId"], 515);
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": claim["id"],
                "error": {
                    "code": 1,
                    "message": "Tab 515 is already part of browser session sky-cua-cursor-proof"
                }
            }),
        )
        .await
        .unwrap();

        let Ok(Ok(Some(finalize))) =
            tokio::time::timeout(Duration::from_millis(250), read_frame(&mut stream)).await
        else {
            return;
        };
        assert_eq!(
            finalize.get("method").and_then(Value::as_str),
            Some("finalizeTabs")
        );
        assert_eq!(finalize["params"]["session_id"], "sky-cua-cursor-proof");
        assert_eq!(finalize["params"]["keep"], json!([]));
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": finalize["id"], "result": {}}),
        )
        .await
        .unwrap();

        let retry = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            retry.get("method").and_then(Value::as_str),
            Some("claimUserTab")
        );
        assert_eq!(retry["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(retry["params"]["tabId"], 515);
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": retry["id"],
                "result": {
                    "id": 515,
                    "title": "Reclaimed Tab",
                    "url": "https://example.test/reclaimed",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();
        reply_to_attach_and_enable(&mut stream, 515).await;
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = claim_tab(Some(BrowserTargetKind::UserChrome), "515".to_string()).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    let tab = response.tab.expect("reclaimed tab should be returned");
    assert_eq!(tab.tab_id, "515");
    assert_eq!(tab.title.as_deref(), Some("Reclaimed Tab"));
    assert_eq!(tab.url.as_deref(), Some("https://example.test/reclaimed"));
    assert!(tab.active);
}

#[tokio::test]
async fn claim_tab_does_not_reclaim_non_sky_cua_session() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-claim-tab-no-reclaim");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;
        let claim = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            claim.get("method").and_then(Value::as_str),
            Some("claimUserTab")
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": claim["id"],
                "error": {
                    "code": 1,
                    "message": "Tab 515 is already part of browser session codex-browser-use"
                }
            }),
        )
        .await
        .unwrap();

        match tokio::time::timeout(Duration::from_millis(100), read_frame(&mut stream)).await {
            Ok(Ok(Some(extra))) => {
                panic!("non-sky-cua owners must not be finalized by claim retry: {extra:?}")
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(error)) => panic!("unexpected socket read error: {error}"),
        }
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = claim_tab(Some(BrowserTargetKind::UserChrome), "515".to_string()).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.tab.is_none());
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserBridgeRequestFailed");
    assert!(
        response.diagnostics[0]
            .message
            .contains("codex-browser-use")
    );
}

#[tokio::test]
async fn claim_tab_reattaches_when_page_enable_finds_stale_debugger_state() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-claim-tab-reattach");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;
        let claim = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            claim.get("method").and_then(Value::as_str),
            Some("claimUserTab")
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": claim["id"],
                "result": {
                    "id": 515,
                    "title": "Reattached Tab",
                    "url": "https://example.test/reattach",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();

        let attach = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(attach.get("method").and_then(Value::as_str), Some("attach"));
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": attach["id"], "result": {}}),
        )
        .await
        .unwrap();

        let enable = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            enable.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(enable["params"]["method"], "Page.enable");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": enable["id"],
                "error": {
                    "code": 1,
                    "message": "Debugger unattached"
                }
            }),
        )
        .await
        .unwrap();

        let detach = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(detach.get("method").and_then(Value::as_str), Some("detach"));
        assert_eq!(detach["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(detach["params"]["tabId"], 515);
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": detach["id"], "result": {}}),
        )
        .await
        .unwrap();

        reply_to_attach_and_enable(&mut stream, 515).await;
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = claim_tab(Some(BrowserTargetKind::UserChrome), "515".to_string()).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    let tab = response.tab.expect("claimed tab should be returned");
    assert_eq!(tab.tab_id, "515");
    assert_eq!(tab.title.as_deref(), Some("Reattached Tab"));
}

#[tokio::test]
async fn move_mouse_recovers_when_bridge_finds_stale_session() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-move-mouse-recover");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, first_move) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            first_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        assert_eq!(first_move["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(first_move["params"]["tabId"], 515);
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": first_move["id"],
                "error": {
                    "code": 1,
                    "message": "Tab 515 is not part of browser session sky-cua-mcp."
                }
            }),
        )
        .await
        .unwrap();

        let claim = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            claim.get("method").and_then(Value::as_str),
            Some("claimUserTab")
        );
        assert_eq!(claim["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(claim["params"]["tabId"], 515);
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": claim["id"],
                "result": {
                    "id": 515,
                    "title": "Recovered Move Tab",
                    "url": "https://example.test/move",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();

        reply_to_detach(&mut stream, 515).await;
        reply_to_attach_and_enable(&mut stream, 515).await;

        let move_mouse = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            move_mouse.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        assert_eq!(move_mouse["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(move_mouse["params"]["tabId"], 515);
        assert_eq!(move_mouse["params"]["x"], 240.0);
        assert_eq!(move_mouse["params"]["y"], 160.0);
        assert_eq!(move_mouse["params"]["waitForArrival"], true);
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": move_mouse["id"], "result": {}}),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = move_mouse(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        240.0,
        160.0,
        true,
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.tab_id, "515");
    assert_eq!(response.x, 240.0);
    assert_eq!(response.y, 160.0);
}
