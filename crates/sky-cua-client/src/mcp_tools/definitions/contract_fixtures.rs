//! Golden fixture data generators backing `fixture_tests`: the surface
//! matrix, the per-tool contract, and the call-case corpus. Split out of
//! `fixture_tests.rs` purely to keep that file under the repo's file-size
//! convention; still `#[cfg(test)]` only.

use serde_json::{Value, json};

use crate::mcp_server::ModelSessionInfo;

use super::build_tool_registry;
use super::fixture_tests::{process_config, process_config_with_surfaces};

pub(super) fn generated_surface_matrix() -> Value {
    let mut rows = Vec::new();
    for can_receive_images in [false, true] {
        for browser_eval_enabled in [false, true] {
            let model = model_with_image_capability(can_receive_images);
            let registry = build_tool_registry(&process_config(browser_eval_enabled), &model);
            let tools_list = registry.tools_list_result();
            let serialized = serde_json::to_string(&tools_list).expect("tools list json");
            rows.push(json!({
                "surface": "grouped",
                "can_receive_images": can_receive_images,
                "browser_eval_enabled": browser_eval_enabled,
                "tool_count": registry.active_names.len(),
                "serialized_bytes": serialized.len(),
                "description_bytes": description_bytes(&registry.tools),
                "largest_schema_bytes": largest_schema_bytes(&registry.tools),
                "tools_list": tools_list
            }));
        }
    }

    json!({
        "version": 1,
        "generated_by": "crates/sky-cua-client/src/mcp_tools/definitions.rs",
        "rows": rows
    })
}

pub(super) fn generated_surface_policy_matrix() -> Value {
    let mut rows = Vec::new();
    for desktop in [false, true] {
        for browser in [false, true] {
            for phone in [false, true] {
                let registry = build_tool_registry(
                    &process_config_with_surfaces(desktop, browser, phone, false),
                    &ModelSessionInfo::default(),
                );
                let tools = registry.tools.as_array().expect("tools");
                let names: Vec<&str> = tools
                    .iter()
                    .filter_map(|tool| tool["name"].as_str())
                    .collect();
                let mut shared = serde_json::Map::new();
                for name in ["status", "list_resources", "observe", "capture_screen"] {
                    let Some(tool) = tools.iter().find(|tool| tool["name"] == name) else {
                        continue;
                    };
                    let properties = tool["inputSchema"]["properties"]
                        .as_object()
                        .expect("properties");
                    let property_names: Vec<&str> = properties.keys().map(String::as_str).collect();
                    shared.insert(
                        name.to_string(),
                        json!({
                            "property_names": property_names,
                            "surface_enum": properties.get("surface").and_then(|value| value.get("enum")).cloned(),
                            "component_enum": properties.get("component").and_then(|value| value.get("enum")).cloned(),
                            "resource_enum": properties.get("resource").and_then(|value| value.get("enum")).cloned(),
                        }),
                    );
                }
                rows.push(json!({
                    "desktop": desktop,
                    "browser": browser,
                    "phone": phone,
                    "tool_names": names,
                    "shared": shared,
                }));
            }
        }
    }
    json!({
        "version": 1,
        "generated_by": "crates/sky-cua-client/src/mcp_tools/definitions.rs",
        "rows": rows,
    })
}

fn model_with_image_capability(can_receive_images: bool) -> ModelSessionInfo {
    ModelSessionInfo {
        supports_images: Some(can_receive_images),
    }
}

fn description_bytes(tools: &Value) -> usize {
    tools
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["description"].as_str())
        .map(str::len)
        .sum()
}

fn largest_schema_bytes(tools: &Value) -> usize {
    tools
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| {
            serde_json::to_string(&tool["inputSchema"])
                .expect("schema json")
                .len()
        })
        .max()
        .unwrap_or(0)
}

pub(super) fn generated_tool_contract() -> Value {
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    let advertised = registry.tools.as_array().expect("tools");
    let tools: Vec<Value> = grouped_contract_tools()
        .into_iter()
        .map(|mut contract| {
            let name = contract["name"].as_str().expect("contract name");
            let public = advertised
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("contract references unadvertised tool {name}"));
            let validation = registry
                .validation_schemas
                .get(name)
                .cloned()
                .unwrap_or_else(|| public["inputSchema"].clone());
            let object = contract.as_object_mut().expect("contract object");
            object.insert("annotations".to_string(), public["annotations"].clone());
            object.insert("input_schema".to_string(), public["inputSchema"].clone());
            object.insert("validation_schema".to_string(), validation);
            object.insert("content_policy".to_string(), json!("grouped_rewrite"));
            object.insert("structured_policy".to_string(), json!("grouped_envelope"));
            contract
        })
        .collect();
    assert!(tools.iter().all(|tool| {
        serde_json::to_string(&tool["input_schema"])
            .expect("schema json")
            .len()
            <= 8192
    }));
    json!({
        "version": 1,
        "surface": "grouped",
        "default_tool_count": 40,
        "eval_tool_count": 41,
        "tools": tools
    })
}

fn grouped_contract_tools() -> Vec<Value> {
    vec![
        contract_tool("doctor", vec![branch("diagnostics", "doctor", json!({}))]),
        contract_tool(
            "status",
            vec![
                branch("browser", "browser_status", json!({"component": "browser"})),
                branch("phone", "phone_status", json!({"component": "phone"})),
                branch(
                    "phone_companion",
                    "phone_companion_status",
                    json!({"component": "phone_companion"}),
                ),
                branch(
                    "session_presence",
                    "session_presence_status",
                    json!({"component": "session_presence"}),
                ),
            ],
        ),
        contract_tool(
            "list_resources",
            vec![
                branch(
                    "desktop/apps",
                    "list_apps",
                    json!({"surface": "desktop", "resource": "apps"}),
                ),
                branch(
                    "desktop/windows",
                    "list_windows",
                    json!({"surface": "desktop", "resource": "windows"}),
                ),
                branch(
                    "desktop/focused_window",
                    "focused_window",
                    json!({"surface": "desktop", "resource": "focused_window"}),
                ),
                branch(
                    "browser/tabs",
                    "browser_list_tabs",
                    json!({"surface": "browser", "resource": "tabs"}),
                ),
                branch(
                    "phone/devices",
                    "phone_list_devices",
                    json!({"surface": "phone", "resource": "devices"}),
                ),
                branch(
                    "phone/apps",
                    "phone_app_list",
                    json!({"surface": "phone", "resource": "apps", "session_id": "phone-1"}),
                ),
                branch(
                    "phone/current_app",
                    "phone_app_current",
                    json!({"surface": "phone", "resource": "current_app", "session_id": "phone-1"}),
                ),
            ],
        ),
        contract_tool(
            "observe",
            vec![
                branch(
                    "desktop",
                    "desktop_observe_appshot",
                    json!({"surface": "desktop"}),
                ),
                branch(
                    "browser",
                    "browser_appshot",
                    json!({"surface": "browser", "tab_id": "tab-1"}),
                ),
                branch(
                    "phone",
                    "phone_observe",
                    json!({"surface": "phone", "session_id": "phone-1"}),
                ),
            ],
        ),
        contract_tool(
            "capture_screen",
            vec![
                branch(
                    "browser",
                    "browser_screenshot",
                    json!({"surface": "browser", "tab_id": "tab-1"}),
                ),
                branch(
                    "phone",
                    "phone_screenshot",
                    json!({"surface": "phone", "session_id": "phone-1"}),
                ),
            ],
        ),
        contract_tool(
            "phone_accessibility_tree",
            vec![branch(
                "default",
                "phone_accessibility_tree",
                json!({"session_id": "phone-1"}),
            )],
        ),
        contract_tool(
            "phone_notifications",
            vec![branch(
                "default",
                "phone_notifications",
                json!({"session_id": "phone-1"}),
            )],
        ),
        contract_tool(
            "capture_desktop",
            vec![branch("default", "screenshot", json!({}))],
        ),
        contract_tool(
            "setup_desktop",
            vec![
                branch(
                    "accessibility",
                    "setup_accessibility",
                    json!({"operation": "accessibility"}),
                ),
                branch(
                    "window_targeting",
                    "setup_window_targeting",
                    json!({"operation": "window_targeting"}),
                ),
            ],
        ),
        contract_tool(
            "session_presence",
            vec![
                branch("hold", "hold_session", json!({"operation": "hold"})),
                branch("unlock", "unlock_session", json!({"operation": "unlock"})),
                branch(
                    "release",
                    "release_session",
                    json!({"operation": "release"}),
                ),
            ],
        ),
        contract_tool(
            "activate_window",
            vec![branch(
                "default",
                "activate_window",
                json!({"window_id": "window-1"}),
            )],
        ),
        contract_tool(
            "desktop_semantic",
            vec![
                branch(
                    "focus",
                    "focus_element",
                    json!({"operation": "focus", "snapshot_id": "snapshot-1", "element_index": 1}),
                ),
                branch(
                    "select",
                    "select_element",
                    json!({"operation": "select", "snapshot_id": "snapshot-1", "element_index": 1}),
                ),
                branch(
                    "expand",
                    "expand_element",
                    json!({"operation": "expand", "snapshot_id": "snapshot-1", "element_index": 1}),
                ),
                branch(
                    "collapse",
                    "collapse_element",
                    json!({"operation": "collapse", "snapshot_id": "snapshot-1", "element_index": 1}),
                ),
            ],
        ),
        contract_tool(
            "browser_claim_tab",
            vec![branch(
                "default",
                "browser_claim_tab",
                json!({"tab_id": "tab-1"}),
            )],
        ),
        contract_tool(
            "browser_move_mouse",
            vec![branch(
                "default",
                "browser_move_mouse",
                json!({"tab_id": "tab-1", "x": 1, "y": 1}),
            )],
        ),
        contract_tool(
            "phone_connection",
            vec![
                branch("connect", "phone_connect", json!({"operation": "connect"})),
                branch(
                    "disconnect",
                    "phone_disconnect",
                    json!({"operation": "disconnect", "session_id": "phone-1"}),
                ),
                branch(
                    "refresh",
                    "phone_refresh_capabilities",
                    json!({"operation": "refresh", "session_id": "phone-1"}),
                ),
            ],
        ),
        contract_tool(
            "phone_pair_wireless",
            vec![branch(
                "default",
                "phone_pair_wireless",
                json!({"host_port": "127.0.0.1:37099", "pairing_code": "123456"}),
            )],
        ),
        contract_tool(
            "phone_setup",
            vec![
                branch(
                    "install_companion",
                    "phone_install_companion",
                    json!({"operation": "install_companion", "session_id": "phone-1"}),
                ),
                branch(
                    "open_settings",
                    "phone_open_settings",
                    json!({"operation": "open_settings", "session_id": "phone-1", "screen": "accessibility"}),
                ),
            ],
        ),
        contract_tool(
            "phone_app_force_stop",
            vec![branch(
                "default",
                "phone_app_force_stop",
                json!({"session_id": "phone-1", "package_name": "com.example.app"}),
            )],
        ),
        contract_tool(
            "desktop_toggle",
            vec![branch(
                "default",
                "toggle_element",
                json!({"snapshot_id": "snapshot-1", "element_index": 1}),
            )],
        ),
        contract_tool(
            "desktop_scroll",
            vec![branch(
                "default",
                "scroll",
                json!({"direction": "down", "snapshot_id": "snapshot-1", "element_index": 1}),
            )],
        ),
        contract_tool(
            "browser_scroll",
            vec![branch(
                "default",
                "browser_scroll",
                json!({"tab_id": "tab-1", "delta_y": 300}),
            )],
        ),
        contract_tool(
            "desktop_pointer",
            vec![
                branch(
                    "click",
                    "click",
                    json!({"operation": "click", "x": 1, "y": 1}),
                ),
                branch(
                    "secondary_click",
                    "perform_secondary_action",
                    json!({"operation": "secondary_click", "x": 1, "y": 1}),
                ),
                branch(
                    "drag",
                    "drag",
                    json!({"operation": "drag", "from_x": 1, "from_y": 1, "to_x": 2, "to_y": 2}),
                ),
            ],
        ),
        contract_tool(
            "desktop_keyboard",
            vec![
                branch(
                    "type_text",
                    "type_text",
                    json!({"operation": "type_text", "text": "hello"}),
                ),
                branch(
                    "press_key",
                    "press_key",
                    json!({"operation": "press_key", "key": "Enter"}),
                ),
            ],
        ),
        contract_tool(
            "desktop_action",
            vec![
                branch(
                    "activate",
                    "activate_element",
                    json!({"operation": "activate", "snapshot_id": "snapshot-1", "element_index": 1}),
                ),
                branch(
                    "perform_action",
                    "perform_action",
                    json!({"operation": "perform_action", "action_index": 0, "snapshot_id": "snapshot-1", "element_index": 1}),
                ),
            ],
        ),
        contract_tool(
            "desktop_launch_app",
            // The advertised `IsolatedDesktopRequired` code is proven live by
            // the `desktop_launch_app_refuses_when_not_isolated` handler test
            // in `mcp_tools`; this contract is the declared surface, that test
            // is its liveness check.
            vec![branch_with_extra_errors(
                "default",
                "desktop_launch_app",
                json!({"command": "xmessage", "args": ["hello"]}),
                &["IsolatedDesktopRequired"],
            )],
        ),
        contract_tool(
            "desktop_set_value",
            vec![branch(
                "default",
                "set_value",
                json!({"snapshot_id": "snapshot-1", "element_index": 1, "value": "hello"}),
            )],
        ),
        contract_tool(
            "browser_open",
            vec![branch("default", "browser_open", json!({}))],
        ),
        contract_tool(
            "browser_navigate",
            vec![branch(
                "default",
                "browser_navigate",
                json!({"tab_id": "tab-1", "url": "https://example.test/"}),
            )],
        ),
        contract_tool(
            "browser_input",
            vec![
                branch(
                    "click",
                    "browser_click",
                    json!({"operation": "click", "tab_id": "tab-1", "x": 1, "y": 1}),
                ),
                branch(
                    "type_text",
                    "browser_type_text",
                    json!({"operation": "type_text", "tab_id": "tab-1", "text": "hello"}),
                ),
                branch(
                    "press_key",
                    "browser_press_key",
                    json!({"operation": "press_key", "tab_id": "tab-1", "key": "Enter"}),
                ),
            ],
        ),
        contract_tool(
            "phone_pointer",
            vec![
                branch(
                    "tap",
                    "phone_tap",
                    json!({"operation": "tap", "session_id": "phone-1", "phone_snapshot_id": "phone-snapshot-1", "x": 1, "y": 1}),
                ),
                branch(
                    "swipe",
                    "phone_swipe",
                    json!({"operation": "swipe", "session_id": "phone-1", "phone_snapshot_id": "phone-snapshot-1", "start_x": 1, "start_y": 1, "end_x": 2, "end_y": 2}),
                ),
            ],
        ),
        contract_tool(
            "phone_keyboard",
            vec![
                branch(
                    "type_text",
                    "phone_type_text",
                    json!({"operation": "type_text", "session_id": "phone-1", "text": "hello"}),
                ),
                branch(
                    "press_key",
                    "phone_press_key",
                    json!({"operation": "press_key", "session_id": "phone-1", "key": "BACK"}),
                ),
            ],
        ),
        contract_tool(
            "phone_notification_action",
            vec![
                branch(
                    "open",
                    "phone_notification_open",
                    json!({"operation": "open", "session_id": "phone-1", "event_id": "event-1"}),
                ),
                branch(
                    "dismiss",
                    "phone_notification_dismiss",
                    json!({"operation": "dismiss", "session_id": "phone-1", "event_id": "event-1"}),
                ),
                branch(
                    "action",
                    "phone_notification_action",
                    json!({"operation": "action", "session_id": "phone-1", "event_id": "event-1", "action_id": "action-1"}),
                ),
            ],
        ),
        contract_tool(
            "phone_notification_reply",
            vec![branch(
                "default",
                "phone_notification_reply",
                json!({"session_id": "phone-1", "event_id": "event-1", "action_id": "reply-1", "text": "reply"}),
            )],
        ),
        contract_tool(
            "phone_app_action",
            vec![
                branch(
                    "launch",
                    "phone_app_launch",
                    json!({"operation": "launch", "session_id": "phone-1", "package_name": "com.example.app"}),
                ),
                branch(
                    "open_intent",
                    "phone_app_open_intent",
                    json!({"operation": "open_intent", "session_id": "phone-1", "intent_uri": "intent://example"}),
                ),
            ],
        ),
        contract_tool(
            "phone_app_install",
            vec![branch(
                "default",
                "phone_app_install",
                json!({"session_id": "phone-1", "apk_paths": ["/tmp/example.apk"]}),
            )],
        ),
        contract_tool(
            "phone_content",
            vec![branch(
                "describe",
                "phone_content",
                json!({"operation":"describe", "session_id":"phone-1", "content_id":"content-1"}),
            )],
        ),
        contract_tool(
            "phone_clipboard",
            vec![branch(
                "get",
                "phone_clipboard",
                json!({"operation":"get", "session_id":"phone-1"}),
            )],
        ),
        contract_tool(
            "phone_editor",
            vec![branch(
                "context",
                "phone_editor",
                json!({"operation":"context", "session_id":"phone-1"}),
            )],
        ),
        contract_tool(
            "phone_camera",
            vec![branch(
                "enumerate",
                "phone_camera",
                json!({"operation":"enumerate", "session_id":"phone-1"}),
            )],
        ),
        contract_tool(
            "phone_storage",
            vec![branch(
                "roots",
                "phone_storage",
                json!({"operation":"roots", "session_id":"phone-1"}),
            )],
        ),
        contract_tool(
            "browser_eval",
            vec![branch(
                "default",
                "browser_eval",
                json!({"tab_id": "tab-1", "expression": "document.title"}),
            )],
        ),
    ]
}

fn contract_tool(name: &'static str, branches: Vec<Value>) -> Value {
    json!({
        "name": name,
        "branches": branches
    })
}

fn branch(name: &'static str, handler_id: &'static str, minimal_valid_arguments: Value) -> Value {
    let mut arguments = minimal_valid_arguments;
    if matches!(
        handler_id,
        "focus_element"
            | "select_element"
            | "expand_element"
            | "collapse_element"
            | "toggle_element"
            | "scroll"
            | "click"
            | "perform_secondary_action"
            | "drag"
            | "type_text"
            | "press_key"
            | "activate_element"
            | "perform_action"
            | "set_value"
            | "browser_scroll"
            | "browser_move_mouse"
            | "browser_click"
            | "browser_type_text"
            | "browser_press_key"
            | "browser_eval"
            | "phone_tap"
            | "phone_swipe"
            | "phone_type_text"
            | "phone_press_key"
            | "phone_notification_open"
            | "phone_notification_dismiss"
            | "phone_notification_action"
            | "phone_notification_reply"
            | "phone_app_force_stop"
            | "phone_app_install"
    ) {
        arguments
            .as_object_mut()
            .expect("branch arguments")
            .insert("appshot_id".into(), json!("appshot-1"));
    }
    branch_with_extra_errors(name, handler_id, arguments, &[])
}

/// Like [`branch`] but advertises additional tool-specific error codes beyond
/// the common `InvalidRequest`/`FeatureDisabled`/`UnknownTool` set — e.g.
/// `desktop_launch_app`'s `IsolatedDesktopRequired` isolated-session gate — so
/// the published contract names every code a host may have to handle.
fn branch_with_extra_errors(
    name: &'static str,
    handler_id: &'static str,
    minimal_valid_arguments: Value,
    extra_errors: &[&str],
) -> Value {
    let mut expected_errors = vec!["InvalidRequest", "FeatureDisabled", "UnknownTool"];
    expected_errors.extend_from_slice(extra_errors);
    json!({
        "name": name,
        "handler_id": handler_id,
        "minimal_valid_arguments": minimal_valid_arguments,
        "expected_errors": expected_errors
    })
}

pub(super) fn generated_call_cases() -> Value {
    let mut cases = Vec::new();
    for tool in grouped_contract_tools() {
        let tool_name = tool["name"].as_str().expect("tool name");
        for branch in tool["branches"].as_array().expect("branches") {
            let branch_name = branch["name"].as_str().expect("branch name");
            cases.push(json!({
                "tool": tool_name,
                "branch": branch_name,
                "handler_id": branch["handler_id"],
                "valid": branch["minimal_valid_arguments"],
                "invalid": invalid_call_case(&branch["minimal_valid_arguments"])
            }));
        }
    }
    json!({
        "version": 1,
        "cases": cases
    })
}

fn invalid_call_case(valid: &Value) -> Value {
    let mut invalid = valid.as_object().expect("valid case object").clone();
    if invalid.get("operation").and_then(Value::as_str) == Some("type_text")
        && invalid.contains_key("tab_id")
    {
        invalid.insert("x".to_string(), json!(1));
        invalid.insert("y".to_string(), json!(1));
    } else if invalid.get("operation").and_then(Value::as_str) == Some("tap") {
        invalid.remove("phone_snapshot_id");
        invalid.remove("use_device_coordinates");
    } else if invalid.get("surface").and_then(Value::as_str) == Some("phone") {
        invalid.insert("window_id".to_string(), json!("window-foreign"));
    } else if invalid.get("surface").and_then(Value::as_str) == Some("desktop") {
        invalid.insert("include_mdns".to_string(), json!(true));
    } else if invalid.contains_key("operation") {
        invalid.insert("operation".to_string(), json!("__invalid__"));
    } else if invalid.contains_key("surface") {
        invalid.insert("surface".to_string(), json!("__invalid__"));
    } else if invalid.contains_key("component") {
        invalid.insert("component".to_string(), json!("__invalid__"));
    } else if let Some(first_key) = invalid.keys().next().cloned() {
        invalid.insert(first_key, json!(false));
    } else {
        invalid.insert("__unexpected".to_string(), json!(true));
    }
    Value::Object(invalid)
}
