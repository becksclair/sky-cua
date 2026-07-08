//! CSS-pixel coordinate dispatch tests for pointer and scroll actions.

use serde_json::{Value, json};
use sky_cua_platform::model::BrowserTargetKind;
use tokio::net::UnixListener;

use crate::browser::bridge::{click, scroll};
use crate::browser::protocol::{read_frame, write_frame};
use crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV;

use super::helpers::*;

#[tokio::test]
async fn click_dispatches_css_pixel_coordinates_directly() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-click-css-pixels");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;

        let cursor_move = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            cursor_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        assert_eq!(cursor_move["params"]["tabId"], 515);
        assert_eq!(cursor_move["params"]["x"], 300.0);
        assert_eq!(cursor_move["params"]["y"], 1214.0);
        assert_eq!(cursor_move["params"]["waitForArrival"], true);
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": cursor_move["id"], "result": {}}),
        )
        .await
        .unwrap();
        drop(stream);

        let mut stream = accept_after_info(&listener).await;
        read_and_ack_focus_emulation(&mut stream).await;

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
            assert_eq!(command["x"], 300.0);
            assert_eq!(command["y"], 1214.0);
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
async fn click_stops_when_agent_cursor_move_fails() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-click-cursor-move-fails");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;

        let cursor_move = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            cursor_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": cursor_move["id"],
                "error": {"code": 1, "message": "cursor overlay did not arrive"}
            }),
        )
        .await
        .unwrap();
        assert!(
            read_frame(&mut stream).await.unwrap().is_none(),
            "click dispatched after the agent cursor move failed"
        );
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

    let diagnostic = response
        .diagnostics
        .first()
        .expect("click should surface the cursor diagnostic");
    assert!(diagnostic.message.contains("cursor overlay did not arrive"));
    assert_eq!(response.action, "click");
}

#[tokio::test]
async fn scroll_dispatches_css_pixel_coordinates_directly() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-scroll-css-pixels");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;

        let cursor_move = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            cursor_move.get("method").and_then(Value::as_str),
            Some("moveMouse")
        );
        assert_eq!(cursor_move["params"]["tabId"], 515);
        assert_eq!(cursor_move["params"]["x"], 300.0);
        assert_eq!(cursor_move["params"]["y"], 1214.0);
        assert_eq!(cursor_move["params"]["waitForArrival"], true);
        write_frame(
            &mut stream,
            &json!({"jsonrpc": "2.0", "id": cursor_move["id"], "result": {}}),
        )
        .await
        .unwrap();
        drop(stream);

        let mut stream = accept_after_info(&listener).await;

        let scroll = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            scroll.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(scroll["params"]["method"], "Runtime.evaluate");
        let expression = scroll["params"]["commandParams"]["expression"]
            .as_str()
            .unwrap_or_default();
        assert!(expression.contains("document.elementFromPoint(eventX, eventY)"));
        assert!(expression.contains("target.scrollBy(deltaX, deltaY)"));
        assert!(expression.contains("window.scrollBy(deltaX, deltaY)"));
        assert!(expression.contains("const eventX = 300"));
        assert!(expression.contains("const eventY = 1214"));
        assert!(expression.contains("const deltaX = 100"));
        assert!(expression.contains("const deltaY = 400"));
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
        Some(300.0),
        Some(1214.0),
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.action, "scroll");
}

#[tokio::test]
async fn scroll_without_coordinates_scrolls_viewport_without_cursor_move() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-scroll-window");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    let server = tokio::spawn(async move {
        let mut stream = accept_after_info(&listener).await;

        let scroll = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(
            scroll.get("method").and_then(Value::as_str),
            Some("executeCdp")
        );
        assert_eq!(scroll["params"]["method"], "Runtime.evaluate");
        let expression = scroll["params"]["commandParams"]["expression"]
            .as_str()
            .unwrap_or_default();
        assert!(!expression.contains("document.elementFromPoint"));
        assert!(!expression.contains("const eventX"));
        assert!(expression.contains("window.scrollBy(deltaX, deltaY)"));
        assert!(expression.contains("const deltaX = 0"));
        assert!(expression.contains("const deltaY = 400"));
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
        0.0,
        400.0,
        None,
        None,
    )
    .await;
    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(response.diagnostics.is_empty());
    assert_eq!(response.action, "scroll");
}

#[tokio::test]
async fn dispatch_click_at_emits_focus_then_trusted_mouse_sequence() {
    // dispatch_click_at is the single code path both the coordinate click and
    // the element-targeted click/type arms use. Drive it directly on a
    // connected stream and assert the exact trusted sequence at the given
    // point: focus emulation, then mouseMoved / mousePressed / mouseReleased.
    // This is the resolve->dispatch success path minus the resolver, which is
    // integration-verified end to end once Stream 1A's live resolver lands.
    use std::time::Duration;

    use tokio::net::UnixStream;
    use tokio::time::Instant;

    use crate::browser::cdp::dispatch_click_at;
    use crate::browser::tabs::tab_id_value;

    let socket_dir = unique_test_dir("sky-cua-browser-dispatch-click-at");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let socket_path = socket_dir.join("extension-123-test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_and_ack_focus_emulation(&mut stream).await;
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
            assert_eq!(command["x"], 128.0);
            assert_eq!(command["y"], 96.0);
            if let Some(button) = expected_button {
                assert_eq!(command["button"], button);
                assert_eq!(command["clickCount"], 1);
            }
            write_frame(
                &mut stream,
                &json!({"jsonrpc": "2.0", "id": event["id"], "result": {}}),
            )
            .await
            .unwrap();
        }
    });

    let mut client = UnixStream::connect(&socket_path).await.unwrap();
    let tab_id = tab_id_value("515");
    let mut mutated = false;
    dispatch_click_at(
        &mut client,
        &socket_path,
        &tab_id,
        128.0,
        96.0,
        Instant::now() + Duration::from_secs(2),
        &mut mutated,
    )
    .await
    .expect("dispatch_click_at should succeed against the fake bridge");

    server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    // The compounding press/release raise the mutated flag so the executor
    // treats a click as not-replay-safe.
    assert!(
        mutated,
        "click press/release must mark the operation mutating"
    );
}

#[tokio::test]
async fn scroll_rejects_zero_deltas_before_cdp_dispatch() {
    let _env_guard = env_lock().await;

    let response = scroll(
        Some(BrowserTargetKind::UserChrome),
        "515".to_string(),
        0.0,
        0.0,
        None,
        None,
    )
    .await;

    assert_eq!(response.action, "scroll");
    assert_eq!(response.diagnostics.len(), 1);
    let diagnostic = response.diagnostics.first().expect("diagnostic");
    assert_eq!(diagnostic.code, "BrowserScrollInvalid");
    assert!(diagnostic.message.contains("at least one non-zero value"));
}
