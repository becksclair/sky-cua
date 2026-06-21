//! Android companion app backend — host RPC side (Phase 4).
//!
//! The companion is the preferred rich backend after the ADB bootstrap: native
//! gesture dispatch, accessibility-tree retrieval, on-device screenshots,
//! notification listening/actions, and the phone-native cursor overlay. This
//! module owns the host half of that:
//!
//! - [`protocol`]: typed serde DTOs for the v1 wire contract (documented in
//!   `docs/runtime/phone-companion-protocol.md`).
//! - [`client`]: a hand-rolled HTTP/1.1 JSON-RPC client over a
//!   [`tokio::net::TcpStream`] to the host-forwarded localhost endpoint, with
//!   structured failures that signal fallback to ADB/scrcpy.
//! - [`identity`]: install/update/refuse decisioning from package version +
//!   signing cert + APK hash, plus ephemeral token generation and the ADB
//!   setup-intent / install argv builders (which do not themselves run adb).
//!
//! Capability reporting builds a [`PhoneCompanionCapabilities`] from a
//! health/capabilities RPC response, extending the Phase 1 `absent` constructor
//! the spine shipped. The integrator (`manager.rs`) wires these into routing;
//! functions not yet called outside tests keep the spine's
//! `#[cfg_attr(not(test), expect(dead_code))]` idiom so non-test builds stay
//! clean.

pub(crate) mod client;
pub(crate) mod identity;
pub(crate) mod protocol;

#[cfg(test)]
mod tests;

use sky_cua_platform::model::{DiagnosticEntry, PhoneCompanionCapabilities};

use identity::{CompanionToken, InstalledCompanion};
use protocol::{CapabilitiesResult, HealthResult};

/// Stable diagnostic for companion operations that are not implemented yet.
///
/// Retained from the Phase 1 spine: the manager still calls this for the
/// session-less `phone_companion_status` path (no live RPC client to query).
pub(super) fn not_implemented_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneCompanionNotImplemented".to_string(),
        message: "phone companion backend is not implemented yet (Phase 1 contract spine)"
            .to_string(),
        details: None,
    }
}

/// Companion capabilities for a device where the companion has not been probed
/// or installed. Identity carries the expected package so callers can reason
/// about a future install.
///
/// Retained from the Phase 1 spine and still used by the manager's session-less
/// status path and by `phone_connect` before the first RPC.
pub(super) fn absent_companion(package_name: &str) -> PhoneCompanionCapabilities {
    PhoneCompanionCapabilities::absent(package_name)
}

/// Build a [`PhoneCompanionCapabilities`] from a `capabilities` RPC response plus
/// the identity/install/token context the host already resolved.
///
/// The raw permission booleans (`can_perform_gestures`, `can_take_screenshot`,
/// ...) come straight from the response. The derived end-to-end capability flags
/// the rest of the runtime routes on are computed conservatively:
/// - `gesture_dispatch` requires accessibility enabled AND `can_perform_gestures`
///   AND the server's `gesture_supported`.
/// - `screenshot` requires `can_take_screenshot` AND `screenshot_supported`.
/// - `accessibility_tree` requires accessibility enabled AND
///   `can_retrieve_window_content`.
/// - `notifications` requires `notification_listener_enabled`.
///
/// The integrator calls this after `adb forward` + token provisioning, passing
/// the install decision (so the report records `signature_matches_expected` /
/// `allow_downgrade` / `auto_install_attempted`), the installed signing-cert
/// SHA-256 the host parsed from `dumpsys` during `read_installed_companion` (so
/// the reachable report carries `installed_cert_sha256` like the unreachable
/// path), and the active token (so `rpc_token_expires_at_ms` is set).
/// `rpc_reachable` is true because a successful `capabilities` call is what
/// produced `caps`.
pub(super) fn capabilities_from_response(
    caps: &CapabilitiesResult,
    token: Option<&CompanionToken>,
    installed_cert_sha256: Option<&str>,
    expected_cert_sha256: Option<&str>,
    apk_sha256: Option<&str>,
    auto_install_attempted: bool,
    allow_downgrade: bool,
) -> PhoneCompanionCapabilities {
    let health = &caps.health;
    // Honest signature report: a match requires BOTH the installed and expected
    // certs to be present and equal. An unreadable installed cert (modern Android
    // does not expose the SHA-256 via `dumpsys`) reports as a non-match, never as
    // a verified match. A readable mismatch never reaches here — it is refused at
    // the install decision.
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
        rpc_token_expires_at_ms: token.map(|t| t.expires_at_ms),
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

/// Build capabilities from a bare `health` response (no screenshot/gesture API
/// detail). Used by `bootstrap_companion` when the richer `capabilities` method
/// is unavailable but `health` succeeds (e.g. an older companion build).
/// `screenshot`/`gesture_dispatch` fall back to the raw permission booleans
/// because the support detail is unknown; `rpc_reachable` is true.
pub(super) fn capabilities_from_health(
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

/// Capabilities for a device where the companion is installed (identity known)
/// but its RPC endpoint is unreachable — e.g. `adb forward` failed, the app was
/// killed, or the token was rejected. Reports the installed identity (including
/// the expected packaged-APK SHA-256 from build metadata) but every runtime
/// capability false and `rpc_reachable=false`, so the manager routes to
/// ADB/scrcpy without claiming companion success.
pub(super) fn capabilities_unreachable(
    package_name: &str,
    installed: &InstalledCompanion,
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
