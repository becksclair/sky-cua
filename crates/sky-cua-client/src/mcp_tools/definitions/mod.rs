//! MCP tool definitions: the host-facing tool registry with input schemas
//! and annotations. Split from `mcp_tools.rs` along the contract-family
//! boundary; dispatch and response shaping stay in the parent module.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::mcp_server::ModelSessionInfo;
use sky_cua_platform::{config::AgentSurfacePolicy, model::BROWSER_EVAL_ENV};

use super::annotations::{
    LOCAL_DESTRUCTIVE_ACTION, LOCAL_NAVIGATION_ACTION, LOCAL_STATEFUL_ACTION, READ_ONLY_TOOL,
    ToolAnnotations,
};

mod browser;
mod common;
#[cfg(test)]
mod contract_fixtures;
mod desktop;
#[cfg(test)]
mod fixture_tests;
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
    pub(crate) surfaces: AgentSurfacePolicy,
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
             `element_query`, `element_offset`, `element_limit`, and `capture_timeout_ms`. \
             Every successful \
             observe returns a canonical AppShot; image-capable hosts also receive \
             its image attachment.",
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
             top-level `operation`, the `appshot_id` returned by `observe`, plus \
             one target: `snapshot_id` with `x`/`y` \
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

pub(crate) fn mcp_process_config_from_env() -> Result<McpProcessConfig, String> {
    let mut diagnostics = Vec::new();
    // Enabled by default; only an explicit off/0/false disables it. An invalid
    // value is reported and falls back to the enabled default.
    let browser_eval_enabled = match std::env::var(BROWSER_EVAL_ENV) {
        Ok(value) => match parse_browser_eval_runtime(&value) {
            Some(value) => value,
            None => {
                diagnostics.push(McpConfigDiagnostic::InvalidBrowserEval { value });
                true
            }
        },
        Err(_) => true,
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

    Ok(McpProcessConfig {
        browser_eval_enabled,
        surfaces: sky_cua_platform::config::resolved_agent_surface_policy()?,
        model_supports_images_override,
        diagnostics,
    })
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
    let (tools, validation_schemas) = build_tool_definitions_split(
        can_receive_images,
        process.browser_eval_enabled,
        process.surfaces,
    );
    let active_names = tool_names(&tools);
    let mut inactive_names = BTreeSet::new();
    if process.surfaces.browser
        && !process.browser_eval_enabled
        && !active_names.contains("browser_eval")
    {
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
    build_tool_definitions_split(
        can_receive_images,
        browser_eval_enabled,
        AgentSurfacePolicy::default(),
    )
    .0
}

/// Tool definitions whose `inputSchema` is the rich validation schema rather
/// than the flattened advertised one. For tests that assert the per-branch
/// constraint shape (which moved out of the advertised surface).
#[cfg(test)]
pub(crate) fn validation_tool_definitions(
    can_receive_images: bool,
    browser_eval_enabled: bool,
) -> Value {
    let (mut tools, validation) = build_tool_definitions_split(
        can_receive_images,
        browser_eval_enabled,
        AgentSurfacePolicy::default(),
    );
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
    surfaces: AgentSurfacePolicy,
) -> (Value, BTreeMap<String, Value>) {
    let mut tools =
        build_grouped_tool_definitions(can_receive_images, browser_eval_enabled, surfaces);
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

fn build_grouped_tool_definitions(
    can_receive_images: bool,
    browser_eval_enabled: bool,
    surfaces: AgentSurfacePolicy,
) -> Value {
    let mut tools = json!([
        grouped_tool(
            "doctor",
            "Run sky-cua runtime readiness diagnostics for the current machine.",
            READ_ONLY_TOOL,
            json!({}),
            json!([])
        ),
        grouped_tool_with_constraints(
            "status",
            &status_description(surfaces),
            READ_ONLY_TOOL,
            status_properties(surfaces),
            json!(["component"]),
            status_constraints(surfaces)
        ),
        grouped_tool_with_constraints(
            "list_resources",
            &list_resources_description(surfaces),
            READ_ONLY_TOOL,
            list_resources_properties(surfaces),
            json!(["surface", "resource"]),
            list_resources_constraints(surfaces)
        ),
        grouped_tool_with_constraints(
            "observe",
            &observe_description(surfaces),
            READ_ONLY_TOOL,
            observe_properties(can_receive_images, surfaces),
            json!(["surface"]),
            observe_constraints(can_receive_images, surfaces)
        ),
        grouped_tool_with_constraints(
            "capture_screen",
            &capture_screen_description(surfaces),
            READ_ONLY_TOOL,
            capture_screen_properties(surfaces),
            json!(["surface"]),
            capture_screen_constraints(surfaces)
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
            json!(["operation", "appshot_id"]),
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
            json!(["tab_id", "x", "y", "appshot_id"])
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
            with_phone_selector(json!({"package_name": non_blank_string_schema()})),
            json!(["session_id", "package_name", "appshot_id"])
        ),
        grouped_tool_with_constraints(
            "desktop_toggle",
            "Toggle a desktop element from observe(surface=\"desktop\").",
            LOCAL_STATEFUL_ACTION,
            desktop_semantic_properties(json!({})),
            json!(["appshot_id"]),
            desktop_selector_constraint()
        ),
        grouped_tool_with_constraints(
            "desktop_scroll",
            "Scroll a snapshot-resolved desktop element. Pass direction and snapshot_id plus element_index, name, or text. Re-observe before reusing an element index.",
            LOCAL_STATEFUL_ACTION,
            desktop_semantic_properties(json!({
                "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                "pages": {"type": "integer", "minimum": 1, "description": "Page-sized scroll steps. Defaults to 1."}
            })),
            json!(["direction", "appshot_id"]),
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
            json!(["tab_id", "appshot_id"]),
            browser_scroll_constraints()
        ),
        grouped_tool_with_constraints(
            "desktop_pointer",
            "Click, secondary-click, or drag on the desktop. Preferred: pass the snapshot_id from the same capture_desktop/observe plus x/y read off that screenshot; those pixels are translated to the screen for you and work even when no semantic elements are exposed (e.g. Wayland apps with no matched AT-SPI tree). Or pass snapshot_id plus element_index/name/text for a semantic target. Bare x/y with no snapshot_id are raw screen coordinates and should be used only when you have no snapshot. Do not call with only operation.",
            LOCAL_DESTRUCTIVE_ACTION,
            desktop_pointer_properties(),
            json!(["operation", "appshot_id"]),
            desktop_pointer_constraints()
        ),
        grouped_tool_with_constraints(
            "desktop_keyboard",
            "Type text or press a key on the desktop. Focus first; text for type_text, key for press_key, e.g. Enter, Escape, Tab, Ctrl+A, Meta+A.",
            LOCAL_DESTRUCTIVE_ACTION,
            desktop_keyboard_properties(),
            json!(["operation", "appshot_id"]),
            desktop_keyboard_constraints()
        ),
        grouped_tool_with_constraints(
            "desktop_action",
            "Activate a desktop element or perform its named/indexed action from observe(surface=\"desktop\"); do not call with only operation.",
            LOCAL_DESTRUCTIVE_ACTION,
            desktop_action_properties(),
            json!(["operation", "appshot_id"]),
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
            json!(["value", "appshot_id"]),
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
            json!(["operation", "tab_id", "appshot_id"]),
            browser_input_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_pointer",
            "Tap or swipe on a connected phone. Use phone_snapshot_id for screenshot pixels or use_device_coordinates for raw pixels.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_pointer_properties(),
            json!(["operation", "appshot_id"]),
            phone_pointer_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_keyboard",
            "Type text or press a key on a connected phone. Focus first; press_key accepts KEYCODE_* names, aliases, or numeric keycodes.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_keyboard_properties(),
            json!(["operation", "appshot_id"]),
            phone_keyboard_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_notification_action",
            "Open, dismiss, or run an action on a connected-phone notification.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_notification_action_properties(),
            json!(["operation", "appshot_id"]),
            phone_notification_action_constraints()
        ),
        grouped_tool(
            "phone_notification_reply",
            "Reply inline to a connected-phone notification using event_id and inline-reply action_id from the same fresh event.",
            LOCAL_DESTRUCTIVE_ACTION,
            with_phone_selector(json!({
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
            json!(["session_id", "appshot_id", "event_id", "action_id", "text"])
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
            json!(["appshot_id"]),
            phone_app_install_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_content",
            "Describe, explicitly transfer, export, or release Companion content. Camera capture returns a phone-local ContentRef; export_host_file is the separate operation that transfers its bytes to the host.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_content_properties(),
            json!(["operation"]),
            phone_feature_constraints(&["describe"])
        ),
        grouped_tool_with_constraints(
            "phone_clipboard",
            "Read, replace, clear, or watch the Android clipboard.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_clipboard_properties(),
            json!(["operation"]),
            phone_feature_constraints(&["get", "changes"])
        ),
        grouped_tool_with_constraints(
            "phone_editor",
            "Inspect or operate the focused Android editor, including selection and rich insertion when supported.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_editor_properties(),
            json!(["operation"]),
            phone_feature_constraints(&["context"])
        ),
        grouped_tool_with_constraints(
            "phone_camera",
            "Enumerate and control Android cameras, capture bounded phone-local media, and request preview frames. V1 capture is at most 1920x1080 (or portrait 1080x1920), video auto-stops at 60 seconds, and capture never automatically transfers media to the host.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_camera_properties(),
            json!(["operation"]),
            phone_camera_constraints()
        ),
        grouped_tool_with_constraints(
            "phone_storage",
            "Browse and mutate permitted Android virtual storage roots.",
            LOCAL_DESTRUCTIVE_ACTION,
            phone_storage_properties(),
            json!(["operation"]),
            phone_feature_constraints(&[
                "roots",
                "list",
                "stat",
                "read",
                "hash",
                "search",
                "thumbnail",
                "metadata",
                "list_saf_roots",
            ])
        )
    ]);
    if surfaces.browser && browser_eval_enabled {
        tools.as_array_mut().expect("tool array").push(grouped_tool(
            "browser_eval",
            "Evaluate JavaScript in a claimed browser tab. This is hidden unless browser eval is explicitly enabled.",
            super::annotations::OPEN_WORLD_DESTRUCTIVE_ACTION,
            merge_properties(
                browser_tab_properties(),
                json!({"expression": non_empty_string_schema(), "appshot_id": non_blank_string_schema()})
            ),
            json!(["tab_id", "expression", "appshot_id"]),
        ));
    }
    tools.as_array_mut().expect("tool array").retain(|tool| {
        tool.get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| tool_enabled_for_surfaces(name, surfaces))
    });
    tools
}

fn tool_enabled_for_surfaces(name: &str, surfaces: AgentSurfacePolicy) -> bool {
    match name {
        "doctor" => true,
        "status" | "list_resources" | "observe" => {
            surfaces.desktop || surfaces.browser || surfaces.phone
        }
        "capture_screen" => surfaces.browser || surfaces.phone,
        "capture_desktop" | "setup_desktop" | "session_presence" | "activate_window" => {
            surfaces.desktop
        }
        name if name.starts_with("desktop_") => surfaces.desktop,
        name if name.starts_with("browser_") => surfaces.browser,
        name if name.starts_with("phone_") => surfaces.phone,
        other => panic!(
            "public MCP tool {other:?} has no surface classification; classify it before advertising it"
        ),
    }
}

fn enabled_surface_names(surfaces: AgentSurfacePolicy) -> Vec<&'static str> {
    let mut names = Vec::new();
    if surfaces.desktop {
        names.push("desktop");
    }
    if surfaces.browser {
        names.push("browser");
    }
    if surfaces.phone {
        names.push("phone");
    }
    names
}

fn status_description(surfaces: AgentSurfacePolicy) -> String {
    format!(
        "Report health for enabled sky-cua components on the {} surface set.",
        enabled_surface_names(surfaces).join(", ")
    )
}

fn list_resources_description(surfaces: AgentSurfacePolicy) -> String {
    let mut families = Vec::new();
    if surfaces.desktop {
        families.push("desktop apps/windows/focused_window");
    }
    if surfaces.browser {
        families.push("browser tabs");
    }
    if surfaces.phone {
        families.push("phone devices/apps/current_app");
    }
    format!(
        "List bounded resources. Valid enabled families: {}.",
        families.join("; ")
    )
}

fn observe_description(surfaces: AgentSurfacePolicy) -> String {
    let mut detail = Vec::new();
    if surfaces.desktop {
        detail.push("Desktop supports exact window selectors and bounded semantic projection.");
    }
    if surfaces.browser {
        detail.push(
            "Browser requires tab_id and binds pixels and semantics to one document generation.",
        );
    }
    if surfaces.phone {
        detail.push("Phone requires session_id and can include accessibility and notifications.");
    }
    format!(
        "Capture one canonical AppShot on an enabled surface. The result binds surface identity, semantic state, action snapshot identity, consistency, and diagnostics; image-capable hosts also receive the same AppShot image as an attachment. {}",
        detail.join(" ")
    )
}

fn capture_screen_description(surfaces: AgentSurfacePolicy) -> String {
    match (surfaces.browser, surfaces.phone) {
        (true, true) => "Capture a browser-tab or phone image only. Browser requires tab_id. Use capture_desktop for desktop screenshots.".to_string(),
        (true, false) => "Capture a browser-tab image only. Browser requires tab_id. Use capture_desktop for desktop screenshots when the desktop surface is enabled.".to_string(),
        (false, true) => "Capture a phone image only. Phone requires session_id.".to_string(),
        (false, false) => String::new(),
    }
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
