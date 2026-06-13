use sky_cua_platform::{
    AppInstructionEntry, AppInstructionIndex, SetValueFallbackMode, SetValueRouting,
    app_instruction_entry_matches, app_instructions_index_path, focused_app_instruction_keys,
    model::FocusedApp, normalize_app_instruction_key,
};
use std::fs;

#[derive(Debug, Clone, Default)]
pub struct AppActionPolicies {
    entries: Vec<AppActionPolicyEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSetValueFallbackPolicy {
    pub key: String,
    pub mode: SetValueFallbackMode,
    pub routing: SetValueRouting,
}

#[derive(Debug, Clone)]
struct AppActionPolicyEntry {
    index: AppInstructionEntry,
    set_value_fallback: SetValueFallbackMode,
    set_value_routing: SetValueRouting,
}

impl AppActionPolicies {
    pub fn load_from_repo() -> std::io::Result<Self> {
        let index_path = app_instructions_index_path();
        let raw = fs::read_to_string(&index_path)?;
        let parsed: AppInstructionIndex = serde_json::from_str(&raw).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse app action policy index {}: {error}",
                    index_path.display()
                ),
            )
        })?;

        let entries = parsed
            .entries
            .into_iter()
            .filter_map(|entry| {
                entry.set_value_fallback.map(|mode| AppActionPolicyEntry {
                    set_value_routing: entry.set_value_routing,
                    index: entry,
                    set_value_fallback: mode,
                })
            })
            .collect();
        Ok(Self { entries })
    }

    pub fn resolve_set_value_fallback(
        &self,
        app: Option<&FocusedApp>,
    ) -> Option<ResolvedSetValueFallbackPolicy> {
        let keys = focused_app_instruction_keys(app?);

        for entry in &self.entries {
            if app_instruction_entry_matches(&entry.index, &keys) {
                return Some(ResolvedSetValueFallbackPolicy {
                    key: normalize_app_instruction_key(&entry.index.key),
                    mode: entry.set_value_fallback,
                    routing: entry.set_value_routing,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::AppActionPolicies;
    use sky_cua_platform::{SetValueFallbackMode, SetValueRouting, model::FocusedApp};

    #[test]
    fn resolves_set_value_fallback_for_kate_by_desktop_file_id() {
        let policies = AppActionPolicies::load_from_repo().expect("policies should load");
        let resolved = policies
            .resolve_set_value_fallback(Some(&FocusedApp {
                app_id: "app".to_string(),
                name: "Kate".to_string(),
                pid: None,
                desktop_file_id: Some("org.kde.kate.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("Qt".to_string()),
                window_title: None,
                display: None,
            }))
            .expect("kate should resolve a set_value fallback policy");

        assert_eq!(resolved.key, "org kde kate desktop");
        assert_eq!(resolved.mode, SetValueFallbackMode::FocusClickSelectAllType);
        assert_eq!(resolved.routing, SetValueRouting::PreferPhysicalFallback);
    }

    #[test]
    fn resolves_set_value_fallback_for_kwrite_by_desktop_file_id() {
        let policies = AppActionPolicies::load_from_repo().expect("policies should load");
        let resolved = policies
            .resolve_set_value_fallback(Some(&FocusedApp {
                app_id: "app".to_string(),
                name: "KWrite".to_string(),
                pid: None,
                desktop_file_id: Some("kwrite.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("Qt".to_string()),
                window_title: None,
                display: None,
            }))
            .expect("kwrite should resolve a set_value fallback policy");

        assert_eq!(resolved.key, "kwrite desktop");
        assert_eq!(resolved.mode, SetValueFallbackMode::FocusClickSelectAllType);
        assert_eq!(resolved.routing, SetValueRouting::PreferPhysicalFallback);
    }

    #[test]
    fn does_not_resolve_set_value_fallback_for_firefox() {
        let policies = AppActionPolicies::load_from_repo().expect("policies should load");
        let resolved = policies.resolve_set_value_fallback(Some(&FocusedApp {
            app_id: "app".to_string(),
            name: "Firefox".to_string(),
            pid: None,
            desktop_file_id: Some("firefox.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("GTK".to_string()),
            window_title: None,
            display: None,
        }));

        assert!(resolved.is_none());
    }
}
