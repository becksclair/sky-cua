//! Schema builders for the desktop tool family: `observe` (desktop
//! surface), `capture_screen`/`capture_desktop`, `desktop_semantic`,
//! `desktop_pointer`, `desktop_keyboard`, `desktop_action`.

use serde_json::{Value, json};

use crate::app_state::{
    APP_STATE_DEFAULT_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_LIMIT, APP_STATE_MAX_ELEMENT_QUERY_CHARS,
};

use super::browser::*;
use super::common::*;
use super::phone::*;

pub(super) fn observe_properties(can_receive_images: bool) -> Value {
    let mut properties = merge_properties(
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
    );
    if let Some(properties) = properties.as_object_mut() {
        // Canonical AppShots always capture the selected surface. These legacy
        // get_app_state knobs do not alter the AppShot producer and must not be
        // advertised as if they did.
        properties.remove("desktop_file_id");
        properties.remove("capture_screen");
        properties.remove("screenshot_delivery");
    }
    properties
}

pub(super) fn observe_constraints(can_receive_images: bool) -> Value {
    let desktop_allowed = vec![
        "surface",
        "app_id",
        "window_title",
        "name",
        "detail",
        "element_query",
        "element_offset",
        "element_limit",
    ];
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

pub(super) fn capture_screen_properties() -> Value {
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

pub(super) fn capture_screen_constraints() -> Value {
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
