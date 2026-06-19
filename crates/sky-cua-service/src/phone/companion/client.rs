//! Hand-rolled HTTP/1.1 JSON-RPC client for the companion app.
//!
//! The companion exposes a localhost-only `POST /rpc` endpoint inside the app,
//! reachable on the host through `adb forward tcp:PORT tcp:PORT`. This client
//! connects to `127.0.0.1:PORT` over a [`tokio::net::TcpStream`], frames a
//! minimal HTTP/1.1 request by hand (no new dependency), reads one response, and
//! decodes the JSON envelope from [`super::protocol`].
//!
//! Every fallible path produces a [`CompanionRpcError`] whose
//! [`CompanionRpcError::is_fallback`] is `true` for transport/protocol failures
//! that mean "route to ADB/scrcpy instead". Per-method application error codes
//! (e.g. screenshot `secure_window`) are surfaced as
//! [`CompanionRpcError::Method`] so the caller can map them to structured phone
//! diagnostics without pretending the operation succeeded.
//!
//! Until the integrator wires `CompanionClient` into `manager.rs`, the client
//! surface is only reached from tests. The module-level expectation keeps
//! non-test builds clean (the spine's `expect(dead_code)` idiom) and becomes
//! self-removing once routing constructs and calls the client.
#![cfg_attr(not(test), expect(dead_code))]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::protocol::{
    AccessibilityTreeParams, AccessibilityTreeResult, AppListParams, AppListResult, AppOp,
    AppOpParams, AppOpResult, CapabilitiesResult, CurrentAppResult, CursorOverlayParams,
    CursorOverlayResult, GestureKind, GestureParams, GesturePoint, GestureResult, HealthResult,
    NoParams, NotificationOp, NotificationOpParams, NotificationOpResult, NotificationsParams,
    NotificationsResult, OverlayActiveParams, OverlayActiveResult, OverlayGestureParams,
    OverlayGestureResult, PROTOCOL_VERSION, RpcEnvelope, RpcError, RpcRequest, ScreenshotParams,
    ScreenshotResult, error_codes, methods,
};

/// Default per-call timeout. The companion endpoint is local (over `adb
/// forward`), so a healthy round trip is sub-second; anything slower is treated
/// as unreachable and falls back.
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(4_000);

/// Cap on the response body we will buffer, so a malformed/hostile companion
/// cannot exhaust host memory. Screenshots are the largest legitimate payload;
/// 32 MiB comfortably covers a base64 full-resolution PNG.
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Why a companion RPC call failed. Transport/protocol variants set
/// [`CompanionRpcError::is_fallback`] so the manager routes to ADB/scrcpy; the
/// [`CompanionRpcError::Method`] variant carries an application error code the
/// caller maps to a structured phone diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompanionRpcError {
    /// TCP connect refused/failed (companion not running, forward not set up).
    Connect { message: String },
    /// The call exceeded the timeout. Treated as unreachable.
    Timeout,
    /// Socket read/write failed mid-exchange.
    Io { message: String },
    /// The HTTP status line was missing or non-200.
    Http { status: u16 },
    /// The response body was not valid JSON or did not match the envelope shape.
    Malformed { message: String },
    /// The envelope `protocol_version` did not match [`PROTOCOL_VERSION`], or the
    /// companion returned the `version_mismatch` error code.
    VersionMismatch { server_version: u32 },
    /// The token was missing/wrong/expired (server `unauthorized` error code).
    Unauthorized { message: String },
    /// A well-formed application-level error for the method, e.g. screenshot
    /// `secure_window` or notification `gone`. Not a transport failure.
    Method { code: String, message: String },
    /// The response `id` did not match the request `id`, or `ok` and the
    /// `result`/`error` fields were inconsistent.
    Protocol { message: String },
}

impl CompanionRpcError {
    /// Whether this failure should make the manager fall back to ADB/scrcpy.
    /// True for every transport/protocol/auth/version failure; false only for a
    /// well-formed per-method application error, which the caller maps to a
    /// structured diagnostic on the companion backend itself.
    pub(crate) fn is_fallback(&self) -> bool {
        !matches!(self, CompanionRpcError::Method { .. })
    }

    /// Stable diagnostic code, so callers route on the field rather than prose.
    pub(crate) fn code(&self) -> &str {
        match self {
            CompanionRpcError::Connect { .. } => "CompanionConnectRefused",
            CompanionRpcError::Timeout => "CompanionTimeout",
            CompanionRpcError::Io { .. } => "CompanionIo",
            CompanionRpcError::Http { .. } => "CompanionHttpStatus",
            CompanionRpcError::Malformed { .. } => "CompanionMalformedResponse",
            CompanionRpcError::VersionMismatch { .. } => "CompanionVersionMismatch",
            CompanionRpcError::Unauthorized { .. } => error_codes::UNAUTHORIZED,
            CompanionRpcError::Method { code, .. } => code.as_str(),
            CompanionRpcError::Protocol { .. } => "CompanionProtocolViolation",
        }
    }
}

impl std::fmt::Display for CompanionRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompanionRpcError::Connect { message } => {
                write!(f, "companion connect failed: {message}")
            }
            CompanionRpcError::Timeout => write!(f, "companion RPC timed out"),
            CompanionRpcError::Io { message } => write!(f, "companion RPC io error: {message}"),
            CompanionRpcError::Http { status } => {
                write!(f, "companion RPC returned HTTP {status}")
            }
            CompanionRpcError::Malformed { message } => {
                write!(f, "companion response was malformed: {message}")
            }
            CompanionRpcError::VersionMismatch { server_version } => {
                write!(
                    f,
                    "companion protocol version mismatch: host={PROTOCOL_VERSION} server={server_version}"
                )
            }
            CompanionRpcError::Unauthorized { message } => {
                write!(f, "companion rejected token: {message}")
            }
            CompanionRpcError::Method { code, message } => {
                write!(f, "companion method error [{code}]: {message}")
            }
            CompanionRpcError::Protocol { message } => {
                write!(f, "companion protocol violation: {message}")
            }
        }
    }
}

/// A typed result from a companion RPC call.
pub(crate) type CompanionResult<T> = Result<T, CompanionRpcError>;

/// HTTP/1.1 JSON-RPC client bound to one companion endpoint + session token.
///
/// The client is cheap to clone and holds no socket; each call opens a fresh
/// `TcpStream` (HTTP/1.1 with `Connection: close`), matching how the companion
/// app serves one request per connection. `id` is a monotonically increasing
/// per-call counter so responses can be matched to requests.
#[derive(Debug, Clone)]
pub(crate) struct CompanionClient {
    addr: SocketAddr,
    token: String,
    timeout: Duration,
    next_id: u64,
}

impl CompanionClient {
    /// Build a client for the loopback `port` the host forwarded to, plus the
    /// ephemeral session `token`. The integrator constructs one of these after
    /// `adb forward` and token provisioning succeed.
    pub(crate) fn new(port: u16, token: impl Into<String>) -> Self {
        Self {
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            token: token.into(),
            timeout: DEFAULT_TIMEOUT,
            next_id: 1,
        }
    }

    /// Override the per-call timeout (used by tests and slow-link tuning).
    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The loopback address this client dials.
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    // -------------------------------------------------------------------
    // Typed RPC methods. Each maps a request DTO to a result DTO and lets the
    // transport/protocol layer surface fallback-worthy failures.
    // -------------------------------------------------------------------

    /// `health` — liveness plus permission/capability booleans.
    pub(crate) async fn health(&mut self) -> CompanionResult<HealthResult> {
        self.call(methods::HEALTH, NoParams {}).await
    }

    /// `capabilities` — health fields plus screenshot/gesture support detail.
    pub(crate) async fn capabilities(&mut self) -> CompanionResult<CapabilitiesResult> {
        self.call(methods::CAPABILITIES, NoParams {}).await
    }

    /// `accessibility_tree` — bounded active-window node list.
    pub(crate) async fn accessibility_tree(
        &mut self,
        max_nodes: u32,
    ) -> CompanionResult<AccessibilityTreeResult> {
        self.call(
            methods::ACCESSIBILITY_TREE,
            AccessibilityTreeParams { max_nodes },
        )
        .await
    }

    /// `screenshot` — on-device capture. Per-method error codes
    /// (`secure_window`, `unsupported_api`, ...) surface as
    /// [`CompanionRpcError::Method`].
    pub(crate) async fn screenshot(
        &mut self,
        include_overlay: bool,
    ) -> CompanionResult<ScreenshotResult> {
        self.call(methods::SCREENSHOT, ScreenshotParams { include_overlay })
            .await
    }

    /// `gesture` — dispatch a tap or swipe over a device-pixel point path.
    pub(crate) async fn gesture(
        &mut self,
        kind: GestureKind,
        points: Vec<GesturePoint>,
        duration_ms: u32,
    ) -> CompanionResult<GestureResult> {
        self.call(
            methods::GESTURE,
            GestureParams {
                kind,
                points,
                duration_ms,
            },
        )
        .await
    }

    /// `cursor_overlay` — show/move/hide the phone-native cursor overlay.
    pub(crate) async fn cursor_overlay(
        &mut self,
        visible: bool,
        x: f64,
        y: f64,
    ) -> CompanionResult<CursorOverlayResult> {
        self.call(
            methods::CURSOR_OVERLAY,
            CursorOverlayParams { visible, x, y },
        )
        .await
    }

    /// `overlay_active` — toggle the persistent "agent in control" breathing
    /// edge glow. `glow_supported` is false only when the accessibility service
    /// is unavailable.
    pub(crate) async fn overlay_active(
        &mut self,
        active: bool,
    ) -> CompanionResult<OverlayActiveResult> {
        self.call(methods::OVERLAY_ACTIVE, OverlayActiveParams { active })
            .await
    }

    /// `overlay_gesture` — animate the agent cursor for one action (tap ripple,
    /// swipe/drag trail) and pulse the edge glow. Visual only; it does not
    /// dispatch real input.
    pub(crate) async fn overlay_gesture(
        &mut self,
        kind: &str,
        points: Vec<GesturePoint>,
        duration_ms: u32,
    ) -> CompanionResult<OverlayGestureResult> {
        self.call(
            methods::OVERLAY_GESTURE,
            OverlayGestureParams {
                kind: kind.to_string(),
                points,
                duration_ms,
            },
        )
        .await
    }

    /// `notifications` — bounded recent notification events.
    pub(crate) async fn notifications(&mut self, max: u32) -> CompanionResult<NotificationsResult> {
        self.call(methods::NOTIFICATIONS, NotificationsParams { max })
            .await
    }

    /// `notification_op` — open/dismiss/action/reply on an explicit event id.
    pub(crate) async fn notification_op(
        &mut self,
        params: NotificationOpParams,
    ) -> CompanionResult<NotificationOpResult> {
        self.call(methods::NOTIFICATION_OP, params).await
    }

    /// Convenience: open a notification by event id.
    pub(crate) async fn notification_open(
        &mut self,
        event_id: impl Into<String>,
    ) -> CompanionResult<NotificationOpResult> {
        self.notification_op(NotificationOpParams {
            event_id: event_id.into(),
            op: NotificationOp::Open,
            action_id: None,
            reply_text: None,
        })
        .await
    }

    /// `current_app` — foreground package/activity/label.
    pub(crate) async fn current_app(&mut self) -> CompanionResult<CurrentAppResult> {
        self.call(methods::CURRENT_APP, NoParams {}).await
    }

    /// `app_list` — installed app inventory.
    pub(crate) async fn app_list(
        &mut self,
        launchable_only: bool,
    ) -> CompanionResult<AppListResult> {
        self.call(methods::APP_LIST, AppListParams { launchable_only })
            .await
    }

    /// `app_op` — launch/open-intent/force-stop.
    pub(crate) async fn app_op(
        &mut self,
        op: AppOp,
        package: Option<String>,
        intent_uri: Option<String>,
    ) -> CompanionResult<AppOpResult> {
        self.call(
            methods::APP_OP,
            AppOpParams {
                op,
                package,
                intent_uri,
            },
        )
        .await
    }

    // -------------------------------------------------------------------
    // Transport
    // -------------------------------------------------------------------

    /// Encode `params`, send one HTTP/1.1 POST, read one response, and decode
    /// the typed `result`. Wraps the whole exchange in the per-call timeout.
    async fn call<P, R>(&mut self, method: &str, params: P) -> CompanionResult<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);

        let request = RpcRequest::new(self.token.clone(), id, method, params);
        let body = serde_json::to_vec(&request).map_err(|e| CompanionRpcError::Malformed {
            message: e.to_string(),
        })?;

        let envelope = match tokio::time::timeout(self.timeout, self.exchange(&body)).await {
            Ok(result) => result?,
            Err(_) => return Err(CompanionRpcError::Timeout),
        };

        decode_result(envelope, id)
    }

    /// Open a connection, write the framed request, and read the full response
    /// body. Returns the parsed JSON envelope. Connection failures map to
    /// [`CompanionRpcError::Connect`]; mid-stream failures to
    /// [`CompanionRpcError::Io`].
    async fn exchange(&self, body: &[u8]) -> CompanionResult<RpcEnvelope> {
        let mut stream =
            TcpStream::connect(self.addr)
                .await
                .map_err(|e| CompanionRpcError::Connect {
                    message: e.to_string(),
                })?;

        let head = format!(
            "POST /rpc HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            body.len()
        );

        stream
            .write_all(head.as_bytes())
            .await
            .map_err(|e| CompanionRpcError::Io {
                message: e.to_string(),
            })?;
        stream
            .write_all(body)
            .await
            .map_err(|e| CompanionRpcError::Io {
                message: e.to_string(),
            })?;
        stream.flush().await.map_err(|e| CompanionRpcError::Io {
            message: e.to_string(),
        })?;

        let raw = read_capped(&mut stream).await?;
        let (status, response_body) = split_http_response(&raw)?;
        if status != 200 {
            return Err(CompanionRpcError::Http { status });
        }

        serde_json::from_slice::<RpcEnvelope>(response_body).map_err(|e| {
            CompanionRpcError::Malformed {
                message: e.to_string(),
            }
        })
    }
}

/// Read the response until EOF (the companion sends `Connection: close`),
/// bounded by [`MAX_RESPONSE_BYTES`].
async fn read_capped(stream: &mut TcpStream) -> CompanionResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| CompanionRpcError::Io {
                message: e.to_string(),
            })?;
        if n == 0 {
            break;
        }
        if buf.len() + n > MAX_RESPONSE_BYTES {
            return Err(CompanionRpcError::Malformed {
                message: format!("response exceeded {MAX_RESPONSE_BYTES} bytes"),
            });
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

/// Parse a raw HTTP/1.1 response into `(status_code, body_bytes)`. Tolerates any
/// header set; only the status line and the blank-line body delimiter matter.
fn split_http_response(raw: &[u8]) -> CompanionResult<(u16, &[u8])> {
    let header_end =
        find_subsequence(raw, b"\r\n\r\n").ok_or_else(|| CompanionRpcError::Malformed {
            message: "no header/body delimiter in HTTP response".to_string(),
        })?;
    let header_block = &raw[..header_end];
    let body = &raw[header_end + 4..];

    let status_line_end = find_subsequence(header_block, b"\r\n").unwrap_or(header_block.len());
    let status_line = std::str::from_utf8(&header_block[..status_line_end]).map_err(|_| {
        CompanionRpcError::Malformed {
            message: "non-utf8 status line".to_string(),
        }
    })?;

    // "HTTP/1.1 200 OK" -> the second whitespace token is the status code.
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| CompanionRpcError::Malformed {
            message: format!("unparseable status line: {status_line}"),
        })?;

    Ok((status, body))
}

/// Find the first index of `needle` in `haystack`.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Validate the envelope (version, id, ok/error consistency) and decode the
/// typed result. This is where `version_mismatch`/`unauthorized`/per-method
/// codes are classified.
fn decode_result<R: DeserializeOwned>(
    envelope: RpcEnvelope,
    request_id: u64,
) -> CompanionResult<R> {
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(CompanionRpcError::VersionMismatch {
            server_version: envelope.protocol_version,
        });
    }
    if envelope.id != request_id {
        return Err(CompanionRpcError::Protocol {
            message: format!(
                "response id {} did not match request id {request_id}",
                envelope.id
            ),
        });
    }

    if envelope.ok {
        let value = envelope.result.ok_or_else(|| CompanionRpcError::Protocol {
            message: "ok response missing `result`".to_string(),
        })?;
        return serde_json::from_value::<R>(value).map_err(|e| CompanionRpcError::Malformed {
            message: format!("result did not match expected shape: {e}"),
        });
    }

    let error = envelope.error.unwrap_or(RpcError {
        code: "unknown".to_string(),
        message: "error response missing `error`".to_string(),
    });
    Err(classify_error(error))
}

/// Map a well-formed `error` object to the right [`CompanionRpcError`] variant.
/// `version_mismatch` and `unauthorized` are promoted to their transport-style
/// variants (fallback). The dispatch-level codes `unknown_method` and `internal`
/// mean the request never reached a working method handler, so they become
/// [`CompanionRpcError::Protocol`] (fallback) rather than a per-method
/// application error. `bad_request` is deliberately NOT treated as fallback:
/// the companion overloads it across two tiers — dispatch-level param/envelope
/// validation AND genuine per-method application errors (e.g. `open_intent`
/// rejecting an unparseable intent URI). Since the wire cannot distinguish the
/// two, classifying all `bad_request` as fallback would tear down the whole
/// companion session over one benign per-method error, so it falls through to a
/// non-fallback [`CompanionRpcError::Method`] (the immediate action still falls
/// back to ADB on its own). Everything else is a per-method application error.
fn classify_error(error: RpcError) -> CompanionRpcError {
    match error.code.as_str() {
        error_codes::VERSION_MISMATCH => CompanionRpcError::VersionMismatch {
            // The server reported a mismatch but did not give us its number in
            // the envelope; record 0 as "unknown server version".
            server_version: 0,
        },
        error_codes::UNAUTHORIZED => CompanionRpcError::Unauthorized {
            message: error.message,
        },
        error_codes::UNKNOWN_METHOD | error_codes::INTERNAL => CompanionRpcError::Protocol {
            message: format!(
                "companion dispatch error [{}]: {}",
                error.code, error.message
            ),
        },
        _ => CompanionRpcError::Method {
            code: error.code,
            message: error.message,
        },
    }
}
