//! Socket inventory, selection, probing, and bridge readiness tests.

use std::path::Path;
use std::time::Duration as StdDuration;

use sky_cua_platform::model::{BrowserTargetKind, DiagnosticEntry};
use tokio::net::UnixListener;
use tokio::time::Instant as TokioInstant;

use crate::browser::bridge::{
    BROWSER_OPEN_TIMEOUT, browser_bridge_diagnostics, list_tabs, open_tab,
};
use crate::browser::sockets::{
    BrowserFamily, BrowserSocketSelection, CODEX_SOCKET_DIR_ENV, MAX_BRIDGE_SOCKET_CANDIDATES,
    SKY_CUA_BROWSER_ENV, SKY_CUA_SOCKET_DIR_ENV, browser_family_from_cmdline,
    browser_socket_selection_from_env, browser_socket_selection_from_value,
    cache_socket_family_for_tests, find_bridge_sockets, record_bridge_socket_result,
    reset_socket_inventory_for_tests, socket_host_pid,
};
use crate::browser::transport::bridge_request_timeout;

use super::helpers::*;

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
        bridge_request_timeout(),
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
        bridge_request_timeout(),
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
    // This test observes the bridge request deadline *firing*, so it pins it short
    // rather than using the generous test default (which exists so happy-path tests
    // do not trip under load). env_lock serializes it with every other browser test
    // that reads it, and restores it when the guard drops.
    unsafe { std::env::set_var("SKY_CUA_TEST_BRIDGE_REQUEST_TIMEOUT_MS", "100") };
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
        BROWSER_OPEN_TIMEOUT + bridge_request_timeout() + bridge_request_timeout(),
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
    assert!(started.elapsed() < BROWSER_OPEN_TIMEOUT + bridge_request_timeout());
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
    // list_tabs aggregates across sockets, so it waits one request timeout for the
    // unresponsive stale sockets. Pin it to a bounded value (not the generous test
    // default, which would make the wait 10s) that still leaves headroom for the
    // live socket to respond under load. env_lock restores it when the guard drops.
    unsafe { std::env::set_var("SKY_CUA_TEST_BRIDGE_REQUEST_TIMEOUT_MS", "1000") };
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
        bridge_request_timeout() + bridge_request_timeout(),
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

#[tokio::test]
async fn browser_selection_resolves_from_machine_config_file() {
    let _env_guard = env_lock().await;
    let temp_dir = std::env::temp_dir().join(format!(
        "sky-cua-machine-config-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create test temp dir");
    let config_path = temp_dir.join("sky-cua.toml");
    std::fs::write(&config_path, "browser = \"brave\"\n").expect("write machine config");
    unsafe {
        std::env::set_var(
            sky_cua_platform::config::MACHINE_CONFIG_PATH_ENV,
            &config_path,
        )
    };

    let selection = browser_socket_selection_from_env().expect("config selection resolves");
    assert_eq!(
        selection,
        BrowserSocketSelection::Browser(BrowserFamily::Brave)
    );

    // The env var is a per-process override on top of the file.
    unsafe { std::env::set_var(SKY_CUA_BROWSER_ENV, "chrome") };
    let selection = browser_socket_selection_from_env().expect("env override resolves");
    assert_eq!(
        selection,
        BrowserSocketSelection::Browser(BrowserFamily::Chrome)
    );
    unsafe { std::env::remove_var(SKY_CUA_BROWSER_ENV) };

    let _ = std::fs::remove_dir_all(&temp_dir);
}
