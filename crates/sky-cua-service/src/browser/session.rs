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
    RECLAIM_SESSION_TABS_REQUEST_ID, RECOVER_CLAIM_TAB_REQUEST_ID,
    RECOVER_CLAIM_TAB_RETRY_REQUEST_ID, RECOVER_ENABLE_PAGE_REQUEST_ID,
    RECOVER_WAKE_TAB_REQUEST_ID, WAKE_TAB_REQUEST_ID,
};
use super::tabs::{parse_single_tab, tab_id_value};
use super::transport::{
    browser_session_params, execute_cdp_capped_until, execute_cdp_until, merge_json,
    send_bridge_request_until,
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
    // The first enable is capped too: an existing user tab can be discarded, and
    // burning the extension's full 10s default on its hang would starve the
    // wake retry below (the same budget asymmetry the executor recovery path
    // avoids). Safe for slow relays: the extension's timeoutMs measures
    // browser-side execution, not relay latency, and a live tab answers
    // Page.enable in milliseconds.
    match enable_page_capped_until(stream, socket, ENABLE_PAGE_REQUEST_ID, tab_id, deadline).await {
        Ok(()) => Ok(()),
        Err(diagnostic)
            if is_debugger_unattached_diagnostic(&diagnostic)
                || is_cdp_command_timeout_diagnostic(&diagnostic) =>
        {
            // A Page.enable timeout right after a successful attach is the
            // discarded-tab signature; a timed-out enable also wedges the
            // session, so the reset below is required before any retry.
            let wake_first = is_cdp_command_timeout_diagnostic(&diagnostic);
            reset_wake_and_enable_until(
                stream,
                socket,
                tab_id,
                deadline,
                wake_first,
                WAKE_TAB_REQUEST_ID,
                ENABLE_PAGE_RETRY_REQUEST_ID,
            )
            .await
        }
        Err(diagnostic) => Err(diagnostic),
    }
}

/// Budget ceiling for `Page.enable` during wedge recovery. A live tab answers
/// it in milliseconds; only a discarded tab's missing renderer makes it hang,
/// so a hang past this cap is the discarded-tab signal. Kept far below the
/// extension's 10s default so the wake + final enable still fit inside the
/// overall operation deadline.
const RECOVERY_ENABLE_TIMEOUT_CAP_MS: u64 = 4_000;

/// Detach (best-effort) and re-attach a tab's debugger session — the only
/// reset that clears a stuck timed-out CDP command wedging the session.
async fn reset_tab_session_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
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
    .await
}

/// Reset a tab's debugger session (detach/attach), then enable the page
/// domain, waking a discarded tab when needed. With `wake_first` (the caller
/// already saw a CDP command timeout — the discarded-tab signature) the wake
/// precedes the enable. Otherwise the wake happens lazily: if the capped
/// enable itself times out, that timeout wedges the fresh session, so the
/// session is reset once more, the tab woken, and the enable retried.
async fn reset_wake_and_enable_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    deadline: TokioInstant,
    wake_first: bool,
    wake_request_id: &'static str,
    enable_request_id: &'static str,
) -> Result<(), DiagnosticEntry> {
    reset_tab_session_until(stream, socket, tab_id, deadline).await?;
    if wake_first {
        let _ = wake_tab_until(stream, socket, wake_request_id, tab_id, deadline).await;
    }
    match enable_page_capped_until(stream, socket, enable_request_id, tab_id, deadline).await {
        Ok(()) => Ok(()),
        Err(diagnostic) if !wake_first && is_cdp_command_timeout_diagnostic(&diagnostic) => {
            reset_tab_session_until(stream, socket, tab_id, deadline).await?;
            let _ = wake_tab_until(stream, socket, wake_request_id, tab_id, deadline).await;
            enable_page_capped_until(stream, socket, enable_request_id, tab_id, deadline)
                .await
                .map_err(with_sleeping_tab_details)
        }
        Err(diagnostic) if wake_first && is_cdp_command_timeout_diagnostic(&diagnostic) => {
            Err(with_sleeping_tab_details(diagnostic))
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

/// `enable_page_until` with the recovery-time budget cap; see
/// [`RECOVERY_ENABLE_TIMEOUT_CAP_MS`].
async fn enable_page_capped_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<(), DiagnosticEntry> {
    execute_cdp_capped_until(
        stream,
        socket,
        request_id,
        tab_id,
        "Page.enable",
        json!({}),
        deadline,
        RECOVERY_ENABLE_TIMEOUT_CAP_MS,
    )
    .await?;
    Ok(())
}

fn is_debugger_unattached_diagnostic(diagnostic: &DiagnosticEntry) -> bool {
    diagnostic.code == "BrowserBridgeRequestFailed"
        && debugger_unattached_message(&diagnostic.message)
}

/// Bridge failures that a detach/attach session reset can cure; the executor
/// recovers once and (when safe) retries the operation on the same socket.
pub(super) fn is_recoverable_cdp_session_diagnostic(diagnostic: &DiagnosticEntry) -> bool {
    if diagnostic.code != "BrowserBridgeRequestFailed" {
        return false;
    }
    debugger_unattached_message(&diagnostic.message)
        || diagnostic.message.contains("not part of browser session")
        || is_cdp_command_timeout_diagnostic(diagnostic)
}

/// The extension abandons a CDP command after its `timeoutMs` budget but
/// cannot cancel it, so the command may still execute and stays outstanding
/// in the tab's debugger session, wedging every later command on that
/// session. Detaching and re-attaching the debugger is the only reset that
/// clears the stuck command. Callers must not blindly replay the failed
/// operation: unlike the unattached/not-in-session causes, a timeout does
/// not prove the command never ran.
pub(super) fn is_cdp_command_timeout_diagnostic(diagnostic: &DiagnosticEntry) -> bool {
    diagnostic.code == "BrowserBridgeRequestFailed"
        && diagnostic.message.contains("waiting for CDP command")
}

/// A tab-not-found answer is the one bridge failure that proves the tab does
/// not exist on the queried browser at all, so trying another bridge socket
/// is legitimate. Every other failure means the socket engaged with the tab
/// (or failed for reasons unrelated to socket choice) and must not trigger a
/// cross-socket retry of the operation. Both wordings are Chrome API errors
/// passed through the extension: `chrome.tabs` says "No tab with id: X.",
/// `chrome.debugger` says "No tab with given id X.".
pub(super) fn is_tab_not_found_diagnostic(diagnostic: &DiagnosticEntry) -> bool {
    diagnostic.code == "BrowserBridgeRequestFailed"
        && (diagnostic.message.contains("No tab with id")
            || diagnostic.message.contains("No tab with given id"))
}

fn debugger_unattached_message(message: &str) -> bool {
    message.contains("Debugger is not attached")
        || message.contains("Debugger unattached")
        || message.contains("Detached while handling command")
}

/// `wake_tab` is true when the triggering failure was a CDP command timeout
/// (`is_cdp_command_timeout_diagnostic`) — the signature of a discarded
/// (asleep) tab whose renderer is gone — so the wake precedes the re-enable.
/// A discarded tab can also enter recovery via `Debugger unattached` (a
/// never-attached sleeping tab); that case is caught lazily when the capped
/// recovery `Page.enable` itself times out. The wake activates the tab — a
/// user-visible tab switch — so it never runs for recoveries whose enable
/// succeeds (healthy background tabs stay in the background).
pub(super) async fn recover_cdp_session_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    deadline: TokioInstant,
    wake_tab: bool,
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
    reset_wake_and_enable_until(
        stream,
        socket,
        tab_id,
        deadline,
        wake_tab,
        RECOVER_WAKE_TAB_REQUEST_ID,
        RECOVER_ENABLE_PAGE_REQUEST_ID,
    )
    .await
}

/// Wake a discarded (asleep) tab. `Page.bringToFront` is handled in the
/// browser process, so it succeeds even when the tab has no renderer — and
/// activating a discarded tab makes Chrome reload it, after which
/// renderer-bound commands (`Page.enable`, `Runtime.evaluate`, input) work
/// again. It is the only wake primitive the extension bridge exposes (no
/// tabs.reload/update relay exists). Live-verified against Brave's sleeping
/// tabs on 2026-07-08. Failures are ignored by callers: the follow-up
/// `Page.enable` reports the truth either way.
async fn wake_tab_until(
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
        "Page.bringToFront",
        json!({}),
        deadline,
    )
    .await?;
    Ok(())
}

fn with_sleeping_tab_details(diagnostic: DiagnosticEntry) -> DiagnosticEntry {
    DiagnosticEntry {
        details: Some(
            "The tab's renderer did not respond, which usually means the tab was \
             discarded (asleep). It was activated to wake it; retry the operation, \
             or reopen the page in a fresh tab with browser_open."
                .to_string(),
        ),
        ..diagnostic
    }
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
    use sky_cua_platform::model::DiagnosticEntry;

    use super::{debugger_unattached_message, is_recoverable_cdp_session_diagnostic};

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

    #[test]
    fn cdp_command_timeout_is_recoverable() {
        let diagnostic = DiagnosticEntry {
            code: "BrowserBridgeRequestFailed".to_string(),
            message: "Chrome extension/native-host executeCdp request failed: \
                      {\"code\":1,\"message\":\"Timed out after 10000ms waiting for \
                      CDP command Page.captureScreenshot.\"}"
                .to_string(),
            details: None,
        };
        assert!(is_recoverable_cdp_session_diagnostic(&diagnostic));
    }

    #[test]
    fn tab_not_found_accepts_both_chrome_api_wordings() {
        let diagnostic = |message: &str| DiagnosticEntry {
            code: "BrowserBridgeRequestFailed".to_string(),
            message: format!("Chrome extension/native-host executeCdp request failed: {message}"),
            details: None,
        };
        // chrome.tabs wording.
        assert!(super::is_tab_not_found_diagnostic(&diagnostic(
            "No tab with id: 515."
        )));
        // chrome.debugger wording.
        assert!(super::is_tab_not_found_diagnostic(&diagnostic(
            "No tab with given id 515."
        )));
        assert!(!super::is_tab_not_found_diagnostic(&diagnostic(
            "Tab 515 is not part of browser session sky-cua-mcp"
        )));
    }

    #[test]
    fn unrelated_bridge_failure_is_not_recoverable() {
        let diagnostic = DiagnosticEntry {
            code: "BrowserBridgeRequestFailed".to_string(),
            message: "Chrome extension/native-host executeCdp request failed: \
                      {\"code\":1,\"message\":\"Navigation failed\"}"
                .to_string(),
            details: None,
        };
        assert!(!is_recoverable_cdp_session_diagnostic(&diagnostic));
    }
}
