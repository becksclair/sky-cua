use std::time::Duration;

use sky_cua_platform::model::{
    BrowserActionResponse, BrowserClaimTabResponse, BrowserEvalResponse, BrowserListTabsResponse,
    BrowserMoveMouseResponse, BrowserNavigateResponse, BrowserOpenResponse,
    BrowserScreenshotResponse, BrowserSnapshotResponse, BrowserTargetKind, DiagnosticEntry,
    normalize_browser_open_url,
};
use tokio::time::Instant as TokioInstant;

use super::cdp::{BrowserCdpAction, BrowserCdpResult};
use super::diagnostics::{
    invalid_key_diagnostic, invalid_scroll_diagnostic, invalid_text_diagnostic,
    normalize_action_tab_id, unsupported_open_url_diagnostic, validate_open_url, validate_point,
    validate_tab_id,
};
use super::executor::BrowserBridgeExecutor;
use super::probe::first_responsive_bridge_socket;
use super::sockets::{
    BrowserSocketSelection, browser_bridge_disconnected_for_selection,
    browser_socket_selection_from_env, find_bridge_sockets,
};

/// Overall deadline for a browser bridge operation. Defaults to 12s but is
/// raised by `SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS` for slow or remote desktops
/// where the extension / native-host CDP relay is sluggish. Keep the default in
/// sync with `BROWSER_OPEN_TIMEOUT_MS` in `diagnostics.rs`.
#[cfg(not(test))]
pub(super) fn browser_open_timeout() -> Duration {
    Duration::from_millis(super::transport::browser_request_timeout_override_ms().unwrap_or(12_000))
}
// Short enough that the aggregate-deadline test stays fast, but with enough
// headroom that happy-path operation tests do not time out under scheduler load.
#[cfg(test)]
pub(super) fn browser_open_timeout() -> Duration {
    Duration::from_secs(2)
}

pub(crate) async fn list_tabs(target: Option<BrowserTargetKind>) -> BrowserListTabsResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);

    match BrowserBridgeExecutor::from_env(TokioInstant::now() + browser_open_timeout()) {
        Ok(executor) => executor.list_tabs(Some(resolved_target)).await,
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

    let executor =
        match BrowserBridgeExecutor::from_env(TokioInstant::now() + browser_open_timeout()) {
            Ok(executor) => executor,
            Err(diagnostic) => {
                return BrowserOpenResponse {
                    target: resolved_target,
                    tab: None,
                    diagnostics: vec![diagnostic],
                };
            }
        };

    match executor.open_tab(resolved_target, url.as_deref()).await {
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

    let executor =
        match BrowserBridgeExecutor::from_env(TokioInstant::now() + browser_open_timeout()) {
            Ok(executor) => executor,
            Err(diagnostic) => {
                return BrowserClaimTabResponse {
                    target: resolved_target,
                    tab: None,
                    diagnostics: vec![diagnostic],
                };
            }
        };

    match executor.claim_tab(resolved_target, &tab_id).await {
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
    let (normalized_tab_id, mut diagnostics) = normalize_action_tab_id(tab_id);
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

    let executor =
        match BrowserBridgeExecutor::from_env(TokioInstant::now() + browser_open_timeout()) {
            Ok(executor) => executor,
            Err(diagnostic) => {
                return BrowserMoveMouseResponse {
                    target: resolved_target,
                    tab_id: normalized_tab_id,
                    x,
                    y,
                    wait_for_arrival,
                    diagnostics: vec![diagnostic],
                };
            }
        };

    match executor
        .bind_tab(resolved_target, &normalized_tab_id)
        .move_mouse(x, y, wait_for_arrival)
        .await
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
    let (normalized_tab_id, mut diagnostics) = normalize_action_tab_id(tab_id);
    let normalized_url = normalize_browser_open_url(&url).unwrap_or_default();
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

    match run_cdp_action(
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
    text_limit: Option<usize>,
    element_offset: Option<usize>,
    element_limit: Option<usize>,
    element_query: Option<String>,
) -> BrowserSnapshotResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, diagnostics) = normalize_action_tab_id(tab_id);
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

    match run_cdp_action(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Snapshot {
            text_limit,
            element_offset,
            element_limit,
            element_query,
        },
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
    include_image_data: bool,
) -> BrowserScreenshotResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, diagnostics) = normalize_action_tab_id(tab_id);
    if !diagnostics.is_empty() {
        return BrowserScreenshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            screenshot_path: None,
            width: None,
            height: None,
            diagnostics,
        };
    }

    match run_cdp_action(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Screenshot,
    )
    .await
    {
        Ok(BrowserCdpResult::Screenshot {
            data_base64,
            css_width,
            css_height,
        }) => {
            let tab_id_for_task = normalized_tab_id.clone();
            let prepared = match tokio::task::spawn_blocking(move || {
                super::model_image::prepare_browser_capture(
                    &tab_id_for_task,
                    &data_base64,
                    css_width,
                    css_height,
                    include_image_data,
                )
            })
            .await
            {
                Ok(prepared) => prepared,
                Err(join_error) => {
                    return BrowserScreenshotResponse {
                        target: resolved_target,
                        tab_id: normalized_tab_id,
                        mime_type: "image/png".to_string(),
                        data_base64: String::new(),
                        screenshot_path: None,
                        width: None,
                        height: None,
                        diagnostics: vec![DiagnosticEntry {
                            code: "BrowserScreenshotDegraded".to_string(),
                            message: format!(
                                "Browser screenshot post-processing task failed to join \
                                 cleanly: {join_error}"
                            ),
                            details: None,
                        }],
                    };
                }
            };
            BrowserScreenshotResponse {
                target: resolved_target,
                tab_id: normalized_tab_id,
                mime_type: prepared.mime_type,
                data_base64: prepared.data_base64,
                screenshot_path: prepared.screenshot_path,
                width: (prepared.width > 0).then_some(prepared.width),
                height: (prepared.height > 0).then_some(prepared.height),
                diagnostics: prepared.diagnostics,
            }
        }
        Ok(_) => unreachable!("screenshot action returns screenshot result"),
        Err(diagnostic) => BrowserScreenshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            screenshot_path: None,
            width: None,
            height: None,
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
        Some((x, y)),
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
        None,
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
        None,
        BrowserCdpAction::PressKey { key },
    )
    .await
}

#[cfg(test)]
pub(crate) async fn eval(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    expression: String,
) -> BrowserEvalResponse {
    eval_with_policy(
        target,
        tab_id,
        expression,
        sky_cua_platform::model::browser_eval_enabled(),
    )
    .await
}

pub(crate) async fn eval_with_policy(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    expression: String,
    browser_eval_enabled: bool,
) -> BrowserEvalResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, mut diagnostics) = normalize_action_tab_id(tab_id);
    if !browser_eval_enabled {
        return BrowserEvalResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            value: None,
            diagnostics: vec![DiagnosticEntry {
                code: "BrowserEvalDisabled".to_string(),
                message: "browser_eval is disabled via SKY_CUA_BROWSER_EVAL. Remove the \
                          override, or set it to on, 1, or true, to re-enable \
                          page-JavaScript execution."
                    .to_string(),
                details: None,
            }],
        };
    }
    if expression.trim().is_empty() {
        diagnostics.push(invalid_text_diagnostic());
    }
    if !diagnostics.is_empty() {
        return BrowserEvalResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            value: None,
            diagnostics,
        };
    }

    match run_cdp_action(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Eval { expression },
    )
    .await
    {
        Ok(BrowserCdpResult::Eval { value }) => BrowserEvalResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            value,
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("eval action returns eval result"),
        Err(diagnostic) => BrowserEvalResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            value: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn scroll(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    delta_x: f64,
    delta_y: f64,
    x: Option<f64>,
    y: Option<f64>,
) -> BrowserActionResponse {
    let coordinates = match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 => {
            Ok(Some((x, y)))
        }
        (None, None) => Ok(None),
        _ => Err(invalid_scroll_diagnostic()),
    };
    let validation = if delta_x.is_finite()
        && delta_y.is_finite()
        && (delta_x != 0.0 || delta_y != 0.0)
        && coordinates.is_ok()
    {
        Ok(())
    } else {
        Err(invalid_scroll_diagnostic())
    };
    let coordinates = coordinates.ok().flatten();
    let (x, y) = coordinates
        .map(|(x, y)| (Some(x), Some(y)))
        .unwrap_or((None, None));
    browser_action_response(
        target,
        tab_id,
        "scroll",
        validation,
        coordinates,
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
    cursor_before_action: Option<(f64, f64)>,
    action: BrowserCdpAction,
) -> BrowserActionResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, mut diagnostics) = normalize_action_tab_id(tab_id);
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

    let executor =
        match BrowserBridgeExecutor::from_env(TokioInstant::now() + browser_open_timeout()) {
            Ok(executor) => executor,
            Err(diagnostic) => {
                return BrowserActionResponse {
                    target: resolved_target,
                    tab_id: normalized_tab_id,
                    action: action_name.to_string(),
                    diagnostics: vec![diagnostic],
                };
            }
        };
    let binding = executor.bind_tab(resolved_target, &normalized_tab_id);

    if let Some((x, y)) = cursor_before_action
        && let Err(diagnostic) = binding.move_mouse(x, y, true).await
    {
        return BrowserActionResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            action: action_name.to_string(),
            diagnostics: vec![diagnostic],
        };
    }

    match binding.run_cdp(action).await {
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

async fn run_cdp_action(
    target: BrowserTargetKind,
    tab_id: &str,
    action: BrowserCdpAction,
) -> Result<BrowserCdpResult, DiagnosticEntry> {
    let executor = BrowserBridgeExecutor::from_env(TokioInstant::now() + browser_open_timeout())?;
    executor.bind_tab(target, tab_id).run_cdp(action).await
}
