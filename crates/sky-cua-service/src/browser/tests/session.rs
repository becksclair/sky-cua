//! Session ownership, claim/reclaim, and CDP recovery tests.

use std::time::Duration;

use serde_json::{Value, json};
use sky_cua_platform::model::BrowserTargetKind;
use tokio::net::UnixListener;

use crate::browser::bridge::{claim_tab, move_mouse, screenshot, snapshot};
use crate::browser::protocol::{read_frame, write_frame};
use crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV;

use super::helpers::*;

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

#[tokio::test]
async fn cdp_action_recovers_when_tab_is_not_in_browser_session() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-cdp-recover-session");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, first_metrics) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            first_metrics.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(first_metrics["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(first_metrics["params"]["target"]["tabId"], 515);
        assert_eq!(first_metrics["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": first_metrics["id"],
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
                    "title": "Recovered Tab",
                    "url": "https://example.test/recovered",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();

        reply_to_detach(&mut stream, 515).await;
        reply_to_attach_and_enable(&mut stream, 515).await;
        reply_to_viewport_metrics(&mut stream, 515, 100.0, 80.0, 2.0).await;

        let retried_screenshot = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            retried_screenshot.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(retried_screenshot["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(retried_screenshot["params"]["target"]["tabId"], 515);
        assert_eq!(
            retried_screenshot["params"]["method"],
            "Page.captureScreenshot"
        );
        let capture_params = &retried_screenshot["params"]["commandParams"];
        assert_eq!(capture_params["captureBeyondViewport"], true);
        assert_eq!(capture_params["clip"]["width"], 100.0);
        assert_eq!(capture_params["clip"]["height"], 80.0);
        assert_eq!(capture_params["clip"]["scale"], 1);
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": retried_screenshot["id"],
                "result": {"data": test_png_base64(200, 160)}
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = screenshot(Some(BrowserTargetKind::UserChrome), "515".to_string()).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    // The capture is normalized to CSS-pixel dimensions and re-encoded with
    // the model screenshot knobs (JPEG by default).
    assert_eq!(response.mime_type, "image/jpeg");
    assert_eq!(response.width, Some(100));
    assert_eq!(response.height, Some(80));
    assert!(!response.data_base64.is_empty());
    let screenshot_path = response
        .screenshot_path
        .as_deref()
        .expect("screenshot should be persisted to disk");
    assert!(std::path::Path::new(screenshot_path).exists());
    let _ = std::fs::remove_file(screenshot_path);
}

#[tokio::test]
async fn cdp_action_recovers_when_debugger_is_unattached() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-cdp-recover-debugger");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, first_snapshot) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            first_snapshot.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(first_snapshot["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(first_snapshot["params"]["target"]["tabId"], 515);
        assert_eq!(first_snapshot["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": first_snapshot["id"],
                "error": {
                    "code": 1,
                    "message": "Debugger is not attached to the tab with id: 515."
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
                    "title": "Recovered Snapshot Tab",
                    "url": "https://example.test/snapshot",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();

        reply_to_detach(&mut stream, 515).await;
        reply_to_attach_and_enable(&mut stream, 515).await;

        let retried_snapshot = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            retried_snapshot.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(retried_snapshot["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(retried_snapshot["params"]["target"]["tabId"], 515);
        assert_eq!(retried_snapshot["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": retried_snapshot["id"],
                "result": {
                    "result": {
                        "type": "object",
                        "value": {
                            "title": "Recovered Snapshot Tab",
                            "url": "https://example.test/snapshot",
                            "viewport": {"width": 800, "height": 600, "devicePixelRatio": 1},
                            "text": "ready",
                            "elements": []
                        }
                    }
                }
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = snapshot(Some(BrowserTargetKind::UserChrome), "515".to_string()).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.title.as_deref(), Some("Recovered Snapshot Tab"));
    assert_eq!(
        response.url.as_deref(),
        Some("https://example.test/snapshot")
    );
}

#[tokio::test]
async fn cdp_action_recovery_reclaims_stale_sky_cua_owner() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-cdp-recover-stale-owner");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, first_snapshot) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            first_snapshot.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(first_snapshot["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": first_snapshot["id"],
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
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": claim["id"],
                "error": {
                    "code": 1,
                    "message": "Tab 515 is already part of browser session sky-cua-old-agent"
                }
            }),
        )
        .await
        .unwrap();

        let finalize = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            finalize.get("method").and_then(Value::as_str),
            Some("finalizeTabs")
        );
        assert_eq!(finalize["params"]["session_id"], "sky-cua-old-agent");
        assert_eq!(finalize["params"]["keep"], json!([]));
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": finalize["id"], "result": {}}),
        )
        .await
        .unwrap();

        let retry_claim = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            retry_claim.get("method").and_then(Value::as_str),
            Some("claimUserTab")
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": retry_claim["id"],
                "result": {
                    "id": 515,
                    "title": "Recovered Stale Owner Tab",
                    "url": "https://example.test/stale-owner",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();

        reply_to_detach(&mut stream, 515).await;
        reply_to_attach_and_enable(&mut stream, 515).await;

        let retried_snapshot = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            retried_snapshot.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(retried_snapshot["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": retried_snapshot["id"],
                "result": {
                    "result": {
                        "type": "object",
                        "value": {
                            "title": "Recovered Stale Owner Tab",
                            "url": "https://example.test/stale-owner",
                            "viewport": {"width": 800, "height": 600, "devicePixelRatio": 1},
                            "text": "ready",
                            "elements": []
                        }
                    }
                }
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = snapshot(Some(BrowserTargetKind::UserChrome), "515".to_string()).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.title.as_deref(), Some("Recovered Stale Owner Tab"));
}
