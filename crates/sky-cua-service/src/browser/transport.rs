use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::diagnostics::{bridge_timeout_diagnostic, unexpected_bridge_response_diagnostic};
use super::protocol::{read_frame, write_frame};

#[cfg(not(test))]
pub(super) const BRIDGE_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(test)]
pub(super) const BRIDGE_REQUEST_TIMEOUT: Duration = Duration::from_millis(100);

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
        TokioInstant::now() + BRIDGE_REQUEST_TIMEOUT,
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
                "timeoutMs": 10_000,
            }),
        ),
        deadline,
    )
    .await
}

pub(super) async fn connect_bridge_socket(socket: &Path) -> Result<UnixStream, DiagnosticEntry> {
    tokio::time::timeout(BRIDGE_REQUEST_TIMEOUT, UnixStream::connect(socket))
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
    tokio::time::timeout(BRIDGE_REQUEST_TIMEOUT, operation)
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
