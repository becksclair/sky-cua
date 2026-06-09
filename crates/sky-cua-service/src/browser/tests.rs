use super::super::sockets::{
    BrowserFamily, CODEX_SOCKET_DIR_ENV, MAX_BRIDGE_SOCKET_CANDIDATES, SKY_CUA_BROWSER_ENV,
    SKY_CUA_SOCKET_DIR_ENV, browser_family_from_cmdline, browser_socket_selection_from_value,
    cache_socket_family_for_tests, find_bridge_sockets, reset_socket_inventory_for_tests,
    socket_host_pid,
};
use super::*;
use crate::browser::protocol::MAX_FRAME_SIZE;
use std::sync::OnceLock;
use std::time::{Duration as StdDuration, SystemTime};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;
use tokio::sync::{Mutex, MutexGuard};

#[test]
fn parses_browser_socket_selection_env_values() {
    assert_eq!(
        browser_socket_selection_from_value(None).unwrap(),
        BrowserSocketSelection::All
    );
    assert_eq!(
        browser_socket_selection_from_value(Some("")).unwrap(),
        BrowserSocketSelection::All
    );
    assert_eq!(
        browser_socket_selection_from_value(Some("all")).unwrap(),
        BrowserSocketSelection::All
    );
    assert_eq!(
        browser_socket_selection_from_value(Some("brave")).unwrap(),
        BrowserSocketSelection::Browser(BrowserFamily::Brave)
    );
    assert_eq!(
        browser_socket_selection_from_value(Some("google-chrome")).unwrap(),
        BrowserSocketSelection::Browser(BrowserFamily::Chrome)
    );
    assert_eq!(
        browser_socket_selection_from_value(Some("chromium_browser")).unwrap(),
        BrowserSocketSelection::Browser(BrowserFamily::Chromium)
    );
    assert!(browser_socket_selection_from_value(Some("firefox")).is_err());
}

#[test]
fn parses_socket_pid_from_native_host_socket_name() {
    assert_eq!(
        socket_host_pid(Path::new(
            "/tmp/codex-browser-use/extension-123-a2fb97377e34aee1.sock"
        )),
        Some(123)
    );
    assert_eq!(
        socket_host_pid(Path::new("/tmp/codex-browser-use/not-extension.sock")),
        None
    );
}

#[test]
fn detects_browser_family_from_parent_cmdline() {
    assert_eq!(
        browser_family_from_cmdline("/opt/brave-bin/brave --ozone-platform=wayland"),
        Some(BrowserFamily::Brave)
    );
    assert_eq!(
        browser_family_from_cmdline("/opt/google/chrome/chrome --type=browser"),
        Some(BrowserFamily::Chrome)
    );
    assert_eq!(
        browser_family_from_cmdline("/usr/bin/chromium --type=browser"),
        Some(BrowserFamily::Chromium)
    );
    assert_eq!(
        browser_family_from_cmdline("/usr/bin/firefox --type=browser"),
        None
    );
}

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
async fn bridge_request_times_out_when_peer_only_sends_pings() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let request = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("getInfo")
        );
        let mut interval = tokio::time::interval(BRIDGE_REQUEST_TIMEOUT / 4);
        loop {
            interval.tick().await;
            write_frame(
                &mut server,
                &json!({
                    "jsonrpc": "2.0",
                    "id": "ping",
                    "method": "ping"
                }),
            )
            .await
            .unwrap();
        }
    });

    let diagnostic = send_bridge_request(
        &mut client,
        Path::new("/tmp/sky-cua-browser-timeout.sock"),
        BRIDGE_INFO_REQUEST_ID,
        "getInfo",
        browser_session_params(),
    )
    .await
    .unwrap_err();
    server.abort();

    assert_eq!(diagnostic.code, "BrowserBridgeRequestTimedOut");
}

#[tokio::test]
async fn read_frame_rejects_invalid_json_without_retry_loop() {
    let (mut client, mut server) = UnixStream::pair().unwrap();

    server.write_all(&4_u32.to_ne_bytes()).await.unwrap();
    server.write_all(b"nope").await.unwrap();

    let error = read_frame(&mut client).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn read_frame_rejects_oversized_browser_bridge_frames() {
    let (mut client, mut server) = UnixStream::pair().unwrap();

    server
        .write_all(&((MAX_FRAME_SIZE as u32) + 1).to_ne_bytes())
        .await
        .unwrap();

    let error = read_frame(&mut client).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
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
                    "message": "Debugger is not attached to the tab with id: 515."
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
async fn move_mouse_targets_claimed_or_session_tab() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-move-mouse");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;
        reply_to_viewport_scale(&mut stream, 515, 1.25).await;
        let move_mouse = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            move_mouse.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        assert_eq!(move_mouse["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(move_mouse["params"]["tabId"], 515);
        assert_eq!(move_mouse["params"]["x"], 192.0);
        assert_eq!(move_mouse["params"]["y"], 128.0);
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
async fn move_mouse_recovers_when_viewport_scale_finds_stale_session() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-move-mouse-recover");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, scale) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            scale.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(scale["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(scale["params"]["target"]["tabId"], 515);
        assert_eq!(scale["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": scale["id"],
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

        reply_to_attach_and_enable(&mut stream, 515).await;
        reply_to_viewport_scale(&mut stream, 515, 2.0).await;

        let move_mouse = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            move_mouse.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        assert_eq!(move_mouse["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(move_mouse["params"]["tabId"], 515);
        assert_eq!(move_mouse["params"]["x"], 120.0);
        assert_eq!(move_mouse["params"]["y"], 80.0);
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
async fn click_converts_browser_screenshot_pixels_to_css_pixels() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-click-device-pixels");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;
        reply_to_viewport_scale(&mut stream, 515, 1.25).await;

        for (expected_type, expected_button) in [
            ("mouseMoved", None),
            ("mousePressed", Some("left")),
            ("mouseReleased", Some("left")),
        ] {
            let event = read_frame(&mut stream).await.unwrap().unwrap();
            assert_eq!(
                event.get("method").and_then(Value::as_str),
                Some("executeCdp")
            );
            assert_eq!(event["params"]["method"], "Input.dispatchMouseEvent");
            let command = &event["params"]["commandParams"];
            assert_eq!(command["type"], expected_type);
            assert_eq!(command["x"], 240.0);
            assert_eq!(command["y"], 971.2);
            if let Some(button) = expected_button {
                assert_eq!(command["button"], button);
            }
            write_frame(
                &mut stream,
                &json!({"jsonrpc": "2.0", "id": event["id"], "result": {}}),
            )
            .await
            .unwrap();
        }
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = click(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        300.0,
        1214.0,
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.action, "click");
}

#[tokio::test]
async fn cdp_action_recovers_when_tab_is_not_in_browser_session() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-cdp-recover-session");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, first_screenshot) = accept_until_non_info_request(&listener).await;
        assert_eq!(
            first_screenshot.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(first_screenshot["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(first_screenshot["params"]["target"]["tabId"], 515);
        assert_eq!(
            first_screenshot["params"]["method"],
            "Page.captureScreenshot"
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": first_screenshot["id"],
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

        reply_to_attach_and_enable(&mut stream, 515).await;

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
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": retried_screenshot["id"],
                "result": {"data": "png-base64"}
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
    assert_eq!(response.data_base64, "png-base64");
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

#[test]
fn browser_snapshot_expression_suppresses_sensitive_form_values() {
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("sensitiveField"));
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("api[-_ ]?key"));
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("password"));
    assert!(
        BROWSER_SNAPSHOT_EXPRESSION
            .contains("if (!('value' in el) || sensitiveField(el)) return null;")
    );
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("return String(el.value).slice"));
}

#[tokio::test]
async fn scroll_converts_browser_screenshot_pixels_to_css_pixels() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-scroll-device-pixels");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;
        reply_to_viewport_scale(&mut stream, 515, 1.25).await;

        let scroll = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            scroll.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(scroll["params"]["method"], "Runtime.evaluate");
        let expression = scroll["params"]["commandParams"]["expression"]
            .as_str()
            .unwrap_or_default();
        assert!(expression.contains("window.scrollBy(80, 320)"));
        assert!(expression.contains("eventX: 240"));
        assert!(expression.contains("eventY: 971.2"));
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": scroll["id"], "result": {}}),
        )
        .await
        .unwrap();
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = scroll(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        100.0,
        400.0,
        300.0,
        1214.0,
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.action, "scroll");
}

#[tokio::test]
async fn open_tab_does_not_wait_for_later_stale_sockets_after_first_live_probe() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-open-first-live");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let stale_a = UnixListener::bind(socket_dir.join("extension-100-hung.sock")).unwrap();
    let stale_b = UnixListener::bind(socket_dir.join("extension-200-hung.sock")).unwrap();
    std::thread::sleep(StdDuration::from_millis(5));
    let live = UnixListener::bind(socket_dir.join("extension-900-live.sock")).unwrap();

    let stale_servers = [stale_a, stale_b].map(|listener| tokio::spawn(hold_connection(listener)));
    let live_server = tokio::spawn(reply_with_opened_tab(live, 717));

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = tokio::time::timeout(
        BRIDGE_REQUEST_TIMEOUT,
        open_tab(Some(BrowserTargetKind::UserChrome), None),
    )
    .await
    .expect("browser_open should not wait for later stale probes once the first socket responds");
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    live_server.await.unwrap();
    for server in stale_servers {
        server.abort();
    }
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    let tab = response.tab.expect("open should return created tab");
    assert_eq!(tab.tab_id, "717");
}

#[tokio::test]
async fn open_tab_does_not_wait_for_preferred_stale_socket_when_later_socket_is_live() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-open-stale-first");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let live = UnixListener::bind(socket_dir.join("extension-100-live.sock")).unwrap();
    std::thread::sleep(StdDuration::from_millis(5));
    let stale = UnixListener::bind(socket_dir.join("extension-900-hung.sock")).unwrap();

    let stale_server = tokio::spawn(hold_connection(stale));
    let live_server = tokio::spawn(reply_with_opened_tab(live, 818));

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = tokio::time::timeout(
        BRIDGE_REQUEST_TIMEOUT,
        open_tab(Some(BrowserTargetKind::UserChrome), None),
    )
    .await
    .expect(
        "browser_open should not wait for a preferred stale probe when another socket responds",
    );
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    live_server.await.unwrap();
    stale_server.abort();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    let tab = response.tab.expect("open should return created tab");
    assert_eq!(tab.tab_id, "818");
}

#[tokio::test]
async fn open_tab_stops_at_aggregate_deadline_across_responsive_bad_sockets() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-open-deadline");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listeners = (0..4)
        .map(|index| {
            UnixListener::bind(socket_dir.join(format!("extension-{}-slow.sock", index + 100)))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let servers = listeners
        .into_iter()
        .map(|listener| tokio::spawn(reply_with_info_then_hang_on_create(listener)))
        .collect::<Vec<_>>();

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let started = TokioInstant::now();
    let response = tokio::time::timeout(
        BROWSER_OPEN_TIMEOUT + BRIDGE_REQUEST_TIMEOUT + BRIDGE_REQUEST_TIMEOUT,
        open_tab(Some(BrowserTargetKind::UserChrome), None),
    )
    .await
    .expect("browser_open should honor the aggregate browser-open deadline");
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    for server in servers {
        server.abort();
    }
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.tab.is_none());
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserBridgeRequestTimedOut");
    assert!(started.elapsed() < BROWSER_OPEN_TIMEOUT + BRIDGE_REQUEST_TIMEOUT);
}

#[tokio::test]
async fn list_tabs_merges_all_native_host_sockets() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-bridge-multiple");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let socket_path_a = socket_dir.join("extension-123-test.sock");
    let socket_path_b = socket_dir.join("extension-456-test.sock");
    let listener_a = UnixListener::bind(&socket_path_a).unwrap();
    let listener_b = UnixListener::bind(&socket_path_b).unwrap();

    let server_a = tokio::spawn(reply_with_tabs(listener_a, 101, "Bridge Tab A"));
    let server_b = tokio::spawn(reply_with_tabs(listener_b, 202, "Bridge Tab B"));

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = list_tabs(Some(BrowserTargetKind::UserChrome)).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server_a.await.unwrap();
    server_b.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    let mut tab_ids = response
        .tabs
        .iter()
        .map(|tab| tab.tab_id.as_str())
        .collect::<Vec<_>>();
    tab_ids.sort_unstable();
    assert_eq!(tab_ids, ["101", "202"]);
}

#[tokio::test]
async fn list_tabs_suppresses_stale_socket_noise_when_a_live_socket_responds() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-bridge-stale");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let live_socket_path = socket_dir.join("extension-123-test.sock");
    let stale_socket_path = socket_dir.join("extension-456-stale.sock");
    let live_listener = UnixListener::bind(&live_socket_path).unwrap();
    drop(UnixListener::bind(&stale_socket_path).unwrap());

    let server = tokio::spawn(reply_with_tabs(live_listener, 303, "Live Tab"));

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = list_tabs(Some(BrowserTargetKind::UserChrome)).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.tabs.len(), 1);
    assert_eq!(response.tabs[0].tab_id, "303");
}

#[tokio::test]
async fn list_tabs_probes_stale_sockets_concurrently() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-bridge-concurrent");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let stale_a = UnixListener::bind(socket_dir.join("extension-100-hung.sock")).unwrap();
    let stale_b = UnixListener::bind(socket_dir.join("extension-200-hung.sock")).unwrap();
    let stale_c = UnixListener::bind(socket_dir.join("extension-300-hung.sock")).unwrap();
    let live = UnixListener::bind(socket_dir.join("extension-900-live.sock")).unwrap();

    let stale_servers =
        [stale_a, stale_b, stale_c].map(|listener| tokio::spawn(hold_connection(listener)));
    let live_server = tokio::spawn(reply_with_tabs(live, 909, "Concurrent Tab"));

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = tokio::time::timeout(
        BRIDGE_REQUEST_TIMEOUT + BRIDGE_REQUEST_TIMEOUT,
        list_tabs(Some(BrowserTargetKind::UserChrome)),
    )
    .await
    .expect("stale sockets should not multiply list_tabs latency");
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    live_server.await.unwrap();
    for server in stale_servers {
        server.abort();
    }
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.tabs.len(), 1);
    assert_eq!(response.tabs[0].tab_id, "909");
}

#[tokio::test]
async fn list_tabs_reports_disconnected_when_socket_is_missing() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-bridge-missing");
    std::fs::create_dir_all(&socket_dir).unwrap();

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = list_tabs(Some(BrowserTargetKind::UserChrome)).await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.tabs.is_empty());
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].code, "BrowserBridgeDisconnected");
}

#[tokio::test]
async fn socket_discovery_ignores_blank_sky_cua_socket_dir_override() {
    let _env_guard = env_lock().await;
    reset_socket_inventory_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-blank-socket-dir");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let socket_path = socket_dir.join("extension-123-test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let previous_sky = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    let previous_codex = std::env::var_os(CODEX_SOCKET_DIR_ENV);
    unsafe {
        std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, "");
        std::env::set_var(CODEX_SOCKET_DIR_ENV, &socket_dir);
    }

    let sockets = find_bridge_sockets(BrowserSocketSelection::All);

    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous_sky);
    restore_env(CODEX_SOCKET_DIR_ENV, previous_codex);
    drop(listener);
    std::fs::remove_dir_all(socket_dir).unwrap();
    reset_socket_inventory_for_tests();

    assert_eq!(sockets, vec![socket_path]);
}

#[tokio::test]
async fn browser_status_reports_invalid_browser_selection() {
    let _env_guard = env_lock().await;
    let previous = std::env::var_os(SKY_CUA_BROWSER_ENV);
    unsafe { std::env::set_var(SKY_CUA_BROWSER_ENV, "firefox") };

    let diagnostics = browser_bridge_diagnostics().await;

    restore_env(SKY_CUA_BROWSER_ENV, previous);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "BrowserSelectionInvalid");
}

#[tokio::test]
async fn browser_status_uses_bridge_info_probe_without_listing_tabs() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-status-bridge");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();
    let server = tokio::spawn(reply_with_info(listener));

    let previous_socket_dir = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    let previous_browser = std::env::var_os(SKY_CUA_BROWSER_ENV);
    unsafe {
        std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir);
        std::env::remove_var(SKY_CUA_BROWSER_ENV);
    }

    let diagnostics = browser_bridge_diagnostics().await;

    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous_socket_dir);
    restore_env(SKY_CUA_BROWSER_ENV, previous_browser);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();
    assert!(diagnostics.is_empty());
}

#[tokio::test]
async fn browser_status_reports_disconnected_when_socket_closes_without_info() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-status-closes");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
    });

    let previous_socket_dir = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    let previous_browser = std::env::var_os(SKY_CUA_BROWSER_ENV);
    unsafe {
        std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir);
        std::env::remove_var(SKY_CUA_BROWSER_ENV);
    }

    let diagnostics = browser_bridge_diagnostics().await;

    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous_socket_dir);
    restore_env(SKY_CUA_BROWSER_ENV, previous_browser);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "BrowserBridgeRequestFailed");
}

#[tokio::test]
async fn browser_status_reports_disconnected_when_socket_is_stale() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-status-stale");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let stale_socket_path = socket_dir.join("extension-123-stale.sock");
    std::fs::write(&stale_socket_path, b"stale socket path").unwrap();

    let previous_socket_dir = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    let previous_browser = std::env::var_os(SKY_CUA_BROWSER_ENV);
    unsafe {
        std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir);
        std::env::remove_var(SKY_CUA_BROWSER_ENV);
    }

    let diagnostics = browser_bridge_diagnostics().await;

    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous_socket_dir);
    restore_env(SKY_CUA_BROWSER_ENV, previous_browser);
    std::fs::remove_dir_all(socket_dir).unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "BrowserBridgeDisconnected");
}

#[tokio::test]
async fn socket_inventory_limits_candidate_count() {
    let _env_guard = env_lock().await;
    reset_socket_inventory_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-many-sockets");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let mut listeners = Vec::new();
    for index in 0..(MAX_BRIDGE_SOCKET_CANDIDATES + 8) {
        listeners.push(
            UnixListener::bind(socket_dir.join(format!("extension-{index}-test.sock"))).unwrap(),
        );
    }

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let sockets = find_bridge_sockets(BrowserSocketSelection::All);
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    drop(listeners);
    std::fs::remove_dir_all(socket_dir).unwrap();
    reset_socket_inventory_for_tests();

    assert_eq!(sockets.len(), MAX_BRIDGE_SOCKET_CANDIDATES);
}

#[tokio::test]
async fn socket_inventory_filters_selected_browser_before_candidate_cap() {
    let _env_guard = env_lock().await;
    reset_socket_inventory_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-selected-cap");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let matching_path = socket_dir.join("extension-999-chrome.sock");
    let matching_listener = UnixListener::bind(&matching_path).unwrap();
    cache_socket_family_for_tests(&matching_path, Some(BrowserFamily::Chrome));
    std::thread::sleep(StdDuration::from_millis(5));
    let mut nonmatching_listeners = Vec::new();
    for index in 0..(MAX_BRIDGE_SOCKET_CANDIDATES + 8) {
        let path = socket_dir.join(format!("extension-{index:03}-brave.sock"));
        nonmatching_listeners.push(UnixListener::bind(&path).unwrap());
        cache_socket_family_for_tests(&path, Some(BrowserFamily::Brave));
    }

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let sockets = find_bridge_sockets(BrowserSocketSelection::Browser(BrowserFamily::Chrome));
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    drop(matching_listener);
    drop(nonmatching_listeners);
    std::fs::remove_dir_all(socket_dir).unwrap();
    reset_socket_inventory_for_tests();

    assert_eq!(sockets, vec![matching_path]);
}

#[tokio::test]
async fn socket_inventory_skips_recently_disconnected_socket() {
    let _env_guard = env_lock().await;
    reset_socket_inventory_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-stale-cache");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let stale_path = socket_dir.join("extension-100-stale.sock");
    let live_path = socket_dir.join("extension-200-live.sock");
    let stale_listener = UnixListener::bind(&stale_path).unwrap();
    let live_listener = UnixListener::bind(&live_path).unwrap();

    record_bridge_socket_result::<()>(
        &stale_path,
        Err(&DiagnosticEntry {
            code: "BrowserBridgeDisconnected".to_string(),
            message: "stale".to_string(),
            details: None,
        }),
    );

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let sockets = find_bridge_sockets(BrowserSocketSelection::All);
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    drop(stale_listener);
    drop(live_listener);
    std::fs::remove_dir_all(socket_dir).unwrap();
    reset_socket_inventory_for_tests();

    assert!(!sockets.contains(&stale_path));
    assert!(sockets.contains(&live_path));
}

fn unique_test_dir(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

async fn reply_with_tabs(
    listener: impl std::borrow::Borrow<UnixListener>,
    tab_id: i64,
    title: &'static str,
) {
    let (mut stream, _) = listener.borrow().accept().await.unwrap();
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
            "result": [
                {
                    "id": tab_id,
                    "title": title,
                    "url": "https://example.test/bridge"
                }
            ]
        }),
    )
    .await
    .unwrap();
}

async fn reply_with_info(listener: impl std::borrow::Borrow<UnixListener>) {
    let mut stream = accept_after_info(listener.borrow()).await;
    let _ = read_frame(&mut stream).await;
}

async fn accept_after_info(listener: impl std::borrow::Borrow<UnixListener>) -> UnixStream {
    let (mut stream, _) = listener.borrow().accept().await.unwrap();
    reply_to_info_request(&mut stream).await;
    stream
}

async fn reply_to_info_request(stream: &mut UnixStream) {
    let request = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(
        request.get("method").and_then(Value::as_str),
        Some("getInfo")
    );
    assert_eq!(
        request.get("id").and_then(Value::as_str),
        Some(BRIDGE_INFO_REQUEST_ID)
    );
    write_frame(
        stream,
        &json!({
            "jsonrpc": "2.0",
            "id": BRIDGE_INFO_REQUEST_ID,
            "result": {"name": "sky-cua-test-bridge"}
        }),
    )
    .await
    .unwrap();
}

async fn accept_until_non_info_request(listener: &UnixListener) -> (UnixStream, Value) {
    let (mut stream, _) = listener.accept().await.unwrap();
    loop {
        let request = read_frame(&mut stream).await.unwrap().unwrap();
        if request.get("method").and_then(Value::as_str) != Some("getInfo") {
            return (stream, request);
        }
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {"name": "sky-cua-test-bridge"}
            }),
        )
        .await
        .unwrap();
    }
}

async fn reply_with_opened_tab(listener: impl std::borrow::Borrow<UnixListener>, tab_id: i64) {
    let mut stream = accept_after_info(listener.borrow()).await;
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
                "id": tab_id,
                "title": "First Live Tab",
                "url": "about:blank",
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
        &json!({"jsonrpc": "2.0", "id": enable["id"], "result": {}}),
    )
    .await
    .unwrap();
}

async fn reply_to_attach_and_enable(stream: &mut UnixStream, tab_id: i64) {
    let attach = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(attach.get("method").and_then(Value::as_str), Some("attach"));
    assert_eq!(attach["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(attach["params"]["tabId"], tab_id);
    write_frame(
        stream,
        &json!({"jsonrpc": "2.0", "id": attach["id"], "result": {}}),
    )
    .await
    .unwrap();

    let enable = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(
        enable.get("method").and_then(Value::as_str),
        Some("executeCdp")
    );
    assert_eq!(enable["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(enable["params"]["target"]["tabId"], tab_id);
    assert_eq!(enable["params"]["method"], "Page.enable");
    write_frame(
        stream,
        &json!({"jsonrpc": "2.0", "id": enable["id"], "result": {}}),
    )
    .await
    .unwrap();
}

async fn reply_to_viewport_scale(stream: &mut UnixStream, tab_id: i64, device_pixel_ratio: f64) {
    let scale = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(
        scale.get("method").and_then(Value::as_str),
        Some("executeCdp")
    );
    assert_eq!(scale["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(scale["params"]["target"]["tabId"], tab_id);
    assert_eq!(scale["params"]["method"], "Runtime.evaluate");
    write_frame(
        stream,
        &json!({
            "jsonrpc": "2.0",
            "id": scale["id"],
            "result": {
                "result": {
                    "value": {"devicePixelRatio": device_pixel_ratio}
                }
            }
        }),
    )
    .await
    .unwrap();
}

async fn reply_with_info_then_hang_on_create(listener: UnixListener) {
    let mut stream = accept_after_info(&listener).await;
    let create = read_frame(&mut stream).await.unwrap().unwrap();
    assert_eq!(
        create.get("method").and_then(Value::as_str),
        Some("createTab")
    );
    std::future::pending::<()>().await;
}

async fn hold_connection(listener: UnixListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let _request = read_frame(&mut stream).await.unwrap().unwrap();
    std::future::pending::<()>().await;
}

struct BrowserEnvGuard {
    _guard: MutexGuard<'static, ()>,
    previous_browser: Option<std::ffi::OsString>,
}

impl Drop for BrowserEnvGuard {
    fn drop(&mut self) {
        restore_env(SKY_CUA_BROWSER_ENV, self.previous_browser.take());
    }
}

async fn env_lock() -> BrowserEnvGuard {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let previous_browser = std::env::var_os(SKY_CUA_BROWSER_ENV);
    unsafe { std::env::remove_var(SKY_CUA_BROWSER_ENV) };
    BrowserEnvGuard {
        _guard: guard,
        previous_browser,
    }
}

fn restore_env(name: &str, previous: Option<std::ffi::OsString>) {
    unsafe {
        if let Some(value) = previous {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}
