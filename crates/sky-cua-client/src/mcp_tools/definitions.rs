//! MCP tool definitions: the host-facing tool registry with input schemas
//! and annotations. Split from `mcp_tools.rs` along the contract-family
//! boundary; dispatch and response shaping stay in the parent module.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

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

    pub(crate) fn validate_arguments(&self, name: &str, arguments: &Value) -> Result<(), String> {
        let Some(schema) = self
            .tools
            .as_array()
            .into_iter()
            .flatten()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|tool| tool.get("inputSchema"))
        else {
            return Err(format!("missing input schema for {name}"));
        };
        if schema_accepts(schema, arguments) {
            Ok(())
        } else {
            Err(schema_rejection_message(name, arguments))
        }
    }
}

fn schema_rejection_message(name: &str, arguments: &Value) -> String {
    let mut message = format!("arguments do not match the advertised input schema for {name}");
    if let Some(hint) = schema_rejection_hint(name, arguments) {
        message.push_str(". Hint: ");
        message.push_str(hint);
    }
    message
}

fn schema_rejection_hint(name: &str, arguments: &Value) -> Option<&'static str> {
    match name {
        "status" => Some(
            "`status` expects top-level `component` set to `browser`, `phone`, \
             `phone_companion`, or `session_presence`; only phone status accepts \
             `refresh_devices`, and phone_companion status may include `session_id`.",
        ),
        "list_resources" => Some(
            "`list_resources` expects a valid top-level `surface`/`resource` pair. \
             Phone `apps` and `current_app` require top-level `session_id`; browser \
             `tabs` may use `url_contains`/`title_contains`; desktop resources do \
             not accept phone or browser fields.",
        ),
        "observe" => Some(
            "`observe` expects one surface branch: desktop uses desktop observation \
             fields, browser requires top-level `tab_id`, and phone requires \
             top-level `session_id`; do not mix fields from another surface.",
        ),
        "capture_screen" => Some(
            "`capture_screen` is only for browser or phone. Browser capture requires \
             top-level `surface=\"browser\"` and `tab_id`; phone capture requires \
             `surface=\"phone\"` and `session_id`. Use `capture_desktop` for \
             desktop screenshots.",
        ),
        "phone_pointer" if phone_snapshot_id_contains_embedded_fields(arguments) => Some(
            "`phone_snapshot_id` must be only the opaque snapshot id string; put \
             `operation`, `session_id`, coordinates, and `use_device_coordinates` as \
             separate top-level JSON keys.",
        ),
        "phone_pointer" => Some(
            "`phone_pointer` expects one flat JSON object. For tap, provide top-level \
             `operation`, `session_id`, `x`, `y`, and either `phone_snapshot_id` or \
             `use_device_coordinates=true`.",
        ),
        "browser_scroll" => Some(
            "`browser_scroll` expects top-level `tab_id` plus non-zero `delta_x` or \
             `delta_y`; omit both `x` and `y` for viewport scroll, or provide both \
             as top-level numbers for targeted scroll.",
        ),
        "browser_input" => Some(
            "`browser_input` expects one flat JSON object with top-level `operation` \
             and `tab_id`; click uses top-level `x`/`y`, while type_text uses \
             top-level `text`.",
        ),
        "browser_open" => Some(
            "`browser_open` creates a new claimed tab. Omit `url` or use a top-level \
             HTTP(S) URL or `about:blank`; it returns the `tab_id` for later calls.",
        ),
        "browser_navigate" => Some(
            "`browser_navigate` expects top-level `tab_id` plus `url`; URL must be \
             HTTP(S) or exactly `about:blank`.",
        ),
        "browser_claim_tab" | "browser_move_mouse" | "browser_eval" => Some(
            "Browser tools expect top-level `tab_id` from `browser_open` or \
             `list_resources(surface=\"browser\", resource=\"tabs\")` after claiming \
             an existing tab.",
        ),
        "capture_desktop" => Some(
            "`capture_desktop` expects one flat JSON object and captures a single \
             screen. Omit selectors to capture the main display, or pass one window \
             selector or one display selector \
             (`display_id`/`display_name`/`display_index`) to target a specific \
             window or non-main monitor; do not mix them.",
        ),
        "activate_window" => Some(
            "`activate_window` expects one top-level window selector such as \
             `window_id`, `pid`, `app_id`, `wm_class`, `title`, or a terminal \
             selector from `list_resources(surface=\"desktop\", resource=\"windows\")`.",
        ),
        "desktop_pointer" if desktop_snapshot_id_contains_embedded_fields(arguments) => Some(
            "`snapshot_id` must be only the opaque desktop snapshot id string; put \
             `operation`, coordinates, `element_index`, `name`, or `text` as \
             separate top-level JSON keys.",
        ),
        "desktop_pointer" => Some(
            "`desktop_pointer` expects one flat JSON object. For click, provide \
             top-level `operation` plus either `x`/`y`, or `snapshot_id` with \
             `element_index`, `name`, or `text`; drag uses top-level \
             `x`/`y`/`to_x`/`to_y` or `from_x`/`from_y`/`to_x`/`to_y`, plus an \
             optional `duration_ms`.",
        ),
        "desktop_scroll" => Some(
            "`desktop_scroll` expects top-level `direction` plus a snapshot-resolved \
             target: `snapshot_id` with `element_index`, `name`, or `text`; it does \
             not accept freeform x/y coordinates.",
        ),
        "desktop_keyboard" => Some(
            "`desktop_keyboard` expects top-level `operation`; `type_text` uses \
             top-level `text`, while `press_key` uses top-level `key`.",
        ),
        "desktop_semantic" | "desktop_toggle" | "desktop_set_value" => Some(
            "Desktop semantic tools expect one flat JSON object with a top-level \
             target: either `element_identifier`, or `snapshot_id` with \
             `element_index`, `name`, or `text`.",
        ),
        "desktop_action" => Some(
            "`desktop_action` expects top-level `operation` plus a semantic target. \
             `perform_action` also needs top-level `action_name` or `action_index`.",
        ),
        "setup_desktop" => Some(
            "`setup_desktop` expects top-level `operation` set to `accessibility` or \
             `window_targeting`.",
        ),
        "session_presence" => Some(
            "`session_presence` expects top-level `operation` set to `hold`, \
             `unlock`, or `release`; hold/unlock accept inhibitor booleans, and \
             release accepts `relock`.",
        ),
        "phone_connection" => Some(
            "`phone_connection` expects top-level `operation`. `connect` may use \
             top-level `serial`, `backend`, `install_companion`, and `start_scrcpy`; \
             `disconnect`/`refresh` require top-level `session_id` and do not accept \
             connect-only fields.",
        ),
        "phone_setup" => Some(
            "`phone_setup` expects top-level `operation` and `session_id`. \
             `install_companion` accepts install flags; `open_settings` requires \
             top-level `screen`, and `screen=\"app_details\"` also needs \
             `package_name`.",
        ),
        "phone_accessibility_tree" | "phone_notifications" | "phone_app_force_stop" => Some(
            "This phone tool requires top-level `session_id` from \
             `phone_connection(operation=\"connect\")`; app force-stop also requires \
             top-level `package_name`.",
        ),
        "phone_keyboard" => Some(
            "`phone_keyboard` expects top-level `operation` and `session_id`; \
             `type_text` uses top-level `text`, while `press_key` uses top-level \
             `key`.",
        ),
        "phone_notification_action" => Some(
            "`phone_notification_action` expects top-level `operation`, `session_id`, \
             and `event_id`; only `operation=\"action\"` also accepts top-level \
             `action_id` from the same fresh notification event.",
        ),
        "phone_notification_reply" => Some(
            "`phone_notification_reply` expects top-level `session_id`, `event_id`, \
             inline-reply `action_id`, and non-empty `text` from the same fresh \
             notification event.",
        ),
        "phone_app_action" => Some(
            "`phone_app_action` expects top-level `operation` and `session_id`; \
             `launch` requires `package_name`, while `open_intent` requires \
             `intent_uri` and may include `package_name`.",
        ),
        "phone_app_install" => Some(
            "`phone_app_install` expects top-level `session_id` and non-empty \
             `apk_paths` array of host paths; there is no `apk_path` singular field.",
        ),
        "phone_pair_wireless" => Some(
            "`phone_pair_wireless` expects top-level `host_port` and one-time \
             `pairing_code` from Android Wireless debugging.",
        ),
        _ => None,
    }
}

fn phone_snapshot_id_contains_embedded_fields(arguments: &Value) -> bool {
    let Some(snapshot_id) = arguments.get("phone_snapshot_id").and_then(Value::as_str) else {
        return false;
    };
    string_contains_embedded_argument_fields(snapshot_id)
}

fn desktop_snapshot_id_contains_embedded_fields(arguments: &Value) -> bool {
    let Some(snapshot_id) = arguments.get("snapshot_id").and_then(Value::as_str) else {
        return false;
    };
    string_contains_embedded_argument_fields(snapshot_id)
}

fn string_contains_embedded_argument_fields(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.contains('\n')
        || lower.contains("\"operation\"")
        || lower.contains("\"x\"")
        || lower.contains("\"y\"")
        || lower.contains("\"element_index\"")
        || lower.contains("\"name\"")
        || lower.contains("\"text\"")
        || lower.contains("\"use_device_coordinates\"")
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
    build_grouped_tool_definitions(can_receive_images, browser_eval_enabled)
}

fn build_grouped_tool_definitions(can_receive_images: bool, browser_eval_enabled: bool) -> Value {
    let mut tools = json!([
        grouped_tool(
            "doctor",
            "Run sky-cua readiness diagnostics for desktop capture/input, browser integration, and session presence.",
            READ_ONLY_TOOL,
            json!({}),
            json!([])
        ),
        grouped_tool_with_constraints(
            "status",
            "Report browser, phone, phone_companion, or session_presence health.",
            READ_ONLY_TOOL,
            status_properties(),
            json!(["component"]),
            status_constraints()
        ),
        grouped_tool_with_constraints(
            "list_resources",
            "List bounded resources. Valid pairs: desktop apps/windows/focused_window; browser tabs; phone devices/apps/current_app.",
            READ_ONLY_TOOL,
            list_resources_properties(),
            json!(["surface", "resource"]),
            list_resources_constraints()
        ),
        grouped_tool_with_constraints(
            "observe",
            "Read structured state for one surface. Desktop returns elements and snapshot_id; detail=\"compact\" controls desktop observation verbosity only. Browser requires tab_id and returns page text/elements. Phone requires session_id and can include accessibility/notifications.",
            READ_ONLY_TOOL,
            observe_properties(can_receive_images),
            json!(["surface"]),
            observe_constraints(can_receive_images)
        ),
        grouped_tool_with_constraints(
            "capture_screen",
            "Capture a browser-tab or phone image only. Browser requires tab_id. Use capture_desktop for desktop screenshots.",
            READ_ONLY_TOOL,
            capture_screen_properties(),
            json!(["surface"]),
            capture_screen_constraints()
        ),
        grouped_tool(
            "phone_accessibility_tree",
            "Read the connected phone accessibility tree, optionally bounded by node_limit.",
            READ_ONLY_TOOL,
            with_phone_session(json!({"node_limit": optional_limit_schema()})),
            json!(["session_id"])
        ),
        grouped_tool(
            "phone_notifications",
            "Read recent connected-phone notifications.",
            READ_ONLY_TOOL,
            with_phone_session(json!({"limit": optional_limit_schema()})),
            json!(["session_id"])
        ),
        grouped_tool_with_constraints(
            "capture_desktop",
            "Capture a fresh desktop frame and return a snapshot_id for pixel actions. Captures exactly one screen, never the whole multi-monitor desktop. Call with no selector for the normal case: it captures the main display. Pass one window selector to capture a single window, or one display selector (display_id/display_name/display_index) only when you specifically need a non-main monitor.",
            LOCAL_NAVIGATION_ACTION,
            screenshot_properties(can_receive_images),
            json!([]),
            screenshot_constraints()
        ),
        grouped_tool(
            "setup_desktop",
            "Set up desktop accessibility or window targeting.",
            LOCAL_NAVIGATION_ACTION,
            json!({"operation": {"type": "string", "enum": ["accessibility", "window_targeting"]}}),
            json!(["operation"])
        ),
        grouped_tool_with_constraints(
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
            json!(["operation"]),
            session_presence_constraints()
        ),
        grouped_tool_with_constraints(
            "activate_window",
            "Activate a desktop window by exact id or selector.",
            LOCAL_NAVIGATION_ACTION,
            window_target_schema(),
            json!([]),
            window_target_constraint()
        ),
        grouped_tool_with_constraints(
            "desktop_semantic",
            "Focus, select, expand, or collapse a desktop element from observe(surface=\"desktop\").",
            LOCAL_NAVIGATION_ACTION,
            desktop_semantic_properties(
                json!({"operation": {"type": "string", "enum": ["focus", "select", "expand", "collapse"]}})
            ),
            json!(["operation"]),
            desktop_selector_constraint()
        ),
        grouped_tool(
            "browser_claim_tab",
            "Claim an existing browser tab and make it controllable for observe, capture_screen, navigation, input, scroll, and eval.",
            LOCAL_NAVIGATION_ACTION,
            browser_tab_properties(),
            json!(["tab_id"])
        ),
        grouped_tool(
            "browser_move_mouse",
            "Move the visible browser agent cursor in CSS-pixel coordinates without clicking.",
            LOCAL_NAVIGATION_ACTION,
            browser_point_properties(),
            json!(["tab_id", "x", "y"])
        ),
        grouped_tool_with_constraints(
            "phone_connection",
            "Connect, disconnect, or refresh a phone session.",
            LOCAL_NAVIGATION_ACTION,
            phone_connection_properties(),
            json!(["operation"]),
            phone_connection_constraints()
        ),
        grouped_tool(
            "phone_pair_wireless",
            "Pair Android wireless debugging using a host:port and one-time pairing code.",
            LOCAL_NAVIGATION_ACTION,
            json!({"host_port": non_blank_string_schema(), "pairing_code": non_blank_string_schema()}),
            json!(["host_port", "pairing_code"])
        ),
        grouped_tool_with_constraints(
            "phone_setup",
            "Install the phone companion app or open a required Android settings screen.",
            ToolAnnotations {
                read_only: false,
                destructive: true,
                idempotent: true,
                open_world: false
            },
            phone_setup_properties(),
            json!(["operation"]),
            phone_setup_constraints()
        ),
        grouped_tool(
            "phone_app_force_stop",
            "Force-stop a connected phone app.",
            ToolAnnotations {
                read_only: false,
                destructive: true,
                idempotent: true,
                open_world: false
            },
            with_phone_session(json!({"package_name": non_blank_string_schema()})),
            json!(["session_id", "package_name"])
        ),
        grouped_tool_with_constraints(
            "desktop_toggle",
            "Toggle a desktop element from observe(surface=\"desktop\").",
            LOCAL_STATEFUL_ACTION,
            desktop_semantic_properties(json!({})),
            json!([]),
            desktop_selector_constraint()
        ),
        grouped_tool_with_constraints(
            "desktop_scroll",
            "Scroll a snapshot-resolved desktop element. Pass direction and snapshot_id plus element_index, name, or text. Re-observe before reusing an element index.",
            LOCAL_STATEFUL_ACTION,
            desktop_semantic_properties(json!({
                "direction": {"type": "string", "enum": ["up", "down"]},
                "pages": {"type": "integer", "minimum": 1, "description": "Page-sized scroll steps. Defaults to 1."}
            })),
            json!(["direction"]),
            desktop_snapshot_selector_constraint()
        ),
        grouped_tool_with_constraints(
            "browser_scroll",
            "Scroll an open-world browser page. Omit x/y for viewport scroll; provide at least one non-zero delta_x or delta_y. Targeted scroll will move the visible browser agent cursor first.",
            ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true
            },
            browser_scroll_properties(),
            json!(["tab_id"]),
            browser_scroll_constraints()
        ),
        grouped_tool_with_constraints(
            "desktop_pointer",
            "Click, secondary-click, or drag on the desktop. Use live coordinates, or snapshot_id plus element_index/name/text from the same desktop observation or capture; do not call with only operation.",
            LOCAL_DESTRUCTIVE_ACTION,
            desktop_pointer_properties(),
            json!(["operation"]),
            desktop_pointer_constraints()
        ),
        grouped_tool_with_constraints(
            "desktop_keyboard",
            "Type text or press a key on the desktop. Focus first; text for type_text, key for press_key, e.g. Enter, Escape, Tab, Ctrl+A, Meta+A.",
            LOCAL_DESTRUCTIVE_ACTION,
            desktop_keyboard_properties(),
            json!(["operation"]),
            desktop_keyboard_constraints()
        ),
        grouped_tool_with_constraints(
            "desktop_action",
            "Activate a desktop element or perform its named/indexed action from observe(surface=\"desktop\"); do not call with only operation.",
            LOCAL_DESTRUCTIVE_ACTION,
            desktop_action_properties(),
            json!(["operation"]),
            desktop_action_constraints()
        ),
        grouped_tool_with_constraints(
            "desktop_set_value",
            "Set a desktop element value. Include replacement value plus target from observe(surface=\"desktop\").",
            ToolAnnotations {
                read_only: false,
                destructive: true,
                idempotent: true,
                open_world: false
            },
            desktop_semantic_properties(json!({
                "value": {
                    "type": "string",
                    "description": "Replacement value to write. The text selector still identifies the target element."
                }
            })),
            json!(["value"]),
            desktop_selector_constraint()
        ),
        grouped_tool(
            "browser_open",
            "Create and claim a browser tab at url, or about:blank when url is omitted. Returns a tab_id for later browser calls.",
            ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true
            },
            browser_target_url_properties(false),
            json!([])
        ),
        grouped_tool(
            "browser_navigate",
            "Navigate an opened or claimed browser tab to an HTTP(S) URL or about:blank.",
            ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: true,
                open_world: true
            },
            browser_target_url_properties(true),
            json!(["tab_id", "url"])
        ),
        grouped_tool_with_constraints(
            "browser_input",
            "Click, type text, or press a key in a claimed browser tab. click uses x/y CSS pixels; keys look like Enter, Escape, Tab, Ctrl+K.",
            super::annotations::OPEN_WORLD_DESTRUCTIVE_ACTION,
            browser_input_properties(),
            json!(["operation", "tab_id"]),
            browser_input_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_pointer",
            "Tap or swipe on a connected phone. Use phone_snapshot_id for screenshot pixels or use_device_coordinates for raw pixels.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_pointer_properties(),
            json!(["operation"]),
            phone_pointer_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_keyboard",
            "Type text or press a key on a connected phone. Focus first; press_key accepts KEYCODE_* names, aliases, or numeric keycodes.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_keyboard_properties(),
            json!(["operation"]),
            phone_keyboard_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_notification_action",
            "Open, dismiss, or run an action on a connected-phone notification.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_notification_action_properties(),
            json!(["operation"]),
            phone_notification_action_constraints()
        ),
        grouped_tool(
            "phone_notification_reply",
            "Reply inline to a connected-phone notification using event_id and inline-reply action_id from the same fresh event.",
            LOCAL_DESTRUCTIVE_ACTION,
            with_phone_session(json!({
                "event_id": {
                    "type": "string",
                    "minLength": 1,
                    "pattern": ".*\\S.*",
                    "description": "event_id from fresh phone notifications."
                },
                "action_id": {
                    "type": "string",
                    "minLength": 1,
                    "pattern": ".*\\S.*",
                    "description": "Inline-reply action_id from that event."
                },
                "text": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Reply text."
                }
            })),
            json!(["session_id", "event_id", "action_id", "text"])
        ),
        grouped_tool_with_constraints(
            "phone_app_action",
            "Launch a phone app or open an Android intent.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_app_action_properties(),
            json!(["operation"]),
            phone_app_action_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_app_install",
            "Install an APK on a connected phone.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_app_install_properties(),
            json!([]),
            phone_app_install_constraints()
        )
    ]);
    if browser_eval_enabled {
        tools.as_array_mut().expect("tool array").push(grouped_tool(
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

fn grouped_tool(
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

fn grouped_tool_with_constraints(
    name: &str,
    description: &str,
    annotations: ToolAnnotations,
    properties: Value,
    required: Value,
    constraints: Value,
) -> Value {
    let mut tool = grouped_tool(name, description, annotations, properties, required);
    let input_schema = tool
        .get_mut("inputSchema")
        .and_then(Value::as_object_mut)
        .expect("tool inputSchema must be an object");
    let constraints = constraints
        .as_object()
        .unwrap_or_else(|| panic!("tool constraints must be object: {constraints:?}"));
    input_schema.extend(constraints.clone());
    normalize_root_composition_schema(input_schema);
    normalize_required_property_schemas(input_schema);
    tool
}

fn normalize_required_property_schemas(input_schema: &mut serde_json::Map<String, Value>) {
    let mut normalized = Value::Object(input_schema.clone());
    normalize_required_property_schemas_in_value(&mut normalized);
    *input_schema = normalized
        .as_object()
        .expect("normalized input schema must remain an object")
        .clone();
}

fn normalize_required_property_schemas_in_value(schema: &mut Value) {
    match schema {
        Value::Object(object) => {
            let missing_required = object
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|field| {
                    !object
                        .get("properties")
                        .and_then(Value::as_object)
                        .is_some_and(|properties| properties.contains_key(*field))
                })
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();

            if !missing_required.is_empty() {
                let properties = object
                    .entry("properties".to_string())
                    .or_insert_with(|| Value::Object(Map::new()))
                    .as_object_mut()
                    .expect("schema properties must be an object");
                for field in missing_required {
                    properties.entry(field).or_insert_with(|| json!({}));
                }
            }

            for value in object.values_mut() {
                normalize_required_property_schemas_in_value(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                normalize_required_property_schemas_in_value(value);
            }
        }
        _ => {}
    }
}

fn normalize_root_composition_schema(input_schema: &mut serde_json::Map<String, Value>) {
    if input_schema.get("type") != Some(&Value::String("object".into())) {
        return;
    }
    for key in ["anyOf", "oneOf"] {
        let Some(mut branches) = input_schema.remove(key).and_then(|value| match value {
            Value::Array(branches) => Some(branches),
            _ => None,
        }) else {
            continue;
        };
        for branch in &mut branches {
            if let Some(branch) = branch.as_object_mut() {
                branch
                    .entry("type".to_string())
                    .or_insert_with(|| Value::String("object".to_string()));
            }
        }
        let constraint = json!({key: branches});
        match input_schema.get_mut("allOf").and_then(Value::as_array_mut) {
            Some(all_of) => all_of.push(constraint),
            None => {
                input_schema.insert("allOf".to_string(), json!([constraint]));
            }
        }
    }
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

fn status_properties() -> Value {
    json!({
        "component": {"type": "string", "enum": ["browser", "phone", "phone_companion", "session_presence"]},
        "refresh_devices": optional_bool_schema(json!({
            "type": "boolean",
            "description": "For component=\"phone\" only, ask the service to refresh device discovery before reporting status."
        })),
        "session_id": phone_session_id_schema()
    })
}

fn status_constraints() -> Value {
    exact_branch_constraints(
        &status_properties(),
        "component",
        &[
            ("browser", &["component"][..], &["component"][..]),
            (
                "phone",
                &["component"][..],
                &["component", "refresh_devices"][..],
            ),
            (
                "phone_companion",
                &["component"][..],
                &["component", "session_id"][..],
            ),
            ("session_presence", &["component"][..], &["component"][..]),
        ],
    )
}

fn exact_branch_constraints(
    properties: &Value,
    discriminator: &str,
    branches: &[(&str, &[&str], &[&str])],
) -> Value {
    json!({
        "allOf": [exact_branch_one_of(
            properties,
            &branches
                .iter()
                .map(|(value, required, allowed)| {
                    (vec![(discriminator, *value)], *required, *allowed, None)
                })
                .collect::<Vec<_>>(),
        )]
    })
}

fn exact_branch_one_of(
    properties: &Value,
    branches: &[(Vec<(&str, &str)>, &[&str], &[&str], Option<Value>)],
) -> Value {
    json!({
        "oneOf": branches
            .iter()
            .map(|(discriminators, required, allowed, extra)| {
                let mut schema = exact_branch_schema(properties, discriminators, required, allowed);
                if let Some(extra) = extra {
                    merge_schema_constraints(&mut schema, extra);
                }
                schema
            })
            .collect::<Vec<_>>()
    })
}

fn exact_branch_schema_with_constraints(
    properties: &Value,
    discriminators: &[(&str, &str)],
    required: &[&str],
    allowed: &[&str],
    extra_constraints: Value,
) -> Value {
    let mut schema = exact_branch_schema(properties, discriminators, required, allowed);
    merge_schema_constraints(&mut schema, &extra_constraints);
    schema
}

fn merge_schema_constraints(schema: &mut Value, extra_constraints: &Value) {
    let schema = schema
        .as_object_mut()
        .expect("branch schema constraints target must be object");
    let extra = extra_constraints.as_object().unwrap_or_else(|| {
        panic!("extra branch constraints must be object: {extra_constraints:?}")
    });
    schema.extend(extra.clone());
}

fn exact_branch_schema(
    properties: &Value,
    discriminators: &[(&str, &str)],
    required: &[&str],
    allowed: &[&str],
) -> Value {
    let root_properties = properties
        .as_object()
        .unwrap_or_else(|| panic!("exact branch properties must be object: {properties:?}"));
    let mut branch_properties = Map::new();
    for name in allowed {
        let schema = root_properties
            .get(*name)
            .unwrap_or_else(|| panic!("exact branch references unknown property {name}"));
        branch_properties.insert((*name).to_string(), schema.clone());
    }
    for (name, value) in discriminators {
        branch_properties.insert((*name).to_string(), json!({"const": value}));
    }

    let mut required_fields = Vec::new();
    for (name, _) in discriminators {
        required_fields.push(*name);
    }
    for name in required {
        if !required_fields.contains(name) {
            required_fields.push(*name);
        }
    }

    json!({
        "type": "object",
        "properties": branch_properties,
        "required": required_fields,
        "additionalProperties": false
    })
}

fn list_resources_properties() -> Value {
    json!({
        "surface": {"type": "string", "enum": ["desktop", "browser", "phone"]},
        "resource": {"type": "string", "enum": ["apps", "windows", "focused_window", "tabs", "devices", "current_app"]},
        "target": optional_absent_string_schema(browser_target_schema()),
        "url_contains": {
            "type": ["string", "null"],
            "description": "For browser tabs only, case-insensitive URL filter."
        },
        "title_contains": {
            "type": ["string", "null"],
            "description": "For browser tabs only, case-insensitive title filter."
        },
        "include_mdns": optional_bool_schema(json!({
            "type": "boolean",
            "description": "For phone devices only, include mDNS wireless-debugging records."
        })),
        "session_id": phone_session_id_schema(),
        "include_system": optional_bool_schema(json!({
            "type": "boolean",
            "description": "For phone apps only, include system packages."
        })),
        "limit": optional_limit_schema()
    })
}

fn list_resources_constraints() -> Value {
    let properties = list_resources_properties();
    json!({
        "oneOf": [
            exact_branch_schema(&properties, &[("surface", "desktop"), ("resource", "apps")], &[], &["surface", "resource"]),
            exact_branch_schema(&properties, &[("surface", "desktop"), ("resource", "windows")], &[], &["surface", "resource"]),
            exact_branch_schema(&properties, &[("surface", "desktop"), ("resource", "focused_window")], &[], &["surface", "resource"]),
            exact_branch_schema(&properties, &[("surface", "browser"), ("resource", "tabs")], &[], &["surface", "resource", "target", "url_contains", "title_contains"]),
            exact_branch_schema(&properties, &[("surface", "phone"), ("resource", "devices")], &[], &["surface", "resource", "include_mdns"]),
            exact_branch_schema(&properties, &[("surface", "phone"), ("resource", "apps")], &["session_id"], &["surface", "resource", "session_id", "include_system", "limit"]),
            exact_branch_schema(&properties, &[("surface", "phone"), ("resource", "current_app")], &["session_id"], &["surface", "resource", "session_id"])
        ]
    })
}

fn observe_properties(can_receive_images: bool) -> Value {
    merge_properties(
        json!({
            "surface": {"type": "string", "enum": ["desktop", "browser", "phone"]},
            "target": optional_absent_string_schema(browser_target_schema()),
            "tab_id": browser_tab_id_schema(),
            "text_limit": optional_null_schema(json!({
                "type": "integer",
                "minimum": 0,
                "maximum": sky_cua_platform::model::BROWSER_SNAPSHOT_MAX_TEXT_LIMIT,
                "description": "For browser only, maximum page text characters."
            })),
            "include_accessibility": optional_bool_schema(json!({
                "type": "boolean",
                "description": "For phone only, include the accessibility tree in the observation."
            })),
            "include_notifications": optional_bool_schema(json!({
                "type": "boolean",
                "description": "For phone only, include recent notifications in the observation."
            })),
            "backend": optional_absent_string_schema(phone_observe_backend_schema())
        }),
        merge_properties(
            get_app_state_properties(can_receive_images),
            merge_properties(
                browser_snapshot_window_properties(),
                phone_session_properties(),
            ),
        ),
    )
}

fn observe_constraints(can_receive_images: bool) -> Value {
    let mut desktop_allowed = vec![
        "surface",
        "app_id",
        "desktop_file_id",
        "window_title",
        "name",
        "detail",
        "element_query",
        "element_offset",
        "element_limit",
    ];
    if can_receive_images {
        desktop_allowed.extend(["capture_screen", "screenshot_delivery"]);
    }
    let properties = observe_properties(can_receive_images);
    exact_branch_constraints(
        &properties,
        "surface",
        &[
            ("desktop", &[][..], desktop_allowed.as_slice()),
            (
                "browser",
                &["tab_id"][..],
                &[
                    "surface",
                    "target",
                    "tab_id",
                    "text_limit",
                    "element_query",
                    "element_offset",
                    "element_limit",
                ][..],
            ),
            (
                "phone",
                &["session_id"][..],
                &[
                    "surface",
                    "session_id",
                    "include_accessibility",
                    "include_notifications",
                    "backend",
                ][..],
            ),
        ],
    )
}

fn capture_screen_properties() -> Value {
    merge_properties(
        json!({
            "surface": {"type": "string", "enum": ["browser", "phone"]},
            "target": optional_absent_string_schema(browser_target_schema()),
            "tab_id": browser_tab_id_schema(),
            "backend": optional_absent_string_schema(phone_observe_backend_schema())
        }),
        phone_session_properties(),
    )
}

fn capture_screen_constraints() -> Value {
    exact_branch_constraints(
        &capture_screen_properties(),
        "surface",
        &[
            (
                "browser",
                &["tab_id"][..],
                &["surface", "target", "tab_id"][..],
            ),
            (
                "phone",
                &["session_id"][..],
                &["surface", "session_id", "backend"][..],
            ),
        ],
    )
}

fn desktop_semantic_properties(properties: Value) -> Value {
    action_tool_properties(merge_properties(properties, semantic_selector_properties()))
}

fn desktop_pointer_properties() -> Value {
    action_tool_properties(merge_properties(
        json!({
            "operation": {"type": "string", "enum": ["click", "secondary_click", "drag"]},
            "x": coordinate_schema("Click x coordinate or drag start x."),
            "y": coordinate_schema("Click y coordinate or drag start y."),
            "from_x": {"type": "number"},
            "from_y": {"type": "number"},
            "to_x": {"type": "number"},
            "to_y": {"type": "number"},
            "to_element_index": {"type": "integer", "minimum": 0},
            "duration_ms": {"type": "integer", "minimum": 0}
        }),
        semantic_selector_properties(),
    ))
}

fn desktop_selector_constraint() -> Value {
    json!({
        "anyOf": desktop_selector_alternatives()
    })
}

fn desktop_one_selector_constraint() -> Value {
    json!({
        "oneOf": desktop_selector_alternatives()
    })
}

fn desktop_snapshot_selector_constraint() -> Value {
    json!({
        "anyOf": [
            snapshot_selector_constraint(&["element_index"]),
            snapshot_selector_constraint(&["name"]),
            snapshot_selector_constraint(&["text"])
        ]
    })
}

fn desktop_point_or_selector_constraint() -> Value {
    json!({
        "anyOf": [
            {"required": ["x", "y"]},
            snapshot_selector_constraint(&["element_index"]),
            snapshot_selector_constraint(&["name"]),
            snapshot_selector_constraint(&["text"])
        ]
    })
}

fn desktop_selector_alternatives() -> Vec<Value> {
    vec![
        snapshot_selector_constraint(&["element_index"]),
        json!({"required": ["element_identifier"]}),
        snapshot_selector_constraint(&["name"]),
        snapshot_selector_constraint(&["text"]),
    ]
}

fn snapshot_selector_constraint(fields: &[&str]) -> Value {
    let mut required = vec!["snapshot_id"];
    required.extend_from_slice(fields);
    json!({
        "required": required,
        "properties": {
            "snapshot_id": {
                "type": "string",
                "minLength": 1,
                "pattern": ".*\\S.*"
            }
        }
    })
}

fn desktop_pointer_constraints() -> Value {
    let properties = desktop_pointer_properties();
    let branches = vec![
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "click")],
            &[],
            &desktop_pointer_click_allowed_fields(),
            desktop_point_or_selector_constraint(),
        ),
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "secondary_click")],
            &[],
            &desktop_pointer_click_allowed_fields(),
            desktop_point_or_selector_constraint(),
        ),
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "drag")],
            &[],
            &desktop_pointer_drag_allowed_fields(),
            json!({
                "anyOf": [
                    {"required": ["from_x", "from_y", "to_x", "to_y"]},
                    {"required": ["x", "y", "to_x", "to_y"]},
                    snapshot_selector_constraint(&["element_index", "to_element_index"]),
                    {
                        "allOf": [
                            snapshot_selector_constraint(&["element_index"]),
                            {"required": ["to_x", "to_y"]}
                        ]
                    },
                    {
                        "allOf": [
                            snapshot_selector_constraint(&["to_element_index"]),
                            {"required": ["from_x", "from_y"]}
                        ]
                    },
                    {
                        "allOf": [
                            snapshot_selector_constraint(&["to_element_index"]),
                            {"required": ["x", "y"]}
                        ]
                    }
                ]
            }),
        ),
    ];
    json!({
        "allOf": [{"oneOf": branches}]
    })
}

fn desktop_selector_allowed_fields() -> [&'static str; 7] {
    [
        "snapshot_id",
        "element_index",
        "element_identifier",
        "role",
        "name",
        "text",
        "states",
    ]
}

fn desktop_window_target_allowed_fields() -> [&'static str; 9] {
    [
        "window_id",
        "pid",
        "tty",
        "terminal_pid",
        "terminal_command",
        "terminal_cwd",
        "app_id",
        "wm_class",
        "title",
    ]
}

fn desktop_pointer_click_allowed_fields() -> Vec<&'static str> {
    let mut fields = vec!["operation", "x", "y"];
    fields.extend(desktop_selector_allowed_fields());
    fields
}

fn desktop_pointer_drag_allowed_fields() -> Vec<&'static str> {
    let mut fields = vec![
        "operation",
        "x",
        "y",
        "from_x",
        "from_y",
        "to_x",
        "to_y",
        "to_element_index",
        "duration_ms",
    ];
    fields.extend(desktop_selector_allowed_fields());
    fields
}

fn desktop_keyboard_allowed_fields(branch_field: &'static str) -> Vec<&'static str> {
    let mut fields = vec!["operation", branch_field, "snapshot_id"];
    fields.extend(desktop_window_target_allowed_fields());
    fields
}

fn desktop_action_allowed_fields(action_fields: &[&'static str]) -> Vec<&'static str> {
    let mut fields = vec!["operation"];
    fields.extend(desktop_selector_allowed_fields());
    fields.extend(action_fields.iter().copied());
    fields
}

fn desktop_keyboard_properties() -> Value {
    action_tool_properties(keyboard_target_properties(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": non_empty_string_schema(),
        "key": non_empty_string_schema()
    })))
}

fn desktop_keyboard_constraints() -> Value {
    exact_branch_constraints(
        &desktop_keyboard_properties(),
        "operation",
        &[
            (
                "type_text",
                &["text"][..],
                &desktop_keyboard_allowed_fields("text"),
            ),
            (
                "press_key",
                &["key"][..],
                &desktop_keyboard_allowed_fields("key"),
            ),
        ],
    )
}

fn desktop_action_properties() -> Value {
    action_tool_properties(merge_properties(
        json!({
            "operation": {"type": "string", "enum": ["activate", "perform_action"]},
            "action_name": {
                "type": "string",
                "minLength": 1,
                "pattern": ".*\\S.*",
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

fn desktop_action_constraints() -> Value {
    let properties = desktop_action_properties();
    let branches = vec![
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "activate")],
            &[],
            &desktop_action_allowed_fields(&[]),
            desktop_selector_constraint(),
        ),
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "perform_action")],
            &[],
            &desktop_action_allowed_fields(&["action_name", "action_index"]),
            desktop_selector_action_constraint(),
        ),
    ];
    json!({
        "allOf": [{"oneOf": branches}]
    })
}

fn desktop_selector_action_constraint() -> Value {
    json!({
        "allOf": [
            desktop_one_selector_constraint(),
            {
                "anyOf": [
                    {"required": ["action_name"]},
                    {"required": ["action_index"]}
                ]
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
        optional_absent_string_schema(json!({
            "type": "string",
            "minLength": 1,
            "description": "Exact snapshot_id from the same observe(surface=\"desktop\") or capture_desktop result that supplied this target."
        })),
    );
    properties
}

fn semantic_selector_properties() -> Value {
    json!({
        "element_index": {
            "type": "integer",
            "minimum": 0,
            "description": "Element index from the same desktop observation; pair with its snapshot_id."
        },
        "element_identifier": {
            "type": "string",
            "minLength": 1,
            "pattern": ".*\\S.*",
            "description": "Exact backend_ref from a desktop observation; direct semantic target that does not require snapshot_id."
        },
        "role": {
            "type": "string",
            "description": "Optional selector refiner; not a standalone target."
        },
        "name": {
            "type": "string",
            "minLength": 1,
            "pattern": ".*\\S.*",
            "description": "Snapshot-scoped element name selector; requires snapshot_id."
        },
        "text": {
            "type": "string",
            "minLength": 1,
            "pattern": ".*\\S.*",
            "description": "Snapshot-scoped element text selector; requires snapshot_id."
        },
        "states": {
            "type": "array",
            "items": non_blank_string_schema(),
            "description": "Optional selector refiners; not a standalone target."
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

fn non_blank_string_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "pattern": ".*\\S.*"
    })
}

fn browser_tab_id_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "pattern": ".*\\S.*",
        "description": "Browser tab_id returned by browser_open or list_resources(surface=\"browser\", resource=\"tabs\"). Claim listed existing tabs before acting."
    })
}

fn browser_tab_properties() -> Value {
    json!({
        "target": optional_absent_string_schema(browser_target_schema()),
        "tab_id": browser_tab_id_schema()
    })
}

fn browser_target_url_properties(require_tab: bool) -> Value {
    let mut properties = json!({
        "target": optional_absent_string_schema(browser_target_schema()),
        "url": if require_tab { browser_url_schema() } else { optional_absent_string_schema(browser_url_schema()) }
    });
    if require_tab && let Some(map) = properties.as_object_mut() {
        map.insert("tab_id".to_string(), browser_tab_id_schema());
    }
    properties
}

fn browser_point_properties() -> Value {
    merge_properties(
        browser_tab_properties(),
        json!({
            "x": {"type": "number", "minimum": 0, "description": "CSS pixel x coordinate."},
            "y": {"type": "number", "minimum": 0, "description": "CSS pixel y coordinate."},
            "wait_for_arrival": optional_bool_schema(json!({
                "type": "boolean",
                "description": "Wait for the visible cursor overlay to arrive. Defaults to true."
            }))
        }),
    )
}

fn browser_xy_properties() -> Value {
    merge_properties(
        browser_tab_properties(),
        json!({
            "x": {"type": "number", "minimum": 0, "description": "CSS pixel x coordinate."},
            "y": {"type": "number", "minimum": 0, "description": "CSS pixel y coordinate."}
        }),
    )
}

fn browser_optional_xy_properties() -> Value {
    merge_properties(
        browser_tab_properties(),
        json!({
            "x": optional_null_schema(json!({"type": "number", "minimum": 0, "description": "CSS pixel x coordinate."})),
            "y": optional_null_schema(json!({"type": "number", "minimum": 0, "description": "CSS pixel y coordinate."}))
        }),
    )
}

fn browser_input_properties() -> Value {
    merge_properties(
        browser_xy_properties(),
        json!({
            "operation": {"type": "string", "enum": ["click", "type_text", "press_key"]},
            "text": non_empty_string_schema(),
            "key": non_blank_string_schema()
        }),
    )
}

fn browser_input_constraints() -> Value {
    exact_branch_constraints(
        &browser_input_properties(),
        "operation",
        &[
            (
                "click",
                &["tab_id", "x", "y"][..],
                &["operation", "target", "tab_id", "x", "y"][..],
            ),
            (
                "type_text",
                &["tab_id", "text"][..],
                &["operation", "target", "tab_id", "text"][..],
            ),
            (
                "press_key",
                &["tab_id", "key"][..],
                &["operation", "target", "tab_id", "key"][..],
            ),
        ],
    )
}

fn browser_scroll_properties() -> Value {
    merge_properties(
        browser_optional_xy_properties(),
        json!({
            "delta_x": {"type": "number", "description": "Horizontal wheel delta in CSS pixels; at least one delta must be non-zero."},
            "delta_y": {"type": "number", "description": "Vertical wheel delta in CSS pixels; at least one delta must be non-zero."}
        }),
    )
}

fn browser_scroll_constraints() -> Value {
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
                "if": {
                    "anyOf": [
                        {"required": ["x"], "properties": {"x": {"type": "number"}}},
                        {"required": ["y"], "properties": {"y": {"type": "number"}}}
                    ]
                },
                "then": {
                    "required": ["x", "y"],
                    "properties": {
                        "x": {"type": "number"},
                        "y": {"type": "number"}
                    }
                }
            }
        ]
    })
}

fn browser_snapshot_window_properties() -> Value {
    json!({
        "element_query": optional_absent_string_schema(json!({"type": "string", "maxLength": APP_STATE_MAX_ELEMENT_QUERY_CHARS})),
        "element_offset": optional_null_schema(json!({"type": "integer", "minimum": 0})),
        "element_limit": optional_null_schema(json!({
            "type": "integer",
            "minimum": 0,
            "maximum": sky_cua_platform::model::BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT
        }))
    })
}

fn browser_url_schema() -> Value {
    json!({
        "type": "string",
        "pattern": "^(https?://[^\\s]+|about:blank)$"
    })
}

fn optional_absent_string_schema(schema: Value) -> Value {
    json!({
        "anyOf": [
            schema,
            {"type": "string", "const": ""},
            {"type": "null"}
        ]
    })
}

fn optional_null_schema(schema: Value) -> Value {
    json!({
        "anyOf": [
            schema,
            {"type": "null"}
        ]
    })
}

fn optional_bool_schema(schema: Value) -> Value {
    optional_null_schema(schema)
}

fn optional_zero_integer_schema(schema: Value) -> Value {
    json!({
        "anyOf": [
            schema,
            {"type": "integer", "const": 0},
            {"type": "null"}
        ]
    })
}

fn phone_session_id_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "Phone session_id returned by phone_connection(operation=\"connect\") or status(component=\"phone\") active sessions; required after connect."
    })
}

fn phone_serial_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "ADB serial from phone discovery; accepted only for discovery, pairing, and connect-like paths."
    })
}

fn phone_connect_backend_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["auto", "adb", "companion", "scrcpy"],
        "description": "Backend hint for phone connect."
    })
}

fn phone_observe_backend_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["auto", "adb", "companion"],
        "description": "Backend hint for phone observe and screenshot. scrcpy/none are response states, not request inputs."
    })
}

fn phone_selector_properties() -> Value {
    json!({
        "session_id": phone_session_id_schema(),
        "serial": optional_absent_string_schema(phone_serial_schema())
    })
}

fn with_phone_selector(properties: Value) -> Value {
    merge_properties(properties, phone_selector_properties())
}

fn phone_session_properties() -> Value {
    json!({
        "session_id": phone_session_id_schema()
    })
}

fn with_phone_session(properties: Value) -> Value {
    merge_properties(properties, phone_session_properties())
}

fn limit_schema() -> Value {
    json!({"type": "integer", "minimum": 0})
}

fn optional_limit_schema() -> Value {
    optional_null_schema(limit_schema())
}

fn phone_connection_properties() -> Value {
    merge_properties(
        with_phone_selector(json!({
            "operation": {"type": "string", "enum": ["connect", "disconnect", "refresh"]},
            "backend": optional_absent_string_schema(phone_connect_backend_schema()),
            "install_companion": optional_bool_schema(json!({"type": "boolean"})),
            "start_scrcpy": optional_bool_schema(json!({"type": "boolean"})),
            "keep_wireless": optional_bool_schema(json!({"type": "boolean"}))
        })),
        json!({}),
    )
}

fn phone_connection_constraints() -> Value {
    exact_branch_constraints(
        &phone_connection_properties(),
        "operation",
        &[
            (
                "connect",
                &[][..],
                &[
                    "operation",
                    "serial",
                    "backend",
                    "install_companion",
                    "start_scrcpy",
                ][..],
            ),
            (
                "disconnect",
                &["session_id"][..],
                &["operation", "session_id", "keep_wireless"][..],
            ),
            (
                "refresh",
                &["session_id"][..],
                &["operation", "session_id"][..],
            ),
        ],
    )
}

fn phone_setup_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["install_companion", "open_settings"]},
        "force_reinstall": optional_bool_schema(json!({"type": "boolean"})),
        "allow_downgrade": optional_bool_schema(json!({"type": "boolean"})),
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
        "package_name": optional_absent_string_schema(json!({
            "type": "string",
            "minLength": 1,
            "description": "Target package for app-scoped screens such as app_details."
        }))
    }))
}

fn phone_setup_constraints() -> Value {
    let properties = phone_setup_properties();
    let branches = vec![
        exact_branch_schema(
            &properties,
            &[("operation", "install_companion")],
            &["session_id"],
            &[
                "operation",
                "session_id",
                "force_reinstall",
                "allow_downgrade",
            ],
        ),
        exact_branch_schema(
            &properties,
            &[("operation", "open_settings")],
            &["session_id", "screen"],
            &["operation", "session_id", "screen", "package_name"],
        ),
    ];
    json!({
        "allOf": [
            {"oneOf": branches},
            {
                "if": {
                    "properties": {
                        "operation": {"const": "open_settings"},
                        "screen": {"const": "app_details"}
                    },
                    "required": ["operation", "screen"]
                },
                "then": {
                    "required": ["package_name"],
                    "properties": {
                        "package_name": {
                            "type": "string",
                            "minLength": 1,
                            "pattern": ".*\\S.*"
                        }
                    }
                }
            }
        ]
    })
}

fn phone_pointer_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["tap", "swipe"]},
        "phone_snapshot_id": {
            "type": "string",
            "minLength": 1,
            "pattern": ".*\\S.*",
            "description": "Fresh phone_snapshot_id from the same phone observe/capture_screen result that supplied screenshot coordinates."
        },
        "x": {"type": "number", "minimum": 0, "description": "Tap x coordinate in snapshot pixels, or raw device pixels when use_device_coordinates=true."},
        "y": {"type": "number", "minimum": 0, "description": "Tap y coordinate in snapshot pixels, or raw device pixels when use_device_coordinates=true."},
        "start_x": {"type": "number", "minimum": 0, "description": "Swipe start x coordinate in snapshot pixels, or raw device pixels when use_device_coordinates=true."},
        "start_y": {"type": "number", "minimum": 0, "description": "Swipe start y coordinate in snapshot pixels, or raw device pixels when use_device_coordinates=true."},
        "end_x": {"type": "number", "minimum": 0, "description": "Swipe end x coordinate in snapshot pixels, or raw device pixels when use_device_coordinates=true."},
        "end_y": {"type": "number", "minimum": 0, "description": "Swipe end y coordinate in snapshot pixels, or raw device pixels when use_device_coordinates=true."},
        "duration_ms": optional_null_schema(json!({"type": "integer", "minimum": 0})),
        "use_device_coordinates": optional_bool_schema(json!({"type": "boolean", "description": "When true, x/y or start/end coordinates are raw device pixels and phone_snapshot_id is not required."}))
    }))
}

fn phone_pointer_constraints() -> Value {
    let properties = phone_pointer_properties();
    let branches = vec![
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "tap")],
            &["session_id", "x", "y"],
            &[
                "operation",
                "session_id",
                "phone_snapshot_id",
                "x",
                "y",
                "use_device_coordinates",
            ],
            json!({
                "anyOf": [
                    {"required": ["phone_snapshot_id"]},
                    {"properties": {"use_device_coordinates": {"const": true}}, "required": ["use_device_coordinates"]}
                ]
            }),
        ),
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "swipe")],
            &["session_id", "start_x", "start_y", "end_x", "end_y"],
            &[
                "operation",
                "session_id",
                "phone_snapshot_id",
                "start_x",
                "start_y",
                "end_x",
                "end_y",
                "duration_ms",
                "use_device_coordinates",
            ],
            json!({
                "anyOf": [
                    {"required": ["phone_snapshot_id"]},
                    {"properties": {"use_device_coordinates": {"const": true}}, "required": ["use_device_coordinates"]}
                ]
            }),
        ),
    ];
    json!({
        "allOf": [{"oneOf": branches}]
    })
}

fn phone_keyboard_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": non_empty_string_schema(),
        "key": non_blank_string_schema()
    }))
}

fn phone_keyboard_constraints() -> Value {
    exact_branch_constraints(
        &phone_keyboard_properties(),
        "operation",
        &[
            (
                "type_text",
                &["session_id", "text"][..],
                &["operation", "session_id", "text"][..],
            ),
            (
                "press_key",
                &["session_id", "key"][..],
                &["operation", "session_id", "key"][..],
            ),
        ],
    )
}

fn phone_notification_action_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["open", "dismiss", "action"]},
        "event_id": {
            "type": "string",
            "minLength": 1,
            "pattern": ".*\\S.*",
            "description": "Exact event_id from a fresh phone_notifications result or notification-bearing phone observation."
        },
        "action_id": {
            "type": "string",
            "minLength": 1,
            "pattern": ".*\\S.*",
            "description": "Exact action_id from that same notification event."
        }
    }))
}

fn phone_notification_action_constraints() -> Value {
    exact_branch_constraints(
        &phone_notification_action_properties(),
        "operation",
        &[
            (
                "open",
                &["session_id", "event_id"][..],
                &["operation", "session_id", "event_id"][..],
            ),
            (
                "dismiss",
                &["session_id", "event_id"][..],
                &["operation", "session_id", "event_id"][..],
            ),
            (
                "action",
                &["session_id", "event_id", "action_id"][..],
                &["operation", "session_id", "event_id", "action_id"][..],
            ),
        ],
    )
}

fn phone_app_action_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["launch", "open_intent"]},
        "package_name": optional_absent_string_schema(json!({
            "type": "string",
            "minLength": 1,
            "pattern": ".*\\S.*",
            "description": "Optional exact Android package name from phone app listing or current-app result, not a display label."
        })),
        "intent_uri": {
            "type": "string",
            "minLength": 1,
            "pattern": ".*\\S.*",
            "description": "Intent URI or deep link."
        }
    }))
}

fn phone_app_action_constraints() -> Value {
    let properties = phone_app_action_properties();
    let mut launch_properties = properties.clone();
    if let Some(property_map) = launch_properties.as_object_mut() {
        property_map.insert(
            "package_name".to_string(),
            json!({
                "type": "string",
                "minLength": 1,
                "pattern": ".*\\S.*",
                "description": "Exact Android package name from phone app listing or current-app result, not a display label."
            }),
        );
    }
    let branches = vec![
        exact_branch_schema(
            &launch_properties,
            &[("operation", "launch")],
            &["session_id", "package_name"],
            &["operation", "session_id", "package_name"],
        ),
        exact_branch_schema(
            &properties,
            &[("operation", "open_intent")],
            &["session_id", "intent_uri"],
            &["operation", "session_id", "intent_uri", "package_name"],
        ),
    ];
    json!({
        "allOf": [{"oneOf": branches}]
    })
}

fn phone_app_install_properties() -> Value {
    with_phone_session(json!({
        "apk_paths": {
            "type": "array",
            "minItems": 1,
            "items": non_blank_string_schema(),
            "description": "Host-side APK path(s) to install. Use one path for single APK installs and multiple paths for split or multi-package installs."
        },
        "mode": optional_null_schema(json!({"type": "string", "enum": ["single", "multiple", "multi_package"], "description": "Install strategy hint."})),
        "reinstall": optional_bool_schema(json!({"type": "boolean"})),
        "allow_downgrade": optional_bool_schema(json!({"type": "boolean"})),
        "allow_test_apk": optional_bool_schema(json!({"type": "boolean"})),
        "grant_runtime_permissions": optional_bool_schema(json!({"type": "boolean"}))
    }))
}

fn phone_app_install_constraints() -> Value {
    json!({
        "required": ["session_id", "apk_paths"]
    })
}

fn get_app_state_properties(can_receive_images: bool) -> Value {
    let mut properties = json!({
        "app_id": { "type": ["string", "null"] },
        "desktop_file_id": { "type": ["string", "null"] },
        "window_title": { "type": ["string", "null"] },
        "name": { "type": ["string", "null"] },
        "detail": optional_null_schema(json!({
            "type": "string",
            "enum": ["full", "compact"],
            "description": "Defaults to compact. Use full for verbose element details and full capability data."
        })),
        "element_query": optional_absent_string_schema(json!({
            "type": "string",
            "maxLength": APP_STATE_MAX_ELEMENT_QUERY_CHARS,
            "description": "Case-insensitive filter over element role/name/description/value/text/states/actions."
        })),
        "element_offset": optional_null_schema(json!({
            "type": "integer",
            "minimum": 0,
            "description": "Zero-based offset into matching elements."
        })),
        "element_limit": optional_null_schema(json!({
            "type": "integer",
            "minimum": 0,
            "maximum": APP_STATE_MAX_ELEMENT_LIMIT,
            "description": format!("Maximum matching elements returned. compact defaults to {APP_STATE_DEFAULT_ELEMENT_LIMIT}; 0 keeps metadata only. element_count is the full total.")
        }))
    });

    if can_receive_images && let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "capture_screen".to_string(),
            optional_null_schema(json!({
                "type": "string",
                "enum": ["auto", "if_changed", "always", "never"],
                "description": "Screen-capture policy. Defaults to if_changed. Use always for a fresh frame, never for structure-only loops."
            })),
        );
        property_map.insert(
            "screenshot_delivery".to_string(),
            optional_null_schema(json!({
                "type": "string",
                "enum": ["path", "inline"],
                "description": "path returns capture.inspection_image_path metadata; inline also attaches the inspection image block."
            })),
        );
    }

    properties
}

fn screenshot_properties(can_receive_images: bool) -> Value {
    let mut properties = window_target_schema();

    if let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "display_id".to_string(),
            optional_absent_string_schema(json!({
                "type": "string",
                "minLength": 1,
                "description": "Exact display_id from environment.displays."
            })),
        );
        property_map.insert(
            "display_name".to_string(),
            optional_absent_string_schema(json!({
                "type": "string",
                "minLength": 1,
                "description": "Display name/connector from environment.displays. Prefer display_id."
            })),
        );
        property_map.insert(
            "display_index".to_string(),
            optional_null_schema(json!({
                "type": "integer",
                "minimum": 0,
                "description": "Zero-based display index from environment.displays. Prefer display_id."
            })),
        );
    }

    if can_receive_images && let Some(property_map) = properties.as_object_mut() {
        property_map.insert(
            "screenshot_delivery".to_string(),
            optional_null_schema(json!({
                "type": "string",
                "enum": ["path", "inline"],
                "description": "path returns capture.inspection_image_path metadata; inline also attaches the inspection image block."
            })),
        );
    }

    properties
}

fn screenshot_constraints() -> Value {
    json!({
        "not": {
            "anyOf": [
                {"allOf": [
                    any_active_selector_constraint(&WINDOW_SELECTOR_KEYS),
                    one_active_selector_constraint(&DISPLAY_SELECTOR_KEYS)
                ]},
                {"anyOf": same_group_pair_constraints(&DISPLAY_SELECTOR_KEYS)}
            ]
        }
    })
}

const WINDOW_SELECTOR_KEYS: [&str; 9] = [
    "window_id",
    "pid",
    "tty",
    "terminal_pid",
    "terminal_command",
    "terminal_cwd",
    "app_id",
    "wm_class",
    "title",
];

const DISPLAY_SELECTOR_KEYS: [&str; 3] = ["display_id", "display_name", "display_index"];

fn any_active_selector_constraint(selectors: &[&str]) -> Value {
    json!({
        "anyOf": selectors.iter().map(|selector| active_selector_constraint(selector)).collect::<Vec<_>>()
    })
}

fn one_active_selector_constraint(selectors: &[&str]) -> Value {
    json!({
        "oneOf": selectors.iter().map(|selector| active_selector_constraint(selector)).collect::<Vec<_>>()
    })
}

fn same_group_pair_constraints(keys: &[&str]) -> Vec<Value> {
    let mut constraints = Vec::new();
    for (index, left) in keys.iter().enumerate() {
        for right in keys.iter().skip(index + 1) {
            constraints.push(json!({
                "allOf": [
                    active_selector_constraint(left),
                    active_selector_constraint(right)
                ]
            }));
        }
    }
    constraints
}

fn active_selector_constraint(selector: &str) -> Value {
    let schema = match selector {
        "pid" | "terminal_pid" => json!({"type": "integer", "minimum": 1}),
        "display_index" => json!({"type": "integer", "minimum": 0}),
        _ => json!({"type": "string", "minLength": 1, "pattern": ".*\\S.*"}),
    };
    json!({
        "required": [selector],
        "properties": {
            selector: schema
        }
    })
}

fn coordinate_schema(description: &str) -> Value {
    json!({
        "type": "number",
        "description": description
    })
}

fn window_target_schema() -> Value {
    json!({
        "window_id": optional_absent_string_schema(json!({
            "type": "string",
            "minLength": 1,
            "description": "Exact window_id from list_resources(surface=\"desktop\", resource=\"windows\")."
        })),
        "pid": optional_zero_integer_schema(json!({
            "type": "integer",
            "minimum": 1,
            "description": "Process ID from list_resources(surface=\"desktop\", resource=\"windows\"). 0 is ignored."
        })),
        "tty": optional_absent_string_schema(json!({
            "type": "string",
            "minLength": 1,
            "description": "Terminal tty such as /dev/pts/7 or pts/7."
        })),
        "terminal_pid": optional_zero_integer_schema(json!({
            "type": "integer",
            "minimum": 1,
            "description": "Terminal process ID from desktop window terminal metadata. 0 is ignored."
        })),
        "terminal_command": optional_absent_string_schema(non_empty_string_schema()),
        "terminal_cwd": optional_absent_string_schema(non_empty_string_schema()),
        "app_id": optional_absent_string_schema(non_empty_string_schema()),
        "wm_class": optional_absent_string_schema(non_empty_string_schema()),
        "title": optional_absent_string_schema(non_empty_string_schema())
    })
}

fn window_target_constraint() -> Value {
    any_active_selector_constraint(&WINDOW_SELECTOR_KEYS)
}

fn session_presence_constraints() -> Value {
    exact_branch_constraints(
        &json!({
            "operation": {"type": "string", "enum": ["hold", "unlock", "release"]},
            "unlock": {"type": "boolean"},
            "inhibit_lock": {"type": "boolean"},
            "inhibit_suspend": {"type": "boolean"},
            "relock": {"type": "boolean"}
        }),
        "operation",
        &[
            (
                "hold",
                &[][..],
                &["operation", "unlock", "inhibit_lock", "inhibit_suspend"][..],
            ),
            (
                "unlock",
                &[][..],
                &["operation", "inhibit_lock", "inhibit_suspend"][..],
            ),
            ("release", &[][..], &["operation", "relock"][..]),
        ],
    )
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

pub(crate) fn schema_accepts(schema: &Value, instance: &Value) -> bool {
    let Some(schema) = schema.as_object() else {
        return true;
    };
    if !schema
        .keys()
        .all(|keyword| schema_keyword_is_supported(keyword))
    {
        return false;
    }
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
        if !schema_pattern_accepts(pattern, value) {
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

fn schema_pattern_accepts(pattern: &str, value: &str) -> bool {
    match pattern {
        "^(https?://[^\\s]+|about:blank)$" => {
            value == "about:blank"
                || url_with_scheme_and_non_empty_rest(value, "http://")
                || url_with_scheme_and_non_empty_rest(value, "https://")
        }
        ".*\\S.*" => value.chars().any(|character| !character.is_whitespace()),
        _ => false,
    }
}

fn schema_keyword_is_supported(keyword: &str) -> bool {
    matches!(
        keyword,
        "additionalProperties"
            | "allOf"
            | "anyOf"
            | "const"
            | "description"
            | "enum"
            | "exclusiveMaximum"
            | "exclusiveMinimum"
            | "if"
            | "items"
            | "maxItems"
            | "maxLength"
            | "maxProperties"
            | "maximum"
            | "minItems"
            | "minLength"
            | "minProperties"
            | "minimum"
            | "not"
            | "oneOf"
            | "pattern"
            | "properties"
            | "required"
            | "then"
            | "type"
    )
}

fn url_with_scheme_and_non_empty_rest(value: &str, scheme: &str) -> bool {
    value
        .strip_prefix(scheme)
        .is_some_and(|rest| !rest.is_empty() && !rest.chars().any(char::is_whitespace))
}

fn schema_type_accepts(expected_type: &Value, instance: &Value) -> bool {
    match expected_type {
        Value::String(expected_type) => schema_single_type_accepts(expected_type, instance),
        Value::Array(expected_types) => expected_types.iter().any(|expected_type| {
            expected_type
                .as_str()
                .is_some_and(|expected_type| schema_single_type_accepts(expected_type, instance))
        }),
        _ => false,
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
        _ => false,
    }
}

#[cfg(test)]
mod annotation_tests {
    use super::{
        InactiveToolReason, McpConfigDiagnostic, McpProcessConfig, build_tool_definitions,
        build_tool_registry, mcp_process_config_from_env, schema_accepts,
        schema_keyword_is_supported,
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
        ("phone_setup", (false, true, true, false)),
        ("phone_app_force_stop", (false, true, true, false)),
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

    fn exact_branch<'a>(schema: &'a Value, discriminator: &str, value: &str) -> &'a Value {
        schema["allOf"]
            .as_array()
            .and_then(|all_of| {
                all_of.iter().find_map(|constraint| {
                    constraint["oneOf"].as_array().and_then(|one_of| {
                        one_of
                            .iter()
                            .find(|branch| branch["properties"][discriminator]["const"] == value)
                    })
                })
            })
            .unwrap_or_else(|| panic!("missing {discriminator}={value} exact branch"))
    }

    fn assert_no_duplicate_merge_keys_in_all_of(schema: &Value) {
        match schema {
            Value::Object(object) => {
                if let Some(all_of) = object.get("allOf").and_then(Value::as_array) {
                    for key in ["if", "not", "anyOf", "oneOf"] {
                        let key_count = all_of
                            .iter()
                            .filter(|item| {
                                item.as_object()
                                    .is_some_and(|object| object.contains_key(key))
                            })
                            .count();
                        assert!(
                            key_count <= 1,
                            "schema contains duplicate {key} constraints under one allOf: {schema:?}"
                        );
                    }
                }
                for value in object.values() {
                    assert_no_duplicate_merge_keys_in_all_of(value);
                }
            }
            Value::Array(items) => {
                for value in items {
                    assert_no_duplicate_merge_keys_in_all_of(value);
                }
            }
            _ => {}
        }
    }

    fn assert_required_properties_are_self_contained(schema: &Value) {
        match schema {
            Value::Object(object) => {
                if let Some(required) = object.get("required").and_then(Value::as_array) {
                    let properties = object.get("properties").and_then(Value::as_object);
                    for field in required.iter().filter_map(Value::as_str) {
                        assert!(
                            properties.is_some_and(|properties| properties.contains_key(field)),
                            "schema requires {field:?} without defining it in local properties: {schema:?}"
                        );
                    }
                }
                for value in object.values() {
                    assert_required_properties_are_self_contained(value);
                }
            }
            Value::Array(items) => {
                for value in items {
                    assert_required_properties_are_self_contained(value);
                }
            }
            _ => {}
        }
    }

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
    fn grouped_action_tool_schemas_reject_vague_desktop_actions() {
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
            exact_branch(pointer_schema, "operation", "click")["anyOf"]
                .as_array()
                .is_some_and(|any_of| {
                    any_of
                        .iter()
                        .any(|item| item["required"] == json!(["x", "y"]))
                        && any_of
                            .iter()
                            .any(|item| item["required"] == json!(["snapshot_id", "element_index"]))
                        && !any_of
                            .iter()
                            .any(|item| item["required"] == json!(["element_identifier"]))
                }),
            "desktop_pointer click branch must require coordinates or a snapshot selector"
        );
        assert!(
            exact_branch(pointer_schema, "operation", "drag")["anyOf"]
                .as_array()
                .is_some_and(|any_of| {
                    !any_of.iter().any(|item| {
                        item["required"]
                            == json!(["snapshot_id", "element_identifier", "to_element_index"])
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

        let activate_schema = &tool("activate_window")["inputSchema"];
        assert!(
            activate_schema["allOf"].as_array().is_some_and(|all_of| {
                all_of.iter().any(|constraint| {
                    constraint["anyOf"].as_array().is_some_and(|any_of| {
                        any_of
                            .iter()
                            .any(|item| item["required"] == json!(["window_id"]))
                            && any_of.iter().any(|item| item["required"] == json!(["pid"]))
                    })
                })
            }),
            "activate_window must require at least one active window target"
        );
        assert_eq!(
            activate_schema["properties"]["window_id"]["anyOf"][0]["minLength"],
            1
        );
        assert_eq!(
            activate_schema["properties"]["pid"]["anyOf"][0]["minimum"],
            1
        );

        let list_resources_schema = &tool("list_resources")["inputSchema"];
        let list_resource_pairs = list_resources_schema["allOf"]
            .as_array()
            .and_then(|all_of| {
                all_of
                    .iter()
                    .find_map(|constraint| constraint["oneOf"].as_array())
            })
            .expect("list_resources oneOf constraint");
        assert!(
            list_resource_pairs.iter().any(|pair| {
                pair["properties"]["surface"]["const"] == "browser"
                    && pair["properties"]["resource"]["const"] == "tabs"
            }) && list_resource_pairs.iter().any(|pair| {
                pair["properties"]["surface"]["const"] == "phone"
                    && pair["properties"]["resource"]["const"] == "current_app"
            }),
            "list_resources must constrain surface/resource pairs to dispatchable branches"
        );

        for schema in tools.iter().map(|tool| &tool["inputSchema"]) {
            assert_eq!(
                schema["type"],
                Value::String("object".to_string()),
                "MCP adapters expect root inputSchema.type=object"
            );
            assert_no_duplicate_merge_keys_in_all_of(schema);
            assert_required_properties_are_self_contained(schema);
            for key in ["anyOf", "oneOf"] {
                assert!(
                    schema.get(key).is_none(),
                    "root {key} schemas must move composition under allOf for opencode-go/moonshot compatibility"
                );
            }
        }

        let observe_schema = &tool("observe")["inputSchema"];
        assert!(
            exact_branch(observe_schema, "surface", "browser")["required"]
                == json!(["surface", "tab_id"]),
            "observe browser branch must require tab_id"
        );

        let capture_schema = &tool("capture_screen")["inputSchema"];
        assert!(
            exact_branch(capture_schema, "surface", "browser")["required"]
                == json!(["surface", "tab_id"]),
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
                        constraint["if"]["anyOf"].as_array().is_some_and(|any_of| {
                            constraint["then"]["required"] == json!(["x", "y"])
                                && constraint["then"]["properties"]["x"]["type"] == "number"
                                && constraint["then"]["properties"]["y"]["type"] == "number"
                                && any_of.iter().any(|item| item["required"] == json!(["x"]))
                                && any_of.iter().any(|item| item["required"] == json!(["y"]))
                        })
                    })
                }),
            "browser_scroll must require numeric x/y together"
        );

        assert_eq!(
            tool("browser_navigate")["inputSchema"]["properties"]["url"]["pattern"],
            "^(https?://[^\\s]+|about:blank)$"
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
        let snapshot_id_options = action_schema["properties"]["snapshot_id"]["anyOf"]
            .as_array()
            .expect("desktop snapshot_id should allow absent sentinels");
        assert!(
            snapshot_id_options
                .iter()
                .any(|schema| schema["type"] == "string" && schema["minLength"] == 1)
                && snapshot_id_options
                    .iter()
                    .any(|schema| schema["type"] == "string" && schema["const"] == "")
                && snapshot_id_options
                    .iter()
                    .any(|schema| schema["type"] == "null"),
            "desktop snapshot_id should advertise non-empty values plus blank/null absent sentinels"
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
            exact_branch(action_schema, "operation", "activate")["anyOf"]
                .as_array()
                .is_some_and(|any_of| {
                    any_of
                        .iter()
                        .any(|item| item["required"] == json!(["snapshot_id", "element_index"]))
                        && any_of
                            .iter()
                            .any(|item| item["required"] == json!(["element_identifier"]))
                        && any_of
                            .iter()
                            .any(|item| item["required"] == json!(["snapshot_id", "name"]))
                        && any_of
                            .iter()
                            .any(|item| item["required"] == json!(["snapshot_id", "text"]))
                }),
            "desktop_action must require snapshot_id for snapshot-bound selectors"
        );
        assert!(
            exact_branch(action_schema, "operation", "perform_action")["allOf"]
                .as_array()
                .is_some_and(|all_of| {
                    all_of.iter().any(|item| {
                        item["oneOf"].as_array().is_some_and(|one_of| {
                            one_of.iter().any(|selector| {
                                selector["required"] == json!(["snapshot_id", "element_index"])
                            })
                        })
                    }) && all_of.iter().any(|item| {
                        item["anyOf"].as_array().is_some_and(|any_of| {
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
            exact_branch(keyboard_schema, "operation", "press_key")["required"]
                == json!(["operation", "key"])
                && exact_branch(keyboard_schema, "operation", "press_key")["additionalProperties"]
                    == json!(false),
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
            "desktop_scroll should expose the pages field"
        );
        let desktop_scroll_any_of = tool("desktop_scroll")["inputSchema"]["allOf"]
            .as_array()
            .and_then(|all_of| {
                all_of
                    .iter()
                    .find_map(|constraint| constraint["anyOf"].as_array())
            })
            .expect("desktop_scroll anyOf constraint");
        assert!(
            desktop_scroll_any_of
                .iter()
                .any(|item| item["required"] == json!(["snapshot_id", "element_index"]))
                && !desktop_scroll_any_of
                    .iter()
                    .any(|item| item["required"] == json!(["element_identifier"])),
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
            exact_branch(phone_setup_schema, "operation", "open_settings")["required"]
                == json!(["operation", "session_id", "screen"]),
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
            exact_branch(phone_pointer_schema, "operation", "tap")["required"]
                == json!(["operation", "session_id", "x", "y"])
                && exact_branch(phone_pointer_schema, "operation", "tap")["additionalProperties"]
                    == json!(false),
            "phone_pointer tap branch must require coordinates"
        );
        assert!(
            exact_branch(phone_pointer_schema, "operation", "swipe")["required"]
                == json!([
                    "operation",
                    "session_id",
                    "start_x",
                    "start_y",
                    "end_x",
                    "end_y"
                ])
                && exact_branch(phone_pointer_schema, "operation", "swipe")["additionalProperties"]
                    == json!(false),
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
            exact_branch(phone_keyboard_schema, "operation", "type_text")["required"]
                == json!(["operation", "session_id", "text"])
                && exact_branch(phone_keyboard_schema, "operation", "type_text")["additionalProperties"]
                    == json!(false),
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
            exact_branch(phone_notification_schema, "operation", "action")["required"]
                == json!(["operation", "session_id", "event_id", "action_id"])
                && exact_branch(phone_notification_schema, "operation", "action")["additionalProperties"]
                    == json!(false),
            "phone_notification_action action branch must require event_id and action_id"
        );

        let phone_install_schema = &tool("phone_app_install")["inputSchema"];
        assert_eq!(
            phone_install_schema["properties"]["apk_paths"]["minItems"], 1,
            "phone_app_install apk_paths must be non-empty"
        );
        assert_eq!(
            phone_install_schema["required"],
            json!(["session_id", "apk_paths"]),
            "phone_app_install must require session_id and apk_paths"
        );
        assert!(
            phone_install_schema["properties"].get("apk_path").is_none(),
            "phone_app_install apk_path alias must stay removed"
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
        let phone_app_action_schema = &tool("phone_app_action")["inputSchema"];
        assert!(
            !schema_accepts(
                phone_app_action_schema,
                &json!({"operation": "launch", "session_id": "phone-1", "package_name": ""})
            ),
            "phone_app_action launch must reject empty package names"
        );
        assert!(
            schema_accepts(
                phone_app_action_schema,
                &json!({"operation": "open_intent", "session_id": "phone-1", "intent_uri": "intent://example", "package_name": ""})
            ),
            "phone_app_action open_intent must allow blank package sentinels"
        );
        assert_eq!(
            phone_app_action_schema["properties"]["intent_uri"]["minLength"], 1,
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
    fn advertised_schemas_only_use_runtime_supported_keywords() {
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        for tool in registry.tools.as_array().expect("tools array") {
            let name = tool["name"].as_str().expect("tool name");
            assert_schema_keywords_supported(&tool["inputSchema"], name);
        }
    }

    #[test]
    fn runtime_schema_validator_fails_closed_for_unsupported_schema_surface() {
        assert!(!schema_accepts(
            &json!({"type": "url"}),
            &json!("https://example.test/")
        ));
        assert!(!schema_accepts(
            &json!({"type": {"name": "string"}}),
            &json!("hello")
        ));
        assert!(!schema_accepts(
            &json!({"type": "string", "format": "uri"}),
            &json!("https://example.test/")
        ));
        assert!(schema_accepts(
            &json!({"type": "string", "pattern": ".*\\S.*"}),
            &json!(" window ")
        ));
        assert!(!schema_accepts(
            &json!({"type": "string", "pattern": ".*\\S.*"}),
            &json!("   ")
        ));
        assert!(!schema_accepts(
            &json!({"type": "string", "pattern": "^custom$"}),
            &json!("custom")
        ));
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
    fn call_cases_match_grouped_dispatcher() {
        for case in generated_call_cases()["cases"]
            .as_array()
            .expect("call cases")
        {
            let tool_name = case["tool"].as_str().expect("case tool");
            let expected_branch = case["branch"].as_str().expect("case branch");
            let expected_handler = case["handler_id"].as_str().expect("case handler");
            let call = crate::mcp_tools::grouped_handler_call(tool_name, case["valid"].clone())
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

    fn assert_schema_keywords_supported(schema: &Value, context: &str) {
        let Some(schema) = schema.as_object() else {
            return;
        };
        for keyword in schema.keys() {
            assert!(
                schema_keyword_is_supported(keyword),
                "{context} inputSchema uses unsupported validation keyword {keyword}"
            );
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            assert!(
                schema_pattern_is_supported(pattern),
                "{context} inputSchema uses unsupported runtime pattern {pattern}"
            );
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, property_schema) in properties {
                assert_schema_keywords_supported(property_schema, &format!("{context}.{name}"));
            }
        }
        if let Some(items) = schema.get("items") {
            assert_schema_keywords_supported(items, &format!("{context}[]"));
        }
        for key in ["allOf", "anyOf", "oneOf"] {
            if let Some(children) = schema.get(key).and_then(Value::as_array) {
                for (index, child) in children.iter().enumerate() {
                    assert_schema_keywords_supported(child, &format!("{context}.{key}[{index}]"));
                }
            }
        }
        for key in ["not", "if", "then"] {
            if let Some(child) = schema.get(key) {
                assert_schema_keywords_supported(child, &format!("{context}.{key}"));
            }
        }
    }

    fn schema_pattern_is_supported(pattern: &str) -> bool {
        matches!(pattern, "^(https?://[^\\s]+|about:blank)$" | ".*\\S.*")
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
        let tools: Vec<Value> = grouped_contract_tools()
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
            "default_tool_count": 34,
            "eval_tool_count": 35,
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
                    branch("desktop", "get_app_state", json!({"surface": "desktop"})),
                    branch(
                        "browser",
                        "browser_snapshot",
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
}
