use std::collections::{HashMap, VecDeque};

use sky_cua_platform::model::AppStateSnapshot;

#[derive(Debug, Default)]
pub struct SnapshotManager {
    snapshots: HashMap<String, AppStateSnapshot>,
    order: VecDeque<String>,
    max_snapshots: usize,
}

impl SnapshotManager {
    #[must_use]
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            snapshots: HashMap::new(),
            order: VecDeque::new(),
            max_snapshots: max_snapshots.max(1),
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
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use sky_cua_platform::model::{
        AppStateSnapshot, CaptureBackendKind, EnvironmentInfo, InputBackendKind,
        PortalCapabilities, SemanticBackendKind, SessionKind, ToolAvailability, ToolCapabilities,
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
        }
    }

    fn unavailable() -> ToolAvailability {
        ToolAvailability {
            available: false,
            reason: Some("nope".to_string()),
        }
    }
}
