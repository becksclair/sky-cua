use std::io::{self, BufRead, BufReader, Write};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use sky_cua_platform::model::{
    ActionName, ActionRequest, AppInfo, AppSelector, AppStateSnapshot, ElementNode, ServiceRequest,
    ServiceResponse, WindowInfo,
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
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let response = match handle_message(&service, &heuristics, &mut initialized, message) {
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
        "doctor" => match service.call(&ServiceRequest::Doctor)? {
            ServiceResponse::Doctor { report } => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": report.readiness.recommended_next_step
                }],
                "structuredContent": report,
                "isError": false
            })),
            ServiceResponse::Error { code, message } => tool_error(code, message),
            other => Err(anyhow!("unexpected response for doctor: {other:?}")),
        },
        "setup_accessibility" => match service.call(&ServiceRequest::SetupAccessibility)? {
            ServiceResponse::SetupAccessibility { report } => {
                let is_error = setup_accessibility_is_error(&report);
                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": report.after.readiness.recommended_next_step
                    }],
                    "structuredContent": report,
                    "isError": is_error
                }))
            }
            ServiceResponse::Error { code, message } => tool_error(code, message),
            other => Err(anyhow!(
                "unexpected response for setup_accessibility: {other:?}"
            )),
        },
        "setup_window_targeting" => match service.call(&ServiceRequest::SetupWindowTargeting)? {
            ServiceResponse::SetupWindowTargeting { report } => {
                let is_error = setup_window_targeting_is_error(&report);
                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": report.message
                    }],
                    "structuredContent": report,
                    "isError": is_error
                }))
            }
            ServiceResponse::Error { code, message } => tool_error(code, message),
            other => Err(anyhow!(
                "unexpected response for setup_window_targeting: {other:?}"
            )),
        },
        "list_apps" => match service.call(&ServiceRequest::ListApps)? {
            ServiceResponse::ListApps {
                environment,
                apps,
                diagnostics,
            } => {
                let runtime_error = list_apps_error_diagnostic(&diagnostics);
                let is_error = runtime_error.is_some();
                let summary = runtime_error
                    .map(|diagnostic| diagnostic.message.clone())
                    .unwrap_or_else(|| list_apps_summary(&apps));
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
                    "isError": is_error
                }))
            }
            ServiceResponse::Error { code, message } => tool_error(code, message),
            other => Err(anyhow!("unexpected response for list_apps: {other:?}")),
        },
        "list_windows" => match service.call(&ServiceRequest::ListWindows)? {
            ServiceResponse::ListWindows {
                environment,
                windows,
                diagnostics,
            } => {
                let runtime_error = diagnostics.first();
                let is_error = runtime_error.is_some();
                let summary = runtime_error
                    .map(|diagnostic| diagnostic.message.clone())
                    .unwrap_or_else(|| list_windows_summary(&windows));
                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": summary
                    }],
                    "structuredContent": {
                        "environment": environment,
                        "windows": windows,
                        "diagnostics": diagnostics
                    },
                    "isError": is_error
                }))
            }
            ServiceResponse::Error { code, message } => tool_error(code, message),
            other => Err(anyhow!("unexpected response for list_windows: {other:?}")),
        },
        "focused_window" => match service.call(&ServiceRequest::FocusedWindow)? {
            ServiceResponse::FocusedWindow {
                environment,
                window,
                diagnostics,
            } => {
                let runtime_error = diagnostics.first();
                let is_error = runtime_error.is_some();
                let summary = runtime_error
                    .map(|diagnostic| diagnostic.message.clone())
                    .unwrap_or_else(|| focused_window_summary(window.as_deref()));
                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": summary
                    }],
                    "structuredContent": {
                        "environment": environment,
                        "window": window,
                        "diagnostics": diagnostics
                    },
                    "isError": is_error
                }))
            }
            ServiceResponse::Error { code, message } => tool_error(code, message),
            other => Err(anyhow!("unexpected response for focused_window: {other:?}")),
        },
        "activate_window" => {
            let target = match parse_window_target(arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&ServiceRequest::ActivateWindow { target })? {
                ServiceResponse::ActivateWindow { outcome } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": action_summary(&outcome)
                    }],
                    "structuredContent": outcome,
                    "isError": !outcome.success
                })),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for activate_window: {other:?}"
                )),
            }
        }
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
        "focus_element" => handle_action_call(service, ActionName::FocusElement, arguments),
        "activate_element" => handle_action_call(service, ActionName::ActivateElement, arguments),
        "select_element" => handle_action_call(service, ActionName::SelectElement, arguments),
        "expand_element" => handle_action_call(service, ActionName::ExpandElement, arguments),
        "collapse_element" => handle_action_call(service, ActionName::CollapseElement, arguments),
        "toggle_element" => handle_action_call(service, ActionName::ToggleElement, arguments),
        "click" => handle_action_call(service, ActionName::Click, arguments),
        "perform_action" => handle_action_call(service, ActionName::PerformAction, arguments),
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
        format!("Discovered {app_count} accessible desktop apps.")
    } else {
        format!(
            "Discovered {app_count} accessible desktop apps. Apps: {}.",
            preview.join("; ")
        )
    }
}

fn list_windows_summary(windows: &[WindowInfo]) -> String {
    if windows.is_empty() {
        return "Discovered 0 desktop windows.".to_string();
    }

    let preview = windows
        .iter()
        .map(|window| {
            let mut label = window
                .title
                .clone()
                .or_else(|| window.app_id.clone())
                .unwrap_or_else(|| window.window_id.clone());
            label.push_str(" (window_id=");
            label.push_str(&window.window_id);
            label.push_str(", backend=");
            label.push_str(&window.backend);
            if let Some(app_id) = window.app_id.as_deref().filter(|value| !value.is_empty()) {
                label.push_str(", app_id=");
                label.push_str(app_id);
            }
            if let Some(pid) = window.pid {
                label.push_str(", pid=");
                label.push_str(&pid.to_string());
            }
            if let Some(terminal) = &window.terminal {
                label.push_str(", tty=");
                label.push_str(&terminal.tty);
                if let Some(active) = &terminal.active_process {
                    label.push_str(", active_process=");
                    label.push_str(&active.command_name);
                }
            }
            if window.focused {
                label.push_str(" [focused]");
            }
            label.push(')');
            label
        })
        .collect::<Vec<_>>();

    format!(
        "Discovered {} desktop windows. Windows: {}.",
        windows.len(),
        preview.join("; ")
    )
}

fn focused_window_summary(window: Option<&WindowInfo>) -> String {
    let Some(window) = window else {
        return "No focused desktop window was reported by the active windowing backends."
            .to_string();
    };

    let label = window
        .title
        .as_deref()
        .or(window.app_id.as_deref())
        .unwrap_or(&window.window_id);
    format!(
        "Focused desktop window: {label} (window_id={}, backend={}).",
        window.window_id, window.backend
    )
}

fn parse_window_target(arguments: Value) -> Result<sky_cua_platform::model::WindowTarget> {
    let target: sky_cua_platform::model::WindowTarget =
        serde_json::from_value(arguments).context("invalid activate_window target arguments")?;
    if !target.has_target() {
        return Err(anyhow!(
            "activate_window requires one of window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd"
        ));
    }
    Ok(target)
}

fn setup_window_targeting_is_error(
    report: &sky_cua_platform::model::WindowTargetingSetupReport,
) -> bool {
    report.windows_error.is_some()
}

fn setup_accessibility_is_error(
    report: &sky_cua_platform::model::AccessibilitySetupReport,
) -> bool {
    !report.accessibility_command.ok || !report.after.readiness.can_build_accessibility_tree
}

fn list_apps_error_diagnostic(
    diagnostics: &[sky_cua_platform::model::DiagnosticEntry],
) -> Option<&sky_cua_platform::model::DiagnosticEntry> {
    diagnostics.first()
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

fn invalid_request_tool_error(message: impl Into<String>) -> Result<Value> {
    tool_error("InvalidRequest", message)
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
            "PortalSessionStarted" | "PortalSessionRestored" => {
                parts.push(diagnostic.message.clone());
            }
            "PortalSessionRestoreMiss"
            | "PortalSessionRebuilt"
            | "PortalSessionTokenRotated"
            | "CaptureBackendDowngraded"
            | "CaptureFrameBlank" => {
                parts.push(match diagnostic.details.as_ref() {
                    Some(details) => {
                        format!("{} Details: {}", diagnostic.message, details)
                    }
                    None => diagnostic.message.clone(),
                });
            }
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
            "name": "doctor",
            "description": "Report Computer Use desktop integration readiness, including environment, semantic, capture, and input backend checks.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "setup_accessibility",
            "description": "Enable toolkit accessibility for AT-SPI-backed semantic app trees, then return a before/after doctor report. Target apps may need restart.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "setup_window_targeting",
            "description": "Install and enable the bundled GNOME Shell window-control extension for exact GNOME window targeting, then report window backend status.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "list_apps",
            "description": "List currently exposed desktop applications from the active platform window and accessibility backends.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "list_windows",
            "description": "List desktop windows from native windowing backends, including backend identity and terminal metadata when available.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "focused_window",
            "description": "Return the focused desktop window reported by native windowing backends, if one is available.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "activate_window",
            "description": "Activate a desktop window by window_id or selector metadata. Supports exact window activation when the matched backend can target windows; otherwise reports unsupported backends honestly.",
            "inputSchema": {
                "type": "object",
                "properties": window_target_schema(),
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
        semantic_element_tool(
            "focus_element",
            "Move semantic focus to an accessibility element from the current snapshot.",
        ),
        semantic_element_tool(
            "activate_element",
            "Perform the element's semantic default action, such as pressing an app-chrome button or opening a menu.",
        ),
        semantic_element_tool(
            "select_element",
            "Select an accessibility element such as a tab, list item, radio item, or selectable row.",
        ),
        semantic_element_tool(
            "expand_element",
            "Expand an accessibility element such as a collapsed menu, combo box, disclosure, or tree item.",
        ),
        semantic_element_tool(
            "collapse_element",
            "Collapse an accessibility element such as an expanded menu, combo box, disclosure, or tree item.",
        ),
        semantic_element_tool(
            "toggle_element",
            "Toggle an accessibility element such as a checkbox, switch, or toggle button.",
        ),
        action_tool(
            "click",
            "Click an element by index from the current snapshot, or explicit x/y screen coordinates without a snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "x": coordinate_schema("X coordinate. With snapshot_id, use screenshot pixel coordinates from that snapshot image; without snapshot_id, use current screen coordinates for the active input backend."),
                "y": coordinate_schema("Y coordinate. With snapshot_id, use screenshot pixel coordinates from that snapshot image; without snapshot_id, use current screen coordinates for the active input backend.")
            }),
            json!([]),
        ),
        action_tool(
            "perform_action",
            "Invoke a specific AT-SPI action by name or index on an element. Prefer named tools such as click, activate_element, select_element, expand_element, collapse_element, and toggle_element for common operations; use this for custom AT-SPI actions exposed in get_app_state.semantic_actions.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "element_identifier": {
                    "type": "string",
                    "description": "Direct AT-SPI backend_ref/object identifier from get_app_state, bypassing element_index lookup."
                },
                "role": { "type": "string" },
                "name": { "type": "string" },
                "text": { "type": "string" },
                "states": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "action_index": {
                    "type": ["integer", "string"],
                    "description": "Zero-based AT-SPI action index. Defaults to 0 when action_name/action are omitted."
                },
                "action_name": {
                    "type": "string",
                    "description": "AT-SPI action name to resolve against the target element's action list."
                },
                "action": {
                    "type": "string",
                    "description": "Compatibility alias: either an action name or numeric action index string."
                }
            }),
            json!([]),
        ),
        action_tool(
            "perform_secondary_action",
            "Perform a secondary click or context action by element index from the current snapshot, or explicit x/y screen coordinates without a snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "x": coordinate_schema("X coordinate. With snapshot_id, use screenshot pixel coordinates from that snapshot image; without snapshot_id, use current screen coordinates for the active input backend."),
                "y": coordinate_schema("Y coordinate. With snapshot_id, use screenshot pixel coordinates from that snapshot image; without snapshot_id, use current screen coordinates for the active input backend."),
                "action": { "type": "string" }
            }),
            json!([]),
        ),
        action_tool(
            "scroll",
            "Scroll within an element from the current snapshot, or the focused area without a snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"]
                },
                "pages": { "type": "integer", "minimum": 1 }
            }),
            json!(["direction"]),
        ),
        action_tool(
            "drag",
            "Drag from one point or element to another; explicit coordinates can run without a snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "x": coordinate_schema("Drag start X coordinate. With snapshot_id, use screenshot pixels; without snapshot_id, use current screen coordinates."),
                "y": coordinate_schema("Drag start Y coordinate. With snapshot_id, use screenshot pixels; without snapshot_id, use current screen coordinates."),
                "from_x": coordinate_schema("Drag start X coordinate. With snapshot_id, use screenshot pixels; without snapshot_id, use current screen coordinates."),
                "from_y": coordinate_schema("Drag start Y coordinate. With snapshot_id, use screenshot pixels; without snapshot_id, use current screen coordinates."),
                "to_x": coordinate_schema("Drag end X coordinate. With snapshot_id, use screenshot pixels; without snapshot_id, use current screen coordinates."),
                "to_y": coordinate_schema("Drag end Y coordinate. With snapshot_id, use screenshot pixels; without snapshot_id, use current screen coordinates."),
                "to_element_index": { "type": "integer", "minimum": 0 }
            }),
            json!([]),
        ),
        action_tool(
            "type_text",
            "Type literal text into the focused control; may use snapshot context or a window target when provided.",
            keyboard_target_properties(json!({
                "text": { "type": "string" }
            })),
            json!(["text"]),
        ),
        action_tool(
            "press_key",
            "Press a keyboard key or key chord in the focused control; may use snapshot context or a window target when provided.",
            keyboard_target_properties(json!({
                "key": { "type": "string" }
            })),
            json!(["key"]),
        ),
        action_tool(
            "set_value",
            "Set an editable element value semantically where supported. Target by element_index, element_identifier, or a semantic selector from the latest get_app_state snapshot.",
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "element_identifier": {
                    "type": "string",
                    "description": "Direct AT-SPI backend_ref/object identifier from get_app_state, bypassing element_index lookup."
                },
                "role": { "type": "string" },
                "name": { "type": "string" },
                "text": { "type": "string" },
                "states": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "value": { "type": "string" }
            }),
            json!(["value"]),
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

fn window_target_schema() -> Value {
    json!({
        "window_id": {
            "type": "string",
            "description": "Exact window_id from list_windows."
        },
        "pid": { "type": "integer", "minimum": 0 },
        "tty": {
            "type": "string",
            "description": "Terminal tty such as /dev/pts/7 or pts/7."
        },
        "terminal_pid": { "type": "integer", "minimum": 0 },
        "terminal_command": { "type": "string" },
        "terminal_cwd": { "type": "string" },
        "app_id": { "type": "string" },
        "wm_class": { "type": "string" },
        "title": { "type": "string" }
    })
}

fn keyboard_target_properties(mut properties: Value) -> Value {
    let Value::Object(properties_map) = &mut properties else {
        return properties;
    };
    if let Value::Object(target_map) = window_target_schema() {
        properties_map.extend(target_map);
    }
    properties
}

fn semantic_element_tool(name: &str, description: &str) -> Value {
    action_tool(
        name,
        description,
        json!({
            "element_index": {
                "type": "integer",
                "minimum": 0,
                "description": "Element index from the current get_app_state snapshot."
            },
            "element_identifier": {
                "type": "string",
                "description": "Direct AT-SPI backend_ref/object identifier from get_app_state, bypassing element_index lookup."
            },
            "role": {
                "type": "string",
                "description": "Optional semantic selector role matched against the latest snapshot."
            },
            "name": {
                "type": "string",
                "description": "Optional semantic selector name matched against the latest snapshot."
            },
            "text": {
                "type": "string",
                "description": "Optional semantic selector text matched against name, description, or value."
            },
            "states": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional semantic selector states; all listed states must match."
            }
        }),
        json!([]),
    )
}

fn action_tool(name: &str, description: &str, mut properties: Value, required: Value) -> Value {
    let Some(property_map) = properties.as_object_mut() else {
        panic!("action_tool called with non-object properties for {name}")
    };
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
        "doctor_report": snapshot.doctor_report,
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
        "bounds": element.bounds,
        "backend_ref": element.backend_ref
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
        MessageFraming, action_summary, compact_element, compact_snapshot,
        invalid_request_tool_error, list_apps_summary, parse_app_state_detail, parse_window_target,
        read_message, setup_accessibility_is_error, setup_window_targeting_is_error,
        snapshot_summary, tool_definitions, tools_list_result, write_message,
    };
    use sky_cua_platform::model::{
        AccessibilitySetupReport, ActionOutcome, AppInfo, AppStateSnapshot, CaptureBackendKind,
        CoordinateSpace, DiagnosticEntry, DoctorCheck, DoctorReadiness, DoctorReport, ElementNode,
        EnvironmentInfo, FocusedApp, InputBackendKind, PortalCapabilities, RectF,
        SemanticBackendKind, SessionKind, SetupCommandReport, ToolAvailability, ToolCapabilities,
        WindowTargetingSetupReport,
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
    fn compact_element_drops_verbose_description_but_keeps_backend_ref() {
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
        assert_eq!(compact["backend_ref"], "opaque-backend-ref");
        assert_eq!(compact["semantic_actions"][0], "click");
    }

    #[test]
    fn compact_snapshot_includes_doctor_report() {
        let report = DoctorReport {
            environment: EnvironmentInfo {
                session_kind: SessionKind::Wayland,
                compositor: None,
                desktop_environment: None,
                capture_backend: CaptureBackendKind::None,
                input_backend: InputBackendKind::None,
                semantic_backend: SemanticBackendKind::None,
                portal_capabilities: PortalCapabilities {
                    screencast_version: None,
                    remote_desktop_version: None,
                    screenshot_version: None,
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: None,
                display: None,
                wayland_display: None,
            },
            checks: vec![DoctorCheck {
                name: "semantic_backend".to_string(),
                ok: true,
                detail: "Atspi".to_string(),
            }],
            readiness: DoctorReadiness {
                can_register_mcp_tools: true,
                can_build_accessibility_tree: true,
                can_capture_screen: true,
                can_send_input: true,
                can_list_windows: false,
                can_target_windows: false,
                recommended_next_step: "Ready".to_string(),
                blockers: Vec::new(),
            },
            platform: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
        };
        let snapshot = AppStateSnapshot {
            snapshot_id: "snap-1".to_string(),
            created_at: Utc::now(),
            environment: report.environment.clone(),
            capabilities: ToolCapabilities {
                list_apps: ToolAvailability {
                    available: true,
                    reason: None,
                },
                get_app_state: ToolAvailability {
                    available: true,
                    reason: None,
                },
                focus_element: ToolAvailability {
                    available: true,
                    reason: None,
                },
                activate_element: ToolAvailability {
                    available: true,
                    reason: None,
                },
                select_element: ToolAvailability {
                    available: true,
                    reason: None,
                },
                expand_element: ToolAvailability {
                    available: true,
                    reason: None,
                },
                collapse_element: ToolAvailability {
                    available: true,
                    reason: None,
                },
                toggle_element: ToolAvailability {
                    available: true,
                    reason: None,
                },
                click: ToolAvailability {
                    available: true,
                    reason: None,
                },
                perform_action: ToolAvailability {
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
            focused_app: None,
            capture: None,
            elements: Vec::new(),
            diagnostics: Vec::new(),
            app_guidance: None,
            doctor_report: Some(report),
        };
        let compact = compact_snapshot(&snapshot);
        assert!(compact.get("doctor_report").is_some());
        assert_eq!(
            compact["doctor_report"]["readiness"]["can_build_accessibility_tree"],
            true
        );
    }

    #[test]
    fn action_tool_schemas_are_strict_and_snapshot_scoped_where_needed() {
        let tools = tool_definitions();
        let click = tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "click")
            .expect("click tool");
        let schema = &click["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!([]));
        assert!(schema["properties"].get("snapshot_id").is_some());
        assert!(schema.get("anyOf").is_none());

        let activate = tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "activate_element")
            .expect("activate_element tool");
        assert_eq!(activate["inputSchema"]["required"], json!([]));
        assert_eq!(activate["inputSchema"]["additionalProperties"], false);
        assert!(
            activate["inputSchema"]["properties"]
                .get("element_identifier")
                .is_some()
        );

        let secondary = tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "perform_secondary_action")
            .expect("perform_secondary_action tool");
        assert_eq!(secondary["inputSchema"]["required"], json!([]));
        assert!(secondary["inputSchema"]["properties"].get("x").is_some());
        assert!(secondary["inputSchema"]["properties"].get("y").is_some());

        let type_text = tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "type_text")
            .expect("type_text tool");
        assert_eq!(type_text["inputSchema"]["additionalProperties"], false);
        assert_eq!(type_text["inputSchema"]["required"], json!(["text"]));
    }

    #[test]
    fn activate_window_parser_rejects_empty_target() {
        let error = parse_window_target(json!({})).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("activate_window requires one of window_id")
        );
    }

    #[test]
    fn activate_window_validation_returns_tool_error() {
        let result =
            invalid_request_tool_error(parse_window_target(json!({})).unwrap_err().to_string())
                .unwrap();

        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["code"], "InvalidRequest");
        assert!(
            result["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("activate_window requires one of window_id")
        );
    }

    #[test]
    fn setup_window_targeting_tool_error_tracks_extension_availability() {
        let report = WindowTargetingSetupReport {
            extension_dir: "/tmp/extension".to_string(),
            wrote_files: false,
            enable_command: SetupCommandReport {
                ok: false,
                detail: "extension file write failed".to_string(),
            },
            windows: Vec::new(),
            windows_error: None,
            requires_shell_reload: false,
            message: "extension available".to_string(),
            permissions_hint: None,
        };

        assert!(!setup_window_targeting_is_error(&report));
    }

    #[test]
    fn setup_accessibility_error_uses_doctor_readiness_contract() {
        let report = AccessibilitySetupReport {
            before: Box::new(doctor_report(false)),
            accessibility_command: SetupCommandReport {
                ok: true,
                detail: "AT-SPI already enabled".to_string(),
            },
            after: Box::new(doctor_report(true)),
            changed: false,
            requires_restart: false,
        };

        assert!(!setup_accessibility_is_error(&report));
    }

    #[test]
    fn setup_accessibility_error_requires_successful_command() {
        let report = AccessibilitySetupReport {
            before: Box::new(doctor_report(false)),
            accessibility_command: SetupCommandReport {
                ok: false,
                detail: "gsettings failed".to_string(),
            },
            after: Box::new(doctor_report(true)),
            changed: false,
            requires_restart: false,
        };

        assert!(setup_accessibility_is_error(&report));
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
                app_user_model_id: None,
                window_handle: None,
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
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: None,
                is_focused_candidate: true,
            },
        ]);

        assert!(summary.contains("Discovered 2 accessible desktop apps."));
        assert!(summary.contains("org.kde.kate (app_id=kwin:{abc}"));
        assert!(summary.contains("desktop_file_id=org.kde.kate.desktop"));
        assert!(summary.contains("window_title=Untitled — Kate"));
        assert!(summary.contains("xfreerdp"));
        assert!(summary.contains("[focused candidate]"));
    }

    #[test]
    fn list_apps_error_diagnostic_detects_failed_list_apps_response() {
        let diagnostics = vec![DiagnosticEntry {
            code: "AccessibilityUnavailable".to_string(),
            message: "AT-SPI is unavailable".to_string(),
            details: None,
        }];

        let diagnostic = super::list_apps_error_diagnostic(&diagnostics)
            .expect("diagnostic from a failed list_apps response should mark it as an MCP error");

        assert_eq!(diagnostic.code, "AccessibilityUnavailable");
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
            capabilities: available_capabilities(),
            focused_app: Some(FocusedApp {
                app_id: "app-1".to_string(),
                name: "zenity".to_string(),
                pid: Some(123),
                desktop_file_id: Some("zenity.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
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
            doctor_report: None,
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
            capabilities: available_capabilities(),
            focused_app: Some(FocusedApp {
                app_id: "app-2".to_string(),
                name: "xmessage".to_string(),
                pid: Some(456),
                desktop_file_id: Some("xmessage.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
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
            doctor_report: None,
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
            capabilities: available_capabilities(),
            focused_app: Some(FocusedApp {
                app_id: "app-restore".to_string(),
                name: "krita".to_string(),
                pid: Some(42),
                desktop_file_id: Some("krita.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
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
            doctor_report: None,
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
            capabilities: available_capabilities(),
            focused_app: Some(FocusedApp {
                app_id: "app-3".to_string(),
                name: "discord".to_string(),
                pid: Some(999),
                desktop_file_id: Some("discord.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
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
            doctor_report: None,
        };

        let summary = snapshot_summary(&snapshot);
        assert!(summary.contains(
            "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback"
        ));
        assert!(summary.contains("image_backend=portal_screenshot"));
    }

    fn available_capabilities() -> ToolCapabilities {
        fn available() -> ToolAvailability {
            ToolAvailability {
                available: true,
                reason: None,
            }
        }

        ToolCapabilities {
            list_apps: available(),
            get_app_state: available(),
            focus_element: available(),
            activate_element: available(),
            select_element: available(),
            expand_element: available(),
            collapse_element: available(),
            toggle_element: available(),
            click: available(),
            perform_action: available(),
            perform_secondary_action: available(),
            scroll: available(),
            drag: available(),
            type_text: available(),
            press_key: available(),
            set_value: available(),
        }
    }

    fn doctor_report(can_build_accessibility_tree: bool) -> DoctorReport {
        DoctorReport {
            environment: EnvironmentInfo {
                session_kind: SessionKind::Wayland,
                compositor: None,
                desktop_environment: None,
                capture_backend: CaptureBackendKind::PortalPipeWire,
                input_backend: InputBackendKind::PortalRemoteDesktop,
                semantic_backend: SemanticBackendKind::Atspi,
                portal_capabilities: PortalCapabilities {
                    screencast_version: Some(5),
                    remote_desktop_version: Some(2),
                    screenshot_version: Some(1),
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: Some("wayland".to_string()),
                display: None,
                wayland_display: Some("wayland-0".to_string()),
            },
            checks: Vec::new(),
            readiness: DoctorReadiness {
                can_register_mcp_tools: true,
                can_build_accessibility_tree,
                can_capture_screen: true,
                can_send_input: true,
                can_list_windows: true,
                can_target_windows: true,
                recommended_next_step: "ready".to_string(),
                blockers: Vec::new(),
            },
            platform: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
        }
    }
}
