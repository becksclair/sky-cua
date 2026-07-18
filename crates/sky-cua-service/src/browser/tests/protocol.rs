//! Bridge frame I/O and request timeout tests.

use std::path::Path;

use serde_json::{Value, json};
use sky_cua_platform::BrowserSessionIdentity;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

use crate::browser::protocol::{BRIDGE_INFO_REQUEST_ID, MAX_FRAME_SIZE, read_frame, write_frame};
use crate::browser::transport::{
    bridge_request_timeout, browser_session_params, send_bridge_request,
};

use super::helpers::env_lock;

fn test_identity() -> BrowserSessionIdentity {
    BrowserSessionIdentity {
        session_id: "sky-cua-mcp".to_string(),
        turn_id: "browser-test".to_string(),
        thread_id: None,
    }
}

#[tokio::test]
async fn bridge_request_times_out_when_peer_only_sends_pings() {
    // Observes the request timeout firing, so pin it short rather than using the
    // generous test default. env_lock serializes the override with other browser
    // tests that read it and restores it when the guard drops.
    let _env_guard = env_lock().await;
    unsafe { std::env::set_var("SKY_CUA_TEST_BRIDGE_REQUEST_TIMEOUT_MS", "100") };

    let (mut client, mut server) = UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let request = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("getInfo")
        );
        let mut interval = tokio::time::interval(bridge_request_timeout() / 4);
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
        browser_session_params(&test_identity()),
    )
    .await
    .unwrap_err();
    server.abort();

    assert_eq!(diagnostic.code, "BrowserBridgeRequestTimedOut");
}

#[tokio::test]
async fn send_bridge_request_skips_belated_reply_to_prior_own_request() {
    // On a reused stream a sluggish relay can answer an earlier request only
    // after the service moved on to the next one. That stale own-reply must be
    // dropped, not treated as an unexpected response that fails the current
    // command.
    let _env_guard = env_lock().await;
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let request = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("getInfo")
        );
        // A belated reply to one of our own earlier requests (prefixed id).
        write_frame(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": "sky-cua-browser-attach-tab",
                "result": {}
            }),
        )
        .await
        .unwrap();
        // The real reply to the in-flight getInfo.
        write_frame(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": BRIDGE_INFO_REQUEST_ID,
                "result": {"name": "sky-cua-test-bridge"}
            }),
        )
        .await
        .unwrap();
    });

    let response = send_bridge_request(
        &mut client,
        Path::new("/tmp/sky-cua-browser-stale.sock"),
        BRIDGE_INFO_REQUEST_ID,
        "getInfo",
        browser_session_params(&test_identity()),
    )
    .await
    .unwrap();
    server.await.unwrap();

    assert_eq!(
        response.get("id").and_then(Value::as_str),
        Some(BRIDGE_INFO_REQUEST_ID)
    );
}

#[tokio::test]
async fn send_bridge_request_rejects_foreign_response() {
    // A response id that is not one of ours (no shared prefix) and carries no
    // method is a genuinely foreign/malformed frame and must still hard-fail —
    // the stale-own-reply skip must not swallow it.
    let _env_guard = env_lock().await;
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let request = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("getInfo")
        );
        write_frame(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": "totally-unrelated-id",
                "result": {}
            }),
        )
        .await
        .unwrap();
    });

    let diagnostic = send_bridge_request(
        &mut client,
        Path::new("/tmp/sky-cua-browser-foreign.sock"),
        BRIDGE_INFO_REQUEST_ID,
        "getInfo",
        browser_session_params(&test_identity()),
    )
    .await
    .unwrap_err();
    server.await.unwrap();

    assert_eq!(diagnostic.code, "BrowserBridgeRequestFailed");
    assert!(
        diagnostic
            .message
            .contains("unexpected browser tab response")
    );
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
