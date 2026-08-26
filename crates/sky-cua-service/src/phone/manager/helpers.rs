#![allow(clippy::empty_line_after_doc_comments)]
//! Small pure helpers: IDs, diagnostics, and direct-health mapping.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::phone::direct::DirectRuntimeError;
use sky_cua_platform::config::ResolvedPhoneSelection;
use sky_cua_platform::model::{
    DiagnosticEntry, PHONE_SMS_QUERY_SCHEMA, PhoneBackendKind, PhoneCompanionCapabilities,
    PhoneConnectRequest, PhoneRequest, PhoneSessionSelector, PhoneSmsQueryError,
    PhoneSmsQueryResponse,
};
/// Whether a `phone_connect` request asks for a managed scrcpy mirror: an
/// explicit `start_scrcpy` flag, or a request that selects the scrcpy backend.
pub(crate) fn request_wants_scrcpy(request: &PhoneConnectRequest) -> bool {
    request.start_scrcpy || request.backend == Some(PhoneBackendKind::Scrcpy)
}

pub(crate) fn is_uuid_format(value: &str) -> bool {
    // Strict 8-4-4-4-12 hex, case-insensitive. Avoids the loose
    // `len>=32 && contains('-')` that would mis-route an ADB serial that
    // happens to contain a hyphen.
    if value.len() != 36 {
        return false;
    }
    let bytes = value.as_bytes();
    const HYPHENS: [usize; 4] = [8, 13, 18, 23];
    for (i, b) in bytes.iter().enumerate() {
        if HYPHENS.contains(&i) {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Current wall-clock time in milliseconds since the Unix epoch.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Mint a session id from a serial: a sanitized serial plus the canonical
/// platform snapshot/uuid minter for uniqueness.
pub(crate) fn new_session_id(serial: &str) -> String {
    let sanitized: String = serial
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!(
        "phone-sess-{sanitized}-{}",
        sky_cua_platform::snapshot::new_snapshot_id()
    )
}

/// The default resolved selection used when config loading fails at startup.
pub(crate) fn default_selection() -> ResolvedPhoneSelection {
    sky_cua_platform::config::resolve_phone_selection(
        &sky_cua_platform::config::PhoneConfig::default(),
    )
}

/// Pull `(session_id, serial)` strings out of a selector, substituting empty
/// strings when the caller named neither.
pub(crate) fn selector_ids(selector: &PhoneSessionSelector) -> (String, String) {
    (
        selector.session_id.clone().unwrap_or_default(),
        selector
            .serial
            .clone()
            .or_else(|| selector.device_id.clone())
            .unwrap_or_default(),
    )
}

/// Structured diagnostic for a tool that requires an active session when none
/// resolves from the selector.
pub(crate) fn no_session_diagnostic(selector: &PhoneSessionSelector) -> DiagnosticEntry {
    let (session_id, serial) = selector_ids(selector);
    DiagnosticEntry {
        code: "PhoneNoSession".to_string(),
        message: format!(
            "no active phone session for selector (session_id={session_id:?}, serial={serial:?}); call phone_connect first"
        ),
        details: None,
    }
}

pub(crate) fn phone_disabled_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneUseDisabled".to_string(),
        message: "phone-use is disabled by configuration; enable [phone].enabled before using device-control tools".to_string(),
        details: None,
    }
}

/// Structured diagnostic when a companion action is attempted with no live
/// companion runtime for the session.
pub(crate) fn no_companion_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: super::protocol::error_codes::DISABLED_SERVICE.to_string(),
        message: "no reachable companion for this session".to_string(),
        details: None,
    }
}

pub(crate) fn companion_from_direct_health(
    health: Option<&serde_json::Value>,
) -> PhoneCompanionCapabilities {
    let string = |key| health?.get(key)?.as_str().map(str::to_owned);
    let boolean = |key| {
        health
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    let mut companion = PhoneCompanionCapabilities::absent(
        string("package")
            .as_deref()
            .unwrap_or("com.skycua.phonecompanion"),
    );
    companion.installed = true;
    companion.rpc_reachable = health.is_some();
    companion.installed_version = string("version");
    companion.accessibility_enabled = boolean("accessibility_enabled");
    companion.can_perform_gestures = boolean("can_perform_gestures");
    companion.can_retrieve_window_content = boolean("can_retrieve_window_content");
    companion.can_take_screenshot = boolean("can_take_screenshot");
    companion.notification_listener_enabled = boolean("notification_listener_enabled");
    companion.native_overlay = boolean("native_overlay");
    companion.native_overlay_pass_through = boolean("native_overlay_pass_through");
    companion.gesture_dispatch = companion.can_perform_gestures;
    companion.screenshot = companion.can_take_screenshot;
    companion.accessibility_tree = companion.can_retrieve_window_content;
    companion.notifications = companion.notification_listener_enabled;
    companion
}

pub(crate) fn schema_sms_failure(
    profile: &str,
    code: &str,
    message: String,
) -> PhoneSmsQueryResponse {
    PhoneSmsQueryResponse {
        schema: PHONE_SMS_QUERY_SCHEMA.to_owned(),
        profile: profile.to_owned(),
        device_id: None,
        transport: None,
        access: None,
        messages: Vec::new(),
        next_cursor: None,
        scan: None,
        error: Some(PhoneSmsQueryError {
            code: code.to_owned(),
            message,
        }),
    }
}

pub(crate) fn sms_page_contract_valid(response: &PhoneSmsQueryResponse, limit: u32) -> bool {
    let Some(scan) = response.scan.as_ref() else {
        return false;
    };
    !scan.snapshot
        && scan.has_more != scan.exhausted_as_observed
        && scan.has_more == response.next_cursor.is_some()
        && response.messages.len() <= limit as usize
        && response
            .next_cursor
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty())
}

pub(crate) fn sms_direct_error_code(error: &DirectRuntimeError) -> &'static str {
    match error {
        DirectRuntimeError::NotConnected
        | DirectRuntimeError::LinkEpochChanged { .. }
        | DirectRuntimeError::Disconnected => "DEVICE_OFFLINE",
        DirectRuntimeError::DeadlineExceeded => "DEADLINE_EXCEEDED",
        DirectRuntimeError::Protocol(_) => "PROTOCOL_ERROR",
        DirectRuntimeError::Remote { code, .. } => match code.as_str() {
            "SMS_PERMISSION_NOT_GRANTED" => "SMS_PERMISSION_NOT_GRANTED",
            "SMS_PERMISSION_RESTRICTED" => "SMS_PERMISSION_RESTRICTED",
            "SMS_PROVIDER_UNAVAILABLE" => "SMS_PROVIDER_UNAVAILABLE",
            "INVALID_ARGUMENT" => "INVALID_ARGUMENT",
            "INVALID_CURSOR" => "INVALID_CURSOR",
            "CURSOR_QUERY_MISMATCH" => "CURSOR_QUERY_MISMATCH",
            "SMS_QUERY_FAILED" => "SMS_QUERY_FAILED",
            "DEADLINE_EXCEEDED" => "DEADLINE_EXCEEDED",
            "PROTOCOL_ERROR" => "PROTOCOL_ERROR",
            _ => "SMS_QUERY_FAILED",
        },
    }
}

pub(crate) fn sms_direct_error_message(error: DirectRuntimeError) -> String {
    match error {
        DirectRuntimeError::Remote { message, .. } => message,
        other => format!("CompanionDirect SMS query failed: {other:?}"),
    }
}

pub(crate) fn phone_request_activity_selector(
    request: &PhoneRequest,
) -> Option<&PhoneSessionSelector> {
    match request {
        PhoneRequest::SmsQuery(_)
        | PhoneRequest::Status(_)
        | PhoneRequest::ListDevices(_)
        | PhoneRequest::PairWireless(_)
        | PhoneRequest::Connect(_)
        | PhoneRequest::Disconnect(_) => None,
        PhoneRequest::Observe(request) => Some(&request.session),
        PhoneRequest::RefreshCapabilities(request) => Some(&request.session),
        PhoneRequest::Screenshot(request) => Some(&request.session),
        PhoneRequest::Tap(request) => Some(&request.session),
        PhoneRequest::Swipe(request) => Some(&request.session),
        PhoneRequest::TypeText(request) => Some(&request.session),
        PhoneRequest::PressKey(request) => Some(&request.session),
        PhoneRequest::InstallCompanion(request) => Some(&request.session),
        PhoneRequest::CompanionStatus(request) => Some(&request.session),
        PhoneRequest::AccessibilityTree(request) => Some(&request.session),
        PhoneRequest::Notifications(request) => Some(&request.session),
        PhoneRequest::NotificationOpen(request) => Some(&request.session),
        PhoneRequest::NotificationDismiss(request) => Some(&request.session),
        PhoneRequest::NotificationAction(request) => Some(&request.session),
        PhoneRequest::NotificationReply(request) => Some(&request.session),
        PhoneRequest::AppCurrent(request) => Some(&request.session),
        PhoneRequest::AppList(request) => Some(&request.session),
        PhoneRequest::AppLaunch(request) => Some(&request.session),
        PhoneRequest::AppOpenIntent(request) => Some(&request.session),
        PhoneRequest::AppForceStop(request) => Some(&request.session),
        PhoneRequest::AppInstall(request) => Some(&request.session),
        PhoneRequest::OpenSettings(request) => Some(&request.session),
        PhoneRequest::Content(call) => Some(&call.session),
        PhoneRequest::Clipboard(call) => Some(&call.session),
        PhoneRequest::Editor(call) => Some(&call.session),
        PhoneRequest::Camera(call) => Some(&call.session),
        PhoneRequest::Storage(call) => Some(&call.session),
    }
}

/// Which mutations require a fresh `appshot_id` pre-check. `AppLaunch`/
/// `AppOpenIntent`/`OpenSettings` are intentionally *not* fenced pre-mutation —
/// they carry post-mutation `attach_destination_appshot` (see `mod.rs:342`) that
/// snapshots the *destination* after the launch, so a stale pre-mutation
/// `AppShotRequired` would be wasteful. All other non-idempotent ops go through
/// the `AppShotRequired` fence when `direct` is active.
pub(crate) fn mutation_selector(request: &PhoneRequest) -> Option<&PhoneSessionSelector> {
    match request {
        PhoneRequest::AppLaunch(_)
        | PhoneRequest::AppOpenIntent(_)
        | PhoneRequest::OpenSettings(_) => None,
        _ if request.is_idempotent() => None,
        _ => phone_request_activity_selector(request),
    }
}

/// Extend `ResolvedPhoneSelection` with the backend-kind parse the manager needs.
pub(crate) trait DefaultBackendKind {
    fn default_backend_kind(&self) -> PhoneBackendKind;
}

impl DefaultBackendKind for ResolvedPhoneSelection {
    fn default_backend_kind(&self) -> PhoneBackendKind {
        match self.default_backend.as_deref().map(str::trim) {
            Some("adb") => PhoneBackendKind::Adb,
            Some("companion") => PhoneBackendKind::Companion,
            Some("scrcpy") => PhoneBackendKind::Scrcpy,
            Some("auto") => PhoneBackendKind::Auto,
            // Unset/unknown: no backend is selected by default. Status reports
            // `None` until a session picks a concrete backend.
            _ => PhoneBackendKind::None,
        }
    }
}

/// Whether a CompanionDirect dispatch failure should stale the companion
/// capability and clear the overlay. Only transport/epoch loss
/// (`NotConnected`/`Disconnected`/`LinkEpochChanged`) stales; `DeadlineExceeded`
/// and per-method `Remote`/`Protocol` errors are swallowed and leave the
/// overlay state as-is.
pub(crate) fn is_direct_disconnected(error: &DirectRuntimeError) -> bool {
    matches!(
        error,
        DirectRuntimeError::NotConnected
            | DirectRuntimeError::Disconnected
            | DirectRuntimeError::LinkEpochChanged { .. }
    )
}
