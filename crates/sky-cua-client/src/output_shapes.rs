use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sky_cua_platform::model::{
    AccessibilitySetupReport, AgentCursorState, AppStateSnapshot, CaptureInfo, DiagnosticEntry,
    DoctorReport, ElementNode, ElementNumericValueReadback, ElementTextReadback, FocusedApp,
    HeuristicMatch, RectF, WindowTargetingSetupReport,
};
use std::fmt::Write as _;

const LIST_APPS_INFORMATIONAL_DIAGNOSTIC_CODES: &[&str] = &["SessionEnvRepaired"];
const SNAPSHOT_TEXT_ELEMENT_LIMIT: usize = 120;
const SNAPSHOT_TEXT_FIELD_LIMIT: usize = 240;

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

pub(crate) fn snapshot_summary(snapshot: &AppStateSnapshot) -> String {
    let app_name = snapshot
        .focused_app
        .as_ref()
        .map(|app| app.name.as_str())
        .unwrap_or("no focused app");
    let mut summary = String::with_capacity(128);
    let _ = write!(
        &mut summary,
        "Snapshot {} captured {} elements for {}.",
        snapshot.snapshot_id,
        snapshot.elements.len(),
        app_name
    );
    if let Some(diag) = portal_approval_pending_diagnostic(&snapshot.diagnostics) {
        summary.push(' ');
        summary.push_str(&portal_approval_summary(diag.message.as_str()));
    }
    if let Some(summary_suffix) = informational_runtime_summary(&snapshot.diagnostics) {
        summary.push(' ');
        summary.push_str(&summary_suffix);
    }
    summary
}

pub(crate) fn snapshot_text_content(snapshot: &AppStateSnapshot) -> String {
    snapshot_text_content_with_elements(snapshot, true)
}

pub(crate) fn compact_snapshot_text_content(snapshot: &AppStateSnapshot) -> String {
    snapshot_text_content_with_elements(snapshot, false)
}

fn snapshot_text_content_with_elements(
    snapshot: &AppStateSnapshot,
    include_elements: bool,
) -> String {
    let mut text = snapshot_summary(snapshot);

    if let Some(app) = &snapshot.focused_app {
        let _ = write!(
            &mut text,
            "\nFocused app: name={} app_id={}",
            app.name, app.app_id
        );
        append_text_field(&mut text, "desktop_file_id", app.desktop_file_id.as_deref());
        append_text_field(&mut text, "window_title", app.window_title.as_deref());
        if let Some(pid) = app.pid {
            let _ = write!(&mut text, " pid={pid}");
        }
    }

    if let Some(capture) = &snapshot.capture {
        let _ = write!(
            &mut text,
            "\nCapture: backend={:?} image_backend={:?}",
            capture.backend, capture.image_backend
        );
        if let Some(size) = &capture.pixel_size {
            let _ = write!(&mut text, " pixel_size={}x{}", size.width, size.height);
        }
        append_text_field(
            &mut text,
            "screenshot_path",
            capture.screenshot_path.as_deref(),
        );
    }

    if !snapshot.diagnostics.is_empty() {
        text.push_str("\nDiagnostics:");
        for diagnostic in &snapshot.diagnostics {
            let _ = write!(
                &mut text,
                "\n- [{}] {}",
                diagnostic.code, diagnostic.message
            );
            append_text_field(&mut text, "details", diagnostic.details.as_deref());
        }
    }

    if snapshot.elements.is_empty() {
        text.push_str("\nElements: none");
        return text;
    }

    if !include_elements {
        let _ = write!(&mut text, "\nElements: {} total.", snapshot.elements.len());
        return text;
    }

    let _ = write!(&mut text, "\nElements ({}):", snapshot.elements.len());
    for element in snapshot.elements.iter().take(SNAPSHOT_TEXT_ELEMENT_LIMIT) {
        append_element_text_line(&mut text, element);
    }
    if snapshot.elements.len() > SNAPSHOT_TEXT_ELEMENT_LIMIT {
        let omitted = snapshot.elements.len() - SNAPSHOT_TEXT_ELEMENT_LIMIT;
        let _ = write!(&mut text, "\n- ... {omitted} more elements omitted");
    }

    text
}

fn append_element_text_line(out: &mut String, element: &ElementNode) {
    let _ = write!(out, "\n- [{}] role={}", element.element_index, element.role);
    if let Some(parent_index) = element.parent_index {
        let _ = write!(out, " parent={parent_index}");
    }
    append_text_field(out, "name", element.name.as_deref());
    append_text_field(out, "description", element.description.as_deref());
    append_text_field(out, "value", element.value.as_deref());
    if let Some(text) = &element.text {
        append_text_field(out, "text", text.content.as_deref());
        if text.content.is_none() && text.character_count > 0 {
            let _ = write!(out, " text_chars={}", text.character_count);
        }
        if text.content_suppressed {
            out.push_str(" text_suppressed=true");
        }
        if text.truncated {
            out.push_str(" text_truncated=true");
        }
    }
    if let Some(numeric) = &element.numeric_value {
        let _ = write!(
            out,
            " numeric_value={:.3} range={:.3}..{:.3}",
            numeric.current, numeric.minimum, numeric.maximum
        );
        append_text_field(out, "numeric_text", numeric.text.as_deref());
    }
    if element.supports_editable_text {
        out.push_str(" editable=true");
    }
    append_csv_field(out, "states", &element.state_flags);
    append_csv_field(out, "actions", &element.semantic_actions);
    if let Some(bounds) = &element.bounds {
        append_bounds(out, bounds);
    }
    append_text_field(out, "backend_ref", element.backend_ref.as_deref());
}

fn append_text_field(out: &mut String, label: &str, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let value = compact_text_field(value, SNAPSHOT_TEXT_FIELD_LIMIT);
    let _ = write!(out, " {label}={value:?}");
}

fn append_csv_field(out: &mut String, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let _ = write!(out, " {label}={}", values.join(","));
}

fn append_bounds(out: &mut String, bounds: &RectF) {
    let _ = write!(
        out,
        " bounds=({:.1},{:.1} {:.1}x{:.1} {:?})",
        bounds.x, bounds.y, bounds.width, bounds.height, bounds.space
    );
}

pub(crate) fn compact_text_field(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut chars = 0;
    let mut truncated = false;

    'parts: for part in value.split_whitespace() {
        if !out.is_empty() {
            if chars == max_chars {
                truncated = true;
                break;
            }
            out.push(' ');
            chars += 1;
        }
        for ch in part.chars() {
            if chars == max_chars {
                truncated = true;
                break 'parts;
            }
            out.push(ch);
            chars += 1;
        }
    }

    if truncated {
        out.push_str("...");
    }
    out
}

fn portal_approval_pending_diagnostic(diagnostics: &[DiagnosticEntry]) -> Option<&DiagnosticEntry> {
    diagnostics
        .iter()
        .find(|diag| diag.code == "PortalApprovalPending")
}

pub(crate) fn portal_approval_summary(message: &str) -> String {
    format!("{message} Approve the KDE portal dialog for screen control, then retry the request.")
}

pub(crate) fn informational_runtime_summary(diagnostics: &[DiagnosticEntry]) -> Option<String> {
    let mut parts = Vec::new();
    for diagnostic in diagnostics {
        match diagnostic.code.as_str() {
            "PortalSessionStarted" | "PortalSessionRestored" => {
                parts.push(diagnostic.message.clone());
            }
            "PortalSessionRestoreMiss"
            | "PortalSessionRebuilt"
            | "PortalSessionTokenRotated"
            | "CaptureBackendDowngraded"
            | "CaptureFrameBlank" => {
                parts.push(match diagnostic.details.as_ref() {
                    Some(details) => {
                        format!("{} Details: {}", diagnostic.message, details)
                    }
                    None => diagnostic.message.clone(),
                });
            }
            _ => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
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

#[cfg(test)]
mod tests {
    use super::compact_text_field;

    #[test]
    fn compact_text_field_preserves_normalized_truncation_shape() {
        assert_eq!(compact_text_field(" one \n two\tthree ", 9), "one two t...");
        assert_eq!(compact_text_field("alpha beta", 6), "alpha ...");
        assert_eq!(compact_text_field("alpha beta", 10), "alpha beta");
    }
}
