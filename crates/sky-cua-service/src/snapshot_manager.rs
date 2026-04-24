use std::collections::HashMap;

use sky_cua_platform::model::AppStateSnapshot;

#[derive(Debug, Default)]
pub struct SnapshotManager {
    snapshots: HashMap<String, AppStateSnapshot>,
    order: Vec<String>,
    max_snapshots: usize,
}

impl SnapshotManager {
    #[must_use]
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            snapshots: HashMap::new(),
            order: Vec::new(),
            max_snapshots: max_snapshots.max(1),
        }
    }

    pub fn store(&mut self, snapshot: AppStateSnapshot) {
        let snapshot_id = snapshot.snapshot_id.clone();
        self.snapshots.insert(snapshot_id.clone(), snapshot);
        self.order.retain(|id| id != &snapshot_id);
        self.order.push(snapshot_id);

        while self.order.len() > self.max_snapshots {
            if let Some(oldest) = self.order.first().cloned() {
                self.order.remove(0);
                self.snapshots.remove(&oldest);
            }
        }
    }

    pub fn get(&self, snapshot_id: &str) -> Option<&AppStateSnapshot> {
        self.snapshots.get(snapshot_id)
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
                click: unavailable(),
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
        }
    }

    fn unavailable() -> ToolAvailability {
        ToolAvailability {
            available: false,
            reason: Some("nope".to_string()),
        }
    }
}
