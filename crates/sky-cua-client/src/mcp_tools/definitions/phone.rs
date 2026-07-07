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
        "serial": optional_absent_string_schema(phone_serial_schema())
    })
}

pub(super) fn with_phone_selector(properties: Value) -> Value {
    merge_properties(properties, phone_selector_properties())
}

pub(super) fn phone_session_properties() -> Value {
    json!({
        "session_id": phone_session_id_schema()
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
    exact_branch_constraints(
        &phone_connection_properties(),
        "operation",
        &[
            (
                "connect",
                &[][..],
                &[
                    "operation",
                    "serial",
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
    )
}

pub(super) fn phone_setup_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["install_companion", "open_settings"]},
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
    json!({
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
    })
}

pub(super) fn phone_pointer_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["tap", "swipe"]},
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
    ];
    json!({
        "allOf": [{"oneOf": branches}]
    })
}

pub(super) fn phone_keyboard_properties() -> Value {
    with_phone_session(json!({
        "operation": {"type": "string", "enum": ["type_text", "press_key"]},
        "text": non_empty_string_schema(),
        "key": non_blank_string_schema()
    }))
}

pub(super) fn phone_keyboard_constraints() -> Value {
    exact_branch_constraints(
        &phone_keyboard_properties(),
        "operation",
        &[
            (
                "type_text",
                &["session_id", "text"][..],
                &["operation", "session_id", "text"][..],
            ),
            (
                "press_key",
                &["session_id", "key"][..],
                &["operation", "session_id", "key"][..],
            ),
        ],
    )
}

pub(super) fn phone_notification_action_properties() -> Value {
    with_phone_session(json!({
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
    exact_branch_constraints(
        &phone_notification_action_properties(),
        "operation",
        &[
            (
                "open",
                &["session_id", "event_id"][..],
                &["operation", "session_id", "event_id"][..],
            ),
            (
                "dismiss",
                &["session_id", "event_id"][..],
                &["operation", "session_id", "event_id"][..],
            ),
            (
                "action",
                &["session_id", "event_id", "action_id"][..],
                &["operation", "session_id", "event_id", "action_id"][..],
            ),
        ],
    )
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
    json!({
        "allOf": [{"oneOf": branches}]
    })
}

pub(super) fn phone_app_install_properties() -> Value {
    with_phone_session(json!({
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
    json!({
        "required": ["session_id", "apk_paths"]
    })
}
