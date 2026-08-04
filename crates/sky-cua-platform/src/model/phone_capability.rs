//! Operation-specific phone capability routing and identity contracts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneTransportKind {
    Adb,
    CompanionV1,
    CompanionDirect,
    Scrcpy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhoneConnectionIdentity {
    Adb {
        serial: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    CompanionV1 {
        serial: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    CompanionDirect {
        device_id: String,
        link_epoch: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
    },
    Scrcpy {
        serial: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneOperationProvider {
    Adb,
    CompanionAccessibility,
    CompanionNative,
    CompanionIme,
    CompanionCamera,
    CompanionStorage,
    SettingsUi,
    InstallerUi,
    Scrcpy,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneCapabilityAvailability {
    Ready,
    PermissionRequired,
    ActivationRequired,
    ReconnectRequired,
    TemporarilyUnavailable,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneActivationClass {
    None,
    VisibleActivity,
    ForegroundService,
    AccessibilityService,
    NotificationListener,
    DefaultIme,
    UserSettings,
    SafGrant,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneCapabilityFidelity {
    Exact,
    Native,
    UiFallback,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCapabilityRoute {
    pub operation: String,
    pub provider: PhoneOperationProvider,
    pub availability: PhoneCapabilityAvailability,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prerequisites: Vec<String>,
    pub activation: PhoneActivationClass,
    pub fidelity: PhoneCapabilityFidelity,
    pub evidenced_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCapabilityProfileIdentity {
    pub profile_id: String,
    pub session_id: String,
    pub connection: PhoneConnectionIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<PhoneCapabilityRoute>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_identity_does_not_require_an_adb_serial() {
        let identity = PhoneCapabilityProfileIdentity {
            profile_id: "p1".into(),
            session_id: "s1".into(),
            connection: PhoneConnectionIdentity::CompanionDirect {
                device_id: "d1".into(),
                link_epoch: 4,
                name: None,
                endpoint: None,
            },
            routes: vec![],
        };
        let value = serde_json::to_value(identity).expect("serializes");
        assert!(value["connection"].get("serial").is_none());
        assert_eq!(value["connection"]["transport"], "companion_direct");
        assert_eq!(value["connection"]["link_epoch"], 4);
    }

    #[test]
    fn transport_identity_rejects_mixed_direct_and_adb_fields() {
        assert!(
            serde_json::from_value::<PhoneConnectionIdentity>(serde_json::json!({
                "transport": "companion_direct",
                "device_id": "d1",
                "link_epoch": 4,
                "serial": "adb-serial"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PhoneConnectionIdentity>(serde_json::json!({
                "transport": "adb",
                "serial": "adb-serial",
                "device_id": "d1"
            }))
            .is_err()
        );
    }
}
