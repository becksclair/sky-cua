use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sky_cua_platform::model::{
    AccessibilitySetupReport, AgentCursorState, AppStateSnapshot, CaptureInfo, DiagnosticEntry,
    DoctorReport, ElementNode, ElementNumericValueReadback, ElementTextReadback, EnvironmentInfo,
    FocusedApp, HeuristicMatch, RectF, ToolCapabilities, WindowTargetingSetupReport,
};
use std::fmt::Write as _;

use crate::app_state::{APP_STATE_DEFAULT_ELEMENT_LIMIT, AppStateElementOptions};

const LIST_APPS_INFORMATIONAL_DIAGNOSTIC_CODES: &[&str] = &["SessionEnvRepaired"];
const SNAPSHOT_TEXT_ELEMENT_LIMIT: usize = 120;
const SNAPSHOT_TEXT_FIELD_LIMIT: usize = 240;

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
    environment: &'a EnvironmentInfo,
    focused_app: &'a Option<FocusedApp>,
    capture: &'a Option<CaptureInfo>,
    agent_cursor: &'a Option<AgentCursorState>,
    diagnostics: &'a Vec<DiagnosticEntry>,
    app_guidance: &'a Option<HeuristicMatch>,
    doctor_report: &'a Option<DoctorReport>,
    elements: &'a [CompactElementNode<'a>],
    element_count: usize,
    filtered_element_count: usize,
    elements_returned: usize,
    element_offset: usize,
    element_limit: Option<usize>,
    element_query: Option<&'a str>,
}

#[derive(Serialize)]
struct ProjectedFullSnapshot<'a> {
    snapshot_id: &'a str,
    created_at: &'a DateTime<Utc>,
    environment: &'a EnvironmentInfo,
    capabilities: &'a ToolCapabilities,
    focused_app: &'a Option<FocusedApp>,
    capture: &'a Option<CaptureInfo>,
    elements: &'a [&'a ElementNode],
    diagnostics: &'a [DiagnosticEntry],
    app_guidance: &'a Option<HeuristicMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doctor_report: Option<&'a DoctorReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_cursor: Option<&'a AgentCursorState>,
}

pub(crate) fn compact_snapshot(snapshot: &AppStateSnapshot) -> Value {
    let options = AppStateElementOptions::default();
    let selection = select_app_state_elements(snapshot, &options, None);
    compact_snapshot_with_element_selection(snapshot, &selection)
}

pub(crate) fn compact_snapshot_with_element_selection(
    snapshot: &AppStateSnapshot,
    selection: &AppStateElementSelection<'_>,
) -> Value {
    let compact_elements: Vec<CompactElementNode> = selection
        .elements
        .iter()
        .copied()
        .map(CompactElementNode::from)
        .collect();
    let compact = CompactSnapshot {
        detail: "compact",
        snapshot_id: &snapshot.snapshot_id,
        created_at: &snapshot.created_at,
        environment: &snapshot.environment,
        focused_app: &snapshot.focused_app,
        capture: &snapshot.capture,
        agent_cursor: &snapshot.agent_cursor,
        diagnostics: &snapshot.diagnostics,
        app_guidance: &snapshot.app_guidance,
        doctor_report: &snapshot.doctor_report,
        elements: &compact_elements,
        element_count: snapshot.elements.len(),
        filtered_element_count: selection.filtered_count,
        elements_returned: compact_elements.len(),
        element_offset: selection.offset,
        element_limit: selection.limit,
        element_query: selection.query,
    };
    serde_json::to_value(compact).expect("CompactSnapshot serialization cannot fail")
}

pub(crate) fn full_snapshot_with_element_selection(
    snapshot: &AppStateSnapshot,
    selection: &AppStateElementSelection<'_>,
) -> Result<Value, serde_json::Error> {
    let projected_snapshot = ProjectedFullSnapshot {
        snapshot_id: &snapshot.snapshot_id,
        created_at: &snapshot.created_at,
        environment: &snapshot.environment,
        capabilities: &snapshot.capabilities,
        focused_app: &snapshot.focused_app,
        capture: &snapshot.capture,
        elements: &selection.elements,
        diagnostics: &snapshot.diagnostics,
        app_guidance: &snapshot.app_guidance,
        doctor_report: snapshot.doctor_report.as_ref(),
        agent_cursor: snapshot.agent_cursor.as_ref(),
    };
    let mut value = serde_json::to_value(&projected_snapshot)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "element_count".to_string(),
            Value::from(selection.total_count),
        );
        object.insert(
            "filtered_element_count".to_string(),
            Value::from(selection.filtered_count),
        );
        object.insert(
            "elements_returned".to_string(),
            Value::from(selection.elements.len()),
        );
        object.insert("element_offset".to_string(), Value::from(selection.offset));
        object.insert(
            "element_limit".to_string(),
            serde_json::to_value(selection.limit)?,
        );
        object.insert(
            "element_query".to_string(),
            serde_json::to_value(selection.query)?,
        );
    }
    Ok(value)
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

#[cfg(test)]
pub(crate) fn snapshot_text_content(snapshot: &AppStateSnapshot) -> String {
    snapshot_text_content_with_elements(snapshot, true)
}

pub(crate) fn compact_snapshot_text_content(snapshot: &AppStateSnapshot) -> String {
    snapshot_text_content_with_elements(snapshot, false)
}

pub(crate) fn snapshot_text_content_with_element_options(
    snapshot: &AppStateSnapshot,
    include_elements: bool,
    options: &AppStateElementOptions,
) -> String {
    snapshot_text_content_with_elements_and_options(snapshot, include_elements, Some(options))
}

fn snapshot_text_content_with_elements(
    snapshot: &AppStateSnapshot,
    include_elements: bool,
) -> String {
    snapshot_text_content_with_elements_and_options(snapshot, include_elements, None)
}

fn snapshot_text_content_with_elements_and_options(
    snapshot: &AppStateSnapshot,
    include_elements: bool,
    options: Option<&AppStateElementOptions>,
) -> String {
    let mut text = snapshot_text_header(snapshot);

    if snapshot.elements.is_empty() {
        text.push_str("\nElements: none");
        return text;
    }

    if !include_elements {
        let Some(options) = options else {
            let _ = write!(&mut text, "\nElements: {} total.", snapshot.elements.len());
            return text;
        };
        let selection =
            select_app_state_elements(snapshot, options, Some(APP_STATE_DEFAULT_ELEMENT_LIMIT));
        append_element_view_summary(&mut text, &selection);
        return text;
    }

    let Some(options) = options else {
        let _ = write!(&mut text, "\nElements ({}):", snapshot.elements.len());
        for element in snapshot.elements.iter().take(SNAPSHOT_TEXT_ELEMENT_LIMIT) {
            append_element_text_line(&mut text, element);
        }
        if snapshot.elements.len() > SNAPSHOT_TEXT_ELEMENT_LIMIT {
            let omitted = snapshot.elements.len() - SNAPSHOT_TEXT_ELEMENT_LIMIT;
            let _ = write!(&mut text, "\n- ... {omitted} more elements omitted");
        }
        return text;
    };
    let text_options = text_element_options(options);
    let selection = select_app_state_elements(snapshot, &text_options, None);
    append_elements_to_text(&mut text, &selection);

    text
}

pub(crate) fn snapshot_text_content_with_element_selection(
    snapshot: &AppStateSnapshot,
    include_elements: bool,
    selection: &AppStateElementSelection<'_>,
) -> String {
    let mut text = snapshot_text_header(snapshot);

    if snapshot.elements.is_empty() {
        text.push_str("\nElements: none");
        return text;
    }

    if include_elements {
        append_elements_to_text(&mut text, selection);
    } else {
        append_element_view_summary(&mut text, selection);
    }

    text
}

fn snapshot_text_header(snapshot: &AppStateSnapshot) -> String {
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

    text
}

fn append_elements_to_text(out: &mut String, selection: &AppStateElementSelection<'_>) {
    append_element_view_summary(out, selection);
    if selection.elements.is_empty() {
        return;
    }
    out.push(':');
    for element in &selection.elements {
        append_element_text_line(out, element);
    }
    let returned_end = selection
        .offset
        .saturating_add(selection.elements.len())
        .min(selection.filtered_count);
    if returned_end < selection.filtered_count {
        let omitted = selection.filtered_count - returned_end;
        let _ = write!(out, "\n- ... {omitted} more elements omitted");
    }
}

pub(crate) fn text_app_state_element_selection<'a>(
    selection: &AppStateElementSelection<'a>,
    options: &AppStateElementOptions,
) -> AppStateElementSelection<'a> {
    let limit = options
        .element_limit
        .unwrap_or(SNAPSHOT_TEXT_ELEMENT_LIMIT)
        .min(SNAPSHOT_TEXT_ELEMENT_LIMIT);
    AppStateElementSelection {
        elements: selection.elements.iter().take(limit).copied().collect(),
        total_count: selection.total_count,
        filtered_count: selection.filtered_count,
        offset: selection.offset,
        limit: Some(limit),
        query: selection.query,
    }
}

pub(crate) struct AppStateElementSelection<'a> {
    elements: Vec<&'a ElementNode>,
    total_count: usize,
    filtered_count: usize,
    offset: usize,
    limit: Option<usize>,
    query: Option<&'a str>,
}

pub(crate) fn select_app_state_elements<'a>(
    snapshot: &'a AppStateSnapshot,
    options: &'a AppStateElementOptions,
    default_limit: Option<usize>,
) -> AppStateElementSelection<'a> {
    let query = options.element_query.as_deref();
    let limit = options.element_limit.or(default_limit);
    if query.is_none() {
        let elements = match limit {
            Some(limit) => snapshot
                .elements
                .iter()
                .skip(options.element_offset)
                .take(limit)
                .collect(),
            None => snapshot
                .elements
                .iter()
                .skip(options.element_offset)
                .collect(),
        };
        return AppStateElementSelection {
            elements,
            total_count: snapshot.elements.len(),
            filtered_count: snapshot.elements.len(),
            offset: options.element_offset,
            limit,
            query,
        };
    }

    let normalized_query = query.map(str::to_lowercase);
    let mut elements = Vec::new();
    let mut filtered_count = 0;
    for element in &snapshot.elements {
        let matches_query = normalized_query
            .as_deref()
            .is_none_or(|query| element_matches_query(element, query));
        if !matches_query {
            continue;
        }
        let filtered_index = filtered_count;
        filtered_count += 1;
        if filtered_index < options.element_offset {
            continue;
        }
        if limit.is_none_or(|limit| elements.len() < limit) {
            elements.push(element);
        }
    }
    AppStateElementSelection {
        elements,
        total_count: snapshot.elements.len(),
        filtered_count,
        offset: options.element_offset,
        limit,
        query,
    }
}

fn element_matches_query(element: &ElementNode, query: &str) -> bool {
    string_matches_query(&element.role, query)
        || option_matches_query(element.name.as_deref(), query)
        || option_matches_query(element.description.as_deref(), query)
        || option_matches_query(element.value.as_deref(), query)
        || element
            .text
            .as_ref()
            .is_some_and(|text| option_matches_query(text.content.as_deref(), query))
        || element
            .numeric_value
            .as_ref()
            .is_some_and(|numeric| option_matches_query(numeric.text.as_deref(), query))
        || element
            .state_flags
            .iter()
            .any(|state| string_matches_query(state, query))
        || element
            .semantic_actions
            .iter()
            .any(|action| string_matches_query(action, query))
}

fn option_matches_query(value: Option<&str>, query: &str) -> bool {
    value.is_some_and(|value| string_matches_query(value, query))
}

fn string_matches_query(value: &str, query: &str) -> bool {
    if value.is_ascii() && query.is_ascii() {
        let value = value.as_bytes();
        let query = query.as_bytes();
        return query.is_empty()
            || value
                .windows(query.len())
                .any(|window| window.eq_ignore_ascii_case(query));
    }
    value.to_lowercase().contains(query)
}

fn text_element_options(options: &AppStateElementOptions) -> AppStateElementOptions {
    let element_limit = Some(
        options
            .element_limit
            .unwrap_or(SNAPSHOT_TEXT_ELEMENT_LIMIT)
            .min(SNAPSHOT_TEXT_ELEMENT_LIMIT),
    );
    AppStateElementOptions {
        element_offset: options.element_offset,
        element_limit,
        element_query: options.element_query.clone(),
    }
}

fn append_element_view_summary(out: &mut String, selection: &AppStateElementSelection<'_>) {
    let _ = write!(
        out,
        "\nElements: {} returned of {} filtered, {} total",
        selection.elements.len(),
        selection.filtered_count,
        selection.total_count
    );
    if let Some(query) = selection.query {
        let _ = write!(out, " query={query:?}");
    }
    if selection.offset > 0 {
        let _ = write!(out, " offset={}", selection.offset);
    }
    if let Some(limit) = selection.limit {
        let _ = write!(out, " limit={limit}");
    }
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
            | "CaptureFrameBlank"
            | "DisplayTopologyInferred"
            | "DisplayTopologyUnavailable" => {
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

#[cfg(test)]
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
    use super::{compact_text_field, string_matches_query};

    #[test]
    fn compact_text_field_preserves_normalized_truncation_shape() {
        assert_eq!(compact_text_field(" one \n two\tthree ", 9), "one two t...");
        assert_eq!(compact_text_field("alpha beta", 6), "alpha ...");
        assert_eq!(compact_text_field("alpha beta", 10), "alpha beta");
    }

    #[test]
    fn string_matches_query_avoids_allocation_for_ascii_case_insensitive_matches() {
        assert!(string_matches_query("Search Submit", "search"));
        assert!(string_matches_query("Search Submit", "SUBMIT"));
        assert!(!string_matches_query("Search Submit", "cancel"));
    }

    #[test]
    fn string_matches_query_preserves_unicode_case_folding_fallback() {
        assert!(string_matches_query("İstanbul", "i"));
        assert!(string_matches_query("Straße", "straße"));
    }
}
