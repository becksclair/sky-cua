//! MCP tool JSON definitions for the 27 `phone_*` tools.
//!
//! Mirrors `browser/schema.rs`: each tool is a `{name, description,
//! annotations, inputSchema}` object with `additionalProperties: false`. The
//! shared `session_id`/`serial` selector is injected into every device-bound
//! tool so callers can name a session by id (preferred) or raw serial.

use serde_json::{Value, json};

use crate::mcp_tools::annotations::{
    LOCAL_DESTRUCTIVE_ACTION, LOCAL_NAVIGATION_ACTION, READ_ONLY_TOOL, ToolAnnotations,
};

/// Action tools that dispatch arbitrary input into the device or other apps.
const PHONE_ACTION_TOOL: ToolAnnotations = LOCAL_DESTRUCTIVE_ACTION;
/// Local navigation actions: connect/disconnect/refresh/install/open-settings.
/// Reversible, idempotent, and cannot trigger arbitrary in-app behavior.
const PHONE_NAVIGATION_ACTION: ToolAnnotations = LOCAL_NAVIGATION_ACTION;

pub(crate) fn push_tool_definitions(tool_array: &mut Vec<Value>) {
    tool_array.push(phone_observe_tool());
    tool_array.push(phone_status_tool());
    tool_array.push(phone_list_devices_tool());
    tool_array.push(phone_refresh_capabilities_tool());
    tool_array.push(phone_pair_wireless_tool());
    tool_array.push(phone_connect_tool());
    tool_array.push(phone_disconnect_tool());
    tool_array.push(phone_screenshot_tool());
    tool_array.push(phone_tap_tool());
    tool_array.push(phone_swipe_tool());
    tool_array.push(phone_type_text_tool());
    tool_array.push(phone_press_key_tool());
    tool_array.push(phone_install_companion_tool());
    tool_array.push(phone_companion_status_tool());
    tool_array.push(phone_accessibility_tree_tool());
    tool_array.push(phone_notifications_tool());
    tool_array.push(phone_notification_open_tool());
    tool_array.push(phone_notification_dismiss_tool());
    tool_array.push(phone_notification_action_tool());
    tool_array.push(phone_notification_reply_tool());
    tool_array.push(phone_app_current_tool());
    tool_array.push(phone_app_list_tool());
    tool_array.push(phone_app_launch_tool());
    tool_array.push(phone_app_open_intent_tool());
    tool_array.push(phone_app_force_stop_tool());
    tool_array.push(phone_app_install_tool());
    tool_array.push(phone_open_settings_tool());
}

/// The shared `session_id`/`serial` selector schema, present on every
/// device-bound tool. Phone tools resolve either to the active session.
fn session_selector_properties() -> serde_json::Map<String, Value> {
    let Value::Object(map) = json!({
        "session_id": {
            "type": "string",
            "description": "Session id from phone_connect (preferred). Resolves to an active session."
        },
        "serial": {
            "type": "string",
            "description": "Device serial. Resolves to the matching active session when session_id is omitted."
        }
    }) else {
        unreachable!("session selector schema is an object literal")
    };
    map
}

fn backend_property() -> serde_json::Map<String, Value> {
    let Value::Object(map) = json!({
        "backend": {
            "type": "string",
            "enum": ["auto", "adb", "companion", "scrcpy", "none"],
            "description": "Force a backend. Defaults to auto-routing the best available backend."
        }
    }) else {
        unreachable!("backend schema is an object literal")
    };
    map
}

/// Build a tool definition with the shared selector merged into `extra`.
fn phone_tool(
    name: &str,
    description: &str,
    annotations: ToolAnnotations,
    extra: Value,
    required: Value,
) -> Value {
    let mut properties = session_selector_properties();
    if let Value::Object(extra) = extra {
        properties.extend(extra);
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

/// Build a tool with a fixed-schema input that does NOT take the session
/// selector (host-tooling tools: status, list_devices, pair_wireless, connect).
fn phone_fixed_tool(
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

fn phone_observe_tool() -> Value {
    let mut extra = backend_property();
    extra.extend(observe_extra_properties());
    phone_observe_tool_inner(Value::Object(extra))
}

fn phone_observe_tool_inner(extra: Value) -> Value {
    phone_tool(
        "phone_observe",
        "Observe a connected phone in one call: foreground app, optional screenshot, optional accessibility summary and recent notifications, available/unavailable actions, and the capability profile id. Image-capable sessions get an image block; text-only sessions get screenshot_path and metadata.",
        READ_ONLY_TOOL,
        extra,
        json!([]),
    )
}

fn observe_extra_properties() -> serde_json::Map<String, Value> {
    let Value::Object(map) = json!({
        "include_accessibility": {
            "type": "boolean",
            "description": "Include a bounded accessibility summary. Defaults to false."
        },
        "include_notifications": {
            "type": "boolean",
            "description": "Include recent notification events. Defaults to false."
        }
    }) else {
        unreachable!("observe extra schema is an object literal")
    };
    map
}

fn phone_status_tool() -> Value {
    phone_fixed_tool(
        "phone_status",
        "Report Phone Use host readiness: adb/scrcpy availability and version, companion enablement, default serial/backend, active sessions, and known devices.",
        READ_ONLY_TOOL,
        json!({
            "refresh_devices": {
                "type": "boolean",
                "description": "Re-probe attached devices before reporting. Defaults to false."
            }
        }),
        json!([]),
    )
}

fn phone_list_devices_tool() -> Value {
    phone_fixed_tool(
        "phone_list_devices",
        "List Android devices visible to adb with serial, state, and connection kind. Use the chosen serial with phone_connect.",
        READ_ONLY_TOOL,
        json!({
            "include_mdns": {
                "type": "boolean",
                "description": "Include mDNS-discovered wireless devices. Defaults to false."
            }
        }),
        json!([]),
    )
}

fn phone_refresh_capabilities_tool() -> Value {
    phone_tool(
        "phone_refresh_capabilities",
        "Re-detect a connected device's capability profile (display, permissions, companion, scrcpy, available actions) and refresh the cache.",
        PHONE_NAVIGATION_ACTION,
        json!({}),
        json!([]),
    )
}

fn phone_pair_wireless_tool() -> Value {
    phone_fixed_tool(
        "phone_pair_wireless",
        "Pair Android 11+ wireless debugging using the host:port and one-time pairing code shown on the device. The pairing code is never echoed back.",
        PHONE_NAVIGATION_ACTION,
        json!({
            "host_port": {
                "type": "string",
                "description": "host:port of the pairing endpoint shown on the device."
            },
            "pairing_code": {
                "type": "string",
                "description": "One-time pairing code from the device. Never logged or echoed."
            }
        }),
        json!(["host_port", "pairing_code"]),
    )
}

fn phone_connect_tool() -> Value {
    let mut properties = backend_property();
    properties.extend(connect_extra_properties());
    phone_fixed_tool(
        "phone_connect",
        "Connect to a device by serial (USB, emulator, or host:port). Detects the capability profile and optionally installs the companion or starts scrcpy. Returns the live session.",
        PHONE_NAVIGATION_ACTION,
        Value::Object(properties),
        json!([]),
    )
}

fn connect_extra_properties() -> serde_json::Map<String, Value> {
    let Value::Object(map) = json!({
        "serial": {
            "type": "string",
            "description": "USB/emulator serial or host:port wireless target. Defaults to the configured default or the single connected device."
        },
        "install_companion": {
            "type": "boolean",
            "description": "Install the companion app during connect when missing. Defaults to false."
        },
        "start_scrcpy": {
            "type": "boolean",
            "description": "Start a managed scrcpy mirror during connect. Defaults to false."
        }
    }) else {
        unreachable!("connect extra schema is an object literal")
    };
    map
}

fn phone_disconnect_tool() -> Value {
    phone_tool(
        "phone_disconnect",
        "Disconnect a phone session, tearing down managed scrcpy/companion processes. Optionally keep the wireless connection alive.",
        PHONE_NAVIGATION_ACTION,
        json!({
            "keep_wireless": {
                "type": "boolean",
                "description": "Keep the wireless adb connection after disconnecting the session. Defaults to false."
            }
        }),
        json!([]),
    )
}

fn phone_screenshot_tool() -> Value {
    phone_tool(
        "phone_screenshot",
        "Capture the device screen. Returns a phone_snapshot_id plus a device-to-screenshot coordinate mapping; phone_tap/phone_swipe coordinates are screenshot pixels in that snapshot. Image-capable sessions get an image block; text-only sessions get screenshot_path and metadata.",
        READ_ONLY_TOOL,
        backend_property_value(),
        json!([]),
    )
}

fn backend_property_value() -> Value {
    Value::Object(backend_property())
}

fn phone_tap_tool() -> Value {
    phone_tool(
        "phone_tap",
        "Tap a screenshot-pixel point from the latest phone_screenshot/phone_observe snapshot. Pass phone_snapshot_id; coordinate translation requires the snapshot's mapping.",
        PHONE_ACTION_TOOL,
        json!({
            "phone_snapshot_id": {
                "type": "string",
                "description": "snapshot id from phone_screenshot or phone_observe. Required unless use_device_coordinates is set."
            },
            "x": { "type": "number", "minimum": 0, "description": "X in screenshot pixels; matches phone_screenshot bounds." },
            "y": { "type": "number", "minimum": 0, "description": "Y in screenshot pixels; matches phone_screenshot bounds." },
            "use_device_coordinates": {
                "type": "boolean",
                "description": "Treat x/y as raw device pixels instead of snapshot screenshot pixels. Defaults to false."
            }
        }),
        json!(["x", "y"]),
    )
}

fn phone_swipe_tool() -> Value {
    phone_tool(
        "phone_swipe",
        "Swipe between two screenshot-pixel points from the latest snapshot, optionally over a duration. Pass phone_snapshot_id for coordinate translation.",
        PHONE_ACTION_TOOL,
        json!({
            "phone_snapshot_id": {
                "type": "string",
                "description": "snapshot id from phone_screenshot or phone_observe. Required unless use_device_coordinates is set."
            },
            "start_x": { "type": "number", "minimum": 0, "description": "Start X in screenshot pixels." },
            "start_y": { "type": "number", "minimum": 0, "description": "Start Y in screenshot pixels." },
            "end_x": { "type": "number", "minimum": 0, "description": "End X in screenshot pixels." },
            "end_y": { "type": "number", "minimum": 0, "description": "End Y in screenshot pixels." },
            "duration_ms": {
                "type": "integer",
                "minimum": 0,
                "description": "Swipe duration in milliseconds. Defaults to the backend's default."
            },
            "use_device_coordinates": {
                "type": "boolean",
                "description": "Treat coordinates as raw device pixels instead of snapshot screenshot pixels. Defaults to false."
            }
        }),
        json!(["start_x", "start_y", "end_x", "end_y"]),
    )
}

fn phone_type_text_tool() -> Value {
    phone_tool(
        "phone_type_text",
        "Type literal text into the focused field on the device. Focus the field first.",
        PHONE_ACTION_TOOL,
        json!({
            "text": { "type": "string", "description": "Literal text; spaces and newlines are preserved." }
        }),
        json!(["text"]),
    )
}

fn phone_press_key_tool() -> Value {
    phone_tool(
        "phone_press_key",
        "Press an Android keycode by name or number (e.g. KEYCODE_BACK, 4, home, KEYCODE_ENTER).",
        PHONE_ACTION_TOOL,
        json!({
            "key": { "type": "string", "description": "Android keycode name or number, e.g. KEYCODE_BACK, 4, home." }
        }),
        json!(["key"]),
    )
}

fn phone_install_companion_tool() -> Value {
    phone_tool(
        "phone_install_companion",
        "Install or update the Phone Use companion app on the connected device. Optionally force a reinstall or allow a downgrade.",
        PHONE_NAVIGATION_ACTION,
        json!({
            "force_reinstall": {
                "type": "boolean",
                "description": "Reinstall even when the expected version is already present. Defaults to false."
            },
            "allow_downgrade": {
                "type": "boolean",
                "description": "Allow installing an older companion version. Defaults to false."
            }
        }),
        json!([]),
    )
}

fn phone_companion_status_tool() -> Value {
    phone_tool(
        "phone_companion_status",
        "Report companion app status on a session: installed version, signature match, permission grants (accessibility, gestures, window content, notifications), and RPC reachability.",
        READ_ONLY_TOOL,
        json!({}),
        json!([]),
    )
}

fn phone_accessibility_tree_tool() -> Value {
    phone_tool(
        "phone_accessibility_tree",
        "Retrieve a bounded accessibility tree for the foreground app: parent-indexed nodes with class, text, content description, bounds, and clickable/focusable/enabled flags. Sensitive text may be redacted.",
        READ_ONLY_TOOL,
        json!({
            "node_limit": {
                "type": "integer",
                "minimum": 0,
                "description": "Maximum nodes returned. The truncated flag marks when more exist."
            }
        }),
        json!([]),
    )
}

fn phone_notifications_tool() -> Value {
    phone_tool(
        "phone_notifications",
        "List recent notification events with stable event_ids for the notification action tools. Body text may be redacted per the device's redaction policy.",
        READ_ONLY_TOOL,
        json!({
            "limit": {
                "type": "integer",
                "minimum": 0,
                "description": "Maximum notification events returned. The truncated flag marks when more exist."
            }
        }),
        json!([]),
    )
}

fn phone_notification_open_tool() -> Value {
    phone_notification_event_tool(
        "phone_notification_open",
        "Open a notification by event_id, launching its content intent. event_id must come from a fresh phone_notifications observation.",
        json!({}),
        json!(["event_id"]),
    )
}

fn phone_notification_dismiss_tool() -> Value {
    phone_notification_event_tool(
        "phone_notification_dismiss",
        "Dismiss a notification by event_id. event_id must come from a fresh phone_notifications observation.",
        json!({}),
        json!(["event_id"]),
    )
}

fn phone_notification_action_tool() -> Value {
    phone_notification_event_tool(
        "phone_notification_action",
        "Invoke a notification action button by event_id and action_id. Use phone_notification_reply for inline-reply actions.",
        json!({
            "action_id": {
                "type": "string",
                "description": "action_id from the notification event's actions list."
            }
        }),
        json!(["event_id", "action_id"]),
    )
}

fn phone_notification_reply_tool() -> Value {
    phone_notification_event_tool(
        "phone_notification_reply",
        "Send an inline reply to a notification's reply action by event_id and action_id with literal reply text.",
        json!({
            "action_id": {
                "type": "string",
                "description": "action_id of an inline-reply-capable notification action."
            },
            "text": {
                "type": "string",
                "description": "Literal reply text; spaces and newlines are preserved."
            }
        }),
        json!(["event_id", "action_id", "text"]),
    )
}

fn phone_notification_event_tool(
    name: &str,
    description: &str,
    extra: Value,
    required: Value,
) -> Value {
    let mut properties = json!({
        "event_id": {
            "type": "string",
            "description": "event_id from a fresh phone_notifications observation."
        }
    });
    if let (Some(properties), Value::Object(extra)) = (properties.as_object_mut(), extra) {
        properties.extend(extra);
    }
    phone_tool(name, description, PHONE_ACTION_TOOL, properties, required)
}

fn phone_app_current_tool() -> Value {
    phone_tool(
        "phone_app_current",
        "Report the foreground app on the device: package, label, activity, and version.",
        READ_ONLY_TOOL,
        json!({}),
        json!([]),
    )
}

fn phone_app_list_tool() -> Value {
    phone_tool(
        "phone_app_list",
        "List installed apps on the device. Include system apps and cap the count with limit; the truncated flag marks when more exist.",
        READ_ONLY_TOOL,
        json!({
            "include_system": {
                "type": "boolean",
                "description": "Include system apps. Defaults to false."
            },
            "limit": {
                "type": "integer",
                "minimum": 0,
                "description": "Maximum apps returned."
            }
        }),
        json!([]),
    )
}

fn phone_app_launch_tool() -> Value {
    phone_tool(
        "phone_app_launch",
        "Launch an app by package name on the connected device.",
        PHONE_ACTION_TOOL,
        json!({
            "package_name": {
                "type": "string",
                "description": "Android package name, e.g. com.android.settings."
            }
        }),
        json!(["package_name"]),
    )
}

fn phone_app_open_intent_tool() -> Value {
    phone_tool(
        "phone_app_open_intent",
        "Open an activity component, deep link, or intent URI on the device. Optionally scope to a target package.",
        PHONE_ACTION_TOOL,
        json!({
            "intent_uri": {
                "type": "string",
                "description": "Activity component, deep link, or intent URI to launch."
            },
            "package_name": {
                "type": "string",
                "description": "Optional target package to scope the intent."
            }
        }),
        json!(["intent_uri"]),
    )
}

fn phone_app_force_stop_tool() -> Value {
    phone_tool(
        "phone_app_force_stop",
        "Force-stop an app by package name on the connected device.",
        PHONE_NAVIGATION_ACTION,
        json!({
            "package_name": {
                "type": "string",
                "description": "Android package name to force-stop."
            }
        }),
        json!(["package_name"]),
    )
}

fn phone_app_install_tool() -> Value {
    phone_tool(
        "phone_app_install",
        "Install one or more host-side APKs on the device. mode selects single, install-multiple (splits of one package), or install-multi-package. Optionally reinstall, allow downgrade/test APKs, or grant runtime permissions.",
        PHONE_ACTION_TOOL,
        json!({
            "apk_paths": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Host-side APK path(s). single mode uses the first entry."
            },
            "mode": {
                "type": "string",
                "enum": ["single", "multiple", "multi_package"],
                "description": "Install strategy. Defaults to single."
            },
            "reinstall": {
                "type": "boolean",
                "description": "Reinstall over an existing package keeping data. Defaults to false."
            },
            "allow_downgrade": {
                "type": "boolean",
                "description": "Allow installing an older version code. Defaults to false."
            },
            "allow_test_apk": {
                "type": "boolean",
                "description": "Allow installing a test-only APK. Defaults to false."
            },
            "grant_runtime_permissions": {
                "type": "boolean",
                "description": "Grant all runtime permissions on install. Defaults to false."
            }
        }),
        json!(["apk_paths"]),
    )
}

fn phone_open_settings_tool() -> Value {
    phone_tool(
        "phone_open_settings",
        "Open a specific Android settings screen on the device to grant a permission or toggle a debugging option. App-scoped screens accept a package_name.",
        PHONE_NAVIGATION_ACTION,
        json!({
            "screen": {
                "type": "string",
                "enum": [
                    "accessibility",
                    "notification_access",
                    "overlay_permission",
                    "app_details",
                    "wireless_debugging",
                    "battery_optimization"
                ],
                "description": "Which settings screen to open."
            },
            "package_name": {
                "type": "string",
                "description": "Target package for app-scoped screens such as app_details."
            }
        }),
        json!(["screen"]),
    )
}
