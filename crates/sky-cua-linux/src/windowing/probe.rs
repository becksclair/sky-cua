#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackendDescriptor {
    pub id: &'static str,
    pub failure_label: &'static str,
    pub list_note: &'static str,
    pub missing_hint: &'static str,
    pub can_exact_focus: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackendProbe {
    pub id: &'static str,
    pub ok: bool,
    pub can_list_windows: bool,
    pub can_focus_apps: bool,
    pub can_focus_windows: bool,
    pub detail: String,
}
