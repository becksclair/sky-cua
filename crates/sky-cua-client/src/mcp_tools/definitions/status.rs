//! Schema builders for the status/meta tool family: `status`,
//! `list_resources`, and `session_presence`.

use serde_json::{Value, json};

use super::browser::browser_target_schema;
use super::common::*;
use super::phone::phone_session_id_schema;

pub(super) fn status_properties() -> Value {
    json!({
        "component": {"type": "string", "enum": ["browser", "phone", "phone_companion", "session_presence"]},
        "refresh_devices": optional_bool_schema(json!({
            "type": "boolean",
            "description": "For component=\"phone\" only, ask the service to refresh device discovery before reporting status."
        })),
        "session_id": phone_session_id_schema()
    })
}

pub(super) fn status_constraints() -> Value {
    exact_branch_constraints(
        &status_properties(),
        "component",
        &[
            ("browser", &["component"][..], &["component"][..]),
            (
                "phone",
                &["component"][..],
                &["component", "refresh_devices"][..],
            ),
            (
                "phone_companion",
                &["component"][..],
                &["component", "session_id"][..],
            ),
            ("session_presence", &["component"][..], &["component"][..]),
        ],
    )
}

pub(super) fn list_resources_properties() -> Value {
    json!({
        "surface": {"type": "string", "enum": ["desktop", "browser", "phone"]},
        "resource": {"type": "string", "enum": ["apps", "windows", "focused_window", "tabs", "devices", "current_app"]},
        "target": optional_absent_string_schema(browser_target_schema()),
        "url_contains": {
            "type": ["string", "null"],
            "description": "For browser tabs only, case-insensitive URL filter."
        },
        "title_contains": {
            "type": ["string", "null"],
            "description": "For browser tabs only, case-insensitive title filter."
        },
        "include_mdns": optional_bool_schema(json!({
            "type": "boolean",
            "description": "For phone devices only, include mDNS wireless-debugging records."
        })),
        "session_id": phone_session_id_schema(),
        "include_system": optional_bool_schema(json!({
            "type": "boolean",
            "description": "For phone apps only, include system packages."
        })),
        "limit": optional_limit_schema()
    })
}

pub(super) fn list_resources_constraints() -> Value {
    let properties = list_resources_properties();
    json!({
        "oneOf": [
            exact_branch_schema(&properties, &[("surface", "desktop"), ("resource", "apps")], &[], &["surface", "resource"]),
            exact_branch_schema(&properties, &[("surface", "desktop"), ("resource", "windows")], &[], &["surface", "resource"]),
            exact_branch_schema(&properties, &[("surface", "desktop"), ("resource", "focused_window")], &[], &["surface", "resource"]),
            exact_branch_schema(&properties, &[("surface", "browser"), ("resource", "tabs")], &[], &["surface", "resource", "target", "url_contains", "title_contains", "limit"]),
            exact_branch_schema(&properties, &[("surface", "phone"), ("resource", "devices")], &[], &["surface", "resource", "include_mdns"]),
            exact_branch_schema(&properties, &[("surface", "phone"), ("resource", "apps")], &["session_id"], &["surface", "resource", "session_id", "include_system", "limit"]),
            exact_branch_schema(&properties, &[("surface", "phone"), ("resource", "current_app")], &["session_id"], &["surface", "resource", "session_id"])
        ]
    })
}

pub(super) fn session_presence_constraints() -> Value {
    exact_branch_constraints(
        &json!({
            "operation": {"type": "string", "enum": ["hold", "unlock", "release"]},
            "unlock": {"type": "boolean"},
            "inhibit_lock": {"type": "boolean"},
            "inhibit_suspend": {"type": "boolean"},
            "relock": {"type": "boolean"}
        }),
        "operation",
        &[
            (
                "hold",
                &[][..],
                &["operation", "unlock", "inhibit_lock", "inhibit_suspend"][..],
            ),
            (
                "unlock",
                &[][..],
                &["operation", "inhibit_lock", "inhibit_suspend"][..],
            ),
            ("release", &[][..], &["operation", "relock"][..]),
        ],
    )
}
