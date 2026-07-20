use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sky_cua_platform::model::BrowserInstanceStability;
use tokio::io::AsyncWrite;
use tokio::net::UnixStream;
use tokio::sync::broadcast;
use tokio::time::Instant;

use super::{
    BridgeActorConfig, BridgeActorError, BridgeActorEvent, BridgeRequestSize, OperationClass,
    Runtime,
};
use crate::browser::protocol::{
    HOST_HELLO_METHOD, HOST_PROTOCOL_VERSION, HOST_SETTLEMENT_METHOD,
    HOST_SETTLEMENT_UNKNOWN_METHOD, read_frame, write_frame,
};

const REQUIRED_CAPABILITIES: &[&str] = &[
    "control_plane",
    "heartbeat",
    "extension_events",
    "private_param_stripping",
    "settlements",
    "settlement_ack",
    "side_panel_requests",
    "owner_release",
];
pub(super) const HEARTBEAT_DEADLINE: Duration = Duration::from_secs(3);
pub(super) const HOST_RELEASE_METHOD: &str = "skyCuaHost/release";
pub(super) const HOST_SETTLEMENT_ACK_METHOD: &str = "skyCuaHost/settlementAck";

pub(super) struct Handshake {
    pub(super) host_instance_id: String,
    pub(super) browser_instance_id: Option<String>,
    pub(super) browser_instance_stability: BrowserInstanceStability,
}

pub(super) async fn perform_handshake(
    stream: &mut UnixStream,
    config: &BridgeActorConfig,
    runtime: &mut Runtime,
    events: &broadcast::Sender<BridgeActorEvent>,
) -> Result<Handshake, BridgeActorError> {
    let request_id = runtime.allocate_request_id(config);
    let hello = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": HOST_HELLO_METHOD,
        "params": {
            "protocol_version": HOST_PROTOCOL_VERSION,
            "client_role": "control_plane",
            "daemon_generation": config.daemon_generation,
            "owner_mode": config.owner_mode.as_str(),
            "capabilities": REQUIRED_CAPABILITIES,
        }
    });
    write_frame_bounded(stream, &hello, config.write_timeout).await?;
    let deadline = Instant::now() + config.handshake_timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let frame = tokio::time::timeout(remaining, read_frame(stream))
            .await
            .map_err(|_| BridgeActorError::TimedOut)?
            .map_err(|error| BridgeActorError::RequestFailed(error.to_string()))?
            .ok_or(BridgeActorError::Disconnected)?;
        if is_ping(&frame) {
            write_pong(stream, &frame).await?;
            continue;
        }
        if frame.get("id").and_then(Value::as_str) != Some(request_id.as_str()) {
            route_notification(events, frame);
            continue;
        }
        if let Some(error) = frame.get("error") {
            return Err(BridgeActorError::Unavailable(format!(
                "host hello rejected: {error}"
            )));
        }
        let result = frame
            .get("result")
            .and_then(Value::as_object)
            .ok_or_else(|| BridgeActorError::Unavailable("invalid host hello response".into()))?;
        if result.get("protocol_version").and_then(Value::as_u64) != Some(HOST_PROTOCOL_VERSION) {
            return Err(BridgeActorError::Unavailable(
                "native host protocol capability mismatch".into(),
            ));
        }
        let negotiated_owner_mode = result.get("mode").and_then(Value::as_str);
        if negotiated_owner_mode != Some(config.owner_mode.as_str()) {
            return Err(BridgeActorError::Unavailable(format!(
                "native host owner mode mismatch: requested {}, negotiated {}",
                config.owner_mode.as_str(),
                negotiated_owner_mode.unwrap_or("<missing or invalid>")
            )));
        }
        let capabilities = result
            .get("capabilities")
            .and_then(Value::as_array)
            .ok_or_else(|| BridgeActorError::Unavailable("host omitted capabilities".into()))?;
        let missing = REQUIRED_CAPABILITIES
            .iter()
            .filter(|required| {
                !capabilities
                    .iter()
                    .any(|capability| capability.as_str() == Some(**required))
            })
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(BridgeActorError::Unavailable(format!(
                "native host capability mismatch; missing {}",
                missing.join(",")
            )));
        }
        let host_instance_id = result
            .get("host_instance_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| BridgeActorError::Unavailable("host omitted instance id".into()))?
            .to_owned();
        let browser_instance_stability = match result
            .get("browser_instance_stability")
            .and_then(Value::as_str)
        {
            Some("stable") => BrowserInstanceStability::Stable,
            Some("connection_only") => BrowserInstanceStability::ConnectionOnly,
            _ => BrowserInstanceStability::Unavailable,
        };
        return Ok(Handshake {
            host_instance_id,
            browser_instance_id: result
                .get("browser_instance_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            browser_instance_stability,
        });
    }
}

pub(super) async fn write_pong(
    stream: &mut UnixStream,
    ping: &Value,
) -> Result<(), BridgeActorError> {
    let id = ping.get("id").cloned().unwrap_or(Value::Null);
    write_frame_bounded(
        stream,
        &json!({ "jsonrpc": "2.0", "id": id, "result": "pong" }),
        HEARTBEAT_DEADLINE,
    )
    .await
}

pub(super) async fn write_frame_bounded(
    stream: &mut (impl AsyncWrite + Unpin),
    frame: &Value,
    timeout: Duration,
) -> Result<(), BridgeActorError> {
    tokio::time::timeout(timeout, write_frame(stream, frame))
        .await
        .map_err(|_| BridgeActorError::TimedOut)?
        .map_err(|error| BridgeActorError::RequestFailed(error.to_string()))
}

pub(super) fn route_notification(events: &broadcast::Sender<BridgeActorEvent>, frame: Value) {
    match frame.get("method").and_then(Value::as_str) {
        Some(HOST_SETTLEMENT_METHOD) => {
            let unknown = frame.pointer("/params/status").and_then(Value::as_str)
                == Some("settlement_unknown");
            let _ = if unknown {
                events.send(BridgeActorEvent::SettlementUnknown(frame))
            } else {
                events.send(BridgeActorEvent::Settlement(frame))
            };
        }
        Some(HOST_SETTLEMENT_UNKNOWN_METHOD) => {
            let _ = events.send(BridgeActorEvent::SettlementUnknown(frame));
        }
        _ => {
            let _ = events.send(BridgeActorEvent::Extension(frame));
        }
    }
}

pub(super) fn settlement_ack_frame(
    settlement: &Value,
    acknowledging_daemon_generation: &str,
) -> Result<Value, BridgeActorError> {
    let params = settlement
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            BridgeActorError::InvalidPayload("settlement params must be an object".into())
        })?;
    let required = |field: &str| {
        params
            .get(field)
            .cloned()
            .ok_or_else(|| BridgeActorError::InvalidPayload(format!("settlement omitted {field}")))
    };
    Ok(json!({
        "jsonrpc": "2.0",
        "method": HOST_SETTLEMENT_ACK_METHOD,
        "params": {
            "operation_id": required("operation_id")?,
            "daemon_generation": required("daemon_generation")?,
            "actor_generation": required("actor_generation")?,
            "chrome_request_id": required("chrome_request_id")?,
            "acknowledging_daemon_generation": acknowledging_daemon_generation,
        }
    }))
}

pub(super) fn request_size(method: &str, params: &Value) -> BridgeRequestSize {
    let cdp_method = params.get("method").and_then(Value::as_str);
    if matches!(method, "screenshot" | "captureScreenshot")
        || cdp_method == Some("Page.captureScreenshot")
    {
        BridgeRequestSize::LargeFrame
    } else {
        BridgeRequestSize::Ordinary
    }
}

pub(super) fn operation_class_name(class: OperationClass) -> &'static str {
    match class {
        OperationClass::ReadOnly => "read_only",
        OperationClass::AbsoluteSet => "absolute_set",
        OperationClass::Mutation => "mutation",
        OperationClass::BrowserGlobal => "browser_global",
    }
}

pub(super) fn requires_settlement(class: OperationClass) -> bool {
    matches!(
        class,
        OperationClass::Mutation | OperationClass::BrowserGlobal
    )
}

pub(super) fn is_ping(frame: &Value) -> bool {
    frame.get("method").and_then(Value::as_str) == Some("ping") && frame.get("id").is_some()
}

pub(super) fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
