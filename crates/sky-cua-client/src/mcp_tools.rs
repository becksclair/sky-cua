use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use sky_cua_platform::model::{
    ActionName, ActionRequest, AppInfo, AppSelector, AppStateSnapshot, CaptureScreenMode,
    DiagnosticEntry, ServiceRequest, ServiceResponse, WindowInfo,
};
use std::fmt::Write as _;

use crate::heuristics::HeuristicsRegistry;
use crate::mcp_server::ModelSessionInfo;
use crate::output_shapes::{
    AppStateDetail, compact_snapshot, compact_snapshot_text_content, informational_runtime_summary,
    list_apps_error_diagnostic, portal_approval_summary, setup_accessibility_is_error,
    setup_window_targeting_is_error, snapshot_text_content,
};
use crate::service_launcher::ServiceClient;

mod annotations;
mod browser;
mod definitions;

pub(crate) use definitions::tools_list_result;
#[cfg(test)]
pub(crate) use definitions::{build_tool_definitions, tool_definitions};
#[cfg(test)]
mod browser_tests;

pub(crate) trait McpService {
    fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse>;
}

impl McpService for ServiceClient {
    fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse> {
        ServiceClient::call(self, request)
    }
}

pub(crate) fn handle_tool_call(
    service: &impl McpService,
    heuristics: &HeuristicsRegistry,
    model: &ModelSessionInfo,
    tool_name: &str,
    arguments: Value,
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
            let capture_screen = effective_capture_screen(&arguments, model);
            let screenshot_delivery = parse_screenshot_delivery(&arguments);
            match service.call(&ServiceRequest::GetAppState {
                selector,
                capture_screen,
            })? {
                ServiceResponse::GetAppState { mut snapshot } => {
                    enrich_snapshot(heuristics, &mut snapshot);
                    if !model.can_receive_images() {
                        snapshot.diagnostics.push(DiagnosticEntry {
                            code: "ModelImageInputUnsupported".to_string(),
                            message: "Screen capture was disabled because the active model does not support image input.".to_string(),
                            details: None,
                        });
                    }
                    let structured_content = match detail {
                        AppStateDetail::Full => serde_json::to_value(&snapshot)?,
                        AppStateDetail::Compact => compact_snapshot(&snapshot),
                    };
                    let mut text_content =
                        if detail == AppStateDetail::Compact && model.can_receive_images() {
                            compact_snapshot_text_content(&snapshot)
                        } else {
                            snapshot_text_content(&snapshot)
                        };

                    let mut content = Vec::with_capacity(2);
                    if screenshot_delivery == ScreenshotDelivery::Inline
                        && model.can_receive_images()
                    {
                        match inline_screenshot_block(&snapshot) {
                            Some(Ok(image_block)) => content.push(image_block),
                            Some(Err(message)) => {
                                text_content.push_str(
                                    "\nInline screenshot delivery failed; read screenshot_path instead: ",
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
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for get_app_state: {other:?}")),
            }
        }
        name if browser::is_browser_tool(name) => {
            browser::handle_tool_call(service, name, arguments, model)
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
    service: &impl McpService,
    action: ActionName,
    mut arguments: Value,
) -> Result<Value> {
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

pub(crate) fn parse_window_target(
    arguments: Value,
) -> Result<sky_cua_platform::model::WindowTarget> {
    sky_cua_platform::model::WindowTarget::from_argument_fields(&arguments)
        .context("invalid activate_window target arguments")?
        .ok_or_else(|| {
            anyhow!(
            "activate_window requires one of window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd"
            )
        })
}

pub(crate) fn action_summary(outcome: &sky_cua_platform::model::ActionOutcome) -> String {
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

pub(crate) fn invalid_request_tool_error(message: impl Into<String>) -> Result<Value> {
    tool_error("InvalidRequest", message)
}

fn doctor_summary(report: &sky_cua_platform::model::DoctorReport) -> String {
    let mut summary = report.readiness.recommended_next_step.clone();
    if report
        .session_env
        .as_ref()
        .is_some_and(sky_cua_platform::model::DoctorSessionEnvReport::changed)
    {
        summary.push_str(" SessionEnvRepaired: detached desktop session environment was repaired.");
    }

    if let Some(input) = &report.input {
        push_input_diagnostics(input, &mut summary);
    }

    summary
}

fn push_input_diagnostics(
    input: &sky_cua_platform::model::DoctorInputReport,
    summary: &mut String,
) {
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

pub(crate) fn parse_app_state_detail(arguments: &Value) -> AppStateDetail {
    match arguments.get("detail").and_then(Value::as_str) {
        Some("compact") => AppStateDetail::Compact,
        _ => AppStateDetail::Full,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ScreenshotDelivery {
    /// Reference the capture by `screenshot_path` only (token-lean default).
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

/// Build an MCP image content block from the snapshot's persisted screenshot.
/// Returns None when the snapshot has no capture, and Err text when the file
/// cannot be read back.
fn inline_screenshot_block(
    snapshot: &sky_cua_platform::model::AppStateSnapshot,
) -> Option<Result<Value, String>> {
    let path = snapshot.capture.as_ref()?.screenshot_path.as_deref()?;
    let mime_type = match std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
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

    use serde_json::json;
    use sky_cua_platform::model::{
        AccessibilitySetupReport, ActionName, ActionOutcome, ActionRequest, AgentCursorPoint,
        AgentCursorState, AppInfo, AppStateSnapshot, CaptureBackendKind, CaptureInfo,
        CaptureScreenMode, CoordinateSpace, DiagnosticEntry, DoctorCheck, DoctorReadiness,
        DoctorReport, ElementNode, ElementTextReadback, EnvironmentInfo, FocusedApp,
        InputBackendKind, PortalCapabilities, RectF, SemanticBackendKind, ServiceRequest,
        ServiceResponse, SessionKind, SetupCommandReport, ToolAvailability, ToolCapabilities,
        WindowTargetingSetupReport,
    };

    use crate::heuristics::HeuristicsRegistry;
    use crate::mcp_server::ModelSessionInfo;

    use crate::output_shapes::{
        AppStateDetail, compact_element, compact_snapshot, list_apps_error_diagnostic,
        setup_accessibility_is_error, setup_window_targeting_is_error, snapshot_summary,
        snapshot_text_content,
    };

    use super::{
        McpService, action_summary, build_tool_definitions, effective_capture_screen,
        handle_action_call, handle_tool_call, invalid_request_tool_error, list_apps_summary,
        parse_app_selector, parse_app_state_detail, parse_window_target, tool_definitions,
        tools_list_result,
    };

    #[derive(Default)]
    struct FakeService {
        requests: RefCell<Vec<ServiceRequest>>,
        responses: RefCell<VecDeque<ServiceResponse>>,
    }

    impl FakeService {
        fn with_response(response: ServiceResponse) -> Self {
            Self {
                requests: RefCell::new(Vec::new()),
                responses: RefCell::new(VecDeque::from([response])),
            }
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

        handle_action_call(&service, action, arguments).unwrap();

        let mut requests = service.take_requests();
        assert_eq!(requests.len(), 1, "expected one ExecuteAction request");
        match requests.remove(0) {
            ServiceRequest::ExecuteAction { request } => *request,
            other => panic!("expected one ExecuteAction request: {other:?}"),
        }
    }

    #[test]
    fn parses_compact_app_state_detail() {
        assert_eq!(
            parse_app_state_detail(&json!({"detail": "compact"})),
            AppStateDetail::Compact
        );
        assert_eq!(
            parse_app_state_detail(&json!({"detail": "full"})),
            AppStateDetail::Full
        );
        assert_eq!(parse_app_state_detail(&json!({})), AppStateDetail::Full);
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
    fn compact_element_drops_verbose_description_but_keeps_backend_ref() {
        let compact = compact_element(&ElementNode {
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

        assert_eq!(compact["element_index"], 7);
        assert_eq!(compact["role"], "text");
        assert!(compact.get("description").is_none());
        assert_eq!(compact["value"], "query");
        assert_eq!(compact["text"]["content"], "query");
        assert_eq!(compact["supports_editable_text"], true);
        assert_eq!(compact["backend_ref"], "opaque-backend-ref");
        assert_eq!(compact["semantic_actions"][0], "set_value");
    }

    #[test]
    fn compact_snapshot_includes_doctor_report_and_agent_cursor() {
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
                can_inhibit_presence: false,
                can_unlock_session: false,
                recommended_next_step: "Ready".to_string(),
                blockers: Vec::new(),
            },
            platform: None,
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
                supported_scroll_directions: vec![
                    sky_cua_platform::model::ScrollDirection::Up,
                    sky_cua_platform::model::ScrollDirection::Down,
                ],
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
        let compact = compact_snapshot(&snapshot);
        assert!(compact.get("doctor_report").is_some());
        assert_eq!(
            compact["doctor_report"]["readiness"]["can_build_accessibility_tree"],
            true
        );
        assert_eq!(compact["agent_cursor"]["sequence"], 7);
        assert_eq!(
            compact["agent_cursor"]["model_point"]["coordinate_space"],
            "stream_pixels"
        );
    }

    #[test]
    fn action_tool_schemas_are_strict_and_snapshot_scoped_where_needed() {
        let tools = tool_definitions(&ModelSessionInfo::default());
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
    fn compact_get_app_state_text_omits_verbose_elements_for_image_hosts() {
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
        assert!(text.contains("Elements: 1 total."));
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

    fn capture_info_with_screenshot(path: &std::path::Path) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            coordinate_space: None,
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: None,
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
            agent_cursor: None,
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
            supported_scroll_directions: vec![
                sky_cua_platform::model::ScrollDirection::Up,
                sky_cua_platform::model::ScrollDirection::Down,
            ],
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
                can_inhibit_presence: false,
                can_unlock_session: false,
                recommended_next_step: "ready".to_string(),
                blockers: Vec::new(),
            },
            platform: None,
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
        let tools_value = build_tool_definitions(true);
        let tools = tools_value.as_array().expect("tools array");
        let names = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "doctor",
                "setup_accessibility",
                "setup_window_targeting",
                "list_apps",
                "list_windows",
                "focused_window",
                "activate_window",
                "get_app_state",
                "focus_element",
                "activate_element",
                "select_element",
                "expand_element",
                "collapse_element",
                "toggle_element",
                "click",
                "perform_action",
                "perform_secondary_action",
                "scroll",
                "drag",
                "type_text",
                "press_key",
                "set_value",
                "browser_status",
                "browser_list_tabs",
                "browser_open",
                "browser_claim_tab",
                "browser_move_mouse",
                "browser_navigate",
                "browser_snapshot",
                "browser_screenshot",
                "browser_click",
                "browser_type_text",
                "browser_press_key",
                "browser_scroll",
            ]
        );

        let get_app_state = tools
            .iter()
            .find(|tool| tool["name"] == "get_app_state")
            .expect("get_app_state tool");
        assert!(
            get_app_state["inputSchema"]["properties"]
                .get("capture_screen")
                .is_some()
        );

        let text_only_tools = build_tool_definitions(false);
        let text_only_get_app_state = text_only_tools
            .as_array()
            .expect("tools array")
            .iter()
            .find(|tool| tool["name"] == "get_app_state")
            .expect("get_app_state tool");
        assert!(
            text_only_get_app_state["inputSchema"]["properties"]
                .get("capture_screen")
                .is_none()
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
            json!({"snapshot_id": "snap-1", "element_index": 3, "x": 12.5, "y": 42.0}),
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
            session_env: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
            session_presence: None,
        }
    }
}
