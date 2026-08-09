use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AppInfo, AppSelector, AppStateSnapshot,
    CaptureScreenMode, DisplayTarget, DoctorDisplayTopologyReport, DoctorInputReport, DoctorReport,
    DoctorSessionEnvReport, ServiceRequest, ServiceResponse, SessionPresenceAction,
    SessionPresenceIntent, SessionPresenceStatus, WindowInfo, WindowTarget,
};
use std::fmt::Write as _;

use crate::heuristics::HeuristicsRegistry;
use crate::mcp_server::ModelSessionInfo;
use crate::output_shapes::{
    informational_runtime_summary, list_apps_error_diagnostic, portal_approval_summary,
    setup_accessibility_is_error, setup_window_targeting_is_error, summary_snapshot,
    summary_snapshot_text_content,
};
use crate::service_launcher::ServiceClient;

mod annotations;
mod app_state;
mod browser;
mod definitions;
mod phone;
mod semantic_text;

#[cfg(test)]
use app_state::parse_app_state_detail;
#[cfg(test)]
pub(crate) use definitions::tools_list_result;
pub(crate) use definitions::{
    InactiveToolReason, McpProcessConfig, McpToolRegistry, build_tool_registry,
    mcp_process_config_from_env,
};
#[cfg(test)]
pub(crate) use definitions::{
    build_tool_definitions, tool_definitions, validation_tool_definitions,
};
#[cfg(test)]
mod browser_tests;
#[cfg(test)]
mod phone_tests;

pub(crate) trait McpService {
    fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse>;
    /// Whether the backing client drives the private isolated desktop. Tools
    /// scoped to the isolated desktop (e.g. `desktop_launch_app`) gate on this.
    /// Defaults to `false` so non-isolated harnesses need not override it.
    fn is_isolated(&self) -> bool {
        false
    }
}

impl McpService for ServiceClient {
    fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse> {
        ServiceClient::call(self, request)
    }

    fn is_isolated(&self) -> bool {
        ServiceClient::is_isolated(self)
    }
}

#[cfg(test)]
pub(crate) fn handle_session_tool_call(
    service: &impl McpService,
    heuristics: &HeuristicsRegistry,
    model: &ModelSessionInfo,
    registry: &McpToolRegistry,
    tool_name: &str,
    arguments: Value,
) -> Result<Value> {
    handle_session_tool_call_with_browser_identity(
        service, heuristics, model, registry, tool_name, arguments, None,
    )
}

pub(crate) fn handle_session_tool_call_with_browser_identity(
    service: &impl McpService,
    heuristics: &HeuristicsRegistry,
    model: &ModelSessionInfo,
    registry: &McpToolRegistry,
    tool_name: &str,
    arguments: Value,
    browser_identity: Option<&sky_cua_platform::BrowserSessionIdentity>,
) -> Result<Value> {
    if !registry.contains(tool_name) {
        return match registry.inactive_reason(tool_name) {
            Some(InactiveToolReason::BrowserEvalDisabled) => tool_error(
                "FeatureDisabled",
                "browser_eval is disabled for this MCP process",
            ),
            None => tool_error("UnknownTool", format!("unknown tool: {tool_name}")),
        };
    }
    handle_grouped_tool_call(
        service,
        heuristics,
        model,
        registry,
        tool_name,
        arguments,
        browser_identity,
    )
}

fn handle_grouped_tool_call(
    service: &impl McpService,
    heuristics: &HeuristicsRegistry,
    model: &ModelSessionInfo,
    registry: &McpToolRegistry,
    tool_name: &str,
    arguments: Value,
    browser_identity: Option<&sky_cua_platform::BrowserSessionIdentity>,
) -> Result<Value> {
    if let Err(message) = registry.validate_arguments(tool_name, &arguments) {
        return Ok(grouped_invalid_request_result(tool_name, message));
    }
    let call = match grouped_handler_call(tool_name, arguments) {
        Ok(call) => call,
        Err(error) => {
            return Ok(grouped_invalid_request_result(tool_name, error.to_string()));
        }
    };
    let handler_result = handle_tool_call_with_browser_eval_policy(
        service,
        heuristics,
        model,
        call.handler_name,
        call.arguments.clone(),
        Some(registry.browser_eval_enabled),
        browser_identity,
    )?;
    Ok(grouped_tool_result(tool_name, &call, handler_result))
}

#[derive(Debug, Clone, PartialEq)]
struct GroupedHandlerCall {
    handler_name: &'static str,
    branch: String,
    arguments: Value,
}

fn grouped_handler_call(tool_name: &str, arguments: Value) -> Result<GroupedHandlerCall> {
    let mut arguments = grouped_arguments_object(arguments)?;
    let (handler_name, branch) = match tool_name {
        "doctor" => ("doctor", "diagnostics".to_string()),
        "status" => match take_required_branch(&mut arguments, "component")?.as_str() {
            "browser" => ("browser_status", "browser".to_string()),
            "phone" => ("phone_status", "phone".to_string()),
            "phone_companion" => ("phone_companion_status", "phone_companion".to_string()),
            "session_presence" => ("session_presence_status", "session_presence".to_string()),
            component => return Err(anyhow!("unsupported status component: {component}")),
        },
        "list_resources" => {
            let surface = take_required_branch(&mut arguments, "surface")?;
            let resource = take_required_branch(&mut arguments, "resource")?;
            let branch = format!("{surface}/{resource}");
            match (surface.as_str(), resource.as_str()) {
                ("desktop", "apps") => ("list_apps", branch),
                ("desktop", "windows") => ("list_windows", branch),
                ("desktop", "focused_window") => ("focused_window", branch),
                ("browser", "tabs") => ("browser_list_tabs", branch),
                ("phone", "devices") => ("phone_list_devices", branch),
                ("phone", "apps") => ("phone_app_list", branch),
                ("phone", "current_app") => ("phone_app_current", branch),
                _ => {
                    return Err(anyhow!(
                        "unsupported list_resources pair: {surface}/{resource}"
                    ));
                }
            }
        }
        "observe" => match take_required_branch(&mut arguments, "surface")?.as_str() {
            "desktop" => ("desktop_observe_appshot", "desktop".to_string()),
            "browser" => ("browser_appshot", "browser".to_string()),
            "phone" => ("phone_observe", "phone".to_string()),
            surface => return Err(anyhow!("unsupported observe surface: {surface}")),
        },
        "capture_screen" => match take_required_branch(&mut arguments, "surface")?.as_str() {
            "browser" => ("browser_screenshot", "browser".to_string()),
            "phone" => ("phone_screenshot", "phone".to_string()),
            surface => return Err(anyhow!("unsupported capture_screen surface: {surface}")),
        },
        "capture_desktop" => ("screenshot", "default".to_string()),
        "setup_desktop" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "accessibility" => ("setup_accessibility", "accessibility".to_string()),
            "window_targeting" => ("setup_window_targeting", "window_targeting".to_string()),
            operation => return Err(anyhow!("unsupported setup_desktop operation: {operation}")),
        },
        "session_presence" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "hold" => ("hold_session", "hold".to_string()),
            "unlock" => ("unlock_session", "unlock".to_string()),
            "release" => ("release_session", "release".to_string()),
            operation => {
                return Err(anyhow!(
                    "unsupported session_presence operation: {operation}"
                ));
            }
        },
        "activate_window" => ("activate_window", "default".to_string()),
        "desktop_semantic" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "focus" => ("focus_element", "focus".to_string()),
            "select" => ("select_element", "select".to_string()),
            "expand" => ("expand_element", "expand".to_string()),
            "collapse" => ("collapse_element", "collapse".to_string()),
            operation => {
                return Err(anyhow!(
                    "unsupported desktop_semantic operation: {operation}"
                ));
            }
        },
        "desktop_toggle" => ("toggle_element", "default".to_string()),
        "desktop_scroll" => ("scroll", "default".to_string()),
        "desktop_pointer" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "click" => ("click", "click".to_string()),
            "secondary_click" => ("perform_secondary_action", "secondary_click".to_string()),
            "drag" => ("drag", "drag".to_string()),
            operation => {
                return Err(anyhow!(
                    "unsupported desktop_pointer operation: {operation}"
                ));
            }
        },
        "desktop_keyboard" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "type_text" => ("type_text", "type_text".to_string()),
            "press_key" => ("press_key", "press_key".to_string()),
            operation => {
                return Err(anyhow!(
                    "unsupported desktop_keyboard operation: {operation}"
                ));
            }
        },
        "desktop_action" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "activate" => ("activate_element", "activate".to_string()),
            "perform_action" => ("perform_action", "perform_action".to_string()),
            operation => return Err(anyhow!("unsupported desktop_action operation: {operation}")),
        },
        "desktop_launch_app" => ("desktop_launch_app", "default".to_string()),
        "desktop_set_value" => ("set_value", "default".to_string()),
        "browser_open" => ("browser_open", "default".to_string()),
        "browser_claim_tab" => ("browser_claim_tab", "default".to_string()),
        "browser_move_mouse" => ("browser_move_mouse", "default".to_string()),
        "browser_navigate" => ("browser_navigate", "default".to_string()),
        "browser_scroll" => ("browser_scroll", "default".to_string()),
        "browser_eval" => ("browser_eval", "default".to_string()),
        "browser_input" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "click" => ("browser_click", "click".to_string()),
            "type_text" => ("browser_type_text", "type_text".to_string()),
            "press_key" => ("browser_press_key", "press_key".to_string()),
            operation => return Err(anyhow!("unsupported browser_input operation: {operation}")),
        },
        "phone_connection" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "connect" => ("phone_connect", "connect".to_string()),
            "disconnect" => ("phone_disconnect", "disconnect".to_string()),
            "refresh" => ("phone_refresh_capabilities", "refresh".to_string()),
            operation => {
                return Err(anyhow!(
                    "unsupported phone_connection operation: {operation}"
                ));
            }
        },
        "phone_pair_wireless" => ("phone_pair_wireless", "default".to_string()),
        "phone_setup" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "create_enrollment" => (
                "phone_direct_create_enrollment",
                "create_enrollment".to_string(),
            ),
            "install_companion" => ("phone_install_companion", "install_companion".to_string()),
            "open_settings" => ("phone_open_settings", "open_settings".to_string()),
            operation => return Err(anyhow!("unsupported phone_setup operation: {operation}")),
        },
        "phone_app_force_stop" => ("phone_app_force_stop", "default".to_string()),
        "phone_pointer" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "tap" => ("phone_tap", "tap".to_string()),
            "swipe" => ("phone_swipe", "swipe".to_string()),
            operation => return Err(anyhow!("unsupported phone_pointer operation: {operation}")),
        },
        "phone_keyboard" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "type_text" => ("phone_type_text", "type_text".to_string()),
            "press_key" => ("phone_press_key", "press_key".to_string()),
            operation => return Err(anyhow!("unsupported phone_keyboard operation: {operation}")),
        },
        "phone_notification_action" => {
            match take_required_branch(&mut arguments, "operation")?.as_str() {
                "open" => ("phone_notification_open", "open".to_string()),
                "dismiss" => ("phone_notification_dismiss", "dismiss".to_string()),
                "action" => ("phone_notification_action", "action".to_string()),
                operation => {
                    return Err(anyhow!(
                        "unsupported phone_notification_action operation: {operation}"
                    ));
                }
            }
        }
        "phone_notification_reply" => ("phone_notification_reply", "default".to_string()),
        "phone_app_action" => match take_required_branch(&mut arguments, "operation")?.as_str() {
            "launch" => ("phone_app_launch", "launch".to_string()),
            "open_intent" => ("phone_app_open_intent", "open_intent".to_string()),
            operation => {
                return Err(anyhow!(
                    "unsupported phone_app_action operation: {operation}"
                ));
            }
        },
        "phone_app_install" => ("phone_app_install", "default".to_string()),
        "phone_content" => preserve_operation(&mut arguments, "phone_content")?,
        "phone_clipboard" => preserve_operation(&mut arguments, "phone_clipboard")?,
        "phone_editor" => preserve_operation(&mut arguments, "phone_editor")?,
        "phone_camera" => preserve_operation(&mut arguments, "phone_camera")?,
        "phone_storage" => preserve_operation(&mut arguments, "phone_storage")?,
        "phone_accessibility_tree" => ("phone_accessibility_tree", "default".to_string()),
        "phone_notifications" => ("phone_notifications", "default".to_string()),
        name => return Err(anyhow!("unknown tool: {name}")),
    };
    Ok(GroupedHandlerCall {
        handler_name,
        branch,
        arguments: Value::Object(arguments),
    })
}

fn preserve_operation(
    arguments: &mut serde_json::Map<String, Value>,
    handler: &'static str,
) -> Result<(&'static str, String)> {
    let operation = take_required_branch(arguments, "operation")?;
    arguments.insert("operation".into(), Value::String(operation.clone()));
    Ok((handler, operation))
}

fn handle_phone_direct_create_enrollment(service: &impl McpService) -> Result<Value> {
    let payload = match service.call(&ServiceRequest::PhoneDirectCreateEnrollment)? {
        ServiceResponse::PhoneDirectEnrollment { payload } => *payload,
        ServiceResponse::Error { code, message, .. } => return tool_error(code, message),
        other => {
            return Err(anyhow!(
                "unexpected response for Companion Direct enrollment: {other:?}"
            ));
        }
    };
    let mut uri = url::Url::parse("skycua://enroll")?;
    uri.query_pairs_mut()
        .append_pair("protocol", &payload.protocol)
        .append_pair("endpoint", &payload.endpoint)
        .append_pair("enrollment_id", &payload.enrollment_id)
        .append_pair("bootstrap_credential", &payload.bootstrap_credential)
        .append_pair("expires_at_ms", &payload.expires_at_ms.to_string());
    let enrollment_uri = uri.to_string();
    let manual_code = format!(
        "{}\n{}\n{}\n{}",
        payload.endpoint,
        payload.enrollment_id,
        payload.bootstrap_credential,
        payload.expires_at_ms
    );
    let code = qrcode::QrCode::new(enrollment_uri.as_bytes())?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(512, 512)
        .build();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image).write_to(&mut png, image::ImageFormat::Png)?;
    use base64::Engine as _;
    let png_base64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": format!(
                    "Scan this single-use Companion Direct enrollment QR for {}. It expires at {} ms since Unix epoch.",
                    payload.endpoint, payload.expires_at_ms
                )
            },
            {"type": "image", "data": png_base64, "mimeType": "image/png"}
        ],
        "structuredContent": {
            "protocol": payload.protocol,
            "endpoint": payload.endpoint,
            "enrollment_id": payload.enrollment_id,
            "enrollment_uri": enrollment_uri,
            "manual_code": manual_code,
            "expires_at_ms": payload.expires_at_ms
        },
        "isError": false
    }))
}

fn grouped_tool_result(tool_name: &str, call: &GroupedHandlerCall, handler_result: Value) -> Value {
    let content = handler_result
        .get("content")
        .and_then(Value::as_array)
        .map(|content| grouped_tool_content(tool_name, call, content))
        .unwrap_or_else(|| {
            vec![json!({
                "type": "text",
                "text": format!(
                    "Grouped {tool_name}/{} completed.",
                    call.branch
                )
            })]
        });
    json!({
        "content": content,
        "structuredContent": {
            "tool": tool_name,
            "branch": call.branch,
            "result": handler_result.get("structuredContent").cloned().unwrap_or(Value::Null)
        },
        "isError": handler_result.get("isError").and_then(Value::as_bool).unwrap_or(false)
    })
}

fn grouped_tool_content(
    tool_name: &str,
    call: &GroupedHandlerCall,
    handler_content: &[Value],
) -> Vec<Value> {
    handler_content
        .iter()
        .enumerate()
        .map(|(index, item)| {
            if index == 0 && item.get("type").and_then(Value::as_str) == Some("text") {
                let handler_text = item.get("text").and_then(Value::as_str).unwrap_or("");
                json!({
                    "type": "text",
                    "text": format!("{tool_name}/{}. {handler_text}", call.branch)
                })
            } else {
                item.clone()
            }
        })
        .collect()
}

fn grouped_invalid_request_result(tool_name: &str, message: String) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": format!("Invalid {tool_name} request: {message}")
        }],
        "structuredContent": {
            "tool": tool_name,
            "branch": Value::Null,
            "error": {
                "code": "InvalidRequest",
                "message": message
            }
        },
        "isError": true
    })
}

fn grouped_arguments_object(arguments: Value) -> Result<serde_json::Map<String, Value>> {
    match arguments {
        Value::Object(map) => Ok(map),
        _ => Err(anyhow!("grouped tool arguments must be an object")),
    }
}

fn take_required_branch(
    arguments: &mut serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<String> {
    let value = arguments
        .remove(field)
        .ok_or_else(|| anyhow!("missing required {field}"))?;
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("{field} must be a string"))
}

#[cfg(test)]
fn handle_tool_call(
    service: &impl McpService,
    heuristics: &HeuristicsRegistry,
    model: &ModelSessionInfo,
    tool_name: &str,
    arguments: Value,
) -> Result<Value> {
    handle_tool_call_with_browser_eval_policy(
        service, heuristics, model, tool_name, arguments, None, None,
    )
}

fn handle_tool_call_with_browser_eval_policy(
    service: &impl McpService,
    heuristics: &HeuristicsRegistry,
    model: &ModelSessionInfo,
    tool_name: &str,
    arguments: Value,
    browser_eval_enabled: Option<bool>,
    browser_identity: Option<&sky_cua_platform::BrowserSessionIdentity>,
) -> Result<Value> {
    match tool_name {
        "doctor" => match service.call(&ServiceRequest::Doctor)? {
            ServiceResponse::Doctor { report } => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": doctor_summary(&report)
                }],
                "structuredContent": report,
                "isError": false
            })),
            ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            ServiceResponse::Error { code, message, .. } => tool_error(code, message),
            other => Err(anyhow!(
                "unexpected response for setup_window_targeting: {other:?}"
            )),
        },
        "desktop_launch_app" => {
            // Scoped to the isolated desktop: this tool launches applications
            // into the agent's PRIVATE desktop only. Launching apps onto the
            // user's live session is intentionally out of scope, so refuse
            // before any spawn when the client is not isolated.
            if !service.is_isolated() {
                return tool_error(
                    "IsolatedDesktopRequired",
                    "desktop_launch_app launches applications only into the agent's private \
                     isolated desktop; launching apps onto the user's live session is \
                     intentionally out of scope. Enable isolated mode (set SKY_CUA_ISOLATED_DESKTOP=1 \
                     or [isolated_desktop] enabled = true in sky-cua.toml) and start a new session.",
                );
            }
            let command = match arguments.get("command").and_then(Value::as_str) {
                Some(command) if !command.trim().is_empty() => command.to_string(),
                _ => {
                    return tool_error(
                        "InvalidRequest",
                        "desktop_launch_app requires a non-empty 'command'",
                    );
                }
            };
            let args = match arguments.get("args") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => {
                    let mut collected = Vec::with_capacity(items.len());
                    for item in items {
                        match item.as_str() {
                            Some(arg) => collected.push(arg.to_string()),
                            None => {
                                return tool_error(
                                    "InvalidRequest",
                                    "desktop_launch_app 'args' must be an array of strings",
                                );
                            }
                        }
                    }
                    collected
                }
                Some(_) => {
                    return tool_error(
                        "InvalidRequest",
                        "desktop_launch_app 'args' must be an array of strings",
                    );
                }
            };
            match service.call(&ServiceRequest::LaunchApplication {
                command: command.clone(),
                args,
            })? {
                ServiceResponse::LaunchApplication {
                    pid,
                    destination_appshot,
                    diagnostics,
                } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Launched '{command}' in the isolated desktop (pid {pid}).")
                    }],
                    "structuredContent": { "pid": pid, "destination_appshot": destination_appshot, "diagnostics": diagnostics },
                    "isError": false
                })),
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for desktop_launch_app: {other:?}"
                )),
            }
        }
        "list_apps" => match service.call(&ServiceRequest::ListApps)? {
            ServiceResponse::ListApps {
                environment,
                mut apps,
                diagnostics,
            } => {
                let limit = parse_optional_usize(&arguments, "limit", "list_resources")?;
                let total = apps.len();
                if let Some(limit) = limit {
                    apps.truncate(limit);
                }
                let runtime_error = list_apps_error_diagnostic(&diagnostics);
                let is_error = runtime_error.is_some();
                let mut summary = runtime_error
                    .map(|diagnostic| diagnostic.message.clone())
                    .unwrap_or_else(|| list_apps_summary(&apps));
                if !is_error && apps.len() < total {
                    summary.push_str(&format!(" Showing first {} of {total} apps.", apps.len()));
                }
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
            ServiceResponse::Error { code, message, .. } => tool_error(code, message),
            other => Err(anyhow!("unexpected response for list_apps: {other:?}")),
        },
        "list_windows" => match service.call(&ServiceRequest::ListWindows)? {
            ServiceResponse::ListWindows {
                environment,
                mut windows,
                diagnostics,
            } => {
                let limit = parse_optional_usize(&arguments, "limit", "list_resources")?;
                let total = windows.len();
                if let Some(limit) = limit {
                    windows.truncate(limit);
                }
                let runtime_error = diagnostics.first();
                let is_error = runtime_error.is_some();
                let mut summary = runtime_error
                    .map(|diagnostic| diagnostic.message.clone())
                    .unwrap_or_else(|| list_windows_summary(&windows));
                if !is_error && windows.len() < total {
                    summary.push_str(&format!(
                        " Showing first {} of {total} windows.",
                        windows.len()
                    ));
                }
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
            ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            ServiceResponse::Error { code, message, .. } => tool_error(code, message),
            other => Err(anyhow!("unexpected response for focused_window: {other:?}")),
        },
        "activate_window" => {
            let target = match parse_window_target(arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&ServiceRequest::ActivateWindow {
                target,
                context: None,
            })? {
                ServiceResponse::ActivateWindow {
                    outcome,
                    destination_appshot,
                } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": action_summary(&outcome)
                    }],
                    "structuredContent": { "outcome": outcome, "destination_appshot": destination_appshot },
                    "isError": !outcome.success
                })),
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for activate_window: {other:?}"
                )),
            }
        }
        "screenshot" => {
            let screenshot_target = match parse_screenshot_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let screenshot_delivery = parse_screenshot_delivery(&arguments);
            match service.call(&ServiceRequest::Screenshot {
                target: screenshot_target.window,
                display_target: screenshot_target.display,
            })? {
                ServiceResponse::Screenshot { mut snapshot } => {
                    enrich_snapshot(heuristics, &mut snapshot);
                    let structured_content = summary_snapshot(&snapshot);
                    let mut text_content = summary_snapshot_text_content(&snapshot);

                    let mut content = Vec::with_capacity(2);
                    if screenshot_delivery == ScreenshotDelivery::Inline
                        && model.can_receive_images()
                    {
                        match inline_screenshot_block(&snapshot) {
                            Some(Ok(image_block)) => content.push(image_block),
                            Some(Err(message)) => {
                                text_content.push_str(
                                    "\nInline screenshot delivery failed; read capture.inspection_image_path instead: ",
                                );
                                text_content.push_str(&message);
                            }
                            None => {}
                        }
                    }
                    content.insert(0, json!({"type": "text", "text": text_content}));

                    Ok(json!({
                        "content": content,
                        "structuredContent": structured_content,
                        "isError": false
                    }))
                }
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for screenshot: {other:?}")),
            }
        }
        "get_app_state" => app_state::handle_get_app_state(service, heuristics, arguments, model),
        "desktop_observe_appshot" => {
            app_state::handle_desktop_observe_appshot(service, arguments, model)
        }
        "phone_direct_create_enrollment" => handle_phone_direct_create_enrollment(service),
        name if browser::is_browser_tool(name) => browser::handle_tool_call(
            service,
            name,
            arguments,
            model,
            browser_eval_enabled,
            browser_identity,
        ),
        name if phone::is_phone_tool(name) => {
            phone::handle_tool_call(service, name, arguments, model)
        }
        "hold_session" | "unlock_session" | "release_session" | "session_presence_status" => {
            handle_session_presence_call(service, tool_name, arguments)
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

fn handle_session_presence_call(
    service: &impl McpService,
    tool_name: &str,
    arguments: Value,
) -> Result<Value> {
    let action = match session_presence_action_from_tool(tool_name, &arguments) {
        Ok(action) => action,
        Err(error) => return invalid_request_tool_error(error.to_string()),
    };

    match service.call(&ServiceRequest::SessionPresence { action })? {
        ServiceResponse::SessionPresence { status } => Ok(json!({
            "content": [{
                "type": "text",
                "text": session_presence_summary(&status)
            }],
            "structuredContent": status,
            "isError": false
        })),
        ServiceResponse::Error { code, message, .. } => tool_error(code, message),
        other => Err(anyhow!(
            "unexpected response for session presence call: {other:?}"
        )),
    }
}

fn session_presence_action_from_tool(
    tool_name: &str,
    arguments: &Value,
) -> Result<SessionPresenceAction> {
    let action = match tool_name {
        "hold_session" => SessionPresenceAction::Ensure(SessionPresenceIntent {
            unlock: parse_optional_bool(arguments, "unlock", false)?,
            inhibit_lock: parse_optional_bool(arguments, "inhibit_lock", true)?,
            inhibit_suspend: parse_optional_bool(arguments, "inhibit_suspend", true)?,
        }),
        "unlock_session" => SessionPresenceAction::Ensure(SessionPresenceIntent {
            unlock: true,
            inhibit_lock: parse_optional_bool(arguments, "inhibit_lock", true)?,
            inhibit_suspend: parse_optional_bool(arguments, "inhibit_suspend", true)?,
        }),
        "release_session" => SessionPresenceAction::Release {
            relock: parse_optional_bool(arguments, "relock", false)?,
        },
        "session_presence_status" => SessionPresenceAction::Status,
        _ => unreachable!("session presence tool name was pre-filtered"),
    };
    Ok(action)
}

fn handle_action_call(
    service: &impl McpService,
    action: ActionName,
    mut arguments: Value,
) -> Result<Value> {
    let appshot_id = arguments
        .get("appshot_id")
        .and_then(Value::as_str)
        .and_then(optional_non_empty_string)
        .ok_or_else(|| anyhow!("appshot_id is required for desktop state-changing actions"))?;
    let snapshot_id = arguments
        .get("snapshot_id")
        .and_then(Value::as_str)
        .and_then(optional_non_empty_string);
    normalize_action_coordinate_targets(&action, snapshot_id.is_some(), &mut arguments);
    normalize_action_selector_targets(&action, snapshot_id.is_some(), &mut arguments);
    let element_index = element_index_from_arguments(&arguments)
        .filter(|index| snapshot_id.is_some() || *index != 0);
    let request = ActionRequest {
        action,
        appshot_id: Some(appshot_id.to_string()),
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
        ServiceResponse::AppShotRequired { rejection } => Ok(json!({
            "content": [{
                "type": "text",
                "text": rejection.message
            }],
            "structuredContent": rejection,
            "isError": true
        })),
        ServiceResponse::Error { code, message, .. } => tool_error(code, message),
        other => Err(anyhow!("unexpected response for action call: {other:?}")),
    }
}

fn parse_optional_bool(arguments: &Value, key: &str, default: bool) -> Result<bool> {
    match arguments.get(key) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(anyhow!("{key} must be a boolean")),
        None => Ok(default),
    }
}

fn session_presence_summary(status: &SessionPresenceStatus) -> String {
    let locked = status
        .locked
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "Session presence backend={} supported={} unlock_supported={} locked={} lock_inhibited={} suspend_inhibited={}. {}",
        status.backend,
        status.supported,
        status.unlock_supported,
        locked,
        status.lock_inhibited,
        status.suspend_inhibited,
        status.detail
    )
}

fn normalize_action_coordinate_targets(
    action: &ActionName,
    has_snapshot: bool,
    arguments: &mut Value,
) {
    let Some(arguments) = arguments.as_object_mut() else {
        return;
    };

    match action {
        ActionName::Click | ActionName::PerformSecondaryAction
            if has_point(arguments, "x", "y") =>
        {
            normalize_point_target(arguments, has_snapshot, "element_index", "x", "y");
        }
        ActionName::Drag => {
            if has_point(arguments, "from_x", "from_y") {
                normalize_point_target(
                    arguments,
                    has_snapshot,
                    "element_index",
                    "from_x",
                    "from_y",
                );
            }
            if !has_point(arguments, "from_x", "from_y") && has_point(arguments, "x", "y") {
                normalize_point_target(arguments, has_snapshot, "element_index", "x", "y");
            }
            if has_point(arguments, "to_x", "to_y") {
                normalize_point_target(arguments, has_snapshot, "to_element_index", "to_x", "to_y");
            }
        }
        _ => {}
    }
}

fn normalize_point_target(
    arguments: &mut serde_json::Map<String, Value>,
    has_snapshot: bool,
    index_field: &str,
    x: &str,
    y: &str,
) {
    if has_snapshot && arguments.contains_key(index_field) && point_is_host_default(arguments, x, y)
    {
        arguments.remove(x);
        arguments.remove(y);
    } else {
        remove_host_default_index(arguments, index_field);
    }
}

fn remove_host_default_index(arguments: &mut serde_json::Map<String, Value>, field: &str) {
    if arguments.get(field).and_then(Value::as_u64) == Some(0) {
        arguments.remove(field);
    }
}

fn point_is_host_default(arguments: &serde_json::Map<String, Value>, x: &str, y: &str) -> bool {
    arguments.get(x).and_then(Value::as_f64) == Some(0.0)
        && arguments.get(y).and_then(Value::as_f64) == Some(0.0)
}

fn has_point(arguments: &serde_json::Map<String, Value>, x: &str, y: &str) -> bool {
    arguments.get(x).and_then(Value::as_f64).is_some()
        && arguments.get(y).and_then(Value::as_f64).is_some()
}

fn element_index_from_arguments(arguments: &Value) -> Option<usize> {
    arguments
        .get("element_index")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn normalize_action_selector_targets(
    action: &ActionName,
    has_snapshot: bool,
    arguments: &mut Value,
) {
    let Some(arguments) = arguments.as_object_mut() else {
        return;
    };

    if has_snapshot
        && arguments.get("element_index").and_then(Value::as_u64) == Some(0)
        && has_semantic_selector(action, arguments)
    {
        arguments.remove("element_index");
    }
}

fn has_semantic_selector(action: &ActionName, arguments: &serde_json::Map<String, Value>) -> bool {
    has_non_empty_string(arguments, "role")
        || has_non_empty_string(arguments, "name")
        || (action != &ActionName::TypeText && has_non_empty_string(arguments, "text"))
        || arguments
            .get("states")
            .and_then(Value::as_array)
            .is_some_and(|states| {
                states.iter().any(|state| {
                    state
                        .as_str()
                        .map(str::trim)
                        .is_some_and(|state| !state.is_empty())
                })
            })
}

fn has_non_empty_string(arguments: &serde_json::Map<String, Value>, field: &str) -> bool {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

pub(crate) fn enrich_snapshot(heuristics: &HeuristicsRegistry, snapshot: &mut AppStateSnapshot) {
    if snapshot.app_guidance.is_none()
        && let Some(focused_app) = snapshot.focused_app.as_ref()
    {
        snapshot.app_guidance = heuristics.resolve_for_focused_app(focused_app);
    }
}

pub(crate) fn list_apps_summary(apps: &[AppInfo]) -> String {
    let app_count = apps.len();
    if apps.is_empty() {
        return format!("Discovered {app_count} accessible desktop apps.");
    }

    let mut summary = String::with_capacity(apps.len() * 64);
    let _ = write!(
        &mut summary,
        "Discovered {app_count} accessible desktop apps. Apps: "
    );
    for (i, app) in apps.iter().enumerate() {
        if i > 0 {
            summary.push_str("; ");
        }
        let _ = write!(&mut summary, "{} (app_id={}", app.name, app.app_id);
        if let Some(desktop_file_id) = app
            .desktop_file_id
            .as_deref()
            .filter(|desktop_file_id| !desktop_file_id.is_empty())
        {
            let _ = write!(&mut summary, ", desktop_file_id={}", desktop_file_id);
        }
        summary.push(')');
        if let Some(window_title) = app
            .window_title
            .as_deref()
            .filter(|title| !title.is_empty())
        {
            let _ = write!(&mut summary, ", window_title={}", window_title);
        }
        if app.is_focused_candidate {
            summary.push_str(" [focused candidate]");
        }
    }
    summary.push('.');
    summary
}

fn list_windows_summary(windows: &[WindowInfo]) -> String {
    if windows.is_empty() {
        return "Discovered 0 desktop windows.".to_string();
    }

    let mut summary = String::with_capacity(windows.len() * 64);
    let _ = write!(
        &mut summary,
        "Discovered {} desktop windows. Windows: ",
        windows.len()
    );
    for (i, window) in windows.iter().enumerate() {
        if i > 0 {
            summary.push_str("; ");
        }
        let label = window
            .title
            .as_deref()
            .or(window.app_id.as_deref())
            .unwrap_or(&window.window_id);
        let _ = write!(
            &mut summary,
            "{} (window_id={}, backend={}",
            label, window.window_id, window.backend
        );
        if let Some(app_id) = window.app_id.as_deref().filter(|value| !value.is_empty()) {
            let _ = write!(&mut summary, ", app_id={}", app_id);
        }
        if let Some(pid) = window.pid {
            let _ = write!(&mut summary, ", pid={}", pid);
        }
        if let Some(terminal) = &window.terminal {
            let _ = write!(&mut summary, ", tty={}", terminal.tty);
            if let Some(active) = &terminal.active_process {
                let _ = write!(&mut summary, ", active_process={}", active.command_name);
            }
        }
        if window.focused {
            summary.push_str(" [focused]");
        }
        summary.push(')');
    }
    summary.push('.');
    summary
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

pub(crate) fn parse_window_target(arguments: Value) -> Result<WindowTarget> {
    WindowTarget::from_argument_fields(&arguments)
        .context("invalid activate_window target arguments")?
        .ok_or_else(|| {
            anyhow!(
            "activate_window requires one of window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd"
            )
        })
}

fn parse_optional_window_target(arguments: &Value) -> Result<Option<WindowTarget>> {
    WindowTarget::from_argument_fields(arguments)
        .context("invalid screenshot window target arguments")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScreenshotTarget {
    window: Option<WindowTarget>,
    display: Option<DisplayTarget>,
}

fn parse_screenshot_target(arguments: &Value) -> Result<ScreenshotTarget> {
    let window = parse_optional_window_target(arguments)?;
    let display = DisplayTarget::from_argument_fields(arguments)
        .context("invalid screenshot display target arguments")?;

    if window.is_some() && display.is_some() {
        return Err(anyhow!(
            "screenshot accepts exactly one capture selector: window target fields or display_id/display_name/display_index"
        ));
    }

    Ok(ScreenshotTarget { window, display })
}

pub(crate) fn action_summary(outcome: &ActionOutcome) -> String {
    if outcome.code == "PortalApprovalPending" {
        return portal_approval_summary(&outcome.message);
    }
    if let Some(summary_suffix) = informational_runtime_summary(&outcome.diagnostics) {
        let mut summary = String::with_capacity(outcome.message.len() + summary_suffix.len() + 1);
        summary.push_str(&outcome.message);
        summary.push(' ');
        summary.push_str(&summary_suffix);
        summary
    } else {
        outcome.message.clone()
    }
}

fn tool_error(code: impl Into<String>, message: impl Into<String>) -> Result<Value> {
    let code = code.into();
    let message = message.into();
    let text = if code == "PortalApprovalPending" {
        portal_approval_summary(&message)
    } else {
        message.clone()
    };
    let mut structured_content = json!({
        "code": code,
        "message": message
    });
    if code == "CaptureSourceGeometryMissing" {
        structured_content["suggestion"] = json!(
            "Refresh the window/display state (re-observe) and retry the same targeted capture once; it returns a snapshot_id for pixel actions. Captures are single-screen, so there is no broader capture to fall back to."
        );
    }
    Ok(json!({
        "content": [{
            "type": "text",
            "text": text
        }],
        "structuredContent": structured_content,
        "isError": true
    }))
}

pub(crate) fn invalid_request_tool_error(message: impl Into<String>) -> Result<Value> {
    tool_error("InvalidRequest", message)
}

fn doctor_summary(report: &DoctorReport) -> String {
    let mut summary = report.readiness.recommended_next_step.clone();
    if report
        .session_env
        .as_ref()
        .is_some_and(DoctorSessionEnvReport::changed)
    {
        summary.push_str(" SessionEnvRepaired: detached desktop session environment was repaired.");
    }

    if let Some(input) = &report.input {
        push_input_diagnostics(input, &mut summary);
    }

    if let Some(display_topology) = &report.display_topology {
        push_display_topology_summary(report, display_topology, &mut summary);
    }

    summary
}

fn push_display_topology_summary(
    report: &DoctorReport,
    display_topology: &DoctorDisplayTopologyReport,
    summary: &mut String,
) {
    if display_topology.display_count == 0 {
        summary.push_str(" DisplayTopologyUnavailable: display-targeted screenshots cannot be authoritative until a display provider reports geometry; refresh desktop state, then retry the targeted screenshot once.");
    } else if report.environment.session_kind == sky_cua_platform::model::SessionKind::Wayland
        && display_topology.selected_provider.as_deref() == Some("xrandr")
    {
        summary.push_str(" DisplayTopologyInferred: display geometry came from XRandR fallback; prefer window-targeted screenshots with the returned snapshot_id for pixel actions.");
    }
}

fn push_input_diagnostics(input: &DoctorInputReport, summary: &mut String) {
    let checks = [
        (&input.ydotool, "ydotool binary"),
        (&input.ydotoold, "ydotoold process"),
        (&input.ydotool_socket, "ydotool socket"),
        (&input.uinput, "/dev/uinput"),
    ];
    let details: Vec<String> = checks
        .iter()
        .filter(|(check, _)| !check.ok)
        .map(|(check, label)| format!("{}: {}", label, check.detail))
        .collect();
    if !details.is_empty() {
        summary.push_str(&format!(" Input details: {}.", details.join("; ")));
    }
}

fn parse_optional_usize(arguments: &Value, name: &str, label: &str) -> Result<Option<usize>> {
    let Some(raw_value) = arguments.get(name) else {
        return Ok(None);
    };
    if raw_value.is_null() {
        return Ok(None);
    }
    let Some(value) = raw_value.as_u64() else {
        return Err(anyhow!("{label} must be a non-negative integer"));
    };
    usize::try_from(value)
        .map(Some)
        .map_err(|_| anyhow!("{label} is too large"))
}

fn parse_optional_string_argument(
    arguments: &Value,
    name: &str,
    label: &str,
) -> Result<Option<String>> {
    let Some(raw_value) = arguments.get(name) else {
        return Ok(None);
    };
    if raw_value.is_null() {
        return Ok(None);
    }
    let Some(raw_value) = raw_value.as_str() else {
        return Err(anyhow!("{label} must be a string"));
    };
    Ok(optional_non_empty_string(raw_value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ScreenshotDelivery {
    /// Reference the capture by `inspection_image_path` only (token-lean default).
    #[default]
    Path,
    /// Also attach the capture as an MCP image content block, for hosts or
    /// agents that cannot read local files by path.
    Inline,
}

fn parse_screenshot_delivery(arguments: &Value) -> ScreenshotDelivery {
    match arguments.get("screenshot_delivery").and_then(Value::as_str) {
        Some("inline") => ScreenshotDelivery::Inline,
        _ => ScreenshotDelivery::Path,
    }
}

/// Build an MCP image content block from the snapshot's persisted inspection image.
/// Returns None when the snapshot has no capture, and Err text when the file
/// cannot be read back.
fn inline_screenshot_block(snapshot: &AppStateSnapshot) -> Option<Result<Value, String>> {
    let path = snapshot.capture.as_ref()?.screenshot_path.as_deref()?;
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let mime_type = match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    };
    Some(match std::fs::read(path) {
        Ok(bytes) => {
            use base64::Engine as _;
            Ok(json!({
                "type": "image",
                "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                "mimeType": mime_type,
            }))
        }
        Err(error) => Err(format!("{path}: {error}")),
    })
}

pub(crate) fn effective_capture_screen(
    arguments: &Value,
    model: &ModelSessionInfo,
) -> CaptureScreenMode {
    if !model.can_receive_images() {
        return CaptureScreenMode::Never;
    }
    parse_capture_screen_mode(arguments).unwrap_or_default()
}

fn parse_capture_screen_mode(arguments: &Value) -> Option<CaptureScreenMode> {
    match arguments.get("capture_screen").and_then(Value::as_str) {
        Some("auto") => Some(CaptureScreenMode::Auto),
        Some("if_changed") => Some(CaptureScreenMode::IfChanged),
        Some("always") => Some(CaptureScreenMode::Always),
        Some("never") => Some(CaptureScreenMode::Never),
        _ => None,
    }
}

fn parse_app_selector(arguments: &Value) -> Option<AppSelector> {
    let selector = AppSelector {
        app_id: arguments
            .get("app_id")
            .and_then(Value::as_str)
            .and_then(optional_non_empty_string),
        desktop_file_id: arguments
            .get("desktop_file_id")
            .and_then(Value::as_str)
            .and_then(optional_non_empty_string),
        window_title: arguments
            .get("window_title")
            .and_then(Value::as_str)
            .and_then(optional_non_empty_string),
        name: arguments
            .get("name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty_string),
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

fn optional_non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use chrono::Utc;

    use serde_json::{Value, json};
    use sky_cua_platform::model::{
        AccessibilitySetupReport, ActionName, ActionOutcome, ActionRequest, AgentCursorPoint,
        AgentCursorState, AppInfo, AppShotAccessibilityStatus, AppShotActionSnapshot,
        AppShotApplication, AppShotCapture, AppShotCaptureResult, AppShotConsistency,
        AppShotCoverage, AppShotEnvelope, AppShotImage, AppShotTrigger, AppStateSnapshot,
        BrowserEvalResponse, BrowserRequest, BrowserResponse, BrowserTargetKind,
        CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, ContentPersistence,
        ContentRef, ContentSource, CoordinateSpace, DiagnosticEntry, DoctorCheck,
        DoctorDisplayTopologyReport, DoctorReadiness, DoctorReport, ElementNode,
        ElementNumericValueReadback, ElementTextReadback, EnvironmentInfo, FocusedApp,
        InputBackendKind, PhoneAppInstallMode, PhoneEnrollmentPayload, PhoneRequest, PixelSize,
        PortalCapabilities, RectF, ScrollDirection, SemanticBackendKind, ServiceRequest,
        ServiceResponse, SessionKind, SessionPresenceAction, SessionPresenceIntent,
        SessionPresenceStatus, SetupCommandReport, ToolAvailability, ToolCapabilities,
        WindowTargetingSetupReport,
    };

    use crate::app_state::{
        APP_STATE_DEFAULT_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_LIMIT,
        APP_STATE_MAX_ELEMENT_QUERY_CHARS, AppStateDetail,
    };
    use crate::heuristics::HeuristicsRegistry;
    use crate::mcp_server::ModelSessionInfo;

    use crate::output_shapes::{
        list_apps_error_diagnostic, setup_accessibility_is_error, setup_window_targeting_is_error,
        snapshot_summary, snapshot_text_content, summary_element, summary_snapshot,
    };

    use super::definitions::schema_accepts;
    use super::{
        McpProcessConfig, McpService, action_summary, build_tool_definitions, build_tool_registry,
        effective_capture_screen, grouped_handler_call, handle_action_call,
        handle_session_tool_call, handle_tool_call, invalid_request_tool_error, list_apps_summary,
        parse_app_selector, parse_app_state_detail, parse_screenshot_target, parse_window_target,
        tools_list_result, validation_tool_definitions,
    };

    const SNAPSHOT_TEXT_TEST_ELEMENT_COUNT: usize = 123;

    #[derive(Default)]
    struct FakeService {
        requests: RefCell<Vec<ServiceRequest>>,
        responses: RefCell<VecDeque<ServiceResponse>>,
        isolated: bool,
    }

    impl FakeService {
        fn with_response(response: ServiceResponse) -> Self {
            Self::with_responses([response])
        }

        fn with_responses(responses: impl IntoIterator<Item = ServiceResponse>) -> Self {
            Self {
                requests: RefCell::new(Vec::new()),
                responses: RefCell::new(responses.into_iter().collect()),
                isolated: false,
            }
        }

        /// Mark this fake as driving the private isolated desktop so
        /// `is_isolated()` reports `true` — the gate `desktop_launch_app` checks.
        fn isolated(mut self) -> Self {
            self.isolated = true;
            self
        }

        fn take_requests(&self) -> Vec<ServiceRequest> {
            self.requests.take()
        }
    }

    impl McpService for FakeService {
        fn call(&self, request: &ServiceRequest) -> anyhow::Result<ServiceResponse> {
            self.requests.borrow_mut().push(request.clone());
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("fake service response queue exhausted"))
        }

        fn is_isolated(&self) -> bool {
            self.isolated
        }
    }

    #[test]
    fn phone_setup_create_enrollment_returns_scannable_single_use_payload() {
        let service = FakeService::with_response(ServiceResponse::PhoneDirectEnrollment {
            payload: Box::new(PhoneEnrollmentPayload {
                protocol: "phone-control.v2".to_string(),
                endpoint: "wss://saga.example.ts.net:43117/phone-control/v2".to_string(),
                enrollment_id: "00000000-0000-4000-8000-000000000001".to_string(),
                bootstrap_credential: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                expires_at_ms: 1_900_000_000_000,
            }),
        });
        let result =
            super::handle_phone_direct_create_enrollment(&service).expect("create enrollment");

        assert_eq!(
            service.take_requests(),
            [ServiceRequest::PhoneDirectCreateEnrollment]
        );
        assert_eq!(result["isError"], false);
        let image = result["content"]
            .as_array()
            .expect("content array")
            .iter()
            .find(|item| item["type"] == "image")
            .expect("QR image attachment");
        assert_eq!(image["mimeType"], "image/png");
        use base64::Engine as _;
        let png = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().expect("base64 image"))
            .expect("valid base64");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        let enrollment_uri = result["structuredContent"]["enrollment_uri"]
            .as_str()
            .expect("enrollment URI");
        let uri = url::Url::parse(enrollment_uri).expect("valid deep link");
        assert_eq!(uri.scheme(), "skycua");
        assert_eq!(uri.host_str(), Some("enroll"));
        let query = uri
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            query.get("protocol").map(|value| value.as_ref()),
            Some("phone-control.v2")
        );
        assert_eq!(
            query.get("endpoint").map(|value| value.as_ref()),
            Some("wss://saga.example.ts.net:43117/phone-control/v2")
        );
        assert_eq!(
            query
                .get("bootstrap_credential")
                .map(|value| value.as_ref()),
            Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        );
        assert_eq!(
            result["structuredContent"]["manual_code"],
            "wss://saga.example.ts.net:43117/phone-control/v2\n00000000-0000-4000-8000-000000000001\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n1900000000000"
        );
    }

    fn captured_action_request(action: ActionName, arguments: serde_json::Value) -> ActionRequest {
        let service = FakeService::with_response(ServiceResponse::ExecuteAction {
            outcome: ActionOutcome {
                success: true,
                message: "action performed".to_string(),
                code: "ActionPerformed".to_string(),
                diagnostics: Vec::new(),
                agent_cursor: None,
            },
        });

        let mut arguments = arguments;
        arguments
            .as_object_mut()
            .expect("test action arguments object")
            .insert("appshot_id".to_string(), serde_json::json!("shot-test"));
        handle_action_call(&service, action, arguments).unwrap();

        let mut requests = service.take_requests();
        assert_eq!(requests.len(), 1, "expected one ExecuteAction request");
        match requests.remove(0) {
            ServiceRequest::ExecuteAction { request } => *request,
            other => panic!("expected one ExecuteAction request: {other:?}"),
        }
    }

    #[test]
    fn desktop_mutation_without_appshot_is_rejected_before_service_dispatch() {
        let service = FakeService::default();
        let error = handle_action_call(
            &service,
            ActionName::Click,
            serde_json::json!({"x": 10.0, "y": 20.0}),
        )
        .expect_err("missing appshot_id must fail closed");
        assert!(error.to_string().contains("appshot_id is required"));
        assert!(service.take_requests().is_empty());
    }

    #[test]
    fn desktop_appshot_required_preserves_fresh_recovery_envelope() {
        let response: ServiceResponse = serde_json::from_value(serde_json::json!({
            "type": "app_shot_required",
            "rejection": {
                "code": "AppShotRequired",
                "reason": "stale",
                "message": "capture again",
                "fresh_appshot": {
                    "appshot_id": "fresh-shot",
                    "trigger": "recovery",
                    "captured_at": "2026-08-03T00:00:00Z",
                    "consistency": "stable",
                    "surface": "desktop",
                    "app_id": "org.example.App",
                    "window_id": "window-1",
                    "bounds": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0, "space": "desktop_logical"},
                    "semantic_projection": {},
                    "image": {
                        "content_id": "image-1",
                        "mime_type": "image/png",
                        "size_bytes": 1,
                        "sha256": "00".repeat(32),
                        "source": "screenshot",
                        "persistence": "temporary"
                    },
                    "action_snapshot": {"snapshot_id": "snapshot-1", "session_id": "session-1"},
                    "coverage": {
                        "pixels_complete": true,
                        "semantics_complete": true,
                        "secure_regions_redacted": false,
                        "projection_truncated": false
                    },
                    "capability_profile_id": "desktop-v1"
                }
            }
        }))
        .expect("valid AppShotRequired response");
        let service = FakeService::with_response(response);
        let result = handle_action_call(
            &service,
            ActionName::Click,
            serde_json::json!({"appshot_id": "stale-shot", "x": 10.0, "y": 20.0}),
        )
        .expect("structured recovery response");
        assert_eq!(result["isError"], true);
        assert_eq!(
            result["structuredContent"]["fresh_appshot"]["appshot_id"],
            "fresh-shot"
        );
    }

    fn process_config(browser_eval_enabled: bool) -> McpProcessConfig {
        McpProcessConfig {
            browser_eval_enabled,
            surfaces: sky_cua_platform::config::AgentSurfacePolicy::default(),
            model_supports_images_override: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn disabled_browser_eval_fails_before_service_dispatch() {
        let service = FakeService::default();
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);

        let eval_result = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "browser_eval",
            json!({"expression": "document.title"}),
        )
        .expect("disabled eval returns feature error");
        assert_eq!(eval_result["isError"], true);
        assert_eq!(eval_result["structuredContent"]["code"], "FeatureDisabled");
        assert!(service.take_requests().is_empty());
    }

    #[test]
    fn desktop_launch_app_refuses_when_not_isolated() {
        // The single most safety-critical branch of this tool: a non-isolated
        // client must be refused before any dispatch so an app can never be
        // launched onto the user's real desktop.
        let service = FakeService::default();
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);

        let result = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "desktop_launch_app",
            json!({"command": "kcalc"}),
        )
        .expect("non-isolated launch returns a gating error");

        assert_eq!(result["isError"], true);
        // The grouped envelope nests the handler's structuredContent under
        // `result`, so the gating code lives at structuredContent.result.code.
        assert_eq!(
            result["structuredContent"]["result"]["code"],
            "IsolatedDesktopRequired"
        );
        // No request reached the service: the refusal precedes dispatch.
        assert!(service.take_requests().is_empty());
    }

    #[test]
    fn desktop_launch_app_dispatches_when_isolated() {
        let service = FakeService::with_response(ServiceResponse::LaunchApplication {
            pid: 4242,
            destination_appshot: None,
            diagnostics: vec![DiagnosticEntry {
                code: "DestinationAppShotUnavailable".into(),
                message: "capture unavailable".into(),
                details: None,
            }],
        })
        .isolated();
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);

        let result = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "desktop_launch_app",
            json!({"command": "kcalc", "args": ["--help"]}),
        )
        .expect("isolated launch dispatches");

        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["result"]["pid"], 4242);
        assert!(
            result["structuredContent"]["result"]
                .get("destination_appshot")
                .is_some()
        );
        assert_eq!(
            result["structuredContent"]["result"]["diagnostics"][0]["code"],
            "DestinationAppShotUnavailable"
        );

        let requests = service.take_requests();
        assert_eq!(requests.len(), 1);
        match &requests[0] {
            ServiceRequest::LaunchApplication { command, args } => {
                assert_eq!(command, "kcalc");
                assert_eq!(args, &vec!["--help".to_string()]);
            }
            other => panic!("expected a LaunchApplication request, got {other:?}"),
        }
    }

    #[test]
    fn activate_window_preserves_unavailable_destination_appshot() {
        let service = FakeService::with_response(ServiceResponse::ActivateWindow {
            outcome: ActionOutcome {
                success: true,
                message: "activated".to_string(),
                code: "WindowActivated".to_string(),
                diagnostics: vec![DiagnosticEntry {
                    code: "DestinationAppShotUnavailable".to_string(),
                    message: "capture unavailable".to_string(),
                    details: None,
                }],
                agent_cursor: None,
            },
            destination_appshot: None,
        });
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let result = handle_tool_call(
            &service,
            &heuristics,
            &model,
            "activate_window",
            json!({"window_id": "window-1"}),
        )
        .expect("activate response");
        assert_eq!(result["isError"], false);
        assert!(result["structuredContent"]["destination_appshot"].is_null());
    }

    #[test]
    fn browser_eval_dispatch_uses_frozen_session_policy() {
        unsafe { std::env::remove_var("SKY_CUA_BROWSER_EVAL") };
        let service = FakeService::with_response(ServiceResponse::Browser {
            response: BrowserResponse::Eval {
                response: BrowserEvalResponse {
                    target: BrowserTargetKind::UserChrome,
                    tab_id: "tab-1".to_string(),
                    value: Some(json!({"title": "ok"})),
                    diagnostics: Vec::new(),
                },
            },
        });
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(true), &model);

        let result = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "browser_eval",
            json!({"tab_id": "tab-1", "appshot_id": "appshot-1", "expression": "document.title"}),
        )
        .expect("frozen eval policy should permit dispatch");

        assert_eq!(result["isError"], false);
        assert_eq!(
            result["structuredContent"]["result"]["value"]["title"],
            "ok"
        );
        let mut requests = service.take_requests();
        assert_eq!(requests.len(), 1);
        match requests.remove(0) {
            ServiceRequest::Browser { request, .. } => {
                assert!(matches!(
                    request,
                    sky_cua_platform::model::BrowserRequest::Eval { .. }
                ));
            }
            other => panic!("expected browser eval request: {other:?}"),
        }
    }

    #[test]
    fn grouped_handler_call_maps_branch_names() {
        let cases = [
            (
                "status",
                json!({"component": "browser"}),
                "browser_status",
                json!({}),
            ),
            (
                "list_resources",
                json!({"surface": "desktop", "resource": "windows"}),
                "list_windows",
                json!({}),
            ),
            (
                "observe",
                json!({"surface": "browser", "tab_id": "tab-1"}),
                "browser_appshot",
                json!({"tab_id": "tab-1"}),
            ),
            (
                "capture_screen",
                json!({"surface": "phone", "session_id": "phone-1"}),
                "phone_screenshot",
                json!({"session_id": "phone-1"}),
            ),
            (
                "desktop_pointer",
                json!({"operation": "secondary_click", "element_index": 2}),
                "perform_secondary_action",
                json!({"element_index": 2}),
            ),
            (
                "browser_input",
                json!({"operation": "press_key", "tab_id": "tab-1", "key": "Enter"}),
                "browser_press_key",
                json!({"tab_id": "tab-1", "key": "Enter"}),
            ),
            (
                "phone_notification_action",
                json!({"operation": "dismiss", "session_id": "phone-1", "event_id": "n-1"}),
                "phone_notification_dismiss",
                json!({"session_id": "phone-1", "event_id": "n-1"}),
            ),
            (
                "phone_notification_reply",
                json!({"session_id": "phone-1", "event_id": "n-1", "action_id": "reply", "text": "hello"}),
                "phone_notification_reply",
                json!({"session_id": "phone-1", "event_id": "n-1", "action_id": "reply", "text": "hello"}),
            ),
            (
                "phone_app_action",
                json!({"operation": "open_intent", "session_id": "phone-1", "intent_uri": "intent://x"}),
                "phone_app_open_intent",
                json!({"session_id": "phone-1", "intent_uri": "intent://x"}),
            ),
            (
                "phone_app_install",
                json!({"session_id": "phone-1", "apk_paths": ["/tmp/app.apk"], "mode": "single"}),
                "phone_app_install",
                json!({"session_id": "phone-1", "apk_paths": ["/tmp/app.apk"], "mode": "single"}),
            ),
        ];

        for (tool, arguments, expected_name, expected_arguments) in cases {
            let call = grouped_handler_call(tool, arguments).expect("grouped call maps");
            assert_eq!(call.handler_name, expected_name, "{tool} handler name");
            assert_eq!(call.arguments, expected_arguments, "{tool} arguments");
        }
    }

    #[test]
    fn grouped_desktop_keyboard_routes_to_typed_action_request() {
        let service = FakeService::with_response(ServiceResponse::ExecuteAction {
            outcome: ActionOutcome {
                success: true,
                message: "pressed key".to_string(),
                code: "ActionPerformed".to_string(),
                diagnostics: Vec::new(),
                agent_cursor: None,
            },
        });
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);

        let result = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "desktop_keyboard",
            json!({"operation": "press_key", "key": "Enter", "appshot_id": "desktop-shot-1"}),
        )
        .expect("desktop keyboard call");
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["tool"], "desktop_keyboard");
        assert_eq!(result["structuredContent"]["branch"], "press_key");
        assert_eq!(
            result["structuredContent"]["result"]["message"],
            "pressed key"
        );
        assert!(
            result["content"][0]["text"]
                .as_str()
                .expect("result text")
                .starts_with("desktop_keyboard/press_key.")
        );

        let mut requests = service.take_requests();
        assert_eq!(requests.len(), 1);
        match requests.remove(0) {
            ServiceRequest::ExecuteAction { request } => {
                assert_eq!(request.action, ActionName::PressKey);
                assert_eq!(request.arguments["key"], "Enter");
            }
            other => panic!("expected ExecuteAction request: {other:?}"),
        }
    }

    #[test]
    fn grouped_invalid_request_returns_grouped_error_envelope_without_dispatch() {
        let service = FakeService::default();
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);

        let result = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "status",
            json!({"component": "desktop"}),
        )
        .expect("invalid request result");

        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["tool"], "status");
        assert_eq!(result["structuredContent"]["branch"], Value::Null);
        assert_eq!(
            result["structuredContent"]["error"]["code"],
            "InvalidRequest"
        );
        assert!(service.take_requests().is_empty());
    }

    #[test]
    fn grouped_schema_rejections_include_shape_repair_hints() {
        let service = FakeService::default();
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);

        let malformed_phone_pointer = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "phone_pointer",
            json!({
                "phone_snapshot_id": "phone-123\n  \"operation\": \"tap\",\n  \"x\": 720,\n  \"y\": 1558"
            }),
        )
        .expect("malformed phone pointer should return invalid request");
        assert_eq!(malformed_phone_pointer["isError"], true);
        let phone_message = malformed_phone_pointer["structuredContent"]["error"]["message"]
            .as_str()
            .expect("phone pointer error message");
        assert!(phone_message.contains("phone_snapshot_id"));
        assert!(phone_message.contains("separate top-level JSON keys"));

        let malformed_browser_scroll = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "browser_scroll",
            json!({"delta_y": 500}),
        )
        .expect("malformed browser scroll should return invalid request");
        assert_eq!(malformed_browser_scroll["isError"], true);
        let scroll_message = malformed_browser_scroll["structuredContent"]["error"]["message"]
            .as_str()
            .expect("browser scroll error message");
        assert!(scroll_message.contains("top-level `tab_id`"));
        assert!(scroll_message.contains("delta_y"));

        let malformed_browser_input = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "browser_input",
            json!({"operation": "type_text", "tab_id": "tab-1", "text": "hello", "x": 1, "y": 2}),
        )
        .expect("malformed browser input should return invalid request");
        assert_eq!(malformed_browser_input["isError"], true);
        let input_message = malformed_browser_input["structuredContent"]["error"]["message"]
            .as_str()
            .expect("browser input error message");
        assert!(input_message.contains("top-level `operation`"));
        assert!(input_message.contains("type_text uses"));

        let malformed_desktop_pointer = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "desktop_pointer",
            json!({
                "snapshot_id": "desktop-123\n  \"operation\": \"click\",\n  \"element_index\": 1"
            }),
        )
        .expect("malformed desktop pointer should return invalid request");
        assert_eq!(malformed_desktop_pointer["isError"], true);
        let pointer_message = malformed_desktop_pointer["structuredContent"]["error"]["message"]
            .as_str()
            .expect("desktop pointer error message");
        assert!(pointer_message.contains("opaque desktop snapshot id"));
        assert!(pointer_message.contains("separate top-level JSON keys"));

        let malformed_desktop_scroll = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "desktop_scroll",
            json!({"direction": "down", "x": 1, "y": 2}),
        )
        .expect("malformed desktop scroll should return invalid request");
        assert_eq!(malformed_desktop_scroll["isError"], true);
        let desktop_scroll_message =
            malformed_desktop_scroll["structuredContent"]["error"]["message"]
                .as_str()
                .expect("desktop scroll error message");
        assert!(desktop_scroll_message.contains("snapshot-resolved"));
        assert!(desktop_scroll_message.contains("does not accept freeform x/y"));

        let malformed_capture = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "capture_desktop",
            json!({"window_id": "win-1", "display_id": "display-1"}),
        )
        .expect("malformed desktop capture should return invalid request");
        assert_eq!(malformed_capture["isError"], true);
        let capture_message = malformed_capture["structuredContent"]["error"]["message"]
            .as_str()
            .expect("desktop capture error message");
        assert!(capture_message.contains("captures a single screen"));
        assert!(capture_message.contains("do not mix them"));

        let retired_all_displays = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "capture_desktop",
            json!({"capture_all_displays": true}),
        )
        .expect("retired all-displays selector should return invalid request");
        assert_eq!(
            retired_all_displays["isError"], true,
            "capture_desktop must not let the model capture every display"
        );

        let malformed_desktop_action = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "desktop_action",
            json!({"operation": "perform_action", "snapshot_id": "desktop-1", "element_index": 1}),
        )
        .expect("malformed desktop action should return invalid request");
        assert_eq!(malformed_desktop_action["isError"], true);
        let desktop_action_message =
            malformed_desktop_action["structuredContent"]["error"]["message"]
                .as_str()
                .expect("desktop action error message");
        assert!(desktop_action_message.contains("semantic target"));
        assert!(desktop_action_message.contains("action_name"));

        let malformed_observe = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "observe",
            json!({"surface": "phone", "tab_id": "tab-1"}),
        )
        .expect("malformed observe should return invalid request");
        assert_eq!(malformed_observe["isError"], true);
        let observe_message = malformed_observe["structuredContent"]["error"]["message"]
            .as_str()
            .expect("observe error message");
        assert!(observe_message.contains("phone requires"));
        assert!(observe_message.contains("do not mix fields"));

        let malformed_list_resources = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "list_resources",
            json!({"surface": "phone", "resource": "apps"}),
        )
        .expect("malformed list resources should return invalid request");
        assert_eq!(malformed_list_resources["isError"], true);
        let list_message = malformed_list_resources["structuredContent"]["error"]["message"]
            .as_str()
            .expect("list resources error message");
        assert!(list_message.contains("Phone `apps`"));
        assert!(list_message.contains("session_id"));

        let malformed_phone_connection = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "phone_connection",
            json!({"operation": "disconnect", "serial": "emulator-5554", "install_companion": true}),
        )
        .expect("malformed phone connection should return invalid request");
        assert_eq!(malformed_phone_connection["isError"], true);
        let connection_message =
            malformed_phone_connection["structuredContent"]["error"]["message"]
                .as_str()
                .expect("phone connection error message");
        assert!(connection_message.contains("disconnect"));
        assert!(connection_message.contains("connect-only fields"));

        let malformed_phone_install = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "phone_app_install",
            json!({"session_id": "phone-1", "apk_path": "/tmp/app.apk"}),
        )
        .expect("malformed phone install should return invalid request");
        assert_eq!(malformed_phone_install["isError"], true);
        let install_message = malformed_phone_install["structuredContent"]["error"]["message"]
            .as_str()
            .expect("phone install error message");
        assert!(install_message.contains("apk_paths"));
        assert!(install_message.contains("no `apk_path`"));

        let malformed_browser_navigate = handle_session_tool_call(
            &service,
            &heuristics,
            &model,
            &registry,
            "browser_navigate",
            json!({"tab_id": "tab-1", "url": "ftp://example.test"}),
        )
        .expect("malformed browser navigate should return invalid request");
        assert_eq!(malformed_browser_navigate["isError"], true);
        let navigate_message = malformed_browser_navigate["structuredContent"]["error"]["message"]
            .as_str()
            .expect("browser navigate error message");
        assert!(navigate_message.contains("HTTP(S)"));
        assert!(navigate_message.contains("about:blank"));

        assert!(
            service.take_requests().is_empty(),
            "schema-rejected calls must not dispatch"
        );
    }

    #[test]
    fn grouped_status_schema_allows_phone_branch_arguments() {
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);

        let phone_status_service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "StatusProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let phone_status_result = handle_session_tool_call(
            &phone_status_service,
            &heuristics,
            &model,
            &registry,
            "status",
            json!({"component": "phone", "refresh_devices": true}),
        )
        .expect("phone status should pass grouped schema validation");
        let mut requests = phone_status_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "phone status should dispatch, got result: {phone_status_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Phone {
                request: PhoneRequest::Status(request),
                ..
            } => assert!(request.refresh_devices),
            other => panic!("expected phone status request: {other:?}"),
        }

        let companion_status_service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "StatusProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let companion_status_result = handle_session_tool_call(
            &companion_status_service,
            &heuristics,
            &model,
            &registry,
            "status",
            json!({"component": "phone_companion", "session_id": "phone-1"}),
        )
        .expect("phone companion status should pass grouped schema validation");
        let mut requests = companion_status_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "phone companion status should dispatch, got result: {companion_status_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Phone {
                request: PhoneRequest::CompanionStatus(request),
                ..
            } => assert_eq!(request.session.session_id.as_deref(), Some("phone-1")),
            other => panic!("expected phone companion status request: {other:?}"),
        }
    }

    #[test]
    fn grouped_schema_allows_parser_tolerated_optional_sentinels() {
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo {
            supports_images: Some(true),
        };
        let registry = build_tool_registry(&process_config(false), &model);

        let cases = [
            json!({"target": "", "url": ""}),
            json!({"target": null, "url": null}),
        ];

        for arguments in cases {
            let service = FakeService::with_response(ServiceResponse::Error {
                ok: false,
                code: "OpenProbe".to_string(),
                message: "captured".to_string(),
                session_id: None,
                turn_id: None,
                retry: None,
            });
            let result = handle_session_tool_call(
                &service,
                &heuristics,
                &model,
                &registry,
                "browser_open",
                arguments,
            )
            .expect("browser_open sentinels should pass grouped schema validation");
            let mut requests = service.take_requests();
            assert_eq!(
                requests.len(),
                1,
                "browser_open should dispatch, got result: {result}"
            );
            match requests.remove(0) {
                ServiceRequest::Browser {
                    request:
                        BrowserRequest::Open {
                            target: None,
                            url: None,
                        },
                    ..
                } => {}
                other => panic!("expected blank browser open request: {other:?}"),
            }
        }

        let scroll_service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "ScrollProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let scroll_result = handle_session_tool_call(
            &scroll_service,
            &heuristics,
            &model,
            &registry,
            "browser_scroll",
            json!({
                "tab_id": "tab-1",
                "appshot_id": "browser-shot-1",
                "delta_y": 500,
                "x": null,
                "y": null
            }),
        )
        .expect("browser_scroll null coordinates should pass grouped schema validation");
        let mut requests = scroll_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "browser_scroll should dispatch, got result: {scroll_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Browser {
                request:
                    sky_cua_platform::model::BrowserRequest::Scroll {
                        tab_id,
                        delta_y,
                        x,
                        y,
                        ..
                    },
                ..
            } => {
                assert_eq!(tab_id, "tab-1");
                assert_eq!(delta_y, 500.0);
                assert_eq!(x, None);
                assert_eq!(y, None);
            }
            other => panic!("expected viewport browser scroll request: {other:?}"),
        }

        let scroll_null_x_service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "ScrollProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let scroll_null_x_result = handle_session_tool_call(
            &scroll_null_x_service,
            &heuristics,
            &model,
            &registry,
            "browser_scroll",
            json!({
                "tab_id": "tab-1",
                "appshot_id": "browser-shot-1",
                "delta_y": 500,
                "x": null
            }),
        )
        .expect("browser_scroll null coordinate sentinel should pass grouped schema validation");
        assert_eq!(
            scroll_null_x_service.take_requests().len(),
            1,
            "browser_scroll null coordinate should dispatch, got result: {scroll_null_x_result}"
        );

        let scroll_half_point_service = FakeService::default();
        let scroll_half_point_result = handle_session_tool_call(
            &scroll_half_point_service,
            &heuristics,
            &model,
            &registry,
            "browser_scroll",
            json!({
                "tab_id": "tab-1",
                "appshot_id": "browser-shot-1",
                "delta_y": 500,
                "x": 10
            }),
        )
        .expect("browser_scroll half coordinate should return an invalid envelope");
        assert_eq!(scroll_half_point_result["isError"], true);
        assert_eq!(
            scroll_half_point_result["structuredContent"]["branch"],
            Value::Null
        );
        assert_eq!(
            scroll_half_point_result["structuredContent"]["error"]["code"],
            "InvalidRequest"
        );
        assert!(
            scroll_half_point_service.take_requests().is_empty(),
            "browser_scroll numeric half coordinate should not dispatch"
        );

        let image_model = ModelSessionInfo {
            supports_images: Some(true),
        };
        let image_registry = build_tool_registry(&process_config(false), &image_model);
        let service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "ObserveProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let result = handle_session_tool_call(
            &service,
            &heuristics,
            &image_model,
            &image_registry,
            "observe",
            json!({
                "surface": "desktop",
                "detail": null
            }),
        )
        .expect("desktop observe sentinels should pass grouped schema validation");
        let mut requests = service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "desktop observe should dispatch, got result: {result}"
        );
        match requests.remove(0) {
            ServiceRequest::AppShotCapture {
                target: None,
                frontmost: true,
                flags,
                ..
            } => assert!(flags.include_ax_text),
            other => panic!("expected default desktop observe request: {other:?}"),
        }

        let pointer_service = FakeService::with_response(ServiceResponse::ExecuteAction {
            outcome: ActionOutcome {
                success: true,
                message: "clicked".to_string(),
                code: "ActionPerformed".to_string(),
                diagnostics: Vec::new(),
                agent_cursor: None,
            },
        });
        let pointer_result = handle_session_tool_call(
            &pointer_service,
            &heuristics,
            &model,
            &registry,
            "desktop_pointer",
            json!({
                "operation": "click",
                "appshot_id": "desktop-shot-1",
                "snapshot_id": "",
                "element_index": 0,
                "x": 12.5,
                "y": 42.0
            }),
        )
        .expect("desktop_pointer blank snapshot sentinel should pass grouped schema validation");
        let mut requests = pointer_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "desktop_pointer should dispatch, got result: {pointer_result}"
        );
        match requests.remove(0) {
            ServiceRequest::ExecuteAction { request } => {
                assert_eq!(request.action, ActionName::Click);
                assert_eq!(request.snapshot_id, None);
                assert_eq!(request.element_index, None);
                assert_eq!(request.arguments["x"], 12.5);
                assert_eq!(request.arguments["y"], 42.0);
            }
            other => panic!("expected click action request: {other:?}"),
        }

        let phone_service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "InstallProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let phone_result = handle_session_tool_call(
            &phone_service,
            &heuristics,
            &model,
            &registry,
            "phone_app_install",
            json!({
                "session_id": "phone-1",
                "appshot_id": "phone-shot-1",
                "apk_paths": ["/tmp/base.apk"],
                "mode": null,
                "reinstall": null,
                "allow_downgrade": null,
                "allow_test_apk": null,
                "grant_runtime_permissions": null
            }),
        )
        .expect("phone app install sentinels should pass grouped schema validation");
        let mut requests = phone_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "phone_app_install should dispatch, got result: {phone_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Phone {
                request: PhoneRequest::AppInstall(request),
                ..
            } => {
                assert_eq!(request.apk_paths, vec!["/tmp/base.apk"]);
                assert_eq!(request.mode, PhoneAppInstallMode::Single);
                assert!(!request.reinstall);
                assert!(!request.allow_downgrade);
                assert!(!request.allow_test_apk);
                assert!(!request.grant_runtime_permissions);
            }
            other => panic!("expected phone app install request: {other:?}"),
        }

        for package_name in [Value::Null, json!("")] {
            let phone_setup_service = FakeService::with_response(ServiceResponse::Error {
                ok: false,
                code: "SettingsProbe".to_string(),
                message: "captured".to_string(),
                session_id: None,
                turn_id: None,
                retry: None,
            });
            let phone_setup_result = handle_session_tool_call(
                &phone_setup_service,
                &heuristics,
                &model,
                &registry,
                "phone_setup",
                json!({
                    "operation": "open_settings",
                    "session_id": "phone-1",
                    "screen": "accessibility",
                    "package_name": package_name
                }),
            )
            .expect("phone settings package sentinels should pass grouped schema validation");
            let mut requests = phone_setup_service.take_requests();
            assert_eq!(
                requests.len(),
                1,
                "phone_setup should dispatch, got result: {phone_setup_result}"
            );
            match requests.remove(0) {
                ServiceRequest::Phone {
                    request: PhoneRequest::OpenSettings(request),
                    ..
                } => {
                    assert_eq!(request.session.session_id.as_deref(), Some("phone-1"));
                    assert_eq!(request.package_name, None);
                }
                other => panic!("expected phone open-settings request: {other:?}"),
            }
        }

        let screenshot_service = FakeService::with_response(ServiceResponse::Screenshot {
            snapshot: Box::new(snapshot_with_verbose_element()),
        });
        let screenshot_result = handle_session_tool_call(
            &screenshot_service,
            &heuristics,
            &model,
            &registry,
            "capture_desktop",
            json!({
                "display_id": "kwin:HDMI-A-1",
                "screenshot_delivery": null
            }),
        )
        .expect(
            "null screenshot_delivery sentinel with display selector should pass grouped schema",
        );
        let mut requests = screenshot_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "capture_desktop should dispatch, got result: {screenshot_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Screenshot {
                target: None,
                display_target: Some(display_target),
            } => assert_eq!(display_target.display_id.as_deref(), Some("kwin:HDMI-A-1")),
            other => panic!("expected display screenshot request: {other:?}"),
        }

        let display_screenshot_service = FakeService::with_response(ServiceResponse::Screenshot {
            snapshot: Box::new(snapshot_with_verbose_element()),
        });
        let display_screenshot_result = handle_session_tool_call(
            &display_screenshot_service,
            &heuristics,
            &model,
            &registry,
            "capture_desktop",
            json!({
                "display_id": "kwin:HDMI-A-1",
                "pid": 0
            }),
        )
        .expect("ignored pid sentinel with display selector should pass grouped schema");
        let mut requests = display_screenshot_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "display capture should dispatch, got result: {display_screenshot_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Screenshot {
                target: None,
                display_target: Some(display_target),
            } => assert_eq!(display_target.display_id.as_deref(), Some("kwin:HDMI-A-1")),
            other => panic!("expected display screenshot request: {other:?}"),
        }

        let primary_screenshot_service = FakeService::with_response(ServiceResponse::Screenshot {
            snapshot: Box::new(snapshot_with_verbose_element()),
        });
        let primary_screenshot_result = handle_session_tool_call(
            &primary_screenshot_service,
            &heuristics,
            &model,
            &registry,
            "capture_desktop",
            json!({"pid": 0, "title": ""}),
        )
        .expect("blank/zero window target sentinels should pass grouped schema");
        let mut requests = primary_screenshot_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "primary capture should dispatch, got result: {primary_screenshot_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Screenshot {
                target: None,
                display_target: None,
            } => {}
            other => panic!("expected primary screenshot request: {other:?}"),
        }

        let whitespace_title_screenshot_service =
            FakeService::with_response(ServiceResponse::Screenshot {
                snapshot: Box::new(snapshot_with_verbose_element()),
            });
        let whitespace_title_screenshot_result = handle_session_tool_call(
            &whitespace_title_screenshot_service,
            &heuristics,
            &model,
            &registry,
            "capture_desktop",
            json!({
                "display_id": "kwin:HDMI-A-1",
                "title": " "
            }),
        )
        .expect(
            "ignored whitespace title sentinel with display selector should pass grouped schema",
        );
        let mut requests = whitespace_title_screenshot_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "display capture should dispatch, got result: {whitespace_title_screenshot_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Screenshot {
                target: None,
                display_target: Some(display_target),
            } => assert_eq!(display_target.display_id.as_deref(), Some("kwin:HDMI-A-1")),
            other => panic!("expected display screenshot request: {other:?}"),
        }

        let notifications_service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "NotificationsProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let notifications_result = handle_session_tool_call(
            &notifications_service,
            &heuristics,
            &model,
            &registry,
            "phone_notifications",
            json!({"session_id": "phone-1", "limit": null}),
        )
        .expect("phone_notifications null limit should pass grouped schema validation");
        let mut requests = notifications_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "phone_notifications should dispatch, got result: {notifications_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Phone {
                request: PhoneRequest::Notifications(request),
                ..
            } => assert_eq!(request.limit, None),
            other => panic!("expected phone notifications request: {other:?}"),
        }

        let app_list_service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "AppListProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let app_list_result = handle_session_tool_call(
            &app_list_service,
            &heuristics,
            &model,
            &registry,
            "list_resources",
            json!({
                "surface": "phone",
                "resource": "apps",
                "session_id": "phone-1",
                "include_system": null,
                "limit": null
            }),
        )
        .expect("phone app list null optionals should pass grouped schema validation");
        let mut requests = app_list_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "phone app list should dispatch, got result: {app_list_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Phone {
                request: PhoneRequest::AppList(request),
                ..
            } => {
                assert!(!request.include_system);
                assert_eq!(request.limit, None);
            }
            other => panic!("expected phone app list request: {other:?}"),
        }

        let intent_service = FakeService::with_response(ServiceResponse::Error {
            ok: false,
            code: "IntentProbe".to_string(),
            message: "captured".to_string(),
            session_id: None,
            turn_id: None,
            retry: None,
        });
        let intent_result = handle_session_tool_call(
            &intent_service,
            &heuristics,
            &model,
            &registry,
            "phone_app_action",
            json!({
                "operation": "open_intent",
                "session_id": "phone-1",
                "intent_uri": "intent://example",
                "package_name": "com.example"
            }),
        )
        .expect("scoped phone open-intent should pass grouped schema validation");
        let mut requests = intent_service.take_requests();
        assert_eq!(
            requests.len(),
            1,
            "phone open-intent should dispatch, got result: {intent_result}"
        );
        match requests.remove(0) {
            ServiceRequest::Phone {
                request: PhoneRequest::AppOpenIntent(request),
                ..
            } => {
                assert_eq!(request.intent_uri, "intent://example");
                assert_eq!(request.package_name.as_deref(), Some("com.example"));
            }
            other => panic!("expected phone app open-intent request: {other:?}"),
        }

        for package_name in [Value::Null, json!("")] {
            let intent_service = FakeService::with_response(ServiceResponse::Error {
                ok: false,
                code: "IntentProbe".to_string(),
                message: "captured".to_string(),
                session_id: None,
                turn_id: None,
                retry: None,
            });
            let intent_result = handle_session_tool_call(
                &intent_service,
                &heuristics,
                &model,
                &registry,
                "phone_app_action",
                json!({
                    "operation": "open_intent",
                    "session_id": "phone-1",
                    "intent_uri": "intent://example",
                    "package_name": package_name
                }),
            )
            .expect("optional phone open-intent package sentinels should pass grouped schema");
            let mut requests = intent_service.take_requests();
            assert_eq!(
                requests.len(),
                1,
                "phone open-intent should dispatch, got result: {intent_result}"
            );
            match requests.remove(0) {
                ServiceRequest::Phone {
                    request: PhoneRequest::AppOpenIntent(request),
                    ..
                } => {
                    assert_eq!(request.intent_uri, "intent://example");
                    assert_eq!(request.package_name, None);
                }
                other => panic!("expected phone app open-intent request: {other:?}"),
            }
        }
    }

    #[test]
    fn grouped_schema_rejections_stop_before_dispatch() {
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics should load");
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);
        let cases = [
            ("doctor", json!({"unexpected": true})),
            (
                "browser_input",
                json!({"operation": "type_text", "tab_id": "tab-1", "text": "hello", "x": 1, "y": 1}),
            ),
            (
                "browser_input",
                json!({"operation": "type_text", "tab_id": 456, "text": "hello"}),
            ),
            (
                "browser_input",
                json!({"operation": "type_text", "tab_id": " ", "text": "hello"}),
            ),
            (
                "browser_input",
                json!({"operation": "press_key", "tab_id": "tab-1", "key": " "}),
            ),
            (
                "desktop_pointer",
                json!({
                    "operation": "click",
                    "x": 1,
                    "y": 1,
                    "to_x": 2
                }),
            ),
            (
                "desktop_pointer",
                json!({
                    "operation": "click",
                    "snapshot_id": " ",
                    "element_index": 0
                }),
            ),
            (
                "desktop_pointer",
                json!({
                    "operation": "click",
                    "snapshot_id": "snapshot-1",
                    "name": " "
                }),
            ),
            (
                "desktop_pointer",
                json!({
                    "operation": "drag",
                    "snapshot_id": "",
                    "element_index": 0,
                    "to_element_index": 1
                }),
            ),
            (
                "desktop_keyboard",
                json!({
                    "operation": "type_text",
                    "text": "hello",
                    "key": "Enter"
                }),
            ),
            (
                "desktop_action",
                json!({
                    "operation": "activate",
                    "snapshot_id": "snapshot-1",
                    "element_index": 1,
                    "action_name": "press"
                }),
            ),
            (
                "desktop_action",
                json!({
                    "operation": "activate",
                    "element_identifier": " "
                }),
            ),
            (
                "desktop_action",
                json!({
                    "operation": "perform_action",
                    "snapshot_id": "snapshot-1",
                    "element_index": 0,
                    "action_name": " "
                }),
            ),
            ("activate_window", json!({"pid": 0})),
            ("activate_window", json!({"window_id": ""})),
            ("activate_window", json!({"title": " "})),
            (
                "phone_connection",
                json!({"operation": "disconnect", "session_id": "phone-1", "install_companion": true}),
            ),
            (
                "phone_pointer",
                json!({"operation": "tap", "session_id": "phone-1", "x": 1, "y": 1}),
            ),
            (
                "phone_pointer",
                json!({
                    "operation": "tap",
                    "session_id": "phone-1",
                    "phone_snapshot_id": " ",
                    "x": 1,
                    "y": 1
                }),
            ),
            (
                "phone_pointer",
                json!({
                    "operation": "tap",
                    "session_id": "phone-1",
                    "phone_snapshot_id": "snap-1",
                    "x": 1,
                    "y": 1,
                    "duration_ms": 10
                }),
            ),
            (
                "phone_pointer",
                json!({
                    "operation": "tap",
                    "session_id": "phone-1",
                    "phone_snapshot_id": "snap-1",
                    "x": 1,
                    "y": 1,
                    "start_x": 1
                }),
            ),
            (
                "phone_setup",
                json!({
                    "operation": "open_settings",
                    "session_id": "phone-1",
                    "screen": "app_details",
                    "package_name": ""
                }),
            ),
            (
                "phone_setup",
                json!({
                    "operation": "open_settings",
                    "session_id": "phone-1",
                    "screen": "app_details",
                    "package_name": " "
                }),
            ),
            (
                "phone_keyboard",
                json!({
                    "operation": "type_text",
                    "session_id": "phone-1",
                    "text": "hello",
                    "key": "Enter"
                }),
            ),
            (
                "phone_keyboard",
                json!({
                    "operation": "press_key",
                    "session_id": "phone-1",
                    "key": " "
                }),
            ),
            (
                "phone_notification_action",
                json!({
                    "operation": "open",
                    "session_id": "phone-1",
                    "event_id": "event-1",
                    "action_id": "reply"
                }),
            ),
            (
                "phone_notification_action",
                json!({
                    "operation": "open",
                    "session_id": "phone-1",
                    "event_id": " "
                }),
            ),
            (
                "phone_notification_reply",
                json!({
                    "session_id": "phone-1",
                    "event_id": " ",
                    "action_id": "reply",
                    "text": "hello"
                }),
            ),
            (
                "phone_notification_reply",
                json!({
                    "session_id": "phone-1",
                    "event_id": "event-1",
                    "action_id": " ",
                    "text": "hello"
                }),
            ),
            (
                "phone_app_action",
                json!({
                    "operation": "launch",
                    "session_id": "phone-1",
                    "package_name": "com.example",
                    "intent_uri": "app://example"
                }),
            ),
            (
                "phone_app_action",
                json!({
                    "operation": "launch",
                    "session_id": "phone-1",
                    "package_name": ""
                }),
            ),
            (
                "phone_app_action",
                json!({
                    "operation": "launch",
                    "session_id": "phone-1",
                    "package_name": " "
                }),
            ),
            (
                "phone_app_action",
                json!({
                    "operation": "open_intent",
                    "session_id": "phone-1",
                    "intent_uri": " "
                }),
            ),
            (
                "phone_app_force_stop",
                json!({"session_id": "phone-1", "package_name": " "}),
            ),
            (
                "capture_desktop",
                json!({"window_id": "window-1", "display_id": "display-1"}),
            ),
            ("browser_open", json!({"url": "junkabout:blank"})),
            ("browser_open", json!({"url": "https://"})),
            ("browser_open", json!({"url": "ftp://example.test"})),
            (
                "observe",
                json!({"surface": "phone", "session_id": "phone-1", "screenshot_delivery": "inline"}),
            ),
            (
                "capture_screen",
                json!({"surface": "phone", "session_id": "phone-1", "tab_id": "tab-1"}),
            ),
            (
                "list_resources",
                json!({"surface": "desktop", "resource": "apps", "include_mdns": true}),
            ),
        ];

        for (tool_name, arguments) in cases {
            let service = FakeService::default();
            let result = handle_session_tool_call(
                &service,
                &heuristics,
                &model,
                &registry,
                tool_name,
                arguments,
            )
            .unwrap_or_else(|error| panic!("{tool_name} should return invalid envelope: {error}"));
            assert_eq!(result["isError"], true, "{tool_name} should be an error");
            assert_eq!(result["structuredContent"]["tool"], tool_name);
            assert_eq!(result["structuredContent"]["branch"], Value::Null);
            assert_eq!(
                result["structuredContent"]["error"]["code"], "InvalidRequest",
                "{tool_name} should use grouped invalid envelope"
            );
            assert!(
                service.take_requests().is_empty(),
                "{tool_name} should not dispatch"
            );
        }
    }

    #[test]
    fn parses_summary_app_state_detail() {
        assert_eq!(
            parse_app_state_detail(&json!({"detail": "compact"})).unwrap(),
            AppStateDetail::Compact
        );
        assert_eq!(
            parse_app_state_detail(&json!({"detail": "full"})).unwrap(),
            AppStateDetail::Full
        );
        assert_eq!(
            parse_app_state_detail(&json!({"detail": null})).unwrap(),
            AppStateDetail::Compact
        );
        assert_eq!(
            parse_app_state_detail(&json!({})).unwrap(),
            AppStateDetail::Compact
        );
        assert!(parse_app_state_detail(&json!({"detail": "verbose"})).is_err());
        assert!(parse_app_state_detail(&json!({"detail": 1})).is_err());
    }

    #[test]
    fn app_selector_ignores_opencode_blank_default_fields() {
        let selector = parse_app_selector(&json!({
            "app_id": "",
            "desktop_file_id": " chrome.desktop ",
            "window_title": "",
            "name": ""
        }))
        .expect("non-empty desktop_file_id should produce a selector");

        assert_eq!(selector.app_id, None);
        assert_eq!(selector.desktop_file_id.as_deref(), Some("chrome.desktop"));
        assert_eq!(selector.window_title, None);
        assert_eq!(selector.name, None);
    }

    #[test]
    fn summary_element_drops_verbose_description_but_keeps_backend_ref() {
        let summary = summary_element(&ElementNode {
            element_index: 7,
            parent_index: Some(1),
            role: "text".to_string(),
            name: Some("Search".to_string()),
            description: Some("verbose guidance that should not ride every loop".to_string()),
            value: Some("query".to_string()),
            text: Some(ElementTextReadback {
                character_count: 5,
                caret_offset: Some(5),
                content: Some("query".to_string()),
                content_suppressed: false,
                truncated: false,
                selections: Vec::new(),
            }),
            numeric_value: None,
            supports_editable_text: true,
            state_flags: vec!["focused".to_string()],
            semantic_actions: vec!["set_value".to_string()],
            bounds: Some(RectF {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            backend_ref: Some("opaque-backend-ref".to_string()),
        });

        assert_eq!(summary["element_index"], 7);
        assert_eq!(summary["role"], "text");
        assert!(summary.get("description").is_none());
        assert_eq!(summary["value"], "query");
        assert_eq!(summary["text"]["content"], "query");
        assert_eq!(summary["supports_editable_text"], true);
        assert_eq!(summary["backend_ref"], "opaque-backend-ref");
        assert_eq!(summary["semantic_actions"][0], "set_value");
    }

    #[test]
    fn summary_snapshot_includes_doctor_report_and_agent_cursor() {
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
                displays: Vec::new(),
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
                can_inhibit_presence: false,
                can_unlock_session: false,
                recommended_next_step: "Ready".to_string(),
                blockers: Vec::new(),
            },
            platform: None,
            display_topology: None,
            session_env: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
            session_presence: None,
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
                supported_scroll_directions: vec![ScrollDirection::Up, ScrollDirection::Down],
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
            agent_cursor: Some(AgentCursorState {
                visible: true,
                sequence: 7,
                model_point: Some(AgentCursorPoint {
                    x: 42.0,
                    y: 64.0,
                    coordinate_space: CoordinateSpace::StreamPixels,
                    mapping_id: Some("pipewire-stream-1".to_string()),
                }),
                native_point: None,
                snapshot_id: Some("snap-1".to_string()),
                source_action: None,
                updated_at_ms: 1234,
            }),
        };
        let summary = summary_snapshot(&snapshot);
        assert_eq!(summary["environment"]["session_kind"], "wayland");
        assert!(summary.get("doctor_report").is_some());
        assert_eq!(
            summary["doctor_report"]["readiness"]["can_build_accessibility_tree"],
            true
        );
        assert_eq!(summary["agent_cursor"]["sequence"], 7);
        assert_eq!(
            summary["agent_cursor"]["model_point"]["coordinate_space"],
            "stream_pixels"
        );
    }

    #[test]
    fn action_tool_schemas_are_strict_and_snapshot_scoped_where_needed() {
        // Constraint shape (allOf branches, accept/reject) lives in the rich
        // validation schema now; the advertised schema is flattened.
        let tools = validation_tool_definitions(false, false);
        let find_tool = |name: &str| {
            tools
                .as_array()
                .expect("tools")
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("{name} tool"))
        };

        let pointer = find_tool("desktop_pointer");
        let schema = &pointer["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["operation", "appshot_id"]));
        assert!(schema["properties"].get("snapshot_id").is_some());
        assert!(schema.get("allOf").is_some());
        assert!(
            schema_accepts(
                schema,
                &json!({"operation": "drag", "appshot_id":"appshot-1", "snapshot_id": "snap-1", "element_index": 3, "to_x": 500, "to_y": 20})
            ),
            "desktop_pointer drag must allow an observed source element dragged to explicit coordinates"
        );
        assert!(
            schema_accepts(
                schema,
                &json!({"operation": "drag", "appshot_id":"appshot-1", "snapshot_id": "snap-1", "from_x": 1, "from_y": 2, "to_element_index": 3})
            ),
            "desktop_pointer drag must allow explicit source coordinates dragged to an observed target element"
        );
        assert!(
            schema_accepts(
                schema,
                &json!({"operation": "drag", "appshot_id":"appshot-1", "from_x": 1, "from_y": 2, "to_x": 3, "to_y": 4, "duration_ms": 500})
            ),
            "desktop_pointer drag must accept an optional duration_ms that paces the gesture"
        );
        assert!(
            schema_accepts(
                schema,
                &json!({"operation": "click", "appshot_id":"appshot-1", "x": 1, "y": 2})
            ),
            "desktop_pointer click must accept a bare coordinate pair"
        );
        assert!(
            !schema_accepts(
                schema,
                &json!({"operation": "click", "appshot_id":"appshot-1", "x": 1, "y": 2, "duration_ms": 500})
            ),
            "duration_ms is drag-only; click must still reject it"
        );

        let activate = find_tool("desktop_action");
        assert_eq!(
            activate["inputSchema"]["required"],
            json!(["operation", "appshot_id"])
        );
        assert_eq!(activate["inputSchema"]["additionalProperties"], false);
        assert!(
            activate["inputSchema"]["properties"]
                .get("element_identifier")
                .is_some()
        );

        assert!(
            activate["inputSchema"]["properties"]
                .get("action_name")
                .is_some()
        );

        let type_text = find_tool("desktop_keyboard");
        assert_eq!(type_text["inputSchema"]["additionalProperties"], false);
        assert_eq!(
            type_text["inputSchema"]["required"],
            json!(["operation", "appshot_id"])
        );
        let type_text_branch = type_text["inputSchema"]["allOf"]
            .as_array()
            .and_then(|all_of| {
                all_of.iter().find_map(|constraint| {
                    constraint["oneOf"].as_array().and_then(|one_of| {
                        one_of.iter().find(|branch| {
                            branch["properties"]["operation"]["const"] == "type_text"
                        })
                    })
                })
            })
            .expect("desktop_keyboard type_text branch");
        assert_eq!(
            type_text_branch["required"],
            json!(["operation", "appshot_id", "text"])
        );

        let get_app_state = find_tool("observe");
        assert!(
            get_app_state["description"]
                .as_str()
                .is_some_and(|description| description.contains("canonical AppShot"))
        );
        let get_app_state_schema = &get_app_state["inputSchema"];
        assert_eq!(
            get_app_state_schema["properties"]["element_limit"]["anyOf"][0]["maximum"],
            APP_STATE_MAX_ELEMENT_LIMIT
        );
        assert!(
            get_app_state_schema["properties"]
                .get("element_query")
                .is_some()
        );
        assert_eq!(
            get_app_state_schema["properties"]["element_query"]["anyOf"][0]["maxLength"],
            APP_STATE_MAX_ELEMENT_QUERY_CHARS
        );
        assert!(
            get_app_state_schema["properties"]
                .get("element_offset")
                .is_some()
        );

        let screenshot = find_tool("capture_desktop");
        let screenshot_schema = &screenshot["inputSchema"];
        assert_eq!(screenshot_schema["additionalProperties"], false);
        assert!(screenshot_schema["properties"].get("display_id").is_some());
        assert!(
            screenshot_schema["properties"]
                .get("display_name")
                .is_some()
        );
        assert!(
            screenshot_schema["properties"]
                .get("display_index")
                .is_some()
        );
        assert!(
            screenshot_schema["properties"]
                .get("capture_all_displays")
                .is_none(),
            "capture_desktop must not advertise capture_all_displays to the model"
        );
        assert!(
            schema_accepts(
                screenshot_schema,
                &json!({"display_id": "", "display_name": "HDMI-A-1"})
            ),
            "capture_desktop must allow blank display_id sentinels with an active display_name"
        );
        assert!(
            !schema_accepts(screenshot_schema, &json!({"capture_all_displays": true})),
            "capture_desktop must reject the retired capture_all_displays selector"
        );
        assert!(
            !schema_accepts(
                screenshot_schema,
                &json!({"display_index": 0, "capture_all_displays": true})
            ),
            "capture_desktop must reject capture_all_displays even alongside a display selector"
        );
        assert!(
            screenshot["description"]
                .as_str()
                .is_some_and(|description| description.contains("main display"))
        );
        assert!(
            screenshot["description"]
                .as_str()
                .is_some_and(|description| {
                    description.contains("exactly one screen")
                        && !description.contains("capture_all_displays")
                })
        );
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
    fn activate_window_parser_ignores_host_default_zero_and_blank_values() {
        let target = parse_window_target(json!({
            "window_id": "",
            "pid": 0,
            "tty": "",
            "terminal_pid": 0,
            "terminal_command": "",
            "terminal_cwd": "",
            "app_id": "chromium.desktop",
            "wm_class": "",
            "title": ""
        }))
        .expect("app_id should remain a target");

        assert_eq!(target.app_id.as_deref(), Some("chromium.desktop"));
        assert_eq!(target.pid, None);
        assert_eq!(target.terminal_pid, None);
    }

    #[test]
    fn screenshot_parser_accepts_each_selector_shape() {
        let omitted = parse_screenshot_target(&json!({})).expect("omitted target is valid");
        assert!(omitted.window.is_none());
        assert!(omitted.display.is_none());

        let window = parse_screenshot_target(&json!({"window_id": "hwnd:0x1"}))
            .expect("window target is valid");
        assert_eq!(
            window.window.unwrap().window_id.as_deref(),
            Some("hwnd:0x1")
        );

        let display = parse_screenshot_target(&json!({"display_id": "kwin:HDMI-A-1"}))
            .expect("display target is valid");
        assert_eq!(
            display.display.unwrap().display_id.as_deref(),
            Some("kwin:HDMI-A-1")
        );
    }

    #[test]
    fn screenshot_parser_ignores_retired_all_displays_flag() {
        // capture_all_displays is no longer an agent-facing selector; the grouped
        // schema rejects it, and any stray value never resolves to a target.
        let display = parse_screenshot_target(
            &json!({"display_id": "kwin:HDMI-A-1", "capture_all_displays": true}),
        )
        .expect("a display selector still resolves regardless of stray fields");
        assert_eq!(
            display.display.unwrap().display_id.as_deref(),
            Some("kwin:HDMI-A-1")
        );

        let omitted = parse_screenshot_target(&json!({"capture_all_displays": true}))
            .expect("stray all-displays flag is not itself a selector");
        assert!(omitted.window.is_none());
        assert!(omitted.display.is_none());
    }

    #[test]
    fn screenshot_parser_rejects_mixed_selectors() {
        let error = parse_screenshot_target(&json!({
            "window_id": "kwin:{window}",
            "display_id": "kwin:eDP-1"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("exactly one capture selector"));
    }

    #[test]
    fn activate_window_validation_returns_tool_error() {
        let result =
            invalid_request_tool_error(parse_window_target(json!({})).unwrap_err().to_string())
                .unwrap();

        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["code"], "InvalidRequest");
        assert!(
            result["structuredContent"]["message"]
                .as_str()
                .expect("message")
                .contains("activate_window requires one of window_id")
        );
        assert!(
            result["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("activate_window requires one of window_id")
        );
    }

    #[test]
    fn capture_source_geometry_tool_error_includes_retry_suggestion() {
        let result = super::tool_error(
            "CaptureSourceGeometryMissing",
            "targeted screenshot requires capture source geometry",
        )
        .unwrap();

        assert_eq!(result["isError"], true);
        assert_eq!(
            result["structuredContent"]["code"],
            "CaptureSourceGeometryMissing"
        );
        assert_eq!(
            result["structuredContent"]["message"],
            "targeted screenshot requires capture source geometry"
        );
        assert!(
            result["structuredContent"]["suggestion"]
                .as_str()
                .expect("suggestion")
                .contains("snapshot_id")
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
        let result = tools_list_result(&ModelSessionInfo::default());

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

        let diagnostic = list_apps_error_diagnostic(&diagnostics)
            .expect("diagnostic from a failed list_apps response should mark it as an MCP error");

        assert_eq!(diagnostic.code, "AccessibilityUnavailable");
    }

    #[test]
    fn list_apps_error_diagnostic_ignores_session_env_repair_context() {
        let diagnostics = vec![DiagnosticEntry {
            code: "SessionEnvRepaired".to_string(),
            message: "Repaired missing Linux desktop session environment.".to_string(),
            details: None,
        }];

        assert!(list_apps_error_diagnostic(&diagnostics).is_none());
    }

    #[test]
    fn list_apps_error_diagnostic_keeps_errors_after_session_env_repair_context() {
        let diagnostics = vec![
            DiagnosticEntry {
                code: "SessionEnvRepaired".to_string(),
                message: "Repaired missing Linux desktop session environment.".to_string(),
                details: None,
            },
            DiagnosticEntry {
                code: "AccessibilityUnavailable".to_string(),
                message: "AT-SPI is unavailable".to_string(),
                details: None,
            },
        ];

        let diagnostic = list_apps_error_diagnostic(&diagnostics)
            .expect("fatal diagnostics after repair context should still mark list_apps as failed");

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
                displays: Vec::new(),
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
                display: None,
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
            agent_cursor: None,
        };

        let summary = snapshot_summary(&snapshot);
        assert!(summary.contains("Approve the KDE portal dialog"));
        assert!(
            summary.contains("timed out waiting for the RemoteDesktop portal session to start")
        );
    }

    #[test]
    fn snapshot_text_content_includes_element_readback_for_text_only_hosts() {
        let snapshot = AppStateSnapshot {
            snapshot_id: "snap-text".to_string(),
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
                displays: Vec::new(),
            },
            capabilities: available_capabilities(),
            focused_app: Some(FocusedApp {
                app_id: "brave-browser.desktop".to_string(),
                name: "Brave Browser".to_string(),
                pid: Some(1234),
                desktop_file_id: Some("brave-browser.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("Chromium".to_string()),
                window_title: Some("Certificate Manager".to_string()),
                display: None,
            }),
            capture: None,
            elements: vec![ElementNode {
                element_index: 7,
                parent_index: Some(3),
                role: "button".to_string(),
                name: Some("Imported from Linux".to_string()),
                description: Some("Local user certificates".to_string()),
                value: None,
                text: Some(ElementTextReadback {
                    character_count: 18,
                    caret_offset: None,
                    content: Some("Example certificate".to_string()),
                    content_suppressed: false,
                    truncated: false,
                    selections: Vec::new(),
                }),
                numeric_value: None,
                supports_editable_text: false,
                state_flags: vec!["enabled".to_string(), "visible".to_string()],
                semantic_actions: vec!["click".to_string()],
                bounds: Some(RectF {
                    x: 1.0,
                    y: 2.0,
                    width: 300.0,
                    height: 40.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                backend_ref: Some("atspi:/7".to_string()),
            }],
            diagnostics: Vec::new(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        };

        let content = snapshot_text_content(&snapshot);

        assert!(content.contains("Snapshot snap-text captured 1 elements"));
        assert!(content.contains("Focused app: name=Brave Browser app_id=brave-browser.desktop"));
        assert!(content.contains("Elements (1):"));
        assert!(content.contains("[7] role=button parent=3"));
        assert!(content.contains("name=\"Imported from Linux\""));
        assert!(content.contains("description=\"Local user certificates\""));
        assert!(content.contains("text=\"Example certificate\""));
        assert!(content.contains("states=enabled,visible"));
        assert!(content.contains("actions=click"));
        assert!(content.contains("backend_ref=\"atspi:/7\""));
    }

    #[test]
    fn summary_get_app_state_text_omits_verbose_elements_for_image_hosts() {
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot_with_verbose_element()),
        });
        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({"detail": "compact", "capture_screen": "never"}),
        )
        .unwrap();

        let text = result["content"][0]["text"].as_str().expect("text content");
        assert!(text.contains("Snapshot snap-compact captured 1 elements"));
        assert!(text.contains("Elements: 1 returned of 1 filtered, 1 total limit=200"));
        assert!(!text.contains("description=\"Verbose element description\""));
        assert!(!text.contains("backend_ref=\"atspi:/7\""));
        assert_eq!(result["structuredContent"]["detail"], "compact");
        assert!(
            result["structuredContent"]["elements"][0]
                .get("description")
                .is_none()
        );
        assert_eq!(
            result["structuredContent"]["elements"][0]["backend_ref"],
            "atspi:/7"
        );
    }

    #[test]
    fn get_app_state_defaults_to_summary_limited_element_view_for_mcp() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.elements = (0..APP_STATE_DEFAULT_ELEMENT_LIMIT + 3)
            .map(|index| test_element(index, "button", &format!("Element {index}")))
            .collect();
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({}),
        )
        .unwrap();

        let structured = &result["structuredContent"];
        assert_eq!(structured["detail"], "compact");
        assert_eq!(
            structured["element_count"],
            APP_STATE_DEFAULT_ELEMENT_LIMIT + 3
        );
        assert_eq!(
            structured["filtered_element_count"],
            APP_STATE_DEFAULT_ELEMENT_LIMIT + 3
        );
        assert_eq!(
            structured["elements_returned"],
            APP_STATE_DEFAULT_ELEMENT_LIMIT
        );
        assert_eq!(structured["element_limit"], APP_STATE_DEFAULT_ELEMENT_LIMIT);
        assert_eq!(
            structured["elements"].as_array().expect("elements").len(),
            APP_STATE_DEFAULT_ELEMENT_LIMIT
        );
        let text = result["content"][0]["text"].as_str().expect("text content");
        assert!(text.contains("Elements: 200 returned of 203 filtered, 203 total limit=200"));
    }

    #[test]
    fn get_app_state_filters_and_paginates_elements() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.elements = vec![
            test_element(0, "button", "Save"),
            test_element(1, "entry", "Search field"),
            test_element(2, "button", "Search submit"),
            test_element(3, "button", "Cancel"),
        ];
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({
                "element_query": "search",
                "element_offset": 1,
                "element_limit": 1
            }),
        )
        .unwrap();

        let structured = &result["structuredContent"];
        assert_eq!(structured["element_count"], 4);
        assert_eq!(structured["filtered_element_count"], 2);
        assert_eq!(structured["elements_returned"], 1);
        assert_eq!(structured["element_offset"], 1);
        assert_eq!(structured["element_limit"], 1);
        assert_eq!(structured["element_query"], "search");
        assert_eq!(structured["elements"][0]["element_index"], 2);
        assert_eq!(structured["elements"][0]["name"], "Search submit");
    }

    #[test]
    fn get_app_state_element_query_matches_advertised_element_fields() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.elements = vec![
            ElementNode {
                description: Some("description-token".to_string()),
                ..test_element(0, "button", "Description match")
            },
            ElementNode {
                value: Some("value-token".to_string()),
                ..test_element(1, "entry", "Value match")
            },
            ElementNode {
                text: Some(ElementTextReadback {
                    character_count: 10,
                    caret_offset: None,
                    content: Some("text-token".to_string()),
                    content_suppressed: false,
                    truncated: false,
                    selections: Vec::new(),
                }),
                ..test_element(2, "text", "Text match")
            },
            ElementNode {
                numeric_value: Some(ElementNumericValueReadback {
                    current: 42.0,
                    minimum: 0.0,
                    maximum: 100.0,
                    minimum_increment: 1.0,
                    text: Some("numeric-token".to_string()),
                }),
                ..test_element(3, "slider", "Numeric match")
            },
            ElementNode {
                state_flags: vec!["state-token".to_string()],
                ..test_element(4, "checkbox", "State match")
            },
            ElementNode {
                semantic_actions: vec!["action-token".to_string()],
                ..test_element(5, "button", "Action match")
            },
            test_element(6, "role-token", "Role match"),
        ];

        let cases = [
            ("DESCRIPTION-TOKEN", 0),
            ("value-token", 1),
            ("text-token", 2),
            ("numeric-token", 3),
            ("state-token", 4),
            ("action-token", 5),
            ("role-token", 6),
        ];

        for (query, expected_index) in cases {
            let service = FakeService::with_response(ServiceResponse::GetAppState {
                snapshot: Box::new(snapshot.clone()),
            });

            let result = handle_tool_call(
                &service,
                &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
                &ModelSessionInfo {
                    supports_images: Some(true),
                },
                "get_app_state",
                json!({"element_query": query}),
            )
            .unwrap();

            let structured = &result["structuredContent"];
            assert_eq!(structured["filtered_element_count"], 1, "query {query}");
            assert_eq!(structured["elements_returned"], 1, "query {query}");
            assert_eq!(
                structured["elements"][0]["element_index"], expected_index,
                "query {query}"
            );
        }
    }

    #[test]
    fn get_app_state_accepts_zero_element_limit_for_metadata_only_projection() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.elements = (0..3)
            .map(|index| test_element(index, "button", &format!("Element {index}")))
            .collect();
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({"element_limit": 0}),
        )
        .unwrap();

        let structured = &result["structuredContent"];
        assert_eq!(structured["element_count"], 3);
        assert_eq!(structured["filtered_element_count"], 3);
        assert_eq!(structured["elements_returned"], 0);
        assert_eq!(structured["element_limit"], 0);
        assert_eq!(
            structured["elements"].as_array().expect("elements").len(),
            0
        );
        let text = result["content"][0]["text"].as_str().expect("text content");
        assert!(text.contains("Elements: 0 returned of 3 filtered, 3 total limit=0"));
        assert!(!text.contains("\n- ["));
    }

    #[test]
    fn get_app_state_rejects_invalid_element_projection_arguments() {
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot_with_verbose_element()),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({"element_limit": APP_STATE_MAX_ELEMENT_LIMIT + 1}),
        )
        .unwrap();
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["code"], "InvalidRequest");
        assert!(
            result["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("get_app_state element_limit must be at most"))
        );
        assert!(
            service.take_requests().is_empty(),
            "invalid element_limit should not reach the service"
        );

        let long_query = "q".repeat(APP_STATE_MAX_ELEMENT_QUERY_CHARS + 1);
        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({"element_query": long_query}),
        )
        .unwrap();
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["code"], "InvalidRequest");
        assert!(
            result["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("get_app_state element_query must be at most"))
        );
        assert!(
            service.take_requests().is_empty(),
            "invalid element_query should not reach the service"
        );
    }

    #[test]
    fn full_get_app_state_element_limit_serializes_only_requested_elements() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.elements = (0..5)
            .map(|index| test_element(index, "button", &format!("Element {index}")))
            .collect();
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({
                "detail": "full",
                "element_offset": 2,
                "element_limit": 1
            }),
        )
        .unwrap();

        let structured = &result["structuredContent"];
        assert_eq!(structured["element_count"], 5);
        assert_eq!(structured["filtered_element_count"], 5);
        assert_eq!(structured["elements_returned"], 1);
        assert_eq!(structured["element_offset"], 2);
        assert_eq!(structured["element_limit"], 1);
        assert_eq!(
            structured["elements"].as_array().expect("elements").len(),
            1
        );
        assert_eq!(structured["elements"][0]["element_index"], 2);
        assert_eq!(structured["elements"][0]["name"], "Element 2");
    }

    #[test]
    fn full_get_app_state_preserves_snapshot_top_level_fields() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.elements = (0..3)
            .map(|index| test_element(index, "button", &format!("Element {index}")))
            .collect();
        let full = serde_json::to_value(&snapshot).expect("full snapshot serializes");
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({
                "detail": "full",
                "element_limit": 1
            }),
        )
        .unwrap();

        let structured = result["structuredContent"]
            .as_object()
            .expect("structured content object");
        for key in full.as_object().expect("full snapshot object").keys() {
            assert!(structured.contains_key(key), "missing full-shape key {key}");
        }
        assert_eq!(structured["element_count"], 3);
        assert_eq!(structured["filtered_element_count"], 3);
        assert_eq!(structured["elements_returned"], 1);
        assert!(
            !structured.contains_key("doctor_report"),
            "empty doctor_report must keep platform skip_serializing_if behavior"
        );
        assert!(
            !structured.contains_key("agent_cursor"),
            "empty agent_cursor must keep platform skip_serializing_if behavior"
        );
    }

    #[test]
    fn full_get_app_state_text_keeps_hard_element_line_cap() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.elements = (0..SNAPSHOT_TEXT_TEST_ELEMENT_COUNT)
            .map(|index| test_element(index, "button", &format!("Element {index}")))
            .collect();
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({
                "detail": "full",
                "element_limit": APP_STATE_DEFAULT_ELEMENT_LIMIT
            }),
        )
        .unwrap();

        let structured = &result["structuredContent"];
        assert_eq!(
            structured["elements_returned"],
            SNAPSHOT_TEXT_TEST_ELEMENT_COUNT
        );
        let text = result["content"][0]["text"].as_str().expect("text content");
        assert_eq!(text.matches("\n- [").count(), 120);
        assert!(text.contains("- ... 3 more elements omitted"));
    }

    fn capture_info_with_screenshot(path: &std::path::Path) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: None,
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: None,
            source_logical_rect: None,
            pixel_size: None,
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: Some(path.display().to_string()),
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }

    fn desktop_appshot_response(path: &std::path::Path) -> ServiceResponse {
        let image_ref = ContentRef {
            content_id: "content-1".into(),
            device_id: None,
            link_epoch: None,
            mime_type: "image/png".into(),
            filename: Some("appshot.png".into()),
            size_bytes: 3,
            sha256: "00".repeat(32),
            source: ContentSource::HostPrivateArtifact,
            expires_at_ms: Some(1_000),
            persistence: ContentPersistence::Temporary,
        };
        ServiceResponse::AppShotCapture {
            result: Box::new(AppShotCaptureResult {
                request_id: "request-1".into(),
                application: AppShotApplication {
                    name: "Fixture".into(),
                    app_id: Some("fixture.app".into()),
                    desktop_file_id: None,
                    pid: Some(42),
                    window_id: Some("window-1".into()),
                    window_title: Some("Fixture".into()),
                },
                image: AppShotImage {
                    path: path.display().to_string(),
                    mime_type: "image/png".into(),
                    size_bytes: 3,
                    dimensions: PixelSize {
                        width: 1,
                        height: 1,
                    },
                },
                ax_status: AppShotAccessibilityStatus::Available,
                ax_text: Some("Fixture".into()),
                capture_scope: CaptureScope::Window,
                capture_backend: CaptureBackendKind::PortalPipeWire,
                image_backend: Some(CaptureBackendKind::PortalPipeWire),
                display: None,
                diagnostics: Vec::new(),
                appshot: Some(Box::new(AppShotEnvelope {
                    appshot_id: "appshot-1".into(),
                    trigger: AppShotTrigger::Observe,
                    captured_at: Utc::now(),
                    consistency: AppShotConsistency::Stable,
                    capture: AppShotCapture::Desktop {
                        app_id: "fixture.app".into(),
                        window_id: "window-1".into(),
                        title: Some("Fixture".into()),
                        bounds: RectF {
                            x: 0.0,
                            y: 0.0,
                            width: 1.0,
                            height: 1.0,
                            space: CoordinateSpace::DesktopLogical,
                        },
                        semantic_projection: json!({
                            "elements": [{
                                "element_index": 0,
                                "role": "button",
                                "name": "Continue",
                                "bounds": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0}
                            }]
                        }),
                    },
                    image: image_ref,
                    action_snapshot: AppShotActionSnapshot {
                        snapshot_id: "snapshot-1".into(),
                        session_id: None,
                        subject_generation: None,
                    },
                    coverage: AppShotCoverage {
                        pixels_complete: true,
                        semantics_complete: true,
                        secure_regions_redacted: false,
                        projection_truncated: false,
                        total_semantic_nodes: Some(0),
                        projected_semantic_nodes: Some(0),
                    },
                    capability_profile_id: "capability-1".into(),
                    diagnostics: Vec::new(),
                })),
            }),
        }
    }

    #[test]
    fn desktop_observe_only_attaches_appshot_for_supported_models() {
        let image_file = std::env::temp_dir().join(format!(
            "sky-cua-desktop-appshot-delivery-{}.png",
            std::process::id()
        ));
        std::fs::write(&image_file, b"png").unwrap();

        for (supports_images, expected_content_len) in
            [(None, 1), (Some(false), 1), (Some(true), 2)]
        {
            let service = FakeService::with_response(desktop_appshot_response(&image_file));
            let result = handle_tool_call(
                &service,
                &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
                &ModelSessionInfo { supports_images },
                "desktop_observe_appshot",
                json!({}),
            )
            .unwrap();

            let content = result["content"].as_array().expect("content array");
            assert_eq!(content.len(), expected_content_len);
            assert_eq!(content[0]["type"], "text");
            assert_eq!(result["structuredContent"]["appshot_id"], "appshot-1");
            if supports_images == Some(true) {
                assert_eq!(content[1]["type"], "image");
            } else {
                let text = content[0]["text"].as_str().expect("text content");
                assert!(text.contains("Model-facing desktop accessibility projection"));
                assert!(text.contains("\"name\":\"Continue\""));
            }
        }

        std::fs::remove_file(image_file).unwrap();
    }

    #[test]
    fn desktop_observe_reports_image_attachment_failure() {
        let missing_image = std::env::temp_dir().join(format!(
            "sky-cua-missing-desktop-appshot-{}-{}.png",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let service = FakeService::with_response(desktop_appshot_response(&missing_image));
        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "desktop_observe_appshot",
            json!({}),
        )
        .unwrap();

        let content = result["content"].as_array().expect("content array");
        assert_eq!(content.len(), 1);
        assert!(
            content[0]["text"]
                .as_str()
                .expect("text content")
                .contains("Image attachment failed")
        );
        assert_eq!(
            result["structuredContent"]["diagnostics"][0]["code"],
            "AppShotImageAttachmentFailed"
        );
    }

    #[test]
    fn get_app_state_inline_delivery_attaches_image_block() {
        let screenshot_file = std::env::temp_dir().join(format!(
            "sky-cua-inline-delivery-{}.jpg",
            std::process::id()
        ));
        std::fs::write(&screenshot_file, b"fake-jpeg-bytes").unwrap();

        let mut snapshot = snapshot_with_verbose_element();
        snapshot.capture = Some(capture_info_with_screenshot(&screenshot_file));
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({"detail": "compact", "screenshot_delivery": "inline"}),
        )
        .unwrap();
        std::fs::remove_file(&screenshot_file).unwrap();

        let content = result["content"].as_array().expect("content array");
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["mimeType"], "image/jpeg");
        use base64::Engine as _;
        assert_eq!(
            content[1]["data"],
            base64::engine::general_purpose::STANDARD.encode(b"fake-jpeg-bytes")
        );
    }

    #[test]
    fn get_app_state_path_delivery_keeps_text_only_content() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.capture = Some(capture_info_with_screenshot(std::path::Path::new(
            "/nonexistent/snapshot.jpg",
        )));
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({"detail": "compact"}),
        )
        .unwrap();

        assert_eq!(
            result["content"].as_array().expect("content array").len(),
            1
        );
        assert_eq!(result["content"][0]["type"], "text");
    }

    #[test]
    fn get_app_state_inline_delivery_reports_unreadable_screenshot() {
        let mut snapshot = snapshot_with_verbose_element();
        snapshot.capture = Some(capture_info_with_screenshot(std::path::Path::new(
            "/nonexistent/snapshot.jpg",
        )));
        let service = FakeService::with_response(ServiceResponse::GetAppState {
            snapshot: Box::new(snapshot),
        });

        let result = handle_tool_call(
            &service,
            &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
            &ModelSessionInfo {
                supports_images: Some(true),
            },
            "get_app_state",
            json!({"detail": "compact", "screenshot_delivery": "inline"}),
        )
        .unwrap();

        let content = result["content"].as_array().expect("content array");
        assert_eq!(content.len(), 1);
        let text = content[0]["text"].as_str().expect("text content");
        assert!(text.contains("Inline screenshot delivery failed"));
        assert!(text.contains("capture.inspection_image_path"));
        assert!(!text.contains("read screenshot_path instead"));
    }

    #[test]
    fn action_summary_surfaces_portal_approval_guidance() {
        let outcome = ActionOutcome {
            success: false,
            message: "timed out waiting for the RemoteDesktop portal session to start".to_string(),
            code: "PortalApprovalPending".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
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
                displays: Vec::new(),
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
                display: None,
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
            agent_cursor: None,
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
            agent_cursor: None,
        };

        let summary = action_summary(&outcome);
        assert!(summary.contains("typed text via portal session"));
        assert!(
            summary.contains("Rebuilt the cached portal session after PipeWire capture failed.")
        );
        assert!(summary.contains("capture timed out on cached stream"));
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
                displays: Vec::new(),
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
                display: None,
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
            agent_cursor: None,
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
                displays: Vec::new(),
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
                display: None,
            }),
            capture: None,
            elements: Vec::new(),
            diagnostics: vec![
                DiagnosticEntry {
                    code: "CaptureBackendDowngraded".to_string(),
                    message:
                        "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback"
                            .to_string(),
                    details: Some(
                        "primary_backend=portal_pipe_wire image_backend=portal_screenshot"
                            .to_string(),
                    ),
                },
                DiagnosticEntry {
                    code: "DisplayTopologyInferred".to_string(),
                    message: "Display topology inferred from XRandR fallback.".to_string(),
                    details: Some("provider=xrandr".to_string()),
                },
                DiagnosticEntry {
                    code: "DisplayTopologyUnavailable".to_string(),
                    message: "Display topology is unavailable.".to_string(),
                    details: Some("kscreen-doctor timed out".to_string()),
                },
            ],
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        };

        let summary = snapshot_summary(&snapshot);
        assert!(summary.contains(
            "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback"
        ));
        assert!(summary.contains("image_backend=portal_screenshot"));
        assert!(summary.contains("Display topology inferred from XRandR fallback."));
        assert!(summary.contains("provider=xrandr"));
        assert!(summary.contains("Display topology is unavailable."));
        assert!(summary.contains("kscreen-doctor timed out"));
    }

    #[test]
    fn doctor_summary_mentions_display_topology_fallback() {
        let mut report = registry_doctor_report();
        report.display_topology = Some(DoctorDisplayTopologyReport {
            display_count: 2,
            selected_provider: Some("xrandr".to_string()),
            probes: Vec::new(),
            detail: "display topology discovered via xrandr fallback".to_string(),
        });

        let summary = super::doctor_summary(&report);

        assert!(summary.contains("DisplayTopologyInferred"));
        assert!(summary.contains("window-targeted screenshots"));
    }

    #[test]
    fn doctor_summary_does_not_label_x11_xrandr_as_wayland_fallback() {
        let mut report = registry_doctor_report();
        report.environment.session_kind = SessionKind::X11;
        report.display_topology = Some(DoctorDisplayTopologyReport {
            display_count: 1,
            selected_provider: Some("xrandr".to_string()),
            probes: Vec::new(),
            detail: "display topology discovered via xrandr".to_string(),
        });

        let summary = super::doctor_summary(&report);

        assert!(!summary.contains("DisplayTopologyInferred"));
        assert!(!summary.contains("fallback"));
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
            supported_scroll_directions: vec![ScrollDirection::Up, ScrollDirection::Down],
            drag: available(),
            type_text: available(),
            press_key: available(),
            set_value: available(),
        }
    }

    fn snapshot_with_verbose_element() -> AppStateSnapshot {
        AppStateSnapshot {
            snapshot_id: "snap-compact".to_string(),
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
                displays: Vec::new(),
            },
            capabilities: available_capabilities(),
            focused_app: Some(FocusedApp {
                app_id: "brave-browser.desktop".to_string(),
                name: "Brave Browser".to_string(),
                pid: Some(1234),
                desktop_file_id: Some("brave-browser.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("Chromium".to_string()),
                window_title: Some("Certificate Manager".to_string()),
                display: None,
            }),
            capture: None,
            elements: vec![ElementNode {
                element_index: 7,
                parent_index: Some(3),
                role: "button".to_string(),
                name: Some("Imported from Linux".to_string()),
                description: Some("Verbose element description".to_string()),
                value: None,
                text: Some(ElementTextReadback {
                    character_count: 18,
                    caret_offset: None,
                    content: Some("Example certificate".to_string()),
                    content_suppressed: false,
                    truncated: false,
                    selections: Vec::new(),
                }),
                numeric_value: None,
                supports_editable_text: false,
                state_flags: vec!["enabled".to_string(), "visible".to_string()],
                semantic_actions: vec!["click".to_string()],
                bounds: Some(RectF {
                    x: 1.0,
                    y: 2.0,
                    width: 300.0,
                    height: 40.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                backend_ref: Some("atspi:/7".to_string()),
            }],
            diagnostics: Vec::new(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        }
    }

    fn test_element(index: usize, role: &str, name: &str) -> ElementNode {
        ElementNode {
            element_index: index,
            parent_index: None,
            role: role.to_string(),
            name: Some(name.to_string()),
            description: Some(format!("{name} description")),
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: role == "entry",
            state_flags: vec!["enabled".to_string(), "visible".to_string()],
            semantic_actions: vec!["click".to_string()],
            bounds: Some(RectF {
                x: 1.0 + index as f64,
                y: 2.0,
                width: 100.0,
                height: 30.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            backend_ref: Some(format!("atspi:/{index}")),
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
                displays: Vec::new(),
            },
            checks: Vec::new(),
            readiness: DoctorReadiness {
                can_register_mcp_tools: true,
                can_build_accessibility_tree,
                can_capture_screen: true,
                can_send_input: true,
                can_list_windows: true,
                can_target_windows: true,
                can_inhibit_presence: false,
                can_unlock_session: false,
                recommended_next_step: "ready".to_string(),
                blockers: Vec::new(),
            },
            platform: None,
            display_topology: None,
            session_env: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
            session_presence: None,
        }
    }
    #[test]
    fn tools_list_registry_preserves_names_and_image_schema_gate() {
        let tools_value = build_tool_definitions(true, false);
        let tools = tools_value.as_array().expect("tools array");
        let names = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "doctor",
                "status",
                "list_resources",
                "observe",
                "capture_screen",
                "phone_accessibility_tree",
                "phone_notifications",
                "capture_desktop",
                "setup_desktop",
                "session_presence",
                "activate_window",
                "desktop_semantic",
                "browser_claim_tab",
                "browser_move_mouse",
                "phone_connection",
                "phone_pair_wireless",
                "phone_setup",
                "phone_app_force_stop",
                "desktop_toggle",
                "desktop_scroll",
                "browser_scroll",
                "desktop_pointer",
                "desktop_keyboard",
                "desktop_action",
                "desktop_launch_app",
                "desktop_set_value",
                "browser_open",
                "browser_navigate",
                "browser_input",
                "phone_pointer",
                "phone_keyboard",
                "phone_notification_action",
                "phone_notification_reply",
                "phone_app_action",
                "phone_app_install",
                "phone_content",
                "phone_clipboard",
                "phone_editor",
                "phone_camera",
                "phone_storage",
            ]
        );

        let observe = tools
            .iter()
            .find(|tool| tool["name"] == "observe")
            .expect("observe tool");
        assert!(
            observe["inputSchema"]["properties"]
                .get("capture_screen")
                .is_none()
        );
        assert!(
            observe["description"]
                .as_str()
                .is_some_and(|description| description.contains("canonical AppShot"))
        );

        let text_only_tools = build_tool_definitions(false, false);
        let text_only_observe = text_only_tools
            .as_array()
            .expect("tools array")
            .iter()
            .find(|tool| tool["name"] == "observe")
            .expect("observe tool");
        assert!(
            text_only_observe["inputSchema"]["properties"]
                .get("capture_screen")
                .is_none()
        );
        assert!(
            text_only_observe["description"]
                .as_str()
                .is_some_and(|description| description.contains("Browser requires tab_id"))
        );
    }

    #[test]
    fn registry_capture_screen_policy_respects_model_image_support() {
        let vision_model = ModelSessionInfo {
            supports_images: Some(true),
        };
        let text_only_model = ModelSessionInfo {
            supports_images: Some(false),
        };

        assert_eq!(
            effective_capture_screen(&json!({}), &vision_model),
            CaptureScreenMode::IfChanged
        );
        assert_eq!(
            effective_capture_screen(&json!({"capture_screen": "always"}), &vision_model),
            CaptureScreenMode::Always
        );
        assert_eq!(
            effective_capture_screen(&json!({"capture_screen": "always"}), &text_only_model),
            CaptureScreenMode::Never
        );
    }

    #[test]
    fn doctor_tool_maps_service_response_to_mcp_result() {
        let service = FakeService::with_response(ServiceResponse::Doctor {
            report: Box::new(registry_doctor_report()),
        });
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics load");

        let result = handle_tool_call(
            &service,
            &heuristics,
            &ModelSessionInfo::default(),
            "doctor",
            json!({}),
        )
        .unwrap();

        assert_eq!(service.take_requests(), vec![ServiceRequest::Doctor]);
        assert_eq!(result["content"][0]["text"], "ready");
        assert_eq!(
            result["structuredContent"]["readiness"]["can_send_input"],
            true
        );
        assert_eq!(result["isError"], false);
    }

    #[test]
    fn session_presence_tools_map_to_service_requests() {
        let service = FakeService::with_responses([
            ServiceResponse::SessionPresence {
                status: session_presence_status(true),
            },
            ServiceResponse::SessionPresence {
                status: session_presence_status(true),
            },
            ServiceResponse::SessionPresence {
                status: session_presence_status(false),
            },
            ServiceResponse::SessionPresence {
                status: session_presence_status(false),
            },
        ]);
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics load");
        let model = ModelSessionInfo::default();

        let hold = handle_tool_call(
            &service,
            &heuristics,
            &model,
            "hold_session",
            json!({"unlock": true, "inhibit_suspend": false}),
        )
        .unwrap();
        let unlock = handle_tool_call(
            &service,
            &heuristics,
            &model,
            "unlock_session",
            json!({"inhibit_lock": false}),
        )
        .unwrap();
        let release = handle_tool_call(
            &service,
            &heuristics,
            &model,
            "release_session",
            json!({"relock": true}),
        )
        .unwrap();
        let status = handle_tool_call(
            &service,
            &heuristics,
            &model,
            "session_presence_status",
            json!({}),
        )
        .unwrap();

        assert_eq!(hold["structuredContent"]["lock_inhibited"], true);
        assert_eq!(unlock["structuredContent"]["suspend_inhibited"], true);
        assert_eq!(release["structuredContent"]["lock_inhibited"], false);
        assert_eq!(status["isError"], false);
        assert!(
            hold["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("backend=fake")
        );

        assert_eq!(
            service.take_requests(),
            vec![
                ServiceRequest::SessionPresence {
                    action: SessionPresenceAction::Ensure(SessionPresenceIntent {
                        unlock: true,
                        inhibit_lock: true,
                        inhibit_suspend: false,
                    }),
                },
                ServiceRequest::SessionPresence {
                    action: SessionPresenceAction::Ensure(SessionPresenceIntent {
                        unlock: true,
                        inhibit_lock: false,
                        inhibit_suspend: true,
                    }),
                },
                ServiceRequest::SessionPresence {
                    action: SessionPresenceAction::Release { relock: true },
                },
                ServiceRequest::SessionPresence {
                    action: SessionPresenceAction::Status,
                },
            ]
        );
    }

    #[test]
    fn session_presence_tool_rejects_non_boolean_arguments() {
        let service = FakeService::default();
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics load");

        let result = handle_tool_call(
            &service,
            &heuristics,
            &ModelSessionInfo::default(),
            "hold_session",
            json!({"unlock": "yes"}),
        )
        .unwrap();

        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["code"], "InvalidRequest");
        assert!(service.take_requests().is_empty());
    }

    #[test]
    fn click_tool_maps_arguments_to_execute_action_request() {
        let service = FakeService::with_response(ServiceResponse::ExecuteAction {
            outcome: ActionOutcome {
                success: true,
                message: "clicked".to_string(),
                code: "ActionPerformed".to_string(),
                diagnostics: Vec::new(),
                agent_cursor: None,
            },
        });
        let heuristics = HeuristicsRegistry::load_from_repo().expect("heuristics load");

        let result = handle_tool_call(
            &service,
            &heuristics,
            &ModelSessionInfo::default(),
            "click",
            json!({"appshot_id": "appshot-1", "snapshot_id": "snap-1", "element_index": 3, "x": 12.5, "y": 42.0}),
        )
        .unwrap();

        assert_eq!(result["content"][0]["text"], "clicked");
        assert_eq!(result["isError"], false);
        let requests = service.take_requests();
        let [ServiceRequest::ExecuteAction { request }] = requests.as_slice() else {
            panic!("expected one ExecuteAction request: {requests:?}");
        };
        assert_eq!(request.action, ActionName::Click);
        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, Some(3));
        assert_eq!(request.arguments["element_index"], 3);
        assert_eq!(request.arguments["x"], 12.5);
        assert_eq!(request.arguments["y"], 42.0);
    }

    #[test]
    fn click_element_target_ignores_host_default_coordinates() {
        let request = captured_action_request(
            ActionName::Click,
            json!({"snapshot_id": "snap-1", "element_index": 3, "x": 0.0, "y": 0.0}),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, Some(3));
        assert_eq!(request.arguments["element_index"], 3);
        assert!(request.arguments.get("x").is_none());
        assert!(request.arguments.get("y").is_none());
    }

    #[test]
    fn first_click_element_target_ignores_host_default_coordinates() {
        let request = captured_action_request(
            ActionName::Click,
            json!({"snapshot_id": "snap-1", "element_index": 0, "x": 0.0, "y": 0.0}),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, Some(0));
        assert_eq!(request.arguments["element_index"], 0);
        assert!(request.arguments.get("x").is_none());
        assert!(request.arguments.get("y").is_none());
    }

    #[test]
    fn click_coordinates_ignore_host_default_element_index() {
        let request = captured_action_request(
            ActionName::Click,
            json!({"snapshot_id": "snap-1", "element_index": 0, "x": 12.5, "y": 42.0}),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, None);
        assert!(request.arguments.get("element_index").is_none());
        assert_eq!(request.arguments["x"], 12.5);
        assert_eq!(request.arguments["y"], 42.0);
    }

    #[test]
    fn secondary_action_coordinates_ignore_host_default_element_index() {
        let request = captured_action_request(
            ActionName::PerformSecondaryAction,
            json!({"snapshot_id": "snap-1", "element_index": 0, "x": 12.5, "y": 42.0}),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, None);
        assert!(request.arguments.get("element_index").is_none());
        assert_eq!(request.arguments["x"], 12.5);
        assert_eq!(request.arguments["y"], 42.0);
    }

    #[test]
    fn click_tool_ignores_host_default_element_index_without_snapshot() {
        let request = captured_action_request(
            ActionName::Click,
            json!({"snapshot_id": "", "element_index": 0, "x": 12.5, "y": 42.0}),
        );

        assert_eq!(request.snapshot_id, None);
        assert_eq!(request.element_index, None);
        assert!(request.arguments.get("element_index").is_none());
        assert_eq!(request.arguments["x"], 12.5);
        assert_eq!(request.arguments["y"], 42.0);
    }

    #[test]
    fn nonzero_element_index_without_snapshot_is_preserved_for_service_validation() {
        let request = captured_action_request(
            ActionName::Click,
            json!({"snapshot_id": "", "element_index": 3}),
        );

        assert_eq!(request.snapshot_id, None);
        assert_eq!(request.element_index, Some(3));
    }

    #[test]
    fn semantic_selector_ignores_host_default_element_index() {
        let request = captured_action_request(
            ActionName::ActivateElement,
            json!({"snapshot_id": "snap-1", "element_index": 0, "name": "Save", "role": "button"}),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, None);
        assert!(request.arguments.get("element_index").is_none());
        assert_eq!(request.arguments["name"], "Save");
        assert_eq!(request.arguments["role"], "button");
    }

    #[test]
    fn set_value_selector_ignores_host_default_element_index_but_preserves_value() {
        let request = captured_action_request(
            ActionName::SetValue,
            json!({"snapshot_id": "snap-1", "element_index": 0, "text": "Search", "value": "needle"}),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, None);
        assert!(request.arguments.get("element_index").is_none());
        assert_eq!(request.arguments["text"], "Search");
        assert_eq!(request.arguments["value"], "needle");
    }

    #[test]
    fn type_text_payload_does_not_make_element_index_look_like_host_default() {
        let request = captured_action_request(
            ActionName::TypeText,
            json!({"snapshot_id": "snap-1", "element_index": 0, "text": "hello"}),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, Some(0));
        assert_eq!(request.arguments["element_index"], 0);
        assert_eq!(request.arguments["text"], "hello");
    }

    #[test]
    fn drag_coordinates_ignore_host_default_element_indexes() {
        let request = captured_action_request(
            ActionName::Drag,
            json!({
                "snapshot_id": "snap-1",
                "element_index": 0,
                "from_x": 1.0,
                "from_y": 2.0,
                "to_element_index": 0,
                "to_x": 3.0,
                "to_y": 4.0
            }),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, None);
        assert!(request.arguments.get("element_index").is_none());
        assert!(request.arguments.get("to_element_index").is_none());
        assert_eq!(request.arguments["from_x"], 1.0);
        assert_eq!(request.arguments["from_y"], 2.0);
        assert_eq!(request.arguments["to_x"], 3.0);
        assert_eq!(request.arguments["to_y"], 4.0);
    }

    #[test]
    fn drag_coordinates_ignore_host_default_from_alias_when_xy_is_present() {
        let request = captured_action_request(
            ActionName::Drag,
            json!({
                "snapshot_id": "snap-1",
                "element_index": 0,
                "x": 120.0,
                "y": 200.0,
                "from_x": 0.0,
                "from_y": 0.0,
                "to_x": 3.0,
                "to_y": 4.0
            }),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, None);
        assert!(request.arguments.get("element_index").is_none());
        assert!(request.arguments.get("from_x").is_none());
        assert!(request.arguments.get("from_y").is_none());
        assert_eq!(request.arguments["x"], 120.0);
        assert_eq!(request.arguments["y"], 200.0);
        assert_eq!(request.arguments["to_x"], 3.0);
        assert_eq!(request.arguments["to_y"], 4.0);
    }

    #[test]
    fn drag_coordinates_preserve_nonzero_element_indexes() {
        let request = captured_action_request(
            ActionName::Drag,
            json!({
                "snapshot_id": "snap-1",
                "element_index": 3,
                "from_x": 1.0,
                "from_y": 2.0,
                "to_element_index": 4,
                "to_x": 3.0,
                "to_y": 4.0
            }),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, Some(3));
        assert_eq!(request.arguments["element_index"], 3);
        assert_eq!(request.arguments["to_element_index"], 4);
        assert_eq!(request.arguments["from_x"], 1.0);
        assert_eq!(request.arguments["from_y"], 2.0);
        assert_eq!(request.arguments["to_x"], 3.0);
        assert_eq!(request.arguments["to_y"], 4.0);
    }

    #[test]
    fn drag_element_targets_ignore_host_default_coordinates() {
        let request = captured_action_request(
            ActionName::Drag,
            json!({
                "snapshot_id": "snap-1",
                "element_index": 3,
                "x": 0.0,
                "y": 0.0,
                "from_x": 0.0,
                "from_y": 0.0,
                "to_element_index": 4,
                "to_x": 0.0,
                "to_y": 0.0
            }),
        );

        assert_eq!(request.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(request.element_index, Some(3));
        assert_eq!(request.arguments["element_index"], 3);
        assert_eq!(request.arguments["to_element_index"], 4);
        assert!(request.arguments.get("x").is_none());
        assert!(request.arguments.get("y").is_none());
        assert!(request.arguments.get("from_x").is_none());
        assert!(request.arguments.get("from_y").is_none());
        assert!(request.arguments.get("to_x").is_none());
        assert!(request.arguments.get("to_y").is_none());
    }

    fn registry_doctor_report() -> DoctorReport {
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
                displays: Vec::new(),
            },
            checks: vec![DoctorCheck {
                name: "service".to_string(),
                ok: true,
                detail: "ready".to_string(),
            }],
            readiness: DoctorReadiness {
                can_register_mcp_tools: true,
                can_build_accessibility_tree: true,
                can_capture_screen: true,
                can_send_input: true,
                can_list_windows: true,
                can_target_windows: true,
                can_inhibit_presence: false,
                can_unlock_session: false,
                recommended_next_step: "ready".to_string(),
                blockers: Vec::new(),
            },
            platform: None,
            display_topology: None,
            session_env: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
            session_presence: None,
        }
    }

    fn session_presence_status(held: bool) -> SessionPresenceStatus {
        SessionPresenceStatus {
            backend: "fake".to_string(),
            supported: true,
            unlock_supported: true,
            locked: Some(false),
            lock_inhibited: held,
            suspend_inhibited: held,
            detail: if held {
                "fake session presence held".to_string()
            } else {
                "fake session presence released".to_string()
            },
        }
    }
}
