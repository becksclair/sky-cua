//! Companion wire DTOs (protocol_version = 1) — retained for direct-only.
//!
//! These types are the serde mirror of the JSON contract documented in
//! `docs/runtime/phone-companion-protocol.md` (legacy `POST /rpc` via
//! `adb forward`) and now also for `phone-control.v2` `ws://` direct
//! (`docs/runtime/direct-lan.md`). Direct re-uses the same DTOs over a
//! persistent WebSocket (`ws://` private LAN/tether or `wss://` tailnet):
//! one JSON request -> one JSON response, envelope `{"protocol_version":1,...}`.
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
#![allow(dead_code)]

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
    pub(crate) const NODE_ACTION: &str = "node_action";
    pub(crate) const GLOBAL_ACTION: &str = "global_action";
    pub(crate) const KEY_EVENT: &str = "key_event";
    pub(crate) const LONG_PRESS: &str = "long_press";
    pub(crate) const DOUBLE_TAP: &str = "double_tap";
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

    // node_action / global_action / key_event
    pub(crate) const UNSUPPORTED_ACTION: &str = "unsupported_action";
    pub(crate) const NODE_NOT_FOUND: &str = "node_not_found";
    pub(crate) const ACTION_FAILED: &str = "action_failed";
    pub(crate) const PERMISSION_DENIED: &str = "permission_denied";
    pub(crate) const APPSHOT_EXPIRED: &str = "appshot_expired";
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
// node_action / global_action / key_event
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct NodeActionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) appshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) view_id: Option<String>,
    pub(crate) action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) args: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NodeActionResult {
    pub(crate) dispatched: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) success: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GlobalActionParams {
    pub(crate) action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GlobalActionResult {
    pub(crate) dispatched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct KeyEventParams {
    pub(crate) key_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) meta_state: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) repeat_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct KeyEventResult {
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

// ---------------------------------------------------------------------------
// Helpers retained from the legacy companion spine for direct-only routing.
// ---------------------------------------------------------------------------

use sky_cua_platform::model::{DiagnosticEntry, PhoneCompanionCapabilities};

pub(crate) fn not_implemented_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneCompanionNotImplemented".to_string(),
        message: "phone companion RPC transport removed; use CompanionDirect".to_string(),
        details: None,
    }
}

pub(crate) fn absent_companion(package_name: &str) -> PhoneCompanionCapabilities {
    PhoneCompanionCapabilities::absent(package_name)
}

/// Direct never sets `rpc_token_expires_at_ms` (HMAC+epoch, not ephemeral
/// `SetupActivity` token). The `_token` param is retained for API parity with
/// the legacy RPC path but is always `None` for direct.
pub(crate) fn capabilities_from_response(
    caps: &CapabilitiesResult,
    _token: Option<&identity::CompanionToken>,
    installed_cert_sha256: Option<&str>,
    expected_cert_sha256: Option<&str>,
    apk_sha256: Option<&str>,
    auto_install_attempted: bool,
    allow_downgrade: bool,
) -> PhoneCompanionCapabilities {
    let health = &caps.health;
    let signature_matches_expected = match (installed_cert_sha256, expected_cert_sha256) {
        (Some(installed), Some(expected)) => identity::certs_match(installed, expected),
        _ => false,
    };
    PhoneCompanionCapabilities {
        installed: true,
        package_name: health.package.clone(),
        installed_version: Some(health.version.clone()),
        expected_version: None,
        installed_cert_sha256: installed_cert_sha256.map(str::to_string),
        expected_cert_sha256: expected_cert_sha256.map(str::to_string),
        apk_sha256: apk_sha256.map(str::to_string),
        signature_matches_expected,
        allow_downgrade,
        auto_install_attempted,
        rpc_reachable: true,
        rpc_token_expires_at_ms: None,
        accessibility_enabled: health.accessibility_enabled,
        can_perform_gestures: health.can_perform_gestures,
        can_retrieve_window_content: health.can_retrieve_window_content,
        can_take_screenshot: health.can_take_screenshot,
        notification_listener_enabled: health.notification_listener_enabled,
        native_overlay: health.native_overlay,
        native_overlay_pass_through: health.native_overlay_pass_through,
        gesture_dispatch: health.accessibility_enabled
            && health.can_perform_gestures
            && caps.gesture_supported,
        screenshot: health.can_take_screenshot && caps.screenshot_supported,
        accessibility_tree: health.accessibility_enabled && health.can_retrieve_window_content,
        notifications: health.notification_listener_enabled,
        privileged_setup: health.privileged_setup.clone(),
    }
}

pub(crate) fn capabilities_from_health(
    health: &HealthResult,
    installed_version: Option<String>,
) -> PhoneCompanionCapabilities {
    PhoneCompanionCapabilities {
        installed: true,
        package_name: health.package.clone(),
        installed_version: installed_version.or_else(|| Some(health.version.clone())),
        expected_version: None,
        installed_cert_sha256: None,
        expected_cert_sha256: None,
        apk_sha256: None,
        signature_matches_expected: true,
        allow_downgrade: false,
        auto_install_attempted: false,
        rpc_reachable: true,
        rpc_token_expires_at_ms: None,
        accessibility_enabled: health.accessibility_enabled,
        can_perform_gestures: health.can_perform_gestures,
        can_retrieve_window_content: health.can_retrieve_window_content,
        can_take_screenshot: health.can_take_screenshot,
        notification_listener_enabled: health.notification_listener_enabled,
        native_overlay: health.native_overlay,
        native_overlay_pass_through: health.native_overlay_pass_through,
        gesture_dispatch: health.accessibility_enabled && health.can_perform_gestures,
        screenshot: health.can_take_screenshot,
        accessibility_tree: health.accessibility_enabled && health.can_retrieve_window_content,
        notifications: health.notification_listener_enabled,
        privileged_setup: health.privileged_setup.clone(),
    }
}

pub(crate) fn capabilities_unreachable(
    package_name: &str,
    installed: &identity::InstalledCompanion,
    expected_cert_sha256: Option<&str>,
    apk_sha256: Option<&str>,
    signature_matches_expected: bool,
) -> PhoneCompanionCapabilities {
    PhoneCompanionCapabilities {
        installed: true,
        package_name: package_name.to_string(),
        installed_version: installed.version_name.clone(),
        expected_version: None,
        installed_cert_sha256: installed.cert_sha256.clone(),
        expected_cert_sha256: expected_cert_sha256.map(str::to_string),
        apk_sha256: apk_sha256.map(str::to_string),
        signature_matches_expected,
        allow_downgrade: false,
        auto_install_attempted: false,
        rpc_reachable: false,
        rpc_token_expires_at_ms: None,
        accessibility_enabled: false,
        can_perform_gestures: false,
        can_retrieve_window_content: false,
        can_take_screenshot: false,
        notification_listener_enabled: false,
        native_overlay: false,
        native_overlay_pass_through: false,
        gesture_dispatch: false,
        screenshot: false,
        accessibility_tree: false,
        notifications: false,
        privileged_setup: None,
    }
}

pub mod client {
    use super::{
        AccessibilityTreeResult, AppOpResult, CurrentAppResult, GestureResult, NotificationsResult,
        ScreenshotResult,
    };

    #[derive(Debug, Clone)]
    pub struct CompanionError(pub String);
    impl CompanionError {
        pub fn is_fallback(&self) -> bool {
            true
        }
        pub fn code(&self) -> &str {
            &self.0
        }
        pub fn message(&self) -> &str {
            &self.0
        }
    }
    impl std::fmt::Display for CompanionError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for CompanionError {}

    #[derive(Debug)]
    pub struct CompanionClient;

    impl CompanionClient {
        pub fn new(_port: u16, _token: impl Into<String>) -> Self {
            Self
        }
        pub fn new_with_addr(_addr: std::net::SocketAddr, _token: impl Into<String>) -> Self {
            Self
        }
        pub async fn health(&mut self) -> Result<super::HealthResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn capabilities(&mut self) -> Result<super::CapabilitiesResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn accessibility_tree(
            &mut self,
            _max: u32,
        ) -> Result<AccessibilityTreeResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn screenshot(
            &mut self,
            _include: bool,
        ) -> Result<ScreenshotResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn gesture(
            &mut self,
            _kind: super::GestureKind,
            _points: Vec<super::GesturePoint>,
            _duration_ms: u32,
        ) -> Result<GestureResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn notifications(
            &mut self,
            _max: u32,
        ) -> Result<NotificationsResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn current_app(&mut self) -> Result<CurrentAppResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn app_op(
            &mut self,
            _op: super::AppOp,
            _package: Option<String>,
            _intent_uri: Option<String>,
        ) -> Result<AppOpResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn notification_op(
            &mut self,
            _params: super::NotificationOpParams,
        ) -> Result<super::NotificationOpResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn overlay_active(
            &mut self,
            _active: bool,
        ) -> Result<super::OverlayActiveResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn overlay_gesture(
            &mut self,
            _kind: &str,
            _points: Vec<super::GesturePoint>,
            _duration_ms: u32,
        ) -> Result<super::OverlayGestureResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
        pub async fn cursor_overlay(
            &mut self,
            _visible: bool,
            _x: f64,
            _y: f64,
        ) -> Result<super::CursorOverlayResult, CompanionError> {
            Err(CompanionError("companion RPC removed".to_string()))
        }
    }
}

pub mod identity {
    #![cfg_attr(not(test), expect(dead_code))]

    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub(crate) struct InstalledCompanion {
        pub(crate) version_name: Option<String>,
        pub(crate) version_code: Option<u64>,
        pub(crate) cert_sha256: Option<String>,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub(crate) struct ExpectedCompanion {
        pub(crate) package_name: String,
        pub(crate) version_name: Option<String>,
        pub(crate) version_code: Option<u64>,
        pub(crate) cert_sha256: Option<String>,
        pub(crate) apk_sha256: Option<String>,
        pub(crate) apk_path: String,
        pub(crate) allow_downgrade: bool,
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub(crate) struct CompanionBootstrapOptions {
        pub(crate) allow_install: bool,
        pub(crate) force_reinstall: bool,
        pub(crate) allow_downgrade: Option<bool>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum CompanionInstallDecision {
        Install,
        Update {
            reason: String,
        },
        UpToDate,
        RefuseSignatureMismatch {
            installed_cert: String,
            expected_cert: String,
        },
        RefuseDowngrade {
            installed_version_code: u64,
            expected_version_code: u64,
        },
    }

    impl CompanionInstallDecision {
        pub(crate) fn requires_install(&self) -> bool {
            matches!(
                self,
                CompanionInstallDecision::Install | CompanionInstallDecision::Update { .. }
            )
        }
        pub(crate) fn code(&self) -> &'static str {
            match self {
                CompanionInstallDecision::Install => "CompanionInstall",
                CompanionInstallDecision::Update { .. } => "CompanionUpdate",
                CompanionInstallDecision::UpToDate => "CompanionUpToDate",
                CompanionInstallDecision::RefuseSignatureMismatch { .. } => {
                    "CompanionSignatureMismatch"
                }
                CompanionInstallDecision::RefuseDowngrade { .. } => "CompanionDowngradeBlocked",
            }
        }
    }

    pub(crate) fn decide_install(
        installed: Option<&InstalledCompanion>,
        expected: &ExpectedCompanion,
        _allow_downgrade_override: Option<bool>,
    ) -> CompanionInstallDecision {
        let Some(installed) = installed else {
            return CompanionInstallDecision::Install;
        };
        match (
            installed.cert_sha256.as_ref(),
            expected.cert_sha256.as_ref(),
        ) {
            (Some(installed_cert), Some(expected_cert))
                if !cert_eq(installed_cert, expected_cert) =>
            {
                return CompanionInstallDecision::RefuseSignatureMismatch {
                    installed_cert: installed_cert.clone(),
                    expected_cert: expected_cert.clone(),
                };
            }
            _ => {}
        }
        let allow_downgrade = _allow_downgrade_override.unwrap_or(expected.allow_downgrade);
        if let (Some(installed_code), Some(expected_code)) =
            (installed.version_code, expected.version_code)
        {
            if installed_code > expected_code && !allow_downgrade {
                return CompanionInstallDecision::RefuseDowngrade {
                    installed_version_code: installed_code,
                    expected_version_code: expected_code,
                };
            }
            if installed_code < expected_code {
                return CompanionInstallDecision::Update {
                    reason: format!(
                        "installed version_code {installed_code} < expected {expected_code}"
                    ),
                };
            }
        }
        CompanionInstallDecision::UpToDate
    }

    fn cert_eq(left: &str, right: &str) -> bool {
        let normalize = |s: &str| {
            s.chars()
                .filter(|c| *c != ':')
                .flat_map(|c| c.to_lowercase())
                .collect::<String>()
        };
        normalize(left) == normalize(right)
    }

    pub(crate) fn certs_match(left: &str, right: &str) -> bool {
        cert_eq(left, right)
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub(crate) struct CompanionMetadata {
        pub(crate) version_name: Option<String>,
        pub(crate) version_code: Option<u64>,
        pub(crate) cert_sha256: Option<String>,
        pub(crate) apk_sha256: Option<String>,
    }

    pub(crate) fn metadata_path_for_apk(apk_path: &str) -> PathBuf {
        Path::new(apk_path).with_extension("json")
    }

    pub(crate) fn load_companion_metadata(apk_path: &str) -> CompanionMetadata {
        match std::fs::read_to_string(metadata_path_for_apk(apk_path)) {
            Ok(text) => parse_companion_metadata(&text),
            Err(_) => CompanionMetadata::default(),
        }
    }

    fn parse_companion_metadata(text: &str) -> CompanionMetadata {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return CompanionMetadata::default();
        };
        let lower_hex = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(|raw| raw.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
        };
        CompanionMetadata {
            version_name: value
                .get("version_name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            version_code: value
                .get("version_code")
                .and_then(serde_json::Value::as_u64),
            cert_sha256: lower_hex("signing_cert_sha256"),
            apk_sha256: lower_hex("apk_sha256"),
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct CompanionToken {
        pub(crate) token: String,
        pub(crate) expires_at_ms: u64,
    }

    impl CompanionToken {
        pub(crate) fn is_expired(&self, now_ms: u64) -> bool {
            now_ms >= self.expires_at_ms
        }
    }

    pub(crate) fn generate_token(now_ms: u64, ttl_ms: u64) -> CompanionToken {
        let bytes = urandom_bytes().unwrap_or_else(|| fallback_token_bytes(now_ms, ttl_ms));
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        CompanionToken {
            token,
            expires_at_ms: now_ms.saturating_add(ttl_ms),
        }
    }

    fn urandom_bytes() -> Option<[u8; 32]> {
        use std::io::Read as _;
        let mut file = std::fs::File::open("/dev/urandom").ok()?;
        let mut bytes = [0u8; 32];
        file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    }

    fn fallback_token_bytes(now_ms: u64, ttl_ms: u64) -> [u8; 32] {
        use std::hash::{BuildHasher, Hash, Hasher};
        let mut bytes = [0u8; 32];
        let pid = u64::from(std::process::id());
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(now_ms);
        for (lane, chunk) in bytes.chunks_mut(8).enumerate() {
            let state = std::collections::hash_map::RandomState::new();
            let mut hasher = state.build_hasher();
            (lane as u64).hash(&mut hasher);
            pid.hash(&mut hasher);
            nanos.hash(&mut hasher);
            now_ms.hash(&mut hasher);
            ttl_ms.hash(&mut hasher);
            (&lane as *const usize as u64).hash(&mut hasher);
            let value = hasher.finish().to_le_bytes();
            chunk.copy_from_slice(&value[..chunk.len()]);
        }
        bytes
    }

    pub(crate) const SETUP_TOKEN_EXPIRES_EXTRA: &str = "sky_cua_rpc_token_expires_at_ms";
    pub(crate) const SETUP_TOKEN_EXTRA: &str = "sky_cua_rpc_token";

    pub(crate) fn setup_intent_argv(
        serial: &str,
        package_name: &str,
        token: &CompanionToken,
    ) -> Vec<String> {
        let component = format!("{package_name}/.SetupActivity");
        let expires = token.expires_at_ms.to_string();
        vec![
            "-s".to_string(),
            serial.to_string(),
            "shell".to_string(),
            "am".to_string(),
            "start".to_string(),
            "-n".to_string(),
            component,
            "--es".to_string(),
            SETUP_TOKEN_EXTRA.to_string(),
            token.token.clone(),
            "--el".to_string(),
            SETUP_TOKEN_EXPIRES_EXTRA.to_string(),
            expires,
        ]
    }

    pub(crate) fn install_argv(serial: &str, expected: &ExpectedCompanion) -> Vec<String> {
        let mut argv = vec![
            "-s".to_string(),
            serial.to_string(),
            "install".to_string(),
            "-r".to_string(),
        ];
        if expected.allow_downgrade {
            argv.push("-d".to_string());
        }
        argv.push(expected.apk_path.clone());
        argv
    }
}
