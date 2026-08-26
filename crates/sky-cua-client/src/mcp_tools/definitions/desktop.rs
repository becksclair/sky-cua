//! Schema builders for the desktop tool family: `observe` (desktop
//! surface), `capture_screen`/`capture_desktop`, `desktop_semantic`,
//! `desktop_pointer`, `desktop_keyboard`, `desktop_action`.

use serde_json::{Map, Value, json};
use sky_cua_platform::config::AgentSurfacePolicy;

use crate::app_state::{
    APP_STATE_DEFAULT_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_QUERY_CHARS,
};

use super::browser::*;
use super::common::*;
use super::phone::*;

pub(super) fn observe_properties(can_receive_images: bool, surfaces: AgentSurfacePolicy) -> Value {
    let mut surface_values = Vec::new();
    let mut properties = Map::new();
    if surfaces.desktop {
        surface_values.push("desktop");
        if let Some(desktop) = get_app_state_properties(can_receive_images).as_object() {
            for (key, value) in desktop {
                if !matches!(
                    key.as_str(),
                    "desktop_file_id" | "capture_screen" | "screenshot_delivery"
                ) {
                    properties.insert(key.clone(), value.clone());
                }
            }
        }
        if let Some(window_target) = window_target_schema().as_object() {
            for (key, value) in window_target {
                properties.insert(key.clone(), value.clone());
            }
        }
    }
    if surfaces.browser {
        surface_values.push("browser");
        properties.insert(
            "target".into(),
            optional_absent_string_schema(browser_target_schema()),
        );
        properties.insert("tab_id".into(), browser_tab_id_schema());
        properties.insert(
            "text_limit".into(),
            optional_null_schema(json!({
                "type": "integer",
                "minimum": 0,
                "maximum": sky_cua_platform::model::BROWSER_SNAPSHOT_MAX_TEXT_LIMIT,
                "description": "For browser only, maximum page text characters."
            })),
        );
        if let Some(browser) = browser_snapshot_window_properties().as_object() {
            for (key, value) in browser {
                properties.insert(key.clone(), value.clone());
            }
        }
        properties.insert(
            "capture_timeout_ms".into(),
            browser_capture_timeout_property(),
        );
    }
    if surfaces.phone {
        surface_values.push("phone");
        properties.insert(
            "include_accessibility".into(),
            optional_bool_schema(json!({
                "type": "boolean",
                "description": "For phone only, include the accessibility tree in the observation."
            })),
        );
        properties.insert(
            "include_notifications".into(),
            optional_bool_schema(json!({
                "type": "boolean",
                "description": "For phone only, include recent notifications in the observation."
            })),
        );
        properties.insert(
            "backend".into(),
            optional_absent_string_schema(phone_observe_backend_schema()),
        );
        if let Some(phone) = phone_session_properties().as_object() {
            for (key, value) in phone {
                properties.insert(key.clone(), value.clone());
            }
        }
    }
    properties.insert(
        "surface".into(),
        json!({"type": "string", "enum": surface_values}),
    );
    Value::Object(properties)
}

pub(super) fn observe_constraints(can_receive_images: bool, surfaces: AgentSurfacePolicy) -> Value {
    let properties = observe_properties(can_receive_images, surfaces);
    let mut branches = Vec::new();
    if surfaces.desktop {
        branches.push(exact_branch_schema(
            &properties,
            &[("surface", "desktop")],
            &[],
            &[
                "surface",
                "window_id",
                "pid",
                "tty",
                "terminal_pid",
                "terminal_command",
                "terminal_cwd",
                "app_id",
                "wm_class",
                "title",
                "window_title",
                "name",
                "detail",
                "element_query",
                "element_offset",
                "element_limit",
            ],
        ));
    }
    if surfaces.browser {
        branches.push(exact_branch_schema(
            &properties,
            &[("surface", "browser")],
            &["tab_id"],
            &[
                "surface",
                "target",
                "tab_id",
                "text_limit",
                "element_query",
                "element_offset",
                "element_limit",
                "capture_timeout_ms",
            ],
        ));
    }
    if surfaces.phone {
        branches.push(exact_branch_schema(
            &properties,
            &[("surface", "phone")],
            &["session_id"],
            &[
                "surface",
                "session_id",
                "device_id",
                "alias",
                "include_accessibility",
                "include_notifications",
                "backend",
            ],
        ));
    }
    phone_selector_alternatives(json!({"oneOf": branches}))
}

pub(super) fn capture_screen_properties(surfaces: AgentSurfacePolicy) -> Value {
    let mut surface_values = Vec::new();
    let mut properties = Map::new();
    if surfaces.browser {
        surface_values.push("browser");
        properties.insert(
            "target".into(),
            optional_absent_string_schema(browser_target_schema()),
        );
        properties.insert("tab_id".into(), browser_tab_id_schema());
    }
    if surfaces.phone {
        surface_values.push("phone");
        properties.insert(
            "backend".into(),
            optional_absent_string_schema(phone_observe_backend_schema()),
        );
        if let Some(phone) = phone_session_properties().as_object() {
            for (key, value) in phone {
                properties.insert(key.clone(), value.clone());
            }
        }
    }
    properties.insert(
        "surface".into(),
        json!({"type": "string", "enum": surface_values}),
    );
    Value::Object(properties)
}

pub(super) fn capture_screen_constraints(surfaces: AgentSurfacePolicy) -> Value {
    let properties = capture_screen_properties(surfaces);
    let mut branches = Vec::new();
    if surfaces.browser {
        branches.push(exact_branch_schema(
            &properties,
            &[("surface", "browser")],
            &["tab_id"],
            &["surface", "target", "tab_id"],
        ));
    }
    if surfaces.phone {
        branches.push(exact_branch_schema(
            &properties,
            &[("surface", "phone")],
            &["session_id"],
            &["surface", "session_id", "device_id", "alias", "backend"],
        ));
    }
    phone_selector_alternatives(json!({"oneOf": branches}))
}

pub(super) fn desktop_semantic_properties(properties: Value) -> Value {
    action_tool_properties(merge_properties(properties, semantic_selector_properties()))
}

pub(super) fn desktop_pointer_properties() -> Value {
    action_tool_properties(merge_properties(
        json!({
            "operation": {"type": "string", "enum": ["click", "secondary_click", "drag"]},
            "x": coordinate_schema("Click x coordinate or drag start x. With snapshot_id it is a screenshot pixel from that capture (translated to the screen for you); without snapshot_id it is a raw screen coordinate."),
            "y": coordinate_schema("Click y coordinate or drag start y. With snapshot_id it is a screenshot pixel from that capture (translated to the screen for you); without snapshot_id it is a raw screen coordinate."),
            "from_x": {"type": "number"},
            "from_y": {"type": "number"},
            "to_x": {"type": "number"},
            "to_y": {"type": "number"},
            "to_element_index": {"type": "integer", "minimum": 0},
            "duration_ms": {"type": "integer", "minimum": 0, "description": "Drag only. Paces the injected pointer path over this many milliseconds of wall-clock time; the drag is always interpolated, but a larger value (~400-800) makes sliders and drag-and-drop gestures track more reliably."}
        }),
        semantic_selector_properties(),
    ))
}

pub(super) fn desktop_selector_constraint() -> Value {
    json!({
        "anyOf": desktop_selector_alternatives()
    })
}

pub(super) fn desktop_one_selector_constraint() -> Value {
    json!({
        "oneOf": desktop_selector_alternatives()
    })
}

pub(super) fn desktop_snapshot_selector_constraint() -> Value {
    json!({
        "anyOf": [
            snapshot_selector_constraint(&["element_index"]),
            snapshot_selector_constraint(&["name"]),
            snapshot_selector_constraint(&["text"])
        ]
    })
}

pub(super) fn desktop_point_or_selector_constraint() -> Value {
    json!({
        "anyOf": [
            {"required": ["x", "y"]},
            snapshot_selector_constraint(&["element_index"]),
            snapshot_selector_constraint(&["name"]),
            snapshot_selector_constraint(&["text"])
        ]
    })
}

pub(super) fn desktop_selector_alternatives() -> Vec<Value> {
    vec![
        snapshot_selector_constraint(&["element_index"]),
        json!({"required": ["element_identifier"]}),
        snapshot_selector_constraint(&["name"]),
        snapshot_selector_constraint(&["text"]),
    ]
}

pub(super) fn snapshot_selector_constraint(fields: &[&str]) -> Value {
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

pub(super) fn desktop_pointer_constraints() -> Value {
    let properties = desktop_pointer_properties();
    let branches = vec![
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "click")],
            &["appshot_id"],
            &desktop_pointer_click_allowed_fields(),
            desktop_point_or_selector_constraint(),
        ),
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "secondary_click")],
            &["appshot_id"],
            &desktop_pointer_click_allowed_fields(),
            desktop_point_or_selector_constraint(),
        ),
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "drag")],
            &["appshot_id"],
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

pub(super) fn desktop_selector_allowed_fields() -> [&'static str; 8] {
    [
        "appshot_id",
        "snapshot_id",
        "element_index",
        "element_identifier",
        "role",
        "name",
        "text",
        "states",
    ]
}

pub(super) fn desktop_window_target_allowed_fields() -> [&'static str; 9] {
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

pub(super) fn desktop_pointer_click_allowed_fields() -> Vec<&'static str> {
    let mut fields = vec!["operation", "x", "y"];
    fields.extend(desktop_selector_allowed_fields());
    fields
}

pub(super) fn desktop_pointer_drag_allowed_fields() -> Vec<&'static str> {
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

pub(super) fn desktop_keyboard_allowed_fields(branch_field: &'static str) -> Vec<&'static str> {
    let mut fields = vec!["operation", "appshot_id", branch_field, "snapshot_id"];
    fields.extend(desktop_window_target_allowed_fields());
    fields
}

pub(super) fn desktop_action_allowed_fields(action_fields: &[&'static str]) -> Vec<&'static str> {
    let mut fields = vec!["operation"];
    fields.extend(desktop_selector_allowed_fields());
    fields.extend(action_fields.iter().copied());
    fields
}

pub(super) fn desktop_keyboard_properties() -> Value {
    action_tool_properties(keyboard_target_properties(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": non_empty_string_schema(),
        "key": non_empty_string_schema()
    })))
}

pub(super) fn desktop_keyboard_constraints() -> Value {
    exact_branch_constraints(
        &desktop_keyboard_properties(),
        "operation",
        &[
            (
                "type_text",
                &["appshot_id", "text"][..],
                &desktop_keyboard_allowed_fields("text"),
            ),
            (
                "press_key",
                &["appshot_id", "key"][..],
                &desktop_keyboard_allowed_fields("key"),
            ),
        ],
    )
}

pub(super) fn desktop_action_properties() -> Value {
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

pub(super) fn desktop_action_constraints() -> Value {
    let properties = desktop_action_properties();
    let branches = vec![
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "activate")],
            &["appshot_id"],
            &desktop_action_allowed_fields(&[]),
            desktop_selector_constraint(),
        ),
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "perform_action")],
            &["appshot_id"],
            &desktop_action_allowed_fields(&["action_name", "action_index"]),
            desktop_selector_action_constraint(),
        ),
    ];
    json!({
        "allOf": [{"oneOf": branches}]
    })
}

pub(super) fn desktop_selector_action_constraint() -> Value {
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

pub(super) fn action_tool_properties(mut properties: Value) -> Value {
    let property_map = properties
        .as_object_mut()
        .expect("action tool properties must be object");
    property_map.insert(
        "appshot_id".to_string(),
        optional_absent_string_schema(json!({
            "type": "string",
            "minLength": 1,
            "description": "Canonical desktop AppShot id from the same observe result; required for every state-changing desktop action."
        })),
    );
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

pub(super) fn semantic_selector_properties() -> Value {
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

pub(super) fn get_app_state_properties(can_receive_images: bool) -> Value {
    let mut properties = json!({
        "app_id": { "type": ["string", "null"] },
        "desktop_file_id": { "type": ["string", "null"] },
        "window_title": { "type": ["string", "null"] },
        "name": { "type": ["string", "null"] },
        "detail": optional_null_schema(json!({
            "type": "string",
            "enum": ["full", "compact"],
            "description": "Desktop only. Defaults to compact. Use full for verbose element details and full capability data."
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

pub(super) fn screenshot_properties(can_receive_images: bool) -> Value {
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

pub(super) fn screenshot_constraints() -> Value {
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

pub(super) fn coordinate_schema(description: &str) -> Value {
    json!({
        "type": "number",
        "description": description
    })
}

pub(super) fn keyboard_target_properties(mut properties: Value) -> Value {
    let Value::Object(properties_map) = &mut properties else {
        return properties;
    };
    if let Value::Object(target_map) = window_target_schema() {
        properties_map.extend(target_map);
    }
    properties
}
