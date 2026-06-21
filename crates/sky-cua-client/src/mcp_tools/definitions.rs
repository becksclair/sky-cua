//! MCP tool definitions: the host-facing tool registry with input schemas
//! and annotations. Split from `mcp_tools.rs` along the contract-family
//! boundary; dispatch and response shaping stay in the parent module.

use std::sync::LazyLock;

use serde_json::{Value, json};

use crate::app_state::{
    APP_STATE_DEFAULT_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_QUERY_CHARS,
};
use crate::mcp_server::ModelSessionInfo;

use super::annotations::{
    LOCAL_DESTRUCTIVE_ACTION, LOCAL_NAVIGATION_ACTION, LOCAL_STATEFUL_ACTION, READ_ONLY_TOOL,
    ToolAnnotations,
};
use super::browser;
use super::phone;

pub(crate) fn tool_definitions(model: &ModelSessionInfo) -> Value {
    let index = usize::from(model.can_receive_images());
    TOOL_DEFINITIONS_CACHE[index].clone()
}

pub(crate) fn tools_list_result(model: &ModelSessionInfo) -> Value {
    json!({
        "tools": tool_definitions(model)
    })
}

static TOOL_DEFINITIONS_CACHE: LazyLock<[Value; 2]> =
    LazyLock::new(|| [build_tool_definitions(false), build_tool_definitions(true)]);

pub(crate) fn build_tool_definitions(can_receive_images: bool) -> Value {
    let point_description = "With snapshot_id from a captured get_app_state or screenshot result, x/y are pixels in that snapshot; otherwise live screen coordinates.";
    let drag_point_description = "With snapshot_id from a captured get_app_state or screenshot result, coordinates are pixels in that snapshot; otherwise live screen coordinates.";
    let mut tools = json!([
        {
            "name": "doctor",
            "description": "Report desktop readiness: environment, session-env repair, semantic tree, capture, windows, input, browser, and presence diagnostics.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "setup_accessibility",
            "description": "Enable toolkit accessibility for AT-SPI semantic trees and return before/after readiness. Target apps may need restart.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "setup_window_targeting",
            "description": "Install/enable the bundled GNOME window-control extension and report exact window-targeting status.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "list_apps",
            "description": "List accessible desktop apps from window/accessibility backends plus session-env diagnostics.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "list_windows",
            "description": "List desktop windows with window_id, backend, bounds, focus, display, and terminal metadata. Use for targeted screenshots or exact activation.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "focused_window",
            "description": "Return the focused desktop window from native windowing backends, including display placement when known.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "activate_window",
            "description": "Activate a desktop window by window_id or selector. Reports unsupported backends honestly. For visual inspection, use screenshot with the same target.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": window_target_schema(),
                "additionalProperties": false
            }
        },
        {
            "name": "screenshot",
            "description": if can_receive_images {
                "Capture a fresh visual frame. Default: primary display. Window targets activate/focus-verify then crop; display_* captures one monitor; capture_all_displays captures the virtual desktop. Use the returned snapshot_id for pixel actions; if capture source geometry is missing, retry the targeted screenshot once before broad fallbacks."
            } else {
                "Capture a fresh visual frame and return capture.inspection_image_path plus snapshot_id. Default: primary display. Window targets activate/focus-verify then crop; display_* captures one monitor; capture_all_displays captures the virtual desktop. Use the returned snapshot_id for pixel actions; if capture source geometry is missing, retry the targeted screenshot once before broad fallbacks."
            },
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": screenshot_properties(can_receive_images),
                "additionalProperties": false
            }
        },
        {
            "name": "get_app_state",
            "description": if can_receive_images {
                format!("Return token-bounded desktop state: app identity, displays, diagnostics, accessibility elements, text/value readback, and optional capture metadata. Defaults to compact detail and {APP_STATE_DEFAULT_ELEMENT_LIMIT} elements; use element_query/offset/limit or detail=full. Inspect capture.inspection_image_path when present.")
            } else {
                format!("Return token-bounded desktop state: app identity, displays, diagnostics, accessibility elements, text/value readback, and optional capture metadata. Defaults to compact detail and {APP_STATE_DEFAULT_ELEMENT_LIMIT} elements; use element_query/offset/limit or detail=full. Inspect capture.inspection_image_path when present.")
            },
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": get_app_state_properties(can_receive_images),
                "additionalProperties": false
            }
        },
        {
            "name": "hold_session",
            "description": "Hold session presence by inhibiting lock and/or suspend; optionally unlock first.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": session_presence_hold_properties(true),
                "additionalProperties": false
            }
        },
        {
            "name": "unlock_session",
            "description": "Unlock the session when supported, then hold lock/suspend inhibitors.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": session_presence_hold_properties(false),
                "additionalProperties": false
            }
        },
        {
            "name": "release_session",
            "description": "Release session-presence inhibitors; optionally re-lock when supported.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "relock": {
                        "type": "boolean",
                        "description": "Re-lock after releasing inhibitors. Defaults to false."
                    }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "session_presence_status",
            "description": "Report session-presence support and current lock/suspend inhibitor state.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        semantic_element_tool(
            "focus_element",
            "Move semantic focus to an accessibility element from the latest snapshot.",
            LOCAL_NAVIGATION_ACTION,
        ),
        semantic_element_tool(
            "activate_element",
            "Run an element's default semantic action, such as pressing a button or opening a menu.",
            LOCAL_DESTRUCTIVE_ACTION,
        ),
        semantic_element_tool(
            "select_element",
            "Select a tab, list item, radio item, row, or similar selectable element.",
            LOCAL_NAVIGATION_ACTION,
        ),
        semantic_element_tool(
            "expand_element",
            "Expand a collapsed menu, combo box, disclosure, tree item, or similar element.",
            LOCAL_NAVIGATION_ACTION,
        ),
        semantic_element_tool(
            "collapse_element",
            "Collapse an expanded menu, combo box, disclosure, tree item, or similar element.",
            LOCAL_NAVIGATION_ACTION,
        ),
        semantic_element_tool(
            "toggle_element",
            "Toggle a checkbox, switch, toggle button, or similar binary element.",
            LOCAL_STATEFUL_ACTION,
        ),
        action_tool(
            "click",
            "Click a snapshot element_index, or x/y coordinates.",
            LOCAL_DESTRUCTIVE_ACTION,
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "x": coordinate_schema(&format!("X coordinate. {point_description}")),
                "y": coordinate_schema(&format!("Y coordinate. {point_description}"))
            }),
            json!([]),
        ),
        action_tool(
            "perform_action",
            "Invoke a named/indexed AT-SPI action. Prefer dedicated tools for common focus, activation, selection, expand/collapse, and toggles.",
            LOCAL_DESTRUCTIVE_ACTION,
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "element_identifier": {
                    "type": "string",
                    "description": "Direct backend_ref from get_app_state; bypasses element_index lookup."
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
                    "description": "Zero-based AT-SPI action index. Defaults to 0."
                },
                "action_name": {
                    "type": "string",
                    "description": "AT-SPI action name from the target element."
                },
                "action": {
                    "type": "string",
                    "description": "Compatibility alias: action name or numeric action-index string."
                }
            }),
            json!([]),
        ),
        action_tool(
            "perform_secondary_action",
            "Perform secondary/context click on a snapshot element_index or x/y coordinates.",
            LOCAL_DESTRUCTIVE_ACTION,
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "x": coordinate_schema(&format!("X coordinate. {point_description}")),
                "y": coordinate_schema(&format!("Y coordinate. {point_description}")),
                "action": { "type": "string" }
            }),
            json!([]),
        ),
        action_tool(
            "scroll",
            "Scroll inside a snapshot element, or the focused area.",
            LOCAL_STATEFUL_ACTION,
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down"]
                },
                "pages": { "type": "integer", "minimum": 1 }
            }),
            json!(["direction"]),
        ),
        action_tool(
            "drag",
            "Drag from one element/point to another.",
            LOCAL_DESTRUCTIVE_ACTION,
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "x": coordinate_schema(&format!("Drag start X coordinate. {drag_point_description}")),
                "y": coordinate_schema(&format!("Drag start Y coordinate. {drag_point_description}")),
                "from_x": coordinate_schema(&format!("Drag start X coordinate. {drag_point_description}")),
                "from_y": coordinate_schema(&format!("Drag start Y coordinate. {drag_point_description}")),
                "to_x": coordinate_schema(&format!("Drag end X coordinate. {drag_point_description}")),
                "to_y": coordinate_schema(&format!("Drag end Y coordinate. {drag_point_description}")),
                "to_element_index": { "type": "integer", "minimum": 0 }
            }),
            json!([]),
        ),
        action_tool(
            "type_text",
            "Type literal text into the focused control. Optional snapshot/window target activates first.",
            LOCAL_DESTRUCTIVE_ACTION,
            keyboard_target_properties(json!({
                "text": { "type": "string" }
            })),
            json!(["text"]),
        ),
        action_tool(
            "press_key",
            "Press a key or chord in the focused control. Optional snapshot/window target activates first.",
            LOCAL_DESTRUCTIVE_ACTION,
            keyboard_target_properties(json!({
                "key": { "type": "string" }
            })),
            json!(["key"]),
        ),
        action_tool(
            "set_value",
            "Set an editable element through a proven semantic write path. Target by element_index, element_identifier, or selector; verify with get_app_state readback.",
            // Overwrites existing content (destructive), but writing the
            // same value twice converges to the same state (idempotent).
            ToolAnnotations {
                read_only: false,
                destructive: true,
                idempotent: true,
                open_world: false,
            },
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "element_identifier": {
                    "type": "string",
                    "description": "Direct backend_ref from get_app_state; bypasses element_index lookup."
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
    ]);

    let tool_array = tools
        .as_array_mut()
        .expect("tool definition registry should be a JSON array");
    browser::push_tool_definitions(tool_array, browser::browser_eval_enabled());
    phone::push_tool_definitions(tool_array);

    tools
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

fn session_presence_hold_properties(include_unlock: bool) -> Value {
    let mut properties = json!({
        "inhibit_lock": {
            "type": "boolean",
            "description": "Hold the lock/screensaver inhibitor. Defaults to true."
        },
        "inhibit_suspend": {
            "type": "boolean",
            "description": "Hold the suspend inhibitor. Defaults to true."
        }
    });

    if include_unlock && let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "unlock".to_string(),
            json!({
                "type": "boolean",
                "description": "Unlock before holding inhibitors when supported. Defaults to false."
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
            "description": "Exact window_id from list_windows."
        },
        "pid": {
            "type": "integer",
            "minimum": 0,
            "description": "Process ID from list_windows. 0 is ignored."
        },
        "tty": {
            "type": "string",
            "description": "Terminal tty such as /dev/pts/7 or pts/7."
        },
        "terminal_pid": {
            "type": "integer",
            "minimum": 0,
            "description": "Terminal process ID from list_windows terminal metadata. 0 is ignored."
        },
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

fn semantic_element_tool(name: &str, description: &str, annotations: ToolAnnotations) -> Value {
    action_tool(
        name,
        description,
        annotations,
        json!({
            "element_index": {
                "type": "integer",
                "minimum": 0,
                "description": "Element index from the latest get_app_state snapshot."
            },
            "element_identifier": {
                "type": "string",
                "description": "Direct backend_ref from get_app_state; bypasses element_index lookup."
            },
            "role": {
                "type": "string",
                "description": "Semantic selector role from the latest snapshot."
            },
            "name": {
                "type": "string",
                "description": "Semantic selector name from the latest snapshot."
            },
            "text": {
                "type": "string",
                "description": "Selector text matched against name, description, or value."
            },
            "states": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Selector states; all listed states must match."
            }
        }),
        json!([]),
    )
}

fn action_tool(
    name: &str,
    description: &str,
    annotations: ToolAnnotations,
    mut properties: Value,
    required: Value,
) -> Value {
    let Some(property_map) = properties.as_object_mut() else {
        panic!("action_tool called with non-object properties for {name}")
    };
    property_map.insert(
        "snapshot_id".to_string(),
        json!({
            "type": "string",
            "description": "snapshot_id from get_app_state or screenshot; coordinate translation requires capture metadata."
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
        "annotations": annotations.to_value(),
        "inputSchema": input_schema
    })
}

#[cfg(test)]
mod annotation_tests {
    use super::build_tool_definitions;

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
        ("setup_accessibility", (false, false, true, false)),
        // Deliberate judgment call: installs a GNOME Shell extension, but the
        // install is reversible, idempotent, and cannot destroy user data, so
        // it is pinned non-destructive rather than worst-case.
        ("setup_window_targeting", (false, false, true, false)),
        ("list_apps", (true, false, true, false)),
        ("list_windows", (true, false, true, false)),
        ("focused_window", (true, false, true, false)),
        ("activate_window", (false, false, true, false)),
        ("screenshot", (false, false, true, false)),
        ("get_app_state", (true, false, true, false)),
        ("hold_session", (false, false, true, false)),
        ("unlock_session", (false, false, true, false)),
        ("release_session", (false, false, true, false)),
        ("session_presence_status", (true, false, true, false)),
        ("focus_element", (false, false, true, false)),
        ("activate_element", (false, true, false, false)),
        ("select_element", (false, false, true, false)),
        ("expand_element", (false, false, true, false)),
        ("collapse_element", (false, false, true, false)),
        ("toggle_element", (false, false, false, false)),
        ("click", (false, true, false, false)),
        ("perform_action", (false, true, false, false)),
        ("perform_secondary_action", (false, true, false, false)),
        ("scroll", (false, false, false, false)),
        ("drag", (false, true, false, false)),
        ("type_text", (false, true, false, false)),
        ("press_key", (false, true, false, false)),
        ("set_value", (false, true, true, false)),
        ("browser_status", (true, false, true, false)),
        ("browser_list_tabs", (true, false, true, false)),
        ("browser_open", (false, false, false, true)),
        ("browser_claim_tab", (false, false, true, false)),
        ("browser_move_mouse", (false, false, true, false)),
        // Deliberate judgment call: navigation can discard unsaved page
        // state, but it is pinned non-destructive + idempotent because
        // re-navigating to the same URL converges and codex would otherwise
        // gate every page load behind an approval.
        ("browser_navigate", (false, false, true, true)),
        ("browser_snapshot", (true, false, true, false)),
        ("browser_screenshot", (true, false, true, false)),
        ("browser_click", (false, true, false, true)),
        ("browser_type_text", (false, true, false, true)),
        ("browser_press_key", (false, true, false, true)),
        ("browser_scroll", (false, false, false, true)),
        ("browser_eval", (false, true, false, true)),
        // Phone Use: read-only observation tools.
        ("phone_observe", (true, false, true, false)),
        ("phone_status", (true, false, true, false)),
        ("phone_list_devices", (true, false, true, false)),
        ("phone_companion_status", (true, false, true, false)),
        ("phone_accessibility_tree", (true, false, true, false)),
        ("phone_notifications", (true, false, true, false)),
        ("phone_app_current", (true, false, true, false)),
        ("phone_app_list", (true, false, true, false)),
        // Phone Use: local navigation actions — reversible, idempotent, and
        // unable to trigger arbitrary in-app behavior. force_stop terminates an
        // app but re-running converges to the same stopped state, so it is
        // pinned idempotent rather than worst-case destructive.
        ("phone_refresh_capabilities", (false, false, true, false)),
        ("phone_pair_wireless", (false, false, true, false)),
        ("phone_connect", (false, false, true, false)),
        ("phone_disconnect", (false, false, true, false)),
        ("phone_install_companion", (false, false, true, false)),
        ("phone_app_force_stop", (false, false, true, false)),
        // Unlike the idempotent companion install, installing arbitrary APKs
        // (reinstall/downgrade/test) can overwrite or downgrade an existing app,
        // so it is destructive and not idempotent.
        ("phone_app_install", (false, true, false, false)),
        ("phone_open_settings", (false, false, true, false)),
        // Phone Use: arbitrary device input and app/notification actions that
        // can press any control in any app, so the destructive hint stays true.
        ("phone_screenshot", (true, false, true, false)),
        ("phone_tap", (false, true, false, false)),
        ("phone_swipe", (false, true, false, false)),
        ("phone_type_text", (false, true, false, false)),
        ("phone_press_key", (false, true, false, false)),
        ("phone_app_launch", (false, true, false, false)),
        ("phone_app_open_intent", (false, true, false, false)),
        ("phone_notification_open", (false, true, false, false)),
        ("phone_notification_dismiss", (false, true, false, false)),
        ("phone_notification_action", (false, true, false, false)),
        ("phone_notification_reply", (false, true, false, false)),
    ];

    #[test]
    fn every_tool_pins_honest_mcp_annotations() {
        for can_receive_images in [false, true] {
            let tools = build_tool_definitions(can_receive_images);
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
        let tools = build_tool_definitions(true);
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
}
