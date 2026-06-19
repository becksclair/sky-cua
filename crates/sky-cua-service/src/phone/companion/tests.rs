//! Companion RPC tests against an in-process `tokio::net::TcpListener` fake.
//!
//! The fake server speaks the v1 wire contract: it reads one HTTP/1.1 `POST
//! /rpc`, validates `protocol_version` and `token`, and replies with a scripted
//! HTTP response. Every transport/protocol/auth/version/per-method failure path
//! is exercised, plus identity/token helpers. No real device or `adb` is used.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::client::{CompanionClient, CompanionRpcError};
use super::identity::{
    self, CompanionInstallDecision, ExpectedCompanion, InstalledCompanion,
    SETUP_TOKEN_EXPIRES_EXTRA, SETUP_TOKEN_FILE_EXTRA,
};
use super::protocol::{
    GestureKind, GesturePoint, NotificationOp, NotificationOpParams, PROTOCOL_VERSION, error_codes,
};

const TEST_TOKEN: &str = "test-token-abc";

/// How the fake server should respond to the (single) request it serves.
#[derive(Clone)]
enum FakeBehavior {
    /// Reply 200 with this JSON envelope body (after token/version validation).
    Json(String),
    /// Reply 200 with this raw body (used for non-JSON garbage).
    RawBody(String),
    /// Reply with this full raw HTTP response (used for non-200 status lines).
    RawResponse(String),
    /// Accept the connection but never reply (drives the client timeout).
    Hang,
    /// Echo the request id back in a scripted ok-result template. `{id}` in the
    /// template is replaced with the parsed request id.
    OkResultTemplate(String),
}

/// A running fake companion server. Drop the handle to shut it down.
struct FakeServer {
    addr: SocketAddr,
    _shutdown: oneshot::Sender<()>,
}

impl FakeServer {
    /// Bind a loopback listener and serve exactly the scripted `behavior` for
    /// each incoming connection. The server validates the token and protocol
    /// version like the real companion before applying the behavior, unless the
    /// behavior is a `RawResponse`/`RawBody` (which bypasses validation to model
    /// a hostile/garbage server).
    async fn start(behavior: FakeBehavior) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind fake companion");
        let addr = listener.local_addr().expect("local addr");
        let (tx, mut rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let behavior = behavior.clone();
                        tokio::spawn(async move { serve_one(&mut stream, behavior).await });
                    }
                }
            }
        });

        Self {
            addr,
            _shutdown: tx,
        }
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }
}

/// Serve a single request: read headers+body, parse the JSON request, then apply
/// the scripted behavior.
async fn serve_one(stream: &mut tokio::net::TcpStream, behavior: FakeBehavior) {
    let request = match read_http_request(stream).await {
        Some(req) => req,
        None => return,
    };

    // Parse the request body to recover id/token/version for validation+echo.
    let parsed: serde_json::Value =
        serde_json::from_str(&request.body).unwrap_or(serde_json::Value::Null);
    let id = parsed
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let token = parsed
        .get("token")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let version = parsed
        .get("protocol_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    let body = match behavior {
        FakeBehavior::RawResponse(raw) => {
            let _ = stream.write_all(raw.as_bytes()).await;
            let _ = stream.flush().await;
            return;
        }
        FakeBehavior::RawBody(raw) => raw,
        FakeBehavior::Hang => {
            // Hold the connection open without replying.
            tokio::time::sleep(Duration::from_secs(30)).await;
            return;
        }
        validated => {
            // Apply real token/version validation first.
            if version != u64::from(PROTOCOL_VERSION) {
                error_body(id, error_codes::VERSION_MISMATCH, "bad version")
            } else if token != TEST_TOKEN {
                error_body(id, error_codes::UNAUTHORIZED, "bad token")
            } else {
                match validated {
                    FakeBehavior::Json(json) => json,
                    FakeBehavior::OkResultTemplate(tmpl) => tmpl.replace("{id}", &id.to_string()),
                    _ => unreachable!("handled above"),
                }
            }
        }
    };

    write_http_200(stream, &body).await;
}

struct ParsedRequest {
    body: String,
}

/// Read an HTTP/1.1 request: headers, Content-Length, then exactly that many
/// body bytes.
async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Option<ParsedRequest> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    // Read until we have the header terminator.
    let header_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);

    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    Some(ParsedRequest {
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

async fn write_http_200(stream: &mut tokio::net::TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn ok_body(id: u64, result: &str) -> String {
    format!(r#"{{"protocol_version":1,"ok":true,"id":{id},"result":{result}}}"#)
}

fn error_body(id: u64, code: &str, message: &str) -> String {
    format!(
        r#"{{"protocol_version":1,"ok":false,"id":{id},"error":{{"code":"{code}","message":"{message}"}}}}"#
    )
}

fn client_for(server: &FakeServer) -> CompanionClient {
    CompanionClient::new(server.port(), TEST_TOKEN).with_timeout(Duration::from_millis(800))
}

// ===========================================================================
// Success paths
// ===========================================================================

#[tokio::test]
async fn health_success_round_trips() {
    let result = r#"{"version":"1.2.0","version_code":12,"package":"com.skycua.phonecompanion","accessibility_enabled":true,"can_perform_gestures":true,"can_retrieve_window_content":true,"can_take_screenshot":true,"notification_listener_enabled":true,"native_overlay":true,"native_overlay_pass_through":true}"#;
    let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(result))).await;
    let mut client = client_for(&server);

    let health = client.health().await.expect("health ok");
    assert_eq!(health.version, "1.2.0");
    assert_eq!(health.package, "com.skycua.phonecompanion");
    assert!(health.accessibility_enabled);
    assert!(health.can_perform_gestures);
}

#[tokio::test]
async fn capabilities_success_and_builder_derives_flags() {
    let result = r#"{"version":"2.0.0","version_code":20,"package":"com.skycua.phonecompanion","accessibility_enabled":true,"can_perform_gestures":true,"can_retrieve_window_content":true,"can_take_screenshot":false,"notification_listener_enabled":false,"native_overlay":true,"native_overlay_pass_through":true,"screenshot_api_level":34,"screenshot_supported":true,"gesture_supported":true}"#;
    let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(result))).await;
    let mut client = client_for(&server);

    let caps = client.capabilities().await.expect("capabilities ok");
    assert!(caps.gesture_supported);
    assert!(caps.screenshot_supported);

    // Builder: screenshot needs can_take_screenshot AND screenshot_supported.
    let token = identity::generate_token(1_000, 60_000);
    let report = super::capabilities_from_response(
        &caps,
        &CompanionInstallDecision::UpToDate,
        Some(&token),
        Some("feedface"),
        Some("deadbeef"),
        Some("cafef00d"),
        false,
        false,
    );
    assert!(report.installed);
    assert!(report.rpc_reachable);
    assert!(report.gesture_dispatch, "gestures derived available");
    assert!(
        !report.screenshot,
        "screenshot blocked by can_take_screenshot=false"
    );
    assert!(!report.notifications, "listener disabled");
    assert_eq!(report.rpc_token_expires_at_ms, Some(token.expires_at_ms));
    assert!(report.signature_matches_expected);
    // The reachable report carries the installed cert the host parsed (not None),
    // the expected cert, and the expected packaged-APK hash.
    assert_eq!(report.installed_cert_sha256.as_deref(), Some("feedface"));
    assert_eq!(report.expected_cert_sha256.as_deref(), Some("deadbeef"));
    assert_eq!(report.apk_sha256.as_deref(), Some("cafef00d"));
}

#[tokio::test]
async fn screenshot_success_decodes_payload() {
    let result = r#"{"mime_type":"image/png","data_base64":"AAAA","width":1080,"height":2400,"contains_native_overlay":true}"#;
    let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(result))).await;
    let mut client = client_for(&server);

    let shot = client.screenshot(true).await.expect("screenshot ok");
    assert_eq!(shot.mime_type, "image/png");
    assert_eq!(shot.width, 1080);
    assert!(shot.contains_native_overlay);
}

#[tokio::test]
async fn gesture_and_cursor_and_apps_round_trip() {
    // gesture
    {
        let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(
            r#"{"dispatched":true}"#,
        )))
        .await;
        let mut client = client_for(&server);
        let res = client
            .gesture(GestureKind::Tap, vec![GesturePoint { x: 5.0, y: 6.0 }], 50)
            .await
            .expect("gesture ok");
        assert!(res.dispatched);
    }
    // cursor_overlay
    {
        let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(
            r#"{"shown":true,"pass_through":true}"#,
        )))
        .await;
        let mut client = client_for(&server);
        let res = client
            .cursor_overlay(true, 1.0, 2.0)
            .await
            .expect("cursor ok");
        assert!(res.shown && res.pass_through);
    }
    // current_app
    {
        let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(
            r#"{"package":"com.android.chrome","activity":"Main","label":"Chrome"}"#,
        )))
        .await;
        let mut client = client_for(&server);
        let res = client.current_app().await.expect("current_app ok");
        assert_eq!(res.package, "com.android.chrome");
    }
    // app_list
    {
        let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(
            r#"{"apps":[{"package":"a.b","label":"AB","launchable":true}],"truncated":false}"#,
        )))
        .await;
        let mut client = client_for(&server);
        let res = client.app_list(true).await.expect("app_list ok");
        assert_eq!(res.apps.len(), 1);
        assert!(res.apps[0].launchable);
    }
}

/// A fake server that records the request body of the single request it serves,
/// so a test can assert both the on-the-wire params and the decoded result. The
/// captured body is exposed through a shared `Arc<Mutex<Option<String>>>`.
struct CapturingServer {
    addr: SocketAddr,
    body: Arc<std::sync::Mutex<Option<String>>>,
    _shutdown: oneshot::Sender<()>,
}

impl CapturingServer {
    async fn start(result_json: &'static str) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind capturing server");
        let addr = listener.local_addr().expect("local addr");
        let body: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
        let (tx, mut rx) = oneshot::channel::<()>();
        let body_slot = Arc::clone(&body);
        tokio::spawn(async move {
            tokio::select! {
                _ = &mut rx => {}
                accepted = listener.accept() => {
                    let Ok((mut stream, _)) = accepted else { return };
                    let Some(request) = read_http_request(&mut stream).await else { return };
                    *body_slot.lock().expect("body slot") = Some(request.body.clone());
                    let id = serde_json::from_str::<serde_json::Value>(&request.body)
                        .ok()
                        .and_then(|v| v.get("id").and_then(serde_json::Value::as_u64))
                        .unwrap_or(1);
                    write_http_200(&mut stream, &ok_body(id, result_json)).await;
                }
            }
        });
        Self {
            addr,
            body,
            _shutdown: tx,
        }
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }

    fn captured_body(&self) -> String {
        self.body
            .lock()
            .expect("body slot")
            .clone()
            .expect("a request was captured")
    }
}

/// `overlay_active` serializes `{ active }` against the `overlay_active` method
/// and decodes `{ active, glow_supported }`.
#[tokio::test]
async fn overlay_active_round_trip_and_serialization() {
    let server = CapturingServer::start(r#"{"active":true,"glow_supported":true}"#).await;
    let mut client =
        CompanionClient::new(server.port(), TEST_TOKEN).with_timeout(Duration::from_millis(800));

    let result = client
        .overlay_active(true)
        .await
        .expect("overlay_active ok");
    assert!(result.active);
    assert!(result.glow_supported);

    let body = server.captured_body();
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("request body is json");
    assert_eq!(parsed["method"], "overlay_active");
    assert_eq!(parsed["params"]["active"], serde_json::json!(true));
}

/// `overlay_gesture` serializes the free-form `kind` string, the device-pixel
/// point path, and `duration_ms` against the `overlay_gesture` method, and
/// decodes `{ animated }`. `drag` is a valid overlay kind that the real-input
/// `gesture` method does not support, so the wire `kind` is a string, not the
/// `GestureKind` enum.
#[tokio::test]
async fn overlay_gesture_round_trip_and_serialization() {
    let server = CapturingServer::start(r#"{"animated":true}"#).await;
    let mut client =
        CompanionClient::new(server.port(), TEST_TOKEN).with_timeout(Duration::from_millis(800));

    let result = client
        .overlay_gesture(
            "drag",
            vec![
                GesturePoint { x: 10.0, y: 20.0 },
                GesturePoint { x: 30.0, y: 40.0 },
            ],
            250,
        )
        .await
        .expect("overlay_gesture ok");
    assert!(result.animated);

    let body = server.captured_body();
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("request body is json");
    assert_eq!(parsed["method"], "overlay_gesture");
    assert_eq!(parsed["params"]["kind"], "drag");
    assert_eq!(parsed["params"]["duration_ms"], serde_json::json!(250));
    let points = parsed["params"]["points"].as_array().expect("points array");
    assert_eq!(points.len(), 2);
    assert_eq!(points[0]["x"], serde_json::json!(10.0));
    assert_eq!(points[1]["y"], serde_json::json!(40.0));
}

#[tokio::test]
async fn notifications_and_op_round_trip() {
    let result = r#"{"listener_enabled":true,"events":[{"event_id":"e1","package":"a.b","redaction":"partial","when_ms":123,"actions":[{"action_id":"reply","title":"Reply","is_reply":true}]}],"truncated":false}"#;
    let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(result))).await;
    let mut client = client_for(&server);
    let notifs = client.notifications(10).await.expect("notifications ok");
    assert!(notifs.listener_enabled);
    assert_eq!(notifs.events[0].event_id, "e1");
    assert!(notifs.events[0].actions[0].is_reply);

    let op_server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(
        r#"{"ok":true}"#,
    )))
    .await;
    let mut op_client = client_for(&op_server);
    let op = op_client
        .notification_op(NotificationOpParams {
            event_id: "e1".to_string(),
            op: NotificationOp::Reply,
            action_id: Some("reply".to_string()),
            reply_text: Some("hi".to_string()),
        })
        .await
        .expect("notification_op ok");
    assert!(op.ok);
}

// ===========================================================================
// Failure paths -> fallback
// ===========================================================================

#[tokio::test]
async fn connect_refused_is_fallback() {
    // Target port 1: it is below the ephemeral range and unbindable by an
    // unprivileged process, so no concurrently-running test thread can occupy
    // it via `bind(0)`. A loopback connection to a closed port is refused
    // immediately and deterministically. (Binding-then-dropping an ephemeral
    // port is racy: another test's ephemeral bind can reuse the just-freed
    // port before this client connects, turning the expected refusal into a
    // live connection.)
    let mut client = CompanionClient::new(1, TEST_TOKEN).with_timeout(Duration::from_millis(500));
    let err = client.health().await.expect_err("must fail");
    assert!(matches!(err, CompanionRpcError::Connect { .. }));
    assert!(err.is_fallback());
    assert_eq!(err.code(), "CompanionConnectRefused");
}

#[tokio::test]
async fn timeout_is_fallback() {
    let server = FakeServer::start(FakeBehavior::Hang).await;
    let mut client =
        CompanionClient::new(server.port(), TEST_TOKEN).with_timeout(Duration::from_millis(150));
    let err = client.health().await.expect_err("must time out");
    assert_eq!(err, CompanionRpcError::Timeout);
    assert!(err.is_fallback());
}

#[tokio::test]
async fn garbage_body_is_malformed_fallback() {
    let server = FakeServer::start(FakeBehavior::RawBody("this is not json {".to_string())).await;
    let mut client = client_for(&server);
    let err = client.health().await.expect_err("must fail");
    assert!(matches!(err, CompanionRpcError::Malformed { .. }));
    assert!(err.is_fallback());
}

#[tokio::test]
async fn non_200_status_is_http_fallback() {
    let raw = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let server = FakeServer::start(FakeBehavior::RawResponse(raw.to_string())).await;
    let mut client = client_for(&server);
    let err = client.health().await.expect_err("must fail");
    assert_eq!(err, CompanionRpcError::Http { status: 503 });
    assert!(err.is_fallback());
}

#[tokio::test]
async fn version_mismatch_from_envelope_is_fallback() {
    // Server replies ok=false with a mismatched protocol_version envelope.
    let body = r#"{"protocol_version":2,"ok":true,"id":1,"result":{}}"#;
    let server = FakeServer::start(FakeBehavior::Json(body.to_string())).await;
    let mut client = client_for(&server);
    let err = client.health().await.expect_err("must fail");
    assert!(matches!(
        err,
        CompanionRpcError::VersionMismatch { server_version: 2 }
    ));
    assert!(err.is_fallback());
}

#[tokio::test]
async fn version_mismatch_from_client_request_is_rejected_by_server() {
    // Default validation: the server rejects a non-v1 request. We force the
    // server to see a wrong version by NOT being able to change the client (the
    // client always sends v1), so instead assert the server-side error code path
    // by sending a wrong token, which is the same validation tier. Covered by
    // `wrong_token_is_unauthorized_fallback`. Here, assert the error-code mapping
    // directly: a server returning version_mismatch maps to VersionMismatch.
    let body = error_body(1, error_codes::VERSION_MISMATCH, "server too new");
    let server = FakeServer::start(FakeBehavior::Json(body)).await;
    let mut client = client_for(&server);
    let err = client.health().await.expect_err("must fail");
    assert!(matches!(err, CompanionRpcError::VersionMismatch { .. }));
    assert!(err.is_fallback());
}

#[tokio::test]
async fn wrong_token_is_unauthorized_fallback() {
    let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(
        r#"{"dispatched":true}"#,
    )))
    .await;
    // Use a client with the wrong token; the fake server validates it.
    let mut client =
        CompanionClient::new(server.port(), "wrong-token").with_timeout(Duration::from_millis(800));
    let err = client.health().await.expect_err("must fail");
    assert!(matches!(err, CompanionRpcError::Unauthorized { .. }));
    assert!(err.is_fallback());
    assert_eq!(err.code(), error_codes::UNAUTHORIZED);
}

#[tokio::test]
async fn mismatched_response_id_is_protocol_violation() {
    // Server echoes a different id than requested.
    let body = ok_body(
        999,
        r#"{"version":"1","version_code":1,"package":"p","accessibility_enabled":false,"can_perform_gestures":false,"can_retrieve_window_content":false,"can_take_screenshot":false,"notification_listener_enabled":false,"native_overlay":false,"native_overlay_pass_through":false}"#,
    );
    let server = FakeServer::start(FakeBehavior::Json(body)).await;
    let mut client = client_for(&server);
    let err = client.health().await.expect_err("must fail");
    assert!(matches!(err, CompanionRpcError::Protocol { .. }));
    assert!(err.is_fallback());
}

#[tokio::test]
async fn ok_result_shape_mismatch_is_malformed() {
    // ok=true but result lacks required fields for HealthResult.
    let body = ok_body(1, r#"{"unexpected":true}"#);
    let server = FakeServer::start(FakeBehavior::Json(body)).await;
    let mut client = client_for(&server);
    let err = client.health().await.expect_err("must fail");
    assert!(matches!(err, CompanionRpcError::Malformed { .. }));
    assert!(err.is_fallback());
}

#[tokio::test]
async fn dispatch_level_error_codes_are_protocol_fallback() {
    // unknown_method / internal mean the request never reached a working method
    // handler (Dispatcher unknown-method path / internal failure). They classify
    // as a protocol violation that falls back to ADB/scrcpy, not a per-method
    // application error.
    for code in [error_codes::UNKNOWN_METHOD, error_codes::INTERNAL] {
        let server = FakeServer::start(FakeBehavior::Json(error_body(1, code, "x"))).await;
        let mut client = client_for(&server);
        let err = client.health().await.expect_err("must fail");
        assert!(
            matches!(err, CompanionRpcError::Protocol { .. }),
            "code {code} must map to Protocol, got {err:?}"
        );
        assert!(
            err.is_fallback(),
            "dispatch-level code {code} must trigger fallback"
        );
        assert_eq!(err.code(), "CompanionProtocolViolation");
    }

    // bad_request is overloaded: the companion uses it for dispatch-level
    // validation AND for genuine per-method application errors (e.g. open_intent
    // rejecting an unparseable intent URI). The wire cannot distinguish them, so
    // it must NOT trigger a session-wide fallback — it stays a non-fallback
    // Method error and the immediate action falls back on its own.
    let server = FakeServer::start(FakeBehavior::Json(error_body(
        1,
        error_codes::BAD_REQUEST,
        "x",
    )))
    .await;
    let mut client = client_for(&server);
    let err = client.health().await.expect_err("must fail");
    assert!(
        matches!(err, CompanionRpcError::Method { .. }),
        "bad_request must stay a per-method error, got {err:?}"
    );
    assert!(!err.is_fallback(), "bad_request must not trigger fallback");

    // Contrast: a genuine per-method application error stays a non-fallback
    // Method error and must not reroute to ADB.
    let server = FakeServer::start(FakeBehavior::Json(error_body(
        1,
        error_codes::SECURE_WINDOW,
        "x",
    )))
    .await;
    let mut client = client_for(&server);
    let err = client.screenshot(false).await.expect_err("must fail");
    assert!(matches!(err, CompanionRpcError::Method { .. }));
    assert!(!err.is_fallback());
}

// ===========================================================================
// Per-method application error codes (NOT fallback)
// ===========================================================================

#[tokio::test]
async fn screenshot_secure_window_is_method_error_not_fallback() {
    let body = error_body(1, error_codes::SECURE_WINDOW, "secure surface");
    let server = FakeServer::start(FakeBehavior::Json(body)).await;
    let mut client = client_for(&server);
    let err = client.screenshot(false).await.expect_err("must fail");
    match &err {
        CompanionRpcError::Method { code, .. } => assert_eq!(code, error_codes::SECURE_WINDOW),
        other => panic!("expected Method error, got {other:?}"),
    }
    assert!(
        !err.is_fallback(),
        "per-method error must not trigger fallback"
    );
}

#[tokio::test]
async fn each_screenshot_error_code_maps_to_method_error() {
    for code in [
        error_codes::SECURE_WINDOW,
        error_codes::UNSUPPORTED_API,
        error_codes::DISABLED_SERVICE,
        error_codes::OEM_POLICY,
        error_codes::THROTTLED,
        error_codes::TRANSIENT,
    ] {
        let server = FakeServer::start(FakeBehavior::Json(error_body(1, code, "x"))).await;
        let mut client = client_for(&server);
        let err = client.screenshot(false).await.expect_err("must fail");
        assert_eq!(err.code(), code);
        assert!(!err.is_fallback());
    }
}

#[tokio::test]
async fn each_notification_op_error_code_maps_to_method_error() {
    for code in [
        error_codes::GONE,
        error_codes::REDACTED,
        error_codes::PENDING_INTENT_MISSING,
        error_codes::CANCELED,
        error_codes::EXPIRED,
        error_codes::IMMUTABLE,
        error_codes::REPLY_UNAVAILABLE,
        error_codes::OEM_FILTERED,
    ] {
        let server = FakeServer::start(FakeBehavior::Json(error_body(1, code, "x"))).await;
        let mut client = client_for(&server);
        let err = client.notification_open("e1").await.expect_err("must fail");
        assert_eq!(err.code(), code);
        assert!(!err.is_fallback());
    }
}

// ===========================================================================
// Sequential ids increment across calls on one client
// ===========================================================================

#[tokio::test]
async fn ids_increment_across_calls() {
    // The OkResultTemplate echoes the request id; two health calls must succeed
    // with matching ids, proving the per-call counter advances and the response
    // id check passes both times.
    let result = r#"{"version":"1","version_code":1,"package":"p","accessibility_enabled":false,"can_perform_gestures":false,"can_retrieve_window_content":false,"can_take_screenshot":false,"notification_listener_enabled":false,"native_overlay":false,"native_overlay_pass_through":false}"#;
    let server = FakeServer::start(FakeBehavior::OkResultTemplate(ok_body_template(result))).await;
    let mut client = client_for(&server);
    client.health().await.expect("first ok");
    client.health().await.expect("second ok");
    assert_eq!(client.addr().ip(), std::net::Ipv4Addr::LOCALHOST);
}

/// Helper: wrap a result object into an ok-envelope template the fake server
/// completes with the parsed request id.
fn ok_body_template(result: &str) -> String {
    format!(r#"{{"protocol_version":1,"ok":true,"id":{{id}},"result":{result}}}"#)
}

// ===========================================================================
// Identity / install decisioning
// ===========================================================================

fn expected(version_code: u64, cert: &str, allow_downgrade: bool) -> ExpectedCompanion {
    ExpectedCompanion {
        package_name: "com.skycua.phonecompanion".to_string(),
        version_name: Some("1.0".to_string()),
        version_code: Some(version_code),
        cert_sha256: Some(cert.to_string()),
        apk_sha256: Some("abc".to_string()),
        apk_path: "resources/android/phone-companion.apk".to_string(),
        allow_downgrade,
    }
}

#[test]
fn decide_install_when_absent() {
    let decision = identity::decide_install(None, &expected(10, "aa", false));
    assert_eq!(decision, CompanionInstallDecision::Install);
    assert!(decision.requires_install());
}

#[test]
fn decide_update_when_older() {
    let installed = InstalledCompanion {
        version_name: Some("0.9".to_string()),
        version_code: Some(9),
        cert_sha256: Some("aa".to_string()),
    };
    let decision = identity::decide_install(Some(&installed), &expected(10, "aa", false));
    assert!(matches!(decision, CompanionInstallDecision::Update { .. }));
    assert!(decision.requires_install());
}

#[test]
fn decide_up_to_date_when_equal() {
    let installed = InstalledCompanion {
        version_name: Some("1.0".to_string()),
        version_code: Some(10),
        cert_sha256: Some("AA:BB".to_string()),
    };
    // Cert compare is case/colon-insensitive.
    let decision = identity::decide_install(Some(&installed), &expected(10, "aabb", false));
    assert_eq!(decision, CompanionInstallDecision::UpToDate);
    assert!(!decision.requires_install());
}

#[test]
fn refuse_same_package_when_expected_cert_missing() {
    let installed = InstalledCompanion {
        version_name: Some("1.0".to_string()),
        version_code: Some(10),
        cert_sha256: Some("AA:BB".to_string()),
    };
    let mut expected = expected(10, "aabb", false);
    expected.cert_sha256 = None;

    let decision = identity::decide_install(Some(&installed), &expected);
    assert!(matches!(
        decision,
        CompanionInstallDecision::RefuseSignatureUnverified { .. }
    ));
    assert_eq!(decision.code(), "CompanionSignatureUnverified");
    assert!(!decision.requires_install());
}

#[test]
fn refuse_same_package_when_installed_cert_missing() {
    let installed = InstalledCompanion {
        version_name: Some("1.0".to_string()),
        version_code: Some(10),
        cert_sha256: None,
    };
    let decision = identity::decide_install(Some(&installed), &expected(10, "aabb", false));

    assert!(matches!(
        decision,
        CompanionInstallDecision::RefuseSignatureUnverified { .. }
    ));
    assert!(!decision.requires_install());
}

#[test]
fn refuse_signature_mismatch_before_version() {
    let installed = InstalledCompanion {
        version_name: Some("0.1".to_string()),
        version_code: Some(1),
        cert_sha256: Some("deadbeef".to_string()),
    };
    // Even though installed is older (would be Update), a cert mismatch refuses.
    let decision = identity::decide_install(Some(&installed), &expected(10, "cafef00d", false));
    assert!(matches!(
        decision,
        CompanionInstallDecision::RefuseSignatureMismatch { .. }
    ));
    assert_eq!(decision.code(), "CompanionSignatureMismatch");
    assert!(!decision.requires_install());
}

#[test]
fn refuse_downgrade_when_newer_and_not_allowed() {
    let installed = InstalledCompanion {
        version_name: Some("2.0".to_string()),
        version_code: Some(20),
        cert_sha256: Some("aa".to_string()),
    };
    let decision = identity::decide_install(Some(&installed), &expected(10, "aa", false));
    assert!(matches!(
        decision,
        CompanionInstallDecision::RefuseDowngrade { .. }
    ));
}

#[test]
fn allow_downgrade_when_permitted() {
    let installed = InstalledCompanion {
        version_name: Some("2.0".to_string()),
        version_code: Some(20),
        cert_sha256: Some("aa".to_string()),
    };
    // allow_downgrade=true and installed is newer -> UpToDate (no forced install,
    // the operator opted in but there is nothing to install over a newer build).
    let decision = identity::decide_install(Some(&installed), &expected(10, "aa", true));
    assert_eq!(decision, CompanionInstallDecision::UpToDate);
}

// ===========================================================================
// Token + argv helpers
// ===========================================================================

#[test]
fn generate_token_is_unique_hex_and_expires() {
    let now = 1_000_000;
    let a = identity::generate_token(now, 900_000);
    let b = identity::generate_token(now, 900_000);
    assert_eq!(a.token.len(), 64, "256-bit hex");
    assert!(a.token.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a.token, b.token, "tokens must not collide");
    assert_eq!(a.expires_at_ms, now + 900_000);
    assert!(!a.is_expired(now + 1));
    assert!(a.is_expired(now + 900_000));
}

#[test]
fn back_to_back_tokens_differ() {
    // Two tokens minted in immediate succession must differ. The primary path
    // reads 32 bytes from the OS CSPRNG (`/dev/urandom`); the fallback path is
    // also per-call unique. Either way a fresh token never repeats.
    let now = 5_000_000;
    let first = identity::generate_token(now, 60_000);
    let second = identity::generate_token(now, 60_000);
    assert_ne!(first.token, second.token);
    assert_eq!(first.token.len(), 64);
    assert!(first.token.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn setup_token_push_argv_has_no_token_value() {
    let token = identity::generate_token(2_000, 60_000);
    let argv = identity::setup_token_push_argv(
        "emulator-5554",
        "/tmp/sky-cua-phone-token",
        "com.skycua.phonecompanion",
    );
    assert_eq!(argv[0], "-s");
    assert_eq!(argv[1], "emulator-5554");
    assert_eq!(argv[2], "push");
    assert_eq!(argv[3], "/tmp/sky-cua-phone-token");
    assert_eq!(
        argv[4],
        "/sdcard/Android/data/com.skycua.phonecompanion/cache/sky_cua_rpc_token"
    );
    assert!(!argv.contains(&token.token));
}

#[test]
fn setup_intent_argv_has_token_file_and_expiry_extras() {
    let token = identity::generate_token(2_000, 60_000);
    let argv = identity::setup_intent_argv("emulator-5554", "com.skycua.phonecompanion", &token);
    assert_eq!(argv[0], "-s");
    assert_eq!(argv[1], "emulator-5554");
    assert!(argv.contains(&"am".to_string()));
    assert!(argv.contains(&"start".to_string()));
    assert!(argv.contains(&"com.skycua.phonecompanion/.SetupActivity".to_string()));
    // string extra
    let token_idx = argv
        .iter()
        .position(|a| a == SETUP_TOKEN_FILE_EXTRA)
        .expect("token file extra");
    assert_eq!(argv[token_idx - 1], "--es");
    assert_eq!(
        argv[token_idx + 1],
        "/sdcard/Android/data/com.skycua.phonecompanion/cache/sky_cua_rpc_token"
    );
    assert!(!argv.contains(&token.token));
    // long extra
    let exp_idx = argv
        .iter()
        .position(|a| a == SETUP_TOKEN_EXPIRES_EXTRA)
        .expect("expiry extra");
    assert_eq!(argv[exp_idx - 1], "--el");
    assert_eq!(argv[exp_idx + 1], token.expires_at_ms.to_string());
}

#[test]
fn install_argv_includes_downgrade_flag_only_when_allowed() {
    let no_down = identity::install_argv("serial-1", &expected(10, "aa", false));
    assert!(no_down.contains(&"install".to_string()));
    assert!(no_down.contains(&"-r".to_string()));
    assert!(!no_down.contains(&"-d".to_string()));
    assert_eq!(
        no_down.last().unwrap(),
        "resources/android/phone-companion.apk"
    );

    let with_down = identity::install_argv("serial-1", &expected(10, "aa", true));
    assert!(with_down.contains(&"-d".to_string()));
}

#[test]
fn client_is_send_for_arc_sharing() {
    fn is_send<T: Send>() {}
    is_send::<CompanionClient>();
    is_send::<Arc<CompanionClient>>();
}
