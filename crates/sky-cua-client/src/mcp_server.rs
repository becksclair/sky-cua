use std::io::{self, BufRead, BufReader, Write};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use sky_cua_platform::model::{
    ActionName, ActionRequest, AppInfo, AppSelector, AppStateSnapshot, ElementNode, ServiceRequest,
    ServiceResponse,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppStateDetail {
    Full,
    Compact,
}

pub fn serve(service: ServiceClient, heuristics: HeuristicsRegistry) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    let mut initialized = false;

    while let Some((message, framing)) = read_message(&mut reader)? {
        let response = match handle_message(&service, &heuristics, &mut initialized, message) {
            Ok(Some(response)) => Some(response),
            Ok(None) => None,
            Err(error) => Some(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {
                    "code": -32603,
                    "message": error.to_string(),
                }
            })),
        };
        if let Some(response) = response {
            write_message(&mut writer, &response, framing)?;
        }
    }

    Ok(())
}

fn handle_message(
    service: &ServiceClient,
    heuristics: &HeuristicsRegistry,
    initialized: &mut bool,
    body: Value,
) -> Result<Option<Value>> {
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("incoming MCP message did not include a method"))?;
    let id = body.get("id").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            *initialized = true;
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
            if !*initialized {
                return Ok(Some(not_initialized(id)));
            }
            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": tools_list_result()
            })))
        }
        "tools/call" => {
            if !*initialized {
                return Ok(Some(not_initialized(id)));
            }
            let tool_name = body
                .pointer("/params/name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("tools/call did not include params.name"))?;
            let arguments = body
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = handle_tool_call(service, heuristics, tool_name, arguments)?;
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

fn handle_tool_call(
    service: &ServiceClient,
    heuristics: &HeuristicsRegistry,
    tool_name: &str,
    arguments: Value,
) -> Result<Value> {
    match tool_name {
        "list_apps" => match service.call(&ServiceRequest::ListApps)? {
            ServiceResponse::ListApps {
                environment,
                apps,
                diagnostics,
            } => {
                let summary = list_apps_summary(&apps);
                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": summary
                    }],
                    "structuredContent": {
                        "environment": environment,
                        "apps": apps,
                        "diagnostics": diagnostics
                    },
                    "isError": false
                }))
            }
            ServiceResponse::Error { code, message } => tool_error(code, message),
            other => Err(anyhow!("unexpected response for list_apps: {other:?}")),
        },
        "get_app_state" => {
            let selector = parse_app_selector(&arguments);
            let detail = parse_app_state_detail(&arguments);
            match service.call(&ServiceRequest::GetAppState { selector })? {
                ServiceResponse::GetAppState { mut snapshot } => {
                    enrich_snapshot(heuristics, &mut snapshot);
                    let structured_content = match detail {
                        AppStateDetail::Full => serde_json::to_value(&snapshot)?,
                        AppStateDetail::Compact => compact_snapshot(&snapshot),
                    };
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": snapshot_summary(&snapshot)
                        }],
                        "structuredContent": structured_content,
                        "isError": false
                    }))
                }
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for get_app_state: {other:?}")),
            }
        }
        "click" => handle_action_call(service, ActionName::Click, arguments),
        "perform_secondary_action" => {
            handle_action_call(service, ActionName::PerformSecondaryAction, arguments)
        }
        "scroll" => handle_action_call(service, ActionName::Scroll, arguments),
        "drag" => handle_action_call(service, ActionName::Drag, arguments),
        "type_text" => handle_action_call(service, ActionName::TypeText, arguments),
        "press_key" => handle_action_call(service, ActionName::PressKey, arguments),
        "set_value" => handle_action_call(service, ActionName::SetValue, arguments),
        _ => tool_error("UnknownTool", format!("unknown tool: {tool_name}")),
    }
}

fn handle_action_call(
    service: &ServiceClient,
    action: ActionName,
    arguments: Value,
) -> Result<Value> {
    let snapshot_id = arguments
        .get("snapshot_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if snapshot_id.is_none() {
        return tool_error(
            "ComputerUseInactive",
            "Computer Use is not active for this action. Call get_app_state first and pass the current snapshot_id with the action.",
        );
    }
    let element_index = arguments
        .get("element_index")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    let request = ActionRequest {
        action,
        snapshot_id,
        element_index,
        arguments,
        resolved_element: None,
        resolved_target_element: None,
        resolved_capture: None,
        resolved_focused_app: None,
        environment: None,
    };
    match service.call(&ServiceRequest::ExecuteAction {
        request: Box::new(request),
    })? {
        ServiceResponse::ExecuteAction { outcome } => Ok(json!({
            "content": [{
                "type": "text",
                "text": action_summary(&outcome)
            }],
            "structuredContent": outcome,
            "isError": !outcome.success
        })),
        ServiceResponse::Error { code, message } => tool_error(code, message),
        other => Err(anyhow!("unexpected response for action call: {other:?}")),
    }
}

fn enrich_snapshot(heuristics: &HeuristicsRegistry, snapshot: &mut AppStateSnapshot) {
    if snapshot.app_guidance.is_none()
        && let Some(focused_app) = snapshot.focused_app.as_ref()
    {
        snapshot.app_guidance = heuristics.resolve_for_focused_app(focused_app);
    }
}

fn snapshot_summary(snapshot: &AppStateSnapshot) -> String {
    let app_name = snapshot
        .focused_app
        .as_ref()
        .map(|app| app.name.clone())
        .unwrap_or_else(|| "no focused app".to_string());
    let mut summary = format!(
        "Snapshot {} captured {} elements for {}.",
        snapshot.snapshot_id,
        snapshot.elements.len(),
        app_name
    );
    if let Some(diag) = portal_approval_pending_diagnostic(&snapshot.diagnostics) {
        summary.push(' ');
        summary.push_str(&portal_approval_summary(diag.message.as_str()));
    }
    if let Some(summary_suffix) = informational_runtime_summary(&snapshot.diagnostics) {
        summary.push(' ');
        summary.push_str(&summary_suffix);
    }
    summary
}

fn list_apps_summary(apps: &[AppInfo]) -> String {
    let app_count = apps.len();
    let preview = apps
        .iter()
        .map(|app| {
            let mut label = app.name.clone();
            label.push_str(" (app_id=");
            label.push_str(&app.app_id);
            if let Some(desktop_file_id) = app
                .desktop_file_id
                .as_deref()
                .filter(|desktop_file_id| !desktop_file_id.is_empty())
            {
                label.push_str(", desktop_file_id=");
                label.push_str(desktop_file_id);
            }
            label.push(')');
            if let Some(window_title) = app
                .window_title
                .as_deref()
                .filter(|title| !title.is_empty())
            {
                label.push_str(", window_title=");
                label.push_str(window_title);
            }
            if app.is_focused_candidate {
                label.push_str(" [focused candidate]");
            }
            label
        })
        .collect::<Vec<_>>();

    if preview.is_empty() {
        format!("Discovered {app_count} accessible Linux apps.")
    } else {
        format!(
            "Discovered {app_count} accessible Linux apps. Apps: {}.",
            preview.join("; ")
        )
    }
}

fn action_summary(outcome: &sky_cua_platform::model::ActionOutcome) -> String {
    if outcome.code == "PortalApprovalPending" {
        return portal_approval_summary(&outcome.message);
    }
    let mut summary = outcome.message.clone();
    if let Some(summary_suffix) = informational_runtime_summary(&outcome.diagnostics) {
        summary.push(' ');
        summary.push_str(&summary_suffix);
    }
    summary
}

fn tool_error(code: impl Into<String>, message: impl Into<String>) -> Result<Value> {
    let code = code.into();
    let message = message.into();
    let text = if code == "PortalApprovalPending" {
        portal_approval_summary(&message)
    } else {
        message.clone()
    };
    Ok(json!({
        "content": [{
            "type": "text",
            "text": text
        }],
        "structuredContent": {
            "code": code
        },
        "isError": true
    }))
}

fn portal_approval_pending_diagnostic(
    diagnostics: &[sky_cua_platform::model::DiagnosticEntry],
) -> Option<&sky_cua_platform::model::DiagnosticEntry> {
    diagnostics
        .iter()
        .find(|diag| diag.code == "PortalApprovalPending")
}

fn portal_approval_summary(message: &str) -> String {
    format!("{message} Approve the KDE portal dialog for screen control, then retry the request.")
}

fn informational_runtime_summary(
    diagnostics: &[sky_cua_platform::model::DiagnosticEntry],
) -> Option<String> {
    let mut parts = Vec::new();
    for diagnostic in diagnostics {
        match diagnostic.code.as_str() {
            "PortalSessionStarted" => parts.push(diagnostic.message.clone()),
            "PortalSessionRestored" => parts.push(diagnostic.message.clone()),
            "PortalSessionRestoreMiss" => parts.push(match diagnostic.details.as_ref() {
                Some(details) => format!("{} Details: {}", diagnostic.message, details),
                None => diagnostic.message.clone(),
            }),
            "PortalSessionRebuilt" => parts.push(match diagnostic.details.as_ref() {
                Some(details) => format!("{} Details: {}", diagnostic.message, details),
                None => diagnostic.message.clone(),
            }),
            "PortalSessionTokenRotated" => parts.push(match diagnostic.details.as_ref() {
                Some(details) => format!("{} Details: {}", diagnostic.message, details),
                None => diagnostic.message.clone(),
            }),
            "CaptureBackendDowngraded" => parts.push(match diagnostic.details.as_ref() {
                Some(details) => format!("{} Details: {}", diagnostic.message, details),
                None => diagnostic.message.clone(),
            }),
            _ => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn not_initialized(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32002,
            "message": "Not initialized"
        }
    })
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "list_apps",
            "description": "List currently exposed desktop applications from the Linux accessibility tree.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "get_app_state",
            "description": "Build a structured desktop app-state snapshot with environment diagnostics and flattened accessibility elements.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "app_id": { "type": "string" },
                    "desktop_file_id": { "type": "string" },
                    "window_title": { "type": "string" },
                    "name": { "type": "string" },
                    "detail": {
                        "type": "string",
                        "enum": ["full", "compact"],
                        "description": "Use compact for fast screenshot-first loops. It keeps identifiers, screenshot metadata, diagnostics, and lean element anchors while omitting verbose element descriptions and static environment/capability details."
                    }
                },
                "additionalProperties": false
            }
        },
        action_tool(
            "click",
            "Click an element by index or x/y coordinates in screenshot pixel coordinates from the current snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "x": coordinate_schema("X coordinate in screenshot pixel coordinates from the snapshot image."),
                "y": coordinate_schema("Y coordinate in screenshot pixel coordinates from the snapshot image.")
            }),
            json!(["snapshot_id"]),
        ),
        action_tool(
            "perform_secondary_action",
            "Perform a secondary click or context action on an element from the current snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "action": { "type": "string" }
            }),
            json!(["snapshot_id", "element_index", "action"]),
        ),
        action_tool(
            "scroll",
            "Scroll within an element or focused area from the current snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"]
                },
                "pages": { "type": "integer", "minimum": 1 }
            }),
            json!(["snapshot_id", "direction"]),
        ),
        action_tool(
            "drag",
            "Drag from one screenshot-pixel point or element to another in the current snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "x": coordinate_schema("Drag start X coordinate in screenshot pixel coordinates."),
                "y": coordinate_schema("Drag start Y coordinate in screenshot pixel coordinates."),
                "from_x": coordinate_schema("Drag start X coordinate in screenshot pixel coordinates."),
                "from_y": coordinate_schema("Drag start Y coordinate in screenshot pixel coordinates."),
                "to_x": coordinate_schema("Drag end X coordinate in screenshot pixel coordinates."),
                "to_y": coordinate_schema("Drag end Y coordinate in screenshot pixel coordinates."),
                "to_element_index": { "type": "integer", "minimum": 0 }
            }),
            json!(["snapshot_id"]),
        ),
        action_tool(
            "type_text",
            "Type literal text into the focused control in the current snapshot.",
            json!({
                "text": { "type": "string" }
            }),
            json!(["snapshot_id", "text"]),
        ),
        action_tool(
            "press_key",
            "Press a keyboard key or key chord in the current snapshot.",
            json!({
                "key": { "type": "string" }
            }),
            json!(["snapshot_id", "key"]),
        ),
        action_tool(
            "set_value",
            "Set an editable element value semantically where supported in the current snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "value": { "type": "string" }
            }),
            json!(["snapshot_id", "element_index", "value"]),
        )
    ])
}

fn tools_list_result() -> Value {
    json!({
        "tools": tool_definitions()
    })
}

fn coordinate_schema(description: &str) -> Value {
    json!({
        "type": "number",
        "description": description
    })
}

fn action_tool(name: &str, description: &str, mut properties: Value, required: Value) -> Value {
    let property_map = properties
        .as_object_mut()
        .expect("properties must be an object");
    property_map.insert(
        "snapshot_id".to_string(),
        json!({
            "type": "string",
            "description": "Current snapshot_id returned by the latest get_app_state call."
        }),
    );
    let input_schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    });
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn parse_app_state_detail(arguments: &Value) -> AppStateDetail {
    match arguments.get("detail").and_then(Value::as_str) {
        Some("compact") => AppStateDetail::Compact,
        _ => AppStateDetail::Full,
    }
}

fn parse_app_selector(arguments: &Value) -> Option<AppSelector> {
    let selector = AppSelector {
        app_id: arguments
            .get("app_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        desktop_file_id: arguments
            .get("desktop_file_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        window_title: arguments
            .get("window_title")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        name: arguments
            .get("name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    };

    if selector.app_id.is_none()
        && selector.desktop_file_id.is_none()
        && selector.window_title.is_none()
        && selector.name.is_none()
    {
        None
    } else {
        Some(selector)
    }
}

fn compact_snapshot(snapshot: &AppStateSnapshot) -> Value {
    let elements: Vec<Value> = snapshot.elements.iter().map(compact_element).collect();
    json!({
        "detail": "compact",
        "snapshot_id": snapshot.snapshot_id,
        "created_at": snapshot.created_at,
        "focused_app": snapshot.focused_app,
        "capture": snapshot.capture,
        "diagnostics": snapshot.diagnostics,
        "app_guidance": snapshot.app_guidance,
        "elements": elements,
        "element_count": snapshot.elements.len()
    })
}

fn compact_element(element: &ElementNode) -> Value {
    json!({
        "element_index": element.element_index,
        "parent_index": element.parent_index,
        "role": element.role,
        "name": element.name,
        "value": element.value,
        "state_flags": element.state_flags,
        "semantic_actions": element.semantic_actions,
        "bounds": element.bounds
    })
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<(Value, MessageFraming)>> {
    let first_line = loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        if !line.is_empty() {
            break line;
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
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err(anyhow!(
                "unexpected EOF while reading MCP headers after: {first_line}"
            ));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        parse_header_line(line, &mut content_length)?;
    }

    let length = content_length.ok_or_else(|| anyhow!("missing Content-Length header"))?;
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(Some((
        serde_json::from_slice(&payload)?,
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

fn write_message(writer: &mut impl Write, message: &Value, framing: MessageFraming) -> Result<()> {
    let payload = serde_json::to_vec(message)?;
    match framing {
        MessageFraming::ContentLength => {
            write!(writer, "Content-Length: {}\r\n\r\n", payload.len())?;
            writer.write_all(&payload)?;
        }
        MessageFraming::JsonLine => {
            writer.write_all(&payload)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use chrono::Utc;
    use serde_json::json;

    use super::{
        MessageFraming, action_summary, compact_element, list_apps_summary, parse_app_state_detail,
        read_message, snapshot_summary, tool_definitions, tools_list_result, write_message,
    };
    use sky_cua_platform::model::{
        ActionOutcome, AppInfo, AppStateSnapshot, CaptureBackendKind, CoordinateSpace,
        DiagnosticEntry, ElementNode, EnvironmentInfo, FocusedApp, InputBackendKind,
        PortalCapabilities, RectF, SemanticBackendKind, SessionKind, ToolAvailability,
        ToolCapabilities,
    };

    #[test]
    fn parses_compact_app_state_detail() {
        assert_eq!(
            parse_app_state_detail(&json!({"detail": "compact"})),
            super::AppStateDetail::Compact
        );
        assert_eq!(
            parse_app_state_detail(&json!({"detail": "full"})),
            super::AppStateDetail::Full
        );
        assert_eq!(
            parse_app_state_detail(&json!({})),
            super::AppStateDetail::Full
        );
    }

    #[test]
    fn compact_element_drops_verbose_description_and_backend_ref() {
        let compact = compact_element(&ElementNode {
            element_index: 7,
            parent_index: Some(1),
            role: "button".to_string(),
            name: Some("OK".to_string()),
            description: Some("verbose guidance that should not ride every loop".to_string()),
            value: None,
            state_flags: vec!["focused".to_string()],
            semantic_actions: vec!["click".to_string()],
            bounds: Some(RectF {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            backend_ref: Some("opaque-backend-ref".to_string()),
        });

        assert_eq!(compact["element_index"], 7);
        assert_eq!(compact["role"], "button");
        assert!(compact.get("description").is_none());
        assert!(compact.get("backend_ref").is_none());
        assert_eq!(compact["semantic_actions"][0], "click");
    }

    #[test]
    fn action_tool_schemas_are_strict_and_snapshot_scoped() {
        let tools = tool_definitions();
        let click = tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "click")
            .expect("click tool");
        let schema = &click["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"][0], "snapshot_id");
        assert!(schema["properties"].get("snapshot_id").is_some());
        assert!(schema.get("anyOf").is_none());

        let type_text = tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "type_text")
            .expect("type_text tool");
        assert_eq!(type_text["inputSchema"]["additionalProperties"], false);
        assert_eq!(
            type_text["inputSchema"]["required"],
            json!(["snapshot_id", "text"])
        );
    }

    #[test]
    fn tools_list_result_omits_empty_next_cursor() {
        let result = tools_list_result();

        assert!(result.get("tools").is_some());
        assert!(result.get("nextCursor").is_none());
    }

    #[test]
    fn list_apps_summary_includes_selectors_for_plain_text_clients() {
        let summary = list_apps_summary(&[
            AppInfo {
                app_id: "kwin:{abc}".to_string(),
                name: "org.kde.kate".to_string(),
                pid: None,
                executable: Some("org.kde.kate".to_string()),
                desktop_file_id: Some("org.kde.kate.desktop".to_string()),
                toolkit_guess: Some("Wayland".to_string()),
                window_title: Some("Untitled — Kate".to_string()),
                is_focused_candidate: false,
            },
            AppInfo {
                app_id: "x11:0x1".to_string(),
                name: "xfreerdp".to_string(),
                pid: None,
                executable: Some("xfreerdp".to_string()),
                desktop_file_id: Some("xfreerdp3.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: None,
                is_focused_candidate: true,
            },
        ]);

        assert!(summary.contains("Discovered 2 accessible Linux apps."));
        assert!(summary.contains("org.kde.kate (app_id=kwin:{abc}"));
        assert!(summary.contains("desktop_file_id=org.kde.kate.desktop"));
        assert!(summary.contains("window_title=Untitled — Kate"));
        assert!(summary.contains("xfreerdp"));
        assert!(summary.contains("[focused candidate]"));
    }

    #[test]
    fn snapshot_summary_surfaces_portal_approval_guidance() {
        let snapshot = AppStateSnapshot {
            snapshot_id: "snap-1".to_string(),
            created_at: Utc::now(),
            environment: EnvironmentInfo {
                session_kind: SessionKind::Wayland,
                compositor: Some("kde-kwin-wayland".to_string()),
                desktop_environment: Some("KDE".to_string()),
                capture_backend: CaptureBackendKind::PortalPipeWire,
                input_backend: InputBackendKind::PortalRemoteDesktop,
                semantic_backend: SemanticBackendKind::Atspi,
                portal_capabilities: PortalCapabilities {
                    screencast_version: Some(5),
                    remote_desktop_version: Some(2),
                    screenshot_version: Some(2),
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: Some("wayland".to_string()),
                display: None,
                wayland_display: Some("wayland-0".to_string()),
            },
            capabilities: ToolCapabilities {
                list_apps: ToolAvailability {
                    available: true,
                    reason: None,
                },
                get_app_state: ToolAvailability {
                    available: true,
                    reason: None,
                },
                click: ToolAvailability {
                    available: true,
                    reason: None,
                },
                perform_secondary_action: ToolAvailability {
                    available: true,
                    reason: None,
                },
                scroll: ToolAvailability {
                    available: true,
                    reason: None,
                },
                drag: ToolAvailability {
                    available: true,
                    reason: None,
                },
                type_text: ToolAvailability {
                    available: true,
                    reason: None,
                },
                press_key: ToolAvailability {
                    available: true,
                    reason: None,
                },
                set_value: ToolAvailability {
                    available: true,
                    reason: None,
                },
            },
            focused_app: Some(FocusedApp {
                app_id: "app-1".to_string(),
                name: "zenity".to_string(),
                pid: Some(123),
                desktop_file_id: Some("zenity.desktop".to_string()),
                toolkit_guess: Some("GTK".to_string()),
                window_title: Some("sky-cua zenity smoke".to_string()),
            }),
            capture: None,
            elements: Vec::new(),
            diagnostics: vec![DiagnosticEntry {
                code: "PortalApprovalPending".to_string(),
                message: "timed out waiting for the RemoteDesktop portal session to start"
                    .to_string(),
                details: None,
            }],
            app_guidance: None,
        };

        let summary = snapshot_summary(&snapshot);
        assert!(summary.contains("Approve the KDE portal dialog"));
        assert!(
            summary.contains("timed out waiting for the RemoteDesktop portal session to start")
        );
    }

    #[test]
    fn action_summary_surfaces_portal_approval_guidance() {
        let outcome = ActionOutcome {
            success: false,
            message: "timed out waiting for the RemoteDesktop portal session to start".to_string(),
            code: "PortalApprovalPending".to_string(),
            diagnostics: Vec::new(),
        };

        let summary = action_summary(&outcome);
        assert!(summary.contains("Approve the KDE portal dialog"));
        assert!(
            summary.contains("timed out waiting for the RemoteDesktop portal session to start")
        );
    }

    #[test]
    fn snapshot_summary_mentions_portal_session_lifecycle() {
        let snapshot = AppStateSnapshot {
            snapshot_id: "snap-2".to_string(),
            created_at: Utc::now(),
            environment: EnvironmentInfo {
                session_kind: SessionKind::Wayland,
                compositor: Some("kde-kwin-wayland".to_string()),
                desktop_environment: Some("KDE".to_string()),
                capture_backend: CaptureBackendKind::PortalPipeWire,
                input_backend: InputBackendKind::PortalRemoteDesktop,
                semantic_backend: SemanticBackendKind::Atspi,
                portal_capabilities: PortalCapabilities {
                    screencast_version: Some(5),
                    remote_desktop_version: Some(2),
                    screenshot_version: Some(2),
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: Some("wayland".to_string()),
                display: None,
                wayland_display: Some("wayland-0".to_string()),
            },
            capabilities: ToolCapabilities {
                list_apps: ToolAvailability {
                    available: true,
                    reason: None,
                },
                get_app_state: ToolAvailability {
                    available: true,
                    reason: None,
                },
                click: ToolAvailability {
                    available: true,
                    reason: None,
                },
                perform_secondary_action: ToolAvailability {
                    available: true,
                    reason: None,
                },
                scroll: ToolAvailability {
                    available: true,
                    reason: None,
                },
                drag: ToolAvailability {
                    available: true,
                    reason: None,
                },
                type_text: ToolAvailability {
                    available: true,
                    reason: None,
                },
                press_key: ToolAvailability {
                    available: true,
                    reason: None,
                },
                set_value: ToolAvailability {
                    available: true,
                    reason: None,
                },
            },
            focused_app: Some(FocusedApp {
                app_id: "app-2".to_string(),
                name: "xmessage".to_string(),
                pid: Some(456),
                desktop_file_id: Some("xmessage.desktop".to_string()),
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("portal lifecycle probe".to_string()),
            }),
            capture: None,
            elements: Vec::new(),
            diagnostics: vec![
                DiagnosticEntry {
                    code: "PortalSessionStarted".to_string(),
                    message: "Started a new combined RemoteDesktop and ScreenCast portal session."
                        .to_string(),
                    details: None,
                },
                DiagnosticEntry {
                    code: "PortalSessionRebuilt".to_string(),
                    message: "Rebuilt the cached portal session after PipeWire capture failed."
                        .to_string(),
                    details: Some("remote fd closed unexpectedly".to_string()),
                },
            ],
            app_guidance: None,
        };

        let summary = snapshot_summary(&snapshot);
        assert!(
            summary.contains("Started a new combined RemoteDesktop and ScreenCast portal session.")
        );
        assert!(
            summary.contains("Rebuilt the cached portal session after PipeWire capture failed.")
        );
        assert!(summary.contains("remote fd closed unexpectedly"));
    }

    #[test]
    fn action_summary_mentions_portal_session_rebuild_details() {
        let outcome = ActionOutcome {
            success: true,
            message: "typed text via portal session".to_string(),
            code: "ActionPerformed".to_string(),
            diagnostics: vec![DiagnosticEntry {
                code: "PortalSessionRebuilt".to_string(),
                message: "Rebuilt the cached portal session after PipeWire capture failed."
                    .to_string(),
                details: Some("capture timed out on cached stream".to_string()),
            }],
        };

        let summary = action_summary(&outcome);
        assert!(summary.contains("typed text via portal session"));
        assert!(
            summary.contains("Rebuilt the cached portal session after PipeWire capture failed.")
        );
        assert!(summary.contains("capture timed out on cached stream"));
    }

    #[test]
    fn read_message_accepts_content_length_framing() {
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

        let (message, framing) = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(framing, MessageFraming::ContentLength);
        assert_eq!(message["method"], "initialize");
        assert_eq!(message["id"], 1);
    }

    #[test]
    fn read_message_accepts_line_delimited_json() {
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

        let (message, framing) = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(framing, MessageFraming::JsonLine);
        assert_eq!(message["method"], "initialize");
        assert_eq!(message["id"], 1);
    }

    #[test]
    fn write_message_mirrors_line_delimited_json() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "ok": true
            }
        });
        let mut output = Vec::new();

        write_message(&mut output, &message, MessageFraming::JsonLine).unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert_eq!(
            rendered,
            format!("{}\n", serde_json::to_string(&message).unwrap())
        );
    }

    #[test]
    fn snapshot_summary_mentions_portal_restore_and_token_rotation() {
        let snapshot = AppStateSnapshot {
            snapshot_id: "snap-restore".to_string(),
            created_at: Utc::now(),
            environment: EnvironmentInfo {
                session_kind: SessionKind::Wayland,
                compositor: Some("kde-kwin-wayland".to_string()),
                desktop_environment: Some("KDE".to_string()),
                capture_backend: CaptureBackendKind::PortalPipeWire,
                input_backend: InputBackendKind::PortalRemoteDesktop,
                semantic_backend: SemanticBackendKind::Atspi,
                portal_capabilities: PortalCapabilities {
                    screencast_version: Some(5),
                    remote_desktop_version: Some(2),
                    screenshot_version: Some(2),
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: Some("wayland".to_string()),
                display: None,
                wayland_display: Some("wayland-0".to_string()),
            },
            capabilities: ToolCapabilities {
                list_apps: ToolAvailability {
                    available: true,
                    reason: None,
                },
                get_app_state: ToolAvailability {
                    available: true,
                    reason: None,
                },
                click: ToolAvailability {
                    available: true,
                    reason: None,
                },
                perform_secondary_action: ToolAvailability {
                    available: true,
                    reason: None,
                },
                scroll: ToolAvailability {
                    available: true,
                    reason: None,
                },
                drag: ToolAvailability {
                    available: true,
                    reason: None,
                },
                type_text: ToolAvailability {
                    available: true,
                    reason: None,
                },
                press_key: ToolAvailability {
                    available: true,
                    reason: None,
                },
                set_value: ToolAvailability {
                    available: true,
                    reason: None,
                },
            },
            focused_app: Some(FocusedApp {
                app_id: "app-restore".to_string(),
                name: "krita".to_string(),
                pid: Some(42),
                desktop_file_id: Some("krita.desktop".to_string()),
                toolkit_guess: Some("Qt".to_string()),
                window_title: Some("Krita".to_string()),
            }),
            capture: None,
            elements: Vec::new(),
            diagnostics: vec![
                DiagnosticEntry {
                    code: "PortalSessionRestored".to_string(),
                    message: "Reused a persisted RemoteDesktop approval token for the combined portal session.".to_string(),
                    details: None,
                },
                DiagnosticEntry {
                    code: "PortalSessionTokenRotated".to_string(),
                    message: "Rotated the persisted RemoteDesktop restore token for future sessions.".to_string(),
                    details: Some("token_path=/tmp/portal-tokens.json".to_string()),
                },
            ],
            app_guidance: None,
        };

        let summary = snapshot_summary(&snapshot);
        assert!(summary.contains("Reused a persisted RemoteDesktop approval token"));
        assert!(summary.contains("Rotated the persisted RemoteDesktop restore token"));
        assert!(summary.contains("token_path=/tmp/portal-tokens.json"));
    }

    #[test]
    fn snapshot_summary_mentions_capture_backend_downgrade() {
        let snapshot = AppStateSnapshot {
            snapshot_id: "snap-3".to_string(),
            created_at: Utc::now(),
            environment: EnvironmentInfo {
                session_kind: SessionKind::Wayland,
                compositor: Some("kde-kwin-wayland".to_string()),
                desktop_environment: Some("KDE".to_string()),
                capture_backend: CaptureBackendKind::PortalPipeWire,
                input_backend: InputBackendKind::PortalRemoteDesktop,
                semantic_backend: SemanticBackendKind::Atspi,
                portal_capabilities: PortalCapabilities {
                    screencast_version: Some(5),
                    remote_desktop_version: Some(2),
                    screenshot_version: Some(2),
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: Some("wayland".to_string()),
                display: None,
                wayland_display: Some("wayland-0".to_string()),
            },
            capabilities: ToolCapabilities {
                list_apps: ToolAvailability {
                    available: true,
                    reason: None,
                },
                get_app_state: ToolAvailability {
                    available: true,
                    reason: None,
                },
                click: ToolAvailability {
                    available: true,
                    reason: None,
                },
                perform_secondary_action: ToolAvailability {
                    available: true,
                    reason: None,
                },
                scroll: ToolAvailability {
                    available: true,
                    reason: None,
                },
                drag: ToolAvailability {
                    available: true,
                    reason: None,
                },
                type_text: ToolAvailability {
                    available: true,
                    reason: None,
                },
                press_key: ToolAvailability {
                    available: true,
                    reason: None,
                },
                set_value: ToolAvailability {
                    available: true,
                    reason: None,
                },
            },
            focused_app: Some(FocusedApp {
                app_id: "app-3".to_string(),
                name: "discord".to_string(),
                pid: Some(999),
                desktop_file_id: Some("discord.desktop".to_string()),
                toolkit_guess: Some("Electron".to_string()),
                window_title: Some("@Sky - Discord".to_string()),
            }),
            capture: None,
            elements: Vec::new(),
            diagnostics: vec![DiagnosticEntry {
                code: "CaptureBackendDowngraded".to_string(),
                message:
                    "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback"
                        .to_string(),
                details: Some(
                    "primary_backend=portal_pipe_wire image_backend=portal_screenshot".to_string(),
                ),
            }],
            app_guidance: None,
        };

        let summary = snapshot_summary(&snapshot);
        assert!(summary.contains(
            "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback"
        ));
        assert!(summary.contains("image_backend=portal_screenshot"));
    }
}
