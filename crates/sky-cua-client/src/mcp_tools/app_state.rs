use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use sky_cua_platform::model::{DiagnosticEntry, ServiceRequest, ServiceResponse};

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
