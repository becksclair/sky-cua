use std::collections::BTreeMap;

use sky_cua_platform::model::{
    BrowserActionResponse, BrowserClaimTabResponse, BrowserIntegrationReport,
    BrowserListTabsResponse, BrowserMoveMouseResponse, BrowserNavigateResponse,
    BrowserOpenResponse, BrowserScreenshotResponse, BrowserSnapshotResponse, BrowserStatusReport,
    BrowserTargetAvailability, BrowserTargetKind, DiagnosticEntry,
};

pub(crate) async fn list_tabs(target: Option<BrowserTargetKind>) -> BrowserListTabsResponse {
    BrowserListTabsResponse {
        target: Some(target.unwrap_or(BrowserTargetKind::UserChrome)),
        tabs: Vec::new(),
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn open_tab(
    target: Option<BrowserTargetKind>,
    _url: Option<String>,
) -> BrowserOpenResponse {
    BrowserOpenResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab: None,
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn claim_tab(
    target: Option<BrowserTargetKind>,
    _tab_id: String,
) -> BrowserClaimTabResponse {
    BrowserClaimTabResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab: None,
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn move_mouse(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
    wait_for_arrival: bool,
) -> BrowserMoveMouseResponse {
    BrowserMoveMouseResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab_id,
        x,
        y,
        wait_for_arrival,
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn navigate(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    url: String,
) -> BrowserNavigateResponse {
    BrowserNavigateResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab_id,
        url,
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn snapshot(
    target: Option<BrowserTargetKind>,
    tab_id: String,
) -> BrowserSnapshotResponse {
    BrowserSnapshotResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab_id,
        title: None,
        url: None,
        snapshot: None,
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn screenshot(
    target: Option<BrowserTargetKind>,
    tab_id: String,
) -> BrowserScreenshotResponse {
    BrowserScreenshotResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab_id,
        mime_type: "image/png".to_string(),
        data_base64: String::new(),
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn click(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _x: f64,
    _y: f64,
) -> BrowserActionResponse {
    unsupported_action_response(target, tab_id, "click")
}

pub(crate) async fn type_text(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _text: String,
) -> BrowserActionResponse {
    unsupported_action_response(target, tab_id, "type_text")
}

pub(crate) async fn press_key(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _key: String,
) -> BrowserActionResponse {
    unsupported_action_response(target, tab_id, "press_key")
}

pub(crate) async fn scroll(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _delta_x: f64,
    _delta_y: f64,
    _x: f64,
    _y: f64,
) -> BrowserActionResponse {
    unsupported_action_response(target, tab_id, "scroll")
}

pub(crate) async fn browser_bridge_diagnostics() -> Vec<DiagnosticEntry> {
    vec![browser_bridge_unsupported_diagnostic()]
}

pub(crate) async fn browser_status_from_deferred_doctor() -> BrowserStatusReport {
    unsupported_browser_status(None, browser_bridge_diagnostics().await)
}

pub(crate) async fn browser_status_from_doctor(
    integration: Option<BrowserIntegrationReport>,
) -> BrowserStatusReport {
    unsupported_browser_status(integration, browser_bridge_diagnostics().await)
}

pub(crate) fn browser_env_values_present() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn unsupported_browser_status(
    integration: Option<BrowserIntegrationReport>,
    diagnostics: Vec<DiagnosticEntry>,
) -> BrowserStatusReport {
    BrowserStatusReport {
        enabled: true,
        available_targets: vec![
            BrowserTargetAvailability {
                target: BrowserTargetKind::Managed,
                available: false,
                detail: "Managed browser lifecycle is not implemented on this platform."
                    .to_string(),
            },
            BrowserTargetAvailability {
                target: BrowserTargetKind::UserChrome,
                available: false,
                detail: "Chrome native-host browser bridge requires a Unix socket platform."
                    .to_string(),
            },
        ],
        tabs_known: None,
        browser_integration: integration,
        diagnostics,
    }
}

fn unsupported_action_response(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    action: &str,
) -> BrowserActionResponse {
    BrowserActionResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab_id,
        action: action.to_string(),
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

fn browser_bridge_unsupported_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserBridgeUnsupported".to_string(),
        message: "Browser MCP tools require the Unix native-host socket bridge on this platform."
            .to_string(),
        details: None,
    }
}
