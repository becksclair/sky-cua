use serde_json::Value;
use sky_cua_platform::model::{BrowserTab, BrowserTargetKind, DiagnosticEntry};

use super::diagnostics::malformed_list_tabs_response_diagnostic;

pub(super) fn parse_tabs(
    result: Option<&Value>,
    target: Option<BrowserTargetKind>,
) -> Vec<BrowserTab> {
    let tab_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let tabs = match result {
        Some(Value::Array(tabs)) => tabs.as_slice(),
        Some(Value::Object(object)) => object
            .get("tabs")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        _ => &[],
    };

    tabs.iter()
        .filter_map(|tab| tab_from_value(tab, tab_target))
        .collect()
}

pub(super) fn parse_list_tabs_response(
    result: Option<&Value>,
    target: Option<BrowserTargetKind>,
) -> Result<Vec<BrowserTab>, DiagnosticEntry> {
    let tab_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let tabs = match result {
        Some(Value::Array(tabs)) => tabs.as_slice(),
        Some(Value::Object(object)) => object
            .get("tabs")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .ok_or_else(|| malformed_list_tabs_response_diagnostic(result))?,
        _ => return Err(malformed_list_tabs_response_diagnostic(result)),
    };

    Ok(tabs
        .iter()
        .filter_map(|tab| tab_from_value(tab, tab_target))
        .collect())
}

pub(super) fn parse_single_tab(
    result: Option<&Value>,
    target: BrowserTargetKind,
) -> Option<BrowserTab> {
    let tab = result?;
    if matches!(tab, Value::Array(_)) || tab.get("tabs").is_some() {
        return parse_tabs(Some(tab), Some(target)).into_iter().next();
    }

    tab_from_value(tab, target)
}

fn tab_from_value(tab: &Value, target: BrowserTargetKind) -> Option<BrowserTab> {
    let tab_id = tab
        .get("id")
        .and_then(value_as_tab_id)
        .or_else(|| tab.get("tabId").and_then(value_as_tab_id))?;
    Some(BrowserTab {
        tab_id,
        target,
        title: optional_non_empty_string(tab.get("title")),
        url: optional_non_empty_string(tab.get("url")),
        active: tab.get("active").and_then(Value::as_bool).unwrap_or(false),
    })
}

pub(super) fn tab_id_value(tab_id: &str) -> Value {
    tab_id
        .parse::<i64>()
        .map(Value::from)
        .unwrap_or_else(|_| Value::String(tab_id.to_string()))
}

fn value_as_tab_id(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn optional_non_empty_string(value: Option<&Value>) -> Option<String> {
    let value = value?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}
