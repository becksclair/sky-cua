use std::fmt::Write as _;

use anyhow::Result;
use serde_json::{Value, json};
use sky_cua_platform::model::{
    BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT, BrowserActionResponse, BrowserClaimTabResponse,
    BrowserEvalResponse, BrowserListTabsResponse, BrowserMoveMouseResponse,
    BrowserNavigateResponse, BrowserOpenResponse, BrowserScreenshotResponse,
    BrowserSnapshotResponse, BrowserStatusReport, BrowserTab, BrowserTargetKind, DiagnosticEntry,
    browser_diagnostic_is_error_code,
};

use super::args::BrowserTabTextFilter;
use crate::output_shapes::summary_text_field;

pub(crate) fn browser_status_summary(report: &BrowserStatusReport) -> String {
    let mut summary = String::from(if report.enabled {
        "Browser MCP tools are available."
    } else {
        "Browser MCP tools are unavailable."
    });
    if !report.available_targets.is_empty() {
        summary.push_str(" Targets: ");
        for (index, target) in report.available_targets.iter().enumerate() {
            if index > 0 {
                summary.push_str("; ");
            }
            let _ = write!(
                &mut summary,
                "{}={} ({})",
                browser_target_label(target.target),
                if target.available {
                    "available"
                } else {
                    "unavailable"
                },
                target.detail
            );
        }
        summary.push('.');
    }
    match report.tabs_known {
        Some(count) => {
            let _ = write!(&mut summary, " Tabs known: {count}.");
        }
        None => summary.push_str(" Tabs known: unknown."),
    }
    if let Some(diagnostic) = report.diagnostics.first() {
        let _ = write!(&mut summary, " Diagnostic: {}", diagnostic.message);
    }
    summary
}

#[cfg(test)]
pub(crate) fn browser_list_tabs_summary(
    response: &BrowserListTabsResponse,
    filter: &BrowserTabTextFilter,
) -> String {
    let matching_tab_indexes = browser_tab_match_indexes(response, filter);
    browser_list_tabs_summary_with_matches(response, filter, matching_tab_indexes.as_deref())
}

pub(crate) fn browser_list_tabs_summary_with_matches(
    response: &BrowserListTabsResponse,
    filter: &BrowserTabTextFilter,
    matching_tab_indexes: Option<&[usize]>,
) -> String {
    let target = response
        .target
        .map(browser_target_label)
        .unwrap_or(browser_target_label(BrowserTargetKind::UserChrome));
    let mut summary = if filter.is_empty() {
        format!(
            "Discovered {} browser tabs for {target}.",
            response.tabs.len()
        )
    } else {
        format!(
            "Discovered {} browser tabs for {target}; {} matched the text filters.",
            response.tabs.len(),
            matching_tab_indexes.map_or(response.tabs.len(), <[usize]>::len)
        )
    };
    append_browser_tab_matches(&mut summary, response, matching_tab_indexes, filter);
    if let Some(diagnostic) = response.diagnostics.first() {
        let _ = write!(&mut summary, " Diagnostic: {}", diagnostic.message);
    }
    summary
}

pub(crate) fn browser_tab_match_indexes(
    response: &BrowserListTabsResponse,
    filter: &BrowserTabTextFilter,
) -> Option<Vec<usize>> {
    if filter.is_empty() {
        return None;
    }

    let title_contains = filter.title_contains.as_deref().map(str::to_lowercase);
    let url_contains = filter.url_contains.as_deref().map(str::to_lowercase);
    Some(
        response
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                let title_matches = title_contains
                    .as_deref()
                    .is_none_or(|needle| browser_text_contains(tab.title.as_deref(), needle));
                let url_matches = url_contains
                    .as_deref()
                    .is_none_or(|needle| browser_text_contains(tab.url.as_deref(), needle));
                (title_matches && url_matches).then_some(index)
            })
            .collect(),
    )
}

fn browser_text_contains(value: Option<&str>, normalized_needle: &str) -> bool {
    value
        .map(|value| value.to_lowercase().contains(normalized_needle))
        .unwrap_or(false)
}

fn browser_matching_tab_count(
    response: &BrowserListTabsResponse,
    matching_tab_indexes: Option<&[usize]>,
) -> usize {
    matching_tab_indexes.map_or(response.tabs.len(), <[usize]>::len)
}

fn browser_tab_matches_is_empty(
    response: &BrowserListTabsResponse,
    matching_tab_indexes: Option<&[usize]>,
) -> bool {
    browser_matching_tab_count(response, matching_tab_indexes) == 0
}

fn filter_browser_tabs_by_indexes(
    tabs: Vec<BrowserTab>,
    matching_tab_indexes: Vec<usize>,
) -> Vec<BrowserTab> {
    let mut next_match = matching_tab_indexes.into_iter().peekable();
    tabs.into_iter()
        .enumerate()
        .filter_map(|(index, tab)| {
            if next_match.peek().copied() == Some(index) {
                next_match.next();
                Some(tab)
            } else {
                None
            }
        })
        .collect()
}

fn append_browser_tab_matches(
    summary: &mut String,
    response: &BrowserListTabsResponse,
    matching_tab_indexes: Option<&[usize]>,
    filter: &BrowserTabTextFilter,
) {
    if browser_tab_matches_is_empty(response, matching_tab_indexes) {
        if !filter.is_empty() {
            summary.push_str(" No matching tabs were found; try browser_open for a new controllable tab or loosen the filters.");
        }
        return;
    }

    let shown_limit = 12;
    let matching_tab_count = browser_matching_tab_count(response, matching_tab_indexes);
    let shown_count = matching_tab_count.min(shown_limit);
    if filter.is_empty() {
        let _ = write!(
            summary,
            " Showing first {shown_count} tab{}; pass url_contains or title_contains to narrow results.",
            if shown_count == 1 { "" } else { "s" }
        );
    } else {
        let _ = write!(
            summary,
            " Matching tab{}:",
            if shown_count == 1 { "" } else { "s" }
        );
    }
    match matching_tab_indexes {
        Some(indexes) => {
            for tab in indexes
                .iter()
                .take(shown_limit)
                .filter_map(|index| response.tabs.get(*index))
            {
                append_browser_tab_match(summary, tab);
            }
        }
        None => {
            for tab in response.tabs.iter().take(shown_limit) {
                append_browser_tab_match(summary, tab);
            }
        }
    }
    if matching_tab_count > shown_limit {
        let _ = write!(
            summary,
            " ... {} more not shown.",
            matching_tab_count - shown_limit
        );
    }
}

fn append_browser_tab_match(summary: &mut String, tab: &BrowserTab) {
    let title = tab.title.as_deref().unwrap_or("<untitled>");
    let url = tab.url.as_deref().unwrap_or("<unknown-url>");
    let _ = write!(
        summary,
        " [{}] title=\"{}\" url=\"{}\" active={}",
        tab.tab_id,
        summary_text_field(title, 80),
        summary_text_field(url, 160),
        tab.active
    );
}

pub(crate) fn browser_list_tabs_structured_response(
    mut response: BrowserListTabsResponse,
    matching_tab_indexes: Option<Vec<usize>>,
) -> BrowserListTabsResponse {
    if let Some(matching_tab_indexes) = matching_tab_indexes {
        response.tabs = filter_browser_tabs_by_indexes(response.tabs, matching_tab_indexes);
    }
    response
}

pub(crate) fn browser_list_tabs_is_error(response: &BrowserListTabsResponse) -> bool {
    response.tabs.is_empty()
        && response
            .diagnostics
            .iter()
            .any(|diagnostic| browser_diagnostic_is_error_code(&diagnostic.code))
}

pub(crate) fn browser_open_summary(response: &BrowserOpenResponse) -> String {
    let target = browser_target_label(response.target);
    let is_partial = response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "BrowserOpenPartial");
    let mut summary = match (&response.tab, is_partial) {
        (Some(tab), true) => format!(
            "Created browser tab {} for {target}, but browser_open did not complete.",
            tab.tab_id
        ),
        (Some(tab), false) => format!("Opened browser tab {} for {target}.", tab.tab_id),
        (None, _) => format!("Could not open browser tab for {target}."),
    };
    if let Some(diagnostic) = response.diagnostics.first() {
        let _ = write!(&mut summary, " Diagnostic: {}", diagnostic.message);
    }
    summary
}

pub(crate) fn browser_open_is_error(response: &BrowserOpenResponse) -> bool {
    response.tab.is_none() || browser_diagnostics_are_error(&response.diagnostics)
}

pub(crate) fn browser_claim_tab_summary(response: &BrowserClaimTabResponse) -> String {
    let target = browser_target_label(response.target);
    let mut summary = match &response.tab {
        Some(tab) => format!("Claimed browser tab {} for {target}.", tab.tab_id),
        None => format!("Could not claim browser tab for {target}."),
    };
    if let Some(diagnostic) = response.diagnostics.first() {
        let _ = write!(&mut summary, " Diagnostic: {}", diagnostic.message);
    }
    summary
}

pub(crate) fn browser_claim_tab_is_error(response: &BrowserClaimTabResponse) -> bool {
    response.tab.is_none() || browser_diagnostics_are_error(&response.diagnostics)
}

pub(crate) fn browser_move_mouse_summary(response: &BrowserMoveMouseResponse) -> String {
    let target = browser_target_label(response.target);
    let mut summary = if response.diagnostics.is_empty() {
        format!(
            "Moved browser cursor in tab {} for {target} to browser screenshot pixel ({}, {}).",
            response.tab_id, response.x, response.y
        )
    } else {
        format!(
            "Could not move browser cursor in tab {} for {target}.",
            response.tab_id
        )
    };
    if let Some(diagnostic) = response.diagnostics.first() {
        let _ = write!(&mut summary, " Diagnostic: {}", diagnostic.message);
    }
    summary
}

pub(crate) fn browser_move_mouse_is_error(response: &BrowserMoveMouseResponse) -> bool {
    browser_diagnostics_are_error(&response.diagnostics)
}

pub(crate) fn browser_navigate_result(response: BrowserNavigateResponse) -> Result<Value> {
    let is_error = browser_diagnostics_are_error(&response.diagnostics);
    let mut text = if is_error {
        format!("Could not navigate browser tab {}.", response.tab_id)
    } else {
        format!(
            "Navigated browser tab {} to {}.",
            response.tab_id, response.url
        )
    };
    append_first_diagnostic(&mut text, &response.diagnostics);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

pub(crate) fn browser_snapshot_result(response: BrowserSnapshotResponse) -> Result<Value> {
    let is_error =
        response.snapshot.is_none() || browser_diagnostics_are_error(&response.diagnostics);
    let mut text = if is_error {
        format!("Could not snapshot browser tab {}.", response.tab_id)
    } else {
        browser_snapshot_summary(&response)
    };
    append_first_diagnostic(&mut text, &response.diagnostics);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

pub(crate) fn browser_snapshot_structured_response(
    mut response: BrowserSnapshotResponse,
    element_offset: Option<usize>,
    element_limit: Option<usize>,
    element_query: Option<&str>,
    text_limit: usize,
) -> BrowserSnapshotResponse {
    let Some(snapshot) = response.snapshot.as_mut() else {
        return response;
    };
    let Some(snapshot_object) = snapshot.as_object_mut() else {
        return response;
    };
    limit_browser_snapshot_text(snapshot_object, text_limit);
    // Move the array out rather than cloning it: the capture can carry up to
    // 5000 elements and `response` is owned, so the clone was pure waste.
    let elements = match snapshot_object.get_mut("elements") {
        Some(Value::Array(elements)) => std::mem::take(elements),
        _ => return response,
    };

    let query = element_query.map(str::to_lowercase);
    let filtered = elements
        .into_iter()
        .filter(|element| {
            query
                .as_deref()
                .is_none_or(|query| browser_snapshot_element_search_text(element).contains(query))
        })
        .skip(element_offset.unwrap_or(0))
        .take(element_limit.unwrap_or(BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT))
        .collect::<Vec<_>>();
    snapshot_object.insert("elements".to_string(), Value::Array(filtered));
    response
}

fn limit_browser_snapshot_text(snapshot_object: &mut serde_json::Map<String, Value>, limit: usize) {
    let Some(text) = snapshot_object.get("text").and_then(Value::as_str) else {
        return;
    };
    let original_len = match snapshot_object.get("textCharCount") {
        Some(Value::Null) => None,
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .or_else(|| Some(text.chars().count())),
        None => Some(text.chars().count()),
    };
    let service_truncated = snapshot_object
        .get("textTruncated")
        .and_then(Value::as_bool);
    let truncated = original_len
        .map(|original_len| original_len > limit)
        .or(service_truncated);
    if truncated == Some(true) {
        let limited = text.chars().take(limit).collect::<String>();
        snapshot_object.insert("text".to_string(), Value::String(limited));
    }
    snapshot_object.insert("textCharCount".to_string(), json!(original_len));
    snapshot_object.insert("textLimit".to_string(), json!(limit));
    snapshot_object.insert("textTruncated".to_string(), json!(truncated));
}

pub(crate) fn browser_snapshot_summary(response: &BrowserSnapshotResponse) -> String {
    let mut text = format!("Captured browser snapshot for tab {}.", response.tab_id);
    if let Some(title) = response.title.as_deref().filter(|title| !title.is_empty()) {
        let _ = write!(&mut text, " Title: \"{}\".", summary_text_field(title, 160));
    }
    if let Some(url) = response.url.as_deref().filter(|url| !url.is_empty()) {
        let _ = write!(&mut text, " URL: {}.", summary_text_field(url, 240));
    }
    if let Some(snapshot) = response.snapshot.as_ref() {
        append_browser_snapshot_viewport(&mut text, snapshot);
        append_browser_snapshot_visible_text(&mut text, snapshot);
        append_browser_snapshot_elements(&mut text, snapshot);
    }
    text
}

fn append_browser_snapshot_viewport(text: &mut String, snapshot: &Value) {
    let Some(viewport) = snapshot.get("viewport") else {
        return;
    };
    let width = viewport.get("width").and_then(Value::as_f64);
    let height = viewport.get("height").and_then(Value::as_f64);
    let dpr = viewport.get("devicePixelRatio").and_then(Value::as_f64);
    if width.is_some() || height.is_some() || dpr.is_some() {
        let _ = write!(
            text,
            " Viewport: width={} height={} devicePixelRatio={}.",
            width
                .map(format_browser_number)
                .unwrap_or_else(|| "unknown".to_string()),
            height
                .map(format_browser_number)
                .unwrap_or_else(|| "unknown".to_string()),
            dpr.map(format_browser_number)
                .unwrap_or_else(|| "unknown".to_string())
        );
    }
}

fn append_browser_snapshot_visible_text(text: &mut String, snapshot: &Value) {
    let Some(page_text) = snapshot
        .get("text")
        .and_then(Value::as_str)
        .map(|value| summary_text_field(value, 800))
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let _ = write!(text, " Visible text: \"{page_text}\".");
}

fn append_browser_snapshot_elements(text: &mut String, snapshot: &Value) {
    let Some(elements) = snapshot.get("elements").and_then(Value::as_array) else {
        return;
    };
    if elements.is_empty() {
        text.push_str(" Actionable elements: none detected.");
        return;
    }
    let shown_limit = 12;
    let shown_count = elements.len().min(shown_limit);
    let _ = write!(
        text,
        " Actionable elements (showing {shown_count}/{}):",
        elements.len()
    );
    for element in elements.iter().take(shown_limit) {
        append_browser_snapshot_element(text, element);
    }
    if elements.len() > shown_limit {
        let _ = write!(
            text,
            " ... {} more not shown.",
            elements.len() - shown_limit
        );
    }
}

fn append_browser_snapshot_element(text: &mut String, element: &Value) {
    let index = element
        .get("index")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string());
    let tag = element.get("tag").and_then(Value::as_str).unwrap_or("?");
    let role = element.get("role").and_then(Value::as_str).unwrap_or("");
    let name = element
        .get("name")
        .and_then(Value::as_str)
        .map(|value| summary_text_field(value, 120))
        .unwrap_or_default();
    let href = element
        .get("href")
        .and_then(Value::as_str)
        .map(|value| summary_text_field(value, 160));
    let bounds = element.get("bounds").map(browser_bounds_summary);
    let _ = write!(text, " [{index}] tag={tag}");
    if !role.is_empty() {
        let _ = write!(text, " role={role}");
    }
    if !name.is_empty() {
        let _ = write!(text, " name=\"{name}\"");
    }
    if let Some(href) = href.filter(|value| !value.is_empty()) {
        let _ = write!(text, " href=\"{href}\"");
    }
    if let Some(bounds) = bounds {
        let _ = write!(text, " bounds={bounds}");
    }
}

fn browser_snapshot_element_search_text(element: &Value) -> String {
    let mut haystack = String::new();
    for field in ["tag", "role", "name", "href", "value"] {
        if let Some(value) = element.get(field).and_then(Value::as_str) {
            haystack.push_str(value);
            haystack.push('\n');
        }
    }
    haystack.to_lowercase()
}

fn browser_bounds_summary(bounds: &Value) -> String {
    let number = |name: &str| {
        bounds
            .get(name)
            .and_then(Value::as_f64)
            .map(format_browser_number)
            .unwrap_or_else(|| "?".to_string())
    };
    format!(
        "x:{} y:{} w:{} h:{}",
        number("x"),
        number("y"),
        number("width"),
        number("height")
    )
}

fn format_browser_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

pub(crate) fn browser_screenshot_result(
    mut response: BrowserScreenshotResponse,
    can_receive_images: bool,
) -> Result<Value> {
    let has_capture_reference = response.screenshot_path.is_some()
        || (can_receive_images && !response.data_base64.is_empty());
    let is_error = !has_capture_reference || browser_diagnostics_are_error(&response.diagnostics);
    let mut text = if is_error {
        format!(
            "Could not capture browser screenshot for tab {}.",
            response.tab_id
        )
    } else {
        let mut text = format!(
            "Captured {} browser screenshot of the visible viewport for tab {}",
            response.mime_type, response.tab_id
        );
        if let (Some(width), Some(height)) = (response.width, response.height) {
            let _ = write!(text, " ({width}x{height} pixels)");
        }
        text.push('.');
        if let Some(path) = response.screenshot_path.as_deref() {
            let _ = write!(text, " Saved to {path}.");
        }
        text.push_str(
            " Image pixels, browser_snapshot element bounds, and browser_click/browser_move_mouse/browser_scroll coordinates all share the same CSS-pixel space.",
        );
        if can_receive_images && !response.data_base64.is_empty() {
            text.push_str(" The image is attached to this result.");
        } else if can_receive_images {
            text.push_str(" Image data was omitted; read screenshot_path if needed.");
        } else {
            text.push_str(
                " Image data was omitted because this session's model does not support image input; use browser_snapshot for page details.",
            );
        }
        text
    };
    append_first_diagnostic(&mut text, &response.diagnostics);

    let image_data = if !is_error && can_receive_images {
        std::mem::take(&mut response.data_base64)
    } else {
        String::new()
    };
    let mut content = vec![json!({"type": "text", "text": text})];
    if !image_data.is_empty() {
        content.push(json!({
            "type": "image",
            "data": image_data,
            "mimeType": response.mime_type,
        }));
    }

    // The image travels as a content block (or on disk at screenshot_path);
    // repeating the base64 payload in structuredContent would only bloat
    // host context windows.
    let mut structured = serde_json::to_value(&response)?;
    if let Some(map) = structured.as_object_mut() {
        map.remove("data_base64");
    }

    Ok(json!({
        "content": content,
        "structuredContent": structured,
        "isError": is_error
    }))
}

pub(crate) fn browser_action_result(response: BrowserActionResponse) -> Result<Value> {
    let is_error = browser_diagnostics_are_error(&response.diagnostics);
    let mut text = if is_error {
        format!(
            "Could not perform browser action {} in tab {}.",
            response.action, response.tab_id
        )
    } else {
        format!(
            "Performed browser action {} in tab {}.",
            response.action, response.tab_id
        )
    };
    append_first_diagnostic(&mut text, &response.diagnostics);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

pub(crate) fn browser_eval_result(response: BrowserEvalResponse) -> Result<Value> {
    let is_error = browser_diagnostics_are_error(&response.diagnostics);
    let mut text = if is_error {
        format!(
            "Could not evaluate JavaScript in browser tab {}.",
            response.tab_id
        )
    } else {
        format!("Evaluated JavaScript in browser tab {}.", response.tab_id)
    };
    append_first_diagnostic(&mut text, &response.diagnostics);
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": is_error
    }))
}

fn browser_diagnostics_are_error(diagnostics: &[DiagnosticEntry]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| browser_diagnostic_is_error_code(&diagnostic.code))
}

fn append_first_diagnostic(summary: &mut String, diagnostics: &[DiagnosticEntry]) {
    if let Some(diagnostic) = diagnostics.first() {
        let _ = write!(summary, " Diagnostic: {}", diagnostic.message);
    }
}

fn browser_target_label(target: BrowserTargetKind) -> &'static str {
    match target {
        BrowserTargetKind::UserChrome => "user_chrome",
    }
}
