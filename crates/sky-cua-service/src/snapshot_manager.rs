use std::collections::{HashMap, VecDeque};

use sky_cua_platform::model::{AppShotEnvelope, AppStateSnapshot};

#[derive(Debug, Default)]
pub struct SnapshotManager {
    snapshots: HashMap<String, AppStateSnapshot>,
    order: VecDeque<String>,
    max_snapshots: usize,
    appshots: HashMap<String, AppShotEnvelope>,
}

impl SnapshotManager {
    #[must_use]
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            snapshots: HashMap::new(),
            order: VecDeque::new(),
            max_snapshots: max_snapshots.max(1),
            appshots: HashMap::new(),
        }
    }

    pub fn store(&mut self, snapshot: AppStateSnapshot) {
        let snapshot_id = snapshot.snapshot_id.clone();
        self.snapshots.insert(snapshot_id.clone(), snapshot);
        self.order.retain(|id| id != &snapshot_id);
        self.order.push_back(snapshot_id);

        while self.order.len() > self.max_snapshots {
            if let Some(oldest) = self.order.pop_front() {
                self.snapshots.remove(&oldest);
            }
        }
    }

    pub fn get(&self, snapshot_id: &str) -> Option<&AppStateSnapshot> {
        self.snapshots.get(snapshot_id)
    }

    pub fn latest_snapshot_id(&self) -> Option<&str> {
        self.order.back().map(String::as_str)
    }

    pub fn latest(&self) -> Option<&AppStateSnapshot> {
        self.latest_snapshot_id()
            .and_then(|snapshot_id| self.snapshots.get(snapshot_id))
    }

    pub fn is_latest(&self, snapshot_id: &str) -> bool {
        self.latest_snapshot_id() == Some(snapshot_id)
    }

    pub fn get_if_latest(&self, snapshot_id: &str) -> Option<&AppStateSnapshot> {
        if self.is_latest(snapshot_id) {
            self.snapshots.get(snapshot_id)
        } else {
            None
        }
    }

    pub fn store_appshot(&mut self, appshot: AppShotEnvelope) {
        self.appshots.insert(appshot.appshot_id.clone(), appshot);
    }

    pub fn appshot(&self, appshot_id: &str) -> Option<&AppShotEnvelope> {
        self.appshots.get(appshot_id)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use sky_cua_platform::model::{
        AppShotActionSnapshot, AppShotCapture, AppShotConsistency, AppShotCoverage,
        AppShotEnvelope, AppShotTrigger, AppStateSnapshot, CaptureBackendKind, ContentPersistence,
        ContentRef, ContentSource, EnvironmentInfo, InputBackendKind, PortalCapabilities,
        SemanticBackendKind, SessionKind, ToolAvailability, ToolCapabilities,
    };

    use super::SnapshotManager;

    #[test]
    fn evicts_old_snapshots() {
        let mut manager = SnapshotManager::new(1);
        manager.store(snapshot("one"));
        manager.store(snapshot("two"));
        assert!(manager.get("one").is_none());
        assert!(manager.get("two").is_some());
        assert_eq!(manager.latest_snapshot_id(), Some("two"));
        assert!(manager.is_latest("two"));
        assert!(!manager.is_latest("one"));
    }

    #[test]
    fn registers_canonical_appshot_by_id_and_replaces_previous_capture() {
        let mut manager = SnapshotManager::new(2);
        manager.store_appshot(appshot("shot-1", "window-a", "snapshot-a"));
        manager.store_appshot(appshot("shot-1", "window-b", "snapshot-b"));

        let stored = manager
            .appshot("shot-1")
            .expect("appshot should be registered");
        let AppShotCapture::Desktop { window_id, .. } = &stored.capture else {
            panic!("expected desktop appshot");
        };
        assert_eq!(window_id, "window-b");
        assert_eq!(stored.action_snapshot.snapshot_id, "snapshot-b");
    }

    fn appshot(id: &str, window_id: &str, snapshot_id: &str) -> AppShotEnvelope {
        let content = ContentRef {
            content_id: format!("content-{id}"),
            device_id: None,
            link_epoch: None,
            mime_type: "image/webp".to_string(),
            filename: None,
            size_bytes: 1,
            sha256: "00".repeat(32),
            source: ContentSource::Screenshot,
            expires_at_ms: None,
            persistence: ContentPersistence::Temporary,
        };
        AppShotEnvelope {
            appshot_id: id.to_string(),
            trigger: AppShotTrigger::Observe,
            captured_at: Utc::now(),
            consistency: AppShotConsistency::Stable,
            capture: AppShotCapture::Desktop {
                app_id: "app.test".to_string(),
                window_id: window_id.to_string(),
                title: Some("Test".to_string()),
                bounds: sky_cua_platform::model::RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                    space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
                },
                semantic_projection: serde_json::json!({}),
            },
            image: content,
            action_snapshot: AppShotActionSnapshot {
                snapshot_id: snapshot_id.to_string(),
                session_id: Some("session-1".to_string()),
                subject_generation: None,
            },
            coverage: AppShotCoverage {
                pixels_complete: true,
                semantics_complete: true,
                secure_regions_redacted: false,
                projection_truncated: false,
                total_semantic_nodes: Some(0),
                projected_semantic_nodes: Some(0),
            },
            capability_profile_id: "desktop:test".to_string(),
            diagnostics: Vec::new(),
        }
    }

    fn snapshot(id: &str) -> AppStateSnapshot {
        AppStateSnapshot {
            snapshot_id: id.to_string(),
            created_at: Utc::now(),
            environment: EnvironmentInfo {
                session_kind: SessionKind::Unsupported,
                compositor: None,
                desktop_environment: None,
                capture_backend: CaptureBackendKind::None,
                input_backend: InputBackendKind::None,
                semantic_backend: SemanticBackendKind::None,
                portal_capabilities: PortalCapabilities {
                    screencast_version: None,
                    remote_desktop_version: None,
                    screenshot_version: None,
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: None,
                display: None,
                wayland_display: None,
                displays: Vec::new(),
            },
            capabilities: ToolCapabilities {
                list_apps: unavailable(),
                get_app_state: unavailable(),
                focus_element: unavailable(),
                activate_element: unavailable(),
                select_element: unavailable(),
                expand_element: unavailable(),
                collapse_element: unavailable(),
                toggle_element: unavailable(),
                click: unavailable(),
                perform_action: unavailable(),
                perform_secondary_action: unavailable(),
                scroll: unavailable(),
                supported_scroll_directions: Vec::new(),
                drag: unavailable(),
                type_text: unavailable(),
                press_key: unavailable(),
                set_value: unavailable(),
            },
            focused_app: None,
            capture: None,
            elements: Vec::new(),
            diagnostics: Vec::new(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        }
    }

    fn unavailable() -> ToolAvailability {
        ToolAvailability {
            available: false,
            reason: Some("nope".to_string()),
        }
    }
}
