use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};
use sky_cua_platform::{
    BrowserCallerKind, BrowserCallerProvenance, BrowserLogicalIdentity, BrowserMcpClientInfo,
    BrowserOperationIdentity, BrowserProvenanceSource, BrowserRequestContext,
    BrowserSessionIdentity, PhoneCallerProvenance, PhoneMcpClientInfo, PhoneRequestContext,
    ServiceRequest,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};

use crate::heuristics::HeuristicsRegistry;
use crate::service_launcher::ServiceClient;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "sky-cua";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const MCP_CALLER_PROVENANCE_ENV: &str = "SKY_CUA_MCP_CALLER_PROVENANCE";
const LEGACY_MCP_HOST_ENV: &str = "SKY_CUA_MCP_HOST";
/// Maximum JSON-RPC payload size accepted through either supported MCP framing
/// mode. The same bound also caps line and aggregate header accumulation.
const MAX_MCP_FRAME_BYTES: usize = 64 * 1024 * 1024;

static MCP_CONNECTION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static BROWSER_OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static PHONE_TURN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CURRENT_BROWSER_REQUEST_CONTEXT: RefCell<Option<BrowserRequestContext>> = const { RefCell::new(None) };
    static CURRENT_PHONE_REQUEST_CONTEXT: RefCell<Option<PhoneRequestContext>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MessageFraming {
    ContentLength,
    JsonLine,
}

#[derive(Debug, Clone)]
struct ServerSession {
    mcp_connection_id: String,
    config: Option<Arc<McpSessionConfig>>,
}

impl Default for ServerSession {
    fn default() -> Self {
        Self {
            mcp_connection_id: new_mcp_connection_id(),
            config: None,
        }
    }
}

#[derive(Debug, Clone)]
struct McpSessionConfig {
    _process: crate::mcp_tools::McpProcessConfig,
    model: ModelSessionInfo,
    initialize_declared_image_capability: bool,
    registry: crate::mcp_tools::McpToolRegistry,
    browser_provenance: BrowserCallerProvenance,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum JsonRpcRequestId {
    Number(String),
    String(String),
}

impl JsonRpcRequestId {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        match value? {
            Value::Number(value) => Some(Self::Number(value.to_string())),
            Value::String(value) => Some(Self::String(value.clone())),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct PreparedBrowserCall {
    request_id: Option<JsonRpcRequestId>,
    operation_id: String,
    identity: Option<BrowserSessionIdentity>,
    context: BrowserRequestContext,
}

#[derive(Clone, Default)]
struct InFlightBrowserCalls(Arc<Mutex<HashMap<JsonRpcRequestId, String>>>);

impl InFlightBrowserCalls {
    fn register(&self, request_id: JsonRpcRequestId, operation_id: String) {
        self.0
            .lock()
            .expect("in-flight browser call registry poisoned")
            .insert(request_id, operation_id);
    }

    fn operation_for(&self, request_id: &JsonRpcRequestId) -> Option<String> {
        self.0
            .lock()
            .expect("in-flight browser call registry poisoned")
            .get(request_id)
            .cloned()
    }

    fn complete(&self, request_id: &JsonRpcRequestId, operation_id: &str) {
        let mut calls = self
            .0
            .lock()
            .expect("in-flight browser call registry poisoned");
        if calls
            .get(request_id)
            .is_some_and(|active| active == operation_id)
        {
            calls.remove(request_id);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModelSessionInfo {
    pub(crate) supports_images: Option<bool>,
}

impl ModelSessionInfo {
    pub(crate) fn can_receive_images(&self) -> bool {
        self.supports_images == Some(true)
    }
}

pub async fn serve(service: ServiceClient, heuristics: HeuristicsRegistry) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let writer = tokio::sync::Mutex::new(tokio::io::BufWriter::new(stdout));
    let mut session = ServerSession::default();
    let mut read_line_buf = Vec::with_capacity(256);
    let mut read_payload_buf = Vec::with_capacity(4096);
    let in_flight_browser_calls = InFlightBrowserCalls::default();
    let mut browser_tasks = tokio::task::JoinSet::new();

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
        while let Some(result) = browser_tasks.try_join_next() {
            if let Err(error) = result {
                tracing::warn!(%error, "browser MCP task failed");
            }
        }

        if method == "tools/call" {
            // Service calls can block for tens of seconds (portal approval).
            // Run them on the blocking thread pool so the read loop stays
            // responsive to pings, cancellations, and new requests.
            let service = service.clone();
            let heuristics = heuristics.clone();
            let response_tx = response_tx.clone();
            let mut session = session.clone();
            let prepared_browser_call = prepare_browser_call(&message, &session);
            if let Some(prepared) = &prepared_browser_call
                && let Some(request_id) = &prepared.request_id
            {
                in_flight_browser_calls.register(request_id.clone(), prepared.operation_id.clone());
            }
            let in_flight_browser_calls = in_flight_browser_calls.clone();

            let is_browser_call = prepared_browser_call.is_some();
            let task = async move {
                let id = message.get("id").cloned().unwrap_or(Value::Null);
                let completion = prepared_browser_call.as_ref().and_then(|prepared| {
                    prepared
                        .request_id
                        .clone()
                        .map(|request_id| (request_id, prepared.operation_id.clone()))
                });
                let response = tokio::task::spawn_blocking(move || {
                    match handle_message(
                        &service,
                        &heuristics,
                        &mut session,
                        message,
                        prepared_browser_call,
                    ) {
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

                if let Some((request_id, operation_id)) = completion {
                    in_flight_browser_calls.complete(&request_id, &operation_id);
                }

                if let Some(response) = response {
                    let _ = response_tx.send((response, framing)).await;
                }
            };
            if is_browser_call {
                browser_tasks.spawn(task);
            } else {
                tokio::spawn(task);
            }
        } else if method == "notifications/cancelled" {
            if let Some(request) =
                cancellation_request(&message, &session, &in_flight_browser_calls)
            {
                issue_service_request_without_blocking(service.clone(), request);
            }
        } else {
            // Fast path: initialize, tools/list, notifications, etc.
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            let response = match handle_message(&service, &heuristics, &mut session, message, None)
            {
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

    // A browser call parsed before EOF owns an admitted logical request even
    // if its worker has not yet reached the service. Keep the connection
    // lifecycle open until every such task has either completed or failed, so
    // EOF cannot race ahead and make an already-read call fail as disconnected.
    while let Some(result) = browser_tasks.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "browser MCP task failed while draining EOF");
        }
    }

    let mut disconnect_notified = false;
    if let Some(request) =
        disconnect_request_once(&mut disconnect_notified, &session.mcp_connection_id)
    {
        let service = service.clone();
        match tokio::task::spawn_blocking(move || service.call(&request)).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "failed to notify service of browser client disconnect");
            }
            Err(error) => {
                tracing::warn!(%error, "browser disconnect service task panicked");
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
    prepared_browser_call: Option<PreparedBrowserCall>,
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
            let process =
                crate::mcp_tools::mcp_process_config_from_env().map_err(anyhow::Error::msg)?;
            let model = parse_model_session_info(&body, process.model_supports_images_override);
            let initialize_declared_image_capability =
                model_image_capability_from_initialize(&body).is_some();
            let registry = crate::mcp_tools::build_tool_registry(&process, &model);
            let declared_provenance = std::env::var(MCP_CALLER_PROVENANCE_ENV)
                .ok()
                .or_else(|| std::env::var(LEGACY_MCP_HOST_ENV).ok());
            let browser_provenance = browser_caller_provenance(
                &body,
                &session.mcp_connection_id,
                declared_provenance.as_deref(),
            );
            session.config = Some(Arc::new(McpSessionConfig {
                _process: process,
                model,
                initialize_declared_image_capability,
                registry,
                browser_provenance,
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
            let call_model = model_session_info_for_tool_call(&body, config);
            let phone_context = (config.registry.contains(tool_name)
                && config
                    .registry
                    .validate_arguments(tool_name, &arguments)
                    .is_ok()
                && is_phone_surface_tool_call(tool_name, Some(&arguments)))
            .then(|| phone_call_context(&body, &config.browser_provenance));
            let result = match prepared_browser_call {
                Some(prepared) => with_browser_request_context(prepared.context, || {
                    crate::mcp_tools::handle_session_tool_call_with_browser_identity(
                        service,
                        heuristics,
                        &call_model,
                        &config.registry,
                        tool_name,
                        arguments,
                        prepared.identity.as_ref(),
                    )
                })?,
                None => match phone_context {
                    Some(context) => with_phone_request_context(context, || {
                        crate::mcp_tools::handle_session_tool_call_with_browser_identity(
                            service,
                            heuristics,
                            &call_model,
                            &config.registry,
                            tool_name,
                            arguments,
                            None,
                        )
                    })?,
                    None => crate::mcp_tools::handle_session_tool_call_with_browser_identity(
                        service,
                        heuristics,
                        &call_model,
                        &config.registry,
                        tool_name,
                        arguments,
                        None,
                    )?,
                },
            };
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

fn prepare_browser_call(body: &Value, session: &ServerSession) -> Option<PreparedBrowserCall> {
    let config = session.config.as_ref()?;
    let tool_name = body.pointer("/params/name")?.as_str()?;
    let empty_arguments = json!({});
    let arguments = body
        .pointer("/params/arguments")
        .unwrap_or(&empty_arguments);
    if !config.registry.contains(tool_name)
        || config
            .registry
            .validate_arguments(tool_name, arguments)
            .is_err()
        || !is_browser_surface_tool_call(tool_name, Some(arguments))
    {
        return None;
    }
    let (identity, context) = browser_call_context(body, &config.browser_provenance);
    Some(PreparedBrowserCall {
        request_id: JsonRpcRequestId::from_value(body.get("id")),
        operation_id: context.operation_identity.operation_id.clone(),
        identity,
        context,
    })
}

fn is_browser_surface_tool_call(tool_name: &str, arguments: Option<&Value>) -> bool {
    if tool_name.starts_with("browser_") {
        return true;
    }
    match tool_name {
        "list_resources" | "observe" | "capture_screen" => {
            arguments
                .and_then(|arguments| arguments.get("surface"))
                .and_then(Value::as_str)
                == Some("browser")
        }
        "status" => {
            arguments
                .and_then(|arguments| arguments.get("component"))
                .and_then(Value::as_str)
                == Some("browser")
        }
        _ => false,
    }
}

fn is_phone_surface_tool_call(tool_name: &str, arguments: Option<&Value>) -> bool {
    if tool_name.starts_with("phone_") {
        return true;
    }
    match tool_name {
        "list_resources" | "observe" | "capture_screen" => {
            arguments
                .and_then(|arguments| arguments.get("surface"))
                .and_then(Value::as_str)
                == Some("phone")
        }
        "status" => matches!(
            arguments
                .and_then(|arguments| arguments.get("component"))
                .and_then(Value::as_str),
            Some("phone" | "phone_companion")
        ),
        _ => false,
    }
}

fn cancellation_request(
    notification: &Value,
    session: &ServerSession,
    in_flight: &InFlightBrowserCalls,
) -> Option<ServiceRequest> {
    let request_id = JsonRpcRequestId::from_value(notification.pointer("/params/requestId"))?;
    let operation_id = in_flight.operation_for(&request_id)?;
    let reason = notification
        .pointer("/params/reason")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Some(ServiceRequest::CancelBrowserOperation {
        connection_id: session.mcp_connection_id.clone(),
        operation_id,
        reason,
    })
}

fn issue_service_request_without_blocking(service: ServiceClient, request: ServiceRequest) {
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(move || service.call(&request)).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "failed to cancel browser operation");
            }
            Err(error) => {
                tracing::warn!(%error, "browser cancellation service task panicked");
            }
        }
    });
}

fn disconnect_request_once(notified: &mut bool, connection_id: &str) -> Option<ServiceRequest> {
    if std::mem::replace(notified, true) {
        return None;
    }
    Some(ServiceRequest::BrowserClientDisconnected {
        connection_id: connection_id.to_owned(),
    })
}

struct BrowserRequestContextGuard(Option<BrowserRequestContext>);

impl Drop for BrowserRequestContextGuard {
    fn drop(&mut self) {
        CURRENT_BROWSER_REQUEST_CONTEXT.with(|current| {
            current.replace(self.0.take());
        });
    }
}

pub(crate) fn with_browser_request_context<T>(
    context: BrowserRequestContext,
    f: impl FnOnce() -> T,
) -> T {
    let previous = CURRENT_BROWSER_REQUEST_CONTEXT.with(|current| current.replace(Some(context)));
    let _guard = BrowserRequestContextGuard(previous);
    f()
}

pub(crate) fn current_browser_request_context() -> Option<BrowserRequestContext> {
    CURRENT_BROWSER_REQUEST_CONTEXT.with(|current| current.borrow().clone())
}

struct PhoneRequestContextGuard(Option<PhoneRequestContext>);

impl Drop for PhoneRequestContextGuard {
    fn drop(&mut self) {
        CURRENT_PHONE_REQUEST_CONTEXT.with(|current| {
            current.replace(self.0.take());
        });
    }
}

pub(crate) fn with_phone_request_context<T>(
    context: PhoneRequestContext,
    f: impl FnOnce() -> T,
) -> T {
    let previous = CURRENT_PHONE_REQUEST_CONTEXT.with(|current| current.replace(Some(context)));
    let _guard = PhoneRequestContextGuard(previous);
    f()
}

pub(crate) fn current_phone_request_context() -> Option<PhoneRequestContext> {
    CURRENT_PHONE_REQUEST_CONTEXT.with(|current| current.borrow().clone())
}

fn new_mcp_connection_id() -> String {
    let sequence = MCP_CONNECTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    stable_fingerprint(&format!("{}:{nanos}:{sequence}", std::process::id()))
}

fn browser_caller_provenance(
    initialize: &Value,
    mcp_connection_id: &str,
    declared: Option<&str>,
) -> BrowserCallerProvenance {
    let declared_caller = declared
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(128).collect::<String>());
    let client_info = mcp_client_info(initialize);
    let normalized = declared_caller
        .as_deref()
        .and_then(normalize_declared_caller);
    let inferred = client_info
        .as_ref()
        .and_then(|info| infer_caller_from_client_info(&info.name));
    let (caller, source) = match normalized {
        Some(caller) => (caller, BrowserProvenanceSource::InstallerDeclaration),
        None if declared_caller.is_some() => (
            BrowserCallerKind::LegacyUnknown,
            BrowserProvenanceSource::LegacyFallback,
        ),
        None => match inferred {
            Some(caller) => (caller, BrowserProvenanceSource::ClientInfoInference),
            None => (
                BrowserCallerKind::LegacyUnknown,
                BrowserProvenanceSource::LegacyFallback,
            ),
        },
    };
    BrowserCallerProvenance {
        caller,
        source,
        connection_id: mcp_connection_id.to_owned(),
        declared_caller,
        client_info,
    }
}

fn normalize_declared_caller(value: &str) -> Option<BrowserCallerKind> {
    BrowserCallerKind::from_provenance_label(value)
}

fn infer_caller_from_client_info(value: &str) -> Option<BrowserCallerKind> {
    normalize_declared_caller(value)
}

fn mcp_client_info(initialize: &Value) -> Option<BrowserMcpClientInfo> {
    let info = initialize.pointer("/params/clientInfo")?.as_object()?;
    let non_empty = |key: &str| {
        info.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let name = non_empty("name")?;
    let version = non_empty("version")?;
    Some(BrowserMcpClientInfo {
        name: name.to_owned(),
        version: version.to_owned(),
        title: non_empty("title").map(ToOwned::to_owned),
    })
}

fn phone_call_context(body: &Value, provenance: &BrowserCallerProvenance) -> PhoneRequestContext {
    let supplied = phone_session_and_turn_from_tool_call(body);
    let identity_synthetic = supplied.is_none();
    let (session_id, turn_id) = supplied.unwrap_or_else(|| {
        let sequence = PHONE_TURN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        (
            provenance.connection_id.clone(),
            format!("phone-turn-{}-{sequence:016x}", provenance.connection_id),
        )
    });
    PhoneRequestContext {
        session_id: Some(session_id),
        turn_id: Some(turn_id),
        caller_provenance: Some(match provenance.caller {
            BrowserCallerKind::CodexDesktop => PhoneCallerProvenance::CodexDesktop,
            BrowserCallerKind::OpenClaw => PhoneCallerProvenance::OpenClaw,
            BrowserCallerKind::OpenCode => PhoneCallerProvenance::OpenCode,
            _ => PhoneCallerProvenance::DirectMcp,
        }),
        identity_synthetic: Some(identity_synthetic),
        client_info: provenance
            .client_info
            .as_ref()
            .map(|info| PhoneMcpClientInfo {
                name: info.name.clone(),
                version: info.version.clone(),
                title: info.title.clone(),
            }),
    }
}

fn phone_session_and_turn_from_tool_call(body: &Value) -> Option<(String, String)> {
    let metadata = body.pointer("/params/_meta/x-codex-turn-metadata")?;
    let parsed;
    let metadata = match metadata {
        Value::Object(map) => map,
        Value::String(raw) => {
            parsed = serde_json::from_str::<Map<String, Value>>(raw).ok()?;
            &parsed
        }
        _ => return None,
    };
    let exact_non_empty = |key: &str| {
        metadata
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
    };
    Some((exact_non_empty("session_id")?, exact_non_empty("turn_id")?))
}

pub(crate) fn browser_call_context(
    body: &Value,
    provenance: &BrowserCallerProvenance,
) -> (Option<BrowserSessionIdentity>, BrowserRequestContext) {
    let legacy_identity = browser_session_identity_from_tool_call(body);
    let connection_id = &provenance.connection_id;
    let logical_identity = match &legacy_identity {
        Some(identity) => BrowserLogicalIdentity {
            session_id: identity.session_id.clone(),
            thread_id: identity.thread_id.clone(),
            turn_id: Some(identity.turn_id.clone()),
        },
        None => BrowserLogicalIdentity {
            session_id: connection_id.clone(),
            thread_id: None,
            turn_id: None,
        },
    };
    let request_id_fingerprint = json_rpc_id_fingerprint(body.get("id"));
    let operation_id = new_browser_operation_id(connection_id);
    (
        legacy_identity,
        BrowserRequestContext {
            provenance: provenance.clone(),
            logical_identity,
            operation_identity: BrowserOperationIdentity {
                operation_id,
                request_id_fingerprint,
            },
        },
    )
}

fn new_browser_operation_id(connection_id: &str) -> String {
    let sequence = BROWSER_OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("op-{connection_id}-{sequence:016x}")
}

fn json_rpc_id_fingerprint(id: Option<&Value>) -> String {
    let encoded = match id {
        Some(Value::String(value)) => format!("string:{value}"),
        Some(Value::Number(value)) => format!("number:{value}"),
        Some(Value::Null) => "null:".to_string(),
        Some(other) => format!("invalid:{other}"),
        None => "missing:".to_string(),
    };
    stable_fingerprint(&encoded)
}

fn stable_fingerprint(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn browser_session_identity_from_tool_call(body: &Value) -> Option<BrowserSessionIdentity> {
    let metadata = body.pointer("/params/_meta/x-codex-turn-metadata")?;
    let parsed;
    let metadata = match metadata {
        Value::Object(map) => map,
        Value::String(raw) => {
            parsed = serde_json::from_str::<Map<String, Value>>(raw).ok()?;
            &parsed
        }
        _ => return None,
    };
    let non_empty = |key: &str| {
        metadata
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };
    Some(BrowserSessionIdentity {
        session_id: non_empty("session_id")?,
        turn_id: non_empty("turn_id")?,
        thread_id: non_empty("thread_id"),
    })
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
    let supports_images = model_image_capability_from_initialize(body)
        .or(env_override)
        .or_else(|| model_name_from_initialize(body).and_then(infer_image_support_from_model_name));
    ModelSessionInfo { supports_images }
}

fn model_session_info_for_tool_call(body: &Value, config: &McpSessionConfig) -> ModelSessionInfo {
    ModelSessionInfo {
        supports_images: model_image_capability_for_tool_call(
            body,
            config.model.supports_images,
            config.initialize_declared_image_capability,
            config._process.model_supports_images_override,
        ),
    }
}

fn model_image_capability_for_tool_call(
    body: &Value,
    initialized_capability: Option<bool>,
    initialize_declared_image_capability: bool,
    env_override: Option<bool>,
) -> Option<bool> {
    let turn_metadata = codex_turn_metadata(body);
    let turn_capability = turn_metadata
        .as_ref()
        .and_then(model_image_capability_from_turn_metadata);
    let turn_model = turn_metadata
        .as_ref()
        .and_then(model_name_from_turn_metadata);
    let initialized_explicit = initialize_declared_image_capability
        .then_some(initialized_capability)
        .flatten();
    turn_capability
        .or(initialized_explicit)
        .or(env_override)
        .or_else(|| turn_model.and_then(infer_image_support_from_model_name))
        .or(initialized_capability)
}

fn codex_turn_metadata(body: &Value) -> Option<Value> {
    match body.pointer("/params/_meta/x-codex-turn-metadata")? {
        Value::Object(map) => Some(Value::Object(map.clone())),
        Value::String(raw) => serde_json::from_str(raw).ok(),
        _ => None,
    }
}

fn model_name_from_turn_metadata(metadata: &Value) -> Option<String> {
    [
        "/model",
        "/model/name",
        "/model/id",
        "/model_id",
        "/modelId",
    ]
    .into_iter()
    .find_map(|pointer| {
        metadata
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn model_image_capability_from_turn_metadata(metadata: &Value) -> Option<bool> {
    [
        "/modelCapabilities/supportsImages",
        "/modelCapabilities/images",
        "/modelCapabilities/imageInput",
        "/modelCapabilities/vision",
        "/model_capabilities/supports_images",
        "/model_capabilities/images",
        "/model_capabilities/image_input",
        "/model_capabilities/vision",
        "/model/capabilities/supportsImages",
        "/model/capabilities/images",
        "/model/capabilities/imageInput",
        "/model/capabilities/vision",
        "/model/supportsImages",
        "/model/supports_images",
    ]
    .into_iter()
    .find_map(|pointer| metadata.pointer(pointer).and_then(parse_bool_like))
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

fn model_image_capability_from_initialize(body: &Value) -> Option<bool> {
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
    if normalized.contains("gpt-5.6") {
        return Some(true);
    }
    None
}

async fn read_message<R>(
    reader: &mut R,
    line_buf: &mut Vec<u8>,
    payload_buf: &mut Vec<u8>,
) -> Result<Option<(Value, MessageFraming)>>
where
    R: AsyncBufRead + Unpin,
{
    read_message_with_limit(reader, line_buf, payload_buf, MAX_MCP_FRAME_BYTES).await
}

async fn read_message_with_limit<R>(
    reader: &mut R,
    line_buf: &mut Vec<u8>,
    payload_buf: &mut Vec<u8>,
    max_frame_bytes: usize,
) -> Result<Option<(Value, MessageFraming)>>
where
    R: AsyncBufRead + Unpin,
{
    let first_line = loop {
        if !read_bounded_line(reader, line_buf, max_frame_bytes).await? {
            return Ok(None);
        }
        let trimmed = std::str::from_utf8(line_buf).context("MCP frame line is not UTF-8")?;
        if !trimmed.is_empty() {
            break trimmed.to_owned();
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
    let mut header_bytes = first_line.len();
    loop {
        let remaining_header_bytes = max_frame_bytes.saturating_sub(header_bytes);
        if !read_bounded_line(reader, line_buf, remaining_header_bytes).await? {
            return Err(anyhow!(
                "unexpected EOF while reading MCP headers after: {first_line}"
            ));
        }
        let line = std::str::from_utf8(line_buf).context("MCP header line is not UTF-8")?;
        if line.is_empty() {
            break;
        }
        header_bytes = header_bytes
            .checked_add(line.len())
            .filter(|bytes| *bytes <= max_frame_bytes)
            .ok_or_else(|| frame_too_large("MCP headers", max_frame_bytes))?;
        parse_header_line(line, &mut content_length)?;
    }

    let length = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
    if length > max_frame_bytes {
        return Err(frame_too_large(
            "MCP Content-Length payload",
            max_frame_bytes,
        ));
    }
    payload_buf.resize(length, 0);
    reader.read_exact(payload_buf).await?;
    Ok(Some((
        serde_json::from_slice(payload_buf)?,
        MessageFraming::ContentLength,
    )))
}

async fn read_bounded_line<R>(
    reader: &mut R,
    line_buf: &mut Vec<u8>,
    max_content_bytes: usize,
) -> Result<bool>
where
    R: AsyncBufRead + Unpin,
{
    line_buf.clear();
    let mut pending_cr = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if pending_cr {
                extend_bounded_line(line_buf, b"\r", max_content_bytes)?;
            }
            return Ok(!line_buf.is_empty());
        }

        if pending_cr {
            if available.first() == Some(&b'\n') {
                reader.consume(1);
                return Ok(true);
            }
            extend_bounded_line(line_buf, b"\r", max_content_bytes)?;
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let content = newline.map_or(available, |index| &available[..index]);
        let (content, trailing_cr) = if content.last() == Some(&b'\r') {
            (&content[..content.len() - 1], true)
        } else {
            (content, false)
        };
        extend_bounded_line(line_buf, content, max_content_bytes)?;
        reader.consume(consumed);

        if newline.is_some() {
            return Ok(true);
        }
        pending_cr = trailing_cr;
    }
}

fn extend_bounded_line(
    line_buf: &mut Vec<u8>,
    content: &[u8],
    max_content_bytes: usize,
) -> Result<()> {
    line_buf
        .len()
        .checked_add(content.len())
        .filter(|length| *length <= max_content_bytes)
        .ok_or_else(|| frame_too_large("MCP line", max_content_bytes))?;
    line_buf.extend_from_slice(content);
    Ok(())
}

fn frame_too_large(kind: &str, max_frame_bytes: usize) -> anyhow::Error {
    anyhow!("{kind} exceeds maximum frame size of {max_frame_bytes} bytes")
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
    use std::{io::Cursor, sync::Arc};

    use serde_json::{Value, json};
    use sky_cua_platform::{
        BrowserCallerKind, BrowserProvenanceSource, PhoneCallerProvenance, ServiceRequest,
        config::AgentSurfacePolicy,
    };

    use crate::mcp_tools::{McpProcessConfig, build_tool_registry, tool_definitions};

    use super::{
        InFlightBrowserCalls, JsonRpcRequestId, McpSessionConfig, MessageFraming, ModelSessionInfo,
        ServerSession, browser_call_context, browser_caller_provenance,
        browser_session_identity_from_tool_call, cancellation_request,
        current_browser_request_context, disconnect_request_once, is_browser_surface_tool_call,
        is_phone_surface_tool_call, json_rpc_id_fingerprint, model_image_capability_for_tool_call,
        normalize_declared_caller, parse_model_session_info, phone_call_context,
        phone_session_and_turn_from_tool_call, prepare_browser_call, read_message,
        read_message_with_limit, with_browser_request_context, write_message,
    };

    #[test]
    fn phone_context_preserves_codex_identity_and_client_info() {
        let initialize = json!({
            "params": {
                "clientInfo": {
                    "name": "codex-desktop",
                    "version": "42.7",
                    "title": "Codex Desktop"
                }
            }
        });
        let provenance =
            browser_caller_provenance(&initialize, "connection-codex", Some("codex_desktop"));
        let context = phone_call_context(
            &json!({
                "id": 17,
                "params": {
                    "_meta": {
                        "x-codex-turn-metadata": {
                            "session_id": "codex-session",
                            "turn_id": "codex-turn",
                            "thread_id": "codex-thread"
                        }
                    }
                }
            }),
            &provenance,
        );

        assert_eq!(context.session_id.as_deref(), Some("codex-session"));
        assert_eq!(context.turn_id.as_deref(), Some("codex-turn"));
        assert_eq!(
            context.caller_provenance,
            Some(PhoneCallerProvenance::CodexDesktop)
        );
        assert_eq!(context.identity_synthetic, Some(false));
        let client_info = context.client_info.expect("client info");
        assert_eq!(client_info.name, "codex-desktop");
        assert_eq!(client_info.version, "42.7");
        assert_eq!(client_info.title.as_deref(), Some("Codex Desktop"));
    }

    #[test]
    fn phone_identity_preserves_supplied_codex_values_exactly() {
        let supplied = phone_session_and_turn_from_tool_call(&json!({
            "params": {
                "_meta": {
                    "x-codex-turn-metadata": serde_json::to_string(&json!({
                        "session_id": " session-as-supplied ",
                        "turn_id": " turn-as-supplied "
                    })).unwrap()
                }
            }
        }))
        .expect("phone identity");
        assert_eq!(supplied.0, " session-as-supplied ");
        assert_eq!(supplied.1, " turn-as-supplied ");
    }

    #[test]
    fn phone_context_synthesizes_stable_session_and_unique_turns_for_generic_callers() {
        let initialize = json!({
            "params": {
                "clientInfo": { "name": "openclaw", "version": "1.2.3" }
            }
        });
        let openclaw =
            browser_caller_provenance(&initialize, "connection-openclaw", Some("openclaw"));
        let first = phone_call_context(&json!({"id": "first"}), &openclaw);
        let second = phone_call_context(&json!({"id": "second"}), &openclaw);
        assert_eq!(first.session_id.as_deref(), Some("connection-openclaw"));
        assert_eq!(second.session_id, first.session_id);
        assert_ne!(second.turn_id, first.turn_id);
        assert_eq!(
            first.caller_provenance,
            Some(PhoneCallerProvenance::OpenClaw)
        );
        assert_eq!(first.identity_synthetic, Some(true));

        let direct = browser_caller_provenance(
            &json!({"params": {"clientInfo": {"name": "other", "version": "9"}}}),
            "connection-direct",
            None,
        );
        assert_eq!(
            phone_call_context(&json!({"id": 1}), &direct).caller_provenance,
            Some(PhoneCallerProvenance::DirectMcp)
        );
    }

    #[test]
    fn phone_surface_aliases_receive_context_without_capturing_other_surfaces() {
        assert!(is_phone_surface_tool_call(
            "status",
            Some(&json!({"component": "phone"}))
        ));
        assert!(is_phone_surface_tool_call(
            "status",
            Some(&json!({"component": "phone_companion"}))
        ));
        for tool in ["observe", "capture_screen", "list_resources"] {
            assert!(is_phone_surface_tool_call(
                tool,
                Some(&json!({"surface": "phone"}))
            ));
            assert!(!is_phone_surface_tool_call(
                tool,
                Some(&json!({"surface": "desktop"}))
            ));
        }
        assert!(is_phone_surface_tool_call(
            "phone_pointer",
            Some(&json!({}))
        ));
        assert!(!is_phone_surface_tool_call(
            "desktop_pointer",
            Some(&json!({}))
        ));
    }

    #[test]
    fn extracts_codex_browser_identity_from_string_metadata() {
        let identity = browser_session_identity_from_tool_call(&json!({
            "params": {
                "_meta": {
                    "x-codex-turn-metadata": serde_json::to_string(&json!({
                        "session_id": "session-uuid",
                        "thread_id": "thread-uuid",
                        "turn_id": "turn-uuid"
                    })).unwrap()
                }
            }
        }))
        .expect("identity");

        assert_eq!(identity.session_id, "session-uuid");
        assert_eq!(identity.turn_id, "turn-uuid");
        assert_eq!(identity.thread_id.as_deref(), Some("thread-uuid"));
    }

    #[test]
    fn extracts_codex_browser_identity_from_object_metadata() {
        let identity = browser_session_identity_from_tool_call(&json!({
            "params": {
                "_meta": {
                    "x-codex-turn-metadata": {
                        "session_id": "session-uuid",
                        "turn_id": "turn-uuid"
                    }
                }
            }
        }))
        .expect("identity");

        assert_eq!(identity.session_id, "session-uuid");
        assert_eq!(identity.turn_id, "turn-uuid");
        assert_eq!(identity.thread_id, None);
    }

    #[test]
    fn incomplete_codex_metadata_uses_non_codex_fallback() {
        assert!(
            browser_session_identity_from_tool_call(&json!({
                "params": {
                    "_meta": {
                        "x-codex-turn-metadata": { "session_id": "session-only" }
                    }
                }
            }))
            .is_none()
        );
    }

    #[test]
    fn installer_provenance_is_normalized_and_wrong_values_use_legacy_fallback() {
        let initialize = json!({
            "params": {
                "clientInfo": {
                    "name": " pi-mcp-adapter ",
                    "version": " 2.4.0 ",
                    "title": " Pi Adapter "
                }
            }
        });
        let declared =
            browser_caller_provenance(&initialize, "connection-stable", Some(" Pi MCP Adapter "));
        assert_eq!(declared.caller, BrowserCallerKind::Pi);
        assert_eq!(
            declared.source,
            BrowserProvenanceSource::InstallerDeclaration
        );
        assert_eq!(declared.connection_id, "connection-stable");
        let client_info = declared.client_info.as_ref().expect("client info");
        assert_eq!(client_info.name, "pi-mcp-adapter");
        assert_eq!(client_info.version, "2.4.0");
        assert_eq!(client_info.title.as_deref(), Some("Pi Adapter"));

        let wrong = browser_caller_provenance(&initialize, "connection-stable", Some("../../bad"));
        assert_eq!(wrong.caller, BrowserCallerKind::LegacyUnknown);
        assert_eq!(wrong.source, BrowserProvenanceSource::LegacyFallback);
        assert_eq!(wrong.declared_caller.as_deref(), Some("../../bad"));
        for (declared, expected) in [
            ("codex_desktop", BrowserCallerKind::CodexDesktop),
            ("codex_cli", BrowserCallerKind::CodexCli),
            ("OpenClaw", BrowserCallerKind::OpenClaw),
            ("OpenCode", BrowserCallerKind::OpenCode),
            ("pi-mcp-adapter", BrowserCallerKind::Pi),
            ("direct_mcp", BrowserCallerKind::DirectMcp),
        ] {
            assert_eq!(normalize_declared_caller(declared), Some(expected));
        }
        assert_eq!(normalize_declared_caller("not-a-real-host"), None);

        let inferred = browser_caller_provenance(
            &json!({
                "params": {
                    "clientInfo": { "name": "OpenCode", "version": "1.0" }
                }
            }),
            "connection-stable",
            None,
        );
        assert_eq!(inferred.caller, BrowserCallerKind::OpenCode);
        assert_eq!(
            inferred.source,
            BrowserProvenanceSource::ClientInfoInference
        );
    }

    #[test]
    fn mcp_connection_identity_is_stable_across_clones_and_unique_per_connection() {
        let first = ServerSession::default();
        let clone = first.clone();
        let second = ServerSession::default();

        assert_eq!(first.mcp_connection_id, clone.mcp_connection_id);
        assert_ne!(first.mcp_connection_id, second.mcp_connection_id);
        assert_eq!(first.mcp_connection_id.len(), 16);
    }

    #[test]
    fn malformed_turn_metadata_keeps_connection_and_operation_context() {
        let provenance =
            browser_caller_provenance(&json!({ "params": {} }), "connection-stable", None);
        let (legacy_identity, context) = browser_call_context(
            &json!({
                "id": "call-9",
                "params": {
                    "_meta": {
                        "x-codex-turn-metadata": "{ definitely not json"
                    }
                }
            }),
            &provenance,
        );

        assert_eq!(legacy_identity, None);
        assert_eq!(context.logical_identity.session_id, "connection-stable");
        assert_eq!(context.logical_identity.thread_id, None);
        assert_eq!(context.logical_identity.turn_id, None);
        assert!(context.operation_identity.operation_id.starts_with("op-"));
        assert_eq!(
            context.operation_identity.request_id_fingerprint,
            json_rpc_id_fingerprint(Some(&json!("call-9")))
        );
    }

    #[test]
    fn codex_logical_metadata_is_separate_from_provenance_and_legacy_identity() {
        let provenance = browser_caller_provenance(
            &json!({
                "params": {
                    "clientInfo": { "name": "codex", "version": "1.0" }
                }
            }),
            "connection-stable",
            Some("codex"),
        );
        let (legacy_identity, context) = browser_call_context(
            &json!({
                "id": 42,
                "params": {
                    "_meta": {
                        "x-codex-turn-metadata": {
                            "session_id": "session-uuid",
                            "thread_id": "thread-uuid",
                            "turn_id": "turn-uuid"
                        }
                    }
                }
            }),
            &provenance,
        );

        let legacy_identity = legacy_identity.expect("legacy Codex identity");
        assert_eq!(legacy_identity.session_id, "session-uuid");
        assert_eq!(legacy_identity.turn_id, "turn-uuid");
        assert_eq!(legacy_identity.thread_id.as_deref(), Some("thread-uuid"));
        assert_eq!(context.provenance.caller, BrowserCallerKind::CodexCli);
        assert_eq!(context.provenance.connection_id, "connection-stable");
        assert_eq!(context.logical_identity.session_id, "session-uuid");
        assert_eq!(
            context.logical_identity.thread_id.as_deref(),
            Some("thread-uuid")
        );
        assert_eq!(
            context.logical_identity.turn_id.as_deref(),
            Some("turn-uuid")
        );
    }

    #[test]
    fn operation_ids_are_fresh_while_fingerprints_correlate_upstream_ids() {
        let numeric = json_rpc_id_fingerprint(Some(&json!(7)));
        let numeric_again = json_rpc_id_fingerprint(Some(&json!(7)));
        let string = json_rpc_id_fingerprint(Some(&json!("7")));
        let missing = json_rpc_id_fingerprint(None);
        let null = json_rpc_id_fingerprint(Some(&Value::Null));

        assert_eq!(numeric, numeric_again);
        assert_ne!(numeric, string);
        assert_ne!(missing, null);

        let provenance_a = browser_caller_provenance(&json!({}), "connection-a", None);
        let provenance_b = browser_caller_provenance(&json!({}), "connection-b", None);
        let call = json!({ "id": 7, "params": {} });
        let (_, context_a) = browser_call_context(&call, &provenance_a);
        let (_, context_a_reused_id) = browser_call_context(&call, &provenance_a);
        let (_, context_b) = browser_call_context(&call, &provenance_b);
        assert_eq!(
            context_a.operation_identity.request_id_fingerprint,
            context_a_reused_id
                .operation_identity
                .request_id_fingerprint
        );
        assert_ne!(
            context_a.operation_identity.operation_id,
            context_a_reused_id.operation_identity.operation_id
        );
        assert_ne!(
            context_a_reused_id.operation_identity.operation_id,
            context_b.operation_identity.operation_id
        );

        let (_, secret_id_context) =
            browser_call_context(&json!({ "id": "caller-authored-secret" }), &provenance_a);
        assert!(
            !secret_id_context
                .operation_identity
                .operation_id
                .contains("caller-authored-secret")
        );
    }

    #[test]
    fn browser_surface_aliases_receive_context_without_capturing_other_surfaces() {
        for tool in ["list_resources", "observe", "capture_screen"] {
            assert!(is_browser_surface_tool_call(
                tool,
                Some(&json!({"surface":"browser"}))
            ));
            assert!(!is_browser_surface_tool_call(
                tool,
                Some(&json!({"surface":"desktop"}))
            ));
        }
        assert!(is_browser_surface_tool_call(
            "status",
            Some(&json!({"component":"browser"}))
        ));
        assert!(!is_browser_surface_tool_call(
            "status",
            Some(&json!({"component":"phone"}))
        ));
        assert!(is_browser_surface_tool_call(
            "browser_input",
            Some(&json!({}))
        ));
    }

    #[test]
    fn browser_open_without_arguments_still_prepares_browser_context() {
        let process = McpProcessConfig {
            browser_eval_enabled: true,
            surfaces: AgentSurfacePolicy::default(),
            model_supports_images_override: None,
            diagnostics: Vec::new(),
        };
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process, &model);
        let provenance = browser_caller_provenance(
            &json!({"params": {"clientInfo": {"name": "test", "version": "1"}}}),
            "connection-browser-open",
            None,
        );
        let session = ServerSession {
            mcp_connection_id: "connection-browser-open".to_string(),
            config: Some(Arc::new(McpSessionConfig {
                _process: process,
                model,
                initialize_declared_image_capability: false,
                registry,
                browser_provenance: provenance,
            })),
        };

        let prepared = prepare_browser_call(
            &json!({
                "id": 1,
                "method": "tools/call",
                "params": {"name": "browser_open"}
            }),
            &session,
        );
        assert!(prepared.is_some());
    }

    #[test]
    fn queued_browser_call_maps_cancellation_to_its_internal_operation() {
        let session = ServerSession {
            mcp_connection_id: "connection-stable".to_string(),
            config: None,
        };
        let calls = InFlightBrowserCalls::default();
        calls.register(
            JsonRpcRequestId::String("request-7".to_string()),
            "op-generated-7".to_string(),
        );

        let request = cancellation_request(
            &json!({
                "method": "notifications/cancelled",
                "params": {
                    "requestId": "request-7",
                    "reason": "caller stopped"
                }
            }),
            &session,
            &calls,
        )
        .expect("queued browser operation should be cancellable");

        assert_eq!(
            request,
            ServiceRequest::CancelBrowserOperation {
                connection_id: "connection-stable".to_string(),
                operation_id: "op-generated-7".to_string(),
                reason: Some("caller stopped".to_string()),
            }
        );
    }

    #[test]
    fn unknown_late_and_reused_request_ids_are_generation_safe() {
        let session = ServerSession {
            mcp_connection_id: "connection-stable".to_string(),
            config: None,
        };
        let calls = InFlightBrowserCalls::default();
        let notification = json!({
            "method": "notifications/cancelled",
            "params": { "requestId": 7 }
        });
        assert!(cancellation_request(&notification, &session, &calls).is_none());

        let request_id = JsonRpcRequestId::Number("7".to_string());
        calls.register(request_id.clone(), "op-old".to_string());
        let delayed_cancellation =
            cancellation_request(&notification, &session, &calls).expect("active cancellation");
        calls.complete(&request_id, "op-old");
        assert!(cancellation_request(&notification, &session, &calls).is_none());

        calls.register(request_id.clone(), "op-new".to_string());
        calls.complete(&request_id, "op-old");
        assert_eq!(
            delayed_cancellation,
            ServiceRequest::CancelBrowserOperation {
                connection_id: "connection-stable".to_string(),
                operation_id: "op-old".to_string(),
                reason: None,
            }
        );
        assert_eq!(
            cancellation_request(&notification, &session, &calls),
            Some(ServiceRequest::CancelBrowserOperation {
                connection_id: "connection-stable".to_string(),
                operation_id: "op-new".to_string(),
                reason: None,
            })
        );
        calls.complete(&request_id, "op-new");
        assert!(cancellation_request(&notification, &session, &calls).is_none());
    }

    #[test]
    fn eof_disconnect_request_is_emitted_once_for_the_connection() {
        let mut notified = false;
        assert_eq!(
            disconnect_request_once(&mut notified, "connection-stable"),
            Some(ServiceRequest::BrowserClientDisconnected {
                connection_id: "connection-stable".to_string(),
            })
        );
        assert_eq!(
            disconnect_request_once(&mut notified, "connection-stable"),
            None
        );
    }

    #[test]
    fn browser_request_context_scope_restores_previous_value() {
        let provenance = browser_caller_provenance(&json!({}), "connection", None);
        let (_, outer) = browser_call_context(&json!({ "id": "outer" }), &provenance);
        let (_, inner) = browser_call_context(&json!({ "id": "inner" }), &provenance);
        let outer_operation_id = outer.operation_identity.operation_id.clone();
        let inner_operation_id = inner.operation_identity.operation_id.clone();
        assert_eq!(current_browser_request_context(), None);
        with_browser_request_context(outer, || {
            assert_eq!(
                current_browser_request_context()
                    .expect("outer context")
                    .operation_identity
                    .operation_id,
                outer_operation_id
            );
            with_browser_request_context(inner, || {
                assert_eq!(
                    current_browser_request_context()
                        .expect("inner context")
                        .operation_identity
                        .operation_id,
                    inner_operation_id
                );
            });
            assert_eq!(
                current_browser_request_context()
                    .expect("restored outer context")
                    .operation_identity
                    .operation_id,
                outer_operation_id
            );
        });
        assert_eq!(current_browser_request_context(), None);
    }

    #[test]
    fn initialize_model_capabilities_gate_capture_schema() {
        let session = parse_model_session_info(
            &json!({
                "method": "initialize",
                "params": {
                    "model": "gpt-5.6-luna",
                    "modelCapabilities": {
                        "images": false
                    }
                }
            }),
            None,
        );
        assert_eq!(session.supports_images, Some(false));

        let tools = tool_definitions(&session);
        let observe = tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "observe")
            .expect("observe tool");
        assert!(
            observe["inputSchema"]["properties"]
                .get("capture_screen")
                .is_none()
        );
    }

    #[test]
    fn initialize_model_capability_wins_over_process_fallback() {
        for (capability, fallback) in [(false, true), (true, false)] {
            let session = parse_model_session_info(
                &json!({
                    "method": "initialize",
                    "params": {
                        "model": "gpt-5.6-luna",
                        "modelCapabilities": {
                            "images": capability
                        }
                    }
                }),
                Some(fallback),
            );
            assert_eq!(session.supports_images, Some(capability));
            assert_eq!(session.can_receive_images(), capability);
        }
    }

    #[test]
    fn process_image_capability_is_only_a_missing_metadata_fallback() {
        for fallback in [false, true] {
            let session = parse_model_session_info(
                &json!({
                    "method": "initialize",
                    "params": {
                        "model": "unknown/model"
                    }
                }),
                Some(fallback),
            );
            assert_eq!(session.supports_images, Some(fallback));
            assert_eq!(session.can_receive_images(), fallback);
        }
    }

    #[test]
    fn unknown_models_fail_closed_for_image_delivery() {
        for model in [
            Value::Null,
            Value::String("opencode/deepseek-v4-flash-free".to_string()),
        ] {
            let session = parse_model_session_info(
                &json!({
                    "method": "initialize",
                    "params": {
                        "model": model
                    }
                }),
                None,
            );
            assert_eq!(session.supports_images, None);
            assert!(!session.can_receive_images());
        }
    }

    #[test]
    fn codex_turn_model_inference_and_unknown_model_fail_closed() {
        let tool_call = json!({
            "method": "tools/call",
            "params": {
                "_meta": {
                    "x-codex-turn-metadata": {
                        "model": "gpt-5.6-luna"
                    }
                }
            }
        });
        assert_eq!(
            model_image_capability_for_tool_call(&tool_call, None, false, None),
            Some(true)
        );

        let text_only_call = json!({
            "params": {
                "_meta": {
                    "x-codex-turn-metadata": serde_json::to_string(&json!({
                        "model": "opencode/deepseek-v4-flash-free"
                    })).unwrap()
                }
            }
        });
        assert_eq!(
            model_image_capability_for_tool_call(&text_only_call, None, false, None),
            None
        );
    }

    #[test]
    fn explicit_capabilities_and_process_override_precede_turn_model_inference() {
        let tool_call = json!({
            "params": {
                "_meta": {
                    "x-codex-turn-metadata": {
                        "model": "gpt-5.6-luna"
                    }
                }
            }
        });
        assert_eq!(
            model_image_capability_for_tool_call(&tool_call, Some(false), true, None),
            Some(false)
        );
        assert_eq!(
            model_image_capability_for_tool_call(&tool_call, None, false, Some(false)),
            Some(false)
        );

        let explicit_turn = json!({
            "params": {
                "_meta": {
                    "x-codex-turn-metadata": {
                        "model": "opencode/deepseek-v4-flash-free",
                        "modelCapabilities": {"supportsImages": true}
                    }
                }
            }
        });
        assert_eq!(
            model_image_capability_for_tool_call(&explicit_turn, Some(false), true, Some(false)),
            Some(true)
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
        // A one-byte reader capacity deterministically splits every CRLF pair.
        let mut reader = tokio::io::BufReader::with_capacity(1, Cursor::new(input.into_bytes()));
        let mut line_buf = Vec::with_capacity(128);
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
        let mut line_buf = Vec::with_capacity(128);
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

    #[tokio::test]
    async fn json_line_frame_limit_accepts_boundary_and_rejects_overflow() {
        const LIMIT: usize = 32;
        let at_limit = format!("{{\"value\":\"{}\"}}", "x".repeat(LIMIT - 12));
        assert_eq!(at_limit.len(), LIMIT);

        let mut reader = Cursor::new(format!("{at_limit}\n").into_bytes());
        let mut line_buf = Vec::new();
        let mut payload_buf = Vec::new();
        let (_, framing) =
            read_message_with_limit(&mut reader, &mut line_buf, &mut payload_buf, LIMIT)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(framing, MessageFraming::JsonLine);

        let mut reader = Cursor::new(format!(" {at_limit}\n").into_bytes());
        let error = read_message_with_limit(&mut reader, &mut line_buf, &mut payload_buf, LIMIT)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("MCP line exceeds maximum frame size")
        );
    }

    #[tokio::test]
    async fn content_length_frame_limit_rejects_before_payload_allocation() {
        const LIMIT: usize = 32;
        let payload = format!("{{\"value\":\"{}\"}}", "x".repeat(LIMIT - 12));
        assert_eq!(payload.len(), LIMIT);
        let mut reader =
            Cursor::new(format!("Content-Length: {LIMIT}\r\n\r\n{payload}").into_bytes());
        let mut line_buf = Vec::new();
        let mut payload_buf = Vec::new();
        let (_, framing) =
            read_message_with_limit(&mut reader, &mut line_buf, &mut payload_buf, LIMIT)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(framing, MessageFraming::ContentLength);

        let mut reader = Cursor::new(format!("Content-Length: {}\r\n\r\n", LIMIT + 1).into_bytes());
        payload_buf.clear();
        payload_buf.shrink_to_fit();
        let error = read_message_with_limit(&mut reader, &mut line_buf, &mut payload_buf, LIMIT)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("MCP Content-Length payload exceeds maximum frame size")
        );
        assert_eq!(payload_buf.capacity(), 0);
    }

    #[tokio::test]
    async fn aggregate_content_length_headers_are_bounded() {
        const LIMIT: usize = 32;
        let mut line_buf = Vec::new();
        let mut payload_buf = Vec::new();

        let mut reader = Cursor::new(b"Content-Length: 2\r\nX-Pad: 12345678\r\n\r\n{}".to_vec());
        let (_, framing) =
            read_message_with_limit(&mut reader, &mut line_buf, &mut payload_buf, LIMIT)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(framing, MessageFraming::ContentLength);

        let mut reader = Cursor::new(b"Content-Length: 2\r\nX-Pad: 123456789\r\n\r\n{}".to_vec());
        let error = read_message_with_limit(&mut reader, &mut line_buf, &mut payload_buf, LIMIT)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("MCP line exceeds maximum frame size")
        );
    }
}
