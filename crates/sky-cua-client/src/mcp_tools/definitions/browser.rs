//! Schema builders for the browser tool family: `browser_open`,
//! `browser_navigate`, `browser_claim_tab`, `browser_input`,
//! `browser_move_mouse`, `browser_scroll`.

use serde_json::{Value, json};

use crate::app_state::APP_STATE_MAX_ELEMENT_QUERY_CHARS;

use super::common::*;

pub(super) fn browser_target_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["user_chrome"],
        "description": "Browser bridge target. Defaults to user_chrome."
    })
}

pub(super) fn browser_tab_id_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "pattern": ".*\\S.*",
        "description": "Browser tab_id returned by browser_open or list_resources(surface=\"browser\", resource=\"tabs\"). Claim listed existing tabs before acting."
    })
}

pub(super) fn browser_tab_properties() -> Value {
    json!({
        "target": optional_absent_string_schema(browser_target_schema()),
        "tab_id": browser_tab_id_schema()
    })
}

pub(super) fn browser_target_url_properties(require_tab: bool) -> Value {
    let mut properties = json!({
        "target": optional_absent_string_schema(browser_target_schema()),
        "url": if require_tab { browser_url_schema() } else { optional_absent_string_schema(browser_url_schema()) }
    });
    if require_tab && let Some(map) = properties.as_object_mut() {
        map.insert("tab_id".to_string(), browser_tab_id_schema());
    }
    properties
}

pub(super) fn browser_point_properties() -> Value {
    merge_properties(
        browser_tab_properties(),
        json!({
            "appshot_id": non_blank_string_schema(),
            "x": {"type": "number", "minimum": 0, "description": "CSS pixel x coordinate."},
            "y": {"type": "number", "minimum": 0, "description": "CSS pixel y coordinate."},
            "wait_for_arrival": optional_bool_schema(json!({
                "type": "boolean",
                "description": "Wait for the visible cursor overlay to arrive. Defaults to true."
            }))
        }),
    )
}

pub(super) fn browser_xy_properties() -> Value {
    merge_properties(
        browser_tab_properties(),
        json!({
            "x": {"type": "number", "minimum": 0, "description": "CSS pixel x coordinate."},
            "y": {"type": "number", "minimum": 0, "description": "CSS pixel y coordinate."}
        }),
    )
}

pub(super) fn browser_optional_xy_properties() -> Value {
    merge_properties(
        browser_tab_properties(),
        json!({
            "x": optional_null_schema(json!({"type": "number", "minimum": 0, "description": "CSS pixel x coordinate."})),
            "y": optional_null_schema(json!({"type": "number", "minimum": 0, "description": "CSS pixel y coordinate."}))
        }),
    )
}

pub(super) fn browser_element_ref_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "An opaque element reference from observe(surface=browser). Prefer it over x/y for reliable clicks on dynamic pages; the service re-resolves the element's live position. Do not construct or parse it."
    })
}

pub(super) fn browser_input_properties() -> Value {
    merge_properties(
        browser_xy_properties(),
        json!({
            "appshot_id": non_blank_string_schema(),
            "operation": {"type": "string", "enum": ["click", "type_text", "press_key"]},
            "text": non_empty_string_schema(),
            "key": non_blank_string_schema(),
            "ref": browser_element_ref_schema()
        }),
    )
}

pub(super) fn browser_input_constraints() -> Value {
    let properties = browser_input_properties();
    json!({
        "allOf": [exact_branch_one_of(
            &properties,
            &[
                // click targets either coordinates ({x, y}) or an element ref
                // ({ref}); exactly one of the two is required.
                (
                    vec![("operation", "click")],
                    &["tab_id", "appshot_id"][..],
                    &["operation", "target", "tab_id", "appshot_id", "x", "y", "ref"][..],
                    Some(json!({
                        "oneOf": [
                            {"required": ["x", "y"]},
                            {"required": ["ref"]}
                        ]
                    })),
                ),
                // type_text always requires text; ref is optional (present =>
                // type into that element, absent => the current focus).
                (
                    vec![("operation", "type_text")],
                    &["tab_id", "appshot_id", "text"][..],
                    &["operation", "target", "tab_id", "appshot_id", "text", "ref"][..],
                    None,
                ),
                (
                    vec![("operation", "press_key")],
                    &["tab_id", "appshot_id", "key"][..],
                    &["operation", "target", "tab_id", "appshot_id", "key"][..],
                    None,
                ),
            ],
        )]
    })
}

pub(super) fn browser_scroll_properties() -> Value {
    merge_properties(
        browser_optional_xy_properties(),
        json!({
            "appshot_id": non_blank_string_schema(),
            "delta_x": {"type": "number", "description": "Horizontal wheel delta in CSS pixels; at least one delta must be non-zero."},
            "delta_y": {"type": "number", "description": "Vertical wheel delta in CSS pixels; at least one delta must be non-zero."}
        }),
    )
}

pub(super) fn browser_scroll_constraints() -> Value {
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

pub(super) fn browser_snapshot_window_properties() -> Value {
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

pub(super) fn browser_capture_timeout_property() -> Value {
    optional_null_schema(json!({
        "type": "integer",
        "minimum": sky_cua_platform::model::BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS,
        "maximum": sky_cua_platform::model::BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS,
        "description": "Browser AppShot capture deadline in milliseconds. Defaults to the service budget."
    }))
}

pub(super) fn browser_url_schema() -> Value {
    json!({
        "type": "string",
        "pattern": "^(https?://[^\\s]+|about:blank)$"
    })
}
