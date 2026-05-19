use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sky_cua_platform::model::{
    AccessibilitySetupReport, AgentCursorState, AppStateSnapshot, CaptureInfo, DiagnosticEntry,
    DoctorReport, ElementNode, ElementNumericValueReadback, ElementTextReadback, FocusedApp,
    HeuristicMatch, RectF, WindowTargetingSetupReport,
};

const LIST_APPS_INFORMATIONAL_DIAGNOSTIC_CODES: &[&str] = &["SessionEnvRepaired"];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AppStateDetail {
    #[default]
    Full,
    Compact,
}

/// Borrowed view of an [`ElementNode`] used for the compact snapshot serialization.
/// Keeps all keys present (including `null` for `None` values) to preserve the contract
/// that the previous `json!` macro implementation produced.
#[derive(Serialize)]
struct CompactElementNode<'a> {
    element_index: usize,
    parent_index: Option<usize>,
    role: &'a str,
    name: &'a Option<String>,
    value: &'a Option<String>,
    text: &'a Option<ElementTextReadback>,
    numeric_value: &'a Option<ElementNumericValueReadback>,
    supports_editable_text: bool,
    state_flags: &'a [String],
    semantic_actions: &'a [String],
    bounds: &'a Option<RectF>,
    backend_ref: &'a Option<String>,
}

impl<'a> From<&'a ElementNode> for CompactElementNode<'a> {
    fn from(element: &'a ElementNode) -> Self {
        Self {
            element_index: element.element_index,
            parent_index: element.parent_index,
            role: &element.role,
            name: &element.name,
            value: &element.value,
            text: &element.text,
            numeric_value: &element.numeric_value,
            supports_editable_text: element.supports_editable_text,
            state_flags: &element.state_flags,
            semantic_actions: &element.semantic_actions,
            bounds: &element.bounds,
            backend_ref: &element.backend_ref,
        }
    }
}

/// Borrowed view of an [`AppStateSnapshot`] used for the compact serialization.
#[derive(Serialize)]
struct CompactSnapshot<'a> {
    detail: &'static str,
    snapshot_id: &'a str,
    created_at: &'a DateTime<Utc>,
    focused_app: &'a Option<FocusedApp>,
    capture: &'a Option<CaptureInfo>,
    agent_cursor: &'a Option<AgentCursorState>,
    diagnostics: &'a Vec<DiagnosticEntry>,
    app_guidance: &'a Option<HeuristicMatch>,
    doctor_report: &'a Option<DoctorReport>,
    elements: &'a [CompactElementNode<'a>],
    element_count: usize,
}

pub(crate) fn compact_snapshot(snapshot: &AppStateSnapshot) -> Value {
    let compact_elements: Vec<CompactElementNode> = snapshot
        .elements
        .iter()
        .map(CompactElementNode::from)
        .collect();
    let compact = CompactSnapshot {
        detail: "compact",
        snapshot_id: &snapshot.snapshot_id,
        created_at: &snapshot.created_at,
        focused_app: &snapshot.focused_app,
        capture: &snapshot.capture,
        agent_cursor: &snapshot.agent_cursor,
        diagnostics: &snapshot.diagnostics,
        app_guidance: &snapshot.app_guidance,
        doctor_report: &snapshot.doctor_report,
        elements: &compact_elements,
        element_count: snapshot.elements.len(),
    };
    serde_json::to_value(compact).expect("CompactSnapshot serialization cannot fail")
}

#[allow(dead_code)]
pub(crate) fn compact_element(element: &ElementNode) -> Value {
    serde_json::to_value(CompactElementNode::from(element))
        .expect("CompactElementNode serialization cannot fail")
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
