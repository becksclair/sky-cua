use std::path::Path;

use serde_json::Value;
use sky_cua_platform::model::{DiagnosticEntry, normalize_browser_open_url};

// Reflects the actual overall browser deadline (default 12s, raised by
// `SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS`) so the timeout message reports the real
// budget. Keep the default in sync with `browser_open_timeout()` in `bridge.rs`.
#[cfg(not(test))]
fn browser_open_timeout_ms() -> u128 {
    u128::from(super::transport::browser_request_timeout_override_ms().unwrap_or(12_000))
}
#[cfg(test)]
fn browser_open_timeout_ms() -> u128 {
    2_000
}

pub(super) fn validate_action_tab_id(normalized_tab_id: &str) -> Vec<DiagnosticEntry> {
    let mut diagnostics = Vec::new();
    if normalized_tab_id.is_empty() {
        diagnostics.push(invalid_tab_id_diagnostic());
    }
    diagnostics
}

pub(super) fn validate_point(x: f64, y: f64) -> Result<(), DiagnosticEntry> {
    if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 {
        Ok(())
    } else {
        Err(invalid_mouse_coordinate_diagnostic())
    }
}

pub(super) fn validate_open_url(url: Option<String>) -> Result<Option<String>, DiagnosticEntry> {
    let Some(url) = url else {
        return Ok(None);
    };
    normalize_browser_open_url(&url)
        .map(Some)
        .ok_or_else(|| unsupported_open_url_diagnostic("browser_open"))
}

pub(super) fn validate_tab_id(tab_id: String) -> Result<String, DiagnosticEntry> {
    let tab_id = tab_id.trim();
    // Bridge tab ids are Chrome's per-browser integer tab ids. Forwarding
    // anything else to the extension yields opaque Chrome API signature errors
    // ("No matching signature", "Invalid type: expected integer, found
    // string"), so reject handles from other tool surfaces (e.g. "t11") here
    // with an actionable diagnostic instead.
    (!tab_id.is_empty() && tab_id.parse::<i64>().is_ok())
        .then(|| tab_id.to_string())
        .ok_or_else(invalid_tab_id_diagnostic)
}

pub(crate) fn browser_bridge_disconnected_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserBridgeDisconnected".to_string(),
        message: "No Chrome extension/native-host browser socket is available for tab enumeration."
            .to_string(),
        details: None,
    }
}

pub(super) fn invalid_text_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserTextInvalid".to_string(),
        message: "browser_type_text text must be non-empty.".to_string(),
        details: None,
    }
}

pub(super) fn invalid_key_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserKeyInvalid".to_string(),
        message: "browser_press_key key must be non-empty.".to_string(),
        details: None,
    }
}

pub(super) fn invalid_scroll_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserScrollInvalid".to_string(),
        message: "browser_scroll deltas must be finite with at least one non-zero value; x/y coordinates must be finite, non-negative, and provided together.".to_string(),
        details: None,
    }
}

pub(super) fn unsupported_open_url_diagnostic(tool_name: &str) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserOpenUrlUnsupported".to_string(),
        message: format!("{tool_name} url must use http://, https://, or about:blank."),
        details: None,
    }
}

pub(super) fn bridge_timeout_diagnostic(action: &str, socket: &Path) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserBridgeRequestTimedOut".to_string(),
        message: format!(
            "Timed out trying to {action} Chrome extension/native-host browser socket {}.",
            socket.display()
        ),
        details: None,
    }
}

pub(super) fn browser_open_timeout_diagnostic() -> DiagnosticEntry {
    let timeout_ms = browser_open_timeout_ms();
    DiagnosticEntry {
        code: "BrowserBridgeRequestTimedOut".to_string(),
        message: format!(
            "Timed out trying to open a browser tab through the Chrome extension/native-host bridge after {timeout_ms} ms."
        ),
        details: None,
    }
}

pub(super) fn malformed_list_tabs_response_diagnostic(result: Option<&Value>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserBridgeRequestFailed".to_string(),
        message: "Chrome extension/native-host getUserTabs response did not include a tabs array."
            .to_string(),
        details: result.map(|value| format!("result={value}")),
    }
}

pub(super) fn unexpected_bridge_response_diagnostic(response: Value) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserBridgeRequestFailed".to_string(),
        message: format!(
            "Chrome extension/native-host returned an unexpected browser tab response: {response}"
        ),
        details: None,
    }
}

fn invalid_tab_id_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserTabIdInvalid".to_string(),
        message: "Browser tab id must be an integer Chrome tab id.".to_string(),
        details: Some(
            "Use a tab id returned by browser_open or list_resources (browser tabs). \
             Tab handles from other browser tool surfaces (for example \"t11\") do not \
             name sky-cua bridge tabs."
                .to_string(),
        ),
    }
}

fn invalid_mouse_coordinate_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserMouseCoordinateInvalid".to_string(),
        message: "browser_move_mouse x and y must be finite non-negative browser screenshot pixel coordinates."
            .to_string(),
        details: None,
    }
}
