use serde_json::{Value, json};
use sky_cua_platform::model::{
    AccessibilitySetupReport, AppStateSnapshot, DiagnosticEntry, ElementNode,
    WindowTargetingSetupReport,
};

const LIST_APPS_INFORMATIONAL_DIAGNOSTIC_CODES: &[&str] = &["SessionEnvRepaired"];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AppStateDetail {
    #[default]
    Full,
    Compact,
}

pub(crate) fn compact_snapshot(snapshot: &AppStateSnapshot) -> Value {
    let elements: Vec<Value> = snapshot.elements.iter().map(compact_element).collect();
    json!({
        "detail": "compact",
        "snapshot_id": snapshot.snapshot_id,
        "created_at": snapshot.created_at,
        "focused_app": snapshot.focused_app,
        "capture": snapshot.capture,
        "agent_cursor": snapshot.agent_cursor,
        "diagnostics": snapshot.diagnostics,
        "app_guidance": snapshot.app_guidance,
        "doctor_report": snapshot.doctor_report,
        "elements": elements,
        "element_count": snapshot.elements.len()
    })
}

pub(crate) fn setup_window_targeting_is_error(report: &WindowTargetingSetupReport) -> bool {
    report.windows_error.is_some()
}

pub(crate) fn setup_accessibility_is_error(report: &AccessibilitySetupReport) -> bool {
    !report.accessibility_command.ok || !report.after.readiness.can_build_accessibility_tree
}

pub(crate) fn list_apps_error_diagnostic(
    diagnostics: &[DiagnosticEntry],
) -> Option<&DiagnosticEntry> {
    diagnostics.iter().find(|diagnostic| {
        !LIST_APPS_INFORMATIONAL_DIAGNOSTIC_CODES.contains(&diagnostic.code.as_str())
    })
}

pub(crate) fn compact_element(element: &ElementNode) -> Value {
    json!({
        "element_index": element.element_index,
        "parent_index": element.parent_index,
        "role": element.role,
        "name": element.name,
        "value": element.value,
        "text": element.text,
        "numeric_value": element.numeric_value,
        "supports_editable_text": element.supports_editable_text,
        "state_flags": element.state_flags,
        "semantic_actions": element.semantic_actions,
        "bounds": element.bounds,
        "backend_ref": element.backend_ref
    })
}
