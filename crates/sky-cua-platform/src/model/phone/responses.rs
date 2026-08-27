use serde::{Deserialize, Serialize};

use crate::model::{AppShotEnvelope, DiagnosticEntry, PixelSize};

use super::capabilities::{
    PhoneBackendKind, PhoneCapabilityRefreshState, PhoneCompanionCapabilities,
};
use super::session::{
    PhoneAccessibilityNode, PhoneAppInfo, PhoneCoordinateMapping, PhoneCursorCapabilities,
    PhoneCursorState, PhoneImage, PhoneNotificationEvent, PhoneSession,
};

// ===========================================================================
// Responses
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneObserveResponse {
    pub session: PhoneSession,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appshot: Option<Box<AppShotEnvelope>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_image: Option<PhoneImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_app: Option<PhoneAppInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_summary: Option<super::session::PhoneAccessibilitySummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_notifications: Vec<PhoneNotificationEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<PhoneCursorState>,
    pub backend: PhoneBackendKind,
    pub capability_profile_id: String,
    /// Whether the profile that drove this observation was freshly detected,
    /// reused from cache, opportunistically refreshed, or stale. Mirrors the
    /// freshness gate carried by [`PhoneActionResponse`].
    pub profile_refresh_state: PhoneCapabilityRefreshState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_actions: Vec<super::capabilities::PhoneAvailableAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_actions: Vec<super::capabilities::PhoneUnavailableAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneScreenshotResponse {
    pub session_id: String,
    pub serial: String,
    pub phone_snapshot_id: String,
    pub backend: PhoneBackendKind,
    pub capability_profile_id: String,
    /// Freshness of the profile in force when this capture was taken. The
    /// returned snapshot feeds later coordinate actions, so the disposition
    /// (detected/reused/refreshed/stale) travels with it.
    pub profile_refresh_state: PhoneCapabilityRefreshState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_image: Option<PhoneImage>,
    pub device_size: PixelSize,
    pub coordinate_mapping: PhoneCoordinateMapping,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<PhoneCursorState>,
    pub cursor_capabilities: PhoneCursorCapabilities,
    pub capture_contains_native_overlay: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// Result of a coordinate/text/key action. `backend` states who actually
/// serviced it; `capability_profile_id` records which profile was in force.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneActionResponse {
    pub session_id: String,
    pub serial: String,
    pub action: String,
    pub backend: PhoneBackendKind,
    pub capability_profile_id: String,
    pub profile_refresh_state: PhoneCapabilityRefreshState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<PhoneCursorState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhonePairWirelessResponse {
    pub paired: bool,
    pub host_port: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneDisconnectResponse {
    pub session_id: String,
    pub serial: String,
    pub disconnected: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCompanionStatusResponse {
    pub session_id: String,
    pub serial: String,
    pub companion: PhoneCompanionCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneAccessibilityTreeResponse {
    pub session_id: String,
    pub serial: String,
    pub backend: PhoneBackendKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<PhoneAccessibilityNode>,
    pub truncated: bool,
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationsResponse {
    pub session_id: String,
    pub serial: String,
    pub backend: PhoneBackendKind,
    pub listener_enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<PhoneNotificationEvent>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}
