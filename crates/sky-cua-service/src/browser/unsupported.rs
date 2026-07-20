use std::collections::BTreeMap;

use sky_cua_platform::model::{
    BrowserActionResponse, BrowserClaimTabResponse, BrowserEvalResponse, BrowserIntegrationReport,
    BrowserListTabsResponse, BrowserMoveMouseResponse, BrowserNavigateResponse,
    BrowserOpenResponse, BrowserRequest, BrowserRequestContext, BrowserResponse,
    BrowserScreenshotResponse, BrowserSessionIdentity, BrowserSnapshotResponse,
    BrowserStatusReport, BrowserTargetAvailability, BrowserTargetKind, DiagnosticEntry,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum BrowserControlMode {
    Legacy,
    Hybrid,
    Strict,
}

impl BrowserControlMode {
    pub(crate) fn uses_persistent_actor(self) -> bool {
        matches!(self, Self::Hybrid | Self::Strict)
    }
}

/// Type-compatible non-Unix placeholder. Production construction is prevented
/// by [`browser_control_mode`], which always reports browser control unsupported.
pub(crate) struct BrowserControlRuntime;

impl BrowserControlRuntime {
    #[cfg(test)]
    pub(crate) fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self)
    }

    pub(crate) fn new_with_mode(_mode: BrowserControlMode) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self)
    }

    pub(crate) fn observe_mcp_client(
        &self,
        _provenance: &sky_cua_platform::model::BrowserCallerProvenance,
    ) {
    }

    pub(crate) async fn high_level(
        &self,
        _request: BrowserRequest,
        _context: BrowserRequestContext,
    ) -> Result<BrowserResponse, DiagnosticEntry> {
        Err(browser_control_unsupported_diagnostic())
    }

    pub(crate) async fn status_report(
        &self,
        integration: Option<BrowserIntegrationReport>,
        _deferred: bool,
    ) -> BrowserStatusReport {
        unsupported_browser_status(integration, browser_bridge_diagnostics().await)
    }
}

pub(crate) fn browser_control_mode() -> Result<BrowserControlMode, DiagnosticEntry> {
    Err(browser_control_unsupported_diagnostic())
}

pub(crate) fn mark_bridge_activity() {}

pub(crate) fn browser_session_lingering() -> bool {
    false
}

pub(crate) async fn list_tabs(target: Option<BrowserTargetKind>) -> BrowserListTabsResponse {
    BrowserListTabsResponse {
        target: Some(target.unwrap_or(BrowserTargetKind::UserChrome)),
        tabs: Vec::new(),
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn list_tabs_with_identity(
    target: Option<BrowserTargetKind>,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserListTabsResponse {
    list_tabs(target).await
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

pub(crate) async fn open_tab_with_identity(
    target: Option<BrowserTargetKind>,
    url: Option<String>,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserOpenResponse {
    open_tab(target, url).await
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

pub(crate) async fn claim_tab_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserClaimTabResponse {
    claim_tab(target, tab_id).await
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

pub(crate) async fn move_mouse_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
    wait_for_arrival: bool,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserMoveMouseResponse {
    move_mouse(target, tab_id, x, y, wait_for_arrival).await
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

pub(crate) async fn navigate_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    url: String,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserNavigateResponse {
    navigate(target, tab_id, url).await
}

pub(crate) async fn snapshot(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _text_limit: Option<usize>,
    _element_offset: Option<usize>,
    _element_limit: Option<usize>,
    _element_query: Option<String>,
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

pub(crate) async fn snapshot_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text_limit: Option<usize>,
    element_offset: Option<usize>,
    element_limit: Option<usize>,
    element_query: Option<String>,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserSnapshotResponse {
    snapshot(
        target,
        tab_id,
        text_limit,
        element_offset,
        element_limit,
        element_query,
    )
    .await
}

pub(crate) async fn screenshot(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _include_image_data: bool,
) -> BrowserScreenshotResponse {
    BrowserScreenshotResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab_id,
        mime_type: "image/png".to_string(),
        data_base64: String::new(),
        screenshot_path: None,
        width: None,
        height: None,
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn screenshot_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    include_image_data: bool,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserScreenshotResponse {
    screenshot(target, tab_id, include_image_data).await
}

pub(crate) async fn eval(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _expression: String,
) -> BrowserEvalResponse {
    BrowserEvalResponse {
        target: target.unwrap_or(BrowserTargetKind::UserChrome),
        tab_id,
        value: None,
        diagnostics: vec![browser_bridge_unsupported_diagnostic()],
    }
}

pub(crate) async fn eval_with_policy_and_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    expression: String,
    _browser_eval_enabled: bool,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserEvalResponse {
    eval(target, tab_id, expression).await
}

pub(crate) async fn click(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _x: f64,
    _y: f64,
) -> BrowserActionResponse {
    unsupported_action_response(target, tab_id, "click")
}

pub(crate) async fn click_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    click(target, tab_id, x, y).await
}

pub(crate) async fn click_element_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _element_ref: String,
    _identity: Option<BrowserSessionIdentity>,
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

pub(crate) async fn type_text_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text: String,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    type_text(target, tab_id, text).await
}

pub(crate) async fn type_text_element_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _element_ref: String,
    _text: String,
    _identity: Option<BrowserSessionIdentity>,
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

pub(crate) async fn press_key_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    key: String,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    press_key(target, tab_id, key).await
}

pub(crate) async fn scroll(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    _delta_x: f64,
    _delta_y: f64,
    _x: Option<f64>,
    _y: Option<f64>,
) -> BrowserActionResponse {
    unsupported_action_response(target, tab_id, "scroll")
}

pub(crate) async fn scroll_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    delta_x: f64,
    delta_y: f64,
    x: Option<f64>,
    y: Option<f64>,
    _identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    scroll(target, tab_id, delta_x, delta_y, x, y).await
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
        enabled: false,
        available_targets: vec![BrowserTargetAvailability {
            target: BrowserTargetKind::UserChrome,
            available: false,
            detail: "Chrome native-host browser bridge requires a Unix socket platform."
                .to_string(),
        }],
        tabs_known: None,
        browser_integration: integration,
        control_plane: None,
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

fn browser_control_unsupported_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserControlUnsupported".to_string(),
        message: "Browser control is unavailable because this platform has no Unix native-host socket bridge.".to_string(),
        details: None,
    }
}
