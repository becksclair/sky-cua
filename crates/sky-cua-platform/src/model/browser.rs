use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{BrowserIntegrationReport, DiagnosticEntry};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserRequest {
    Status,
    ListTabs {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
    },
    Open {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    ClaimTab {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
    },
    MoveMouse {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        x: f64,
        y: f64,
        #[serde(default = "default_wait_for_arrival")]
        wait_for_arrival: bool,
    },
    Navigate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        url: String,
    },
    Snapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text_limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_offset: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_query: Option<String>,
    },
    Screenshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        #[serde(
            default = "default_include_image_data",
            skip_serializing_if = "is_true"
        )]
        include_image_data: bool,
    },
    Click {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        x: f64,
        y: f64,
    },
    TypeText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        text: String,
    },
    PressKey {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        key: String,
    },
    Scroll {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        delta_x: f64,
        delta_y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
    },
    Eval {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        expression: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserResponse {
    Status { report: BrowserStatusReport },
    ListTabs { response: BrowserListTabsResponse },
    Open { response: BrowserOpenResponse },
    ClaimTab { response: BrowserClaimTabResponse },
    MoveMouse { response: BrowserMoveMouseResponse },
    Navigate { response: BrowserNavigateResponse },
    Snapshot { response: BrowserSnapshotResponse },
    Screenshot { response: BrowserScreenshotResponse },
    Click { response: BrowserActionResponse },
    TypeText { response: BrowserActionResponse },
    PressKey { response: BrowserActionResponse },
    Scroll { response: BrowserActionResponse },
    Eval { response: BrowserEvalResponse },
}

fn default_wait_for_arrival() -> bool {
    true
}

fn default_include_image_data() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// Browser automation targets. `user_chrome` (the user's real, logged-in
/// Chrome-family browser) is the only target: managed/isolated browser
/// lifecycle was retired on 2026-06-11 because an isolated profile defeats
/// the purpose of real-browser control.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTargetKind {
    UserChrome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTargetAvailability {
    pub target: BrowserTargetKind,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserStatusReport {
    pub enabled: bool,
    pub available_targets: Vec<BrowserTargetAvailability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs_known: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_integration: Option<BrowserIntegrationReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTab {
    pub tab_id: String,
    pub target: BrowserTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserListTabsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<BrowserTargetKind>,
    pub tabs: Vec<BrowserTab>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserOpenResponse {
    pub target: BrowserTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<BrowserTab>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserClaimTabResponse {
    pub target: BrowserTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<BrowserTab>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserMoveMouseResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub x: f64,
    pub y: f64,
    pub wait_for_arrival: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserNavigateResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserSnapshotResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// Environment variable gating arbitrary page-JavaScript execution via
/// `browser_eval`. This is a security boundary the MCP client and the service
/// must agree on, so the opt-in check lives here and is shared by both rather
/// than duplicated per crate.
pub const BROWSER_EVAL_ENV: &str = "SKY_CUA_BROWSER_EVAL";

/// Default visible-text budget for MCP `browser_snapshot` calls. Direct
/// service callers may omit the field to request the service maximum.
pub const BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT: usize = 4_000;

/// Default actionable-element budget for MCP `browser_snapshot` calls.
pub const BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT: usize = 200;

/// Maximum actionable-element budget for browser snapshots. This preserves
/// the original service-side capture ceiling.
pub const BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT: usize = 5_000;

/// Maximum visible-text budget for browser snapshots across MCP and service
/// boundaries.
pub const BROWSER_SNAPSHOT_MAX_TEXT_LIMIT: usize = 20_000;

/// Whether the operator has opted in to `browser_eval` page-JavaScript
/// execution. Off unless `SKY_CUA_BROWSER_EVAL` is `on`, `1`, or `true`.
pub fn browser_eval_enabled() -> bool {
    matches!(
        std::env::var(BROWSER_EVAL_ENV)
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Ok("on" | "1" | "true")
    )
}

pub fn browser_diagnostic_is_error_code(code: &str) -> bool {
    matches!(
        code,
        "BrowserBridgeDisconnected"
            | "BrowserBridgeUnsupported"
            | "BrowserBridgeRequestFailed"
            | "BrowserBridgeRequestTimedOut"
            | "BrowserSelectionInvalid"
            | "BrowserTabIdInvalid"
            | "BrowserMouseCoordinateInvalid"
            | "BrowserTextInvalid"
            | "BrowserKeyInvalid"
            | "BrowserScrollInvalid"
            | "BrowserOpenUrlUnsupported"
            | "BrowserNavigationFailed"
            | "BrowserOpenPartial"
            | "BrowserClaimPartial"
            | "BrowserEvalException"
            | "BrowserEvalDisabled"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserScreenshotResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub mime_type: String,
    pub data_base64: String,
    /// Filesystem path of the persisted capture, when the service wrote one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    /// Image width in pixels. Matches CSS viewport width so image pixels,
    /// snapshot element bounds, and pointer coordinates share one space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Image height in pixels. See `width`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserActionResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserEvalResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[must_use]
pub fn normalize_browser_open_url(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && (value == "about:blank"
            || value.starts_with("https://")
            || value.starts_with("http://")))
    .then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_browser_open_url;

    #[test]
    fn browser_open_url_allows_only_http_https_and_about_blank() {
        assert_eq!(
            normalize_browser_open_url(" https://example.test/ "),
            Some("https://example.test/".to_string())
        );
        assert_eq!(
            normalize_browser_open_url("http://127.0.0.1:8080/page"),
            Some("http://127.0.0.1:8080/page".to_string())
        );
        assert_eq!(
            normalize_browser_open_url("about:blank"),
            Some("about:blank".to_string())
        );
        assert_eq!(normalize_browser_open_url(""), None);
        assert_eq!(normalize_browser_open_url("file:///etc/passwd"), None);
        assert_eq!(normalize_browser_open_url("javascript:alert(1)"), None);
    }
}
