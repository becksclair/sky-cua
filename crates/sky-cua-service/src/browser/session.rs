use std::path::Path;

use serde_json::{Value, json};
use sky_cua_platform::model::{
    BrowserClaimTabResponse, BrowserOpenResponse, BrowserTab, BrowserTargetKind, DiagnosticEntry,
};
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::protocol::{
    ATTACH_TAB_REQUEST_ID, ATTACH_TAB_RETRY_REQUEST_ID, CLAIM_TAB_REQUEST_ID,
    CLAIM_TAB_RETRY_REQUEST_ID, DETACH_TAB_FOR_RETRY_REQUEST_ID, ENABLE_PAGE_REQUEST_ID,
    ENABLE_PAGE_RETRY_REQUEST_ID, MOVE_MOUSE_REQUEST_ID, NAVIGATE_REQUEST_ID, OPEN_TAB_REQUEST_ID,
    RECLAIM_SESSION_TABS_REQUEST_ID, RECOVER_ATTACH_TAB_REQUEST_ID, RECOVER_CLAIM_TAB_REQUEST_ID,
    RECOVER_CLAIM_TAB_RETRY_REQUEST_ID, RECOVER_ENABLE_PAGE_REQUEST_ID,
};
use super::tabs::{parse_single_tab, tab_id_value};
use super::transport::{
    browser_session_params, execute_cdp_until, merge_json, send_bridge_request_until,
};

pub(super) async fn open_tab_from_socket(
    socket: &Path,
    target: BrowserTargetKind,
    url: Option<&str>,
    deadline: TokioInstant,
    mut stream: UnixStream,
) -> Result<BrowserOpenResponse, DiagnosticEntry> {
    let created = send_bridge_request_until(
        &mut stream,
        socket,
        OPEN_TAB_REQUEST_ID,
        "createTab",
        browser_session_params(),
        deadline,
    )
    .await?;
    let Some(mut tab) = parse_single_tab(created.get("result"), target) else {
        return Err(DiagnosticEntry {
            code: "BrowserBridgeRequestFailed".to_string(),
            message: "Chrome extension/native-host createTab response did not include a tab id."
                .to_string(),
            details: None,
        });
    };
    let tab_id = tab.tab_id.clone();
    let tab_id_value = tab_id_value(&tab_id);

    if let Err((failed_step, diagnostic)) =
        attach_and_enable_open_tab_until(&mut stream, socket, &tab_id_value, deadline).await
    {
        return Ok(partial_open_response(target, tab, failed_step, diagnostic));
    }

    if let Some(url) = url {
        let response = match execute_cdp_until(
            &mut stream,
            socket,
            NAVIGATE_REQUEST_ID,
            &tab_id_value,
            "Page.navigate",
            json!({ "url": url }),
            deadline,
        )
        .await
        {
            Ok(response) => response,
            Err(diagnostic) => {
                return Ok(partial_open_response(target, tab, "navigate", diagnostic));
            }
        };
        if let Some(error_text) = response
            .get("result")
            .and_then(|result| result.get("errorText"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(partial_open_response(
                target,
                tab,
                "navigate",
                DiagnosticEntry {
                    code: "BrowserNavigationFailed".to_string(),
                    message: format!("Browser navigation failed: {error_text}"),
                    details: None,
                },
            ));
        }
        tab.url = Some(url.to_string());
    }

    Ok(BrowserOpenResponse {
        target,
        tab: Some(tab),
        diagnostics: Vec::new(),
    })
}

pub(super) async fn claim_tab_from_socket(
    socket: &Path,
    target: BrowserTargetKind,
    tab_id: &str,
    deadline: TokioInstant,
    mut stream: UnixStream,
) -> Result<BrowserClaimTabResponse, DiagnosticEntry> {
    let requested_tab_id_value = tab_id_value(tab_id);
    let claimed = claim_user_tab_with_stale_sky_cua_reclaim_until(
        &mut stream,
        socket,
        CLAIM_TAB_REQUEST_ID,
        CLAIM_TAB_RETRY_REQUEST_ID,
        requested_tab_id_value.clone(),
        deadline,
    )
    .await?;
    let Some(tab) = parse_single_tab(claimed.get("result"), target) else {
        return Err(DiagnosticEntry {
            code: "BrowserBridgeRequestFailed".to_string(),
            message: "Chrome extension/native-host claimUserTab response did not include a tab id."
                .to_string(),
            details: None,
        });
    };
    let tab_id_value = tab_id_value(&tab.tab_id);

    if let Err(diagnostic) =
        attach_and_enable_tab_until(&mut stream, socket, &tab_id_value, deadline).await
    {
        return Ok(partial_claim_response(target, tab, diagnostic));
    }

    Ok(BrowserClaimTabResponse {
        target,
        tab: Some(tab),
        diagnostics: Vec::new(),
    })
}

async fn claim_user_tab_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    tab_id: Value,
    deadline: TokioInstant,
) -> Result<Value, DiagnosticEntry> {
    send_bridge_request_until(
        stream,
        socket,
        request_id,
        "claimUserTab",
        merge_json(
            browser_session_params(),
            json!({
                "tabId": tab_id,
            }),
        ),
        deadline,
    )
    .await
}

async fn claim_user_tab_with_stale_sky_cua_reclaim_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    retry_request_id: &'static str,
    tab_id: Value,
    deadline: TokioInstant,
) -> Result<Value, DiagnosticEntry> {
    match claim_user_tab_until(stream, socket, request_id, tab_id.clone(), deadline).await {
        Ok(claimed) => Ok(claimed),
        Err(diagnostic) => {
            let Some(stale_session_id) = stale_sky_cua_owner_session_from_claim_error(&diagnostic)
            else {
                return Err(diagnostic);
            };
            finalize_stale_sky_cua_session_until(stream, socket, &stale_session_id, deadline)
                .await?;
            claim_user_tab_until(stream, socket, retry_request_id, tab_id, deadline).await
        }
    }
}

async fn finalize_stale_sky_cua_session_until(
    stream: &mut UnixStream,
    socket: &Path,
    stale_session_id: &str,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
    send_bridge_request_until(
        stream,
        socket,
        RECLAIM_SESSION_TABS_REQUEST_ID,
        "finalizeTabs",
        json!({
            "session_id": stale_session_id,
            "turn_id": "sky-cua-reclaim-stale-tabs",
            "keep": [],
        }),
        deadline,
    )
    .await
    .map(|_| ())
}

async fn attach_and_enable_tab_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
    attach_tab_until(stream, socket, ATTACH_TAB_REQUEST_ID, tab_id, deadline).await?;
    match enable_page_until(stream, socket, ENABLE_PAGE_REQUEST_ID, tab_id, deadline).await {
        Ok(()) => Ok(()),
        Err(diagnostic) if is_debugger_unattached_diagnostic(&diagnostic) => {
            let _ = detach_tab_until(
                stream,
                socket,
                DETACH_TAB_FOR_RETRY_REQUEST_ID,
                tab_id,
                deadline,
            )
            .await;
            attach_tab_until(
                stream,
                socket,
                ATTACH_TAB_RETRY_REQUEST_ID,
                tab_id,
                deadline,
            )
            .await?;
            enable_page_until(
                stream,
                socket,
                ENABLE_PAGE_RETRY_REQUEST_ID,
                tab_id,
                deadline,
            )
            .await
        }
        Err(diagnostic) => Err(diagnostic),
    }
}

async fn attach_and_enable_open_tab_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<(), (&'static str, DiagnosticEntry)> {
    attach_tab_until(stream, socket, ATTACH_TAB_REQUEST_ID, tab_id, deadline)
        .await
        .map_err(|diagnostic| ("attach", diagnostic))?;
    enable_page_until(stream, socket, ENABLE_PAGE_REQUEST_ID, tab_id, deadline)
        .await
        .map_err(|diagnostic| ("enable page", diagnostic))
}

async fn attach_tab_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
    send_bridge_request_until(
        stream,
        socket,
        request_id,
        "attach",
        merge_json(
            browser_session_params(),
            json!({
                "tabId": tab_id.clone(),
            }),
        ),
        deadline,
    )
    .await?;
    Ok(())
}

async fn detach_tab_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
    send_bridge_request_until(
        stream,
        socket,
        request_id,
        "detach",
        merge_json(
            browser_session_params(),
            json!({
                "tabId": tab_id.clone(),
            }),
        ),
        deadline,
    )
    .await?;
    Ok(())
}

async fn enable_page_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
    execute_cdp_until(
        stream,
        socket,
        request_id,
        tab_id,
        "Page.enable",
        json!({}),
        deadline,
    )
    .await?;
    Ok(())
}

fn is_debugger_unattached_diagnostic(diagnostic: &DiagnosticEntry) -> bool {
    diagnostic.code == "BrowserBridgeRequestFailed"
        && debugger_unattached_message(&diagnostic.message)
}

pub(super) fn is_recoverable_cdp_session_diagnostic(diagnostic: &DiagnosticEntry) -> bool {
    diagnostic.code == "BrowserBridgeRequestFailed"
        && (debugger_unattached_message(&diagnostic.message)
            || diagnostic.message.contains("not part of browser session"))
}

fn debugger_unattached_message(message: &str) -> bool {
    message.contains("Debugger is not attached")
        || message.contains("Debugger unattached")
        || message.contains("Detached while handling command")
}

pub(super) async fn recover_cdp_session_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
    claim_user_tab_with_stale_sky_cua_reclaim_until(
        stream,
        socket,
        RECOVER_CLAIM_TAB_REQUEST_ID,
        RECOVER_CLAIM_TAB_RETRY_REQUEST_ID,
        tab_id.clone(),
        deadline,
    )
    .await?;
    let _ = detach_tab_until(
        stream,
        socket,
        DETACH_TAB_FOR_RETRY_REQUEST_ID,
        tab_id,
        deadline,
    )
    .await;
    attach_tab_until(
        stream,
        socket,
        RECOVER_ATTACH_TAB_REQUEST_ID,
        tab_id,
        deadline,
    )
    .await?;
    enable_page_until(
        stream,
        socket,
        RECOVER_ENABLE_PAGE_REQUEST_ID,
        tab_id,
        deadline,
    )
    .await
}

pub(super) async fn move_mouse_on_stream(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id_value: &Value,
    x: f64,
    y: f64,
    wait_for_arrival: bool,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
    send_bridge_request_until(
        stream,
        socket,
        MOVE_MOUSE_REQUEST_ID,
        "moveMouse",
        merge_json(
            browser_session_params(),
            json!({
                "tabId": tab_id_value.clone(),
                "x": x,
                "y": y,
                "waitForArrival": wait_for_arrival,
            }),
        ),
        deadline,
    )
    .await?;
    Ok(())
}

fn partial_open_response(
    target: BrowserTargetKind,
    tab: BrowserTab,
    failed_step: &str,
    diagnostic: DiagnosticEntry,
) -> BrowserOpenResponse {
    BrowserOpenResponse {
        target,
        diagnostics: vec![DiagnosticEntry {
            code: "BrowserOpenPartial".to_string(),
            message: format!(
                "Created browser tab {}, but browser_open could not complete {failed_step}: {}",
                tab.tab_id, diagnostic.message
            ),
            details: Some(format!(
                "source_code={}{}",
                diagnostic.code,
                diagnostic
                    .details
                    .as_deref()
                    .map(|details| format!(" source_details={details}"))
                    .unwrap_or_default()
            )),
        }],
        tab: Some(tab),
    }
}

fn partial_claim_response(
    target: BrowserTargetKind,
    tab: BrowserTab,
    diagnostic: DiagnosticEntry,
) -> BrowserClaimTabResponse {
    BrowserClaimTabResponse {
        target,
        diagnostics: vec![DiagnosticEntry {
            code: "BrowserClaimPartial".to_string(),
            message: format!(
                "Claimed browser tab {}, but browser_claim_tab could not attach it for browser actions: {}",
                tab.tab_id, diagnostic.message
            ),
            details: Some(format!(
                "source_code={}{}",
                diagnostic.code,
                diagnostic
                    .details
                    .as_deref()
                    .map(|details| format!(" source_details={details}"))
                    .unwrap_or_default()
            )),
        }],
        tab: Some(tab),
    }
}

fn stale_sky_cua_owner_session_from_claim_error(diagnostic: &DiagnosticEntry) -> Option<String> {
    if diagnostic.code != "BrowserBridgeRequestFailed" {
        return None;
    }
    let session_id = diagnostic
        .message
        .split("already part of browser session ")
        .nth(1)?
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .collect::<String>();

    if session_id.starts_with("sky-cua-") && session_id != "sky-cua-mcp" {
        Some(session_id)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::debugger_unattached_message;

    #[test]
    fn debugger_unattached_message_accepts_live_extension_wordings() {
        assert!(debugger_unattached_message(
            "Debugger is not attached to the tab with id: 515."
        ));
        assert!(debugger_unattached_message("Debugger unattached"));
        assert!(debugger_unattached_message(
            "Detached while handling command."
        ));
        assert!(!debugger_unattached_message("Navigation failed"));
    }
}
