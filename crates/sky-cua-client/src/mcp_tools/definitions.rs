//! MCP tool definitions: the host-facing tool registry with input schemas
//! and annotations. Split from `mcp_tools.rs` along the contract-family
//! boundary; dispatch and response shaping stay in the parent module.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::sync::LazyLock;

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
use super::browser;
use super::phone;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum McpToolProfile {
    Legacy,
    Compact,
}

impl McpToolProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Compact => "compact",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum McpConfigDiagnostic {
    InvalidToolProfile { value: String },
    InvalidBrowserEval { value: String },
    InvalidModelSupportsImages { value: String },
}

#[derive(Debug, Clone)]
pub(crate) struct McpProcessConfig {
    pub(crate) profile: McpToolProfile,
    pub(crate) browser_eval_enabled: bool,
    pub(crate) model_supports_images_override: Option<bool>,
    #[allow(dead_code)]
    pub(crate) diagnostics: Vec<McpConfigDiagnostic>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum InactiveToolReason {
    OtherProfile,
    BrowserEvalDisabled,
}

#[derive(Debug, Clone)]
pub(crate) struct McpToolRegistry {
    pub(crate) profile: McpToolProfile,
    pub(crate) browser_eval_enabled: bool,
    tools: Value,
    active_names: BTreeSet<String>,
    inactive_names: BTreeMap<String, InactiveToolReason>,
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
        self.inactive_names.get(name).copied()
    }
}

pub(crate) fn mcp_process_config_from_env() -> McpProcessConfig {
    let mut diagnostics = Vec::new();
    let profile = match std::env::var("SKY_CUA_MCP_TOOL_PROFILE") {
        Ok(value) => match parse_mcp_tool_profile_runtime(Some(&value)) {
            Ok(profile) => profile,
            Err(()) => {
                diagnostics.push(McpConfigDiagnostic::InvalidToolProfile { value });
                McpToolProfile::Legacy
            }
        },
        Err(_) => McpToolProfile::Legacy,
    };
    let browser_eval_enabled = match std::env::var(BROWSER_EVAL_ENV) {
        Ok(value) => match parse_bool_runtime(&value) {
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
        profile,
        browser_eval_enabled,
        model_supports_images_override,
        diagnostics,
    }
}

pub(crate) fn parse_mcp_tool_profile_runtime(value: Option<&str>) -> Result<McpToolProfile, ()> {
    match value.map(str::trim) {
        None | Some("") => Ok(McpToolProfile::Legacy),
        Some("legacy") => Ok(McpToolProfile::Legacy),
        Some("compact") => Ok(McpToolProfile::Compact),
        Some(_) => Err(()),
    }
}

fn parse_bool_runtime(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "supported" | "enabled" => Some(true),
        "0" | "false" | "no" | "off" | "unsupported" | "disabled" => Some(false),
        _ => None,
    }
}

pub(crate) fn build_tool_registry(
    process: &McpProcessConfig,
    model: &ModelSessionInfo,
) -> McpToolRegistry {
    let can_receive_images = model.can_receive_images();
    let legacy_tools =
        build_tool_definitions_for_policy(can_receive_images, process.browser_eval_enabled);
    let compact_tools =
        build_compact_tool_definitions(can_receive_images, process.browser_eval_enabled);
    let (tools, inactive_profile_tools) = match process.profile {
        McpToolProfile::Legacy => (legacy_tools.clone(), compact_tools),
        McpToolProfile::Compact => (compact_tools.clone(), legacy_tools),
    };
    let active_names = tool_names(&tools);
    let mut inactive_names = BTreeMap::new();
    for name in tool_names(&inactive_profile_tools) {
        if !active_names.contains(&name) {
            inactive_names.insert(name, InactiveToolReason::OtherProfile);
        }
    }
    if !process.browser_eval_enabled && !active_names.contains("browser_eval") {
        inactive_names.insert(
            "browser_eval".to_string(),
            InactiveToolReason::BrowserEvalDisabled,
        );
    }

    McpToolRegistry {
        profile: process.profile,
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
    let index = usize::from(model.can_receive_images());
    TOOL_DEFINITIONS_CACHE[index].clone()
}

#[cfg(test)]
pub(crate) fn tools_list_result(model: &ModelSessionInfo) -> Value {
    json!({
        "tools": tool_definitions(model)
    })
}

#[cfg(test)]
static TOOL_DEFINITIONS_CACHE: LazyLock<[Value; 2]> =
    LazyLock::new(|| [build_tool_definitions(false), build_tool_definitions(true)]);

#[cfg(test)]
pub(crate) fn build_tool_definitions(can_receive_images: bool) -> Value {
    build_tool_definitions_for_policy(can_receive_images, browser::browser_eval_enabled())
}

pub(crate) fn build_tool_definitions_for_policy(
    can_receive_images: bool,
    browser_eval_enabled: bool,
) -> Value {
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
    browser::push_tool_definitions(tool_array, browser_eval_enabled);
    phone::push_tool_definitions(tool_array);

    tools
}

fn build_compact_tool_definitions(can_receive_images: bool, browser_eval_enabled: bool) -> Value {
    let mut tools = json!([
        compact_tool(
            "doctor",
            "Run sky-cua readiness diagnostics for desktop, browser, phone, and session-presence integration.",
            LOCAL_NAVIGATION_ACTION,
            json!({}),
            json!([])
        ),
        compact_tool(
            "status",
            "Report status for browser, phone, phone companion, or session presence.",
            READ_ONLY_TOOL,
            json!({"component": {"type": "string", "enum": ["browser", "phone", "phone_companion", "session_presence"]}}),
            json!(["component"])
        ),
        compact_tool(
            "list_resources",
            "List bounded resources such as desktop windows/apps, browser tabs, phone devices/apps, or current focused resources.",
            READ_ONLY_TOOL,
            compact_list_resources_properties(),
            json!(["surface", "resource"])
        ),
        compact_tool(
            "observe",
            "Observe desktop, browser, or phone state using the compact profile branch selected by surface.",
            READ_ONLY_TOOL,
            compact_observe_properties(can_receive_images),
            json!(["surface"])
        ),
        compact_tool(
            "capture_screen",
            "Capture browser or phone screen state. Use capture_desktop for desktop screenshots.",
            READ_ONLY_TOOL,
            compact_capture_screen_properties(),
            json!(["surface"])
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
            "Capture a desktop display or window; this may activate/focus a target window before capture.",
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
        compact_tool(
            "activate_window",
            "Activate a desktop window by exact id or selector.",
            LOCAL_NAVIGATION_ACTION,
            window_target_schema(),
            json!([])
        ),
        compact_tool(
            "desktop_semantic",
            "Focus, select, expand, or collapse a desktop semantic element.",
            LOCAL_NAVIGATION_ACTION,
            compact_desktop_semantic_properties(
                json!({"operation": {"type": "string", "enum": ["focus", "select", "expand", "collapse"]}})
            ),
            json!(["operation"])
        ),
        compact_tool(
            "browser_claim_tab",
            "Claim an existing browser tab for control.",
            LOCAL_NAVIGATION_ACTION,
            browser_tab_properties(),
            json!(["tab_id"])
        ),
        compact_tool(
            "browser_move_mouse",
            "Move the visible browser agent cursor in CSS-pixel coordinates.",
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
            json!({"host_port": {"type": "string"}, "pairing_code": {"type": "string"}}),
            json!(["host_port", "pairing_code"])
        ),
        compact_tool(
            "phone_setup",
            "Install the phone companion app or open a required Android settings screen.",
            LOCAL_NAVIGATION_ACTION,
            compact_phone_setup_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "phone_app_force_stop",
            "Force-stop a connected phone app.",
            LOCAL_NAVIGATION_ACTION,
            with_phone_selector(json!({"package_name": {"type": "string"}})),
            json!(["package_name"])
        ),
        compact_tool(
            "desktop_toggle",
            "Toggle a desktop semantic element.",
            LOCAL_STATEFUL_ACTION,
            compact_desktop_semantic_properties(json!({})),
            json!([])
        ),
        compact_tool(
            "desktop_scroll",
            "Scroll a desktop semantic element or focused area.",
            LOCAL_STATEFUL_ACTION,
            compact_desktop_semantic_properties(json!({
                "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                "amount": {"type": "number", "description": "Optional scroll amount passed through to the desktop backend."}
            })),
            json!(["direction"])
        ),
        compact_tool(
            "browser_scroll",
            "Scroll an open-world browser page.",
            ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true
            },
            compact_browser_scroll_properties(),
            json!(["tab_id"])
        ),
        compact_tool(
            "desktop_pointer",
            "Click, secondary-click, or drag on the desktop.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_desktop_pointer_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "desktop_keyboard",
            "Type text or press a key on the desktop.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_desktop_keyboard_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "desktop_action",
            "Activate a desktop semantic element or perform a named/indexed action.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_desktop_action_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "desktop_set_value",
            "Set a desktop semantic element value.",
            ToolAnnotations {
                read_only: false,
                destructive: true,
                idempotent: true,
                open_world: false
            },
            compact_desktop_semantic_properties(json!({"value": {"type": "string"}})),
            json!(["value"])
        ),
        compact_tool(
            "browser_open",
            "Open a new open-world browser tab.",
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
            "Navigate a claimed open-world browser tab.",
            ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: true,
                open_world: true
            },
            browser_target_url_properties(true),
            json!(["tab_id", "url"])
        ),
        compact_tool(
            "browser_input",
            "Click, type text, or press a key in an open-world browser tab.",
            super::annotations::OPEN_WORLD_DESTRUCTIVE_ACTION,
            compact_browser_input_properties(),
            json!(["operation", "tab_id"])
        ),
        compact_tool(
            "phone_pointer",
            "Tap or swipe on a connected phone.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_pointer_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "phone_keyboard",
            "Type text or press a key on a connected phone.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_keyboard_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "phone_notification_action",
            "Open, dismiss, or run an action on a connected-phone notification.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_notification_action_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "phone_notification_reply",
            "Reply inline to a connected-phone notification.",
            LOCAL_DESTRUCTIVE_ACTION,
            with_phone_selector(
                json!({"event_id": {"type": "string"}, "text": {"type": "string"}})
            ),
            json!(["event_id", "text"])
        ),
        compact_tool(
            "phone_app_action",
            "Launch a phone app or open an Android intent.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_app_action_properties(),
            json!(["operation"])
        ),
        compact_tool(
            "phone_app_install",
            "Install an APK on a connected phone.",
            LOCAL_DESTRUCTIVE_ACTION,
            compact_phone_app_install_properties(),
            json!([])
        )
    ]);
    if browser_eval_enabled {
        tools.as_array_mut().expect("compact tool array").push(compact_tool(
            "browser_eval",
            "Evaluate JavaScript in a claimed browser tab. This is hidden unless browser eval is explicitly enabled.",
            super::annotations::OPEN_WORLD_DESTRUCTIVE_ACTION,
            merge_properties(
                browser_tab_properties(),
                json!({"expression": {"type": "string"}})
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

fn compact_desktop_keyboard_properties() -> Value {
    action_tool_properties(keyboard_target_properties(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": {"type": "string"},
        "key": {"type": "string"}
    })))
}

fn compact_desktop_action_properties() -> Value {
    action_tool_properties(merge_properties(
        json!({
            "operation": {"type": "string", "enum": ["activate", "perform_action"]},
            "action": {
                "type": "string",
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

fn action_tool_properties(mut properties: Value) -> Value {
    let property_map = properties
        .as_object_mut()
        .expect("action tool properties must be object");
    property_map.insert(
        "snapshot_id".to_string(),
        json!({
            "type": "string",
            "description": "snapshot_id from observe(surface=desktop) or capture_desktop; coordinate translation requires capture metadata."
        }),
    );
    properties
}

fn semantic_selector_properties() -> Value {
    json!({
        "element_index": {
            "type": "integer",
            "minimum": 0,
            "description": "Element index from the latest desktop observation."
        },
        "element_identifier": {
            "type": "string",
            "description": "Direct backend_ref from desktop observation; bypasses element_index lookup."
        },
        "role": {"type": "string"},
        "name": {"type": "string"},
        "text": {"type": "string"},
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

fn browser_tab_id_schema() -> Value {
    json!({
        "type": "string",
        "description": "tab_id from browser_open, browser_claim_tab, or list_resources(surface=browser, resource=tabs)."
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
            "description": "HTTP(S), file, about, data, or chrome URL accepted by the browser bridge."
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
            "text": {"type": "string"},
            "key": {"type": "string"}
        }),
    )
}

fn compact_browser_scroll_properties() -> Value {
    merge_properties(
        compact_browser_point_properties(),
        json!({
            "delta_x": {"type": "number", "description": "Horizontal wheel delta in CSS pixels."},
            "delta_y": {"type": "number", "description": "Vertical wheel delta in CSS pixels."}
        }),
    )
}

fn browser_snapshot_window_properties() -> Value {
    json!({
        "element_query": {"type": "string"},
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
        "description": "session_id from phone_connection(connect) or list_resources(surface=phone, resource=devices)."
    })
}

fn phone_serial_schema() -> Value {
    json!({
        "type": "string",
        "description": "ADB serial from list_resources(surface=phone, resource=devices). session_id is preferred when both are present."
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
            "enum": ["accessibility", "notification_listener", "developer_options", "wireless_debugging", "app_info"]
        }
    }))
}

fn compact_phone_pointer_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["tap", "swipe"]},
        "phone_snapshot_id": {"type": "string"},
        "x": {"type": "number"},
        "y": {"type": "number"},
        "start_x": {"type": "number"},
        "start_y": {"type": "number"},
        "end_x": {"type": "number"},
        "end_y": {"type": "number"},
        "duration_ms": {"type": "integer", "minimum": 0},
        "use_device_coordinates": {"type": "boolean"}
    }))
}

fn compact_phone_keyboard_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": {"type": "string"},
        "key": {"type": "string"}
    }))
}

fn compact_phone_notification_action_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["open", "dismiss", "action"]},
        "event_id": {"type": "string"},
        "action_id": {"type": "string"}
    }))
}

fn compact_phone_app_action_properties() -> Value {
    with_phone_selector(json!({
        "operation": {"type": "string", "enum": ["launch", "open_intent"]},
        "package_name": {"type": "string"},
        "activity": {"type": "string"},
        "action": {"type": "string"},
        "data_uri": {"type": "string"},
        "mime_type": {"type": "string"},
        "extras": {"type": "object"},
        "flags": {"type": "array", "items": {"type": "string"}}
    }))
}

fn compact_phone_app_install_properties() -> Value {
    with_phone_selector(json!({
        "apk_paths": {"type": "array", "items": {"type": "string"}},
        "apk_path": {"type": "string"},
        "mode": {"type": "string", "enum": ["install", "streaming", "fastdeploy"]},
        "reinstall": {"type": "boolean"},
        "allow_downgrade": {"type": "boolean"},
        "allow_test_apk": {"type": "boolean"}
    }))
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
    use super::{
        InactiveToolReason, McpConfigDiagnostic, McpProcessConfig, McpToolProfile,
        build_tool_definitions, build_tool_registry, mcp_process_config_from_env,
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

    fn process_config(profile: McpToolProfile, browser_eval_enabled: bool) -> McpProcessConfig {
        McpProcessConfig {
            profile,
            browser_eval_enabled,
            model_supports_images_override: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn compact_registry_has_expected_name_budget() {
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(McpToolProfile::Compact, false), &model);
        assert_eq!(registry.profile, McpToolProfile::Compact);
        assert_eq!(registry.active_names.len(), 34);
        assert!(registry.contains("observe"));
        assert!(registry.contains("browser_input"));
        let doctor = registry
            .tools
            .as_array()
            .expect("compact tools array")
            .iter()
            .find(|tool| tool["name"] == "doctor")
            .expect("compact doctor");
        assert_eq!(doctor["annotations"]["readOnlyHint"], false);
        assert_eq!(doctor["annotations"]["idempotentHint"], true);
        assert!(!registry.contains("browser_eval"));
        assert!(!registry.contains("get_app_state"));
        assert_eq!(
            registry.inactive_reason("get_app_state"),
            Some(InactiveToolReason::OtherProfile)
        );
        assert_eq!(
            registry.inactive_reason("browser_eval"),
            Some(InactiveToolReason::BrowserEvalDisabled)
        );
    }

    #[test]
    fn compact_registry_adds_browser_eval_only_when_enabled() {
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(McpToolProfile::Compact, true), &model);
        assert_eq!(registry.active_names.len(), 35);
        assert!(registry.contains("browser_eval"));
        assert_eq!(registry.inactive_reason("browser_eval"), None);
    }

    #[test]
    fn legacy_registry_preserves_current_default_budget() {
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(McpToolProfile::Legacy, false), &model);
        assert_eq!(registry.profile, McpToolProfile::Legacy);
        assert_eq!(registry.active_names.len(), 66);
        assert!(registry.contains("get_app_state"));
        assert!(!registry.contains("observe"));
        assert_eq!(
            registry.inactive_reason("observe"),
            Some(InactiveToolReason::OtherProfile)
        );
    }

    #[test]
    fn legacy_registry_adds_browser_eval_only_when_enabled() {
        let model = ModelSessionInfo::default();
        let registry = build_tool_registry(&process_config(McpToolProfile::Legacy, true), &model);
        assert_eq!(registry.active_names.len(), 67);
        assert!(registry.contains("browser_eval"));
    }

    #[test]
    fn mcp_runtime_config_invalid_values_fallback() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        unsafe {
            std::env::set_var("SKY_CUA_MCP_TOOL_PROFILE", "compact-ish");
            std::env::set_var("SKY_CUA_BROWSER_EVAL", "perhaps");
            std::env::set_var("SKY_CUA_MODEL_SUPPORTS_IMAGES", "sometimes");
        }
        let config = mcp_process_config_from_env();
        unsafe {
            std::env::remove_var("SKY_CUA_MCP_TOOL_PROFILE");
            std::env::remove_var("SKY_CUA_BROWSER_EVAL");
            std::env::remove_var("SKY_CUA_MODEL_SUPPORTS_IMAGES");
        }
        assert_eq!(config.profile, McpToolProfile::Legacy);
        assert!(!config.browser_eval_enabled);
        assert_eq!(config.model_supports_images_override, None);
        assert_eq!(
            config.diagnostics,
            vec![
                McpConfigDiagnostic::InvalidToolProfile {
                    value: "compact-ish".to_string()
                },
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
    fn mcp_tool_surface_matrix_fixture_matches_generated_registry() {
        assert_fixture_matches(
            "mcp_tool_surface_matrix.json",
            include_str!("../../tests/fixtures/mcp_tool_surface_matrix.json"),
            generated_surface_matrix(),
        );
    }

    #[test]
    fn compact_tool_contract_fixture_matches_generated_registry() {
        let generated = generated_compact_contract();
        let tools = generated["tools"].as_array().expect("contract tools");
        let contract_names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("contract tool name"))
            .collect();
        let registry = build_tool_registry(
            &process_config(McpToolProfile::Compact, true),
            &ModelSessionInfo::default(),
        );
        let advertised_names: Vec<&str> = registry
            .tools
            .as_array()
            .expect("compact tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect();
        assert_eq!(contract_names, advertised_names);

        assert_fixture_matches(
            "compact_tool_contract.json",
            include_str!("../../tests/fixtures/compact_tool_contract.json"),
            generated,
        );
    }

    #[test]
    fn compact_call_cases_fixture_matches_contract() {
        let generated = generated_compact_call_cases();
        let cases = generated["cases"].as_array().expect("call cases");
        assert!(
            cases.iter().all(|case| case["valid"].is_object()),
            "every valid compact call case must be an object"
        );
        assert!(
            cases.iter().all(|case| case["invalid"].is_object()),
            "every invalid compact call case must be an object"
        );
        let contract_branch_count: usize = generated_compact_contract()["tools"]
            .as_array()
            .expect("contract tools")
            .iter()
            .map(|tool| tool["branches"].as_array().expect("branches").len())
            .sum();
        assert_eq!(cases.len(), contract_branch_count);

        assert_fixture_matches(
            "compact_call_cases.json",
            include_str!("../../tests/fixtures/compact_call_cases.json"),
            generated,
        );
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
        for profile in [McpToolProfile::Legacy, McpToolProfile::Compact] {
            for can_receive_images in [false, true] {
                for browser_eval_enabled in [false, true] {
                    let model = model_with_image_capability(can_receive_images);
                    let registry =
                        build_tool_registry(&process_config(profile, browser_eval_enabled), &model);
                    let tools_list = registry.tools_list_result();
                    let serialized = serde_json::to_string(&tools_list).expect("tools list json");
                    rows.push(json!({
                        "profile": profile.as_str(),
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
        }

        for can_receive_images in [false, true] {
            for browser_eval_enabled in [false, true] {
                let legacy = rows
                    .iter()
                    .find(|row| {
                        row["profile"] == "legacy"
                            && row["can_receive_images"] == can_receive_images
                            && row["browser_eval_enabled"] == browser_eval_enabled
                    })
                    .expect("legacy row");
                let compact = rows
                    .iter()
                    .find(|row| {
                        row["profile"] == "compact"
                            && row["can_receive_images"] == can_receive_images
                            && row["browser_eval_enabled"] == browser_eval_enabled
                    })
                    .expect("compact row");
                let legacy_bytes = legacy["serialized_bytes"].as_u64().expect("legacy bytes");
                let compact_bytes = compact["serialized_bytes"].as_u64().expect("compact bytes");
                assert!(
                    compact_bytes * 100 <= legacy_bytes * 65,
                    "compact profile exceeds 65% serialized budget for images={can_receive_images} eval={browser_eval_enabled}: compact={compact_bytes} legacy={legacy_bytes}"
                );
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

    fn generated_compact_contract() -> Value {
        let registry = build_tool_registry(
            &process_config(McpToolProfile::Compact, true),
            &ModelSessionInfo::default(),
        );
        let advertised = registry.tools.as_array().expect("compact tools");
        let tools: Vec<Value> = compact_contract_tools()
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
                object.insert("content_policy".to_string(), json!("profile_rewrite"));
                object.insert("structured_policy".to_string(), json!("compact_envelope"));
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
            "profile": "compact",
            "default_tool_count": 34,
            "eval_tool_count": 35,
            "tools": tools
        })
    }

    fn compact_contract_tools() -> Vec<Value> {
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
                        json!({"operation": "focus", "element_index": 1}),
                    ),
                    branch(
                        "select",
                        "select_element",
                        json!({"operation": "select", "element_index": 1}),
                    ),
                    branch(
                        "expand",
                        "expand_element",
                        json!({"operation": "expand", "element_index": 1}),
                    ),
                    branch(
                        "collapse",
                        "collapse_element",
                        json!({"operation": "collapse", "element_index": 1}),
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
                        json!({"operation": "open_settings"}),
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
                    json!({"element_index": 1}),
                )],
            ),
            contract_tool(
                "desktop_scroll",
                vec![branch("default", "scroll", json!({"direction": "down"}))],
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
                        json!({"operation": "activate", "element_index": 1}),
                    ),
                    branch(
                        "perform_action",
                        "perform_action",
                        json!({"operation": "perform_action", "action_index": 0}),
                    ),
                ],
            ),
            contract_tool(
                "desktop_set_value",
                vec![branch(
                    "default",
                    "set_value",
                    json!({"element_index": 1, "value": "hello"}),
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
                    json!({"event_id": "event-1", "text": "reply"}),
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
                        json!({"operation": "open_intent", "action": "android.intent.action.VIEW"}),
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
        legacy_tool: &'static str,
        minimal_valid_arguments: Value,
    ) -> Value {
        json!({
            "name": name,
            "legacy_tool": legacy_tool,
            "handler_id": legacy_tool,
            "minimal_valid_arguments": minimal_valid_arguments,
            "expected_errors": ["InvalidRequest", "ToolNotInActiveProfile", "UnknownTool"]
        })
    }

    fn generated_compact_call_cases() -> Value {
        let mut cases = Vec::new();
        for tool in compact_contract_tools() {
            let tool_name = tool["name"].as_str().expect("tool name");
            for branch in tool["branches"].as_array().expect("branches") {
                let branch_name = branch["name"].as_str().expect("branch name");
                cases.push(json!({
                    "tool": tool_name,
                    "branch": branch_name,
                    "legacy_tool": branch["legacy_tool"],
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
