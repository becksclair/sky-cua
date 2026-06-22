use anyhow::{Result, anyhow};
use serde_json::Value;
use sky_cua_platform::model::{
    BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT, BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
    BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserTargetKind, normalize_browser_open_url,
};

use super::super::{
    optional_non_empty_string, parse_optional_string_argument, parse_optional_usize,
};

pub(crate) fn parse_browser_target(arguments: &Value) -> Result<Option<BrowserTargetKind>> {
    let Some(raw_target) = arguments.get("target") else {
        return Ok(None);
    };
    if raw_target.is_null() {
        return Ok(None);
    }
    let Some(raw_target) = raw_target.as_str() else {
        return Err(anyhow!("browser target must be a string"));
    };
    let Some(target) = optional_non_empty_string(raw_target) else {
        return Ok(None);
    };
    match target.as_str() {
        "user_chrome" => Ok(Some(BrowserTargetKind::UserChrome)),
        other => Err(anyhow!("browser target must be user_chrome, got {other}")),
    }
}

pub(crate) fn parse_browser_open_url(arguments: &Value) -> Result<Option<String>> {
    let Some(raw_url) = arguments.get("url") else {
        return Ok(None);
    };
    if raw_url.is_null() {
        return Ok(None);
    }
    let Some(raw_url) = raw_url.as_str() else {
        return Err(anyhow!("browser_open url must be a string"));
    };
    if raw_url.is_empty() {
        return Ok(None);
    }
    if raw_url.trim() != raw_url {
        return Err(anyhow!(
            "browser_open url must use http://, https://, or about:blank"
        ));
    }
    if let Some(url) = normalize_browser_open_url(raw_url) {
        return Ok(Some(url));
    }
    Err(anyhow!(
        "browser_open url must use http://, https://, or about:blank"
    ))
}

pub(crate) fn parse_required_browser_url(arguments: &Value, tool_name: &str) -> Result<String> {
    let raw_url = parse_required_literal_string(arguments, "url", &format!("{tool_name} url"))?;
    if raw_url.trim() != raw_url {
        return Err(anyhow!(
            "{tool_name} url must use http://, https://, or about:blank"
        ));
    }
    normalize_browser_open_url(&raw_url)
        .ok_or_else(|| anyhow!("{tool_name} url must use http://, https://, or about:blank"))
}

pub(crate) fn parse_required_string(arguments: &Value, name: &str, label: &str) -> Result<String> {
    let Some(raw_value) = arguments.get(name) else {
        return Err(anyhow!("{label} is required"));
    };
    let Some(raw_value) = raw_value.as_str() else {
        return Err(anyhow!("{label} must be a string"));
    };
    optional_non_empty_string(raw_value).ok_or_else(|| anyhow!("{label} is required"))
}

pub(crate) fn parse_required_literal_string(
    arguments: &Value,
    name: &str,
    label: &str,
) -> Result<String> {
    let Some(raw_value) = arguments.get(name) else {
        return Err(anyhow!("{label} is required"));
    };
    let Some(raw_value) = raw_value.as_str() else {
        return Err(anyhow!("{label} must be a string"));
    };
    if raw_value.is_empty() {
        Err(anyhow!("{label} is required"))
    } else {
        Ok(raw_value.to_owned())
    }
}

pub(crate) fn parse_browser_tab_id(arguments: &Value) -> Result<String> {
    let Some(raw_tab_id) = arguments.get("tab_id") else {
        return Err(anyhow!("browser tab_id is required"));
    };
    let raw_tab_id = match raw_tab_id {
        Value::String(value) => value.as_str(),
        _ => return Err(anyhow!("browser tab_id must be a string")),
    };
    optional_non_empty_string(raw_tab_id).ok_or_else(|| anyhow!("browser tab_id is required"))
}

pub(crate) fn parse_browser_point(arguments: &Value, label: &str) -> Result<(f64, f64)> {
    let x = parse_non_negative_finite_number(arguments, "x", label)?;
    let y = parse_non_negative_finite_number(arguments, "y", label)?;
    Ok((x, y))
}

pub(crate) fn parse_browser_scroll(
    arguments: &Value,
) -> Result<(f64, f64, Option<f64>, Option<f64>)> {
    let delta_x = parse_optional_number(arguments, "delta_x", 0.0, "browser_scroll delta_x")?;
    let delta_y = parse_optional_number(arguments, "delta_y", 0.0, "browser_scroll delta_y")?;
    if !delta_x.is_finite() || !delta_y.is_finite() {
        return Err(anyhow!("browser_scroll deltas must be finite numbers"));
    }
    if delta_x == 0.0 && delta_y == 0.0 {
        return Err(anyhow!(
            "browser_scroll requires non-zero delta_x or delta_y"
        ));
    }
    let x = parse_optional_scroll_coordinate(arguments, "x")?;
    let y = parse_optional_scroll_coordinate(arguments, "y")?;
    if x.is_some() != y.is_some() {
        return Err(anyhow!("browser_scroll x and y must be provided together"));
    }
    Ok((delta_x, delta_y, x, y))
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BrowserSnapshotOptions {
    pub(crate) element_offset: Option<usize>,
    pub(crate) element_limit: Option<usize>,
    pub(crate) element_query: Option<String>,
    pub(crate) text_limit: usize,
}

pub(crate) fn parse_browser_snapshot_options(arguments: &Value) -> Result<BrowserSnapshotOptions> {
    let element_limit =
        parse_optional_usize(arguments, "element_limit", "browser_snapshot element_limit")?;
    if element_limit.is_some_and(|limit| limit > BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT) {
        return Err(anyhow!(
            "browser_snapshot element_limit must be at most {BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT}"
        ));
    }

    Ok(BrowserSnapshotOptions {
        element_offset: parse_optional_usize(
            arguments,
            "element_offset",
            "browser_snapshot element_offset",
        )?,
        element_limit,
        element_query: parse_optional_string_argument(
            arguments,
            "element_query",
            "browser_snapshot element_query",
        )?,
        text_limit: parse_optional_usize_with_max(
            arguments,
            "text_limit",
            BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT,
            BROWSER_SNAPSHOT_MAX_TEXT_LIMIT,
            "browser_snapshot text_limit",
        )?,
    })
}

fn parse_optional_number(arguments: &Value, name: &str, default: f64, label: &str) -> Result<f64> {
    let Some(raw_value) = arguments.get(name) else {
        return Ok(default);
    };
    raw_value
        .as_f64()
        .ok_or_else(|| anyhow!("{label} must be a number"))
}

fn parse_optional_scroll_coordinate(arguments: &Value, name: &str) -> Result<Option<f64>> {
    let Some(raw_value) = arguments.get(name) else {
        return Ok(None);
    };
    if raw_value.is_null() {
        return Ok(None);
    }
    let Some(value) = raw_value.as_f64() else {
        return Err(anyhow!("browser_scroll {name} must be a number"));
    };
    if value.is_finite() && value >= 0.0 {
        Ok(Some(value))
    } else {
        Err(anyhow!(
            "browser_scroll {name} must be a finite non-negative browser screenshot pixel coordinate"
        ))
    }
}

fn parse_optional_usize_with_max(
    arguments: &Value,
    name: &str,
    default: usize,
    max: usize,
    label: &str,
) -> Result<usize> {
    let Some(raw_value) = arguments.get(name) else {
        return Ok(default);
    };
    if raw_value.is_null() {
        return Ok(default);
    }
    let Some(value) = raw_value.as_u64() else {
        return Err(anyhow!("{label} must be a non-negative integer"));
    };
    let value = usize::try_from(value).map_err(|_| anyhow!("{label} is too large"))?;
    if value > max {
        return Err(anyhow!("{label} must be at most {max}"));
    }
    Ok(value)
}

fn parse_non_negative_finite_number(arguments: &Value, name: &str, label: &str) -> Result<f64> {
    let Some(raw_value) = arguments.get(name) else {
        return Err(anyhow!("{label} {name} is required"));
    };
    let Some(value) = raw_value.as_f64() else {
        return Err(anyhow!("{label} {name} must be a number"));
    };
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(anyhow!(
            "{label} {name} must be a finite non-negative browser screenshot pixel coordinate"
        ))
    }
}

pub(crate) fn parse_optional_bool(arguments: &Value, name: &str, default: bool) -> Result<bool> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(anyhow!("{name} must be a boolean when provided")),
    }
}

#[derive(Debug, Default)]
pub(crate) struct BrowserTabTextFilter {
    pub(crate) title_contains: Option<String>,
    pub(crate) url_contains: Option<String>,
}

impl BrowserTabTextFilter {
    pub(crate) fn is_empty(&self) -> bool {
        self.title_contains.is_none() && self.url_contains.is_none()
    }
}

pub(crate) fn parse_browser_tab_filter(arguments: &Value) -> Result<BrowserTabTextFilter> {
    Ok(BrowserTabTextFilter {
        title_contains: parse_optional_string_argument(
            arguments,
            "title_contains",
            "title_contains",
        )?,
        url_contains: parse_optional_string_argument(arguments, "url_contains", "url_contains")?,
    })
}
