use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};

use crate::heuristics::HeuristicsRegistry;
use crate::service_launcher::ServiceClient;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "sky-cua";
const SERVER_VERSION: &str = "0.1.0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MessageFraming {
    ContentLength,
    JsonLine,
}

#[derive(Debug, Clone, Default)]
struct ServerSession {
    config: Option<Arc<McpSessionConfig>>,
}

#[derive(Debug, Clone)]
struct McpSessionConfig {
    _process: crate::mcp_tools::McpProcessConfig,
    model: ModelSessionInfo,
    registry: crate::mcp_tools::McpToolRegistry,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModelSessionInfo {
    pub(crate) supports_images: Option<bool>,
}

impl ModelSessionInfo {
    pub(crate) fn can_receive_images(&self) -> bool {
        self.supports_images.unwrap_or(true)
    }
}

pub async fn serve(service: ServiceClient, heuristics: HeuristicsRegistry) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let writer = tokio::sync::Mutex::new(tokio::io::BufWriter::new(stdout));
    let mut session = ServerSession::default();
    let mut read_line_buf = String::with_capacity(256);
    let mut read_payload_buf = Vec::with_capacity(4096);

    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<(Value, MessageFraming)>(32);

    // Spawn a dedicated task to serialize responses to stdout so that
    // concurrent in-flight tool calls cannot interleave writes.
    let writer_task = tokio::spawn(async move {
        let mut payload_buf = Vec::with_capacity(4096);
        while let Some((response, framing)) = response_rx.recv().await {
            let mut w = writer.lock().await;
            payload_buf.clear();
            if let Err(error) = write_message(&mut *w, &response, framing, &mut payload_buf).await {
                tracing::warn!(
                    message = %error,
                    "failed to write MCP response; aborting writer task"
                );
                break;
            }
        }
    });

    loop {
        let (message, framing) =
            match read_message(&mut reader, &mut read_line_buf, &mut read_payload_buf).await? {
                Some(v) => v,
                None => break,
            };

        let method = message.get("method").and_then(Value::as_str).unwrap_or("");

        if method == "tools/call" {
            // Service calls can block for tens of seconds (portal approval).
            // Run them on the blocking thread pool so the read loop stays
            // responsive to pings, cancellations, and new requests.
            let service = service.clone();
            let heuristics = heuristics.clone();
            let response_tx = response_tx.clone();
            let mut session = session.clone();

            tokio::spawn(async move {
                let id = message.get("id").cloned().unwrap_or(Value::Null);
                let response = tokio::task::spawn_blocking(move || {
                    match handle_message(&service, &heuristics, &mut session, message) {
                        Ok(Some(response)) => Some(response),
                        Ok(None) => None,
                        Err(error) => Some(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32603,
                                "message": error.to_string(),
                            }
                        })),
                    }
                })
                .await
                .unwrap_or_else(|_| {
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": {
                            "code": -32603,
                            "message": "service call panicked",
                        }
                    }))
                });

                if let Some(response) = response {
                    let _ = response_tx.send((response, framing)).await;
                }
            });
        } else {
            // Fast path: initialize, tools/list, notifications, etc.
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            let response = match handle_message(&service, &heuristics, &mut session, message) {
                Ok(Some(response)) => Some(response),
                Ok(None) => None,
                Err(error) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32603,
                        "message": error.to_string(),
                    }
                })),
            };
            if let Some(response) = response
                && response_tx.send((response, framing)).await.is_err()
            {
                break;
            }
        }
    }

    drop(response_tx);
    writer_task.await?;
    Ok(())
}

fn handle_message(
    service: &ServiceClient,
    heuristics: &HeuristicsRegistry,
    session: &mut ServerSession,
    body: Value,
) -> Result<Option<Value>> {
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("incoming MCP message did not include a method"))?;
    let id = body.get("id").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            if session.config.is_some() {
                return Ok(Some(already_initialized(id)));
            }
            let process = crate::mcp_tools::mcp_process_config_from_env();
            let model = parse_model_session_info(&body, process.model_supports_images_override);
            let registry = crate::mcp_tools::build_tool_registry(&process, &model);
            session.config = Some(Arc::new(McpSessionConfig {
                _process: process,
                model,
                registry,
            }));
            let protocol_version = body
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": protocol_version,
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    }
                }
            })))
        }
        "notifications/initialized" | "initialized" => Ok(None),
        "tools/list" => {
            let Some(config) = &session.config else {
                return Ok(Some(not_initialized(id)));
            };
            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": config.registry.tools_list_result()
            })))
        }
        "tools/call" => {
            let Some(config) = &session.config else {
                return Ok(Some(not_initialized(id)));
            };
            let tool_name = body
                .pointer("/params/name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("tools/call did not include params.name"))?;
            let arguments = body
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = crate::mcp_tools::handle_session_tool_call(
                service,
                heuristics,
                &config.model,
                &config.registry,
                tool_name,
                arguments,
            )?;
            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            })))
        }
        other if other.starts_with("notifications/") => Ok(None),
        other => Ok(Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("method not found: {other}")
            }
        }))),
    }
}

fn already_initialized(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32600,
            "message": "MCP session is already initialized",
            "data": {
                "code": "AlreadyInitialized"
            }
        }
    })
}

fn not_initialized(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32002,
            "message": "Not initialized",
            "data": {
                "code": "NotInitialized"
            }
        }
    })
}

fn parse_model_session_info(body: &Value, env_override: Option<bool>) -> ModelSessionInfo {
    let supports_images = env_override.or_else(|| model_supports_images_from_initialize(body));
    ModelSessionInfo { supports_images }
}

fn model_name_from_initialize(body: &Value) -> Option<String> {
    [
        "/params/model",
        "/params/model/name",
        "/params/model/id",
        "/params/modelId",
        "/params/model_id",
    ]
    .into_iter()
    .find_map(|pointer| {
        body.pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn model_supports_images_from_initialize(body: &Value) -> Option<bool> {
    [
        "/params/modelCapabilities/supportsImages",
        "/params/modelCapabilities/images",
        "/params/modelCapabilities/imageInput",
        "/params/modelCapabilities/vision",
        "/params/model_capabilities/supports_images",
        "/params/model_capabilities/images",
        "/params/model_capabilities/image_input",
        "/params/model_capabilities/vision",
        "/params/model/capabilities/supportsImages",
        "/params/model/capabilities/images",
        "/params/model/capabilities/imageInput",
        "/params/model/capabilities/vision",
        "/params/model/supportsImages",
        "/params/model/supports_images",
    ]
    .into_iter()
    .find_map(|pointer| body.pointer(pointer).and_then(parse_bool_like))
    .or_else(|| model_name_from_initialize(body).and_then(infer_image_support_from_model_name))
}

fn parse_bool_like(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "supported" | "enabled" => Some(true),
            "0" | "false" | "no" | "off" | "unsupported" | "disabled" => Some(false),
            _ => None,
        },
        Value::Array(values) => {
            if values.is_empty() {
                Some(false)
            } else {
                values.iter().find_map(parse_bool_like).or(Some(true))
            }
        }
        Value::Object(map) => [
            "input",
            "input_image",
            "image",
            "images",
            "vision",
            "supported",
        ]
        .into_iter()
        .find_map(|key| map.get(key).and_then(parse_bool_like)),
        _ => None,
    }
}

fn infer_image_support_from_model_name(name: String) -> Option<bool> {
    let normalized = name.to_ascii_lowercase();
    if normalized.contains("codex-spark") {
        return Some(false);
    }
    if normalized.contains("gpt-5.4") || normalized.contains("gpt-5.5") {
        return Some(true);
    }
    None
}

async fn read_message<R>(
    reader: &mut R,
    line_buf: &mut String,
    payload_buf: &mut Vec<u8>,
) -> Result<Option<(Value, MessageFraming)>>
where
    R: AsyncBufRead + Unpin,
{
    let first_line = loop {
        line_buf.clear();
        let bytes = reader.read_line(line_buf).await?;
        if bytes == 0 {
            return Ok(None);
        }
        let trimmed = line_buf.trim_end_matches(['\r', '\n']);
        if !trimmed.is_empty() {
            break trimmed.to_string();
        }
    };

    let trimmed = first_line.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Ok(Some((
            serde_json::from_str(trimmed).context("invalid line-delimited JSON-RPC message")?,
            MessageFraming::JsonLine,
        )));
    }

    let mut content_length: Option<usize> = None;
    parse_header_line(&first_line, &mut content_length)?;
    loop {
        line_buf.clear();
        let bytes = reader.read_line(line_buf).await?;
        if bytes == 0 {
            return Err(anyhow!(
                "unexpected EOF while reading MCP headers after: {first_line}"
            ));
        }
        let line = line_buf.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        parse_header_line(line, &mut content_length)?;
    }

    let length = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
    payload_buf.resize(length, 0);
    reader.read_exact(payload_buf).await?;
    Ok(Some((
        serde_json::from_slice(payload_buf)?,
        MessageFraming::ContentLength,
    )))
}

fn parse_header_line(line: &str, content_length: &mut Option<usize>) -> Result<()> {
    if let Some(rest) = line.strip_prefix("Content-Length:") {
        let value = rest
            .trim()
            .parse::<usize>()
            .context("invalid Content-Length header")?;
        *content_length = Some(value);
        return Ok(());
    }

    if line.contains(':') {
        return Ok(());
    }

    Err(anyhow!("unexpected MCP header line: {line}"))
}

async fn write_message<W>(
    writer: &mut W,
    message: &Value,
    framing: MessageFraming,
    payload_buf: &mut Vec<u8>,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    serde_json::to_writer(&mut *payload_buf, message)?;
    match framing {
        MessageFraming::ContentLength => {
            writer
                .write_all(format!("Content-Length: {}\r\n\r\n", payload_buf.len()).as_bytes())
                .await?;
            writer.write_all(payload_buf).await?;
        }
        MessageFraming::JsonLine => {
            writer.write_all(payload_buf).await?;
            writer.write_all(b"\n").await?;
        }
    }
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use crate::mcp_tools::tool_definitions;

    use super::{MessageFraming, parse_model_session_info, read_message, write_message};

    #[test]
    fn initialize_model_capabilities_gate_capture_schema() {
        let session = parse_model_session_info(
            &json!({
                "method": "initialize",
                "params": {
                    "model": "gpt-5.3-codex-spark",
                    "modelCapabilities": {
                        "images": false
                    }
                }
            }),
            None,
        );
        assert_eq!(session.supports_images, Some(false));

        let tools = tool_definitions(&session);
        let get_app_state = tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "get_app_state")
            .expect("get_app_state tool");
        assert!(
            get_app_state["inputSchema"]["properties"]
                .get("capture_screen")
                .is_none()
        );
    }

    #[tokio::test]
    async fn read_message_accepts_content_length_framing() {
        let payload = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18"
            }
        }))
        .unwrap();
        let input = format!("Content-Length: {}\r\n\r\n{}", payload.len(), payload);
        let mut reader = Cursor::new(input.into_bytes());
        let mut line_buf = String::with_capacity(128);
        let mut payload_buf = Vec::with_capacity(256);

        let (message, framing) = read_message(&mut reader, &mut line_buf, &mut payload_buf)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(framing, MessageFraming::ContentLength);
        assert_eq!(message["method"], "initialize");
        assert_eq!(message["id"], 1);
    }

    #[tokio::test]
    async fn read_message_accepts_line_delimited_json() {
        let payload = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18"
            }
        }))
        .unwrap();
        let mut reader = Cursor::new(format!("{payload}\n").into_bytes());
        let mut line_buf = String::with_capacity(128);
        let mut payload_buf = Vec::with_capacity(256);

        let (message, framing) = read_message(&mut reader, &mut line_buf, &mut payload_buf)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(framing, MessageFraming::JsonLine);
        assert_eq!(message["method"], "initialize");
        assert_eq!(message["id"], 1);
    }

    #[tokio::test]
    async fn write_message_mirrors_line_delimited_json() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "ok": true
            }
        });
        let mut output = Vec::new();
        let mut payload_buf = Vec::with_capacity(256);

        write_message(
            &mut output,
            &message,
            MessageFraming::JsonLine,
            &mut payload_buf,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert_eq!(
            rendered,
            format!("{}\n", serde_json::to_string(&message).unwrap())
        );
    }
}
