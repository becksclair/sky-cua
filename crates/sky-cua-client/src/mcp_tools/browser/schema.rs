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
        "description": "Report Browser Use readiness for the user's Chrome-family browser: bridge status, target availability, and known-tab count.",
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
        "description": "List existing user_chrome tabs. Use title_contains/url_contains to narrow results; pass the chosen tab_id to browser_claim_tab before page actions.",
        "annotations": READ_ONLY_TOOL.to_value(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": ["user_chrome"],
                    "description": "Browser context. user_chrome is the user's Chrome/Chromium/Brave profile."
                },
                "url_contains": {
                    "type": "string",
                    "description": "Case-insensitive URL filter for text and structuredContent.tabs."
                },
                "title_contains": {
                    "type": "string",
                    "description": "Case-insensitive title filter for text and structuredContent.tabs."
                }
            },
            "additionalProperties": false
        }
    })
}

fn browser_open_tool() -> Value {
    json!({
        "name": "browser_open",
        "description": "Open a new controllable user_chrome tab, optionally at http(s) or about:blank. Use browser_claim_tab for existing tabs.",
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
                    "description": "Browser context. user_chrome opens in the user's Chrome/Chromium/Brave profile."
                },
                "url": {
                    "type": "string",
                    "description": "Initial URL. Allowed schemes: http://, https://, about:blank."
                }
            },
            "additionalProperties": false
        }
    })
}

fn browser_claim_tab_tool() -> Value {
    json!({
        "name": "browser_claim_tab",
        "description": "Make a browser_list_tabs tab controllable. Required before snapshot, screenshot, click, type, key, scroll, navigate, or eval on listed tabs.",
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
                    "description": "tab_id from browser_list_tabs."
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
        "description": "Move the visible browser agent cursor without clicking. Use for hover/placement; browser_click and targeted browser_scroll move it automatically.",
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
                    "description": "tab_id from browser_open or browser_claim_tab."
                },
                "x": {
                    "type": "number",
                    "minimum": 0,
                    "description": "X in CSS pixels; matches browser_screenshot and browser_snapshot bounds."
                },
                "y": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Y in CSS pixels; matches browser_screenshot and browser_snapshot bounds."
                },
                "wait_for_arrival": {
                    "type": "boolean",
                    "description": "Wait for cursor arrival before returning. Defaults to true."
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
        "Navigate a controllable user_chrome tab to http(s) or about:blank.",
        ToolAnnotations {
            read_only: false,
            destructive: false,
            idempotent: true,
            open_world: true,
        },
        json!({
            "url": {
                "type": "string",
                "description": "Destination URL. Allowed schemes: http://, https://, about:blank."
            }
        }),
        json!(["tab_id", "url"]),
    )
}

fn browser_snapshot_tool() -> Value {
    browser_tab_tool(
        "browser_snapshot",
        &format!(
            "Inspect a controllable user_chrome page as token-bounded structured state: title, URL, viewport, visible text, and actionable elements with CSS-pixel bounds. Defaults: {BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT} text chars, {BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT} elements. Tune with text_limit, element_query, element_offset, element_limit."
        ),
        READ_ONLY_TOOL,
        json!({
            "element_offset": {
                "type": "integer",
                "minimum": 0,
                "description": "Zero-based offset after element_query filtering."
            },
            "element_limit": {
                "type": "integer",
                "minimum": 0,
                "maximum": BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
                "description": format!("Maximum elements returned after filtering. Defaults to {BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT}; 0 keeps metadata only. snapshot.elementCount is the full total.")
            },
            "element_query": {
                "type": "string",
                "description": "Case-insensitive filter over element tag/role/name/value/href."
            },
            "text_limit": {
                "type": "integer",
                "minimum": 0,
                "maximum": BROWSER_SNAPSHOT_MAX_TEXT_LIMIT,
                "description": format!("Maximum visible-text chars. Defaults to {BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT}; 0 for controls-only, max {BROWSER_SNAPSHOT_MAX_TEXT_LIMIT}.")
            }
        }),
        json!(["tab_id"]),
    )
}

fn browser_screenshot_tool() -> Value {
    browser_tab_tool(
        "browser_screenshot",
        "Capture the visible viewport. Screenshot pixels are CSS pixels matching browser_snapshot bounds and actions. Image-capable sessions get an image block; text-only sessions get screenshot_path and metadata.",
        READ_ONLY_TOOL,
        json!({}),
        json!(["tab_id"]),
    )
}

fn browser_click_tool() -> Value {
    browser_tab_tool(
        "browser_click",
        "Click a CSS-pixel point from browser_screenshot/browser_snapshot; the visible browser agent cursor moves there first.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "x": { "type": "number", "minimum": 0, "description": "X in CSS pixels; matches browser_screenshot and browser_snapshot bounds." },
            "y": { "type": "number", "minimum": 0, "description": "Y in CSS pixels; matches browser_screenshot and browser_snapshot bounds." }
        }),
        json!(["tab_id", "x", "y"]),
    )
}

fn browser_type_text_tool() -> Value {
    browser_tab_tool(
        "browser_type_text",
        "Insert literal text into the focused page control. Focus the control first.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "text": { "type": "string", "description": "Literal text; spaces and newlines are preserved." }
        }),
        json!(["tab_id", "text"]),
    )
}

fn browser_press_key_tool() -> Value {
    browser_tab_tool(
        "browser_press_key",
        "Press a key or modifier chord for page shortcuts or the focused control.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "key": { "type": "string", "description": "CDP key or chord, e.g. Enter, Tab, Escape, ArrowDown, Ctrl+K, Ctrl+L, Shift+Tab, Ctrl+-, Ctrl++." }
        }),
        json!(["tab_id", "key"]),
    )
}

fn browser_scroll_tool() -> Value {
    browser_tab_tool(
        "browser_scroll",
        "Scroll a controllable user_chrome tab with non-zero delta_x or delta_y. Omit x/y for viewport scroll. Provide x/y together to move the visible browser agent cursor there and scroll the nearest container. Positive delta_y scrolls down.",
        ToolAnnotations {
            read_only: false,
            destructive: false,
            idempotent: false,
            open_world: true,
        },
        json!({
            "delta_x": { "type": "number", "description": "Horizontal CSS-pixel scroll delta. Defaults to 0; at least one delta must be non-zero." },
            "delta_y": { "type": "number", "description": "Vertical CSS-pixel scroll delta. Defaults to 0; at least one delta must be non-zero. Positive scrolls down." },
            "x": { "type": "number", "minimum": 0, "description": "CSS-pixel X for container-targeted scroll. Provide with y." },
            "y": { "type": "number", "minimum": 0, "description": "CSS-pixel Y for container-targeted scroll. Provide with x." }
        }),
        json!(["tab_id"]),
    )
}

fn browser_eval_tool() -> Value {
    browser_tab_tool(
        "browser_eval",
        "Evaluate JavaScript in a controllable user_chrome tab and return a serializable value. Diagnostic fallback; hidden unless SKY_CUA_BROWSER_EVAL is on, 1, or true.",
        OPEN_WORLD_DESTRUCTIVE_ACTION,
        json!({
            "expression": {
                "type": "string",
                "description": "JavaScript expression. Promises are awaited; serializable results return by value."
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
            "description": "tab_id from browser_open or browser_claim_tab. Claim ids from browser_list_tabs first."
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
