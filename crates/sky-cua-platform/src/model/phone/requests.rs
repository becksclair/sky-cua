use serde::{Deserialize, Serialize};

use super::capabilities::PhoneBackendKind;

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

// ===========================================================================
// Requests
// ===========================================================================

/// Common session selector. Tools accept either a `session_id` (preferred) or a
/// raw `serial`; the service resolves either to an active session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneSessionSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Human alias mapped in `[phone.aliases]`. Mutually exclusive with
    /// `serial`/`device_id`/`session_id`; resolved to the underlying id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Canonical AppShot required before a state-changing phone operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneObserveRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<PhoneBackendKind>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub include_image_data: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_accessibility: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_notifications: bool,
}

impl Default for PhoneObserveRequest {
    fn default() -> Self {
        Self {
            session: PhoneSessionSelector::default(),
            backend: None,
            include_image_data: true,
            include_accessibility: false,
            include_notifications: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneStatusRequest {
    #[serde(default, skip_serializing_if = "is_false")]
    pub refresh_devices: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneListDevicesRequest {
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_mdns: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneRefreshCapabilitiesRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhonePairWirelessRequest {
    /// `host:port` of the pairing endpoint shown on the device.
    pub host_port: String,
    /// One-time pairing code. Never logged, stored, or echoed in responses.
    pub pairing_code: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneConnectRequest {
    /// USB serial, emulator serial, or `host:port` wireless target. Unset means
    /// the configured default or the single connected device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Stable Companion device id. Mutually exclusive with `serial`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Human alias from `[phone.aliases]`. Mutually exclusive with
    /// `serial`/`device_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<PhoneBackendKind>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub install_companion: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub start_scrcpy: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneDisconnectRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_wireless: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneScreenshotRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<PhoneBackendKind>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub include_image_data: bool,
}

impl Default for PhoneScreenshotRequest {
    fn default() -> Self {
        Self {
            session: PhoneSessionSelector::default(),
            backend: None,
            include_image_data: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneInstallCompanionRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "is_false")]
    pub force_reinstall: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_downgrade: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCompanionStatusRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAccessibilityTreeRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationsRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationOpenRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationDismissRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationActionRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub event_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationReplyRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub event_id: String,
    pub action_id: String,
    pub text: String,
}
