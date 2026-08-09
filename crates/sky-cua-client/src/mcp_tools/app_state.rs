use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use sky_cua_platform::model::{
    AppShotCapture, AppShotCaptureFlags, AppShotEnvelope, DiagnosticEntry, ServiceRequest,
    ServiceResponse, WindowTarget,
};

use crate::app_state::{
    APP_STATE_DEFAULT_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_LIMIT,
    APP_STATE_MAX_ELEMENT_QUERY_CHARS, AppStateDetail, AppStateElementOptions,
};
use crate::heuristics::HeuristicsRegistry;
use crate::mcp_server::ModelSessionInfo;
use crate::output_shapes::{
    full_snapshot_with_element_selection, select_app_state_elements,
    snapshot_text_content_with_element_options, snapshot_text_content_with_element_selection,
    summary_snapshot_with_element_selection, text_app_state_element_selection,
};

use super::semantic_text::append_appshot_semantics;
use super::{
    McpService, ScreenshotDelivery, effective_capture_screen, enrich_snapshot,
    inline_screenshot_block, invalid_request_tool_error, parse_app_selector,
    parse_optional_string_argument, parse_optional_usize, parse_screenshot_delivery, tool_error,
};

pub(super) fn handle_get_app_state(
    service: &impl McpService,
    heuristics: &HeuristicsRegistry,
    arguments: Value,
    model: &ModelSessionInfo,
) -> Result<Value> {
    let selector = parse_app_selector(&arguments);
    let detail = match parse_app_state_detail(&arguments) {
        Ok(detail) => detail,
        Err(error) => return invalid_request_tool_error(error.to_string()),
    };
    let element_options = match parse_app_state_element_options(&arguments) {
        Ok(options) => options,
        Err(error) => return invalid_request_tool_error(error.to_string()),
    };
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
            let default_element_limit =
                (detail == AppStateDetail::Compact).then_some(APP_STATE_DEFAULT_ELEMENT_LIMIT);
            let element_selection = (detail == AppStateDetail::Compact
                || element_options.constrains_elements())
            .then(|| select_app_state_elements(&snapshot, &element_options, default_element_limit));
            let structured_content = match detail {
                AppStateDetail::Full => match &element_selection {
                    Some(selection) => full_snapshot_with_element_selection(&snapshot, selection)?,
                    None => serde_json::to_value(&snapshot)?,
                },
                AppStateDetail::Compact => summary_snapshot_with_element_selection(
                    &snapshot,
                    element_selection
                        .as_ref()
                        .expect("compact detail always selects projected elements"),
                ),
            };
            let include_text_elements =
                !(detail == AppStateDetail::Compact && model.can_receive_images());
            let mut text_content = match (&element_selection, include_text_elements) {
                (Some(selection), false) => {
                    snapshot_text_content_with_element_selection(&snapshot, false, selection)
                }
                (Some(selection), true) => {
                    let text_selection =
                        text_app_state_element_selection(selection, &element_options);
                    snapshot_text_content_with_element_selection(&snapshot, true, &text_selection)
                }
                (None, include_elements) => snapshot_text_content_with_element_options(
                    &snapshot,
                    include_elements,
                    &element_options,
                ),
            };

            let mut content = Vec::with_capacity(2);
            if screenshot_delivery == ScreenshotDelivery::Inline && model.can_receive_images() {
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
        other => Err(anyhow!("unexpected response for get_app_state: {other:?}")),
    }
}

/// Canonical desktop `observe` path. It calls the service-owned exact-window
/// producer directly, so every MCP host receives the same AppShot without an
/// AppServer or host-specific pre-turn hook.
pub(super) fn handle_desktop_observe_appshot(
    service: &impl McpService,
    arguments: Value,
    model: &ModelSessionInfo,
) -> Result<Value> {
    if arguments.get("desktop_file_id").is_some_and(|value| {
        !value.is_null() && value.as_str().is_none_or(|value| !value.is_empty())
    }) {
        return invalid_request_tool_error(
            "desktop observe AppShots require an exact window-resolvable selector; desktop_file_id is not supported"
                .to_string(),
        );
    }
    let element_options = match parse_app_state_element_options(&arguments) {
        Ok(options) => options,
        Err(error) => return invalid_request_tool_error(error.to_string()),
    };
    let target = WindowTarget {
        app_id: arguments
            .get("app_id")
            .and_then(Value::as_str)
            .and_then(super::optional_non_empty_string),
        title: arguments
            .get("window_title")
            .and_then(Value::as_str)
            .and_then(super::optional_non_empty_string),
        wm_class: arguments
            .get("name")
            .and_then(Value::as_str)
            .and_then(super::optional_non_empty_string),
        ..Default::default()
    };
    let target = (target != WindowTarget::default()).then_some(target);
    let request_id = format!("observe-{}", uuid::Uuid::new_v4());
    match service.call(&ServiceRequest::AppShotCapture {
        request_id,
        frontmost: target.is_none(),
        target,
        flags: AppShotCaptureFlags {
            include_ax_text: true,
        },
    })? {
        ServiceResponse::AppShotCapture { result } => {
            let mut appshot = result.appshot.map(|value| *value).ok_or_else(|| {
                anyhow!("desktop AppShot response omitted its canonical envelope")
            })?;
            project_desktop_appshot(&mut appshot, &element_options);
            let mut text = format!(
                "Desktop AppShot {} for window {}; consistency={:?}",
                appshot.appshot_id,
                desktop_window_id(&appshot).unwrap_or("unknown"),
                appshot.consistency
            );
            if !model.can_receive_images() {
                append_appshot_semantics(&mut text, &appshot);
            }
            let mut image_content = None;
            if model.can_receive_images() {
                match std::fs::read(&result.image.path) {
                    Ok(bytes) => {
                        use base64::Engine as _;
                        image_content = Some(json!({
                            "type": "image",
                            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                            "mimeType": result.image.mime_type,
                        }));
                    }
                    Err(error) => {
                        let message = format!("{}: {error}", result.image.path);
                        appshot.diagnostics.push(DiagnosticEntry {
                            code: "AppShotImageAttachmentFailed".to_string(),
                            message: message.clone(),
                            details: None,
                        });
                        text.push_str("\nImage attachment failed: ");
                        text.push_str(&message);
                    }
                }
            }
            let structured_content = serde_json::to_value(&appshot)?;
            let mut content = vec![json!({
                "type": "text",
                "text": text
            })];
            content.extend(image_content);
            Ok(json!({
                "content": content,
                "structuredContent": structured_content,
                "isError": false
            }))
        }
        ServiceResponse::Error { code, message, .. } => tool_error(code, message),
        other => Err(anyhow!(
            "unexpected response for desktop observe AppShot: {other:?}"
        )),
    }
}

fn desktop_window_id(appshot: &AppShotEnvelope) -> Option<&str> {
    match &appshot.capture {
        AppShotCapture::Desktop { window_id, .. } => Some(window_id),
        _ => None,
    }
}

fn project_desktop_appshot(appshot: &mut AppShotEnvelope, options: &AppStateElementOptions) {
    let AppShotCapture::Desktop {
        semantic_projection,
        ..
    } = &mut appshot.capture
    else {
        return;
    };
    let Some(elements) = semantic_projection
        .get_mut("elements")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let total = elements.len();
    let query = options
        .element_query
        .as_deref()
        .map(str::to_ascii_lowercase);
    let mut projected: Vec<Value> = elements
        .drain(..)
        .filter(|element| {
            query
                .as_ref()
                .is_none_or(|query| element.to_string().to_ascii_lowercase().contains(query))
        })
        .skip(options.element_offset)
        .take(
            options
                .element_limit
                .unwrap_or(APP_STATE_DEFAULT_ELEMENT_LIMIT),
        )
        .collect();
    let projected_len = projected.len();
    elements.append(&mut projected);
    appshot.coverage.total_semantic_nodes = Some(total as u64);
    appshot.coverage.projected_semantic_nodes = Some(projected_len as u64);
    appshot.coverage.projection_truncated = projected_len < total;
}

pub(super) fn parse_app_state_detail(arguments: &Value) -> Result<AppStateDetail> {
    match arguments.get("detail") {
        None | Some(Value::Null) => Ok(AppStateDetail::Compact),
        Some(Value::String(value)) if value == "full" => Ok(AppStateDetail::Full),
        Some(Value::String(value)) if value == "compact" => Ok(AppStateDetail::Compact),
        Some(Value::String(_)) => Err(anyhow!(
            "get_app_state detail must be \"full\" or \"compact\""
        )),
        Some(_) => Err(anyhow!("get_app_state detail must be a string")),
    }
}

fn parse_app_state_element_options(arguments: &Value) -> Result<AppStateElementOptions> {
    let element_limit =
        parse_optional_usize(arguments, "element_limit", "get_app_state element_limit")?;
    if element_limit.is_some_and(|limit| limit > APP_STATE_MAX_ELEMENT_LIMIT) {
        return Err(anyhow!(
            "get_app_state element_limit must be at most {APP_STATE_MAX_ELEMENT_LIMIT}"
        ));
    }
    let element_query =
        parse_optional_string_argument(arguments, "element_query", "get_app_state element_query")?;
    if element_query
        .as_ref()
        .is_some_and(|query| query.chars().count() > APP_STATE_MAX_ELEMENT_QUERY_CHARS)
    {
        return Err(anyhow!(
            "get_app_state element_query must be at most {APP_STATE_MAX_ELEMENT_QUERY_CHARS} characters"
        ));
    }
    Ok(AppStateElementOptions {
        element_offset: parse_optional_usize(
            arguments,
            "element_offset",
            "get_app_state element_offset",
        )?
        .unwrap_or(0),
        element_limit,
        element_query,
    })
}
