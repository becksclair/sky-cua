use serde::{Deserialize, Serialize};

use crate::model::{PhoneCapabilityRoute, PixelSize};

// ===========================================================================
// Backend / connection / target enums
// ===========================================================================

/// How the host is transported to the device.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneConnectionKind {
    Usb,
    Emulator,
    /// Legacy `adb tcpip 5555` then `adb connect host:5555`.
    LegacyTcpip,
    /// Android 11+ wireless debugging via `adb pair`.
    WirelessDebugging,
    /// Phone-initiated `phone-control.v2` link; no ADB serial exists.
    CompanionDirect,
    Unknown,
}

/// The backend family that handled (or would handle) an operation. Every action
/// response states which backend actually serviced it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneBackendKind {
    /// Auto-routing: the service picks the best available backend.
    Auto,
    /// ADB baseline (required): shell screencap/input, install, forward.
    Adb,
    /// Android companion app: native gestures, accessibility, notifications.
    Companion,
    /// scrcpy mirror/control acceleration.
    Scrcpy,
    /// No backend could service the operation.
    None,
}

/// Coarse classification of the connected device for compatibility lanes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneTargetDeviceKind {
    GalaxyS26Ultra,
    RedmiTablet,
    Emulator,
    UnknownAndroid,
}

/// Lifecycle of the cached capability profile relative to the latest request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneCapabilityRefreshState {
    /// Detected fresh during this request.
    Detected,
    /// Reused an unexpired cached profile.
    Reused,
    /// Opportunistically refreshed an expired profile.
    Refreshed,
    /// Reused, but the cache TTL has elapsed and availability is not re-proven.
    Stale,
}

// ===========================================================================
// Capability profile, companion, scrcpy capabilities
// ===========================================================================

/// Per-session companion app capability and identity report. Identity fields
/// (`package_name`, versions, cert/apk hashes) let `phone_connect` decide
/// install/update/refuse before any backend RPC happens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCompanionCapabilities {
    pub installed: bool,
    pub package_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_cert_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_cert_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_sha256: Option<String>,
    pub signature_matches_expected: bool,
    pub allow_downgrade: bool,
    pub auto_install_attempted: bool,
    pub rpc_reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_token_expires_at_ms: Option<u64>,
    pub accessibility_enabled: bool,
    pub can_perform_gestures: bool,
    pub can_retrieve_window_content: bool,
    pub can_take_screenshot: bool,
    pub notification_listener_enabled: bool,
    pub native_overlay: bool,
    pub native_overlay_pass_through: bool,
    pub gesture_dispatch: bool,
    pub screenshot: bool,
    pub accessibility_tree: bool,
    pub notifications: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privileged_setup: Option<String>,
}

impl PhoneCompanionCapabilities {
    /// A companion that is not installed and exposes nothing. Identity fields
    /// still carry the expected package/cert so callers can reason about
    /// install/update before the APK exists.
    #[must_use]
    pub fn absent(package_name: impl Into<String>) -> Self {
        Self {
            installed: false,
            package_name: package_name.into(),
            installed_version: None,
            expected_version: None,
            installed_cert_sha256: None,
            expected_cert_sha256: None,
            apk_sha256: None,
            signature_matches_expected: false,
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
}

/// scrcpy acceleration capability for this session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneScrcpyCapabilities {
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub active: bool,
    pub host_window_mapped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PhoneScrcpyCapabilities {
    /// scrcpy not installed or not in use.
    #[must_use]
    pub fn absent() -> Self {
        Self {
            installed: false,
            version: None,
            active: false,
            host_window_mapped: false,
            window_title: None,
            video_codec: None,
            reason: None,
        }
    }
}

/// Backend availability summary for a session, distinct from the full
/// capability profile. This is the quick "what can this session do" view that
/// rides on `PhoneSession`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneBackendCapabilities {
    pub adb: bool,
    pub companion: bool,
    pub scrcpy: bool,
    pub screenshot: bool,
    pub gestures: bool,
    pub text_input: bool,
    pub key_input: bool,
    pub accessibility_tree: bool,
    pub notifications: bool,
    pub app_management: bool,
    pub host_visible_overlay: bool,
    pub screenshot_synthetic_cursor: bool,
    pub phone_native_overlay: bool,
}

/// Structured, cached description of what a device/session can do right now.
/// Detected during `phone_connect`, invalidated on reconnect, companion
/// install/update, permission/orientation/display change, RPC failure, wireless
/// disconnect, and explicit `phone_refresh_capabilities`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCapabilityProfile {
    pub profile_id: String,
    pub session_id: String,
    pub serial: String,
    pub detected_at_ms: u64,
    pub stale: bool,
    pub refresh_state: PhoneCapabilityRefreshState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub target_device_kind: PhoneTargetDeviceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hyperos_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_sdk: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_release: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_size: Option<PixelSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density_dpi: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    /// The device's live screen rotation as an exact quarter turn
    /// (0/90/180/270), read from the `dumpsys` rotation probe. `orientation` is
    /// the coarse portrait/landscape label for humans; this carries the precise
    /// quarter the host content-rect math needs so 180/270 are not collapsed
    /// back into the label's two states. `None` when no live rotation was
    /// probed, in which case consumers fall back to the label-derived quarter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_rotation_degrees: Option<i32>,
    pub connection_kind: PhoneConnectionKind,
    pub companion: PhoneCompanionCapabilities,
    pub scrcpy: PhoneScrcpyCapabilities,
    pub root_available: bool,
    pub shizuku_available: bool,
    pub device_owner: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_actions: Vec<PhoneAvailableAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_actions: Vec<PhoneUnavailableAction>,
    /// Provider-specific truth for each operation. `available_actions` and
    /// `unavailable_actions` remain the compact agent-facing projection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<PhoneCapabilityRoute>,
}

// ===========================================================================
// Action affordances
// ===========================================================================

/// An action the agent can take right now, with the backend that would service
/// it. The `action` string is the canonical tool name (e.g. `phone_tap`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAvailableAction {
    pub action: String,
    pub backend: PhoneBackendKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// An action that is not currently possible, with a structured reason so the
/// agent understands why (disabled permission, missing companion, wrong API
/// level, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneUnavailableAction {
    pub action: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
