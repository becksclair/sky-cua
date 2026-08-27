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

/// Readiness reported by a browser page while an AppShot is being captured.
/// `Unknown` is the compatibility default for older AppShots.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppShotBrowserReadinessState {
    Ready,
    Loading,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AppShotBrowserReadiness {
    #[serde(default)]
    pub state: AppShotBrowserReadinessState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_ready_state: Option<String>,
}

/// Result of the browser capture pipeline, independent of the older
/// `AppShotConsistency` field (which describes mutation during capture).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppShotBrowserCaptureStatus {
    /// The producer predates structured browser capture outcomes or did not
    /// report one. Callers must not infer success from absence of evidence.
    #[default]
    Unknown,
    Complete,
    Partial,
    DeadlineExceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppShotBrowserCaptureOutcome {
    #[serde(default)]
    pub status: AppShotBrowserCaptureStatus,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl Default for AppShotBrowserCaptureOutcome {
    fn default() -> Self {
        Self {
            status: AppShotBrowserCaptureStatus::Unknown,
            retryable: false,
            phase: None,
            timeout_ms: None,
        }
    }
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
        #[serde(default)]
        readiness: AppShotBrowserReadiness,
        #[serde(default)]
        capture_outcome: AppShotBrowserCaptureOutcome,
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
    Expired,
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

    #[test]
    fn browser_appshot_defaults_new_fields_for_old_payloads() {
        let value = serde_json::json!({
            "surface": "browser",
            "tab_id": "tab-1",
            "url": "https://example.test/",
            "viewport": {"width": 800, "height": 600},
            "document_generation": 7,
            "semantic_snapshot": {"elements": []}
        });
        let decoded: AppShotCapture = serde_json::from_value(value).expect("legacy payload");
        let AppShotCapture::Browser {
            readiness,
            capture_outcome,
            ..
        } = decoded
        else {
            panic!("expected browser capture");
        };
        assert_eq!(readiness.state, AppShotBrowserReadinessState::Unknown);
        assert_eq!(capture_outcome.status, AppShotBrowserCaptureStatus::Unknown);
        assert!(!capture_outcome.retryable);
    }

    #[test]
    fn browser_appshot_serializes_structured_readiness_and_outcome() {
        let capture = AppShotCapture::Browser {
            tab_id: "tab-1".into(),
            url: "https://example.test/".into(),
            title: None,
            viewport: PixelSize {
                width: 800,
                height: 600,
            },
            document_generation: 7,
            semantic_snapshot: serde_json::json!({"elements": []}),
            readiness: AppShotBrowserReadiness {
                state: AppShotBrowserReadinessState::Loading,
                raw_ready_state: Some("interactive".into()),
            },
            capture_outcome: AppShotBrowserCaptureOutcome {
                status: AppShotBrowserCaptureStatus::DeadlineExceeded,
                retryable: true,
                phase: Some("semantic_snapshot".into()),
                timeout_ms: Some(5000),
            },
        };
        let value = serde_json::to_value(capture).expect("serialize browser capture");
        assert_eq!(value["readiness"]["state"], "loading");
        assert_eq!(value["readiness"]["raw_ready_state"], "interactive");
        assert_eq!(value["capture_outcome"]["status"], "deadline_exceeded");
        assert_eq!(value["capture_outcome"]["retryable"], true);
        assert_eq!(value["capture_outcome"]["timeout_ms"], 5000);
    }
}
