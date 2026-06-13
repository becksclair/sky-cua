//! MCP tool definitions: the host-facing tool registry with input schemas
//! and annotations. Split from `mcp_tools.rs` along the contract-family
//! boundary; dispatch and response shaping stay in the parent module.

use std::sync::LazyLock;

use serde_json::{Value, json};

use crate::mcp_server::ModelSessionInfo;

use super::annotations::{
    LOCAL_DESTRUCTIVE_ACTION, LOCAL_NAVIGATION_ACTION, LOCAL_STATEFUL_ACTION, READ_ONLY_TOOL,
    ToolAnnotations,
};
use super::browser;

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
    let point_description = if can_receive_images {
        "With snapshot_id, use screenshot pixel coordinates from that get_app_state or screenshot image; without snapshot_id, use current screen coordinates for the active input backend."
    } else {
        "Use current screen coordinates for the active input backend. This session's model does not support image input, so screenshot-coordinate targeting is disabled."
    };
    let drag_point_description = if can_receive_images {
        "With snapshot_id, use screenshot pixels from that get_app_state or screenshot image; without snapshot_id, use current screen coordinates."
    } else {
        "Use current screen coordinates for the active input backend. This session's model does not support image input, so screenshot-coordinate targeting is disabled."
    };
    let mut tools = json!([
        {
            "name": "doctor",
            "description": "Report Computer Use desktop integration readiness, including environment, detached session-env repair diagnostics, semantic, capture, and input backend checks.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "setup_accessibility",
            "description": "Enable toolkit accessibility for AT-SPI-backed semantic app trees, then return a before/after doctor report. Target apps may need restart.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "setup_window_targeting",
            "description": "Install and enable the bundled GNOME Shell window-control extension for exact GNOME window targeting, then report window backend status.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "list_apps",
            "description": "List currently exposed desktop applications from the active platform window and accessibility backends, with diagnostics when detached session-env repair affected runtime readiness.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "list_windows",
            "description": "List desktop windows from native windowing backends, including backend identity, stable window_id values, bounds, focus state, display placement, and terminal metadata when available. Use this first when you need a window_id or display_id for a targeted screenshot or exact window activation.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "focused_window",
            "description": "Return the focused desktop window reported by native windowing backends, if one is available, including display placement when known.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "activate_window",
            "description": "Activate a desktop window by window_id or selector metadata. Supports exact window activation when the matched backend can target windows; otherwise reports unsupported backends honestly. For visual inspection of a specific window, prefer screenshot with the same target fields because it activates and focus-verifies before capture.",
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
                "Capture a fresh screenshot for visual inspection and screenshot-coordinate actions. With no selector, captures the primary display only. For a known window, pass window_id or another window target field: the backend activates and focus-verifies that window first, then returns a cropped, unoccluded window screenshot. For a specific monitor, pass display_id from environment.displays. Use capture_all_displays=true only when the whole virtual desktop is required."
            } else {
                "Capture a fresh screenshot and return screenshot_path plus snapshot_id metadata. With no selector, captures the primary display only. For a known window, pass window_id or another window target field: the backend activates and focus-verifies that window first, then returns a cropped, unoccluded window screenshot. For a specific monitor, pass display_id from environment.displays. Use capture_all_displays=true only when the whole virtual desktop is required."
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
                "Build a structured desktop app-state snapshot with environment displays, detached session-env diagnostics, flattened accessibility elements, optional focused-screen capture, and readback for focused or editable text/value controls when the backend can prove it. Use screenshot, not get_app_state, when the primary need is an unoccluded cropped capture of a known window or one display."
            } else {
                "Build a structured desktop app-state snapshot with environment displays, detached session-env diagnostics, flattened accessibility elements, and readback for focused or editable text/value controls when the backend can prove it. This session's model does not support image input, so screen capture is disabled; use screenshot for screenshot_path capture workflows."
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
            "description": "Hold session presence for remote automation by inhibiting lock and/or suspend; optionally unlock the session first.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": session_presence_hold_properties(true),
                "additionalProperties": false
            }
        },
        {
            "name": "unlock_session",
            "description": "Unlock the current session when supported, then hold session presence for remote automation.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": session_presence_hold_properties(false),
                "additionalProperties": false
            }
        },
        {
            "name": "release_session",
            "description": "Release held session-presence inhibitors and optionally re-lock the session when supported.",
            "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "relock": {
                        "type": "boolean",
                        "description": "Re-lock the session after releasing inhibitors. Defaults to false."
                    }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "session_presence_status",
            "description": "Report whether session presence is supported and whether lock/suspend inhibitors are currently held.",
            "annotations": READ_ONLY_TOOL.to_value(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        semantic_element_tool(
            "focus_element",
            "Move semantic focus to an accessibility element from the current snapshot.",
            LOCAL_NAVIGATION_ACTION,
        ),
        semantic_element_tool(
            "activate_element",
            "Perform the element's semantic default action, such as pressing an app-chrome button or opening a menu.",
            LOCAL_DESTRUCTIVE_ACTION,
        ),
        semantic_element_tool(
            "select_element",
            "Select an accessibility element such as a tab, list item, radio item, or selectable row.",
            LOCAL_NAVIGATION_ACTION,
        ),
        semantic_element_tool(
            "expand_element",
            "Expand an accessibility element such as a collapsed menu, combo box, disclosure, or tree item.",
            LOCAL_NAVIGATION_ACTION,
        ),
        semantic_element_tool(
            "collapse_element",
            "Collapse an accessibility element such as an expanded menu, combo box, disclosure, or tree item.",
            LOCAL_NAVIGATION_ACTION,
        ),
        semantic_element_tool(
            "toggle_element",
            "Toggle an accessibility element such as a checkbox, switch, or toggle button.",
            LOCAL_STATEFUL_ACTION,
        ),
        action_tool(
            "click",
            "Click an element by index from the current snapshot, or explicit x/y screen coordinates without a snapshot.",
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
            "Invoke a specific AT-SPI action by name or index on an element. Prefer named tools such as click, activate_element, select_element, expand_element, collapse_element, and toggle_element for common operations; use this for custom AT-SPI actions exposed in get_app_state.semantic_actions.",
            LOCAL_DESTRUCTIVE_ACTION,
            json!({
                "element_index": { "type": "integer", "minimum": 0 },
                "element_identifier": {
                    "type": "string",
                    "description": "Direct AT-SPI backend_ref/object identifier from get_app_state, bypassing element_index lookup."
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
                    "description": "Zero-based AT-SPI action index. Defaults to 0 when action_name/action are omitted."
                },
                "action_name": {
                    "type": "string",
                    "description": "AT-SPI action name to resolve against the target element's action list."
                },
                "action": {
                    "type": "string",
                    "description": "Compatibility alias: either an action name or numeric action index string."
                }
            }),
            json!([]),
        ),
        action_tool(
            "perform_secondary_action",
            "Perform a secondary click or context action by element index from the current snapshot, or explicit x/y screen coordinates without a snapshot.",
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
            "Scroll within an element from the current snapshot, or the focused area without a snapshot.",
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
            "Drag from one point or element to another; explicit coordinates can run without a snapshot.",
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
            "Type literal text into the focused control; may use snapshot context or a window target when provided.",
            LOCAL_DESTRUCTIVE_ACTION,
            keyboard_target_properties(json!({
                "text": { "type": "string" }
            })),
            json!(["text"]),
        ),
        action_tool(
            "press_key",
            "Press a keyboard key or key chord in the focused control; may use snapshot context or a window target when provided.",
            LOCAL_DESTRUCTIVE_ACTION,
            keyboard_target_properties(json!({
                "key": { "type": "string" }
            })),
            json!(["key"]),
        ),
        action_tool(
            "set_value",
            "Set an editable element value semantically where supported. Target by element_index, element_identifier, or a semantic selector from the latest get_app_state snapshot, then reacquire get_app_state and inspect value/text readback to verify the edit landed.",
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
                    "description": "Direct AT-SPI backend_ref/object identifier from get_app_state, bypassing element_index lookup."
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
            "description": "Use compact after discovery. It keeps identifiers, diagnostics, app identity, and lean element anchors while omitting verbose element descriptions and static environment/capability details."
        }
    });

    if can_receive_images && let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "capture_screen".to_string(),
            json!({
                "type": "string",
                "enum": ["auto", "if_changed", "always", "never"],
                "description": "Screen-capture policy for this state snapshot. Defaults to if_changed. Use always when a fresh visual frame is required, never for structure-only loops, and auto when the runtime should choose."
            }),
        );
        property_map.insert(
            "screenshot_delivery".to_string(),
            json!({
                "type": "string",
                "enum": ["path", "inline"],
                "description": "How the captured screenshot is delivered. path (default) returns only screenshot_path for reading the image file on demand; inline also attaches the image to this result for sessions that cannot read local files."
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
                "description": "Exact display_id from environment.displays. Use this to capture one monitor."
            }),
        );
        property_map.insert(
            "display_name".to_string(),
            json!({
                "type": "string",
                "description": "Display name/connector from environment.displays. Prefer display_id when available."
            }),
        );
        property_map.insert(
            "display_index".to_string(),
            json!({
                "type": "integer",
                "minimum": 0,
                "description": "Zero-based display index from environment.displays. Prefer display_id when available."
            }),
        );
        property_map.insert(
            "capture_all_displays".to_string(),
            json!({
                "type": "boolean",
                "description": "Capture the full virtual desktop across all displays. Defaults to false; use only when all displays are required."
            }),
        );
    }

    if can_receive_images && let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "screenshot_delivery".to_string(),
            json!({
                "type": "string",
                "enum": ["path", "inline"],
                "description": "How the captured screenshot is delivered. path (default) returns only screenshot_path for reading the image file on demand; inline also attaches the image to this result for sessions that cannot read local files."
            }),
        );
    }

    properties
}

fn session_presence_hold_properties(include_unlock: bool) -> Value {
    let mut properties = json!({
        "inhibit_lock": {
            "type": "boolean",
            "description": "Hold the desktop lock/screensaver inhibitor. Defaults to true."
        },
        "inhibit_suspend": {
            "type": "boolean",
            "description": "Hold the system suspend inhibitor. Defaults to true."
        }
    });

    if include_unlock && let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "unlock".to_string(),
            json!({
                "type": "boolean",
                "description": "Unlock the session before holding inhibitors when supported. Defaults to false."
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
            "description": "Process ID from list_windows. Omit unless known; 0 is ignored."
        },
        "tty": {
            "type": "string",
            "description": "Terminal tty such as /dev/pts/7 or pts/7."
        },
        "terminal_pid": {
            "type": "integer",
            "minimum": 0,
            "description": "Terminal process ID from list_windows terminal metadata. Omit unless known; 0 is ignored."
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
                "description": "Element index from the current get_app_state snapshot."
            },
            "element_identifier": {
                "type": "string",
                "description": "Direct AT-SPI backend_ref/object identifier from get_app_state, bypassing element_index lookup."
            },
            "role": {
                "type": "string",
                "description": "Optional semantic selector role matched against the latest snapshot."
            },
            "name": {
                "type": "string",
                "description": "Optional semantic selector name matched against the latest snapshot."
            },
            "text": {
                "type": "string",
                "description": "Optional semantic selector text matched against name, description, or value."
            },
            "states": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional semantic selector states; all listed states must match."
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
            "description": "Current snapshot_id returned by the latest get_app_state or screenshot call."
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
