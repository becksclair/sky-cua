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
        #[serde(default)]
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
    },
    Screenshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
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
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTargetKind {
    Managed,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserScreenshotResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub mime_type: String,
    pub data_base64: String,
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
