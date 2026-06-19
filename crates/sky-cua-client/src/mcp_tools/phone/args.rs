//! Typed argument parsing and validation for the `phone_*` MCP tools.
//!
//! Mirrors `browser/args.rs`: every tool's arguments are pulled out of the raw
//! JSON `Value` into the platform request structs, with the same blank/zero
//! tolerance the browser surface uses (OpenCode-style hosts send empty-string
//! and zero defaults for omitted fields, which must read as "absent").

use anyhow::{Result, anyhow};
use serde_json::Value;
use sky_cua_platform::model::{
    PhoneAppInstallMode, PhoneBackendKind, PhoneSessionSelector, PhoneSettingsScreen,
};

use super::super::{optional_non_empty_string, parse_optional_usize};

/// Pull the shared `session_id`/`serial` selector out of any phone request.
/// Both fields are optional and tolerate blank strings (treated as absent).
pub(crate) fn parse_phone_selector(arguments: &Value) -> Result<PhoneSessionSelector> {
    Ok(PhoneSessionSelector {
        session_id: parse_optional_string(arguments, "session_id", "session_id")?,
        serial: parse_optional_string(arguments, "serial", "serial")?,
    })
}

/// Parse an optional `backend` enum field. Absent or blank means "auto-route".
pub(crate) fn parse_phone_backend(arguments: &Value) -> Result<Option<PhoneBackendKind>> {
    let Some(raw_backend) = arguments.get("backend") else {
        return Ok(None);
    };
    if raw_backend.is_null() {
        return Ok(None);
    }
    let Some(raw_backend) = raw_backend.as_str() else {
        return Err(anyhow!("phone backend must be a string"));
    };
    let Some(backend) = optional_non_empty_string(raw_backend) else {
        return Ok(None);
    };
    match backend.as_str() {
        "auto" => Ok(Some(PhoneBackendKind::Auto)),
        "adb" => Ok(Some(PhoneBackendKind::Adb)),
        "companion" => Ok(Some(PhoneBackendKind::Companion)),
        "scrcpy" => Ok(Some(PhoneBackendKind::Scrcpy)),
        "none" => Ok(Some(PhoneBackendKind::None)),
        other => Err(anyhow!(
            "phone backend must be auto, adb, companion, scrcpy, or none, got {other}"
        )),
    }
}

/// Parse the `screen` enum required by `phone_open_settings`.
pub(crate) fn parse_phone_settings_screen(arguments: &Value) -> Result<PhoneSettingsScreen> {
    let raw = parse_required_string(arguments, "screen", "phone_open_settings screen")?;
    match raw.as_str() {
        "accessibility" => Ok(PhoneSettingsScreen::Accessibility),
        "notification_access" => Ok(PhoneSettingsScreen::NotificationAccess),
        "overlay_permission" => Ok(PhoneSettingsScreen::OverlayPermission),
        "app_details" => Ok(PhoneSettingsScreen::AppDetails),
        "wireless_debugging" => Ok(PhoneSettingsScreen::WirelessDebugging),
        "battery_optimization" => Ok(PhoneSettingsScreen::BatteryOptimization),
        other => Err(anyhow!(
            "phone_open_settings screen must be accessibility, notification_access, \
             overlay_permission, app_details, wireless_debugging, or battery_optimization, \
             got {other}"
        )),
    }
}

/// Parse the optional `mode` enum for `phone_app_install`. Defaults to `single`.
pub(crate) fn parse_phone_app_install_mode(arguments: &Value) -> Result<PhoneAppInstallMode> {
    let Some(raw_mode) = arguments.get("mode") else {
        return Ok(PhoneAppInstallMode::Single);
    };
    if raw_mode.is_null() {
        return Ok(PhoneAppInstallMode::Single);
    }
    let Some(raw_mode) = raw_mode.as_str() else {
        return Err(anyhow!("phone_app_install mode must be a string"));
    };
    match raw_mode {
        "single" => Ok(PhoneAppInstallMode::Single),
        "multiple" => Ok(PhoneAppInstallMode::Multiple),
        "multi_package" => Ok(PhoneAppInstallMode::MultiPackage),
        other => Err(anyhow!(
            "phone_app_install mode must be single, multiple, or multi_package, got {other}"
        )),
    }
}

/// Parse the required non-empty `apk_paths` array for `phone_app_install`.
pub(crate) fn parse_phone_apk_paths(arguments: &Value) -> Result<Vec<String>> {
    let Some(raw_paths) = arguments.get("apk_paths") else {
        return Err(anyhow!("phone_app_install apk_paths is required"));
    };
    let Some(array) = raw_paths.as_array() else {
        return Err(anyhow!("phone_app_install apk_paths must be an array"));
    };
    let mut paths = Vec::with_capacity(array.len());
    for entry in array {
        let Some(entry) = entry.as_str() else {
            return Err(anyhow!(
                "phone_app_install apk_paths entries must be strings"
            ));
        };
        let Some(entry) = optional_non_empty_string(entry) else {
            continue;
        };
        paths.push(entry);
    }
    if paths.is_empty() {
        return Err(anyhow!(
            "phone_app_install apk_paths must contain at least one path"
        ));
    }
    Ok(paths)
}

/// Parse the required `host_port` for `phone_pair_wireless`/`phone_connect`.
pub(crate) fn parse_required_string(arguments: &Value, name: &str, label: &str) -> Result<String> {
    let Some(raw_value) = arguments.get(name) else {
        return Err(anyhow!("{label} is required"));
    };
    let Some(raw_value) = raw_value.as_str() else {
        return Err(anyhow!("{label} must be a string"));
    };
    optional_non_empty_string(raw_value).ok_or_else(|| anyhow!("{label} is required"))
}

/// Parse a required string that preserves interior/leading/trailing whitespace
/// (literal text such as `phone_type_text` / `phone_notification_reply`).
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

/// Parse an optional trimmed string field, treating blank as absent.
pub(crate) fn parse_optional_string(
    arguments: &Value,
    name: &str,
    label: &str,
) -> Result<Option<String>> {
    let Some(raw_value) = arguments.get(name) else {
        return Ok(None);
    };
    if raw_value.is_null() {
        return Ok(None);
    }
    let Some(raw_value) = raw_value.as_str() else {
        return Err(anyhow!("{label} must be a string"));
    };
    Ok(optional_non_empty_string(raw_value))
}

/// Parse an optional bool field, defaulting when absent or null.
pub(crate) fn parse_optional_bool(arguments: &Value, name: &str, default: bool) -> Result<bool> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(anyhow!("{name} must be a boolean when provided")),
    }
}

/// Parse a required finite coordinate. Phone coordinates may be negative only
/// when off-device gestures are intended; the spine keeps them finite and
/// non-negative to match screenshot/snapshot pixel space.
pub(crate) fn parse_phone_coordinate(arguments: &Value, name: &str, label: &str) -> Result<f64> {
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
            "{label} {name} must be a finite non-negative phone screenshot pixel coordinate"
        ))
    }
}

/// Parse the optional `duration_ms` for `phone_swipe`.
pub(crate) fn parse_optional_duration_ms(arguments: &Value) -> Result<Option<u32>> {
    let Some(raw_value) = arguments.get("duration_ms") else {
        return Ok(None);
    };
    if raw_value.is_null() {
        return Ok(None);
    }
    let Some(value) = raw_value.as_u64() else {
        return Err(anyhow!(
            "phone_swipe duration_ms must be a non-negative integer"
        ));
    };
    u32::try_from(value)
        .map(Some)
        .map_err(|_| anyhow!("phone_swipe duration_ms is too large"))
}

/// Parse the optional `node_limit`/`limit` integer fields shared by several
/// list-style tools.
pub(crate) fn parse_optional_limit(
    arguments: &Value,
    name: &str,
    label: &str,
) -> Result<Option<usize>> {
    parse_optional_usize(arguments, name, label)
}
