use serde_json::{Value, json};

use crate::mcp_tools::annotations::{
    LOCAL_NAVIGATION_ACTION, LOCAL_STATEFUL_ACTION, OPEN_WORLD_DESTRUCTIVE_ACTION, READ_ONLY_TOOL,
    ToolAnnotations,
};

// `browser_eval` executes arbitrary page JavaScript in real signed-in user
// tabs — a stronger trust boundary than visible UI automation. The opt-in
// check is shared with the service execution boundary so the two cannot
// diverge; see `sky_cua_platform::model::browser_eval_enabled`.
#[cfg(test)]
pub(crate) use sky_cua_platform::model::BROWSER_EVAL_ENV;
pub(crate) use sky_cua_platform::model::browser_eval_enabled;

pub(crate) fn push_tool_definitions(tool_array: &mut Vec<Value>, eval_enabled: bool) {
    tool_array.push(browser_status_tool());
    tool_array.push(browser_list_tabs_tool());
    tool_array.push(browser_open_tool());
    tool_array.push(browser_claim_tab_tool());
    tool_array.push(browser_move_mouse_tool());
    tool_array.push(browser_navigate_tool());
    tool_array.push(browser_snapshot_tool());
    tool_array.push(browser_screenshot_tool());
    tool_array.push(browser_click_tool());
    tool_array.push(browser_type_text_tool());
    tool_array.push(browser_press_key_tool());
    tool_array.push(browser_scroll_tool());
    if eval_enabled {
        tool_array.push(browser_eval_tool());
    }
}

fn browser_status_tool() -> Value {
    json!({
        "name": "browser_status",
        "description": "Report first-class Browser Use readiness for sky-cua, including user-Chrome availability, planned managed-browser lifecycle status, native host manifest state, and browser bridge diagnostics.",
        "annotations": READ_ONLY_TOOL.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }
    })
}

fn browser_list_tabs_tool() -> Value {
    json!({
        "name": "browser_list_tabs",
        "description": "List browser tabs known to sky-cua Browser Use for the user's Chrome-family browser. Use url_contains or title_contains when many tabs are open so the text response includes actionable tab ids. Returns honest diagnostics when no browser bridge is connected yet.",
        "annotations": READ_ONLY_TOOL.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": ["user_chrome"],
                    "description": "Optional browser context to inspect. user_chrome is the user's Chrome/Chromium/Brave profile via the Chrome extension/native host."
                },
                "url_contains": {
                    "type": "string",
                    "description": "Optional case-insensitive filter applied to tab URLs for both the text summary and structuredContent.tabs. Use this when many tabs are open. Example: chamber.heliasar.com"
                },
                "title_contains": {
                    "type": "string",
                    "description": "Optional case-insensitive filter applied to tab titles for both the text summary and structuredContent.tabs. Use this when many tabs are open. Example: OpenChamber"
                }
            },
            "additionalProperties": false
        }
    })
}

fn browser_open_tool() -> Value {
    json!({
        "name": "browser_open",
        "description": "Create a session-owned browser tab through the Chrome-family native-host bridge, optionally navigating to an HTTP(S) URL or about:blank. Existing user tabs are listed separately and are not adopted by this tool.",
        "annotations": ToolAnnotations {
            read_only: false,
            destructive: false,
            idempotent: false,
            open_world: true,
        }
        .to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": ["user_chrome"],
                    "description": "Browser context to use. Currently user_chrome creates a new session-owned tab in the user's Chrome/Chromium/Brave profile."
                },
                "url": {
                    "type": "string",
                    "description": "Optional URL to navigate the new tab to. Allowed schemes: http://, https://, and about:blank."
                }
            },
            "additionalProperties": false
        }
    })
}

fn browser_claim_tab_tool() -> Value {
    json!({
        "name": "browser_claim_tab",
        "description": "Adopt an existing user_chrome browser tab into sky-cua's browser session so browser actions can target it. Use tab_id from browser_list_tabs.",
        "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": ["user_chrome"],
                    "description": "Browser context to use. Currently only user_chrome is supported."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Browser tab id from browser_list_tabs."
                }
            },
            "required": ["tab_id"],
            "additionalProperties": false
        }
    })
}

fn browser_move_mouse_tool() -> Value {
    json!({
        "name": "browser_move_mouse",
        "description": "Move the webpage/browser cursor in a claimed or session-owned user_chrome tab. Coordinates are CSS pixels, the same space as browser_screenshot image pixels and browser_snapshot element bounds. Use browser_claim_tab for existing user tabs, or browser_open for session-owned tabs.",
        "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": ["user_chrome"],
                    "description": "Browser context to use. Currently only user_chrome is supported."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Browser tab id from browser_list_tabs or browser_open."
                },
                "x": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Target X coordinate in CSS pixels, matching browser_screenshot image pixels and browser_snapshot element bounds."
                },
                "y": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Target Y coordinate in CSS pixels, matching browser_screenshot image pixels and browser_snapshot element bounds."
                },
                "wait_for_arrival": {
                    "type": "boolean",
                    "description": "Wait for the extension cursor animation to arrive before returning. Defaults to true."
                }
            },
            "required": ["tab_id", "x", "y"],
            "additionalProperties": false
        }
    })
}

fn browser_navigate_tool() -> Value {
    browser_tab_tool(
        "browser_navigate",
        "Navigate a claimed or session-owned user_chrome tab to an HTTP(S) URL or about:blank.",
        ToolAnnotations {
            read_only: false,
            destructive: false,
            idempotent: true,
            open_world: true,
        },
        json!({
            "url": {
                "type": "string",
                "description": "URL to navigate to. Allowed schemes: http://, https://, and about:blank."
            }
        }),
        json!(["tab_id", "url"]),
    )
}

fn browser_snapshot_tool() -> Value {
    browser_tab_tool(
        "browser_snapshot",
        "Return a structured browser page snapshot for a claimed or session-owned user_chrome tab, including title, URL, visible text, viewport, and common element summaries. Element bounds are CSS pixels, the same space as browser_screenshot image pixels and browser_click coordinates. Use element_query or element_offset/element_limit when many controls are present.",
        READ_ONLY_TOOL,
        json!({
            "element_offset": {
                "type": "integer",
                "minimum": 0,
                "description": "Optional zero-based offset into actionable elements returned in structuredContent and shown in the text summary."
            },
            "element_limit": {
                "type": "integer",
                "minimum": 1,
                "description": "Optional maximum number of actionable elements returned in structuredContent and shown in the text summary. Defaults to 200; snapshot.elementCount always reports the full total."
            },
            "element_query": {
                "type": "string",
                "description": "Optional case-insensitive filter over element tag, role, name, value, and href. Example: update"
            }
        }),
        json!(["tab_id"]),
    )
}

fn browser_screenshot_tool() -> Value {
    browser_tab_tool(
        "browser_screenshot",
        "Capture a screenshot of the visible viewport in a claimed or session-owned user_chrome tab. The image is attached to the result when the session's model supports image input, and saved to a file whose path is returned in structuredContent.screenshot_path. Image pixels are CSS pixels, the same space as browser_snapshot element bounds and browser_click coordinates.",
        READ_ONLY_TOOL,
        json!({}),
        json!(["tab_id"]),
    )
}

fn browser_click_tool() -> Value {
    browser_tab_tool(
        "browser_click",
        "Click within a claimed or session-owned user_chrome tab. Coordinates are CSS pixels, the same space as browser_screenshot image pixels and browser_snapshot element bounds.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "x": { "type": "number", "minimum": 0, "description": "Target X coordinate in CSS pixels, matching browser_screenshot image pixels and browser_snapshot element bounds." },
            "y": { "type": "number", "minimum": 0, "description": "Target Y coordinate in CSS pixels, matching browser_screenshot image pixels and browser_snapshot element bounds." }
        }),
        json!(["tab_id", "x", "y"]),
    )
}

fn browser_type_text_tool() -> Value {
    browser_tab_tool(
        "browser_type_text",
        "Type literal text into the focused element in a claimed or session-owned user_chrome tab.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "text": { "type": "string", "description": "Text to insert into the focused webpage control." }
        }),
        json!(["tab_id", "text"]),
    )
}

fn browser_press_key_tool() -> Value {
    browser_tab_tool(
        "browser_press_key",
        "Press a keyboard key in a claimed or session-owned user_chrome tab using CDP Input.dispatchKeyEvent key names. Modifier chords such as Ctrl+K, Ctrl+L, Shift+Tab, Meta+K, and hyphen targets like Ctrl+- are accepted.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "key": { "type": "string", "description": "CDP key name or modifier chord, such as Enter, Tab, Escape, ArrowDown, Ctrl+K, Ctrl+L, Shift+Tab, or Ctrl+- (zoom out)." }
        }),
        json!(["tab_id", "key"]),
    )
}

fn browser_scroll_tool() -> Value {
    browser_tab_tool(
        "browser_scroll",
        "Scroll within a claimed or session-owned user_chrome tab. When x/y are provided, sky-cua scrolls the nearest scrollable DOM container under that point; otherwise it falls back to the page viewport. Positive delta_y scrolls down.",
        LOCAL_STATEFUL_ACTION,
        json!({
            "delta_x": { "type": "number", "description": "Horizontal scroll delta in CSS pixels. Defaults to 0." },
            "delta_y": { "type": "number", "description": "Vertical scroll delta in CSS pixels. Positive values scroll down." },
            "x": { "type": "number", "minimum": 0, "description": "Wheel event X context coordinate in CSS pixels, matching browser_screenshot image pixels. Defaults to 0." },
            "y": { "type": "number", "minimum": 0, "description": "Wheel event Y context coordinate in CSS pixels, matching browser_screenshot image pixels. Defaults to 0." }
        }),
        json!(["tab_id"]),
    )
}

fn browser_eval_tool() -> Value {
    browser_tab_tool(
        "browser_eval",
        "Evaluate JavaScript in a claimed or session-owned user_chrome tab through CDP Runtime.evaluate and return the result by value. Use for diagnostics or controlled page-level fallbacks when visible UI automation is blocked. Available only when the operator sets SKY_CUA_BROWSER_EVAL=on.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "expression": {
                "type": "string",
                "description": "JavaScript expression to evaluate in the page. Promises are awaited; serializable results are returned by value."
            }
        }),
        json!(["tab_id", "expression"]),
    )
}

fn browser_tab_tool(
    name: &str,
    description: &str,
    annotations: ToolAnnotations,
    extra_properties: Value,
    required: Value,
) -> Value {
    let mut properties = json!({
        "target": {
            "type": "string",
            "enum": ["user_chrome"],
            "description": "Browser context to use. Currently only user_chrome is supported."
        },
        "tab_id": {
            "type": "string",
            "description": "Browser tab id from browser_list_tabs, browser_open, or browser_claim_tab."
        }
    });
    if let (Some(properties), Some(extra)) =
        (properties.as_object_mut(), extra_properties.as_object())
    {
        properties.extend(extra.clone());
    }
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
