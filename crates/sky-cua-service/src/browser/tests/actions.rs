//! Bridge and tab operation tests: listing, opening, navigation, input, and eval.

use serde_json::{Value, json};
use sky_cua_platform::model::{BROWSER_EVAL_ENV, BrowserTargetKind};
use tokio::net::UnixListener;

use crate::browser::bridge::{eval, list_tabs, move_mouse, navigate, open_tab, press_key};
use crate::browser::protocol::{LIST_TABS_REQUEST_ID, read_frame, write_frame};
use crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV;
use crate::browser::tabs::parse_tabs;
use crate::browser::transport::list_tabs_method;

use super::helpers::*;

#[test]
fn user_chrome_lists_real_browser_tabs() {
    assert_eq!(list_tabs_method(), "getUserTabs");
}

#[tokio::test]
async fn managed_target_reports_unsupported_until_lifecycle_lands() {
    let response = list_tabs(Some(BrowserTargetKind::Managed)).await;

    assert!(response.tabs.is_empty());
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserTargetUnsupported");
}

#[tokio::test]
async fn open_tab_rejects_unsupported_url_before_bridge() {
    let response = open_tab(
        Some(BrowserTargetKind::UserChrome),
        Some("file:///etc/passwd".to_string()),
    )
    .await;

    assert!(response.tab.is_none());
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserOpenUrlUnsupported");
    assert_eq!(
        response.diagnostics[0].message,
        "browser_open url must use http://, https://, or about:blank."
    );
}

#[tokio::test]
async fn navigate_rejects_unsupported_url_with_navigate_diagnostic() {
    let response = navigate(
        Some(BrowserTargetKind::UserChrome),
        "tab-1".to_string(),
        "file:///etc/passwd".to_string(),
    )
    .await;

    assert_eq!(response.tab_id, "tab-1");
    assert_eq!(response.url, "");
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserOpenUrlUnsupported");
    assert_eq!(
        response.diagnostics[0].message,
        "browser_navigate url must use http://, https://, or about:blank."
    );
}

#[test]
fn parses_get_tabs_array_response_into_browser_tabs() {
    let tabs = parse_tabs(
        Some(&json!([
            {
                "id": 7,
                "title": "Example",
                "url": "https://example.test/",
                "active": true
            },
            {
                "tabId": "tab-8",
                "title": "No URL"
            },
            {
                "title": "missing id"
            }
        ])),
        Some(BrowserTargetKind::Managed),
    );

    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].tab_id, "7");
    assert_eq!(tabs[0].target, BrowserTargetKind::Managed);
    assert_eq!(tabs[0].title.as_deref(), Some("Example"));
    assert_eq!(tabs[0].url.as_deref(), Some("https://example.test/"));
    assert!(tabs[0].active);
    assert_eq!(tabs[1].tab_id, "tab-8");
    assert!(!tabs[1].active);
}

#[test]
fn parses_get_tabs_object_response_into_browser_tabs() {
    let tabs = parse_tabs(
        Some(&json!({
            "tabs": [
                {
                    "id": 9,
                    "title": "Object Shape",
                    "url": "https://example.test/object"
                }
            ]
        })),
        None,
    );

    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].tab_id, "9");
    assert_eq!(tabs[0].target, BrowserTargetKind::UserChrome);
    assert_eq!(tabs[0].title.as_deref(), Some("Object Shape"));
    assert_eq!(tabs[0].url.as_deref(), Some("https://example.test/object"));
}

#[tokio::test]
async fn list_tabs_uses_native_host_socket_get_user_tabs_for_user_chrome() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-bridge");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let socket_path = socket_dir.join("extension-123-test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("getUserTabs")
        );
        assert_eq!(
            request.get("id").and_then(Value::as_str),
            Some(LIST_TABS_REQUEST_ID)
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": LIST_TABS_REQUEST_ID,
                "result": [
                    {
                        "id": 42,
                        "title": "Bridge Tab",
                        "url": "https://example.test/bridge",
                        "active": true
                    }
                ]
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = list_tabs(Some(BrowserTargetKind::UserChrome)).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.tabs.len(), 1);
    assert_eq!(response.tabs[0].tab_id, "42");
    assert_eq!(response.tabs[0].target, BrowserTargetKind::UserChrome);
    assert_eq!(response.tabs[0].title.as_deref(), Some("Bridge Tab"));
    assert_eq!(
        response.tabs[0].url.as_deref(),
        Some("https://example.test/bridge")
    );
    assert!(response.tabs[0].active);
}

#[tokio::test]
async fn list_tabs_reports_malformed_bridge_payload_as_diagnostic() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-bridge-malformed-tabs");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let socket_path = socket_dir.join("extension-123-test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("getUserTabs")
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": LIST_TABS_REQUEST_ID,
                "result": {"unexpected": true}
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = list_tabs(Some(BrowserTargetKind::UserChrome)).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.tabs.is_empty());
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserBridgeRequestFailed");
    assert!(
        response.diagnostics[0]
            .message
            .contains("did not include a tabs array")
    );
}

#[tokio::test]
async fn list_tabs_defaults_omitted_target_to_user_chrome() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-default-target");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();
    let server = tokio::spawn(reply_with_tabs(listener, 505, "Default Target Tab"));

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = list_tabs(None).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert_eq!(response.target, Some(BrowserTargetKind::UserChrome));
    assert_eq!(response.tabs.len(), 1);
    assert_eq!(response.tabs[0].target, BrowserTargetKind::UserChrome);
}

#[tokio::test]
async fn open_tab_creates_session_owned_tab_and_navigates_requested_url() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-open");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let socket_path = socket_dir.join("extension-123-test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;

        let create = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            create.get("method").and_then(Value::as_str),
            Some("createTab")
        );
        assert_eq!(create["params"]["session_id"], "sky-cua-mcp");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": create["id"],
                "result": {
                    "id": 515,
                    "title": "New Tab",
                    "url": "about:blank",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();

        let attach = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(attach.get("method").and_then(Value::as_str), Some("attach"));
        assert_eq!(attach["params"]["tabId"], 515);
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
        assert_eq!(enable["params"]["target"]["tabId"], 515);
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": enable["id"], "result": {}}),
        )
        .await
        .unwrap();

        let navigate = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            navigate.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(navigate["params"]["method"], "Page.navigate");
        assert_eq!(
            navigate["params"]["commandParams"]["url"],
            "https://example.test/"
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": navigate["id"],
                "result": {"frameId": "frame-1"}
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = open_tab(
        Some(BrowserTargetKind::UserChrome),
        Some("https://example.test/".to_string()),
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    let tab = response.tab.expect("open should return created tab");
    assert_eq!(tab.tab_id, "515");
    assert_eq!(tab.target, BrowserTargetKind::UserChrome);
    assert_eq!(tab.url.as_deref(), Some("https://example.test/"));
    assert!(tab.active);
}

#[tokio::test]
async fn open_tab_returns_partial_created_tab_when_attach_fails() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-open-partial");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let socket_path = socket_dir.join("extension-123-test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;

        let create = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            create.get("method").and_then(Value::as_str),
            Some("createTab")
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": create["id"],
                "result": {
                    "id": 616,
                    "title": "Partial Tab",
                    "url": "about:blank",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();

        let attach = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(attach.get("method").and_then(Value::as_str), Some("attach"));
        assert_eq!(attach["params"]["tabId"], 616);
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": attach["id"],
                "error": {"message": "session refused tab"}
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = open_tab(
        Some(BrowserTargetKind::UserChrome),
        Some("https://example.test/".to_string()),
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    let tab = response.tab.expect("created tab should be returned");
    assert_eq!(tab.tab_id, "616");
    assert_eq!(tab.url.as_deref(), Some("about:blank"));
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserOpenPartial");
    assert!(response.diagnostics[0].message.contains("browser tab 616"));
    assert!(
        response.diagnostics[0]
            .details
            .as_deref()
            .unwrap_or_default()
            .contains("BrowserBridgeRequestFailed")
    );
}

#[tokio::test]
async fn move_mouse_targets_claimed_or_session_tab() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-move-mouse");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;
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
    assert!(response.wait_for_arrival);
}

#[tokio::test]
async fn press_key_dispatches_modifier_chord() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-press-key-modifier");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;

        let key_down = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            key_down.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(key_down["params"]["method"], "Input.dispatchKeyEvent");
        assert_eq!(key_down["params"]["commandParams"]["type"], "keyDown");
        assert_eq!(key_down["params"]["commandParams"]["key"], "K");
        assert_eq!(key_down["params"]["commandParams"]["modifiers"], 2);
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": key_down["id"], "result": {}}),
        )
        .await
        .unwrap();

        let key_up = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(key_up["params"]["method"], "Input.dispatchKeyEvent");
        assert_eq!(key_up["params"]["commandParams"]["type"], "keyUp");
        assert_eq!(key_up["params"]["commandParams"]["key"], "K");
        assert_eq!(key_up["params"]["commandParams"]["modifiers"], 2);
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": key_up["id"], "result": {}}),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = press_key(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        "Ctrl+K".to_string(),
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.action, "press_key");
}

#[tokio::test]
async fn eval_returns_runtime_value() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-eval");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;

        let eval_request = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            eval_request.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(eval_request["params"]["method"], "Runtime.evaluate");
        assert_eq!(
            eval_request["params"]["commandParams"]["expression"],
            "(() => ({ok: true}))()"
        );
        assert_eq!(
            eval_request["params"]["commandParams"]["awaitPromise"],
            true
        );
        assert_eq!(
            eval_request["params"]["commandParams"]["returnByValue"],
            true
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": eval_request["id"],
                "result": {
                    "result": {
                        "type": "object",
                        "value": {"ok": true}
                    }
                }
            }),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    unsafe { std::env::set_var(BROWSER_EVAL_ENV, "on") };
    let response = eval(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        "(() => ({ok: true}))()".to_string(),
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.value, Some(json!({"ok": true})));
}

#[tokio::test]
async fn eval_disabled_without_opt_in_returns_diagnostic() {
    // env_lock() already cleared BROWSER_EVAL_ENV, so eval is off here.
    let _env_guard = env_lock().await;
    let response = eval(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        "(() => 1)()".to_string(),
    )
    .await;

    assert_eq!(response.value, None);
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserEvalDisabled");
}

#[tokio::test]
async fn eval_reports_thrown_exception_as_diagnostic() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-eval-throw");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;
        let eval_request = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(eval_request["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": eval_request["id"],
                "result": {
                    "result": {"type": "object", "subtype": "error"},
                    "exceptionDetails": {
                        "text": "Uncaught",
                        "exception": {
                            "type": "object",
                            "subtype": "error",
                            "description": "TypeError: boom"
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
    unsafe { std::env::set_var(BROWSER_EVAL_ENV, "on") };
    let response = eval(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        "throw new TypeError('boom')".to_string(),
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert_eq!(response.value, None);
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserEvalException");
    assert!(response.diagnostics[0].message.contains("TypeError: boom"));
}
