//! Text summaries and `{content, structuredContent, isError}` shaping for the
//! `phone_*` MCP tools.
//!
//! Mirrors `browser/response.rs`: each response variant gets a plain-text
//! summary (secondary to the structured truth) and an is-error rule keyed on
//! structured diagnostics rather than prose. Screenshots are encoded exactly
//! like `browser_screenshot_result`: an image content block only when the model
//! can receive images, and `data_base64` stripped from `structuredContent`.

use std::fmt::Write as _;

use anyhow::Result;
use serde_json::{Value, json};
use sky_cua_platform::model::{
    DiagnosticEntry, PhoneAccessibilityTreeResponse, PhoneActionResponse, PhoneAppResponse,
    PhoneAppResponseKind, PhoneBackendKind, PhoneCompanionStatusResponse, PhoneDisconnectResponse,
    PhoneImage, PhoneListDevicesResponse, PhoneNotificationsResponse, PhoneObserveResponse,
    PhonePairWirelessResponse, PhoneScreenshotResponse, PhoneSession, PhoneStatusReport,
};

use crate::output_shapes::summary_text_field;

/// Diagnostic codes that mark a phone tool result as an MCP error. These are
/// honest "could not do it" states, not informational context, so they flip
/// `isError`. The families covered are, in order:
///
/// - not-implemented stubs and unsupported-platform states;
/// - host/command failures (`adb` spawn/exit failures, the `PhoneAdb*`/`Phone*`
///   ADB-lane diagnostics);
/// - companion bootstrap/transport failures (`PhoneCompanion*`) and the
///   companion RPC error family the client surfaces verbatim from the service
///   (`Companion*` transport codes plus the lowercase per-method/protocol
///   domain codes such as `unauthorized`/`secure_window`/`gone`);
/// - snapshot-safety rejections (a coordinate action referencing a missing,
///   stale, or mismatched snapshot never dispatched);
/// - coordinate-mapping and synthetic-cursor rejections;
/// - app/notification/foreground operation failures; and
/// - explicit action rejections and no-session states.
///
/// Every code here is genuinely emitted by `sky-cua-service`'s phone lanes; the
/// list is verified against the `code:`/`code()` emission sites under
/// `phone/**` and `phone/companion/`.
fn phone_diagnostic_is_error_code(code: &str) -> bool {
    matches!(
        code,
        "PhoneNotImplemented"
            | "PhoneAdbNotImplemented"
            | "PhoneSnapshotNotImplemented"
            | "PhoneCompanionNotImplemented"
            | "PhoneCommandNotImplemented"
            | "PhoneCommandSpawnFailed"
            | "PhoneCommandTimedOut"
            | "PhoneUnsupportedPlatform"
            | "PhoneUseDisabled"
            | "PhoneSessionNotFound"
            | "PhoneDeviceUnavailable"
            | "PhoneBackendUnavailable"
            | "PhoneSnapshotRequired"
            | "PhoneSnapshotUnknown"
            | "PhoneSnapshotStale"
            | "PhoneSnapshotSessionMismatch"
            | "PhoneSnapshotSerialMismatch"
            | "PhoneSnapshotOrientationMismatch"
            | "PhoneSnapshotResolutionMismatch"
            | "PhoneActionRejected"
            | "PhoneMappingNonFinite"
            | "PhoneMappingOutOfBounds"
            | "PhoneMappingDegenerateRect"
            | "PhoneMappingNoHostSurface"
            | "PhoneMappingUnsupportedRotation"
            // ADB-lane command/operation failures.
            | "PhoneAdbCommandFailed"
            | "PhoneForegroundUnknown"
            | "PhoneInstallNoApk"
            | "PhoneInstallFailed"
            | "PhoneAppActionFailed"
            | "PhonePairFailed"
            | "PhoneConnectFailed"
            | "PhoneScrcpyLaunchFailed"
            | "PhoneNoSession"
            // Companion bootstrap / transport / decode failures.
            | "PhoneCompanionForwardFailed"
            | "PhoneCompanionUnreachable"
            | "PhoneCompanionRequired"
            | "PhoneCompanionScreenshotDecode"
            | "PhoneCompanionInstallFailed"
            | "PhoneCompanionSetupIntentFailed"
            | "CompanionSignatureMismatch"
            | "CompanionSignatureUnverified"
            | "PhoneNotificationOpRejected"
            // Synthetic / mapped cursor rejections.
            | "PhoneCursorSessionMismatch"
            | "PhoneCursorSerialMismatch"
            | "PhoneCursorSyntheticOutOfBounds"
            | "PhoneCursorSyntheticFailed"
            // Screencap decode failure (truncated/non-PNG capture).
            | "PhoneScreencapDecodeFailed"
            // Companion RPC transport/protocol error family.
            | "CompanionConnectRefused"
            | "CompanionTimeout"
            | "CompanionIo"
            | "CompanionHttpStatus"
            | "CompanionMalformedResponse"
            | "CompanionVersionMismatch"
            | "CompanionProtocolViolation"
            // Companion per-method / protocol domain codes (surfaced verbatim).
            | "unauthorized"
            | "version_mismatch"
            | "secure_window"
            | "unsupported_api"
            // NOTE: `disabled_service` is intentionally NOT listed. It can ride
            // along on a successful ADB-fallback screenshot/observe after a
            // companion screenshot attempt fails. Companion-only failures already
            // flip `isError` through `backend == None`, so adding the literal here
            // would mislabel successful perception fallback as an error.
            | "oem_policy"
            | "throttled"
            | "transient"
            | "gone"
            | "redacted"
            | "pending_intent_missing"
            | "canceled"
            | "expired"
            | "immutable"
            | "reply_unavailable"
            | "oem_filtered"
    )
}

fn phone_diagnostics_are_error(diagnostics: &[DiagnosticEntry]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| phone_diagnostic_is_error_code(&diagnostic.code))
}

fn append_first_diagnostic(summary: &mut String, diagnostics: &[DiagnosticEntry]) {
    if let Some(diagnostic) = diagnostics.first() {
        let _ = write!(summary, " Diagnostic: {}", diagnostic.message);
    }
}

// ---------------------------------------------------------------------------
// status / devices
// ---------------------------------------------------------------------------

pub(crate) fn phone_status_summary(report: &PhoneStatusReport) -> String {
    let mut summary = String::from(if report.enabled {
        "Phone Use tools are enabled."
    } else {
        "Phone Use tools are disabled."
    });
    let _ = write!(
        &mut summary,
        " adb={} scrcpy={} companion={} sessions={} devices={}.",
        if report.adb_available {
            "available"
        } else {
            "unavailable"
        },
        if report.scrcpy_available {
            "available"
        } else {
            "unavailable"
        },
        if report.companion_enabled {
            "enabled"
        } else {
            "disabled"
        },
        report.sessions.len(),
        report.devices.len()
    );
    if let Some(serial) = report
        .default_serial
        .as_deref()
        .filter(|serial| !serial.is_empty())
    {
        let _ = write!(&mut summary, " Default serial: {serial}.");
    }
    append_first_diagnostic(&mut summary, &report.diagnostics);
    summary
}

pub(crate) fn phone_status_result(report: PhoneStatusReport) -> Result<Value> {
    let is_error = phone_diagnostics_are_error(&report.diagnostics);
    let text = phone_status_summary(&report);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": report,
        "isError": is_error
    }))
}

pub(crate) fn phone_list_devices_summary(response: &PhoneListDevicesResponse) -> String {
    let mut summary = format!("Discovered {} Android devices.", response.devices.len());
    for device in &response.devices {
        let _ = write!(
            &mut summary,
            " [{}] state={:?} connection={:?}",
            device.serial, device.state, device.connection_kind
        );
        if let Some(model) = device.model.as_deref().filter(|model| !model.is_empty()) {
            let _ = write!(&mut summary, " model={model}");
        }
    }
    append_first_diagnostic(&mut summary, &response.diagnostics);
    summary
}

pub(crate) fn phone_list_devices_result(response: PhoneListDevicesResponse) -> Result<Value> {
    let is_error =
        response.devices.is_empty() && phone_diagnostics_are_error(&response.diagnostics);
    let text = phone_list_devices_summary(&response);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

// ---------------------------------------------------------------------------
// pair / connect / observe / disconnect
// ---------------------------------------------------------------------------

pub(crate) fn phone_pair_wireless_summary(response: &PhonePairWirelessResponse) -> String {
    let mut summary = if response.paired {
        format!("Paired wireless debugging endpoint {}.", response.host_port)
    } else {
        format!(
            "Could not pair wireless debugging endpoint {}.",
            response.host_port
        )
    };
    if let Some(serial) = response
        .serial
        .as_deref()
        .filter(|serial| !serial.is_empty())
    {
        let _ = write!(&mut summary, " Serial: {serial}.");
    }
    append_first_diagnostic(&mut summary, &response.diagnostics);
    summary
}

pub(crate) fn phone_pair_wireless_result(response: PhonePairWirelessResponse) -> Result<Value> {
    let is_error = !response.paired || phone_diagnostics_are_error(&response.diagnostics);
    let text = phone_pair_wireless_summary(&response);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

pub(crate) fn phone_session_summary(session: &PhoneSession) -> String {
    format!(
        "Connected phone session {} (serial {}, connection {:?}, backend {:?}).",
        session.session_id, session.serial, session.connection_kind, session.backend
    )
}

pub(crate) fn phone_connected_result(session: PhoneSession) -> Result<Value> {
    let text = phone_session_summary(&session);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": session,
        "isError": false
    }))
}

pub(crate) fn phone_observe_summary(response: &PhoneObserveResponse) -> String {
    let mut summary = format!(
        "Observed phone session {} (backend {:?}).",
        response.session.session_id, response.backend
    );
    if let Some(app) = response.current_app.as_ref() {
        let _ = write!(&mut summary, " Foreground: {}.", app.package_name);
    }
    if let Some(snapshot_id) = response
        .phone_snapshot_id
        .as_deref()
        .filter(|id| !id.is_empty())
    {
        let _ = write!(&mut summary, " snapshot_id={snapshot_id}.");
    }
    append_first_diagnostic(&mut summary, &response.diagnostics);
    summary
}

pub(crate) fn phone_observe_result(
    response: PhoneObserveResponse,
    can_receive_images: bool,
) -> Result<Value> {
    // Like `phone_screenshot_result`: an observation is an error only when it
    // never reached a backend (`backend == None`). A companion failure that fell
    // back to ADB still produced a usable observation, so its diagnostic is
    // informational and must not flip `isError`.
    let is_error = matches!(response.backend, PhoneBackendKind::None);
    let text = phone_observe_summary(&response);
    image_carrying_result(
        text,
        response.inline_image.clone(),
        &response,
        can_receive_images,
        is_error,
    )
}

pub(crate) fn phone_disconnect_summary(response: &PhoneDisconnectResponse) -> String {
    let mut summary = if response.disconnected {
        format!("Disconnected phone session {}.", response.session_id)
    } else {
        format!(
            "Could not disconnect phone session {}.",
            response.session_id
        )
    };
    append_first_diagnostic(&mut summary, &response.diagnostics);
    summary
}

pub(crate) fn phone_disconnect_result(response: PhoneDisconnectResponse) -> Result<Value> {
    let is_error = !response.disconnected || phone_diagnostics_are_error(&response.diagnostics);
    let text = phone_disconnect_summary(&response);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

// ---------------------------------------------------------------------------
// screenshot
// ---------------------------------------------------------------------------

pub(crate) fn phone_screenshot_summary(response: &PhoneScreenshotResponse) -> String {
    let mut text = format!(
        "Captured phone screenshot for session {} ({}x{} device pixels, snapshot_id={}).",
        response.session_id,
        response.device_size.width,
        response.device_size.height,
        response.phone_snapshot_id
    );
    if let Some(path) = response.screenshot_path.as_deref() {
        let _ = write!(text, " Saved to {path}.");
    }
    text
}

pub(crate) fn phone_screenshot_result(
    response: PhoneScreenshotResponse,
    can_receive_images: bool,
) -> Result<Value> {
    // A screenshot is an error only when it never produced a frame
    // (`backend == None`, e.g. a truncated/decode-failed PNG routed through
    // `screenshot_failure`). When a frame WAS produced it succeeded, even if the
    // preferred companion backend failed first and the request fell back to ADB
    // (`backend == Adb`): the companion-failure diagnostic (e.g. a transient
    // `throttled`) rides along on a *successful* capture and is informational,
    // not an error. This mirrors the `disabled_service` exclusion in
    // `phone_diagnostic_is_error_code`, which already keeps the no-companion
    // fallback marker from mislabeling a good frame.
    let is_error = matches!(response.backend, PhoneBackendKind::None);
    let mut text = if is_error {
        format!(
            "Could not capture phone screenshot for session {}.",
            response.session_id
        )
    } else {
        let mut text = phone_screenshot_summary(&response);
        if can_receive_images && response.inline_image.is_some() {
            text.push_str(" The image is attached to this result.");
        } else if can_receive_images {
            text.push_str(" Image data was omitted; read screenshot_path if needed.");
        } else {
            text.push_str(
                " Image data was omitted because this session's model does not support image input; \
                 use phone_accessibility_tree for structured page details.",
            );
        }
        text
    };
    append_first_diagnostic(&mut text, &response.diagnostics);
    image_carrying_result(
        text,
        response.inline_image.clone(),
        &response,
        can_receive_images,
        is_error,
    )
}

/// Shared helper for the two image-bearing responses (`observe`, `screenshot`).
/// Attaches an MCP image content block only when the model accepts images and an
/// `inline_image` is present, and always strips `inline_image.data_base64` from
/// `structuredContent` so the base64 payload never rides the structured channel.
fn image_carrying_result<T: serde::Serialize>(
    text: String,
    inline_image: Option<PhoneImage>,
    response: &T,
    can_receive_images: bool,
    is_error: bool,
) -> Result<Value> {
    let mut content = vec![json!({"type": "text", "text": text})];
    if !is_error
        && can_receive_images
        && let Some(image) = inline_image
            .as_ref()
            .filter(|image| !image.data_base64.is_empty())
    {
        content.push(json!({
            "type": "image",
            "data": image.data_base64,
            "mimeType": image.mime_type,
        }));
    }

    // The base64 image travels as a content block (or on disk at
    // screenshot_path); repeating it in structuredContent would only bloat the
    // host context window.
    let mut structured = serde_json::to_value(response)?;
    strip_inline_image_base64(&mut structured);

    Ok(json!({
        "content": content,
        "structuredContent": structured,
        "isError": is_error
    }))
}

fn strip_inline_image_base64(structured: &mut Value) {
    if let Some(image) = structured
        .as_object_mut()
        .and_then(|map| map.get_mut("inline_image"))
        .and_then(Value::as_object_mut)
    {
        image.remove("data_base64");
    }
}

// ---------------------------------------------------------------------------
// action / companion / accessibility / notifications / app
// ---------------------------------------------------------------------------

pub(crate) fn phone_action_result(response: PhoneActionResponse) -> Result<Value> {
    // An action that never reached a backend (no companion/scrcpy/ADB dispatch)
    // did not happen — a snapshot-safety rejection, an unavailable backend, or
    // an out-of-bounds coordinate. Treat that as an error even if the specific
    // diagnostic code is not in the allowlist, so the agent never reads a
    // rejected tap/swipe as a success.
    let is_error = matches!(response.backend, PhoneBackendKind::None)
        || phone_diagnostics_are_error(&response.diagnostics);
    let mut text = if is_error {
        format!(
            "Could not perform phone action {} on session {}.",
            response.action, response.session_id
        )
    } else {
        format!(
            "Performed phone action {} on session {}.",
            response.action, response.session_id
        )
    };
    append_first_diagnostic(&mut text, &response.diagnostics);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

pub(crate) fn phone_companion_status_summary(response: &PhoneCompanionStatusResponse) -> String {
    let companion = &response.companion;
    let mut summary = format!(
        "Companion {} on session {}: installed={} accessibility_enabled={} rpc_reachable={}.",
        companion.package_name,
        response.session_id,
        companion.installed,
        companion.accessibility_enabled,
        companion.rpc_reachable
    );
    append_first_diagnostic(&mut summary, &response.diagnostics);
    summary
}

pub(crate) fn phone_companion_status_result(
    response: PhoneCompanionStatusResponse,
) -> Result<Value> {
    let is_error = phone_diagnostics_are_error(&response.diagnostics);
    let text = phone_companion_status_summary(&response);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

pub(crate) fn phone_accessibility_tree_summary(
    response: &PhoneAccessibilityTreeResponse,
) -> String {
    let mut summary = format!(
        "Phone accessibility tree for session {}: {} node{}{}.",
        response.session_id,
        response.nodes.len(),
        if response.nodes.len() == 1 { "" } else { "s" },
        if response.truncated {
            " (truncated)"
        } else {
            ""
        }
    );
    if let Some(package) = response
        .package_name
        .as_deref()
        .filter(|package| !package.is_empty())
    {
        let _ = write!(&mut summary, " Foreground: {package}.");
    }
    if response.redacted {
        summary.push_str(" Some node text was redacted.");
    }
    append_first_diagnostic(&mut summary, &response.diagnostics);
    summary
}

pub(crate) fn phone_accessibility_tree_result(
    response: PhoneAccessibilityTreeResponse,
) -> Result<Value> {
    let is_error = response.nodes.is_empty() && phone_diagnostics_are_error(&response.diagnostics);
    let text = phone_accessibility_tree_summary(&response);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

pub(crate) fn phone_notifications_summary(response: &PhoneNotificationsResponse) -> String {
    let mut summary = format!(
        "Phone notifications for session {}: {} event{}{} (listener {}).",
        response.session_id,
        response.events.len(),
        if response.events.len() == 1 { "" } else { "s" },
        if response.truncated {
            " (truncated)"
        } else {
            ""
        },
        if response.listener_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    for event in response.events.iter().take(8) {
        let title = event
            .title
            .as_deref()
            .map(|value| summary_text_field(value, 80))
            .unwrap_or_default();
        let _ = write!(
            &mut summary,
            " [{}] {}{}",
            event.event_id,
            event.package_name,
            if title.is_empty() {
                String::new()
            } else {
                format!(" \"{title}\"")
            }
        );
    }
    append_first_diagnostic(&mut summary, &response.diagnostics);
    summary
}

pub(crate) fn phone_notifications_result(response: PhoneNotificationsResponse) -> Result<Value> {
    // A notifications result that never reached a backend (`backend == None`,
    // e.g. an unavailable/required-companion response or a notification-op
    // rejection) did not happen; flip `isError` even when the diagnostic code
    // is not in the allowlist, mirroring `phone_action_result`.
    let is_error = matches!(response.backend, PhoneBackendKind::None)
        || phone_diagnostics_are_error(&response.diagnostics);
    let text = phone_notifications_summary(&response);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

fn phone_app_response_kind_label(kind: PhoneAppResponseKind) -> &'static str {
    match kind {
        PhoneAppResponseKind::Current => "current",
        PhoneAppResponseKind::List => "list",
        PhoneAppResponseKind::Launch => "launch",
        PhoneAppResponseKind::OpenIntent => "open_intent",
        PhoneAppResponseKind::ForceStop => "force_stop",
        PhoneAppResponseKind::Install => "install",
        PhoneAppResponseKind::OpenSettings => "open_settings",
    }
}

pub(crate) fn phone_app_summary(response: &PhoneAppResponse) -> String {
    let kind = phone_app_response_kind_label(response.kind);
    let mut summary = if response.success {
        format!(
            "Phone app {} succeeded on session {} (backend {:?}).",
            kind, response.session_id, response.backend
        )
    } else {
        format!(
            "Phone app {} did not complete on session {}.",
            kind, response.session_id
        )
    };
    if let Some(app) = response.current_app.as_ref() {
        let _ = write!(&mut summary, " Foreground: {}.", app.package_name);
    }
    if !response.apps.is_empty() {
        let _ = write!(
            &mut summary,
            " {} app{}{}.",
            response.apps.len(),
            if response.apps.len() == 1 { "" } else { "s" },
            if response.truncated {
                " (truncated)"
            } else {
                ""
            }
        );
    }
    append_first_diagnostic(&mut summary, &response.diagnostics);
    summary
}

pub(crate) fn phone_app_result(response: PhoneAppResponse) -> Result<Value> {
    let is_error = !response.success || phone_diagnostics_are_error(&response.diagnostics);
    let text = phone_app_summary(&response);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}
