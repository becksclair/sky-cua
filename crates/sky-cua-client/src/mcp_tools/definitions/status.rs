//! Schema builders for the status/meta tool family: `status`,
//! `list_resources`, and `session_presence`.

use serde_json::{Map, Value, json};
use sky_cua_platform::config::AgentSurfacePolicy;

use super::browser::browser_target_schema;
use super::common::*;
use super::phone::phone_session_id_schema;

fn enum_schema(values: Vec<&'static str>) -> Value {
    json!({"type": "string", "enum": values})
}

pub(super) fn status_properties(surfaces: AgentSurfacePolicy) -> Value {
    let mut components = Vec::new();
    if surfaces.browser {
        components.push("browser");
    }
    if surfaces.phone {
        components.extend(["phone", "phone_companion"]);
    }
    if surfaces.desktop {
        components.push("session_presence");
    }

    let mut properties = Map::new();
    properties.insert("component".into(), enum_schema(components));
    if surfaces.phone {
        properties.insert(
            "refresh_devices".into(),
            optional_bool_schema(json!({
                "type": "boolean",
                "description": "For component=\"phone\" only, ask the service to refresh device discovery before reporting status."
            })),
        );
        properties.insert("session_id".into(), phone_session_id_schema());
    }
    Value::Object(properties)
}

pub(super) fn status_constraints(surfaces: AgentSurfacePolicy) -> Value {
    let properties = status_properties(surfaces);
    let mut branches = Vec::new();
    if surfaces.browser {
        branches.push(exact_branch_schema(
            &properties,
            &[("component", "browser")],
            &[],
            &["component"],
        ));
    }
    if surfaces.phone {
        branches.push(exact_branch_schema(
            &properties,
            &[("component", "phone")],
            &[],
            &["component", "refresh_devices"],
        ));
        branches.push(exact_branch_schema(
            &properties,
            &[("component", "phone_companion")],
            &[],
            &["component", "session_id"],
        ));
    }
    if surfaces.desktop {
        branches.push(exact_branch_schema(
            &properties,
            &[("component", "session_presence")],
            &[],
            &["component"],
        ));
    }
    json!({"oneOf": branches})
}

pub(super) fn list_resources_properties(surfaces: AgentSurfacePolicy) -> Value {
    let mut surface_values = Vec::new();
    if surfaces.desktop {
        surface_values.push("desktop");
    }
    if surfaces.browser {
        surface_values.push("browser");
    }
    if surfaces.phone {
        surface_values.push("phone");
    }
    let mut resource_values = Vec::new();
    if surfaces.desktop || surfaces.phone {
        resource_values.push("apps");
    }
    if surfaces.desktop {
        resource_values.extend(["windows", "focused_window"]);
    }
    if surfaces.browser {
        resource_values.push("tabs");
    }
    if surfaces.phone {
        resource_values.extend(["devices", "current_app"]);
    }

    let mut properties = Map::new();
    properties.insert("surface".into(), enum_schema(surface_values));
    properties.insert("resource".into(), enum_schema(resource_values));
    if surfaces.browser {
        properties.insert(
            "target".into(),
            optional_absent_string_schema(browser_target_schema()),
        );
        properties.insert(
            "url_contains".into(),
            json!({
                "type": ["string", "null"],
                "description": "For browser tabs only, case-insensitive URL filter."
            }),
        );
        properties.insert(
            "title_contains".into(),
            json!({
                "type": ["string", "null"],
                "description": "For browser tabs only, case-insensitive title filter."
            }),
        );
    }
    if surfaces.phone {
        properties.insert(
            "include_mdns".into(),
            optional_bool_schema(json!({
                "type": "boolean",
                "description": "For phone devices only, include mDNS wireless-debugging records."
            })),
        );
        properties.insert("session_id".into(), phone_session_id_schema());
        properties.insert(
            "include_system".into(),
            optional_bool_schema(json!({
                "type": "boolean",
                "description": "For phone apps only, include system packages."
            })),
        );
    }
    if surfaces.desktop || surfaces.browser || surfaces.phone {
        properties.insert("limit".into(), optional_limit_schema());
    }
    Value::Object(properties)
}

pub(super) fn list_resources_constraints(surfaces: AgentSurfacePolicy) -> Value {
    let properties = list_resources_properties(surfaces);
    let mut branches = Vec::new();
    if surfaces.desktop {
        branches.extend([
            exact_branch_schema(
                &properties,
                &[("surface", "desktop"), ("resource", "apps")],
                &[],
                &["surface", "resource", "limit"],
            ),
            exact_branch_schema(
                &properties,
                &[("surface", "desktop"), ("resource", "windows")],
                &[],
                &["surface", "resource", "limit"],
            ),
            exact_branch_schema(
                &properties,
                &[("surface", "desktop"), ("resource", "focused_window")],
                &[],
                &["surface", "resource"],
            ),
        ]);
    }
    if surfaces.browser {
        branches.push(exact_branch_schema(
            &properties,
            &[("surface", "browser"), ("resource", "tabs")],
            &[],
            &[
                "surface",
                "resource",
                "target",
                "url_contains",
                "title_contains",
                "limit",
            ],
        ));
    }
    if surfaces.phone {
        branches.extend([
            exact_branch_schema(
                &properties,
                &[("surface", "phone"), ("resource", "devices")],
                &[],
                &["surface", "resource", "include_mdns"],
            ),
            exact_branch_schema(
                &properties,
                &[("surface", "phone"), ("resource", "apps")],
                &["session_id"],
                &[
                    "surface",
                    "resource",
                    "session_id",
                    "include_system",
                    "limit",
                ],
            ),
            exact_branch_schema(
                &properties,
                &[("surface", "phone"), ("resource", "current_app")],
                &["session_id"],
                &["surface", "resource", "session_id"],
            ),
        ]);
    }
    json!({"oneOf": branches})
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
