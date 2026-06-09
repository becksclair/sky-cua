use serde_json::{Value, json};

pub(crate) fn push_tool_definitions(tool_array: &mut Vec<Value>) {
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
}

fn browser_status_tool() -> Value {
    json!({
        "name": "browser_status",
        "description": "Report first-class Browser Use readiness for sky-cua, including user-Chrome availability, planned managed-browser lifecycle status, native host manifest state, and browser bridge diagnostics.",
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
        "description": "Move the webpage/browser cursor in a claimed or session-owned user_chrome tab using browser screenshot pixel coordinates. Use browser_claim_tab for existing user tabs, or browser_open for session-owned tabs.",
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
                    "description": "Target X coordinate in browser screenshot pixels; sky-cua converts through devicePixelRatio before sending browser input."
                },
                "y": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Target Y coordinate in browser screenshot pixels; sky-cua converts through devicePixelRatio before sending browser input."
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
        "Return a structured browser page snapshot for a claimed or session-owned user_chrome tab, including title, URL, visible text, viewport, and common element summaries when available.",
        json!({}),
        json!(["tab_id"]),
    )
}

fn browser_screenshot_tool() -> Value {
    browser_tab_tool(
        "browser_screenshot",
        "Capture a PNG screenshot from a claimed or session-owned user_chrome tab through CDP and return base64 image data.",
        json!({}),
        json!(["tab_id"]),
    )
}

fn browser_click_tool() -> Value {
    browser_tab_tool(
        "browser_click",
        "Click within a claimed or session-owned user_chrome tab using browser screenshot pixel coordinates.",
        json!({
            "x": { "type": "number", "minimum": 0, "description": "Target X coordinate in browser screenshot pixels; sky-cua converts through devicePixelRatio before sending browser input." },
            "y": { "type": "number", "minimum": 0, "description": "Target Y coordinate in browser screenshot pixels; sky-cua converts through devicePixelRatio before sending browser input." }
        }),
        json!(["tab_id", "x", "y"]),
    )
}

fn browser_type_text_tool() -> Value {
    browser_tab_tool(
        "browser_type_text",
        "Type literal text into the focused element in a claimed or session-owned user_chrome tab.",
        json!({
            "text": { "type": "string", "description": "Text to insert into the focused webpage control." }
        }),
        json!(["tab_id", "text"]),
    )
}

fn browser_press_key_tool() -> Value {
    browser_tab_tool(
        "browser_press_key",
        "Press a keyboard key in a claimed or session-owned user_chrome tab using CDP Input.dispatchKeyEvent key names.",
        json!({
            "key": { "type": "string", "description": "CDP key name, such as Enter, Tab, Escape, ArrowDown, or a printable character." }
        }),
        json!(["tab_id", "key"]),
    )
}

fn browser_scroll_tool() -> Value {
    browser_tab_tool(
        "browser_scroll",
        "Scroll the page viewport within a claimed or session-owned user_chrome tab. Positive delta_y scrolls down.",
        json!({
            "delta_x": { "type": "number", "description": "Horizontal scroll delta in browser screenshot pixels. Defaults to 0; sky-cua converts through devicePixelRatio." },
            "delta_y": { "type": "number", "description": "Vertical scroll delta in browser screenshot pixels. Positive values scroll down; sky-cua converts through devicePixelRatio." },
            "x": { "type": "number", "minimum": 0, "description": "Wheel event X context coordinate in browser screenshot pixels. Defaults to 0; sky-cua converts through devicePixelRatio." },
            "y": { "type": "number", "minimum": 0, "description": "Wheel event Y context coordinate in browser screenshot pixels. Defaults to 0; sky-cua converts through devicePixelRatio." }
        }),
        json!(["tab_id"]),
    )
}

fn browser_tab_tool(
    name: &str,
    description: &str,
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
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}
