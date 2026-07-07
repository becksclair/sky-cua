//! CDP action recovery tests: wedged or lost debugger sessions are reset
//! (claim, detach, attach, Page.enable) and the action is retried only when
//! replaying it cannot mutate the page twice.

use serde_json::{Value, json};
use sky_cua_platform::model::BrowserTargetKind;
use tokio::net::UnixListener;

use crate::browser::bridge::{claim_tab, click, screenshot, snapshot};
use crate::browser::protocol::{read_frame, write_frame};
use crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV;

use super::helpers::*;

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
    let response = screenshot(Some(BrowserTargetKind::UserChrome), "515".to_string(), true).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    // The capture is normalized to CSS-pixel dimensions and re-encoded with
    // the model screenshot knobs (WebP by default).
    assert_eq!(response.mime_type, "image/webp");
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
async fn cdp_action_recovers_when_cdp_command_times_out() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-cdp-recover-timeout");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, first_metrics) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            first_metrics.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(first_metrics["params"]["method"], "Runtime.evaluate");
        // The command budget is derived from the remaining call deadline
        // (2s under test) minus the 750ms response margin, so it must come
        // in at or under 1250ms — a hardcoded 10s budget would fail here.
        let timeout_ms = first_metrics["params"]["timeoutMs"]
            .as_u64()
            .expect("executeCdp carries a timeoutMs budget");
        assert!(
            (250..=1_250).contains(&timeout_ms),
            "timeoutMs {timeout_ms} is not derived from the call deadline"
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": first_metrics["id"],
                "result": {
                    "result": {
                        "value": {
                            "width": 100.0,
                            "height": 80.0,
                            "devicePixelRatio": 2.0
                        }
                    }
                }
            }),
        )
        .await
        .unwrap();

        let capture = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(capture["params"]["method"], "Page.captureScreenshot");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": capture["id"],
                "error": {
                    "code": 1,
                    "message": "Timed out after 1250ms waiting for CDP command Page.captureScreenshot."
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
        reply_to_attach_wake_and_enable(&mut stream, 515).await;
        reply_to_viewport_metrics(&mut stream, 515, 100.0, 80.0, 2.0).await;

        let retried_capture = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            retried_capture["params"]["method"],
            "Page.captureScreenshot"
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": retried_capture["id"],
                "result": {"data": test_png_base64(200, 160)}
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = screenshot(Some(BrowserTargetKind::UserChrome), "515".to_string(), true).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(
        response.diagnostics.is_empty(),
        "expected clean recovery, got {:?}",
        response.diagnostics
    );
    assert_eq!(response.width, Some(100));
    assert_eq!(response.height, Some(80));
    assert!(!response.data_base64.is_empty());
    if let Some(screenshot_path) = response.screenshot_path.as_deref() {
        let _ = std::fs::remove_file(screenshot_path);
    }
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

        reply_to_snapshot_request(
            &mut stream,
            515,
            "Recovered Snapshot Tab",
            "https://example.test/snapshot",
        )
        .await;
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = snapshot(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        None,
        None,
        None,
        None,
    )
    .await;
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
        assert_eq!(retry_claim["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(retry_claim["params"]["tabId"], 515);
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

        reply_to_snapshot_request(
            &mut stream,
            515,
            "Recovered Stale Owner Tab",
            "https://example.test/stale-owner",
        )
        .await;
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = snapshot(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        None,
        None,
        None,
        None,
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.title.as_deref(), Some("Recovered Stale Owner Tab"));
}

#[tokio::test]
async fn cdp_command_timeout_resets_session_without_replaying_input_action() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-cdp-timeout-no-replay");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, cursor_move) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            cursor_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": cursor_move["id"], "result": {}}),
        )
        .await
        .unwrap();
        drop(stream);

        let (mut stream, focus) = accept_until_non_info_request(&listener).await;
        ack_focus_emulation_frame(&mut stream, &focus).await;
        let mouse_move = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            mouse_move.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(mouse_move["params"]["method"], "Input.dispatchMouseEvent");
        assert_eq!(mouse_move["params"]["commandParams"]["type"], "mouseMoved");
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": mouse_move["id"], "result": {}}),
        )
        .await
        .unwrap();

        let mouse_down = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(mouse_down["params"]["method"], "Input.dispatchMouseEvent");
        assert_eq!(
            mouse_down["params"]["commandParams"]["type"],
            "mousePressed"
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": mouse_down["id"],
                "error": {
                    "code": 1,
                    "message": "Timed out after 1250ms waiting for CDP command Input.dispatchMouseEvent."
                }
            }),
        )
        .await
        .unwrap();

        // The session must still be reset so the wedged debugger session is
        // healed for the next call...
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
                    "title": "Reset Tab",
                    "url": "https://example.test/reset",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();
        reply_to_detach(&mut stream, 515).await;
        reply_to_attach_wake_and_enable(&mut stream, 515).await;

        // ...but the click must not be replayed: the timed-out dispatch may
        // still have landed in the browser, and a replay would double-click.
        assert!(
            read_frame(&mut stream).await.unwrap().is_none(),
            "input action was replayed after a CDP command timeout"
        );
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = click(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        10.0,
        20.0,
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    let diagnostic = response
        .diagnostics
        .first()
        .expect("click should surface the timeout diagnostic");
    assert!(diagnostic.message.contains("waiting for CDP command"));
    assert!(
        diagnostic
            .details
            .as_deref()
            .is_some_and(|details| details.contains("was not replayed")),
        "diagnostic should explain why the action was not replayed: {diagnostic:?}"
    );
}

#[tokio::test]
async fn cdp_detach_resets_session_without_replaying_input_action() {
    // A mid-sequence "Detached while handling command" is recoverable but is
    // NOT a CDP-command timeout. A click dispatches mouseMoved -> mousePressed
    // -> mouseReleased on one stream; if mousePressed lands and mouseReleased
    // detaches, the session must be reset but the click must not be replayed
    // (a replay re-presses the button => double activation). This pins the fix
    // for the earlier gate that only blocked replay for the timeout class.
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-cdp-detach-no-replay");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, cursor_move) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            cursor_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": cursor_move["id"], "result": {}}),
        )
        .await
        .unwrap();
        drop(stream);

        let (mut stream, focus) = accept_until_non_info_request(&listener).await;
        ack_focus_emulation_frame(&mut stream, &focus).await;
        let mouse_move = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            mouse_move.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(mouse_move["params"]["commandParams"]["type"], "mouseMoved");
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": mouse_move["id"], "result": {}}),
        )
        .await
        .unwrap();

        // mousePressed lands, then mouseReleased comes back detached.
        let mouse_down = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            mouse_down["params"]["commandParams"]["type"],
            "mousePressed"
        );
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": mouse_down["id"], "result": {}}),
        )
        .await
        .unwrap();

        let mouse_up = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(mouse_up["params"]["commandParams"]["type"], "mouseReleased");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": mouse_up["id"],
                "error": {
                    "code": 1,
                    "message": "Detached while handling command."
                }
            }),
        )
        .await
        .unwrap();

        // The wedged debugger session must still be reset for the next call...
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
                    "title": "Reset Tab",
                    "url": "https://example.test/reset",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();
        reply_to_detach(&mut stream, 515).await;
        reply_to_attach_and_enable(&mut stream, 515).await;

        // ...but the click must NOT be replayed after the detach: mousePressed
        // already landed, so a replay would double-activate the target.
        assert!(
            read_frame(&mut stream).await.unwrap().is_none(),
            "input action was replayed after a mid-sequence debugger detach"
        );
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = click(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        10.0,
        20.0,
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    let diagnostic = response
        .diagnostics
        .first()
        .expect("click should surface the detach diagnostic");
    assert!(
        diagnostic
            .message
            .contains("Detached while handling command")
    );
    assert!(
        diagnostic
            .details
            .as_deref()
            .is_some_and(|details| details.contains("was not replayed")),
        "diagnostic should explain why the action was not replayed: {diagnostic:?}"
    );
}

#[tokio::test]
async fn cdp_command_timeout_is_not_replayed_on_another_bridge_socket() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-cdp-timeout-two-sockets");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener_a = UnixListener::bind(socket_dir.join("extension-1-a.sock")).unwrap();
    let listener_b = UnixListener::bind(socket_dir.join("extension-2-b.sock")).unwrap();
    // B answers its probe only after A's exchange has fully completed, so A
    // is deterministically the first responsive socket. The signal arrives
    // well inside the bridge request timeout B's pending probe is under.
    let (a_done_tx, a_done_rx) = tokio::sync::oneshot::channel::<()>();

    let server_a = tokio::spawn(async move {
        let (mut stream, cursor_move) = accept_until_non_info_request(&listener_a).await;
        assert_eq!(
            cursor_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": cursor_move["id"], "result": {}}),
        )
        .await
        .unwrap();
        drop(stream);

        let (mut stream, focus) = accept_until_non_info_request(&listener_a).await;
        ack_focus_emulation_frame(&mut stream, &focus).await;
        let mouse_move = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(mouse_move["params"]["commandParams"]["type"], "mouseMoved");
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": mouse_move["id"], "result": {}}),
        )
        .await
        .unwrap();

        let mouse_down = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            mouse_down["params"]["commandParams"]["type"],
            "mousePressed"
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": mouse_down["id"],
                "error": {
                    "code": 1,
                    "message": "Timed out after 1250ms waiting for CDP command Input.dispatchMouseEvent."
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
                "result": {
                    "id": 515,
                    "title": "Reset Tab",
                    "url": "https://example.test/reset",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();
        reply_to_detach(&mut stream, 515).await;
        reply_to_attach_wake_and_enable(&mut stream, 515).await;
        assert!(read_frame(&mut stream).await.unwrap().is_none());
        let _ = a_done_tx.send(());
    });

    let server_b = tokio::spawn(async move {
        let (mut stream, _) = listener_b.accept().await.unwrap();
        let request = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("getInfo")
        );
        a_done_rx.await.unwrap();
        let wrote_probe_response = write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {"name": "sky-cua-test-bridge"}
            }),
        )
        .await
        .is_ok();
        // The successful cursor move has already established affinity to A,
        // so B may only see a now-closed discovery probe. It must never see
        // any part of the destructive click sequence.
        if wrote_probe_response {
            assert!(
                read_frame(&mut stream).await.unwrap().is_none(),
                "operation was replayed on a second bridge socket after a CDP command timeout"
            );
        }
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = click(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        10.0,
        20.0,
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server_a.await.unwrap();
    server_b.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    let diagnostic = response
        .diagnostics
        .first()
        .expect("click should surface the timeout diagnostic");
    assert!(diagnostic.message.contains("waiting for CDP command"));
}

#[tokio::test]
async fn claim_wakes_a_discarded_tab_when_page_enable_times_out() {
    // A discarded (asleep) tab attaches browser-side but its renderer is gone,
    // so Page.enable times out. The retry must wake the tab with
    // Page.bringToFront (browser-side, works without a renderer) before
    // re-enabling; activation makes Chrome reload the tab.
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-claim-wake");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, claim) = accept_until_non_info_request(&listener).await;
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
                    "id": 616,
                    "title": "Sleeping Tab",
                    "url": "https://example.test/asleep",
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
        assert_eq!(enable["params"]["method"], "Page.enable");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": enable["id"],
                "error": {
                    "code": 1,
                    "message": "Timed out after 1250ms waiting for CDP command Page.enable."
                }
            }),
        )
        .await
        .unwrap();

        reply_to_detach(&mut stream, 616).await;
        reply_to_attach_wake_and_enable(&mut stream, 616).await;
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = claim_tab(Some(BrowserTargetKind::UserChrome), "616".to_string()).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(
        response.diagnostics.is_empty(),
        "expected the wake retry to recover the claim, got {:?}",
        response.diagnostics
    );
    let tab = response.tab.expect("claim should return the tab");
    assert_eq!(tab.tab_id, "616");
}
