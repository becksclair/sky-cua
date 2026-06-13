use serde_json::{Value, json};

use crate::mcp_tools::annotations::{
    LOCAL_NAVIGATION_ACTION, OPEN_WORLD_DESTRUCTIVE_ACTION, READ_ONLY_TOOL, ToolAnnotations,
};

// `browser_eval` executes arbitrary page JavaScript in real signed-in user
// tabs — a stronger trust boundary than visible UI automation. The opt-in
// check is shared with the service execution boundary so the two cannot
// diverge; see `sky_cua_platform::model::browser_eval_enabled`.
#[cfg(test)]
pub(crate) use sky_cua_platform::model::BROWSER_EVAL_ENV;
pub(crate) use sky_cua_platform::model::{
    BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT, BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT,
    BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT, BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, browser_eval_enabled,
};

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
        "description": "Check whether Browser Use can control the user's Chrome-family browser, including bridge readiness and known-tab count.",
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
        "description": "Find existing user_chrome tabs. Use title_contains or url_contains to reduce returned tabs, then pass the chosen tab_id to browser_claim_tab before page actions.",
        "annotations": READ_ONLY_TOOL.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": ["user_chrome"],
                    "description": "Optional browser context. user_chrome is the user's Chrome/Chromium/Brave profile."
                },
                "url_contains": {
                    "type": "string",
                    "description": "Optional case-insensitive URL filter. Filters both text summary and structuredContent.tabs. Example: chamber.heliasar.com"
                },
                "title_contains": {
                    "type": "string",
                    "description": "Optional case-insensitive title filter. Filters both text summary and structuredContent.tabs. Example: OpenChamber"
                }
            },
            "additionalProperties": false
        }
    })
}

fn browser_open_tool() -> Value {
    json!({
        "name": "browser_open",
        "description": "Create a new controllable user_chrome tab, optionally at an HTTP(S) URL or about:blank. Use browser_claim_tab for existing tabs.",
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
                    "description": "Browser context. user_chrome creates a new tab in the user's Chrome/Chromium/Brave profile."
                },
                "url": {
                    "type": "string",
                    "description": "Optional initial URL. Allowed schemes: http://, https://, and about:blank."
                }
            },
            "additionalProperties": false
        }
    })
}

fn browser_claim_tab_tool() -> Value {
    json!({
        "name": "browser_claim_tab",
        "description": "Make an existing user_chrome tab from browser_list_tabs controllable. Required before snapshot, screenshot, click, type, key, scroll, navigate, or eval on listed tabs.",
        "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": ["user_chrome"],
                    "description": "Browser context. Only user_chrome is supported."
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
        "description": "Move the visible browser agent cursor without clicking. Use for hover or visual cursor placement; browser_click and targeted browser_scroll move it automatically.",
        "annotations": LOCAL_NAVIGATION_ACTION.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": ["user_chrome"],
                    "description": "Browser context. Only user_chrome is supported."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Browser tab id from browser_open or browser_claim_tab."
                },
                "x": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Target X coordinate in CSS pixels, matching browser_screenshot pixels and browser_snapshot element bounds."
                },
                "y": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Target Y coordinate in CSS pixels, matching browser_screenshot pixels and browser_snapshot element bounds."
                },
                "wait_for_arrival": {
                    "type": "boolean",
                    "description": "Wait until the extension cursor reaches the target point before returning. Defaults to true."
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
        "Navigate a controllable user_chrome tab to an HTTP(S) URL or about:blank.",
        ToolAnnotations {
            read_only: false,
            destructive: false,
            idempotent: true,
            open_world: true,
        },
        json!({
            "url": {
                "type": "string",
                "description": "Destination URL. Allowed schemes: http://, https://, and about:blank."
            }
        }),
        json!(["tab_id", "url"]),
    )
}

fn browser_snapshot_tool() -> Value {
    browser_tab_tool(
        "browser_snapshot",
        &format!(
            "Inspect a controllable user_chrome page as token-bounded structured state: title, URL, viewport, visible text, and actionable elements with CSS-pixel bounds. Default output returns up to {BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT} text chars and {BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT} elements; use text_limit, element_query, element_offset, and element_limit to tune."
        ),
        READ_ONLY_TOOL,
        json!({
            "element_offset": {
                "type": "integer",
                "minimum": 0,
                "description": "Optional zero-based offset into actionable elements after element_query filtering."
            },
            "element_limit": {
                "type": "integer",
                "minimum": 0,
                "maximum": BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
                "description": format!("Maximum actionable elements to return after filtering. Defaults to {BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT}; use 0 to omit elements while keeping snapshot metadata. snapshot.elementCount reports the full captured total.")
            },
            "element_query": {
                "type": "string",
                "description": "Optional case-insensitive filter over element tag, role, name, value, and href before offset/limit. Example: settings"
            },
            "text_limit": {
                "type": "integer",
                "minimum": 0,
                "maximum": BROWSER_SNAPSHOT_MAX_TEXT_LIMIT,
                "description": format!("Maximum visible-text characters to return. Defaults to {BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT}. Use 0 for controls-only snapshots or up to {BROWSER_SNAPSHOT_MAX_TEXT_LIMIT} for full-page text review.")
            }
        }),
        json!(["tab_id"]),
    )
}

fn browser_screenshot_tool() -> Value {
    browser_tab_tool(
        "browser_screenshot",
        "Capture the visible viewport of a controllable user_chrome tab. Screenshot pixels are CSS pixels matching browser_snapshot bounds and action coordinates. Image-capable sessions get an image block; text-only sessions get screenshot_path and metadata without inline image data.",
        READ_ONLY_TOOL,
        json!({}),
        json!(["tab_id"]),
    )
}

fn browser_click_tool() -> Value {
    browser_tab_tool(
        "browser_click",
        "Click a point in a controllable user_chrome tab. Coordinates are CSS pixels from browser_screenshot or browser_snapshot bounds; the visible browser agent cursor moves there first.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "x": { "type": "number", "minimum": 0, "description": "Target X coordinate in CSS pixels, matching browser_screenshot pixels and browser_snapshot element bounds." },
            "y": { "type": "number", "minimum": 0, "description": "Target Y coordinate in CSS pixels, matching browser_screenshot pixels and browser_snapshot element bounds." }
        }),
        json!(["tab_id", "x", "y"]),
    )
}

fn browser_type_text_tool() -> Value {
    browser_tab_tool(
        "browser_type_text",
        "Insert literal text into the currently focused page control in a controllable user_chrome tab. Focus the control first.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "text": { "type": "string", "description": "Literal text to insert. Spaces and newlines are preserved." }
        }),
        json!(["tab_id", "text"]),
    )
}

fn browser_press_key_tool() -> Value {
    browser_tab_tool(
        "browser_press_key",
        "Press a key or modifier chord in a controllable user_chrome tab. Use for page shortcuts and focused-control interactions.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "key": { "type": "string", "description": "CDP key name or modifier chord, such as Enter, Tab, Escape, ArrowDown, Ctrl+K, Ctrl+L, Shift+Tab, Ctrl+- (zoom out), or Ctrl++ (zoom in)." }
        }),
        json!(["tab_id", "key"]),
    )
}

fn browser_scroll_tool() -> Value {
    browser_tab_tool(
        "browser_scroll",
        "Scroll a controllable user_chrome tab. Omit x/y for viewport scroll. Provide x/y together to move the visible browser agent cursor there and scroll the nearest scrollable container. Positive delta_y scrolls down.",
        ToolAnnotations {
            read_only: false,
            destructive: false,
            idempotent: false,
            open_world: true,
        },
        json!({
            "delta_x": { "type": "number", "description": "Horizontal scroll delta in CSS pixels. Defaults to 0." },
            "delta_y": { "type": "number", "description": "Vertical scroll delta in CSS pixels. Defaults to 0; positive values scroll down." },
            "x": { "type": "number", "minimum": 0, "description": "Optional CSS-pixel X coordinate for container-targeted scroll. Provide together with y." },
            "y": { "type": "number", "minimum": 0, "description": "Optional CSS-pixel Y coordinate for container-targeted scroll. Provide together with x." }
        }),
        json!(["tab_id"]),
    )
}

fn browser_eval_tool() -> Value {
    browser_tab_tool(
        "browser_eval",
        "Evaluate JavaScript in a controllable user_chrome tab and return a serializable value. Diagnostic fallback only; hidden unless SKY_CUA_BROWSER_EVAL=on.",
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
            "description": "Browser context. Only user_chrome is supported."
        },
        "tab_id": {
            "type": "string",
            "description": "Browser tab id from browser_open or browser_claim_tab. Use browser_claim_tab first for ids from browser_list_tabs."
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
