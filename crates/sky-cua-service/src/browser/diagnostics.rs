use std::path::Path;

use serde_json::Value;
use sky_cua_platform::model::{BrowserTargetKind, DiagnosticEntry, normalize_browser_open_url};

#[cfg(not(test))]
const BROWSER_OPEN_TIMEOUT_MS: u128 = 12_000;
#[cfg(test)]
const BROWSER_OPEN_TIMEOUT_MS: u128 = 250;

pub(super) fn validate_action_target(
    target: BrowserTargetKind,
    normalized_tab_id: &str,
) -> Vec<DiagnosticEntry> {
    let mut diagnostics = Vec::new();
    if target == BrowserTargetKind::Managed {
        diagnostics.push(managed_action_unsupported_diagnostic());
    }
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
    (!tab_id.is_empty())
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

pub(super) fn managed_tabs_unsupported_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserTargetUnsupported".to_string(),
        message: "Managed browser tab lifecycle is not implemented yet; use target=user_chrome."
            .to_string(),
        details: None,
    }
}

pub(super) fn managed_open_unsupported_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserTargetUnsupported".to_string(),
        message: "Managed browser lifecycle is not implemented yet; use target=user_chrome."
            .to_string(),
        details: None,
    }
}

pub(super) fn managed_action_unsupported_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserTargetUnsupported".to_string(),
        message: "Managed browser actions are not implemented yet; use target=user_chrome."
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
        message: "browser_scroll coordinates and deltas must be finite browser screenshot pixels, and x/y must be non-negative.".to_string(),
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
    DiagnosticEntry {
        code: "BrowserBridgeRequestTimedOut".to_string(),
        message: format!(
            "Timed out trying to open a browser tab through the Chrome extension/native-host bridge after {BROWSER_OPEN_TIMEOUT_MS} ms."
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
        message: "Browser tab id must be a non-empty string.".to_string(),
        details: None,
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
