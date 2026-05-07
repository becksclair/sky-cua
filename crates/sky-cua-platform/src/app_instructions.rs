use std::path::{Path, PathBuf};

use crate::model::FocusedApp;

const APP_INSTRUCTIONS_INDEX_RELATIVE_PATH: &str = "resources/app-instructions/index.json";

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppInstructionIndex {
    pub entries: Vec<AppInstructionEntry>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppInstructionEntry {
    pub key: String,
    pub path: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub set_value_fallback: Option<SetValueFallbackMode>,
    #[serde(default)]
    pub set_value_routing: SetValueRouting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetValueFallbackMode {
    FocusClickSelectAllType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SetValueRouting {
    #[default]
    PreferSemantic,
    PreferPhysicalFallback,
}

#[must_use]
pub fn app_instruction_entry_matches(entry: &AppInstructionEntry, keys: &[String]) -> bool {
    let normalized_key = normalize_app_instruction_key(&entry.key);
    let normalized_aliases: Vec<String> = entry
        .aliases
        .iter()
        .map(|alias| normalize_app_instruction_key(alias))
        .collect();

    keys.iter()
        .any(|key| key == &normalized_key || normalized_aliases.iter().any(|alias| alias == key))
}

#[must_use]
pub fn app_instructions_root() -> PathBuf {
    repo_root().join("resources/app-instructions")
}

#[must_use]
pub fn app_instructions_index_path() -> PathBuf {
    repo_root().join(APP_INSTRUCTIONS_INDEX_RELATIVE_PATH)
}

#[must_use]
pub fn normalize_app_instruction_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn focused_app_instruction_keys(app: &FocusedApp) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(desktop_file_id) = app.desktop_file_id.as_deref() {
        keys.push(normalize_app_instruction_key(desktop_file_id));
    }
    keys.push(normalize_app_instruction_key(&app.name));
    if let Some(toolkit_guess) = app.toolkit_guess.as_deref() {
        keys.push(normalize_app_instruction_key(toolkit_guess));
    }
    keys
}

fn repo_root() -> PathBuf {
    if let Some(path) = std::env::var_os("SKY_CUA_REPO_ROOT") {
        return PathBuf::from(path);
    }

    std::env::current_dir()
        .ok()
        .and_then(|cwd| find_repo_root_from(&cwd))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn find_repo_root_from(start: &Path) -> Option<PathBuf> {
    for candidate in start.ancestors() {
        if candidate
            .join(APP_INSTRUCTIONS_INDEX_RELATIVE_PATH)
            .exists()
        {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        AppInstructionEntry, app_instruction_entry_matches, focused_app_instruction_keys,
        normalize_app_instruction_key,
    };
    use crate::model::FocusedApp;

    #[test]
    fn normalizes_instruction_keys() {
        assert_eq!(
            normalize_app_instruction_key("org.kde.kate.desktop"),
            "org kde kate desktop"
        );
        assert_eq!(
            normalize_app_instruction_key("Mozilla Firefox"),
            "mozilla firefox"
        );
    }

    #[test]
    fn derives_instruction_keys_from_focused_app() {
        let keys = focused_app_instruction_keys(&FocusedApp {
            app_id: "app".to_string(),
            name: "Kate".to_string(),
            pid: None,
            desktop_file_id: Some("org.kde.kate.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("Qt".to_string()),
            window_title: None,
        });

        assert_eq!(keys, vec!["org kde kate desktop", "kate", "qt"]);
    }

    #[test]
    fn matches_entries_by_key_or_alias() {
        let entry = AppInstructionEntry {
            key: "org.kde.kate.desktop".to_string(),
            path: "Kate.md".to_string(),
            aliases: vec!["KATE.desktop".to_string()],
            set_value_fallback: None,
            set_value_routing: Default::default(),
        };

        assert!(app_instruction_entry_matches(
            &entry,
            &["kate desktop".to_string()]
        ));
        assert!(!app_instruction_entry_matches(
            &entry,
            &["firefox desktop".to_string()]
        ));
    }
}
