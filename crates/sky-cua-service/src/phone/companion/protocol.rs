//! Companion RPC wire DTOs (protocol_version = 1).
//!
//! These types are the serde mirror of the JSON contract documented in
//! `docs/runtime/phone-companion-protocol.md`. Transport is HTTP/1.1
//! `POST /rpc` to a host-managed `adb forward tcp:PORT tcp:PORT` localhost
//! endpoint inside the companion app: one JSON request -> one JSON response.
//!
//! Request:  `{"protocol_version":1,"token":"<tok>","id":<int>,"method":"<m>","params":{...}}`
//! Response (ok):    `{"protocol_version":1,"ok":true,"id":<int>,"result":{...}}`
//! Response (error): `{"protocol_version":1,"ok":false,"id":<int>,"error":{"code":"<c>","message":"<m>"}}`
//!
//! Every DTO carries `pub(crate)` visibility so the integrator (`manager.rs`)
//! can name the request/result types when it wires the client into routing.
//!
//! Until the integrator wires the client into `manager.rs`, these DTOs are only
//! referenced from the client/tests. The module-level expectation keeps non-test
//! builds clean and matches the spine's `expect(dead_code)` idiom; it becomes
//! unfulfilled (and self-removing) once routing references the types.
#![cfg_attr(not(test), expect(dead_code))]

use serde::{Deserialize, Serialize};

/// The only protocol version this host speaks. Sent on every request; the
/// companion rejects mismatches with [`error_codes::VERSION_MISMATCH`].
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// Wire-level RPC method names. Kept as constants so the client and tests never
/// drift on a string literal.
pub(crate) mod methods {
    pub(crate) const HEALTH: &str = "health";
    pub(crate) const CAPABILITIES: &str = "capabilities";
    pub(crate) const ACCESSIBILITY_TREE: &str = "accessibility_tree";
    pub(crate) const SCREENSHOT: &str = "screenshot";
    pub(crate) const GESTURE: &str = "gesture";
    pub(crate) const CURSOR_OVERLAY: &str = "cursor_overlay";
    pub(crate) const OVERLAY_ACTIVE: &str = "overlay_active";
    pub(crate) const OVERLAY_GESTURE: &str = "overlay_gesture";
    pub(crate) const NOTIFICATIONS: &str = "notifications";
    pub(crate) const NOTIFICATION_OP: &str = "notification_op";
    pub(crate) const CURRENT_APP: &str = "current_app";
    pub(crate) const APP_LIST: &str = "app_list";
    pub(crate) const APP_OP: &str = "app_op";
}

/// Per-method error codes the companion may return in `error.code`, plus the
/// protocol-level codes (`unauthorized`, `version_mismatch`) the server applies
/// before method dispatch. Clients route on these stable codes, never prose.
pub(crate) mod error_codes {
    // Protocol-level (validated before method dispatch). `unauthorized` and
    // `version_mismatch` map to their own host variants. `unknown_method` and
    // `internal` signal the request never reached a working method handler, so
    // the host falls back rather than treating them as per-method application
    // errors. `bad_request` is overloaded (dispatch-level validation AND genuine
    // per-method app errors such as a bad intent URI), indistinguishable on the
    // wire, so the host does NOT fall back on it — it is a non-fallback Method
    // error and the affected action falls back on its own.
    pub(crate) const UNAUTHORIZED: &str = "unauthorized";
    pub(crate) const VERSION_MISMATCH: &str = "version_mismatch";
    pub(crate) const BAD_REQUEST: &str = "bad_request";
    pub(crate) const UNKNOWN_METHOD: &str = "unknown_method";
    pub(crate) const INTERNAL: &str = "internal";

    // screenshot method.
    pub(crate) const SECURE_WINDOW: &str = "secure_window";
    pub(crate) const UNSUPPORTED_API: &str = "unsupported_api";
    pub(crate) const DISABLED_SERVICE: &str = "disabled_service";
    pub(crate) const OEM_POLICY: &str = "oem_policy";
    pub(crate) const THROTTLED: &str = "throttled";
    pub(crate) const TRANSIENT: &str = "transient";

    // notification_op method.
    pub(crate) const GONE: &str = "gone";
    pub(crate) const REDACTED: &str = "redacted";
    pub(crate) const PENDING_INTENT_MISSING: &str = "pending_intent_missing";
    pub(crate) const CANCELED: &str = "canceled";
    pub(crate) const EXPIRED: &str = "expired";
    pub(crate) const IMMUTABLE: &str = "immutable";
    pub(crate) const REPLY_UNAVAILABLE: &str = "reply_unavailable";
    pub(crate) const OEM_FILTERED: &str = "oem_filtered";
}

// ===========================================================================
// Envelopes
// ===========================================================================

/// Outgoing request envelope. `params` is a typed payload that serializes to the
/// `params` object; methods with no params use [`NoParams`].
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct RpcRequest<P> {
    pub(crate) protocol_version: u32,
    pub(crate) token: String,
    pub(crate) id: u64,
    pub(crate) method: String,
    pub(crate) params: P,
}

impl<P> RpcRequest<P> {
    /// Build a v1 request for `method` with `token`, `id`, and typed `params`.
    pub(crate) fn new(token: impl Into<String>, id: u64, method: &str, params: P) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            token: token.into(),
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// Empty params payload (serializes to `{}`). Used by `health`, `capabilities`,
/// and `current_app`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NoParams {}

/// The structured `error` object on a non-ok response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RpcError {
    pub(crate) code: String,
    pub(crate) message: String,
}

/// Raw response envelope before result typing. `result` stays as a
/// `serde_json::Value` so one decode pass validates the envelope, then the
/// client decodes `result` into the per-method DTO. This lets malformed-result
/// payloads surface as a typed decode failure rather than an envelope failure.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct RpcEnvelope {
    pub(crate) protocol_version: u32,
    pub(crate) ok: bool,
    pub(crate) id: u64,
    #[serde(default)]
    pub(crate) result: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) error: Option<RpcError>,
}

// ===========================================================================
// health / capabilities
// ===========================================================================

/// `health` result. `capabilities` returns a superset (see [`CapabilitiesResult`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HealthResult {
    pub(crate) version: String,
    pub(crate) version_code: u64,
    pub(crate) package: String,
    pub(crate) accessibility_enabled: bool,
    pub(crate) can_perform_gestures: bool,
    pub(crate) can_retrieve_window_content: bool,
    pub(crate) can_take_screenshot: bool,
    pub(crate) notification_listener_enabled: bool,
    pub(crate) native_overlay: bool,
    pub(crate) native_overlay_pass_through: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) privileged_setup: Option<String>,
}

/// `capabilities` result: the health fields plus screenshot/gesture support
/// detail. Flattening `health` keeps the wire object a single flat map while the
/// Rust type composes the shared fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CapabilitiesResult {
    #[serde(flatten)]
    pub(crate) health: HealthResult,
    pub(crate) screenshot_api_level: u32,
    pub(crate) screenshot_supported: bool,
    pub(crate) gesture_supported: bool,
}

// ===========================================================================
// accessibility_tree
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AccessibilityTreeParams {
    pub(crate) max_nodes: u32,
}

/// One node in the companion's flat accessibility list. `bounds` is the raw
/// `[left, top, right, bottom]` device-pixel rect the companion reports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AccessibilityNodeDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) content_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) bounds: Option<[i32; 4]>,
    pub(crate) focusable: bool,
    pub(crate) enabled: bool,
    pub(crate) clickable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AccessibilityTreeResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) activity: Option<String>,
    #[serde(default)]
    pub(crate) nodes: Vec<AccessibilityNodeDto>,
    pub(crate) truncated: bool,
    pub(crate) redacted: bool,
}

// ===========================================================================
// screenshot
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ScreenshotParams {
    pub(crate) include_overlay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ScreenshotResult {
    pub(crate) mime_type: String,
    pub(crate) data_base64: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) contains_native_overlay: bool,
}

// ===========================================================================
// gesture
// ===========================================================================

/// Gesture kind. `tap` uses one point; `swipe` uses two (start, end).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GestureKind {
    Tap,
    Swipe,
}

/// A device-pixel point in a gesture path.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub(crate) struct GesturePoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct GestureParams {
    pub(crate) kind: GestureKind,
    pub(crate) points: Vec<GesturePoint>,
    pub(crate) duration_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GestureResult {
    pub(crate) dispatched: bool,
}

// ===========================================================================
// cursor_overlay
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct CursorOverlayParams {
    pub(crate) visible: bool,
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CursorOverlayResult {
    pub(crate) shown: bool,
    pub(crate) pass_through: bool,
}

// ===========================================================================
// overlay_active
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OverlayActiveParams {
    pub(crate) active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OverlayActiveResult {
    pub(crate) active: bool,
    pub(crate) glow_supported: bool,
}

// ===========================================================================
// overlay_gesture
// ===========================================================================

/// Reuses [`GesturePoint`] for the device-pixel path. `kind` is the free-form
/// wire string (`tap`/`swipe`/`drag`) rather than [`GestureKind`], because the
/// visual overlay supports `drag`, which the real-input `gesture` method does
/// not.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct OverlayGestureParams {
    pub(crate) kind: String,
    pub(crate) points: Vec<GesturePoint>,
    pub(crate) duration_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OverlayGestureResult {
    pub(crate) animated: bool,
}

// ===========================================================================
// notifications / notification_op
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationsParams {
    pub(crate) max: u32,
}

/// Redaction state of a companion notification event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationRedactionDto {
    None,
    Partial,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationActionDto {
    pub(crate) action_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    pub(crate) is_reply: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationEventDto {
    pub(crate) event_id: String,
    pub(crate) package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) body: Option<String>,
    pub(crate) redaction: NotificationRedactionDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ranking: Option<u32>,
    /// Whether the notification carries a content-intent the agent can open.
    /// Absent on older companions; the host defaults it conservatively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) can_open: Option<bool>,
    /// Whether the notification is user-dismissable (`StatusBarNotification.isClearable`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) can_dismiss: Option<bool>,
    /// Whether the notification is an ongoing/non-clearable event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ongoing: Option<bool>,
    pub(crate) when_ms: u64,
    #[serde(default)]
    pub(crate) actions: Vec<NotificationActionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationsResult {
    pub(crate) listener_enabled: bool,
    #[serde(default)]
    pub(crate) events: Vec<NotificationEventDto>,
    pub(crate) truncated: bool,
}

/// `notification_op` operation kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationOp {
    Open,
    Dismiss,
    Action,
    Reply,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationOpParams {
    pub(crate) event_id: String,
    pub(crate) op: NotificationOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reply_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationOpResult {
    pub(crate) ok: bool,
}

// ===========================================================================
// current_app / app_list / app_op
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CurrentAppResult {
    pub(crate) package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppListParams {
    pub(crate) launchable_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppListEntryDto {
    pub(crate) package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    pub(crate) launchable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppListResult {
    #[serde(default)]
    pub(crate) apps: Vec<AppListEntryDto>,
    pub(crate) truncated: bool,
}

/// `app_op` operation kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppOp {
    Launch,
    OpenIntent,
    ForceStop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppOpParams {
    pub(crate) op: AppOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) intent_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppOpResult {
    pub(crate) ok: bool,
}
