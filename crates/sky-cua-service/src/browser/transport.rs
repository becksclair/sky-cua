use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::diagnostics::{bridge_timeout_diagnostic, unexpected_bridge_response_diagnostic};
use super::protocol::{read_frame, write_frame};

#[cfg(not(test))]
pub(super) fn bridge_request_timeout() -> Duration {
    Duration::from_secs(3)
}

/// Operator override (milliseconds) for the overall browser request budget and
/// the per-CDP-command cap. Slow or remote desktops where the extension /
/// native-host CDP relay is sluggish can raise this without changing the default
/// for everyone else. Returns `None` when unset or invalid so callers keep their
/// own default. Ignored under test so deadline math stays deterministic.
#[cfg(not(test))]
pub(super) fn browser_request_timeout_override_ms() -> Option<u64> {
    std::env::var("SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
}

#[cfg(test)]
pub(super) fn browser_request_timeout_override_ms() -> Option<u64> {
    None
}

/// Per-bridge-IO deadline. Generous under test so happy-path operations do not
/// trip it when `cargo test --workspace` starves the in-process fake servers and
/// a reply that normally takes microseconds takes seconds. Tests that need to
/// observe the deadline *firing* set `SKY_CUA_TEST_BRIDGE_REQUEST_TIMEOUT_MS` to a
/// small millisecond value so they stay fast.
#[cfg(test)]
pub(super) fn bridge_request_timeout() -> Duration {
    std::env::var("SKY_CUA_TEST_BRIDGE_REQUEST_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(10))
}

pub(super) async fn send_bridge_request(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    method: &'static str,
    params: Value,
) -> Result<Value, DiagnosticEntry> {
    send_bridge_request_until(
        stream,
        socket,
        request_id,
        method,
        params,
        TokioInstant::now() + bridge_request_timeout(),
    )
    .await
}

pub(super) async fn send_bridge_request_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    method: &'static str,
    params: Value,
    deadline: TokioInstant,
) -> Result<Value, DiagnosticEntry> {
    timeout_bridge_io_until(
        write_frame(
            stream,
            &json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": params,
            }),
        ),
        deadline,
        "send browser bridge request to",
        socket,
    )
    .await?;

    loop {
        let response = timeout_bridge_io_until(
            read_frame(stream),
            deadline,
            "read browser bridge response from",
            socket,
        )
        .await?;
        let Some(response) = response else {
            return Err(DiagnosticEntry {
                code: "BrowserBridgeDisconnected".to_string(),
                message: format!(
                    "Chrome extension/native-host browser socket closed before returning {method}."
                ),
                details: None,
            });
        };

        if response.get("method").and_then(Value::as_str) == Some("ping") {
            respond_to_ping(stream, &response, socket).await?;
            continue;
        }

        if response.get("id").and_then(Value::as_str) != Some(request_id) {
            if response.get("method").and_then(Value::as_str).is_some() {
                continue;
            }
            return Err(unexpected_bridge_response_diagnostic(response));
        }

        if let Some(error) = response.get("error") {
            return Err(DiagnosticEntry {
                code: "BrowserBridgeRequestFailed".to_string(),
                message: format!("Chrome extension/native-host {method} request failed: {error}"),
                details: None,
            });
        }

        return Ok(response);
    }
}

pub(super) async fn execute_cdp_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    tab_id: &Value,
    method: &'static str,
    command_params: Value,
    deadline: TokioInstant,
) -> Result<Value, DiagnosticEntry> {
    let timeout_ms = cdp_command_timeout_ms(deadline, TokioInstant::now());
    send_bridge_request_until(
        stream,
        socket,
        request_id,
        "executeCdp",
        merge_json(
            browser_session_params(),
            json!({
                "target": { "tabId": tab_id.clone() },
                "method": method,
                "commandParams": command_params,
                "timeoutMs": timeout_ms,
            }),
        ),
        deadline,
    )
    .await
}

/// The extension aborts a CDP command after `timeoutMs` and returns a
/// structured timeout error. That budget must expire before the service-side
/// read deadline: if the service abandons the read first, the caller loses
/// the structured error that recovery keys on, and the abandoned command
/// wedges the extension session without a reset. The floor therefore yields
/// to the remaining read budget: near an expired deadline the timer shrinks
/// below `MIN_COMMAND_TIMEOUT_MS` rather than outliving the read.
const MIN_COMMAND_TIMEOUT_MS: u64 = 250;

fn cdp_command_timeout_ms(deadline: TokioInstant, now: TokioInstant) -> u64 {
    const DEFAULT_MAX_COMMAND_TIMEOUT_MS: u64 = 10_000;
    // The per-command cap scales with the operator override so a raised overall
    // deadline actually reaches individual CDP commands on slow relays.
    let max_command_timeout_ms =
        browser_request_timeout_override_ms().unwrap_or(DEFAULT_MAX_COMMAND_TIMEOUT_MS);
    let remaining = deadline.checked_duration_since(now).unwrap_or_default();
    let remaining_ms = u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX);
    command_budget_ms(remaining_ms, max_command_timeout_ms)
}

/// Derive the per-command CDP budget from the remaining deadline and the cap.
///
/// The cap is floored at [`MIN_COMMAND_TIMEOUT_MS`] before clamping: an operator
/// override below the minimum (e.g. a mistyped
/// `SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS=200`) would otherwise make `min > max` and
/// panic `u64::clamp` on the first CDP command.
fn command_budget_ms(remaining_ms: u64, max_command_timeout_ms: u64) -> u64 {
    const RESPONSE_MARGIN_MS: u64 = 750;
    const READ_DEADLINE_HEADROOM_MS: u64 = 100;
    let max_command_timeout_ms = max_command_timeout_ms.max(MIN_COMMAND_TIMEOUT_MS);
    let budget = remaining_ms
        .saturating_sub(RESPONSE_MARGIN_MS)
        .clamp(MIN_COMMAND_TIMEOUT_MS, max_command_timeout_ms);
    budget.min(
        remaining_ms
            .saturating_sub(READ_DEADLINE_HEADROOM_MS)
            .max(1),
    )
}

pub(super) async fn connect_bridge_socket(socket: &Path) -> Result<UnixStream, DiagnosticEntry> {
    tokio::time::timeout(bridge_request_timeout(), UnixStream::connect(socket))
        .await
        .map_err(|_| bridge_timeout_diagnostic("connect to", socket))?
        .map_err(|error| DiagnosticEntry {
            code: "BrowserBridgeDisconnected".to_string(),
            message: format!(
                "Could not connect to Chrome extension/native-host browser socket {}: {error}",
                socket.display()
            ),
            details: None,
        })
}

pub(super) fn list_tabs_method() -> &'static str {
    "getUserTabs"
}

pub(super) fn browser_session_params() -> Value {
    json!({
        "session_id": "sky-cua-mcp",
        "turn_id": "browser-list-tabs",
    })
}

pub(super) fn merge_json(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Value::Object(extra)) = (base.as_object_mut(), extra) {
        for (key, value) in extra {
            base.insert(key, value);
        }
    }
    base
}

async fn respond_to_ping(
    stream: &mut UnixStream,
    request: &Value,
    socket: &Path,
) -> Result<(), DiagnosticEntry> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    timeout_bridge_io(
        write_frame(
            stream,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": "pong",
            }),
        ),
        "respond to ping on",
        socket,
    )
    .await
}

pub(super) async fn timeout_bridge_io<T>(
    operation: impl std::future::Future<Output = std::io::Result<T>>,
    action: &'static str,
    socket: &Path,
) -> Result<T, DiagnosticEntry> {
    tokio::time::timeout(bridge_request_timeout(), operation)
        .await
        .map_err(|_| bridge_timeout_diagnostic(action, socket))?
        .map_err(|error| DiagnosticEntry {
            code: "BrowserBridgeRequestFailed".to_string(),
            message: format!(
                "Could not {action} Chrome extension/native-host browser socket {}: {error}",
                socket.display()
            ),
            details: None,
        })
}

async fn timeout_bridge_io_until<T>(
    operation: impl std::future::Future<Output = std::io::Result<T>>,
    deadline: TokioInstant,
    action: &'static str,
    socket: &Path,
) -> Result<T, DiagnosticEntry> {
    let remaining = deadline
        .checked_duration_since(TokioInstant::now())
        .ok_or_else(|| bridge_timeout_diagnostic(action, socket))?;
    tokio::time::timeout(remaining, operation)
        .await
        .map_err(|_| bridge_timeout_diagnostic(action, socket))?
        .map_err(|error| DiagnosticEntry {
            code: "BrowserBridgeRequestFailed".to_string(),
            message: format!(
                "Could not {action} Chrome extension/native-host browser socket {}: {error}",
                socket.display()
            ),
            details: None,
        })
}

#[cfg(test)]
mod cdp_timeout_tests {
    use std::time::Duration;

    use tokio::time::Instant as TokioInstant;

    use super::{cdp_command_timeout_ms, command_budget_ms};

    #[test]
    fn sub_minimum_cap_is_floored_instead_of_panicking() {
        // A mistyped operator override below the 250ms floor must not invert the
        // clamp range (min > max) and panic; it is floored to the minimum.
        assert_eq!(command_budget_ms(60_000, 200), 250);
        assert_eq!(command_budget_ms(60_000, 1), 250);
        // A legitimately raised cap is still honored.
        assert_eq!(command_budget_ms(60_000, 30_000), 30_000);
    }

    #[test]
    fn generous_deadline_is_capped_at_extension_default() {
        let now = TokioInstant::now();
        assert_eq!(
            cdp_command_timeout_ms(now + Duration::from_secs(60), now),
            10_000
        );
    }

    #[test]
    fn command_timeout_expires_before_the_read_deadline() {
        let now = TokioInstant::now();
        let timeout = cdp_command_timeout_ms(now + Duration::from_secs(2), now);
        assert_eq!(timeout, 1_250);
    }

    #[test]
    fn short_deadline_keeps_the_floor_inside_the_read_budget() {
        let now = TokioInstant::now();
        assert_eq!(
            cdp_command_timeout_ms(now + Duration::from_millis(500), now),
            250
        );
    }

    #[test]
    fn near_expired_deadline_shrinks_below_the_floor() {
        // The command timer must never outlive the service-side read
        // deadline, so the 250ms floor yields when almost no budget remains.
        let now = TokioInstant::now();
        assert_eq!(cdp_command_timeout_ms(now, now), 1);
        assert_eq!(
            cdp_command_timeout_ms(now + Duration::from_millis(100), now),
            1
        );
        assert_eq!(
            cdp_command_timeout_ms(now + Duration::from_millis(200), now),
            100
        );
    }
}
