use serde::{Deserialize, Serialize};

use crate::model::{DiagnosticEntry, PhoneConnectionIdentity, RectF};

use super::capabilities::{
    PhoneBackendCapabilities, PhoneBackendKind, PhoneCapabilityProfile, PhoneCompanionCapabilities,
    PhoneConnectionKind,
};

// ===========================================================================
// Image payload
// ===========================================================================

/// Inline image payload. Field shape matches `BrowserScreenshotResponse` so the
/// MCP client encodes phone screenshots exactly like browser screenshots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneImage {
    pub mime_type: String,
    pub data_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

// ===========================================================================
// Session, cursor, coordinate mapping
// ===========================================================================

/// A live phone session. Created by `phone_connect`, referenced by every
/// follow-up tool through `session_id`/`serial`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneSession {
    pub session_id: String,
    /// Present for ADB-backed compatibility sessions. Direct sessions omit it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
    /// Typed transport identity. New callers should use this instead of
    /// inferring transport or identity from `serial`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<PhoneConnectionIdentity>,
    pub connection_kind: PhoneConnectionKind,
    pub backend: PhoneBackendKind,
    pub capabilities: PhoneBackendCapabilities,
    pub capability_profile: PhoneCapabilityProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion: Option<PhoneCompanionCapabilities>,
    pub managed_process: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    pub created_at_ms: u64,
}

/// Which cursor planes are live for a session/snapshot. Mirrors the three-plane
/// design: host-visible overlay, screenshot-synthetic marker, phone-native
/// accessibility overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCursorCapabilities {
    pub host_visible_overlay: bool,
    pub screenshot_synthetic_cursor: bool,
    pub phone_native_overlay: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_overlay_reason: Option<String>,
}

/// Cursor position state after an action, in device coordinates plus the
/// snapshot it was captured against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCursorState {
    pub visible: bool,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_point: Option<PhonePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_point: Option<PhonePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action: Option<String>,
    pub updated_at_ms: u64,
}

/// A 2D point used for cursor/tap coordinates. The plane (device, screenshot, or
/// host) is implied by the field that holds it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PhonePoint {
    pub x: f64,
    pub y: f64,
}

/// Data to translate between device pixels, screenshot pixels, and host desktop
/// pixels for a captured snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCoordinateMapping {
    pub mapping_id: String,
    pub session_id: String,
    pub serial: String,
    pub device_rect: RectF,
    pub screenshot_rect: RectF,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_window_rect: Option<RectF>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_content_rect: Option<RectF>,
    pub rotation_degrees: i32,
    pub captured_at_ms: u64,
}

// ===========================================================================
// Device listing / status
// ===========================================================================

/// State of a device as ADB reports it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneDeviceState {
    Device,
    Unauthorized,
    Offline,
    NoPermissions,
    Connecting,
    Bootloader,
    Recovery,
    Unknown,
}

/// One device as seen by `phone_list_devices`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneDevice {
    /// Present for ADB-discovered devices. Direct devices omit it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
    /// Stable Companion identity for a direct device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Current authenticated link epoch for a direct device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_epoch: Option<u64>,
    /// Explicit transport identity; avoids synthetic ADB serials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<PhoneConnectionIdentity>,
    pub state: PhoneDeviceState,
    pub connection_kind: PhoneConnectionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_id: Option<String>,
    /// Whether this device's `model` matches one of the operator's configured
    /// `[phone] primary_target_models`. Set by the host device-list path (not the
    /// ADB wire parse); primaries are surfaced first in the listing. Defaults to
    /// `false` and is omitted from the wire when unset.
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary: bool,
    /// Operator-configured human alias for this device (`[phone.aliases]`).
    /// Present only when the configured alias value matches this device's
    /// `device_id` (CompanionDirect) or `serial` (ADB). Omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// Response for `phone_list_devices`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneListDevicesResponse {
    pub devices: Vec<PhoneDevice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// Response for `phone_status`: host tooling readiness plus active sessions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneStatusReport {
    pub enabled: bool,
    pub adb_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_server_running: Option<bool>,
    pub scrcpy_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrcpy_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrcpy_version: Option<String>,
    pub companion_enabled: bool,
    pub mdns_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_serial: Option<String>,
    pub default_backend: PhoneBackendKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<PhoneSession>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<PhoneDevice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

// ===========================================================================
// Accessibility / notifications / apps
// ===========================================================================

/// Bounded accessibility summary embedded in `phone_observe`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAccessibilitySummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    pub node_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headline_texts: Vec<String>,
    pub truncated: bool,
    pub redacted: bool,
}

/// One accessibility tree node. A flat parent-indexed list mirrors the desktop
/// `ElementNode` shape without recreating that desktop-specific type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneAccessibilityNode {
    pub node_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<RectF>,
    pub clickable: bool,
    pub focusable: bool,
    pub enabled: bool,
    pub redacted: bool,
}

/// Redaction state of a notification's content.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneNotificationRedaction {
    None,
    Partial,
    Full,
}

/// One notification action button exposed for `phone_notification_action`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationAction {
    pub action_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub supports_inline_reply: bool,
}

/// One notification event. `event_id` is the stable handle required by all
/// notification action tools; an action must reference a fresh observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationEvent {
    pub event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub package_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub redaction: PhoneNotificationRedaction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    pub ongoing: bool,
    pub can_open: bool,
    pub can_dismiss: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PhoneNotificationAction>,
    pub posted_at_ms: u64,
}

/// Foreground or launchable app description.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAppInfo {
    pub package_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<u64>,
    pub launchable: bool,
    pub system_app: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}
