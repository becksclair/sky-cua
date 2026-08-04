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
    parse_browser_target, parse_required_appshot_id, parse_required_browser_url,
    parse_required_literal_string, parse_required_string,
};
use args::{
    parse_browser_snapshot_options, parse_browser_tab_filter, parse_optional_bool,
    parse_optional_element_ref,
};
#[cfg(test)]
pub(super) use response::browser_list_tabs_summary;
#[cfg(test)]
pub(super) use response::browser_snapshot_summary;
use response::{
    apply_browser_tab_limit, browser_action_result, browser_appshot_required_result,
    browser_claim_tab_is_error, browser_claim_tab_summary, browser_eval_result,
    browser_list_tabs_is_error, browser_list_tabs_structured_response,
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
            | "browser_appshot"
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
    browser_identity: Option<&sky_cua_platform::BrowserSessionIdentity>,
) -> Result<Value> {
    let browser_service_request = |request| browser_service_request(request, browser_identity);
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
            ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            let limit = match super::parse_optional_usize(
                &arguments,
                "limit",
                "list_resources browser tabs limit",
            ) {
                Ok(limit) => limit,
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
                    let (response, matching_tab_indexes) =
                        apply_browser_tab_limit(response, matching_tab_indexes, limit);
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
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            let appshot_id = match parse_required_appshot_id(&arguments, "browser_move_mouse") {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::MoveMouse {
                target,
                tab_id,
                x,
                y,
                wait_for_arrival,
                appshot_id: Some(appshot_id),
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
                ServiceResponse::Browser {
                    response: BrowserResponse::AppShotRequired { rejection },
                } => browser_appshot_required_result(rejection),
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_snapshot: {other:?}"
                )),
            }
        }
        "browser_appshot" => {
            let target = match parse_browser_target(&arguments) {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            let tab_id = match parse_browser_tab_id(&arguments) {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            let options = match parse_browser_snapshot_options(&arguments) {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::ObserveAppShot {
                target,
                tab_id,
                text_limit: Some(options.text_limit),
                element_limit: options.element_limit,
                include_image_data: model.can_receive_images(),
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::AppShot { response },
                } => {
                    let text = format!(
                        "Captured browser AppShot for tab {}.",
                        match &response.appshot.capture {
                            sky_cua_platform::model::AppShotCapture::Browser { tab_id, .. } =>
                                tab_id,
                            _ => "unknown",
                        }
                    );
                    let mut content = vec![json!({"type":"text","text":text})];
                    if model.can_receive_images() && !response.image_data_base64.is_empty() {
                        content.push(json!({"type":"image","data":response.image_data_base64,"mimeType":response.image_mime_type}));
                    }
                    Ok(
                        json!({"content":content,"structuredContent":response.appshot,"isError":false}),
                    )
                }
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
                other => Err(anyhow!(
                    "unexpected response for browser_appshot: {other:?}"
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
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            // Element-target clicking: when the caller passes an opaque `ref`
            // from observe(surface=browser), click by element identity (the
            // service re-resolves its live position) instead of by coordinates.
            let appshot_id = match parse_required_appshot_id(&arguments, "browser_click") {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            let request = match parse_optional_element_ref(&arguments) {
                Some(element_ref) => BrowserRequest::ClickElement {
                    target,
                    tab_id,
                    element_ref,
                    appshot_id: Some(appshot_id),
                },
                None => {
                    let (x, y) = match parse_browser_point(&arguments, "browser_click") {
                        Ok(point) => point,
                        Err(error) => return invalid_request_tool_error(error.to_string()),
                    };
                    BrowserRequest::Click {
                        target,
                        tab_id,
                        x,
                        y,
                        appshot_id: Some(appshot_id),
                    }
                }
            };
            match service.call(&browser_service_request(request))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Click { response },
                } => browser_action_result(response),
                ServiceResponse::Browser {
                    response: BrowserResponse::AppShotRequired { rejection },
                } => browser_appshot_required_result(rejection),
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            let appshot_id = match parse_required_appshot_id(&arguments, "browser_type_text") {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            let request = match parse_optional_element_ref(&arguments) {
                Some(element_ref) => BrowserRequest::TypeTextElement {
                    target,
                    tab_id,
                    element_ref,
                    text,
                    appshot_id: Some(appshot_id),
                },
                None => BrowserRequest::TypeText {
                    target,
                    tab_id,
                    text,
                    appshot_id: Some(appshot_id),
                },
            };
            match service.call(&browser_service_request(request))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::TypeText { response },
                } => browser_action_result(response),
                ServiceResponse::Browser {
                    response: BrowserResponse::AppShotRequired { rejection },
                } => browser_appshot_required_result(rejection),
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            let appshot_id = match parse_required_appshot_id(&arguments, "browser_press_key") {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::PressKey {
                target,
                tab_id,
                key,
                appshot_id: Some(appshot_id),
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::PressKey { response },
                } => browser_action_result(response),
                ServiceResponse::Browser {
                    response: BrowserResponse::AppShotRequired { rejection },
                } => browser_appshot_required_result(rejection),
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
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
            let appshot_id = match parse_required_appshot_id(&arguments, "browser_scroll") {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::Scroll {
                target,
                tab_id,
                delta_x,
                delta_y,
                x,
                y,
                appshot_id: Some(appshot_id),
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Scroll { response },
                } => browser_action_result(response),
                ServiceResponse::Browser {
                    response: BrowserResponse::AppShotRequired { rejection },
                } => browser_appshot_required_result(rejection),
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for browser_scroll: {other:?}")),
            }
        }
        "browser_eval" => {
            if !browser_eval_enabled_policy.unwrap_or_else(browser_eval_enabled) {
                return tool_error(
                    "BrowserEvalDisabled",
                    "browser_eval is disabled via SKY_CUA_BROWSER_EVAL. Remove the \
                     override, or set it to on, 1, or true, to re-enable it.",
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
            let appshot_id = match parse_required_appshot_id(&arguments, "browser_eval") {
                Ok(v) => v,
                Err(e) => return invalid_request_tool_error(e.to_string()),
            };
            match service.call(&browser_service_request(BrowserRequest::Eval {
                target,
                tab_id,
                expression,
                appshot_id: Some(appshot_id),
            }))? {
                ServiceResponse::Browser {
                    response: BrowserResponse::Eval { response },
                } => browser_eval_result(response),
                ServiceResponse::Browser {
                    response: BrowserResponse::AppShotRequired { rejection },
                } => browser_appshot_required_result(rejection),
                ServiceResponse::Error { code, message, .. } => tool_error(code, message),
                other => Err(anyhow!("unexpected response for browser_eval: {other:?}")),
            }
        }
        other => Err(anyhow!("unexpected browser tool name: {other}")),
    }
}

fn browser_service_request(
    request: BrowserRequest,
    identity: Option<&sky_cua_platform::BrowserSessionIdentity>,
) -> ServiceRequest {
    ServiceRequest::Browser {
        request,
        identity: identity.cloned(),
        context: crate::mcp_server::current_browser_request_context(),
    }
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

#[cfg(test)]
mod context_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn browser_request_builder_propagates_scoped_context_and_legacy_identity() {
        let provenance = sky_cua_platform::BrowserCallerProvenance {
            caller: sky_cua_platform::BrowserCallerKind::CodexDesktop,
            source: sky_cua_platform::BrowserProvenanceSource::InstallerDeclaration,
            connection_id: "connection".to_string(),
            declared_caller: Some("codex_desktop".to_string()),
            client_info: Some(sky_cua_platform::BrowserMcpClientInfo {
                name: "codex".to_string(),
                version: "1.0".to_string(),
                title: None,
            }),
        };
        let (identity, context) = crate::mcp_server::browser_call_context(
            &json!({
                "id": "reused-upstream-id",
                "params": {
                    "_meta": {
                        "x-codex-turn-metadata": {
                            "session_id": "session",
                            "thread_id": "thread",
                            "turn_id": "turn"
                        }
                    }
                }
            }),
            &provenance,
        );
        let operation_id = context.operation_identity.operation_id.clone();

        let request = crate::mcp_server::with_browser_request_context(context, || {
            browser_service_request(BrowserRequest::Status, identity.as_ref())
        });
        let rendered = serde_json::to_value(&request).expect("service request should serialize");
        assert_eq!(rendered["identity"]["session_id"], "session");
        assert_eq!(rendered["context"]["provenance"]["caller"], "codex_desktop");
        assert_eq!(
            rendered["context"]["operation_identity"]["operation_id"],
            operation_id
        );
        let rerendered = serde_json::to_value(request)
            .expect("the same in-flight request should remain serializable");
        assert_eq!(
            rerendered["context"]["operation_identity"]["operation_id"],
            rendered["context"]["operation_identity"]["operation_id"]
        );
    }

    #[test]
    fn browser_request_builder_without_scope_preserves_legacy_shape() {
        let request = browser_service_request(BrowserRequest::Status, None);
        let rendered = serde_json::to_value(request).expect("legacy request should serialize");
        assert!(rendered.get("identity").is_none());
        assert!(rendered.get("context").is_none());
    }
}
