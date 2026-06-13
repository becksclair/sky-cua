use std::fs;

use anyhow::{Context, Result};
use sky_cua_platform::{
    AppInstructionIndex, app_instruction_entry_matches, app_instructions_index_path,
    app_instructions_root, focused_app_instruction_keys,
    model::{FocusedApp, HeuristicMatch},
};

#[derive(Debug, Clone)]
pub struct HeuristicsRegistry {
    entries: Vec<HeuristicEntry>,
}

#[derive(Debug, Clone)]
struct HeuristicEntry {
    index: sky_cua_platform::AppInstructionEntry,
    markdown: String,
}

impl HeuristicsRegistry {
    pub fn load_from_repo() -> Result<Self> {
        let base = app_instructions_root();
        let index_path = app_instructions_index_path();
        let raw = fs::read_to_string(&index_path)
            .with_context(|| format!("failed to read heuristics index {}", index_path.display()))?;
        let parsed: AppInstructionIndex =
            serde_json::from_str(&raw).context("failed to parse heuristics index JSON")?;
        let mut entries = Vec::new();
        for entry in parsed.entries {
            let markdown_path = base.join(&entry.path);
            let markdown = fs::read_to_string(&markdown_path).with_context(|| {
                format!(
                    "failed to read heuristics markdown at {}",
                    markdown_path.display()
                )
            })?;
            entries.push(HeuristicEntry {
                index: entry,
                markdown,
            });
        }
        Ok(Self { entries })
    }

    pub fn resolve_for_focused_app(&self, app: &FocusedApp) -> Option<HeuristicMatch> {
        self.resolve_from_keys(focused_app_instruction_keys(app))
    }

    fn resolve_from_keys(&self, keys: Vec<String>) -> Option<HeuristicMatch> {
        for entry in &self.entries {
            if app_instruction_entry_matches(&entry.index, &keys) {
                return Some(HeuristicMatch {
                    key: sky_cua_platform::normalize_app_instruction_key(&entry.index.key),
                    markdown: entry.markdown.clone(),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::HeuristicsRegistry;
    use sky_cua_platform::model::FocusedApp;

    #[test]
    fn resolves_by_desktop_file_id() {
        let registry = HeuristicsRegistry::load_from_repo().expect("registry should load");
        let resolved = registry.resolve_for_focused_app(&FocusedApp {
            app_id: "app".to_string(),
            name: "Kate".to_string(),
            pid: None,
            desktop_file_id: Some("org.kde.kate.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: None,
            window_title: None,
            display: None,
        });
        assert!(resolved.is_some());
    }

    #[test]
    fn resolves_kwrite_by_desktop_file_id() {
        let registry = HeuristicsRegistry::load_from_repo().expect("registry should load");
        let resolved = registry.resolve_for_focused_app(&FocusedApp {
            app_id: "app".to_string(),
            name: "KWrite".to_string(),
            pid: None,
            desktop_file_id: Some("kwrite.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: None,
            window_title: None,
            display: None,
        });
        assert!(resolved.is_some());
    }
}
