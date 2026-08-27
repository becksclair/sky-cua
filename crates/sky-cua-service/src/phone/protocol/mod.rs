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

pub mod client;
pub mod dto;
pub mod identity;

// Re-exports to preserve `crate::phone::protocol::HealthResult` etc.
#[allow(unused_imports)]
pub(crate) use self::dto::{
    AccessibilityNodeDto, AccessibilityTreeParams, AccessibilityTreeResult, AppListEntryDto,
    AppListParams, AppListResult, AppOp, AppOpParams, AppOpResult, CapabilitiesResult,
    CurrentAppResult, CursorOverlayParams, CursorOverlayResult, GestureKind, GestureParams,
    GesturePoint, GestureResult, GlobalActionParams, GlobalActionResult, HealthResult,
    KeyEventParams, KeyEventResult, NodeActionParams, NodeActionResult, NotificationActionDto,
    NotificationEventDto, NotificationOp, NotificationOpParams, NotificationOpResult,
    NotificationRedactionDto, NotificationsParams, NotificationsResult, OverlayActiveParams,
    OverlayActiveResult, OverlayGestureParams, OverlayGestureResult, ScreenshotParams,
    ScreenshotResult,
};

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
