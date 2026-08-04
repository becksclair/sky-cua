//! Universal AppShot envelope returned by MCP `observe`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{ContentRef, DiagnosticEntry, PixelSize, RectF};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppShotSurface {
    Desktop,
    Browser,
    Phone,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppShotTrigger {
    Observe,
    Discovery,
    Connect,
    DesktopActivation,
    BrowserNavigation,
    PhoneAppLaunch,
    PhoneOpenIntent,
    Recovery,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppShotConsistency {
    Stable,
    ChangedDuringCapture,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "surface", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppShotCapture {
    Desktop {
        app_id: String,
        window_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        bounds: RectF,
        semantic_projection: serde_json::Value,
    },
    Browser {
        tab_id: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        viewport: PixelSize,
        document_generation: u64,
        semantic_snapshot: serde_json::Value,
    },
    Phone {
        device_id: String,
        link_epoch: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity_name: Option<String>,
        display_id: i32,
        window_ids: Vec<i32>,
        semantic_projection: serde_json::Value,
        event_sequence_before: u64,
        event_sequence_after: u64,
        full_tree_artifact: ContentRef,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppShotActionSnapshot {
    pub snapshot_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppShotCoverage {
    pub pixels_complete: bool,
    pub semantics_complete: bool,
    pub secure_regions_redacted: bool,
    pub projection_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_semantic_nodes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projected_semantic_nodes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppShotEnvelope {
    pub appshot_id: String,
    pub trigger: AppShotTrigger,
    pub captured_at: DateTime<Utc>,
    pub consistency: AppShotConsistency,
    #[serde(flatten)]
    pub capture: AppShotCapture,
    pub image: ContentRef,
    pub action_snapshot: AppShotActionSnapshot,
    pub coverage: AppShotCoverage,
    pub capability_profile_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppShotRejectionReason {
    Missing,
    Stale,
    WrongSurface,
    WrongTarget,
    WrongSession,
    WrongEpoch,
}

/// Structured mutation rejection. `fresh_appshot` is captured before return;
/// the requested state-changing operation has not executed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppShotRequired {
    pub code: String,
    pub reason: AppShotRejectionReason,
    pub message: String,
    pub fresh_appshot: Box<AppShotEnvelope>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContentPersistence, ContentSource};

    fn content() -> ContentRef {
        ContentRef {
            content_id: "content-1".into(),
            device_id: Some("device-1".into()),
            link_epoch: Some(9),
            mime_type: "image/webp".into(),
            filename: None,
            size_bytes: 10,
            sha256: "00".repeat(32),
            source: ContentSource::Screenshot,
            expires_at_ms: Some(1000),
            persistence: ContentPersistence::Temporary,
        }
    }

    #[test]
    fn phone_appshot_round_trips_with_epoch_and_tree_artifact() {
        let shot = AppShotEnvelope {
            appshot_id: "shot-1".into(),
            trigger: AppShotTrigger::Observe,
            captured_at: Utc::now(),
            consistency: AppShotConsistency::Stable,
            capture: AppShotCapture::Phone {
                device_id: "device-1".into(),
                link_epoch: 9,
                package_name: Some("dev.sky".into()),
                activity_name: None,
                display_id: 0,
                window_ids: vec![1],
                semantic_projection: serde_json::json!({"nodes": []}),
                event_sequence_before: 4,
                event_sequence_after: 4,
                full_tree_artifact: content(),
            },
            image: content(),
            action_snapshot: AppShotActionSnapshot {
                snapshot_id: "snapshot-1".into(),
                session_id: Some("session-1".into()),
                subject_generation: Some(9),
            },
            coverage: AppShotCoverage {
                pixels_complete: true,
                semantics_complete: true,
                secure_regions_redacted: false,
                projection_truncated: false,
                total_semantic_nodes: Some(0),
                projected_semantic_nodes: Some(0),
            },
            capability_profile_id: "cap-1".into(),
            diagnostics: vec![],
        };
        let encoded = serde_json::to_string(&shot).expect("serializes");
        let decoded: AppShotEnvelope = serde_json::from_str(&encoded).expect("round trip");
        assert_eq!(decoded, shot);
        let value = serde_json::to_value(decoded).expect("serializes");
        assert_eq!(value["surface"], "phone");
        assert!(value.get("subject").is_none());
        assert!(value.get("semantics").is_none());
    }

    #[test]
    fn phone_appshot_rejects_fields_from_another_surface() {
        let value = serde_json::json!({
            "surface": "phone",
            "device_id": "device-1",
            "link_epoch": 1,
            "display_id": 0,
            "window_ids": [],
            "semantic_projection": {},
            "event_sequence_before": 1,
            "event_sequence_after": 1,
            "full_tree_artifact": content(),
            "tab_id": "foreign-browser-tab"
        });
        assert!(serde_json::from_value::<AppShotCapture>(value).is_err());
    }
}
