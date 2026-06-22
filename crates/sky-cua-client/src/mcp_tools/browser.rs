use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use sky_cua_platform::model::{
    BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT, BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT, BrowserRequest,
    BrowserResponse, ServiceRequest, ServiceResponse,
};

mod args;
mod response;

#[cfg(test)]
pub(super) use args::BrowserTabTextFilter;
pub(super) use args::{
    parse_browser_open_url, parse_browser_point, parse_browser_scroll, parse_browser_tab_id,
    parse_browser_target, parse_required_browser_url, parse_required_literal_string,
    parse_required_string,
};
use args::{parse_browser_snapshot_options, parse_browser_tab_filter, parse_optional_bool};
#[cfg(test)]
pub(super) use response::browser_list_tabs_summary;
#[cfg(test)]
pub(super) use response::browser_snapshot_summary;
use response::{
    browser_action_result, browser_claim_tab_is_error, browser_claim_tab_summary,
    browser_eval_result, browser_list_tabs_is_error, browser_list_tabs_structured_response,
    browser_list_tabs_summary_with_matches, browser_move_mouse_is_error,
    browser_move_mouse_summary, browser_navigate_result, browser_open_is_error,
    browser_screenshot_result, browser_snapshot_result, browser_snapshot_structured_response,
    browser_tab_match_indexes,
};
pub(super) use response::{browser_open_summary, browser_status_summary};
#[cfg(test)]
pub(super) use sky_cua_platform::model::BROWSER_EVAL_ENV;
pub(super) use sky_cua_platform::model::browser_eval_enabled;

use super::{McpService, invalid_request_tool_error, tool_error};

pub(super) fn is_browser_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "browser_status"
            | "browser_list_tabs"
            | "browser_open"
            | "browser_claim_tab"
            | "browser_move_mouse"
            | "browser_navigate"
            | "browser_snapshot"
            | "browser_screenshot"
            | "browser_click"
            | "browser_type_text"
            | "browser_press_key"
            | "browser_scroll"
            | "browser_eval"
    )
}

pub(super) fn handle_tool_call(
    service: &impl McpService,
    tool_name: &str,
    arguments: Value,
    model: &crate::mcp_server::ModelSessionInfo,
    browser_eval_enabled_policy: Option<bool>,
) -> Result<Value> {
    match tool_name {
        "browser_status" => match service.call(&browser_service_request(BrowserRequest::Status))? {
            ServiceResponse::Browser {
                response: BrowserResponse::Status { report },
            } => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": browser_status_summary(&report)
                }],
                "structuredContent": report,
                "isError": false
            })),
            ServiceResponse::Error { code, message } => tool_error(code, message),
            other => Err(anyhow!("unexpected response for browser_status: {other:?}")),
        },
        "browser_list_tabs" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let filter = match parse_browser_tab_filter(&arguments) {
                Ok(filter) => filter,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::ListTabs {
                target,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::ListTabs { response },
                } => {
                    let is_error = browser_list_tabs_is_error(&response);
                    let matching_tab_indexes = browser_tab_match_indexes(&response, &filter);
                    let text = browser_list_tabs_summary_with_matches(
                        &response,
                        &filter,
                        matching_tab_indexes.as_deref(),
                    );
                    let structured_response =
                        browser_list_tabs_structured_response(response, matching_tab_indexes);
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": text
                        }],
                        "structuredContent": structured_response,
                        "isError": is_error
                    }))
                }
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_list_tabs: {other:?}"
                )),
            }
        }
        "browser_open" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let url = match parse_browser_open_url(&arguments) {
                Ok(url) => url,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::Open {
                target,
                url,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Open { response },
                } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": browser_open_summary(&response)
                    }],
                    "structuredContent": response,
                    "isError": browser_open_is_error(&response)
                })),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for browser_open: {other:?}")),
            }
        }
        "browser_claim_tab" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::ClaimTab {
                target,
                tab_id,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::ClaimTab { response },
                } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": browser_claim_tab_summary(&response)
                    }],
                    "structuredContent": response,
                    "isError": browser_claim_tab_is_error(&response)
                })),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_claim_tab: {other:?}"
                )),
            }
        }
        "browser_move_mouse" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let (x, y) = match parse_browser_point(&arguments, "browser_move_mouse") {
                Ok(point) => point,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let wait_for_arrival = match parse_optional_bool(&arguments, "wait_for_arrival", true) {
                Ok(wait_for_arrival) => wait_for_arrival,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::MoveMouse {
                target,
                tab_id,
                x,
                y,
                wait_for_arrival,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::MoveMouse { response },
                } => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": browser_move_mouse_summary(&response)
                    }],
                    "structuredContent": response,
                    "isError": browser_move_mouse_is_error(&response)
                })),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_move_mouse: {other:?}"
                )),
            }
        }
        "browser_navigate" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let url = match parse_required_browser_url(&arguments, "browser_navigate") {
                Ok(url) => url,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::Navigate {
                target,
                tab_id,
                url,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Navigate { response },
                } => browser_navigate_result(response),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_navigate: {other:?}"
                )),
            }
        }
        "browser_snapshot" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let snapshot_options = match parse_browser_snapshot_options(&arguments) {
                Ok(options) => options,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let (service_element_offset, service_element_limit) = service_snapshot_element_window(
                snapshot_options.element_offset,
                snapshot_options.element_limit,
            );
            match service.call(&browser_service_request(BrowserRequest::Snapshot {
                target,
                tab_id,
                text_limit: Some(snapshot_options.text_limit),
                element_offset: service_element_offset,
                element_limit: Some(service_element_limit),
                element_query: snapshot_options.element_query.clone(),
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Snapshot { response },
                } => browser_snapshot_result(browser_snapshot_structured_response(
                    response,
                    None,
                    snapshot_options.element_limit,
                    snapshot_options.element_query.as_deref(),
                    snapshot_options.text_limit,
                )),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_snapshot: {other:?}"
                )),
            }
        }
        "browser_screenshot" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let include_image_data = model.can_receive_images();
            match service.call(&browser_service_request(BrowserRequest::Screenshot {
                target,
                tab_id,
                include_image_data,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Screenshot { response },
                } => browser_screenshot_result(response, model.can_receive_images()),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_screenshot: {other:?}"
                )),
            }
        }
        "browser_click" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let (x, y) = match parse_browser_point(&arguments, "browser_click") {
                Ok(point) => point,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::Click {
                target,
                tab_id,
                x,
                y,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Click { response },
                } => browser_action_result(response),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for browser_click: {other:?}")),
            }
        }
        "browser_type_text" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let text =
                match parse_required_literal_string(&arguments, "text", "browser_type_text text") {
                    Ok(text) => text,
                    Err(error) => return invalid_request_tool_error(error.to_string()),
                };
            match service.call(&browser_service_request(BrowserRequest::TypeText {
                target,
                tab_id,
                text,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::TypeText { response },
                } => browser_action_result(response),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_type_text: {other:?}"
                )),
            }
        }
        "browser_press_key" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let key = match parse_required_string(&arguments, "key", "browser_press_key key") {
                Ok(key) => key,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::PressKey {
                target,
                tab_id,
                key,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::PressKey { response },
                } => browser_action_result(response),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_press_key: {other:?}"
                )),
            }
        }
        "browser_scroll" => {
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let (delta_x, delta_y, x, y) = match parse_browser_scroll(&arguments) {
                Ok(scroll) => scroll,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::Scroll {
                target,
                tab_id,
                delta_x,
                delta_y,
                x,
                y,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Scroll { response },
                } => browser_action_result(response),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for browser_scroll: {other:?}")),
            }
        }
        "browser_eval" => {
            if !browser_eval_enabled_policy.unwrap_or_else(browser_eval_enabled) {
                return tool_error(
                    "BrowserEvalDisabled",
                    "browser_eval is disabled by default because it runs arbitrary \
                     JavaScript in real user tabs. The operator can enable it with \
                     SKY_CUA_BROWSER_EVAL=on, 1, or true.",
                );
            }
            let target = match parse_browser_target(&arguments) {
                Ok(target) => target,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(tab_id) => tab_id,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            let expression = match parse_required_literal_string(
                &arguments,
                "expression",
                "browser_eval expression",
            ) {
                Ok(expression) => expression,
                Err(error) => return invalid_request_tool_error(error.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::Eval {
                target,
                tab_id,
                expression,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Eval { response },
                } => browser_eval_result(response),
                ServiceResponse::Error { code, message } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for browser_eval: {other:?}")),
            }
        }
        other => Err(anyhow!("unexpected browser tool name: {other}")),
    }
}

fn browser_service_request(request: BrowserRequest) -> ServiceRequest {
    ServiceRequest::Browser { request }
}

fn service_snapshot_element_window(
    element_offset: Option<usize>,
    element_limit: Option<usize>,
) -> (Option<usize>, usize) {
    let offset = element_offset.unwrap_or(0);
    if element_limit == Some(0) || offset >= BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT {
        return (None, 0);
    }
    let remaining = BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT - offset;
    let limit = match element_limit {
        Some(limit) => limit.min(remaining),
        None => BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT.min(remaining),
    };
    let offset = (offset > 0).then_some(offset);
    (offset, limit)
}
