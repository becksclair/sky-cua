//! MCP tool definitions: the host-facing tool registry with input schemas
//! and annotations. Split from `mcp_tools.rs` along the contract-family
//! boundary; dispatch and response shaping stay in the parent module.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::mcp_server::ModelSessionInfo;
use sky_cua_platform::model::BROWSER_EVAL_ENV;

use super::annotations::{
    LOCAL_DESTRUCTIVE_ACTION, LOCAL_NAVIGATION_ACTION, LOCAL_STATEFUL_ACTION, READ_ONLY_TOOL,
    ToolAnnotations,
};

mod browser;
mod common;
mod desktop;
mod phone;
mod status;

use browser::*;
pub(crate) use common::schema_accepts;
use common::*;
use desktop::*;
use phone::*;
use status::*;

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
    /// Advertised tool definitions sent to the host (`tools/list`). Their input
    /// schemas are flattened: no top-level `allOf`/`oneOf`/`anyOf`/`not`, which
    /// the Anthropic Messages API rejects (and Claude Code then drops the tool).
    tools: Value,
    /// Rich per-tool input schemas carrying the discriminated-union/exact-branch
    /// constraints. These are NOT advertised; they back `validate_arguments` so
    /// vague or cross-branch calls are still rejected at dispatch time.
    validation_schemas: BTreeMap<String, Value>,
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
        let Some(schema) = self.validation_schemas.get(name) else {
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
    let mut message = format!("arguments do not match the input schema for {name}");
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
             `tabs` accepts only `target`, `url_contains`, `title_contains`, and \
             `limit` (limit caps the returned tab list); desktop resources do not \
             accept phone or browser fields.",
        ),
        "observe" => Some(
            "`observe` expects one surface branch: desktop uses desktop observation \
             fields, browser requires top-level `tab_id`, and phone requires \
             top-level `session_id`; do not mix fields from another surface. The \
             browser branch accepts only `surface`, `tab_id`, `target`, `text_limit`, \
             `element_query`, `element_offset`, and `element_limit`; \
             `capture_screen`/`screenshot_delivery` are desktop-only, so use the \
             `capture_screen` tool for a browser-tab or phone image.",
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
             top-level `operation` plus one target: `snapshot_id` with `x`/`y` \
             read off that capture's screenshot (pixels are translated to the \
             screen for you — the most reliable option, and the one to use when \
             no semantic elements are exposed), or `snapshot_id` with \
             `element_index`, `name`, or `text` (semantic), or bare `x`/`y` with \
             no `snapshot_id` (raw screen coordinates — only when you have no \
             snapshot). Drag uses top-level `x`/`y`/`to_x`/`to_y` or \
             `from_x`/`from_y`/`to_x`/`to_y`, plus an optional `duration_ms` \
             that paces the interpolated drag motion.",
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
    let (tools, validation_schemas) =
        build_tool_definitions_split(can_receive_images, process.browser_eval_enabled);
    let active_names = tool_names(&tools);
    let mut inactive_names = BTreeSet::new();
    if !process.browser_eval_enabled && !active_names.contains("browser_eval") {
        inactive_names.insert("browser_eval".to_string());
    }

    McpToolRegistry {
        browser_eval_enabled: process.browser_eval_enabled,
        tools,
        validation_schemas,
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
    build_tool_definitions(
        model.can_receive_images(),
        super::browser::browser_eval_enabled(),
    )
}

#[cfg(test)]
pub(crate) fn tools_list_result(model: &ModelSessionInfo) -> Value {
    json!({
        "tools": tool_definitions(model)
    })
}

#[cfg(test)]
pub(crate) fn build_tool_definitions(
    can_receive_images: bool,
    browser_eval_enabled: bool,
) -> Value {
    build_tool_definitions_split(can_receive_images, browser_eval_enabled).0
}

/// Tool definitions whose `inputSchema` is the rich validation schema rather
/// than the flattened advertised one. For tests that assert the per-branch
/// constraint shape (which moved out of the advertised surface).
#[cfg(test)]
pub(crate) fn validation_tool_definitions(
    can_receive_images: bool,
    browser_eval_enabled: bool,
) -> Value {
    let (mut tools, validation) =
        build_tool_definitions_split(can_receive_images, browser_eval_enabled);
    for tool in tools
        .as_array_mut()
        .expect("tool definitions must be an array")
    {
        let name = tool["name"].as_str().expect("tool name").to_string();
        if let Some(rich) = validation.get(&name) {
            tool["inputSchema"] = rich.clone();
        }
    }
    tools
}

/// Build the advertised tool list (flattened input schemas) alongside the rich
/// per-tool validation schemas.
///
/// The grouped builders produce schemas whose root carries the exact-branch
/// discriminated union (`allOf`/`oneOf`), plus `capture_desktop`'s root `not`.
/// The Anthropic Messages API rejects top-level `allOf`/`oneOf`/`anyOf` in a
/// tool `input_schema` ("input_schema does not support oneOf, allOf, or anyOf at
/// the top level"), and under Claude Code's deferred loading the tool silently
/// never surfaces. We therefore advertise a flattened schema (the root object's
/// `properties`/`required` already enumerate every field) and keep the rich
/// schema only for runtime argument validation, so the per-branch guardrails
/// (e.g. "a click needs coordinates or a selector") still reject vague calls.
fn build_tool_definitions_split(
    can_receive_images: bool,
    browser_eval_enabled: bool,
) -> (Value, BTreeMap<String, Value>) {
    let mut tools = build_grouped_tool_definitions(can_receive_images, browser_eval_enabled);
    let mut validation_schemas = BTreeMap::new();
    for tool in tools
        .as_array_mut()
        .expect("tool definitions must be an array")
    {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .expect("tool must have a name")
            .to_string();
        if let Some(schema) = tool.get("inputSchema").cloned() {
            validation_schemas.insert(name, schema);
        }
        if let Some(schema) = tool.get_mut("inputSchema").and_then(Value::as_object_mut) {
            flatten_advertised_input_schema(schema);
        }
    }
    (tools, validation_schemas)
}

/// Strip the root-level schema combinators the Anthropic Messages API rejects.
/// The root `properties`/`required` already describe the full field set, so the
/// remaining flat schema stays a valid, self-describing object schema.
fn flatten_advertised_input_schema(schema: &mut Map<String, Value>) {
    for key in ["allOf", "oneOf", "anyOf", "not"] {
        schema.remove(key);
    }
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
            "List bounded resources. Valid pairs: desktop apps/windows/focused_window; browser tabs; phone devices/apps/current_app. Desktop windows include each window's display (its monitor: display_id, name, and primary flag); read it before capturing a specific app so you target the right monitor.",
            READ_ONLY_TOOL,
            list_resources_properties(),
            json!(["surface", "resource"]),
            list_resources_constraints()
        ),
        grouped_tool_with_constraints(
            "observe",
            "Read structured state for one surface. Desktop returns elements and snapshot_id; detail=\"compact\" controls desktop observation verbosity only. Browser requires tab_id and returns page text/elements. Phone requires session_id and can include accessibility/notifications. observe never returns an image: capture_screen/screenshot_delivery apply to the desktop surface only. For a browser tab image call the capture_screen tool (surface=\"browser\", tab_id); for a phone image call capture_screen (surface=\"phone\", session_id) or capture_desktop.",
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
            "Capture a fresh desktop frame and return a snapshot_id for pixel actions. Captures exactly one screen, never the whole multi-monitor desktop. With no selector it captures the main display. To drive or verify a specific application, target it instead of relying on the default: pass one window selector (window_id/pid/app_id/wm_class/title) to activate and crop to that window, or one display selector (display_id/display_name/display_index) for a specific monitor. An application on a secondary monitor will not appear in the default main-display capture.",
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
            "Click, secondary-click, or drag on the desktop. Preferred: pass the snapshot_id from the same capture_desktop/observe plus x/y read off that screenshot; those pixels are translated to the screen for you and work even when no semantic elements are exposed (e.g. Wayland apps with no matched AT-SPI tree). Or pass snapshot_id plus element_index/name/text for a semantic target. Bare x/y with no snapshot_id are raw screen coordinates and should be used only when you have no snapshot. Do not call with only operation.",
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
        grouped_tool(
            "desktop_launch_app",
            "Launch an application into the agent's private isolated desktop (e.g. a browser, file manager, or editor). Only available in isolated mode; it never launches onto the user's live session. Pass the executable as command and any arguments as args.",
            LOCAL_DESTRUCTIVE_ACTION,
            json!({
                "command": non_blank_string_schema(),
                "args": {
                    "type": "array",
                    "items": non_blank_string_schema(),
                    "description": "Command-line arguments passed to the application."
                }
            }),
            json!(["command"])
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
        ("desktop_launch_app", (false, true, false, false)),
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
        assert_eq!(registry.active_names.len(), 35);
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
        assert_eq!(registry.active_names.len(), 36);
        assert!(registry.contains("browser_eval"));
        assert_eq!(registry.inactive_reason("browser_eval"), None);
    }

    #[test]
    fn grouped_action_tool_schemas_reject_vague_desktop_actions() {
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        // The advertised schemas are flattened (no root composition); this test
        // asserts the *constraint* shape, which now lives in the validation
        // schemas. Synthesize per-tool objects that carry the advertised metadata
        // (name/description/annotations) with the rich validation `inputSchema`.
        let merged_tools: Vec<Value> = registry
            .tools
            .as_array()
            .expect("tools")
            .iter()
            .map(|tool| {
                let mut tool = tool.clone();
                let name = tool["name"].as_str().expect("tool name").to_string();
                if let Some(rich) = registry.validation_schemas.get(&name) {
                    tool["inputSchema"] = rich.clone();
                }
                tool
            })
            .collect();
        let tool = |name: &str| -> &Value {
            merged_tools
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
                .contains("Do not call with only operation")
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

        for schema in merged_tools.iter().map(|tool| &tool["inputSchema"]) {
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
            include_str!("../../../tests/fixtures/mcp_tool_surface_matrix.json"),
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
    fn advertised_schema_is_validation_schema_minus_root_composition() {
        // The advertised schema must differ from the rich validation schema by
        // exactly the four root composition keywords the Anthropic API rejects —
        // nothing else. This guards against a future change to
        // `flatten_advertised_input_schema` silently dropping a property or
        // `required` entry that runtime enforcement still depends on, which would
        // desync what the model sees from what `validate_arguments` checks.
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        for tool in registry.tools.as_array().expect("tools") {
            let name = tool["name"].as_str().expect("tool name");
            let advertised = tool["inputSchema"]
                .as_object()
                .unwrap_or_else(|| panic!("{name} advertised inputSchema must be an object"));
            let mut expected = registry
                .validation_schemas
                .get(name)
                .unwrap_or_else(|| panic!("{name} missing validation schema"))
                .as_object()
                .expect("validation schema must be an object")
                .clone();
            for key in ["allOf", "oneOf", "anyOf", "not"] {
                expected.remove(key);
            }
            assert_eq!(
                advertised, &expected,
                "advertised schema for {name} must equal its validation schema minus root \
                 composition keywords; flattening dropped or changed something else"
            );
        }
    }

    #[test]
    fn advertised_schemas_have_no_top_level_composition() {
        // The Anthropic Messages API rejects top-level `allOf`/`oneOf`/`anyOf` in
        // a tool `input_schema` ("input_schema does not support oneOf, allOf, or
        // anyOf at the top level"), and under Claude Code's deferred loading the
        // tool then silently never surfaces. `not` is undocumented and treated as
        // unsafe. Validation richness lives in `validation_schemas`, not here.
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        for tool in registry.tools.as_array().expect("tools") {
            let name = tool["name"].as_str().expect("tool name");
            let schema = tool["inputSchema"]
                .as_object()
                .unwrap_or_else(|| panic!("{name} inputSchema must be an object"));
            for key in ["allOf", "oneOf", "anyOf", "not"] {
                assert!(
                    !schema.contains_key(key),
                    "advertised inputSchema for {name} carries top-level {key}; the Anthropic \
                     Messages API rejects it and Claude Code drops the tool. Keep the constraint \
                     in the validation schema instead."
                );
            }
        }
    }

    #[test]
    fn validation_schemas_still_reject_vague_grouped_calls() {
        // Flattening the advertised schema must not weaken runtime enforcement:
        // `validate_arguments` uses the rich validation schema, so a bare grouped
        // call with only a discriminator is still rejected.
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        assert!(
            registry
                .validate_arguments("desktop_pointer", &json!({"operation": "click"}))
                .is_err(),
            "a click with no coordinates or selector must be rejected at dispatch"
        );
        assert!(
            registry
                .validate_arguments(
                    "desktop_pointer",
                    &json!({"operation": "click", "x": 10, "y": 20})
                )
                .is_ok(),
            "a click with coordinates must be accepted"
        );
        assert!(
            registry
                .validate_arguments(
                    "capture_desktop",
                    &json!({"window_id": "w1", "display_index": 0})
                )
                .is_err(),
            "capture_desktop must still reject mixing a window target with a display target"
        );
    }

    #[test]
    fn list_resources_browser_tabs_accepts_limit() {
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        assert!(
            registry
                .validate_arguments(
                    "list_resources",
                    &json!({
                        "surface": "browser",
                        "resource": "tabs",
                        "target": "user_chrome",
                        "limit": 20
                    })
                )
                .is_ok(),
            "browser tabs listing must accept a top-level limit"
        );
    }

    #[test]
    fn observe_browser_branch_rejects_capture_fields() {
        // Regression guard: capture_screen/screenshot_delivery are desktop-only.
        // observe returns structure; a browser-tab image comes from the separate
        // capture_screen tool, so the browser branch must keep rejecting them.
        let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
        assert!(
            registry
                .validate_arguments("observe", &json!({"surface": "browser", "tab_id": "tab-1"}))
                .is_ok(),
            "a plain browser observe must be accepted"
        );
        assert!(
            registry
                .validate_arguments(
                    "observe",
                    &json!({
                        "surface": "browser",
                        "tab_id": "tab-1",
                        "capture_screen": "if_changed"
                    })
                )
                .is_err(),
            "the browser observe branch must reject capture_screen"
        );
        assert!(
            registry
                .validate_arguments(
                    "observe",
                    &json!({
                        "surface": "browser",
                        "tab_id": "tab-1",
                        "screenshot_delivery": "path"
                    })
                )
                .is_err(),
            "the browser observe branch must reject screenshot_delivery"
        );
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
            include_str!("../../../tests/fixtures/tool_contract.json"),
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
            // Validate against the rich validation schema (what `validate_arguments`
            // enforces), not the flattened advertised schema, which by design no
            // longer carries the per-branch constraints that reject bad calls.
            let schema = registry
                .validation_schemas
                .get(tool_name)
                .unwrap_or_else(|| panic!("missing schema for {tool_name}"));
            assert!(
                schema_accepts(schema, &case["valid"]),
                "call case {}/{} is not valid for its generated schema: {}",
                tool_name,
                case["branch"].as_str().expect("case branch"),
                case["valid"]
            );
            assert!(
                !schema_accepts(schema, &case["invalid"]),
                "call case {}/{} invalid sample was accepted by its generated schema: {}",
                tool_name,
                case["branch"].as_str().expect("case branch"),
                case["invalid"]
            );
        }

        assert_fixture_matches(
            "call_cases.json",
            include_str!("../../../tests/fixtures/call_cases.json"),
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
            "default_tool_count": 35,
            "eval_tool_count": 36,
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
        branch_with_extra_errors(name, handler_id, minimal_valid_arguments, &[])
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
