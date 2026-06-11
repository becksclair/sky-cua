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
async fn scroll_dispatches_css_pixel_coordinates_directly() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-scroll-css-pixels");
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
