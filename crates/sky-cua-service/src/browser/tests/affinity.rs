//! Tab-to-socket affinity tests: tab-bound requests route to the socket that
//! owns the tab, fall back to discovery only for unknown tabs, and treat
//! `No tab with id` as the sole license for a mutating request to try
//! another bridge socket (read-only operations may fall through on any
//! failure, since retrying them cannot double-apply input).

use serde_json::{Value, json};
use sky_cua_platform::model::BrowserTargetKind;
use tokio::net::UnixListener;

use crate::browser::affinity::{
    forget_tab_socket_if_owner, record_tab_socket, reset_tab_socket_affinity_for_tests,
    tab_socket_affinity,
};
use crate::browser::bridge::{click, snapshot};
use crate::browser::protocol::{read_frame, write_frame};
use crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV;

use super::helpers::*;

#[tokio::test]
async fn bound_operation_routes_to_the_affinity_socket_only() {
    let _env_guard = env_lock().await;
    reset_tab_socket_affinity_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-affinity-routing");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let other_socket = socket_dir.join("extension-1-other.sock");
    let owner_socket = socket_dir.join("extension-2-owner.sock");
    let listener_other = UnixListener::bind(&other_socket).unwrap();
    let listener_owner = UnixListener::bind(&owner_socket).unwrap();
    record_tab_socket("515", &owner_socket);

    let owner = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener_owner).await;
        reply_to_snapshot_request(&mut stream, 515, "Owner Tab", "https://example.test/owner")
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
    owner.await.unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.title.as_deref(), Some("Owner Tab"));
    // The other bridge socket must never have been contacted: affinity
    // restricts the candidate set to the owning socket alone.
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            listener_other.accept()
        )
        .await
        .is_err(),
        "a tab-bound request reached a socket other than the recorded owner"
    );
    std::fs::remove_dir_all(socket_dir).unwrap();
}

#[tokio::test]
async fn tab_not_found_drops_the_affinity_entry() {
    let _env_guard = env_lock().await;
    reset_tab_socket_affinity_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-affinity-not-found");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let owner_socket = socket_dir.join("extension-1-owner.sock");
    let listener = UnixListener::bind(&owner_socket).unwrap();
    record_tab_socket("515", &owner_socket);

    let server = tokio::spawn(async move {
        let (mut stream, first) = accept_until_non_info_request(&listener).await;
        assert_eq!(first["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": first["id"],
                "error": {"code": 1, "message": "No tab with id: 515."}
            }),
        )
        .await
        .unwrap();
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
    let stale = tab_socket_affinity("515", std::slice::from_ref(&owner_socket));
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    let diagnostic = response
        .diagnostics
        .first()
        .expect("snapshot should surface the tab-not-found failure");
    assert!(diagnostic.message.contains("No tab with id"));
    assert!(
        stale.is_none(),
        "a tab the owner no longer has must lose its affinity entry"
    );
}

#[tokio::test]
async fn unknown_tab_falls_back_to_the_socket_that_has_it() {
    let _env_guard = env_lock().await;
    reset_tab_socket_affinity_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-affinity-fallback");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener_a = UnixListener::bind(socket_dir.join("extension-1-a.sock")).unwrap();
    let listener_b = UnixListener::bind(socket_dir.join("extension-2-b.sock")).unwrap();
    // B answers its probe only after A has reported the tab missing, so A is
    // deterministically the first responsive socket.
    let (a_done_tx, a_done_rx) = tokio::sync::oneshot::channel::<()>();

    let server_a = tokio::spawn(async move {
        let (mut stream, first) = accept_until_non_info_request(&listener_a).await;
        assert_eq!(first["params"]["method"], "Runtime.evaluate");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": first["id"],
                "error": {"code": 1, "message": "No tab with id: 515."}
            }),
        )
        .await
        .unwrap();
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
        reply_to_snapshot_request(
            &mut stream,
            515,
            "Fallback Tab",
            "https://example.test/fallback",
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
    server_a.await.unwrap();
    server_b.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(
        response.diagnostics.is_empty(),
        "tab-not-found on the first socket must fall through to the owner, got {:?}",
        response.diagnostics
    );
    assert_eq!(response.title.as_deref(), Some("Fallback Tab"));
}

#[tokio::test]
async fn not_found_from_a_non_owner_keeps_the_affinity_entry() {
    let _env_guard = env_lock().await;
    reset_tab_socket_affinity_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-affinity-non-owner");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let owner_socket = socket_dir.join("extension-1-owner.sock");
    let other_socket = socket_dir.join("extension-2-other.sock");
    // The lookup only checks that the path exists (to prune dead owners);
    // no bridge traffic happens in this test, so plain files suffice.
    std::fs::File::create(&owner_socket).unwrap();
    std::fs::File::create(&other_socket).unwrap();
    record_tab_socket("515", &owner_socket);

    // A not-found answer from a socket that is not the recorded owner says
    // nothing about the owner and must not erase the mapping; only the
    // owner's own not-found clears it.
    forget_tab_socket_if_owner("515", &other_socket);
    assert_eq!(
        tab_socket_affinity("515", &[owner_socket.clone(), other_socket.clone()]),
        Some(owner_socket.clone()),
        "a non-owner not-found must not erase the owner mapping"
    );

    forget_tab_socket_if_owner("515", &owner_socket);
    assert_eq!(
        tab_socket_affinity("515", &[owner_socket, other_socket]),
        None,
        "the owner's own not-found must clear the mapping"
    );
    std::fs::remove_dir_all(socket_dir).unwrap();
}

#[tokio::test]
async fn terminal_diagnostic_outranks_an_earlier_not_found() {
    let _env_guard = env_lock().await;
    reset_tab_socket_affinity_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-affinity-terminal-rank");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener_a = UnixListener::bind(socket_dir.join("extension-1-a.sock")).unwrap();
    let listener_b = UnixListener::bind(socket_dir.join("extension-2-b.sock")).unwrap();
    // B answers its probe only after A has reported the tab missing, so A's
    // not-found is deterministically the first diagnostic the loop collects.
    let (a_done_tx, a_done_rx) = tokio::sync::oneshot::channel::<()>();

    let server_a = tokio::spawn(async move {
        let (mut stream, cursor_move) = accept_until_non_info_request(&listener_a).await;
        assert_eq!(
            cursor_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": cursor_move["id"],
                "error": {"code": 1, "message": "No tab with id: 515."}
            }),
        )
        .await
        .unwrap();
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

        let cursor_move = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            cursor_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        assert_eq!(cursor_move["params"]["tabId"], 515);
        assert_eq!(cursor_move["params"]["x"], 10.0);
        assert_eq!(cursor_move["params"]["y"], 20.0);
        assert_eq!(cursor_move["params"]["waitForArrival"], true);
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": cursor_move["id"], "result": {}}),
        )
        .await
        .unwrap();
        drop(stream);

        let (mut stream, focus) = accept_until_non_info_request(&listener_b).await;
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
                    "title": "Owner Tab",
                    "url": "https://example.test/owner",
                    "active": true
                }
            }),
        )
        .await
        .unwrap();
        reply_to_detach(&mut stream, 515).await;
        reply_to_attach_and_enable(&mut stream, 515).await;
        assert!(read_frame(&mut stream).await.unwrap().is_none());
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

    // The owner's terminal no-replay timeout is the authoritative failure;
    // the non-owner's earlier "No tab with id" must not be what surfaces.
    let diagnostic = response
        .diagnostics
        .first()
        .expect("click should surface a diagnostic");
    assert!(
        diagnostic.message.contains("waiting for CDP command"),
        "expected the owner's terminal timeout, got: {}",
        diagnostic.message
    );
}

#[tokio::test]
async fn listing_prunes_entries_for_closed_tabs_on_a_live_socket() {
    let _env_guard = env_lock().await;
    reset_tab_socket_affinity_for_tests();
    let socket_dir = unique_test_dir("sky-cua-browser-affinity-closed-tab");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let owner_socket = socket_dir.join("extension-1-owner.sock");
    let listener = UnixListener::bind(&owner_socket).unwrap();
    // 515 is still open; 616 was closed since it was recorded. The socket
    // stays alive, so only the listing sweep can prune the stale entry.
    record_tab_socket("515", &owner_socket);
    record_tab_socket("616", &owner_socket);

    let server = tokio::spawn(async move {
        reply_with_tabs(&listener, 515, "Live Tab").await;
    });

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };
    let response = crate::browser::bridge::list_tabs(Some(BrowserTargetKind::UserChrome)).await;
    let live = tab_socket_affinity("515", std::slice::from_ref(&owner_socket));
    let closed = tab_socket_affinity("616", std::slice::from_ref(&owner_socket));
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert_eq!(response.tabs.len(), 1);
    assert_eq!(live, Some(owner_socket));
    assert_eq!(
        closed, None,
        "an entry for a closed tab must be pruned by the listing sweep"
    );
}
