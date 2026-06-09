use std::time::Duration;

#[cfg(test)]
use serde_json::{Value, json};
use sky_cua_platform::model::{
    BrowserActionResponse, BrowserClaimTabResponse, BrowserListTabsResponse,
    BrowserMoveMouseResponse, BrowserNavigateResponse, BrowserOpenResponse,
    BrowserScreenshotResponse, BrowserSnapshotResponse, BrowserTargetKind, DiagnosticEntry,
    normalize_browser_open_url,
};
#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::cdp::{BrowserCdpAction, BrowserCdpResult, cdp_action_from_sockets};
use super::diagnostics::{
    invalid_key_diagnostic, invalid_scroll_diagnostic, invalid_text_diagnostic,
    managed_action_unsupported_diagnostic, managed_open_unsupported_diagnostic,
    managed_tabs_unsupported_diagnostic, unsupported_open_url_diagnostic, validate_action_target,
    validate_open_url, validate_point, validate_tab_id,
};
use super::probe::{first_responsive_bridge_socket, list_tabs_from_sockets};
#[cfg(test)]
use super::protocol::{BRIDGE_INFO_REQUEST_ID, LIST_TABS_REQUEST_ID, read_frame, write_frame};
use super::session::{claim_tab_from_sockets, move_mouse_from_sockets, open_tab_from_sockets};
#[cfg(test)]
use super::snapshot::BROWSER_SNAPSHOT_EXPRESSION;
#[cfg(test)]
use super::sockets::record_bridge_socket_result;
use super::sockets::{
    BrowserSocketSelection, browser_bridge_disconnected_for_selection,
    browser_socket_selection_from_env, find_bridge_sockets,
};
#[cfg(test)]
use super::tabs::parse_tabs;
#[cfg(test)]
use super::transport::{
    BRIDGE_REQUEST_TIMEOUT, browser_session_params, list_tabs_method, send_bridge_request,
};

#[cfg(not(test))]
const BROWSER_OPEN_TIMEOUT: Duration = Duration::from_secs(12);
#[cfg(test)]
const BROWSER_OPEN_TIMEOUT: Duration = Duration::from_millis(250);

pub(crate) async fn list_tabs(target: Option<BrowserTargetKind>) -> BrowserListTabsResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    if resolved_target == BrowserTargetKind::Managed {
        return BrowserListTabsResponse {
            target: Some(resolved_target),
            tabs: Vec::new(),
            diagnostics: vec![managed_tabs_unsupported_diagnostic()],
        };
    }

    match list_tabs_from_bridge(Some(resolved_target)).await {
        Ok(response) => response,
        Err(diagnostic) => BrowserListTabsResponse {
            target: Some(resolved_target),
            tabs: Vec::new(),
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn open_tab(
    target: Option<BrowserTargetKind>,
    url: Option<String>,
) -> BrowserOpenResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    if resolved_target == BrowserTargetKind::Managed {
        return BrowserOpenResponse {
            target: resolved_target,
            tab: None,
            diagnostics: vec![managed_open_unsupported_diagnostic()],
        };
    }

    let url = match validate_open_url(url) {
        Ok(url) => url,
        Err(diagnostic) => {
            return BrowserOpenResponse {
                target: resolved_target,
                tab: None,
                diagnostics: vec![diagnostic],
            };
        }
    };

    match open_tab_from_bridge(resolved_target, url).await {
        Ok(response) => response,
        Err(diagnostic) => BrowserOpenResponse {
            target: resolved_target,
            tab: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn claim_tab(
    target: Option<BrowserTargetKind>,
    tab_id: String,
) -> BrowserClaimTabResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    if resolved_target == BrowserTargetKind::Managed {
        return BrowserClaimTabResponse {
            target: resolved_target,
            tab: None,
            diagnostics: vec![managed_action_unsupported_diagnostic()],
        };
    }

    let tab_id = match validate_tab_id(tab_id) {
        Ok(tab_id) => tab_id,
        Err(diagnostic) => {
            return BrowserClaimTabResponse {
                target: resolved_target,
                tab: None,
                diagnostics: vec![diagnostic],
            };
        }
    };

    match claim_tab_from_bridge(resolved_target, &tab_id).await {
        Ok(response) => response,
        Err(diagnostic) => BrowserClaimTabResponse {
            target: resolved_target,
            tab: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn move_mouse(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
    wait_for_arrival: bool,
) -> BrowserMoveMouseResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let normalized_tab_id = validate_tab_id(tab_id).unwrap_or_default();
    let mut diagnostics = validate_action_target(resolved_target, &normalized_tab_id);
    if let Err(diagnostic) = validate_point(x, y) {
        diagnostics.push(diagnostic);
    }
    if !diagnostics.is_empty() {
        return BrowserMoveMouseResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            x,
            y,
            wait_for_arrival,
            diagnostics,
        };
    }

    match move_mouse_from_bridge(resolved_target, &normalized_tab_id, x, y, wait_for_arrival).await
    {
        Ok(response) => response,
        Err(diagnostic) => BrowserMoveMouseResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            x,
            y,
            wait_for_arrival,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn navigate(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    url: String,
) -> BrowserNavigateResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let normalized_tab_id = validate_tab_id(tab_id).unwrap_or_default();
    let normalized_url = normalize_browser_open_url(&url).unwrap_or_default();
    let mut diagnostics = validate_action_target(resolved_target, &normalized_tab_id);
    if normalized_url.is_empty() {
        diagnostics.push(unsupported_open_url_diagnostic("browser_navigate"));
    }
    if !diagnostics.is_empty() {
        return BrowserNavigateResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            url: normalized_url,
            diagnostics,
        };
    }

    match run_cdp_action_from_bridge(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Navigate {
            url: normalized_url.clone(),
        },
    )
    .await
    {
        Ok(BrowserCdpResult::Navigate { url }) => BrowserNavigateResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            url,
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("navigate action returns navigate result"),
        Err(diagnostic) => BrowserNavigateResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            url: normalized_url,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn snapshot(
    target: Option<BrowserTargetKind>,
    tab_id: String,
) -> BrowserSnapshotResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let normalized_tab_id = validate_tab_id(tab_id).unwrap_or_default();
    let diagnostics = validate_action_target(resolved_target, &normalized_tab_id);
    if !diagnostics.is_empty() {
        return BrowserSnapshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            title: None,
            url: None,
            snapshot: None,
            diagnostics,
        };
    }

    match run_cdp_action_from_bridge(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Snapshot,
    )
    .await
    {
        Ok(BrowserCdpResult::Snapshot {
            title,
            url,
            snapshot,
        }) => BrowserSnapshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            title,
            url,
            snapshot: Some(snapshot),
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("snapshot action returns snapshot result"),
        Err(diagnostic) => BrowserSnapshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            title: None,
            url: None,
            snapshot: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn screenshot(
    target: Option<BrowserTargetKind>,
    tab_id: String,
) -> BrowserScreenshotResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let normalized_tab_id = validate_tab_id(tab_id).unwrap_or_default();
    let diagnostics = validate_action_target(resolved_target, &normalized_tab_id);
    if !diagnostics.is_empty() {
        return BrowserScreenshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            diagnostics,
        };
    }

    match run_cdp_action_from_bridge(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Screenshot,
    )
    .await
    {
        Ok(BrowserCdpResult::Screenshot { data_base64 }) => BrowserScreenshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            mime_type: "image/png".to_string(),
            data_base64,
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("screenshot action returns screenshot result"),
        Err(diagnostic) => BrowserScreenshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn click(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
) -> BrowserActionResponse {
    browser_action_response(
        target,
        tab_id,
        "click",
        validate_point(x, y),
        BrowserCdpAction::Click { x, y },
    )
    .await
}

pub(crate) async fn type_text(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text: String,
) -> BrowserActionResponse {
    let validation = (!text.is_empty())
        .then_some(())
        .ok_or_else(invalid_text_diagnostic);
    browser_action_response(
        target,
        tab_id,
        "type_text",
        validation,
        BrowserCdpAction::TypeText { text },
    )
    .await
}

pub(crate) async fn press_key(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    key: String,
) -> BrowserActionResponse {
    let key = key.trim().to_string();
    let validation = (!key.is_empty())
        .then_some(())
        .ok_or_else(invalid_key_diagnostic);
    browser_action_response(
        target,
        tab_id,
        "press_key",
        validation,
        BrowserCdpAction::PressKey { key },
    )
    .await
}

pub(crate) async fn scroll(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    delta_x: f64,
    delta_y: f64,
    x: f64,
    y: f64,
) -> BrowserActionResponse {
    let validation = if delta_x.is_finite()
        && delta_y.is_finite()
        && x.is_finite()
        && y.is_finite()
        && x >= 0.0
        && y >= 0.0
    {
        Ok(())
    } else {
        Err(invalid_scroll_diagnostic())
    };
    browser_action_response(
        target,
        tab_id,
        "scroll",
        validation,
        BrowserCdpAction::Scroll {
            delta_x,
            delta_y,
            x,
            y,
        },
    )
    .await
}

async fn browser_action_response(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    action_name: &'static str,
    action_validation: Result<(), DiagnosticEntry>,
    action: BrowserCdpAction,
) -> BrowserActionResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let normalized_tab_id = validate_tab_id(tab_id).unwrap_or_default();
    let mut diagnostics = validate_action_target(resolved_target, &normalized_tab_id);
    if let Err(diagnostic) = action_validation {
        diagnostics.push(diagnostic);
    }
    if !diagnostics.is_empty() {
        return BrowserActionResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            action: action_name.to_string(),
            diagnostics,
        };
    }

    match run_cdp_action_from_bridge(resolved_target, &normalized_tab_id, action).await {
        Ok(BrowserCdpResult::Action) => BrowserActionResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            action: action_name.to_string(),
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("browser action returns action result"),
        Err(diagnostic) => BrowserActionResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            action: action_name.to_string(),
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn browser_bridge_diagnostics() -> Vec<DiagnosticEntry> {
    match browser_socket_selection_from_env() {
        Ok(selection) => bridge_readiness_diagnostic(selection)
            .await
            .into_iter()
            .collect(),
        Err(diagnostic) => vec![diagnostic],
    }
}

pub(crate) fn browser_env_values_present() -> std::collections::BTreeMap<String, String> {
    super::sockets::browser_env_values_present()
}

async fn bridge_readiness_diagnostic(selection: BrowserSocketSelection) -> Option<DiagnosticEntry> {
    let sockets = find_bridge_sockets(selection);
    if sockets.is_empty() {
        return Some(browser_bridge_disconnected_for_selection(selection));
    }

    first_responsive_bridge_socket(sockets).await.err()
}

async fn list_tabs_from_bridge(
    target: Option<BrowserTargetKind>,
) -> Result<BrowserListTabsResponse, DiagnosticEntry> {
    let selection = browser_socket_selection_from_env()?;
    let sockets = find_bridge_sockets(selection);
    if sockets.is_empty() {
        return Err(browser_bridge_disconnected_for_selection(selection));
    }

    let results = list_tabs_from_sockets(sockets, target).await;
    let mut tabs = Vec::new();
    let mut diagnostics = Vec::new();
    let mut connected_any = false;
    for (_, result) in results {
        match result {
            Ok(mut socket_tabs) => {
                connected_any = true;
                tabs.append(&mut socket_tabs);
            }
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    if connected_any {
        diagnostics.clear();
    }

    Ok(BrowserListTabsResponse {
        target,
        tabs,
        diagnostics,
    })
}

async fn open_tab_from_bridge(
    target: BrowserTargetKind,
    url: Option<String>,
) -> Result<BrowserOpenResponse, DiagnosticEntry> {
    let selection = browser_socket_selection_from_env()?;
    let sockets = find_bridge_sockets(selection);
    if sockets.is_empty() {
        return Err(browser_bridge_disconnected_for_selection(selection));
    }

    let deadline = TokioInstant::now() + BROWSER_OPEN_TIMEOUT;
    open_tab_from_sockets(sockets, target, url.as_deref(), deadline).await
}

async fn claim_tab_from_bridge(
    target: BrowserTargetKind,
    tab_id: &str,
) -> Result<BrowserClaimTabResponse, DiagnosticEntry> {
    let selection = browser_socket_selection_from_env()?;
    let sockets = find_bridge_sockets(selection);
    if sockets.is_empty() {
        return Err(browser_bridge_disconnected_for_selection(selection));
    }

    let deadline = TokioInstant::now() + BROWSER_OPEN_TIMEOUT;
    claim_tab_from_sockets(sockets, target, tab_id, deadline).await
}

async fn move_mouse_from_bridge(
    target: BrowserTargetKind,
    tab_id: &str,
    x: f64,
    y: f64,
    wait_for_arrival: bool,
) -> Result<BrowserMoveMouseResponse, DiagnosticEntry> {
    let selection = browser_socket_selection_from_env()?;
    let sockets = find_bridge_sockets(selection);
    if sockets.is_empty() {
        return Err(browser_bridge_disconnected_for_selection(selection));
    }

    let deadline = TokioInstant::now() + BROWSER_OPEN_TIMEOUT;
    move_mouse_from_sockets(sockets, target, tab_id, x, y, wait_for_arrival, deadline).await
}

async fn run_cdp_action_from_bridge(
    target: BrowserTargetKind,
    tab_id: &str,
    action: BrowserCdpAction,
) -> Result<BrowserCdpResult, DiagnosticEntry> {
    let selection = browser_socket_selection_from_env()?;
    let sockets = find_bridge_sockets(selection);
    if sockets.is_empty() {
        return Err(browser_bridge_disconnected_for_selection(selection));
    }

    let deadline = TokioInstant::now() + BROWSER_OPEN_TIMEOUT;
    cdp_action_from_sockets(sockets, target, tab_id, &action, deadline).await
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
