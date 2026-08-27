//! Schema builders for the phone tool family: `phone_connection`,
//! `phone_setup`, `phone_pointer`, `phone_keyboard`,
//! `phone_notification_action`, `phone_app_action`, `phone_app_install`.

use serde_json::{Value, json};

use super::common::*;

pub(super) fn phone_session_id_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "Phone session_id returned by phone_connection(operation=\"connect\") or status(component=\"phone\") active sessions; required after connect."
    })
}

pub(super) fn phone_serial_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "ADB serial from phone discovery; accepted only for discovery, pairing, and connect-like paths."
    })
}

pub(super) fn phone_device_id_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "Stable CompanionDirect device id; direct links do not have an ADB serial."
    })
}

pub(super) fn phone_alias_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "pattern": ".*\\S.*",
        "description": "Human alias from [phone.aliases] mapping to a device_id or ADB serial (e.g. \"phone\", \"tablet\")."
    })
}

/// Replace post-connect `session_id` requirements with typed selector
/// alternatives: `session_id` (active session), `device_id` (direct id), or
/// human `alias` mapped in `[phone.aliases]`.
pub(crate) fn phone_selector_alternatives(mut schema: Value) -> Value {
    let device_schema = schema
        .get("properties")
        .and_then(|properties| properties.get("device_id"))
        .cloned()
        .unwrap_or_else(phone_device_id_schema);
    let alias_schema = schema
        .get("properties")
        .and_then(|properties| properties.get("alias"))
        .cloned()
        .unwrap_or_else(phone_alias_schema);

    fn visit(value: &mut Value, device_schema: &Value, alias_schema: &Value) {
        match value {
            Value::Array(items) => items
                .iter_mut()
                .for_each(|item| visit(item, device_schema, alias_schema)),
            Value::Object(object) => {
                object
                    .values_mut()
                    .for_each(|item| visit(item, device_schema, alias_schema));
                let Some(Value::Array(required)) = object.remove("required") else {
                    return;
                };
                let names: Vec<String> = required
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                if !names.iter().any(|name| name == "session_id") {
                    object.insert(
                        "required".to_string(),
                        Value::Array(names.into_iter().map(Value::String).collect()),
                    );
                    return;
                }
                let rest: Vec<Value> = names
                    .into_iter()
                    .filter(|name| name != "session_id")
                    .map(Value::String)
                    .collect();
                let mut session_required = vec![Value::String("session_id".into())];
                session_required.extend(rest.clone());
                let mut device_required = vec![Value::String("device_id".into())];
                device_required.extend(rest.clone());
                let mut alias_required = vec![Value::String("alias".into())];
                alias_required.extend(rest);
                if let Some(Value::Object(properties)) = object.get_mut("properties") {
                    properties.insert("device_id".into(), device_schema.clone());
                    properties.insert("alias".into(), alias_schema.clone());
                }
                object.insert(
                    "oneOf".to_string(),
                    Value::Array(vec![
                        json!({"required": session_required}),
                        json!({"required": device_required}),
                        json!({"required": alias_required}),
                    ]),
                );
            }
            _ => {}
        }
    }
    visit(&mut schema, &device_schema, &alias_schema);
    schema
}

pub(super) fn phone_connect_backend_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["auto", "adb", "companion", "scrcpy"],
        "description": "Backend hint for phone connect."
    })
}

pub(super) fn phone_observe_backend_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["auto", "adb", "companion"],
        "description": "Backend hint for phone observe and screenshot. scrcpy/none are response states, not request inputs."
    })
}

pub(super) fn phone_selector_properties() -> Value {
    json!({
        "session_id": phone_session_id_schema(),
        "serial": optional_absent_string_schema(phone_serial_schema()),
        "device_id": optional_absent_string_schema(phone_device_id_schema()),
        "alias": optional_absent_string_schema(phone_alias_schema()),
        "appshot_id": optional_absent_string_schema(json!({"type":"string", "minLength":1, "description":"Canonical phone AppShot id returned by phone_observe; required for state-changing phone actions."}))
    })
}

pub(super) fn with_phone_selector(properties: Value) -> Value {
    merge_properties(properties, phone_selector_properties())
}

fn with_phone_action_selector(properties: Value) -> Value {
    merge_properties(
        properties,
        json!({
            "session_id": phone_session_id_schema(),
            "device_id": optional_absent_string_schema(phone_device_id_schema()),
            "alias": optional_absent_string_schema(phone_alias_schema()),
            "appshot_id": optional_absent_string_schema(json!({"type":"string", "minLength":1}))
        }),
    )
}

pub(super) fn phone_session_properties() -> Value {
    json!({
        "session_id": phone_session_id_schema(),
        "device_id": optional_absent_string_schema(phone_device_id_schema()),
        "alias": optional_absent_string_schema(phone_alias_schema())
    })
}

pub(super) fn with_phone_session(properties: Value) -> Value {
    merge_properties(properties, phone_session_properties())
}

pub(super) fn phone_connection_properties() -> Value {
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

pub(super) fn phone_connection_constraints() -> Value {
    phone_selector_alternatives(exact_branch_constraints(
        &phone_connection_properties(),
        "operation",
        &[
            (
                "connect",
                &[][..],
                &[
                    "operation",
                    "serial",
                    "device_id",
                    "alias",
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
    ))
}

pub(super) fn phone_setup_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["create_enrollment", "install_companion", "open_settings"]},
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

pub(super) fn phone_setup_constraints() -> Value {
    let properties = phone_setup_properties();
    let branches = vec![
        exact_branch_schema(
            &properties,
            &[("operation", "create_enrollment")],
            &[],
            &["operation"],
        ),
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
    phone_selector_alternatives(json!({
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
    }))
}

pub(super) fn phone_pointer_properties() -> Value {
    with_phone_action_selector(json!({
        "operation": {"type": "string", "enum": ["tap", "swipe", "long_press", "double_tap"]},
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
        "interval_ms": optional_null_schema(json!({"type": "integer", "minimum": 0, "description": "Interval between taps for double_tap."})),
        "use_device_coordinates": optional_bool_schema(json!({"type": "boolean", "description": "When true, x/y or start/end coordinates are raw device pixels and phone_snapshot_id is not required."}))
    }))
}

pub(super) fn phone_pointer_constraints() -> Value {
    let properties = phone_pointer_properties();
    let branches = vec![
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "tap")],
            &["session_id", "x", "y"],
            &[
                "operation",
                "session_id",
                "appshot_id",
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
                "appshot_id",
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
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "long_press")],
            &["session_id", "x", "y"],
            &[
                "operation",
                "session_id",
                "appshot_id",
                "phone_snapshot_id",
                "x",
                "y",
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
        exact_branch_schema_with_constraints(
            &properties,
            &[("operation", "double_tap")],
            &["session_id", "x", "y"],
            &[
                "operation",
                "session_id",
                "appshot_id",
                "phone_snapshot_id",
                "x",
                "y",
                "interval_ms",
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
    phone_selector_alternatives(json!({
        "allOf": [{"oneOf": branches}]
    }))
}

pub(super) fn phone_keyboard_properties() -> Value {
    with_phone_action_selector(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": non_empty_string_schema(),
        "key": non_blank_string_schema()
    }))
}

pub(super) fn phone_keyboard_constraints() -> Value {
    phone_selector_alternatives(exact_branch_constraints(
        &phone_keyboard_properties(),
        "operation",
        &[
            (
                "type_text",
                &["session_id", "text"][..],
                &["operation", "session_id", "device_id", "appshot_id", "text"][..],
            ),
            (
                "press_key",
                &["session_id", "key"][..],
                &["operation", "session_id", "device_id", "appshot_id", "key"][..],
            ),
        ],
    ))
}

pub(super) fn phone_notification_action_properties() -> Value {
    with_phone_action_selector(json!({
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

pub(super) fn phone_notification_action_constraints() -> Value {
    phone_selector_alternatives(exact_branch_constraints(
        &phone_notification_action_properties(),
        "operation",
        &[
            (
                "open",
                &["session_id", "event_id"][..],
                &[
                    "operation",
                    "session_id",
                    "device_id",
                    "appshot_id",
                    "event_id",
                ][..],
            ),
            (
                "dismiss",
                &["session_id", "event_id"][..],
                &[
                    "operation",
                    "session_id",
                    "device_id",
                    "appshot_id",
                    "event_id",
                ][..],
            ),
            (
                "action",
                &["session_id", "event_id", "action_id"][..],
                &[
                    "operation",
                    "session_id",
                    "device_id",
                    "appshot_id",
                    "event_id",
                    "action_id",
                ][..],
            ),
        ],
    ))
}

pub(super) fn phone_app_action_properties() -> Value {
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

pub(super) fn phone_app_action_constraints() -> Value {
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
    phone_selector_alternatives(json!({
        "allOf": [{"oneOf": branches}]
    }))
}

pub(super) fn phone_app_install_properties() -> Value {
    with_phone_action_selector(json!({
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

pub(super) fn phone_app_install_constraints() -> Value {
    phone_selector_alternatives(json!({
        "required": ["session_id", "apk_paths"]
    }))
}

fn feature_properties(operations: &[&str]) -> Value {
    with_phone_action_selector(json!({
        "operation": {"type": "string", "enum": operations},
        "content_id": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "path": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "mime_type": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "payload": optional_null_schema(json!({"type":"object"})),
        "since_sequence": optional_null_schema(json!({"type":"integer", "minimum":0})),
        "limit": optional_null_schema(json!({"type":"integer", "minimum":1, "maximum":1000})),
        "text": optional_absent_string_schema(json!({"type":"string"})),
        "start": optional_null_schema(json!({"type":"integer", "minimum":0})),
        "end": optional_null_schema(json!({"type":"integer", "minimum":0})),
        "content": optional_null_schema(json!({"type":"object"})),
        "camera_id": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "options": optional_null_schema(json!({"type":"object"})),
        "controls": optional_null_schema(json!({"type":"object"})),
        "uri": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "source": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "destination": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "name": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "algorithm": optional_absent_string_schema(json!({"type":"string", "enum":["sha256"]})),
        "root": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "query": optional_absent_string_schema(json!({"type":"string"})),
        "max_width": optional_null_schema(json!({"type":"integer", "minimum":1})),
        "max_height": optional_null_schema(json!({"type":"integer", "minimum":1})),
        "root_id": optional_absent_string_schema(json!({"type":"string", "minLength":1}))
    }))
}

pub(super) fn phone_content_properties() -> Value {
    feature_properties(&[
        "describe",
        "import_host_file",
        "export_host_file",
        "release",
    ])
}
pub(super) fn phone_clipboard_properties() -> Value {
    feature_properties(&["get", "set", "clear", "changes"])
}
pub(super) fn phone_editor_properties() -> Value {
    feature_properties(&[
        "context",
        "set_text",
        "insert_text",
        "set_selection",
        "select_all",
        "copy",
        "cut",
        "paste",
        "insert_content",
    ])
}
pub(super) fn phone_camera_properties() -> Value {
    let mut properties = feature_properties(&[
        "enumerate",
        "capabilities",
        "photo",
        "video_start",
        "video_pause",
        "video_resume",
        "video_stop",
        "preview_start",
        "preview_frame",
        "preview_stop",
        "controls",
    ]);
    if let Some(properties) = properties.as_object_mut() {
        properties.insert(
            "camera_session_id".into(),
            optional_absent_string_schema(json!({
                "type": "string",
                "minLength": 1,
                "description": "Opaque preview/video session handle returned by phone_camera preview_start or video_start."
            })),
        );
        properties.insert(
            "options".into(),
            optional_null_schema(json!({
                "type": "object",
                "description": "Capture options. V1 rejects image dimensions above 1920x1080 or portrait 1080x1920; video stops after 60000 ms.",
                "properties": {
                    "size": {
                        "type": "object",
                        "properties": {
                            "width": {"type": "integer", "minimum": 1, "maximum": 1920},
                            "height": {"type": "integer", "minimum": 1, "maximum": 1920}
                        },
                        "required": ["width", "height"],
                        "additionalProperties": false,
                        "anyOf": [
                            {"properties": {"width": {"maximum": 1920}, "height": {"maximum": 1080}}},
                            {"properties": {"width": {"maximum": 1080}, "height": {"maximum": 1920}}}
                        ]
                    },
                    "fps": {"type": "integer", "minimum": 1},
                    "flash": {"type": "string", "enum": ["off", "on", "auto", "screen"]},
                    "include_audio": {"type": "boolean"},
                    "mime_type": {"type": "string", "minLength": 1}
                },
                "additionalProperties": false
            })),
        );
    }
    properties
}

pub(super) fn phone_camera_constraints() -> Value {
    json!({
        "allOf": [
            phone_feature_constraints(&["enumerate", "capabilities", "preview_frame"]),
            {
                "if": {
                    "required": ["operation"],
                    "properties": {
                        "operation": {
                            "enum": [
                                "video_pause", "video_resume", "video_stop",
                                "preview_frame", "preview_stop", "controls"
                            ]
                        }
                    }
                },
                "then": {"required": ["camera_session_id"]}
            }
        ]
    })
}
pub(super) fn phone_storage_properties() -> Value {
    feature_properties(&[
        "roots",
        "list",
        "stat",
        "read",
        "write",
        "mkdir",
        "copy",
        "move",
        "rename",
        "delete",
        "trash",
        "hash",
        "search",
        "thumbnail",
        "metadata",
        "add_saf_root",
        "remove_saf_root",
        "list_saf_roots",
    ])
}

pub(super) fn phone_feature_constraints(read_operations: &[&str]) -> Value {
    let selector = phone_selector_alternatives(json!({
        "required": ["session_id", "operation"]
    }));
    json!({
        "allOf": [
            selector,
            {
                "if": {
                    "required": ["operation"],
                    "properties": {
                        "operation": {"not": {"enum": read_operations}}
                    }
                },
                "then": {"required": ["appshot_id"]}
            }
        ]
    })
}

pub(super) fn phone_node_action_properties() -> Value {
    with_phone_action_selector(json!({
        "action": {"type": "string", "enum": ["click","long_click","context_click","dismiss","expand","collapse","scroll_forward","scroll_backward","scroll_up","scroll_down","scroll_left","scroll_right","page_up","page_down","page_left","page_right","scroll_to_position","focus","clear_focus","accessibility_focus","clear_accessibility_focus","select","clear_selection","show_on_screen","set_progress","set_text","set_selection","copy","cut","paste","next_at_movement_granularity","previous_at_movement_granularity","next_html_element","previous_html_element","press_and_hold","ime_enter","move_window","show_tooltip","hide_tooltip"]},
        "appshot_id": optional_absent_string_schema(json!({"type":"string", "minLength":1})),
        "node_id": optional_null_schema(json!({"type":"integer"})),
        "view_id": optional_absent_string_schema(json!({"type":"string", "minLength":1, "description":"viewIdResourceName like com.skycua.phonecompanion:id/playground_click_button"})),
        "args": optional_null_schema(json!({"type":"object", "description":"Action-specific bundle args like {\"text\":\"hi\"} for set_text or {\"progress\":0.5} for set_progress"}))
    }))
}

pub(super) fn phone_node_action_constraints() -> Value {
    phone_selector_alternatives(json!({
        "allOf": [
            {"required": ["session_id", "action"]},
            {"anyOf": [{"required": ["node_id"]}, {"required": ["view_id"]}]}
        ]
    }))
}

pub(super) fn phone_global_action_properties() -> Value {
    with_phone_action_selector(json!({
        "action": {"type": "string", "enum": ["back","home","recents","notifications","quick_settings","power_dialog","toggle_split_screen","lock_screen","take_screenshot","keycode_headset_hook","accessibility_button","accessibility_button_chooser","accessibility_shortcut","accessibility_all_apps","dismiss_notification_shade","dpad_up","dpad_down","dpad_left","dpad_right","dpad_center","menu"]}
    }))
}

pub(super) fn phone_global_action_constraints() -> Value {
    phone_selector_alternatives(json!({"required": ["session_id", "action"]}))
}

pub(super) fn phone_key_event_properties() -> Value {
    with_phone_action_selector(json!({
        "key_code": {"type":"string", "minLength":1, "description":"Android keycode name (KEYCODE_VOLUME_UP) or numeric string (24)"},
        "meta_state": optional_null_schema(json!({"type":"integer", "minimum":0})),
        "repeat_count": optional_null_schema(json!({"type":"integer", "minimum":0}))
    }))
}

pub(super) fn phone_key_event_constraints() -> Value {
    phone_selector_alternatives(json!({"required": ["session_id", "key_code"]}))
}
