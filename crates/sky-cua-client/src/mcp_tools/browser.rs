use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use sky_cua_platform::model::{BrowserRequest, BrowserResponse, ServiceRequest, ServiceResponse};

mod args;
mod response;
mod schema;

#[cfg(test)]
pub(super) use args::BrowserTabTextFilter;
pub(super) use args::{
    parse_browser_open_url, parse_browser_point, parse_browser_scroll, parse_browser_tab_id,
    parse_browser_target, parse_required_browser_url, parse_required_literal_string,
    parse_required_string,
};
use args::{parse_browser_tab_filter, parse_optional_bool};
#[cfg(test)]
pub(super) use response::browser_list_tabs_summary;
#[cfg(test)]
pub(super) use response::browser_snapshot_summary;
use response::{
    browser_action_result, browser_claim_tab_is_error, browser_claim_tab_summary,
    browser_list_tabs_is_error, browser_list_tabs_structured_response,
    browser_list_tabs_summary_with_matches, browser_move_mouse_is_error,
    browser_move_mouse_summary, browser_navigate_result, browser_open_is_error,
    browser_screenshot_result, browser_snapshot_result, browser_tab_match_indexes,
};
pub(super) use response::{browser_open_summary, browser_status_summary};
pub(super) use schema::push_tool_definitions;

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
    )
}

pub(super) fn handle_tool_call(
    service: &impl McpService,
    tool_name: &str,
    arguments: Value,
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
            match service.call(&browser_service_request(BrowserRequest::Snapshot {
                target,
                tab_id,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Snapshot { response },
                } => browser_snapshot_result(response),
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
            match service.call(&browser_service_request(BrowserRequest::Screenshot {
                target,
                tab_id,
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Screenshot { response },
                } => browser_screenshot_result(response),
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
        other => Err(anyhow!("unexpected browser tool name: {other}")),
    }
}

fn browser_service_request(request: BrowserRequest) -> ServiceRequest {
    ServiceRequest::Browser { request }
}
