//! MCP tool definitions: the host-facing tool registry with input schemas
//! and annotations. Split from `mcp_tools.rs` along the contract-family
//! boundary; dispatch and response shaping stay in the parent module.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::app_state::{
    APP_STATE_DEFAULT_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_QUERY_CHARS,
};
use crate::mcp_server::ModelSessionInfo;
use sky_cua_platform::model::BROWSER_EVAL_ENV;

use super::annotations::{
    LOCAL_DESTRUCTIVE_ACTION, LOCAL_NAVIGATION_ACTION, LOCAL_STATEFUL_ACTION, READ_ONLY_TOOL,
    ToolAnnotations,
};
#[cfg(test)]
use super::browser;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum McpConfigDiagnostic {
    InvalidBrowserEval { value: String },
    InvalidModelSupportsImages { value: String },
}

#[derive(Debug, Clone)]
pub(crate) struct McpProcessConfig {
    pub(crate) browser_eval_enabled: bool,
    pub(crate) model_supports_images_override: Option<bool>,
    #[allow(dead_code)]
    pub(crate) diagnostics: Vec<McpConfigDiagnostic>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum InactiveToolReason {
    BrowserEvalDisabled,
}

#[derive(Debug, Clone)]
pub(crate) struct McpToolRegistry {
    pub(crate) browser_eval_enabled: bool,
    tools: Value,
    active_names: BTreeSet<String>,
    inactive_names: BTreeSet<String>,
}

impl McpToolRegistry {
    pub(crate) fn tools_list_result(&self) -> Value {
        json!({
            "tools": self.tools.clone()
        })
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.active_names.contains(name)
    }

    pub(crate) fn inactive_reason(&self, name: &str) -> Option<InactiveToolReason> {
        self.inactive_names
            .contains(name)
            .then_some(InactiveToolReason::BrowserEvalDisabled)
    }
}

pub(crate) fn mcp_process_config_from_env() -> McpProcessConfig {
    let mut diagnostics = Vec::new();
    let browser_eval_enabled = match std::env::var(BROWSER_EVAL_ENV) {
        Ok(value) => match parse_browser_eval_runtime(&value) {
            Some(value) => value,
            None => {
                diagnostics.push(McpConfigDiagnostic::InvalidBrowserEval { value });
                false
            }
        },
        Err(_) => false,
    };
    let model_supports_images_override = match std::env::var("SKY_CUA_MODEL_SUPPORTS_IMAGES") {
        Ok(value) => match parse_bool_runtime(&value) {
            Some(value) => Some(value),
            None => {
                diagnostics.push(McpConfigDiagnostic::InvalidModelSupportsImages { value });
                None
            }
        },
        Err(_) => None,
    };

    McpProcessConfig {
        browser_eval_enabled,
        model_supports_images_override,
        diagnostics,
    }
}

fn parse_bool_runtime(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "supported" | "enabled" => Some(true),
        "0" | "false" | "no" | "off" | "unsupported" | "disabled" => Some(false),
        _ => None,
    }
}

fn parse_browser_eval_runtime(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" => Some(true),
        "0" | "false" | "off" => Some(false),
        _ => None,
    }
}

pub(crate) fn build_tool_registry(
    process: &McpProcessConfig,
    model: &ModelSessionInfo,
) -> McpToolRegistry {
    let can_receive_images = model.can_receive_images();
    let tools = build_tool_definitions(can_receive_images, process.browser_eval_enabled);
    let active_names = tool_names(&tools);
    let mut inactive_names = BTreeSet::new();
    if !process.browser_eval_enabled && !active_names.contains("browser_eval") {
        inactive_names.insert("browser_eval".to_string());
    }

    McpToolRegistry {
        browser_eval_enabled: process.browser_eval_enabled,
        tools,
        active_names,
        inactive_names,
    }
}

fn tool_names(tools: &Value) -> BTreeSet<String> {
    tools
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
pub(crate) fn tool_definitions(model: &ModelSessionInfo) -> Value {
    build_tool_definitions(model.can_receive_images(), browser::browser_eval_enabled())
}

#[cfg(test)]
pub(crate) fn tools_list_result(model: &ModelSessionInfo) -> Value {
    json!({
        "tools": tool_definitions(model)
    })
}

pub(crate) fn build_tool_definitions(
    can_receive_images: bool,
    browser_eval_enabled: bool,
) -> Value {
    build_compact_tool_definitions(can_receive_images, browser_eval_enabled)
}

fn build_compact_tool_definitions(can_receive_images: bool, browser_eval_enabled: bool) -> Value {
    let mut tools = json!([
        compact_tool(
            "doctor",
            "Run sky-cua readiness diagnostics for desktop capture/input, browser integration, and session presence.",
            READ_ONLY_TOOL,
            json!({}),
            json!([])
        ),
        compact_tool(
            "status",
            "Report browser, phone, phone_companion, or session_presence health.",
            READ_ONLY_TOOL,
            json!({"component": {"type": "string", "enum": ["browser", "phone", "phone_companion", "session_presence"]}}),
            json!(["component"])
        ),
        compact_tool_with_constraints(
            "list_resources",
            "List bounded resources. Valid pairs: desktop apps/windows/focused_window; browser tabs; phone devices/apps/current_app.",
            READ_ONLY_TOOL,
            compact_list_resources_properties(),
            json!(["surface", "resource"]),
            compact_list_resources_constraints()
        ),
        compact_tool_with_constraints(
            "observe",
            "Read structured state. Desktop returns elements and snapshot_id; browser requires tab_id; phone can include accessibility/notifications. Observe before acting; detail=\"compact\" is observation verbosity.",
            READ_ONLY_TOOL,
            compact_observe_properties(can_receive_images),
            json!(["surface"]),
            compact_observe_constraints()
        ),
        compact_tool_with_constraints(
            "capture_screen",
            "Capture a browser-tab or phone image only. Browser requires tab_id. Use capture_desktop for desktop screenshots.",
            READ_ONLY_TOOL,
            compact_capture_screen_properties(),
            json!(["surface"]),
            compact_capture_screen_constraints()
        ),
        compact_tool(
            "phone_accessibility_tree",
            "Read the full connected-phone accessibility tree.",
            READ_ONLY_TOOL,
            with_phone_selector(json!({"node_limit": limit_schema()})),
            json!([])
        ),
        compact_tool(
            "phone_notifications",
            "Read recent connected-phone notifications.",
            READ_ONLY_TOOL,
            with_phone_selector(json!({"limit": limit_schema()})),
            json!([])
        ),
        compact_tool(
            "capture_desktop",
            "Capture a fresh desktop frame. Omit targets for primary display; target one window/display or capture_all_displays. Use the returned snapshot_id and capture source geometry for pixel actions.",
            LOCAL_NAVIGATION_ACTION,
            screenshot_properties(can_receive_images),
            json!([])
        ),
        compact_tool(
            "setup_desktop",
            "Set up desktop accessibility or window targeting.",
            LOCAL_NAVIGATION_ACTION,
            json!({"operation": {"type": "string", "enum": ["accessibility", "window_targeting"]}}),
            json!(["operation"])
        ),
        compact_tool(
            "session_presence",
            "Hold, unlock, or release session-presence inhibitors.",
            LOCAL_NAVIGATION_ACTION,
            json!({
                "operation": {"type": "string", "enum": ["hold", "unlock", "release"]},
                "unlock": {"type": "boolean", "description": "For hold only, unlock before holding inhibitors when supported."},
                "inhibit_lock": {"type": "boolean", "description": "Defaults to true for hold/unlock."},
                "inhibit_suspend": {"type": "boolean", "description": "Defaults to true for hold/unlock."},
                "relock": {"type": "boolean", "description": "For release only, relock after releasing when supported."}
            }),
            json!(["operation"])
        ),
        compact_tool_with_constraints(
            "activate_window",
            "Activate a desktop window by exact id or selector.",
            LOCAL_NAVIGATION_ACTION,
            window_target_schema(),
            json!([]),
            window_target_constraint()
        ),
        compact_tool_with_constraints(
            "desktop_semantic",
            "Focus, select, expand, or collapse a desktop element from observe(surface=\"desktop\").",
            LOCAL_NAVIGATION_ACTION,
            compact_desktop_semantic_properties(
                json!({"operation": {"type": "string", "enum": ["focus", "select", "expand", "collapse"]}})
            ),
            json!(["operation"]),
            compact_desktop_selector_constraint()
        ),
        compact_tool(
            "browser_claim_tab",
            "Claim an existing browser tab and make it controllable for observe, capture_screen, navigation, input, scroll, and eval.",
            LOCAL_NAVIGATION_ACTION,
            browser_tab_properties(),
            json!(["tab_id"])
        ),
        compact_tool(
            "browser_move_mouse",
            "Move the visible browser agent cursor in CSS-pixel coordinates without clicking.",
            LOCAL_NAVIGATION_ACTION,
            compact_browser_point_properties(),
            json!(["tab_id", "x", "y"])
        ),
        compact_tool(
            "phone_connection",
            "Connect, disconnect, or refresh a phone session.",
            LOCAL_NAVIGATION_ACTION,
            compact_phone_connection_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "phone_pair_wireless",
            "Pair Android wireless debugging using a host:port and one-time pairing code.",
            LOCAL_NAVIGATION_ACTION,
            json!({"host_port": non_empty_string_schema(), "pairing_code": non_empty_string_schema()}),
            json!(["host_port", "pairing_code"])
        ),
        compact_tool_with_constraints(
            "phone_setup",
            "Install the phone companion app or open a required Android settings screen.",
            LOCAL_NAVIGATION_ACTION,
            compact_phone_setup_properties(),
            json!(["operation"]),
            compact_phone_setup_constraints()
        ),
        compact_tool(
            "phone_app_force_stop",
            "Force-stop a connected phone app.",
            LOCAL_NAVIGATION_ACTION,
            with_phone_selector(json!({"package_name": non_empty_string_schema()})),
            json!(["package_name"])
        ),
        compact_tool_with_constraints(
            "desktop_toggle",
            "Toggle a desktop element from observe(surface=\"desktop\").",
            LOCAL_STATEFUL_ACTION,
            compact_desktop_semantic_properties(json!({})),
            json!([]),
            compact_desktop_selector_constraint()
        ),
        compact_tool_with_constraints(
            "desktop_scroll",
            "Scroll a desktop target. Provide direction plus snapshot-bound target or coordinates; re-observe before reusing element indexes.",
            LOCAL_STATEFUL_ACTION,
            compact_desktop_semantic_properties(json!({
                "direction": {"type": "string", "enum": ["up", "down"]},
                "pages": {"type": "integer", "minimum": 1, "description": "Preferred magnitude: number of page-sized scroll steps."},
                "steps": {"type": "integer", "description": "Discrete wheel step count; use pages for page-sized motion."},
                "delta_y": {"type": "number", "description": "Smooth vertical delta; use pages for page-sized motion."}
            })),
            json!(["direction"]),
            compact_desktop_snapshot_selector_constraint()
        ),
        compact_tool_with_constraints(
            "browser_scroll",
            "Scroll an open-world browser page. Omit x/y for viewport scroll; provide at least one non-zero delta_x or delta_y. Targeted scroll will move the visible browser agent cursor first.",
            ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true
            },
            compact_browser_scroll_properties(),
            json!(["tab_id"]),
            compact_browser_scroll_constraints()
        ),
        compact_tool_with_constraints(
            "desktop_pointer",
            "Click, secondary-click, or drag on the desktop. Use coordinates or snapshot-bound targets; do not call with only operation.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_desktop_pointer_properties(),
            json!(["operation"]),
            compact_desktop_pointer_constraints()
        ),
        compact_tool_with_constraints(
            "desktop_keyboard",
            "Type text or press a key on the desktop. Focus first; text for type_text, key for press_key, e.g. Enter, Escape, Tab, Ctrl+A, Meta+A.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_desktop_keyboard_properties(),
            json!(["operation"]),
            compact_desktop_keyboard_constraints()
        ),
        compact_tool_with_constraints(
            "desktop_action",
            "Activate a desktop element or perform its named/indexed action from observe(surface=\"desktop\"); do not call with only operation.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_desktop_action_properties(),
            json!(["operation"]),
            compact_desktop_action_constraints()
        ),
        compact_tool_with_constraints(
            "desktop_set_value",
            "Set a desktop element value. Include replacement value plus target from observe(surface=\"desktop\").",
            ToolAnnotations {
                read_only: false,
                destructive: true,
                idempotent: true,
                open_world: false
            },
            compact_desktop_semantic_properties(json!({"value": {"type": "string"}})),
            json!(["value"]),
            compact_desktop_selector_constraint()
        ),
        compact_tool(
            "browser_open",
            "Create a browser tab at url, or about:blank when url is omitted. Returns a tab_id for later browser calls.",
            ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true
            },
            browser_target_url_properties(false),
            json!([])
        ),
        compact_tool(
            "browser_navigate",
            "Navigate a claimed browser tab to an HTTP(S) URL or about:blank.",
            ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: true,
                open_world: true
            },
            browser_target_url_properties(true),
            json!(["tab_id", "url"])
        ),
        compact_tool_with_constraints(
            "browser_input",
            "Click, type text, or press a key in a claimed browser tab. click uses x/y CSS pixels; keys look like Enter, Escape, Tab, Ctrl+K.",
            super::annotations::OPEN_WORLD_DESTRUCTIVE_ACTION,
            compact_browser_input_properties(),
            json!(["operation", "tab_id"]),
            compact_browser_input_constraints()
        ),
        compact_tool_with_constraints(
            "phone_pointer",
            "Tap or swipe on a connected phone. Use phone_snapshot_id for screenshot pixels or use_device_coordinates for raw pixels.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_pointer_properties(),
            json!(["operation"]),
            compact_phone_pointer_constraints()
        ),
        compact_tool_with_constraints(
            "phone_keyboard",
            "Type text or press a key on a connected phone. Focus first; press_key accepts KEYCODE_* names, aliases, or numeric keycodes.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_keyboard_properties(),
            json!(["operation"]),
            compact_phone_keyboard_constraints()
        ),
        compact_tool_with_constraints(
            "phone_notification_action",
            "Open, dismiss, or run an action on a connected-phone notification.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_notification_action_properties(),
            json!(["operation"]),
            compact_phone_notification_action_constraints()
        ),
        compact_tool(
            "phone_notification_reply",
            "Reply inline to a connected-phone notification using event_id and inline-reply action_id from the same fresh event.",
            LOCAL_DESTRUCTIVE_ACTION,
            with_phone_selector(json!({
                "event_id": {
                    "type": "string",
                    "minLength": 1,
                    "description": "event_id from fresh phone notifications."
                },
                "action_id": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Inline-reply action_id from that event."
                },
                "text": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Reply text."
                }
            })),
            json!(["event_id", "action_id", "text"])
        ),
        compact_tool_with_constraints(
            "phone_app_action",
            "Launch a phone app or open an Android intent.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_app_action_properties(),
            json!(["operation"]),
            compact_phone_app_action_constraints()
        ),
        compact_tool_with_constraints(
            "phone_app_install",
            "Install an APK on a connected phone.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_app_install_properties(),
            json!([]),
            compact_phone_app_install_constraints()
        )
    ]);
    if browser_eval_enabled {
        tools.as_array_mut().expect("tool array").push(compact_tool(
            "browser_eval",
            "Evaluate JavaScript in a claimed browser tab. This is hidden unless browser eval is explicitly enabled.",
            super::annotations::OPEN_WORLD_DESTRUCTIVE_ACTION,
            merge_properties(
                browser_tab_properties(),
                json!({"expression": non_empty_string_schema()})
            ),
            json!(["tab_id", "expression"]),
        ));
    }
    tools
}

fn compact_tool(
    name: &str,
    description: &str,
    annotations: ToolAnnotations,
    properties: Value,
    required: Value,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "annotations": annotations.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

fn compact_tool_with_constraints(
    name: &str,
    description: &str,
    annotations: ToolAnnotations,
    properties: Value,
    required: Value,
    constraints: Value,
) -> Value {
    let mut tool = compact_tool(name, description, annotations, properties, required);
    let input_schema = tool
        .get_mut("inputSchema")
        .and_then(Value::as_object_mut)
        .expect("tool inputSchema must be an object");
    let constraints = constraints
        .as_object()
        .unwrap_or_else(|| panic!("tool constraints must be object: {constraints:?}"));
    input_schema.extend(constraints.clone());
    tool
}

fn merge_properties(left: Value, right: Value) -> Value {
    let mut merged = left
        .as_object()
        .unwrap_or_else(|| panic!("merge_properties left must be object: {left:?}"))
        .clone();
    let right = right
        .as_object()
        .unwrap_or_else(|| panic!("merge_properties right must be object: {right:?}"));
    merged.extend(right.clone());
    Value::Object(merged)
}

fn compact_list_resources_properties() -> Value {
    json!({
        "surface": {"type": "string", "enum": ["desktop", "browser", "phone"]},
        "resource": {"type": "string", "enum": ["apps", "windows", "focused_window", "tabs", "devices", "current_app"]},
        "target": browser_target_schema(),
        "url_contains": {
            "type": "string",
            "description": "For browser tabs only, case-insensitive URL filter."
        },
        "title_contains": {
            "type": "string",
            "description": "For browser tabs only, case-insensitive title filter."
        },
        "include_mdns": {
            "type": "boolean",
            "description": "For phone devices only, include mDNS wireless-debugging records."
        },
        "session_id": phone_session_id_schema(),
        "serial": phone_serial_schema(),
        "include_system": {
            "type": "boolean",
            "description": "For phone apps only, include system packages."
        },
        "limit": limit_schema()
    })
}

fn surface_resource_pair(surface: &str, resource: &str) -> Value {
    json!({
        "properties": {
            "surface": {"const": surface},
            "resource": {"const": resource}
        },
        "required": ["surface", "resource"]
    })
}

fn compact_list_resources_constraints() -> Value {
    json!({
        "oneOf": [
            surface_resource_pair("desktop", "apps"),
            surface_resource_pair("desktop", "windows"),
            surface_resource_pair("desktop", "focused_window"),
            surface_resource_pair("browser", "tabs"),
            surface_resource_pair("phone", "devices"),
            surface_resource_pair("phone", "apps"),
            surface_resource_pair("phone", "current_app")
        ]
    })
}

fn compact_observe_properties(can_receive_images: bool) -> Value {
    merge_properties(
        json!({
            "surface": {"type": "string", "enum": ["desktop", "browser", "phone"]},
            "target": browser_target_schema(),
            "tab_id": browser_tab_id_schema(),
            "text_limit": {
                "type": "integer",
                "minimum": 0,
                "maximum": sky_cua_platform::model::BROWSER_SNAPSHOT_MAX_TEXT_LIMIT,
                "description": "For browser only, maximum page text characters."
            },
            "include_accessibility": {
                "type": "boolean",
                "description": "For phone only, include the accessibility tree in the observation."
            },
            "include_notifications": {
                "type": "boolean",
                "description": "For phone only, include recent notifications in the observation."
            },
            "backend": phone_backend_schema()
        }),
        merge_properties(
            get_app_state_properties(can_receive_images),
            merge_properties(
                browser_snapshot_window_properties(),
                phone_selector_properties(),
            ),
        ),
    )
}

fn compact_observe_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"surface": {"const": "browser"}}, "required": ["surface"]},
                "then": {"required": ["tab_id"]}
            }
        ]
    })
}

fn compact_capture_screen_properties() -> Value {
    merge_properties(
        json!({
            "surface": {"type": "string", "enum": ["browser", "phone"]},
            "target": browser_target_schema(),
            "tab_id": browser_tab_id_schema(),
            "backend": phone_backend_schema()
        }),
        phone_selector_properties(),
    )
}

fn compact_capture_screen_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"surface": {"const": "browser"}}, "required": ["surface"]},
                "then": {"required": ["tab_id"]}
            }
        ]
    })
}

fn compact_desktop_semantic_properties(properties: Value) -> Value {
    action_tool_properties(merge_properties(properties, semantic_selector_properties()))
}

fn compact_desktop_pointer_properties() -> Value {
    action_tool_properties(merge_properties(
        json!({
            "operation": {"type": "string", "enum": ["click", "secondary_click", "drag"]},
            "x": coordinate_schema("Click x coordinate or drag start x."),
            "y": coordinate_schema("Click y coordinate or drag start y."),
            "from_x": coordinate_schema("Drag start x coordinate."),
            "from_y": coordinate_schema("Drag start y coordinate."),
            "to_x": coordinate_schema("Drag end x coordinate."),
            "to_y": coordinate_schema("Drag end y coordinate."),
            "to_element_index": {"type": "integer", "minimum": 0}
        }),
        semantic_selector_properties(),
    ))
}

fn compact_desktop_selector_constraint() -> Value {
    json!({
        "anyOf": [
            {"required": ["snapshot_id", "element_index"]},
            {"required": ["element_identifier"]},
            {"required": ["snapshot_id", "name"]},
            {"required": ["snapshot_id", "text"]}
        ]
    })
}

fn compact_desktop_snapshot_selector_constraint() -> Value {
    json!({
        "anyOf": [
            {"required": ["snapshot_id", "element_index"]},
            {"required": ["snapshot_id", "name"]},
            {"required": ["snapshot_id", "text"]}
        ]
    })
}

fn compact_desktop_point_or_selector_constraint() -> Value {
    json!({
        "anyOf": [
            {"required": ["x", "y"]},
            {"required": ["snapshot_id", "element_index"]},
            {"required": ["snapshot_id", "name"]},
            {"required": ["snapshot_id", "text"]}
        ]
    })
}

fn compact_desktop_pointer_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"operation": {"const": "click"}}, "required": ["operation"]},
                "then": compact_desktop_point_or_selector_constraint()
            },
            {
                "if": {"properties": {"operation": {"const": "secondary_click"}}, "required": ["operation"]},
                "then": compact_desktop_point_or_selector_constraint()
            },
            {
                "if": {"properties": {"operation": {"const": "drag"}}, "required": ["operation"]},
                "then": {
                    "anyOf": [
                        {"required": ["from_x", "from_y", "to_x", "to_y"]},
                        {"required": ["x", "y", "to_x", "to_y"]},
                        {"required": ["snapshot_id", "element_index", "to_element_index"]}
                    ]
                }
            }
        ]
    })
}

fn compact_desktop_keyboard_properties() -> Value {
    action_tool_properties(keyboard_target_properties(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": non_empty_string_schema(),
        "key": non_empty_string_schema()
    })))
}

fn compact_desktop_keyboard_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"operation": {"const": "type_text"}}, "required": ["operation"]},
                "then": {"required": ["text"]}
            },
            {
                "if": {"properties": {"operation": {"const": "press_key"}}, "required": ["operation"]},
                "then": {"required": ["key"]}
            }
        ]
    })
}

fn compact_desktop_action_properties() -> Value {
    action_tool_properties(merge_properties(
        json!({
            "operation": {"type": "string", "enum": ["activate", "perform_action"]},
            "action_name": {
                "type": "string",
                "minLength": 1,
                "description": "Named backend action when operation=perform_action."
            },
            "action_index": {
                "type": "integer",
                "minimum": 0,
                "description": "Indexed backend action when operation=perform_action."
            }
        }),
        semantic_selector_properties(),
    ))
}

fn compact_desktop_action_constraints() -> Value {
    json!({
        "allOf": [
            compact_desktop_selector_constraint(),
            {
                "if": {"properties": {"operation": {"const": "perform_action"}}, "required": ["operation"]},
                "then": {
                    "anyOf": [
                        {"required": ["action_name"]},
                        {"required": ["action_index"]}
                    ]
                }
            }
        ]
    })
}

fn action_tool_properties(mut properties: Value) -> Value {
    let property_map = properties
        .as_object_mut()
        .expect("action tool properties must be object");
    property_map.insert(
        "snapshot_id".to_string(),
        json!({
            "type": "string",
            "minLength": 1,
            "description": "Snapshot id."
        }),
    );
    properties
}

fn semantic_selector_properties() -> Value {
    json!({
        "element_index": {
            "type": "integer",
            "minimum": 0,
            "description": "Snapshot element index."
        },
        "element_identifier": {
            "type": "string",
            "minLength": 1,
            "description": "Element backend_ref."
        },
        "role": {
            "type": "string"
        },
        "name": {
            "type": "string",
            "minLength": 1,
            "description": "Element name."
        },
        "text": {
            "type": "string",
            "minLength": 1,
            "description": "Element text."
        },
        "states": {
            "type": "array",
            "items": {"type": "string"}
        }
    })
}

fn browser_target_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["user_chrome"],
        "description": "Browser bridge target. Defaults to user_chrome."
    })
}

fn non_empty_string_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1
    })
}

fn browser_tab_id_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "Browser tab id."
    })
}

fn browser_tab_properties() -> Value {
    json!({
        "target": browser_target_schema(),
        "tab_id": browser_tab_id_schema()
    })
}

fn browser_target_url_properties(require_tab: bool) -> Value {
    let mut properties = json!({
        "target": browser_target_schema(),
        "url": {
            "type": "string",
            "pattern": "^(https?://|about:blank$)"
        }
    });
    if require_tab && let Some(map) = properties.as_object_mut() {
        map.insert("tab_id".to_string(), browser_tab_id_schema());
    }
    properties
}

fn compact_browser_point_properties() -> Value {
    merge_properties(
        browser_tab_properties(),
        json!({
            "x": {"type": "number", "minimum": 0, "description": "CSS pixel x coordinate."},
            "y": {"type": "number", "minimum": 0, "description": "CSS pixel y coordinate."},
            "wait_for_arrival": {
                "type": "boolean",
                "description": "Wait for the visible cursor overlay to arrive. Defaults to true."
            }
        }),
    )
}

fn compact_browser_input_properties() -> Value {
    merge_properties(
        compact_browser_point_properties(),
        json!({
            "operation": {"type": "string", "enum": ["click", "type_text", "press_key"]},
            "text": non_empty_string_schema(),
            "key": non_empty_string_schema()
        }),
    )
}

fn compact_browser_input_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"operation": {"const": "click"}}, "required": ["operation"]},
                "then": {"required": ["x", "y"]}
            },
            {
                "if": {"properties": {"operation": {"const": "type_text"}}, "required": ["operation"]},
                "then": {"required": ["text"]}
            },
            {
                "if": {"properties": {"operation": {"const": "press_key"}}, "required": ["operation"]},
                "then": {"required": ["key"]}
            }
        ]
    })
}

fn compact_browser_scroll_properties() -> Value {
    merge_properties(
        compact_browser_point_properties(),
        json!({
            "delta_x": {"type": "number", "description": "Horizontal wheel delta in CSS pixels; at least one delta must be non-zero."},
            "delta_y": {"type": "number", "description": "Vertical wheel delta in CSS pixels; at least one delta must be non-zero."}
        }),
    )
}

fn compact_browser_scroll_constraints() -> Value {
    json!({
        "allOf": [
            {
                "anyOf": [
                    {
                        "required": ["delta_x"],
                        "properties": {"delta_x": {"not": {"const": 0}}}
                    },
                    {
                        "required": ["delta_y"],
                        "properties": {"delta_y": {"not": {"const": 0}}}
                    }
                ]
            },
            {
                "if": {"required": ["x"]},
                "then": {"required": ["y"]}
            },
            {
                "if": {"required": ["y"]},
                "then": {"required": ["x"]}
            }
        ]
    })
}

fn browser_snapshot_window_properties() -> Value {
    json!({
        "element_query": {"type": "string", "maxLength": APP_STATE_MAX_ELEMENT_QUERY_CHARS},
        "element_offset": {"type": "integer", "minimum": 0},
        "element_limit": {
            "type": "integer",
            "minimum": 0,
            "maximum": sky_cua_platform::model::BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT
        }
    })
}

fn phone_session_id_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "Phone session id."
    })
}

fn phone_serial_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "ADB serial."
    })
}

fn phone_backend_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["auto", "adb", "companion", "scrcpy", "none"],
        "description": "Only honored by phone observe, phone screenshot, and phone connect branches."
    })
}

fn phone_selector_properties() -> Value {
    json!({
        "session_id": phone_session_id_schema(),
        "serial": phone_serial_schema()
    })
}

fn with_phone_selector(properties: Value) -> Value {
    merge_properties(properties, phone_selector_properties())
}

fn limit_schema() -> Value {
    json!({"type": "integer", "minimum": 0})
}

fn compact_phone_connection_properties() -> Value {
    merge_properties(
        with_phone_selector(json!({
            "operation": {"type": "string", "enum": ["connect", "disconnect", "refresh"]},
            "backend": phone_backend_schema(),
            "install_companion": {"type": "boolean"},
            "start_scrcpy": {"type": "boolean"},
            "keep_wireless": {"type": "boolean"}
        })),
        json!({}),
    )
}

fn compact_phone_setup_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["install_companion", "open_settings"]},
        "force_reinstall": {"type": "boolean"},
        "allow_downgrade": {"type": "boolean"},
        "screen": {
            "type": "string",
            "enum": [
                "accessibility",
                "notification_access",
                "overlay_permission",
                "app_details",
                "wireless_debugging",
                "battery_optimization"
            ]
        },
        "package_name": {
            "type": "string",
            "minLength": 1,
            "description": "Target package for app-scoped screens such as app_details."
        }
    }))
}

fn compact_phone_setup_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"operation": {"const": "open_settings"}}, "required": ["operation"]},
                "then": {"required": ["screen"]}
            }
        ]
    })
}

fn compact_phone_pointer_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["tap", "swipe"]},
        "phone_snapshot_id": {
            "type": "string",
            "minLength": 1,
            "description": "Phone snapshot id."
        },
        "x": {"type": "number", "minimum": 0},
        "y": {"type": "number", "minimum": 0},
        "start_x": {"type": "number", "minimum": 0},
        "start_y": {"type": "number", "minimum": 0},
        "end_x": {"type": "number", "minimum": 0},
        "end_y": {"type": "number", "minimum": 0},
        "duration_ms": {"type": "integer", "minimum": 0},
        "use_device_coordinates": {"type": "boolean", "description": "Raw device pixels."}
    }))
}

fn compact_phone_pointer_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"operation": {"const": "tap"}}, "required": ["operation"]},
                "then": {"required": ["x", "y"]}
            },
            {
                "if": {"properties": {"operation": {"const": "swipe"}}, "required": ["operation"]},
                "then": {"required": ["start_x", "start_y", "end_x", "end_y"]}
            }
        ]
    })
}

fn compact_phone_keyboard_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": non_empty_string_schema(),
        "key": non_empty_string_schema()
    }))
}

fn compact_phone_keyboard_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"operation": {"const": "type_text"}}, "required": ["operation"]},
                "then": {"required": ["text"]}
            },
            {
                "if": {"properties": {"operation": {"const": "press_key"}}, "required": ["operation"]},
                "then": {"required": ["key"]}
            }
        ]
    })
}

fn compact_phone_notification_action_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["open", "dismiss", "action"]},
        "event_id": {
            "type": "string",
            "minLength": 1,
            "description": "Notification event id."
        },
        "action_id": {
            "type": "string",
            "minLength": 1,
            "description": "Notification action id."
        }
    }))
}

fn compact_phone_notification_action_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"operation": {"const": "open"}}, "required": ["operation"]},
                "then": {"required": ["event_id"]}
            },
            {
                "if": {"properties": {"operation": {"const": "dismiss"}}, "required": ["operation"]},
                "then": {"required": ["event_id"]}
            },
            {
                "if": {"properties": {"operation": {"const": "action"}}, "required": ["operation"]},
                "then": {"required": ["event_id", "action_id"]}
            }
        ]
    })
}

fn compact_phone_app_action_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["launch", "open_intent"]},
        "package_name": {
            "type": "string",
            "minLength": 1,
            "description": "Android package name."
        },
        "activity": {"type": "string"},
        "intent_uri": {
            "type": "string",
            "minLength": 1,
            "description": "Intent URI or deep link."
        }
    }))
}

fn compact_phone_app_action_constraints() -> Value {
    json!({
        "allOf": [
            {
                "if": {"properties": {"operation": {"const": "launch"}}, "required": ["operation"]},
                "then": {"required": ["package_name"]}
            },
            {
                "if": {"properties": {"operation": {"const": "open_intent"}}, "required": ["operation"]},
                "then": {"required": ["intent_uri"]}
            }
        ]
    })
}

fn compact_phone_app_install_properties() -> Value {
    with_phone_selector(json!({
        "apk_paths": {
            "type": "array",
            "minItems": 1,
            "items": {"type": "string", "minLength": 1},
            "description": "APK paths."
        },
        "apk_path": {
            "type": "string",
            "minLength": 1,
            "description": "Single APK path; apk_paths wins if both are present."
        },
        "mode": {"type": "string", "enum": ["single", "multiple", "multi_package"], "description": "Install strategy hint."},
        "reinstall": {"type": "boolean"},
        "allow_downgrade": {"type": "boolean"},
        "allow_test_apk": {"type": "boolean"},
        "grant_runtime_permissions": {"type": "boolean"}
    }))
}

fn compact_phone_app_install_constraints() -> Value {
    json!({
        "anyOf": [
            {"required": ["apk_paths"]},
            {"required": ["apk_path"]}
        ]
    })
}

fn get_app_state_properties(can_receive_images: bool) -> Value {
    let mut properties = json!({
        "app_id": { "type": "string" },
        "desktop_file_id": { "type": "string" },
        "window_title": { "type": "string" },
        "name": { "type": "string" },
        "detail": {
            "type": "string",
            "enum": ["full", "compact"],
            "description": "Defaults to compact. Use full for verbose element details and full capability data."
        },
        "element_query": {
            "type": "string",
            "maxLength": APP_STATE_MAX_ELEMENT_QUERY_CHARS,
            "description": "Case-insensitive filter over element role/name/description/value/text/states/actions."
        },
        "element_offset": {
            "type": "integer",
            "minimum": 0,
            "description": "Zero-based offset into matching elements."
        },
        "element_limit": {
            "type": "integer",
            "minimum": 0,
            "maximum": APP_STATE_MAX_ELEMENT_LIMIT,
            "description": format!("Maximum matching elements returned. compact defaults to {APP_STATE_DEFAULT_ELEMENT_LIMIT}; 0 keeps metadata only. element_count is the full total.")
        }
    });

    if can_receive_images && let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "capture_screen".to_string(),
            json!({
                "type": "string",
                "enum": ["auto", "if_changed", "always", "never"],
                "description": "Screen-capture policy. Defaults to if_changed. Use always for a fresh frame, never for structure-only loops."
            }),
        );
        property_map.insert(
            "screenshot_delivery".to_string(),
            json!({
                "type": "string",
                "enum": ["path", "inline"],
                "description": "path returns capture.inspection_image_path metadata; inline also attaches the inspection image block."
            }),
        );
    }

    properties
}

fn screenshot_properties(can_receive_images: bool) -> Value {
    let mut properties = window_target_schema();

    if let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "display_id".to_string(),
            json!({
                "type": "string",
                "description": "Exact display_id from environment.displays."
            }),
        );
        property_map.insert(
            "display_name".to_string(),
            json!({
                "type": "string",
                "description": "Display name/connector from environment.displays. Prefer display_id."
            }),
        );
        property_map.insert(
            "display_index".to_string(),
            json!({
                "type": "integer",
                "minimum": 0,
                "description": "Zero-based display index from environment.displays. Prefer display_id."
            }),
        );
        property_map.insert(
            "capture_all_displays".to_string(),
            json!({
                "type": "boolean",
                "description": "Capture the full virtual desktop. Defaults to false."
            }),
        );
    }

    if can_receive_images && let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "screenshot_delivery".to_string(),
            json!({
                "type": "string",
                "enum": ["path", "inline"],
                "description": "path returns capture.inspection_image_path metadata; inline also attaches the inspection image block."
            }),
        );
    }

    properties
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
            "minLength": 1,
            "description": "Exact window_id from list_windows."
        },
        "pid": {
            "type": "integer",
            "minimum": 1,
            "description": "Process ID from list_windows. 0 is ignored."
        },
        "tty": {
            "type": "string",
            "minLength": 1,
            "description": "Terminal tty such as /dev/pts/7 or pts/7."
        },
        "terminal_pid": {
            "type": "integer",
            "minimum": 1,
            "description": "Terminal process ID from list_windows terminal metadata. 0 is ignored."
        },
        "terminal_command": { "type": "string", "minLength": 1 },
        "terminal_cwd": { "type": "string", "minLength": 1 },
        "app_id": { "type": "string", "minLength": 1 },
        "wm_class": { "type": "string", "minLength": 1 },
        "title": { "type": "string", "minLength": 1 }
    })
}

fn window_target_constraint() -> Value {
    json!({"minProperties": 1})
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

#[cfg(test)]
mod annotation_tests {
    use super::{
        InactiveToolReason, McpConfigDiagnostic, McpProcessConfig, build_tool_definitions,
        build_tool_registry, mcp_process_config_from_env,
    };
    use crate::mcp_server::ModelSessionInfo;
    use serde_json::{Value, json};
    use std::{fs, path::PathBuf, sync::Mutex};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// (read_only, destructive, idempotent, open_world) per tool.
    type AnnotationRow = (&'static str, (bool, bool, bool, bool));

    /// Pinned annotation rows per tool.
    ///
    /// Hosts gate per-tool approval on these MCP annotations — Codex "auto"
    /// approval mode silently approves read-only tools and prompts for
    /// destructive/open-world ones, and treats unannotated tools as both.
    /// Changing a row here changes which sky-cua calls hosts auto-approve,
    /// so it must be a deliberate decision.
    const EXPECTED: &[AnnotationRow] = &[
        ("doctor", (true, false, true, false)),
        ("status", (true, false, true, false)),
        ("list_resources", (true, false, true, false)),
        ("observe", (true, false, true, false)),
        ("capture_screen", (true, false, true, false)),
        ("phone_accessibility_tree", (true, false, true, false)),
        ("phone_notifications", (true, false, true, false)),
        ("capture_desktop", (false, false, true, false)),
        ("setup_desktop", (false, false, true, false)),
        ("session_presence", (false, false, true, false)),
        ("activate_window", (false, false, true, false)),
        ("desktop_semantic", (false, false, true, false)),
        ("browser_claim_tab", (false, false, true, false)),
        ("browser_move_mouse", (false, false, true, false)),
        ("phone_connection", (false, false, true, false)),
        ("phone_pair_wireless", (false, false, true, false)),
        ("phone_setup", (false, false, true, false)),
        ("phone_app_force_stop", (false, false, true, false)),
        ("desktop_toggle", (false, false, false, false)),
        ("desktop_scroll", (false, false, false, false)),
        ("browser_scroll", (false, false, false, true)),
        ("desktop_pointer", (false, true, false, false)),
        ("desktop_keyboard", (false, true, false, false)),
        ("desktop_action", (false, true, false, false)),
        ("desktop_set_value", (false, true, true, false)),
        ("browser_open", (false, false, false, true)),
        ("browser_navigate", (false, false, true, true)),
        ("browser_input", (false, true, false, true)),
        ("phone_pointer", (false, true, false, false)),
        ("phone_keyboard", (false, true, false, false)),
        ("phone_notification_action", (false, true, false, false)),
        ("phone_notification_reply", (false, true, false, false)),
        ("phone_app_action", (false, true, false, false)),
        ("phone_app_install", (false, true, false, false)),
        ("browser_eval", (false, true, false, true)),
    ];

    #[test]
    fn every_tool_pins_honest_mcp_annotations() {
        for can_receive_images in [false, true] {
            let tools = build_tool_definitions(can_receive_images, false);
            let tools = tools.as_array().expect("tool definitions array");
            assert!(!tools.is_empty());
            for tool in tools {
                let name = tool["name"].as_str().expect("tool name");
                let annotations = tool
                    .get("annotations")
                    .unwrap_or_else(|| panic!("tool {name} is missing annotations"));
                let expected = EXPECTED
                    .iter()
                    .find(|(expected_name, _)| *expected_name == name)
                    .unwrap_or_else(|| panic!("tool {name} has no pinned annotation row"));
                let (read_only, destructive, idempotent, open_world) = expected.1;
                assert_eq!(
                    annotations["readOnlyHint"].as_bool(),
                    Some(read_only),
                    "{name} readOnlyHint"
                );
                assert_eq!(
                    annotations["destructiveHint"].as_bool(),
                    Some(destructive),
                    "{name} destructiveHint"
                );
                assert_eq!(
                    annotations["idempotentHint"].as_bool(),
                    Some(idempotent),
                    "{name} idempotentHint"
                );
                assert_eq!(
                    annotations["openWorldHint"].as_bool(),
                    Some(open_world),
                    "{name} openWorldHint"
                );
            }
        }
    }

    #[test]
    fn read_only_tools_never_mutate_per_their_own_hints() {
        let tools = build_tool_definitions(true, false);
        for tool in tools.as_array().expect("tool definitions array") {
            let annotations = &tool["annotations"];
            if annotations["readOnlyHint"] == true {
                assert_eq!(
                    annotations["destructiveHint"], false,
                    "read-only tool {} must not be destructive",
                    tool["name"]
                );
            }
        }
    }

    fn process_config(browser_eval_enabled: bool) -> McpProcessConfig {
        McpProcessConfig {
            browser_eval_enabled,
            model_supports_images_override: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn registry_has_expected_name_budget() {
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(false), &model);
        assert_eq!(registry.active_names.len(), 34);
        assert!(registry.contains("observe"));
        assert!(registry.contains("browser_input"));
        let doctor = registry
            .tools
            .as_array()
            .expect("tools array")
            .iter()
            .find(|tool| tool["name"] == "doctor")
            .expect("doctor");
        assert_eq!(doctor["annotations"]["readOnlyHint"], true);
        assert_eq!(doctor["annotations"]["idempotentHint"], true);
        assert!(!registry.contains("browser_eval"));
        assert!(!registry.contains("get_app_state"));
        assert_eq!(registry.inactive_reason("get_app_state"), None);
        assert_eq!(
            registry.inactive_reason("browser_eval"),
            Some(InactiveToolReason::BrowserEvalDisabled)
        );
    }

    #[test]
    fn registry_adds_browser_eval_only_when_enabled() {
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(true), &model);
        assert_eq!(registry.active_names.len(), 35);
        assert!(registry.contains("browser_eval"));
        assert_eq!(registry.inactive_reason("browser_eval"), None);
    }

    #[test]
    fn compact_action_tool_schemas_reject_vague_desktop_actions() {
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        let tools = registry.tools.as_array().expect("tools");
        let tool = |name: &str| -> &Value {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("missing tool {name}"))
        };

        let pointer_schema = &tool("desktop_pointer")["inputSchema"];
        assert_eq!(pointer_schema["required"], json!(["operation"]));
        assert!(
            pointer_schema["description"].is_null(),
            "tool-level description should stay on the MCP tool object"
        );
        assert!(
            pointer_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "click"
                            && constraint["then"]["anyOf"]
                                .as_array()
                                .is_some_and(|any_of| {
                                    any_of
                                        .iter()
                                        .any(|item| item["required"] == json!(["x", "y"]))
                                        && any_of.iter().any(|item| {
                                            item["required"]
                                                == json!(["snapshot_id", "element_index"])
                                        })
                                        && !any_of.iter().any(|item| {
                                            item["required"] == json!(["element_identifier"])
                                        })
                                })
                    })
                }),
            "desktop_pointer click branch must require coordinates or a snapshot selector"
        );
        assert!(
            pointer_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "drag"
                            && constraint["then"]["anyOf"]
                                .as_array()
                                .is_some_and(|any_of| {
                                    !any_of.iter().any(|item| {
                                        item["required"]
                                            == json!([
                                                "snapshot_id",
                                                "element_identifier",
                                                "to_element_index"
                                            ])
                                    })
                                })
                    })
                }),
            "desktop_pointer drag must not advertise unsupported direct backend-ref starts"
        );
        assert!(
            tool("desktop_pointer")["description"]
                .as_str()
                .expect("desktop_pointer description")
                .contains("do not call with only operation")
        );

        assert!(
            tool("activate_window")["inputSchema"]["minProperties"] == 1,
            "activate_window must require at least one window target"
        );
        assert_eq!(
            tool("activate_window")["inputSchema"]["properties"]["window_id"]["minLength"],
            1
        );
        assert_eq!(
            tool("activate_window")["inputSchema"]["properties"]["pid"]["minimum"],
            1
        );

        let list_resources_schema = &tool("list_resources")["inputSchema"];
        assert!(
            list_resources_schema["oneOf"]
                .as_array()
                .is_some_and(|pairs| {
                    pairs.iter().any(|pair| {
                        pair["properties"]["surface"]["const"] == "browser"
                            && pair["properties"]["resource"]["const"] == "tabs"
                    }) && pairs.iter().any(|pair| {
                        pair["properties"]["surface"]["const"] == "phone"
                            && pair["properties"]["resource"]["const"] == "current_app"
                    })
                }),
            "list_resources must constrain surface/resource pairs to dispatchable branches"
        );

        let observe_schema = &tool("observe")["inputSchema"];
        assert!(
            observe_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["surface"]["const"] == "browser"
                            && constraint["then"]["required"] == json!(["tab_id"])
                    })
                }),
            "observe browser branch must require tab_id"
        );

        let capture_schema = &tool("capture_screen")["inputSchema"];
        assert!(
            capture_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["surface"]["const"] == "browser"
                            && constraint["then"]["required"] == json!(["tab_id"])
                    })
                }),
            "capture_screen browser branch must require tab_id"
        );

        let browser_scroll_schema = &tool("browser_scroll")["inputSchema"];
        assert!(
            browser_scroll_schema["allOf"]
                .as_array()
                .is_some_and(|all_of| {
                    all_of.iter().any(|constraint| {
                        constraint["anyOf"].as_array().is_some_and(|any_of| {
                            any_of
                                .iter()
                                .any(|item| item["required"] == json!(["delta_x"]))
                                && any_of
                                    .iter()
                                    .any(|item| item["required"] == json!(["delta_y"]))
                        })
                    })
                }),
            "browser_scroll must require at least one scroll delta"
        );
        assert!(
            browser_scroll_schema["allOf"]
                .as_array()
                .is_some_and(|all_of| {
                    all_of.iter().any(|constraint| {
                        constraint["if"]["required"] == json!(["x"])
                            && constraint["then"]["required"] == json!(["y"])
                    }) && all_of.iter().any(|constraint| {
                        constraint["if"]["required"] == json!(["y"])
                            && constraint["then"]["required"] == json!(["x"])
                    })
                }),
            "browser_scroll must require x/y together"
        );

        assert_eq!(
            tool("browser_navigate")["inputSchema"]["properties"]["url"]["pattern"],
            "^(https?://|about:blank$)"
        );
        assert_eq!(
            tool("browser_input")["inputSchema"]["properties"]["tab_id"]["minLength"],
            1,
            "browser tools must reject empty tab_id"
        );
        assert_eq!(
            tool("browser_input")["inputSchema"]["properties"]["text"]["minLength"],
            1,
            "browser_input type_text must reject empty text"
        );
        assert_eq!(
            tool("browser_input")["inputSchema"]["properties"]["key"]["minLength"],
            1,
            "browser_input press_key must reject empty key"
        );

        let action_schema = &tool("desktop_action")["inputSchema"];
        assert_eq!(action_schema["required"], json!(["operation"]));
        assert_eq!(
            action_schema["properties"]["action_name"]["minLength"], 1,
            "desktop_action perform_action must reject empty action names"
        );
        assert_eq!(
            action_schema["properties"]["snapshot_id"]["minLength"], 1,
            "snapshot-bound desktop selectors must reject empty snapshot ids"
        );
        assert_eq!(
            action_schema["properties"]["element_identifier"]["minLength"], 1,
            "direct desktop element identifiers must reject empty strings"
        );
        assert_eq!(
            action_schema["properties"]["name"]["minLength"], 1,
            "desktop name selectors must reject empty strings"
        );
        assert_eq!(
            action_schema["properties"]["text"]["minLength"], 1,
            "desktop text selectors must reject empty strings"
        );
        assert!(
            action_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["anyOf"].as_array().is_some_and(|any_of| {
                            any_of.iter().any(|item| {
                                item["required"] == json!(["snapshot_id", "element_index"])
                            }) && any_of
                                .iter()
                                .any(|item| item["required"] == json!(["element_identifier"]))
                                && any_of
                                    .iter()
                                    .any(|item| item["required"] == json!(["snapshot_id", "name"]))
                                && any_of
                                    .iter()
                                    .any(|item| item["required"] == json!(["snapshot_id", "text"]))
                        })
                    })
                }),
            "desktop_action must require snapshot_id for snapshot-bound selectors"
        );
        assert!(
            action_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "perform_action"
                            && constraint["then"]["anyOf"]
                                .as_array()
                                .is_some_and(|any_of| {
                                    any_of
                                        .iter()
                                        .any(|item| item["required"] == json!(["action_name"]))
                                })
                    })
                }),
            "desktop_action perform_action branch must require action_name or action_index"
        );

        let keyboard_schema = &tool("desktop_keyboard")["inputSchema"];
        assert_eq!(
            keyboard_schema["properties"]["text"]["minLength"], 1,
            "desktop_keyboard type_text must reject empty text"
        );
        assert_eq!(
            keyboard_schema["properties"]["key"]["minLength"], 1,
            "desktop_keyboard press_key must reject empty key"
        );
        assert!(
            keyboard_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "press_key"
                            && constraint["then"]["required"] == json!(["key"])
                    })
                }),
            "desktop_keyboard press_key branch must require key"
        );

        assert_eq!(
            tool("desktop_semantic")["inputSchema"]["required"],
            json!(["operation"]),
            "desktop_semantic must allow non-index selectors"
        );
        assert_eq!(
            tool("desktop_toggle")["inputSchema"]["required"],
            json!([]),
            "desktop_toggle must allow non-index selectors"
        );
        assert_eq!(
            tool("desktop_scroll")["inputSchema"]["required"],
            json!(["direction"]),
            "desktop_scroll must allow non-index selectors"
        );
        assert!(
            tool("desktop_scroll")["inputSchema"]["properties"]["amount"].is_null(),
            "desktop_scroll must not advertise ignored amount"
        );
        assert!(
            tool("desktop_scroll")["inputSchema"]["properties"]["pages"].is_object(),
            "desktop_scroll should expose the canonical pages field"
        );
        assert!(
            tool("desktop_scroll")["inputSchema"]["anyOf"]
                .as_array()
                .is_some_and(|any_of| {
                    any_of
                        .iter()
                        .any(|item| item["required"] == json!(["snapshot_id", "element_index"]))
                        && !any_of
                            .iter()
                            .any(|item| item["required"] == json!(["element_identifier"]))
                }),
            "desktop_scroll must only advertise snapshot-resolved semantic targets"
        );
        assert_eq!(
            tool("desktop_set_value")["inputSchema"]["required"],
            json!(["value"]),
            "desktop_set_value must allow non-index selectors"
        );

        let phone_setup_schema = &tool("phone_setup")["inputSchema"];
        assert_eq!(
            phone_setup_schema["properties"]["screen"]["enum"],
            json!([
                "accessibility",
                "notification_access",
                "overlay_permission",
                "app_details",
                "wireless_debugging",
                "battery_optimization"
            ])
        );
        assert!(
            phone_setup_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "open_settings"
                            && constraint["then"]["required"] == json!(["screen"])
                    })
                }),
            "phone_setup open_settings branch must require screen"
        );

        let phone_pointer_schema = &tool("phone_pointer")["inputSchema"];
        for coordinate in ["x", "y", "start_x", "start_y", "end_x", "end_y"] {
            assert_eq!(
                phone_pointer_schema["properties"][coordinate]["minimum"], 0,
                "phone_pointer {coordinate} must reject negative coordinates"
            );
        }
        assert!(
            phone_pointer_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "tap"
                            && constraint["then"]["required"] == json!(["x", "y"])
                    })
                }),
            "phone_pointer tap branch must require coordinates"
        );
        assert!(
            phone_pointer_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "swipe"
                            && constraint["then"]["required"]
                                == json!(["start_x", "start_y", "end_x", "end_y"])
                    })
                }),
            "phone_pointer swipe branch must require start/end coordinates"
        );

        let phone_keyboard_schema = &tool("phone_keyboard")["inputSchema"];
        assert_eq!(
            phone_keyboard_schema["properties"]["text"]["minLength"], 1,
            "phone_keyboard type_text must reject empty text"
        );
        assert_eq!(
            phone_keyboard_schema["properties"]["key"]["minLength"], 1,
            "phone_keyboard press_key must reject empty key"
        );
        assert!(
            phone_keyboard_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "type_text"
                            && constraint["then"]["required"] == json!(["text"])
                    })
                }),
            "phone_keyboard type_text branch must require text"
        );

        let phone_notification_schema = &tool("phone_notification_action")["inputSchema"];
        assert_eq!(
            phone_notification_schema["properties"]["event_id"]["minLength"], 1,
            "phone_notification_action must reject empty event ids"
        );
        assert_eq!(
            phone_notification_schema["properties"]["action_id"]["minLength"], 1,
            "phone_notification_action must reject empty action ids"
        );
        assert!(
            phone_notification_schema["allOf"]
                .as_array()
                .is_some_and(|constraints| {
                    constraints.iter().any(|constraint| {
                        constraint["if"]["properties"]["operation"]["const"] == "action"
                            && constraint["then"]["required"] == json!(["event_id", "action_id"])
                    })
                }),
            "phone_notification_action action branch must require event_id and action_id"
        );

        let phone_install_schema = &tool("phone_app_install")["inputSchema"];
        assert_eq!(
            phone_install_schema["properties"]["apk_paths"]["minItems"], 1,
            "phone_app_install apk_paths must be non-empty"
        );
        assert_eq!(
            phone_install_schema["properties"]["apk_path"]["minLength"], 1,
            "phone_app_install apk_path alias must be non-empty"
        );
        assert!(
            phone_install_schema["properties"]["grant_runtime_permissions"].is_object(),
            "phone_app_install must expose all supported handler options"
        );
        assert_eq!(
            tool("phone_notification_reply")["inputSchema"]["properties"]["text"]["minLength"],
            1,
            "phone_notification_reply must reject empty reply text"
        );
        assert_eq!(
            tool("phone_app_action")["inputSchema"]["properties"]["package_name"]["minLength"],
            1,
            "phone_app_action launch must reject empty package names"
        );
        assert_eq!(
            tool("phone_app_action")["inputSchema"]["properties"]["intent_uri"]["minLength"],
            1,
            "phone_app_action open_intent must reject empty intent URIs"
        );
        assert_eq!(
            tool("browser_eval")["inputSchema"]["properties"]["expression"]["minLength"],
            1,
            "browser_eval must reject empty expressions"
        );
    }

    #[test]
    fn mcp_runtime_config_invalid_values_fallback() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        unsafe {
            std::env::set_var("SKY_CUA_BROWSER_EVAL", "perhaps");
            std::env::set_var("SKY_CUA_MODEL_SUPPORTS_IMAGES", "sometimes");
        }
        let config = mcp_process_config_from_env();
        unsafe {
            std::env::remove_var("SKY_CUA_BROWSER_EVAL");
            std::env::remove_var("SKY_CUA_MODEL_SUPPORTS_IMAGES");
        }
        assert!(!config.browser_eval_enabled);
        assert_eq!(config.model_supports_images_override, None);
        assert_eq!(
            config.diagnostics,
            vec![
                McpConfigDiagnostic::InvalidBrowserEval {
                    value: "perhaps".to_string()
                },
                McpConfigDiagnostic::InvalidModelSupportsImages {
                    value: "sometimes".to_string()
                }
            ]
        );
    }

    #[test]
    fn browser_eval_runtime_config_matches_service_truthy_values() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        unsafe {
            std::env::set_var("SKY_CUA_BROWSER_EVAL", "enabled");
            std::env::remove_var("SKY_CUA_MODEL_SUPPORTS_IMAGES");
        }
        let config = mcp_process_config_from_env();
        unsafe {
            std::env::remove_var("SKY_CUA_BROWSER_EVAL");
        }
        assert!(
            !config.browser_eval_enabled,
            "browser eval advertisement must use the same on/1/true truthy values as service execution"
        );
        assert!(config.diagnostics.iter().any(|diagnostic| {
            matches!(diagnostic, McpConfigDiagnostic::InvalidBrowserEval { value } if value == "enabled")
        }));
    }

    #[test]
    fn mcp_tool_surface_matrix_fixture_matches_generated_registry() {
        assert_fixture_matches(
            "mcp_tool_surface_matrix.json",
            include_str!("../../tests/fixtures/mcp_tool_surface_matrix.json"),
            generated_surface_matrix(),
        );
    }

    #[test]
    fn tool_contract_fixture_matches_generated_registry() {
        let generated = generated_tool_contract();
        let tools = generated["tools"].as_array().expect("contract tools");
        let contract_names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("contract tool name"))
            .collect();
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        let advertised_names: Vec<&str> = registry
            .tools
            .as_array()
            .expect("tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect();
        assert_eq!(contract_names, advertised_names);

        assert_fixture_matches(
            "tool_contract.json",
            include_str!("../../tests/fixtures/tool_contract.json"),
            generated,
        );
    }

    #[test]
    fn call_cases_fixture_matches_contract() {
        let generated = generated_call_cases();
        let cases = generated["cases"].as_array().expect("call cases");
        assert!(
            cases.iter().all(|case| case["valid"].is_object()),
            "every valid call case must be an object"
        );
        assert!(
            cases.iter().all(|case| case["invalid"].is_object()),
            "every invalid call case must be an object"
        );
        let contract_branch_count: usize = generated_tool_contract()["tools"]
            .as_array()
            .expect("contract tools")
            .iter()
            .map(|tool| tool["branches"].as_array().expect("branches").len())
            .sum();
        assert_eq!(cases.len(), contract_branch_count);

        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        for case in cases {
            let tool_name = case["tool"].as_str().expect("case tool");
            let schema = registry
                .tools
                .as_array()
                .expect("tools")
                .iter()
                .find(|tool| tool["name"] == tool_name)
                .unwrap_or_else(|| panic!("missing schema for {tool_name}"));
            assert!(
                schema_accepts(&schema["inputSchema"], &case["valid"]),
                "call case {}/{} is not valid for its generated schema: {}",
                tool_name,
                case["branch"].as_str().expect("case branch"),
                case["valid"]
            );
            assert!(
                !schema_accepts(&schema["inputSchema"], &case["invalid"]),
                "call case {}/{} invalid sample was accepted by its generated schema: {}",
                tool_name,
                case["branch"].as_str().expect("case branch"),
                case["invalid"]
            );
        }

        assert_fixture_matches(
            "call_cases.json",
            include_str!("../../tests/fixtures/call_cases.json"),
            generated,
        );
    }

    #[test]
    fn call_cases_match_canonical_dispatcher() {
        for case in generated_call_cases()["cases"]
            .as_array()
            .expect("call cases")
        {
            let tool_name = case["tool"].as_str().expect("case tool");
            let expected_branch = case["branch"].as_str().expect("case branch");
            let expected_handler = case["handler_id"].as_str().expect("case handler");
            let call = crate::mcp_tools::canonical_handler_call(tool_name, case["valid"].clone())
                .unwrap_or_else(|error| {
                    panic!("call case {tool_name}/{expected_branch} did not dispatch: {error}")
                });
            assert_eq!(call.branch, expected_branch, "{tool_name} branch");
            assert_eq!(
                call.handler_name, expected_handler,
                "{tool_name}/{expected_branch} handler"
            );
        }
    }

    fn schema_accepts(schema: &Value, instance: &Value) -> bool {
        let Some(schema) = schema.as_object() else {
            return true;
        };
        if let Some(expected_type) = schema.get("type")
            && !schema_type_accepts(expected_type, instance)
        {
            return false;
        }
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            let Some(instance_object) = instance.as_object() else {
                return false;
            };
            if !required.iter().all(|field| {
                field
                    .as_str()
                    .is_some_and(|field| instance_object.contains_key(field))
            }) {
                return false;
            }
        }
        if let Some(expected) = schema.get("const")
            && instance != expected
        {
            return false;
        }
        if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
            && !allowed.iter().any(|allowed| allowed == instance)
        {
            return false;
        }
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
            let Some(value) = instance.as_f64() else {
                return false;
            };
            if value < minimum {
                return false;
            }
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
            let Some(value) = instance.as_f64() else {
                return false;
            };
            if value > maximum {
                return false;
            }
        }
        if let Some(minimum) = schema.get("exclusiveMinimum").and_then(Value::as_f64) {
            let Some(value) = instance.as_f64() else {
                return false;
            };
            if value <= minimum {
                return false;
            }
        }
        if let Some(maximum) = schema.get("exclusiveMaximum").and_then(Value::as_f64) {
            let Some(value) = instance.as_f64() else {
                return false;
            };
            if value >= maximum {
                return false;
            }
        }
        if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64) {
            let Some(value) = instance.as_str() else {
                return false;
            };
            if value.chars().count() < minimum as usize {
                return false;
            }
        }
        if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64) {
            let Some(value) = instance.as_str() else {
                return false;
            };
            if value.chars().count() > maximum as usize {
                return false;
            }
        }
        if let Some(minimum) = schema.get("minItems").and_then(Value::as_u64) {
            let Some(value) = instance.as_array() else {
                return false;
            };
            if value.len() < minimum as usize {
                return false;
            }
        }
        if let Some(maximum) = schema.get("maxItems").and_then(Value::as_u64) {
            let Some(value) = instance.as_array() else {
                return false;
            };
            if value.len() > maximum as usize {
                return false;
            }
        }
        if let Some(minimum) = schema.get("minProperties").and_then(Value::as_u64) {
            let Some(value) = instance.as_object() else {
                return false;
            };
            if value.len() < minimum as usize {
                return false;
            }
        }
        if let Some(maximum) = schema.get("maxProperties").and_then(Value::as_u64) {
            let Some(value) = instance.as_object() else {
                return false;
            };
            if value.len() > maximum as usize {
                return false;
            }
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            let Some(value) = instance.as_str() else {
                return false;
            };
            if pattern == "^(https?://|about:blank$)"
                && !(value == "about:blank"
                    || value.starts_with("http://")
                    || value.starts_with("https://"))
            {
                return false;
            }
        }
        if let Some(rejected) = schema.get("not")
            && schema_accepts(rejected, instance)
        {
            return false;
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object)
            && let Some(instance_object) = instance.as_object()
        {
            for (name, property_schema) in properties {
                if let Some(property_value) = instance_object.get(name)
                    && !schema_accepts(property_schema, property_value)
                {
                    return false;
                }
            }
            if schema.get("additionalProperties") == Some(&Value::Bool(false))
                && !instance_object
                    .keys()
                    .all(|name| properties.contains_key(name))
            {
                return false;
            }
        }
        if let Some(item_schema) = schema.get("items")
            && let Some(instance_array) = instance.as_array()
            && !instance_array
                .iter()
                .all(|item| schema_accepts(item_schema, item))
        {
            return false;
        }
        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array)
            && !all_of.iter().all(|schema| schema_accepts(schema, instance))
        {
            return false;
        }
        if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array)
            && !any_of.iter().any(|schema| schema_accepts(schema, instance))
        {
            return false;
        }
        if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array)
            && one_of
                .iter()
                .filter(|schema| schema_accepts(schema, instance))
                .count()
                != 1
        {
            return false;
        }
        if let Some(if_schema) = schema.get("if")
            && schema_accepts(if_schema, instance)
            && let Some(then_schema) = schema.get("then")
            && !schema_accepts(then_schema, instance)
        {
            return false;
        }
        true
    }

    fn schema_type_accepts(expected_type: &Value, instance: &Value) -> bool {
        match expected_type {
            Value::String(expected_type) => schema_single_type_accepts(expected_type, instance),
            Value::Array(expected_types) => expected_types.iter().any(|expected_type| {
                expected_type.as_str().is_some_and(|expected_type| {
                    schema_single_type_accepts(expected_type, instance)
                })
            }),
            _ => true,
        }
    }

    fn schema_single_type_accepts(expected_type: &str, instance: &Value) -> bool {
        match expected_type {
            "array" => instance.is_array(),
            "boolean" => instance.is_boolean(),
            "integer" => instance
                .as_i64()
                .or_else(|| {
                    instance
                        .as_u64()
                        .and_then(|value| i64::try_from(value).ok())
                })
                .is_some(),
            "null" => instance.is_null(),
            "number" => instance.is_number(),
            "object" => instance.is_object(),
            "string" => instance.is_string(),
            _ => true,
        }
    }

    fn assert_fixture_matches(name: &str, expected: &str, generated: Value) {
        let generated_text =
            serde_json::to_string_pretty(&generated).expect("generated fixture json");
        if std::env::var_os("SKY_CUA_UPDATE_MCP_FIXTURES").is_some() {
            let path = fixture_path(name);
            fs::write(&path, format!("{generated_text}\n"))
                .unwrap_or_else(|error| panic!("failed to update {}: {error}", path.display()));
            return;
        }
        let expected: Value = serde_json::from_str(expected)
            .unwrap_or_else(|error| panic!("{name} fixture is invalid json: {error}"));
        let generated: Value =
            serde_json::from_str(&generated_text).expect("generated fixture should parse");
        assert_eq!(
            expected, generated,
            "{name} is stale; rerun with SKY_CUA_UPDATE_MCP_FIXTURES=1"
        );
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn generated_surface_matrix() -> Value {
        let mut rows = Vec::new();
        for can_receive_images in [false, true] {
            for browser_eval_enabled in [false, true] {
                let model = model_with_image_capability(can_receive_images);
                let registry = build_tool_registry(&process_config(browser_eval_enabled), &model);
                let tools_list = registry.tools_list_result();
                let serialized = serde_json::to_string(&tools_list).expect("tools list json");
                rows.push(json!({
                    "surface": "canonical",
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

    fn generated_tool_contract() -> Value {
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        let advertised = registry.tools.as_array().expect("tools");
        let tools: Vec<Value> = canonical_contract_tools()
            .into_iter()
            .map(|mut contract| {
                let name = contract["name"].as_str().expect("contract name");
                let public = advertised
                    .iter()
                    .find(|tool| tool["name"] == name)
                    .unwrap_or_else(|| panic!("contract references unadvertised tool {name}"));
                let object = contract.as_object_mut().expect("contract object");
                object.insert("annotations".to_string(), public["annotations"].clone());
                object.insert("input_schema".to_string(), public["inputSchema"].clone());
                object.insert("content_policy".to_string(), json!("canonical_rewrite"));
                object.insert("structured_policy".to_string(), json!("canonical_envelope"));
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
            "surface": "canonical",
            "default_tool_count": 34,
            "eval_tool_count": 35,
            "tools": tools
        })
    }

    fn canonical_contract_tools() -> Vec<Value> {
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
                        json!({"surface": "phone", "resource": "apps"}),
                    ),
                    branch(
                        "phone/current_app",
                        "phone_app_current",
                        json!({"surface": "phone", "resource": "current_app"}),
                    ),
                ],
            ),
            contract_tool(
                "observe",
                vec![
                    branch("desktop", "get_app_state", json!({"surface": "desktop"})),
                    branch(
                        "browser",
                        "browser_snapshot",
                        json!({"surface": "browser", "tab_id": "tab-1"}),
                    ),
                    branch("phone", "phone_observe", json!({"surface": "phone"})),
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
                    branch("phone", "phone_screenshot", json!({"surface": "phone"})),
                ],
            ),
            contract_tool(
                "phone_accessibility_tree",
                vec![branch("default", "phone_accessibility_tree", json!({}))],
            ),
            contract_tool(
                "phone_notifications",
                vec![branch("default", "phone_notifications", json!({}))],
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
                        json!({"operation": "disconnect"}),
                    ),
                    branch(
                        "refresh",
                        "phone_refresh_capabilities",
                        json!({"operation": "refresh"}),
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
                        json!({"operation": "install_companion"}),
                    ),
                    branch(
                        "open_settings",
                        "phone_open_settings",
                        json!({"operation": "open_settings", "screen": "accessibility"}),
                    ),
                ],
            ),
            contract_tool(
                "phone_app_force_stop",
                vec![branch(
                    "default",
                    "phone_app_force_stop",
                    json!({"package_name": "com.example.app"}),
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
                        json!({"operation": "tap", "x": 1, "y": 1}),
                    ),
                    branch(
                        "swipe",
                        "phone_swipe",
                        json!({"operation": "swipe", "start_x": 1, "start_y": 1, "end_x": 2, "end_y": 2}),
                    ),
                ],
            ),
            contract_tool(
                "phone_keyboard",
                vec![
                    branch(
                        "type_text",
                        "phone_type_text",
                        json!({"operation": "type_text", "text": "hello"}),
                    ),
                    branch(
                        "press_key",
                        "phone_press_key",
                        json!({"operation": "press_key", "key": "BACK"}),
                    ),
                ],
            ),
            contract_tool(
                "phone_notification_action",
                vec![
                    branch(
                        "open",
                        "phone_notification_open",
                        json!({"operation": "open", "event_id": "event-1"}),
                    ),
                    branch(
                        "dismiss",
                        "phone_notification_dismiss",
                        json!({"operation": "dismiss", "event_id": "event-1"}),
                    ),
                    branch(
                        "action",
                        "phone_notification_action",
                        json!({"operation": "action", "event_id": "event-1", "action_id": "action-1"}),
                    ),
                ],
            ),
            contract_tool(
                "phone_notification_reply",
                vec![branch(
                    "default",
                    "phone_notification_reply",
                    json!({"event_id": "event-1", "action_id": "reply-1", "text": "reply"}),
                )],
            ),
            contract_tool(
                "phone_app_action",
                vec![
                    branch(
                        "launch",
                        "phone_app_launch",
                        json!({"operation": "launch", "package_name": "com.example.app"}),
                    ),
                    branch(
                        "open_intent",
                        "phone_app_open_intent",
                        json!({"operation": "open_intent", "intent_uri": "intent://example"}),
                    ),
                ],
            ),
            contract_tool(
                "phone_app_install",
                vec![branch(
                    "default",
                    "phone_app_install",
                    json!({"apk_path": "/tmp/example.apk"}),
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

    fn branch(
        name: &'static str,
        handler_id: &'static str,
        minimal_valid_arguments: Value,
    ) -> Value {
        json!({
            "name": name,
            "handler_id": handler_id,
            "minimal_valid_arguments": minimal_valid_arguments,
            "expected_errors": ["InvalidRequest", "FeatureDisabled", "UnknownTool"]
        })
    }

    fn generated_call_cases() -> Value {
        let mut cases = Vec::new();
        for tool in canonical_contract_tools() {
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
        if invalid.contains_key("operation") {
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
}
