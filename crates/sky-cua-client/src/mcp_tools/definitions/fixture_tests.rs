//! Golden-fixture tests for the MCP tool contract: pins tool
//! annotations and cross-checks the advertised registry against
//! `tests/fixtures/{tool_contract,call_cases,mcp_tool_surface_matrix}.json`.

use super::{
    InactiveToolReason, McpConfigDiagnostic, McpProcessConfig, McpToolRegistry,
    build_tool_definitions, build_tool_registry, mcp_process_config_from_env, schema_accepts,
    schema_keyword_is_supported,
};
use crate::mcp_server::ModelSessionInfo;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Mutex};

use super::contract_fixtures::*;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// (read_only, destructive, idempotent, open_world) per tool.
type AnnotationRow = (&'static str, (bool, bool, bool, bool));

/// Pinned annotation rows per tool.
///
/// Hosts gate per-tool approval on these MCP annotations — Codex "auto"
/// approval mode silently approves read-only tools and prompts for
/// destructive/open-world ones, and treats unannotated tools as both.
/// Changing a row here changes which sky-cua calls hosts auto-approve,
/// so it must be a deliberate decision.
const EXPECTED: &[AnnotationRow] = &[
    ("doctor", (true, false, true, false)),
    ("status", (true, false, true, false)),
    ("list_resources", (true, false, true, false)),
    ("observe", (true, false, true, false)),
    ("capture_screen", (true, false, true, false)),
    ("phone_accessibility_tree", (true, false, true, false)),
    ("phone_notifications", (true, false, true, false)),
    ("capture_desktop", (false, false, true, false)),
    ("setup_desktop", (false, false, true, false)),
    ("session_presence", (false, false, true, false)),
    ("activate_window", (false, false, true, false)),
    ("desktop_semantic", (false, false, true, false)),
    ("browser_claim_tab", (false, false, true, false)),
    ("browser_move_mouse", (false, false, true, false)),
    ("phone_connection", (false, false, true, false)),
    ("phone_pair_wireless", (false, false, true, false)),
    ("phone_setup", (false, true, true, false)),
    ("phone_app_force_stop", (false, true, true, false)),
    ("desktop_toggle", (false, false, false, false)),
    ("desktop_scroll", (false, false, false, false)),
    ("browser_scroll", (false, false, false, true)),
    ("desktop_pointer", (false, true, false, false)),
    ("desktop_keyboard", (false, true, false, false)),
    ("desktop_action", (false, true, false, false)),
    ("desktop_launch_app", (false, true, false, false)),
    ("desktop_set_value", (false, true, true, false)),
    ("browser_open", (false, false, false, true)),
    ("browser_navigate", (false, false, true, true)),
    ("browser_input", (false, true, false, true)),
    ("phone_pointer", (false, true, false, false)),
    ("phone_keyboard", (false, true, false, false)),
    ("phone_notification_action", (false, true, false, false)),
    ("phone_notification_reply", (false, true, false, false)),
    ("phone_app_action", (false, true, false, false)),
    ("phone_app_install", (false, true, false, false)),
    ("phone_content", (false, true, false, false)),
    ("phone_clipboard", (false, true, false, false)),
    ("phone_editor", (false, true, false, false)),
    ("phone_camera", (false, true, false, false)),
    ("phone_storage", (false, true, false, false)),
    ("browser_eval", (false, true, false, true)),
];

fn exact_branch<'a>(schema: &'a Value, discriminator: &str, value: &str) -> &'a Value {
    schema["allOf"]
        .as_array()
        .and_then(|all_of| {
            all_of.iter().find_map(|constraint| {
                constraint["oneOf"].as_array().and_then(|one_of| {
                    one_of
                        .iter()
                        .find(|branch| branch["properties"][discriminator]["const"] == value)
                })
            })
        })
        .unwrap_or_else(|| panic!("missing {discriminator}={value} exact branch"))
}

fn assert_no_duplicate_merge_keys_in_all_of(schema: &Value) {
    match schema {
        Value::Object(object) => {
            if let Some(all_of) = object.get("allOf").and_then(Value::as_array) {
                for key in ["if", "not", "anyOf", "oneOf"] {
                    let key_count = all_of
                        .iter()
                        .filter(|item| {
                            item.as_object()
                                .is_some_and(|object| object.contains_key(key))
                        })
                        .count();
                    assert!(
                        key_count <= 1,
                        "schema contains duplicate {key} constraints under one allOf: {schema:?}"
                    );
                }
            }
            for value in object.values() {
                assert_no_duplicate_merge_keys_in_all_of(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                assert_no_duplicate_merge_keys_in_all_of(value);
            }
        }
        _ => {}
    }
}

fn assert_required_properties_are_self_contained(schema: &Value) {
    match schema {
        Value::Object(object) => {
            if let Some(required) = object.get("required").and_then(Value::as_array) {
                let properties = object.get("properties").and_then(Value::as_object);
                for field in required.iter().filter_map(Value::as_str) {
                    assert!(
                        properties.is_some_and(|properties| properties.contains_key(field)),
                        "schema requires {field:?} without defining it in local properties: {schema:?}"
                    );
                }
            }
            for value in object.values() {
                assert_required_properties_are_self_contained(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                assert_required_properties_are_self_contained(value);
            }
        }
        _ => {}
    }
}

#[test]
fn every_tool_pins_honest_mcp_annotations() {
    for can_receive_images in [false, true] {
        let tools = build_tool_definitions(can_receive_images, false);
        let tools = tools.as_array().expect("tool definitions array");
        assert!(!tools.is_empty());
        for tool in tools {
            let name = tool["name"].as_str().expect("tool name");
            let annotations = tool
                .get("annotations")
                .unwrap_or_else(|| panic!("tool {name} is missing annotations"));
            let expected = EXPECTED
                .iter()
                .find(|(expected_name, _)| *expected_name == name)
                .unwrap_or_else(|| panic!("tool {name} has no pinned annotation row"));
            let (read_only, destructive, idempotent, open_world) = expected.1;
            assert_eq!(
                annotations["readOnlyHint"].as_bool(),
                Some(read_only),
                "{name} readOnlyHint"
            );
            assert_eq!(
                annotations["destructiveHint"].as_bool(),
                Some(destructive),
                "{name} destructiveHint"
            );
            assert_eq!(
                annotations["idempotentHint"].as_bool(),
                Some(idempotent),
                "{name} idempotentHint"
            );
            assert_eq!(
                annotations["openWorldHint"].as_bool(),
                Some(open_world),
                "{name} openWorldHint"
            );
        }
    }
}

#[test]
fn read_only_tools_never_mutate_per_their_own_hints() {
    let tools = build_tool_definitions(true, false);
    for tool in tools.as_array().expect("tool definitions array") {
        let annotations = &tool["annotations"];
        if annotations["readOnlyHint"] == true {
            assert_eq!(
                annotations["destructiveHint"], false,
                "read-only tool {} must not be destructive",
                tool["name"]
            );
        }
    }
}

pub(super) fn process_config(browser_eval_enabled: bool) -> McpProcessConfig {
    McpProcessConfig {
        browser_eval_enabled,
        surfaces: sky_cua_platform::config::AgentSurfacePolicy::default(),
        model_supports_images_override: None,
        diagnostics: Vec::new(),
    }
}

#[test]
fn registry_has_expected_name_budget() {
    let model = ModelSessionInfo::default();
    let registry = build_tool_registry(&process_config(false), &model);
    assert_eq!(registry.active_names.len(), 40);
    assert!(registry.contains("observe"));
    assert!(registry.contains("browser_input"));
    let doctor = registry
        .tools
        .as_array()
        .expect("tools array")
        .iter()
        .find(|tool| tool["name"] == "doctor")
        .expect("doctor");
    assert_eq!(doctor["annotations"]["readOnlyHint"], true);
    assert_eq!(doctor["annotations"]["idempotentHint"], true);
    assert!(!registry.contains("browser_eval"));
    assert!(!registry.contains("get_app_state"));
    assert_eq!(registry.inactive_reason("get_app_state"), None);
    assert_eq!(
        registry.inactive_reason("browser_eval"),
        Some(InactiveToolReason::BrowserEvalDisabled)
    );
}

#[test]
fn registry_adds_browser_eval_only_when_enabled() {
    let model = ModelSessionInfo::default();
    let registry = build_tool_registry(&process_config(true), &model);
    assert_eq!(registry.active_names.len(), 41);
    assert!(registry.contains("browser_eval"));
    assert_eq!(registry.inactive_reason("browser_eval"), None);
}

pub(super) fn process_config_with_surfaces(
    desktop: bool,
    browser: bool,
    phone: bool,
    browser_eval_enabled: bool,
) -> McpProcessConfig {
    McpProcessConfig {
        browser_eval_enabled,
        surfaces: sky_cua_platform::config::AgentSurfacePolicy {
            desktop,
            browser,
            phone,
        },
        model_supports_images_override: None,
        diagnostics: Vec::new(),
    }
}

#[test]
fn registry_projects_exact_surface_subset_and_shared_schemas() {
    let model = ModelSessionInfo::default();
    let browser = build_tool_registry(
        &process_config_with_surfaces(false, true, false, true),
        &model,
    );
    assert!(browser.contains("browser_input"));
    assert!(browser.contains("browser_eval"));
    assert!(!browser.contains("desktop_pointer"));
    assert!(!browser.contains("phone_pointer"));
    assert_eq!(browser.inactive_reason("phone_pointer"), None);

    fn tool<'a>(registry: &'a McpToolRegistry, name: &str) -> &'a Value {
        registry
            .tools
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    }
    assert_eq!(
        tool(&browser, "status")["inputSchema"]["properties"]["component"]["enum"],
        json!(["browser"])
    );
    assert_eq!(
        tool(&browser, "list_resources")["inputSchema"]["properties"]["surface"]["enum"],
        json!(["browser"])
    );
    assert_eq!(
        tool(&browser, "observe")["inputSchema"]["properties"]["surface"]["enum"],
        json!(["browser"])
    );
    assert!(
        tool(&browser, "observe")["inputSchema"]["properties"]
            .get("session_id")
            .is_none()
    );
    assert_eq!(
        tool(&browser, "capture_screen")["inputSchema"]["properties"]["surface"]["enum"],
        json!(["browser"])
    );

    let desktop = build_tool_registry(
        &process_config_with_surfaces(true, false, false, false),
        &model,
    );
    assert!(desktop.contains("capture_desktop"));
    assert!(!desktop.contains("capture_screen"));
    assert!(!desktop.contains("browser_open"));
    assert!(!desktop.contains("phone_connection"));
    assert_eq!(
        tool(&desktop, "observe")["inputSchema"]["properties"]["surface"]["enum"],
        json!(["desktop"])
    );

    let none = build_tool_registry(
        &process_config_with_surfaces(false, false, false, false),
        &model,
    );
    assert_eq!(
        none.active_names,
        ["doctor".to_string()].into_iter().collect()
    );
    assert_eq!(none.inactive_reason("browser_eval"), None);
}

#[test]
fn disabled_shared_branch_is_rejected_before_dispatch_contract() {
    let registry = build_tool_registry(
        &process_config_with_surfaces(true, false, true, false),
        &ModelSessionInfo::default(),
    );
    assert!(
        registry
            .validate_arguments("observe", &json!({"surface": "browser", "tab_id": "tab-1"}),)
            .is_err()
    );
    assert!(
        registry
            .validate_arguments("status", &json!({"component": "browser"}),)
            .is_err()
    );
}

#[test]
fn grouped_action_tool_schemas_reject_vague_desktop_actions() {
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    // The advertised schemas are flattened (no root composition); this test
    // asserts the *constraint* shape, which now lives in the validation
    // schemas. Synthesize per-tool objects that carry the advertised metadata
    // (name/description/annotations) with the rich validation `inputSchema`.
    let merged_tools: Vec<Value> = registry
        .tools
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| {
            let mut tool = tool.clone();
            let name = tool["name"].as_str().expect("tool name").to_string();
            if let Some(rich) = registry.validation_schemas.get(&name) {
                tool["inputSchema"] = rich.clone();
            }
            tool
        })
        .collect();
    let tool = |name: &str| -> &Value {
        merged_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing tool {name}"))
    };

    let pointer_schema = &tool("desktop_pointer")["inputSchema"];
    assert_eq!(
        pointer_schema["required"],
        json!(["operation", "appshot_id"])
    );
    assert!(
        pointer_schema["description"].is_null(),
        "tool-level description should stay on the MCP tool object"
    );
    assert!(
        exact_branch(pointer_schema, "operation", "click")["anyOf"]
            .as_array()
            .is_some_and(|any_of| {
                any_of
                    .iter()
                    .any(|item| item["required"] == json!(["x", "y"]))
                    && any_of
                        .iter()
                        .any(|item| item["required"] == json!(["snapshot_id", "element_index"]))
                    && !any_of
                        .iter()
                        .any(|item| item["required"] == json!(["element_identifier"]))
            }),
        "desktop_pointer click branch must require coordinates or a snapshot selector"
    );
    assert!(
        exact_branch(pointer_schema, "operation", "drag")["anyOf"]
            .as_array()
            .is_some_and(|any_of| {
                !any_of.iter().any(|item| {
                    item["required"]
                        == json!(["snapshot_id", "element_identifier", "to_element_index"])
                })
            }),
        "desktop_pointer drag must not advertise unsupported direct backend-ref starts"
    );
    assert!(
        tool("desktop_pointer")["description"]
            .as_str()
            .expect("desktop_pointer description")
            .contains("Do not call with only operation")
    );

    let activate_schema = &tool("activate_window")["inputSchema"];
    assert!(
        activate_schema["allOf"].as_array().is_some_and(|all_of| {
            all_of.iter().any(|constraint| {
                constraint["anyOf"].as_array().is_some_and(|any_of| {
                    any_of
                        .iter()
                        .any(|item| item["required"] == json!(["window_id"]))
                        && any_of.iter().any(|item| item["required"] == json!(["pid"]))
                })
            })
        }),
        "activate_window must require at least one active window target"
    );
    assert_eq!(
        activate_schema["properties"]["window_id"]["anyOf"][0]["minLength"],
        1
    );
    assert_eq!(
        activate_schema["properties"]["pid"]["anyOf"][0]["minimum"],
        1
    );

    let list_resources_schema = &tool("list_resources")["inputSchema"];
    let list_resource_pairs = list_resources_schema["allOf"]
        .as_array()
        .and_then(|all_of| {
            all_of
                .iter()
                .find_map(|constraint| constraint["oneOf"].as_array())
        })
        .expect("list_resources oneOf constraint");
    assert!(
        list_resource_pairs.iter().any(|pair| {
            pair["properties"]["surface"]["const"] == "browser"
                && pair["properties"]["resource"]["const"] == "tabs"
        }) && list_resource_pairs.iter().any(|pair| {
            pair["properties"]["surface"]["const"] == "phone"
                && pair["properties"]["resource"]["const"] == "current_app"
        }),
        "list_resources must constrain surface/resource pairs to dispatchable branches"
    );

    for schema in merged_tools.iter().map(|tool| &tool["inputSchema"]) {
        assert_eq!(
            schema["type"],
            Value::String("object".to_string()),
            "MCP adapters expect root inputSchema.type=object"
        );
        assert_no_duplicate_merge_keys_in_all_of(schema);
        assert_required_properties_are_self_contained(schema);
        for key in ["anyOf", "oneOf"] {
            assert!(
                schema.get(key).is_none(),
                "root {key} schemas must move composition under allOf for opencode-go/moonshot compatibility"
            );
        }
    }

    let observe_schema = &tool("observe")["inputSchema"];
    assert!(
        exact_branch(observe_schema, "surface", "browser")["required"]
            == json!(["surface", "tab_id"]),
        "observe browser branch must require tab_id"
    );

    let capture_schema = &tool("capture_screen")["inputSchema"];
    assert!(
        exact_branch(capture_schema, "surface", "browser")["required"]
            == json!(["surface", "tab_id"]),
        "capture_screen browser branch must require tab_id"
    );

    let browser_scroll_schema = &tool("browser_scroll")["inputSchema"];
    assert!(
        browser_scroll_schema["allOf"]
            .as_array()
            .is_some_and(|all_of| {
                all_of.iter().any(|constraint| {
                    constraint["anyOf"].as_array().is_some_and(|any_of| {
                        any_of
                            .iter()
                            .any(|item| item["required"] == json!(["delta_x"]))
                            && any_of
                                .iter()
                                .any(|item| item["required"] == json!(["delta_y"]))
                    })
                })
            }),
        "browser_scroll must require at least one scroll delta"
    );
    assert!(
        browser_scroll_schema["allOf"]
            .as_array()
            .is_some_and(|all_of| {
                all_of.iter().any(|constraint| {
                    constraint["if"]["anyOf"].as_array().is_some_and(|any_of| {
                        constraint["then"]["required"] == json!(["x", "y"])
                            && constraint["then"]["properties"]["x"]["type"] == "number"
                            && constraint["then"]["properties"]["y"]["type"] == "number"
                            && any_of.iter().any(|item| item["required"] == json!(["x"]))
                            && any_of.iter().any(|item| item["required"] == json!(["y"]))
                    })
                })
            }),
        "browser_scroll must require numeric x/y together"
    );

    assert_eq!(
        tool("browser_navigate")["inputSchema"]["properties"]["url"]["pattern"],
        "^(https?://[^\\s]+|about:blank)$"
    );
    assert_eq!(
        tool("browser_input")["inputSchema"]["properties"]["tab_id"]["minLength"],
        1,
        "browser tools must reject empty tab_id"
    );
    assert_eq!(
        tool("browser_input")["inputSchema"]["properties"]["text"]["minLength"],
        1,
        "browser_input type_text must reject empty text"
    );
    assert_eq!(
        tool("browser_input")["inputSchema"]["properties"]["key"]["minLength"],
        1,
        "browser_input press_key must reject empty key"
    );

    let action_schema = &tool("desktop_action")["inputSchema"];
    assert_eq!(
        action_schema["required"],
        json!(["operation", "appshot_id"])
    );
    assert_eq!(
        action_schema["properties"]["action_name"]["minLength"], 1,
        "desktop_action perform_action must reject empty action names"
    );
    let snapshot_id_options = action_schema["properties"]["snapshot_id"]["anyOf"]
        .as_array()
        .expect("desktop snapshot_id should allow absent sentinels");
    assert!(
        snapshot_id_options
            .iter()
            .any(|schema| schema["type"] == "string" && schema["minLength"] == 1)
            && snapshot_id_options
                .iter()
                .any(|schema| schema["type"] == "string" && schema["const"] == "")
            && snapshot_id_options
                .iter()
                .any(|schema| schema["type"] == "null"),
        "desktop snapshot_id should advertise non-empty values plus blank/null absent sentinels"
    );
    assert_eq!(
        action_schema["properties"]["element_identifier"]["minLength"], 1,
        "direct desktop element identifiers must reject empty strings"
    );
    assert_eq!(
        action_schema["properties"]["name"]["minLength"], 1,
        "desktop name selectors must reject empty strings"
    );
    assert_eq!(
        action_schema["properties"]["text"]["minLength"], 1,
        "desktop text selectors must reject empty strings"
    );
    assert!(
        exact_branch(action_schema, "operation", "activate")["anyOf"]
            .as_array()
            .is_some_and(|any_of| {
                any_of
                    .iter()
                    .any(|item| item["required"] == json!(["snapshot_id", "element_index"]))
                    && any_of
                        .iter()
                        .any(|item| item["required"] == json!(["element_identifier"]))
                    && any_of
                        .iter()
                        .any(|item| item["required"] == json!(["snapshot_id", "name"]))
                    && any_of
                        .iter()
                        .any(|item| item["required"] == json!(["snapshot_id", "text"]))
            }),
        "desktop_action must require snapshot_id for snapshot-bound selectors"
    );
    assert!(
        exact_branch(action_schema, "operation", "perform_action")["allOf"]
            .as_array()
            .is_some_and(|all_of| {
                all_of.iter().any(|item| {
                    item["oneOf"].as_array().is_some_and(|one_of| {
                        one_of.iter().any(|selector| {
                            selector["required"] == json!(["snapshot_id", "element_index"])
                        })
                    })
                }) && all_of.iter().any(|item| {
                    item["anyOf"].as_array().is_some_and(|any_of| {
                        any_of
                            .iter()
                            .any(|item| item["required"] == json!(["action_name"]))
                    })
                })
            }),
        "desktop_action perform_action branch must require action_name or action_index"
    );

    let keyboard_schema = &tool("desktop_keyboard")["inputSchema"];
    assert_eq!(
        keyboard_schema["properties"]["text"]["minLength"], 1,
        "desktop_keyboard type_text must reject empty text"
    );
    assert_eq!(
        keyboard_schema["properties"]["key"]["minLength"], 1,
        "desktop_keyboard press_key must reject empty key"
    );
    assert!(
        exact_branch(keyboard_schema, "operation", "press_key")["required"]
            == json!(["operation", "appshot_id", "key"])
            && exact_branch(keyboard_schema, "operation", "press_key")["additionalProperties"]
                == json!(false),
        "desktop_keyboard press_key branch must require key"
    );

    assert_eq!(
        tool("desktop_semantic")["inputSchema"]["required"],
        json!(["operation", "appshot_id"]),
        "desktop_semantic must allow non-index selectors"
    );
    assert_eq!(
        tool("desktop_toggle")["inputSchema"]["required"],
        json!(["appshot_id"]),
        "desktop_toggle must allow non-index selectors"
    );
    assert_eq!(
        tool("desktop_scroll")["inputSchema"]["required"],
        json!(["direction", "appshot_id"]),
        "desktop_scroll must allow non-index selectors"
    );
    assert!(
        tool("desktop_scroll")["inputSchema"]["properties"]["amount"].is_null(),
        "desktop_scroll must not advertise ignored amount"
    );
    assert!(
        tool("desktop_scroll")["inputSchema"]["properties"]["pages"].is_object(),
        "desktop_scroll should expose the pages field"
    );
    assert_eq!(
        tool("desktop_scroll")["inputSchema"]["properties"]["direction"]["enum"],
        json!(["up", "down", "left", "right"]),
        "desktop_scroll should expose every implemented scroll direction"
    );
    let desktop_scroll_any_of = tool("desktop_scroll")["inputSchema"]["allOf"]
        .as_array()
        .and_then(|all_of| {
            all_of
                .iter()
                .find_map(|constraint| constraint["anyOf"].as_array())
        })
        .expect("desktop_scroll anyOf constraint");
    assert!(
        desktop_scroll_any_of
            .iter()
            .any(|item| item["required"] == json!(["snapshot_id", "element_index"]))
            && !desktop_scroll_any_of
                .iter()
                .any(|item| item["required"] == json!(["element_identifier"])),
        "desktop_scroll must only advertise snapshot-resolved semantic targets"
    );
    assert_eq!(
        tool("desktop_set_value")["inputSchema"]["required"],
        json!(["value", "appshot_id"]),
        "desktop_set_value must allow non-index selectors"
    );

    let phone_setup_schema = &tool("phone_setup")["inputSchema"];
    assert_eq!(
        phone_setup_schema["properties"]["screen"]["enum"],
        json!([
            "accessibility",
            "notification_access",
            "overlay_permission",
            "app_details",
            "wireless_debugging",
            "battery_optimization"
        ])
    );
    let open_settings = exact_branch(phone_setup_schema, "operation", "open_settings");
    assert!(
        open_settings["oneOf"].as_array().is_some_and(|selectors| {
            selectors
                .iter()
                .any(|branch| branch["required"] == json!(["session_id", "operation", "screen"]))
                && selectors
                    .iter()
                    .any(|branch| branch["required"] == json!(["device_id", "operation", "screen"]))
        }),
        "phone_setup open_settings must require screen plus a typed selector"
    );

    let phone_pointer_schema = &tool("phone_pointer")["inputSchema"];
    for coordinate in ["x", "y", "start_x", "start_y", "end_x", "end_y"] {
        assert_eq!(
            phone_pointer_schema["properties"][coordinate]["minimum"], 0,
            "phone_pointer {coordinate} must reject negative coordinates"
        );
    }
    let tap = exact_branch(phone_pointer_schema, "operation", "tap");
    assert!(
        tap["oneOf"].as_array().is_some_and(|selectors| {
            selectors.iter().any(|branch| {
                branch["required"] == json!(["session_id", "operation", "appshot_id", "x", "y"])
            }) && selectors.iter().any(|branch| {
                branch["required"] == json!(["device_id", "operation", "appshot_id", "x", "y"])
            })
        }) && tap["additionalProperties"] == json!(false),
        "phone_pointer tap branch must require coordinates and a typed selector"
    );
    let swipe = exact_branch(phone_pointer_schema, "operation", "swipe");
    assert!(
        swipe["oneOf"].as_array().is_some_and(|selectors| {
            selectors.iter().any(|branch| {
                branch["required"]
                    == json!([
                        "session_id",
                        "operation",
                        "appshot_id",
                        "start_x",
                        "start_y",
                        "end_x",
                        "end_y"
                    ])
            }) && selectors.iter().any(|branch| {
                branch["required"]
                    == json!([
                        "device_id",
                        "operation",
                        "appshot_id",
                        "start_x",
                        "start_y",
                        "end_x",
                        "end_y"
                    ])
            })
        }) && swipe["additionalProperties"] == json!(false),
        "phone_pointer swipe branch must require start/end coordinates and a typed selector"
    );

    let phone_keyboard_schema = &tool("phone_keyboard")["inputSchema"];
    assert_eq!(
        phone_keyboard_schema["properties"]["text"]["minLength"], 1,
        "phone_keyboard type_text must reject empty text"
    );
    assert_eq!(
        phone_keyboard_schema["properties"]["key"]["minLength"], 1,
        "phone_keyboard press_key must reject empty key"
    );
    let type_text = exact_branch(phone_keyboard_schema, "operation", "type_text");
    assert!(
        type_text["oneOf"].as_array().is_some_and(|selectors| {
            selectors.iter().any(|branch| {
                branch["required"] == json!(["session_id", "operation", "appshot_id", "text"])
            }) && selectors.iter().any(|branch| {
                branch["required"] == json!(["device_id", "operation", "appshot_id", "text"])
            })
        }) && type_text["additionalProperties"] == json!(false),
        "phone_keyboard type_text branch must require text and a typed selector"
    );

    let phone_notification_schema = &tool("phone_notification_action")["inputSchema"];
    assert_eq!(
        phone_notification_schema["properties"]["event_id"]["minLength"], 1,
        "phone_notification_action must reject empty event ids"
    );
    assert_eq!(
        phone_notification_schema["properties"]["action_id"]["minLength"], 1,
        "phone_notification_action must reject empty action ids"
    );
    let notification_action = exact_branch(phone_notification_schema, "operation", "action");
    assert!(
        notification_action["oneOf"]
            .as_array()
            .is_some_and(|selectors| {
                selectors.iter().any(|branch| {
                    branch["required"]
                        == json!([
                            "session_id",
                            "operation",
                            "appshot_id",
                            "event_id",
                            "action_id"
                        ])
                }) && selectors.iter().any(|branch| {
                    branch["required"]
                        == json!([
                            "device_id",
                            "operation",
                            "appshot_id",
                            "event_id",
                            "action_id"
                        ])
                })
            })
            && notification_action["additionalProperties"] == json!(false),
        "phone_notification_action action branch must require event fields and a typed selector"
    );

    let phone_install_schema = &tool("phone_app_install")["inputSchema"];
    assert_eq!(
        phone_install_schema["properties"]["apk_paths"]["minItems"], 1,
        "phone_app_install apk_paths must be non-empty"
    );
    assert!(
        phone_install_schema["allOf"]
            .as_array()
            .and_then(|all_of| all_of.first())
            .and_then(|branch| branch["oneOf"].as_array())
            .is_some_and(|selectors| {
                selectors.iter().any(|branch| {
                    branch["required"] == json!(["session_id", "appshot_id", "apk_paths"])
                }) && selectors.iter().any(|branch| {
                    branch["required"] == json!(["device_id", "appshot_id", "apk_paths"])
                })
            }),
        "phone_app_install must require apk_paths and a typed selector"
    );
    assert!(
        phone_install_schema["properties"].get("apk_path").is_none(),
        "phone_app_install apk_path alias must stay removed"
    );
    assert!(
        phone_install_schema["properties"]["grant_runtime_permissions"].is_object(),
        "phone_app_install must expose all supported handler options"
    );
    assert_eq!(
        tool("phone_notification_reply")["inputSchema"]["properties"]["text"]["minLength"],
        1,
        "phone_notification_reply must reject empty reply text"
    );
    let phone_app_action_schema = &tool("phone_app_action")["inputSchema"];
    assert!(
        !schema_accepts(
            phone_app_action_schema,
            &json!({"operation": "launch", "session_id": "phone-1", "package_name": ""})
        ),
        "phone_app_action launch must reject empty package names"
    );
    assert!(
        schema_accepts(
            phone_app_action_schema,
            &json!({"operation": "open_intent", "session_id": "phone-1", "intent_uri": "intent://example", "package_name": ""})
        ),
        "phone_app_action open_intent must allow blank package sentinels"
    );
    assert_eq!(
        phone_app_action_schema["properties"]["intent_uri"]["minLength"], 1,
        "phone_app_action open_intent must reject empty intent URIs"
    );
    assert_eq!(
        tool("browser_eval")["inputSchema"]["properties"]["expression"]["minLength"],
        1,
        "browser_eval must reject empty expressions"
    );
}

#[test]
fn mcp_runtime_config_invalid_values_fallback() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    unsafe {
        std::env::set_var("SKY_CUA_BROWSER_EVAL", "perhaps");
        std::env::set_var("SKY_CUA_MODEL_SUPPORTS_IMAGES", "sometimes");
        std::env::set_var("SKY_CUA_CONFIG_PATH", "");
        std::env::remove_var("SKY_CUA_SURFACES");
        std::env::remove_var("SKY_CUA_PHONE");
    }
    let config = mcp_process_config_from_env().expect("runtime config");
    unsafe {
        std::env::remove_var("SKY_CUA_BROWSER_EVAL");
        std::env::remove_var("SKY_CUA_MODEL_SUPPORTS_IMAGES");
        std::env::remove_var("SKY_CUA_CONFIG_PATH");
    }
    // An invalid value is reported and falls back to the enabled default.
    assert!(config.browser_eval_enabled);
    assert_eq!(config.model_supports_images_override, None);
    assert_eq!(
        config.diagnostics,
        vec![
            McpConfigDiagnostic::InvalidBrowserEval {
                value: "perhaps".to_string()
            },
            McpConfigDiagnostic::InvalidModelSupportsImages {
                value: "sometimes".to_string()
            }
        ]
    );
}

#[test]
fn browser_eval_runtime_config_matches_service_falsy_values() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    unsafe {
        std::env::set_var("SKY_CUA_BROWSER_EVAL", "off");
        std::env::remove_var("SKY_CUA_MODEL_SUPPORTS_IMAGES");
        std::env::set_var("SKY_CUA_CONFIG_PATH", "");
        std::env::remove_var("SKY_CUA_SURFACES");
        std::env::remove_var("SKY_CUA_PHONE");
    }
    let config = mcp_process_config_from_env().expect("runtime config");
    unsafe {
        std::env::remove_var("SKY_CUA_BROWSER_EVAL");
        std::env::remove_var("SKY_CUA_CONFIG_PATH");
    }
    assert!(
        !config.browser_eval_enabled,
        "browser eval advertisement must use the same off/0/false disabling values as service execution"
    );
    assert!(config.diagnostics.is_empty());
}

#[test]
fn mcp_tool_surface_matrix_fixture_matches_generated_registry() {
    assert_fixture_matches(
        "mcp_tool_surface_matrix.json",
        include_str!("../../../tests/fixtures/mcp_tool_surface_matrix.json"),
        generated_surface_matrix(),
    );
}

#[test]
fn mcp_surface_policy_matrix_fixture_matches_generated_registry() {
    assert_fixture_matches(
        "mcp_surface_policy_matrix.json",
        include_str!("../../../tests/fixtures/mcp_surface_policy_matrix.json"),
        generated_surface_policy_matrix(),
    );
}

#[test]
fn advertised_schemas_only_use_runtime_supported_keywords() {
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    for tool in registry.tools.as_array().expect("tools array") {
        let name = tool["name"].as_str().expect("tool name");
        assert_schema_keywords_supported(&tool["inputSchema"], name);
    }
}

#[test]
fn advertised_schema_is_validation_schema_minus_root_composition() {
    // The advertised schema must differ from the rich validation schema by
    // exactly the four root composition keywords the Anthropic API rejects —
    // nothing else. This guards against a future change to
    // `flatten_advertised_input_schema` silently dropping a property or
    // `required` entry that runtime enforcement still depends on, which would
    // desync what the model sees from what `validate_arguments` checks.
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    for tool in registry.tools.as_array().expect("tools") {
        let name = tool["name"].as_str().expect("tool name");
        let advertised = tool["inputSchema"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} advertised inputSchema must be an object"));
        let mut expected = registry
            .validation_schemas
            .get(name)
            .unwrap_or_else(|| panic!("{name} missing validation schema"))
            .as_object()
            .expect("validation schema must be an object")
            .clone();
        for key in ["allOf", "oneOf", "anyOf", "not"] {
            expected.remove(key);
        }
        assert_eq!(
            advertised, &expected,
            "advertised schema for {name} must equal its validation schema minus root \
             composition keywords; flattening dropped or changed something else"
        );
    }
}

#[test]
fn advertised_schemas_have_no_top_level_composition() {
    // The Anthropic Messages API rejects top-level `allOf`/`oneOf`/`anyOf` in
    // a tool `input_schema` ("input_schema does not support oneOf, allOf, or
    // anyOf at the top level"), and under Claude Code's deferred loading the
    // tool then silently never surfaces. `not` is undocumented and treated as
    // unsafe. Validation richness lives in `validation_schemas`, not here.
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    for tool in registry.tools.as_array().expect("tools") {
        let name = tool["name"].as_str().expect("tool name");
        let schema = tool["inputSchema"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} inputSchema must be an object"));
        for key in ["allOf", "oneOf", "anyOf", "not"] {
            assert!(
                !schema.contains_key(key),
                "advertised inputSchema for {name} carries top-level {key}; the Anthropic \
                 Messages API rejects it and Claude Code drops the tool. Keep the constraint \
                 in the validation schema instead."
            );
        }
    }
}

#[test]
fn validation_schemas_still_reject_vague_grouped_calls() {
    // Flattening the advertised schema must not weaken runtime enforcement:
    // `validate_arguments` uses the rich validation schema, so a bare grouped
    // call with only a discriminator is still rejected.
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    assert!(
        registry
            .validate_arguments("desktop_pointer", &json!({"operation": "click"}))
            .is_err(),
        "a click with no coordinates or selector must be rejected at dispatch"
    );
    assert!(
        registry
            .validate_arguments(
                "desktop_pointer",
                &json!({"operation": "click", "x": 10, "y": 20, "appshot_id": "appshot-1"})
            )
            .is_ok(),
        "a click with coordinates must be accepted"
    );
    assert!(
        registry
            .validate_arguments(
                "capture_desktop",
                &json!({"window_id": "w1", "display_index": 0})
            )
            .is_err(),
        "capture_desktop must still reject mixing a window target with a display target"
    );
}

#[test]
fn phone_validation_accepts_device_id_only_selectors() {
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    let cases = [
        (
            "phone_connection",
            json!({"operation": "disconnect", "device_id": "d1"}),
        ),
        (
            "phone_pointer",
            json!({"operation": "tap", "device_id": "d1", "appshot_id": "a1", "x": 1, "y": 2, "use_device_coordinates": true}),
        ),
        (
            "phone_keyboard",
            json!({"operation": "type_text", "device_id": "d1", "appshot_id": "a1", "text": "hello"}),
        ),
        (
            "phone_notification_action",
            json!({"operation": "open", "device_id": "d1", "appshot_id": "a1", "event_id": "e1"}),
        ),
        (
            "phone_app_action",
            json!({"operation": "launch", "device_id": "d1", "package_name": "com.example.app"}),
        ),
        (
            "phone_setup",
            json!({"operation": "install_companion", "device_id": "d1"}),
        ),
        (
            "phone_app_install",
            json!({"device_id": "d1", "appshot_id": "a1", "apk_paths": ["/tmp/app.apk"]}),
        ),
    ];
    for (tool, arguments) in cases {
        assert!(
            registry.validate_arguments(tool, &arguments).is_ok(),
            "device_id-only call must validate for {tool}: {arguments}"
        );
    }
}

#[test]
fn list_resources_browser_tabs_accepts_limit() {
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    assert!(
        registry
            .validate_arguments(
                "list_resources",
                &json!({
                    "surface": "browser",
                    "resource": "tabs",
                    "target": "user_chrome",
                    "limit": 20
                })
            )
            .is_ok(),
        "browser tabs listing must accept a top-level limit"
    );
}

#[test]
fn observe_browser_branch_rejects_capture_fields() {
    // Canonical observe produces the AppShot image itself; legacy screenshot
    // controls are not accepted on any surface branch.
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    assert!(
        registry
            .validate_arguments("observe", &json!({"surface": "browser", "tab_id": "tab-1"}))
            .is_ok(),
        "a plain browser observe must be accepted"
    );
    assert!(
        registry
            .validate_arguments(
                "observe",
                &json!({
                    "surface": "browser",
                    "tab_id": "tab-1",
                    "capture_screen": "if_changed"
                })
            )
            .is_err(),
        "the browser observe branch must reject capture_screen"
    );
    assert!(
        registry
            .validate_arguments(
                "observe",
                &json!({
                    "surface": "browser",
                    "tab_id": "tab-1",
                    "screenshot_delivery": "path"
                })
            )
            .is_err(),
        "the browser observe branch must reject screenshot_delivery"
    );
    assert!(
        registry
            .validate_arguments(
                "observe",
                &json!({"surface": "desktop", "capture_screen": "always"})
            )
            .is_err(),
        "desktop observe must not advertise ignored legacy capture controls"
    );
    let rejection = registry
        .validate_arguments(
            "observe",
            &json!({"surface": "browser", "tab_id": "tab-1", "detail": "full"}),
        )
        .expect_err("browser detail must be rejected");
    assert!(
        rejection.contains("capture_timeout_ms"),
        "browser observe repair guidance must mention every accepted branch field: {rejection}"
    );
}

#[test]
fn runtime_schema_validator_fails_closed_for_unsupported_schema_surface() {
    assert!(!schema_accepts(
        &json!({"type": "url"}),
        &json!("https://example.test/")
    ));
    assert!(!schema_accepts(
        &json!({"type": {"name": "string"}}),
        &json!("hello")
    ));
    assert!(!schema_accepts(
        &json!({"type": "string", "format": "uri"}),
        &json!("https://example.test/")
    ));
    assert!(schema_accepts(
        &json!({"type": "string", "pattern": ".*\\S.*"}),
        &json!(" window ")
    ));
    assert!(!schema_accepts(
        &json!({"type": "string", "pattern": ".*\\S.*"}),
        &json!("   ")
    ));
    assert!(!schema_accepts(
        &json!({"type": "string", "pattern": "^custom$"}),
        &json!("custom")
    ));
}

#[test]
fn desktop_mutation_schemas_require_appshot_id() {
    let tools = build_tool_definitions(false, false);
    let tools = tools.as_array().expect("tool definitions array");
    for name in [
        "desktop_semantic",
        "desktop_toggle",
        "desktop_scroll",
        "desktop_pointer",
        "desktop_keyboard",
        "desktop_action",
        "desktop_set_value",
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert!(
            tool["inputSchema"]["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|field| field == "appshot_id")),
            "{name} must require appshot_id"
        );
    }
}

#[test]
fn phone_camera_schema_enforces_v1_capture_resolution_bound() {
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    let base = json!({
        "operation": "photo",
        "session_id": "phone-1",
        "appshot_id": "shot-1",
        "camera_id": "0"
    });
    let mut portrait = base.clone();
    portrait["options"] = json!({"size": {"width": 1080, "height": 1920}});
    assert!(
        registry
            .validate_arguments("phone_camera", &portrait)
            .is_ok()
    );
    let mut too_large = base;
    too_large["options"] = json!({"size": {"width": 3840, "height": 2160}});
    assert!(
        registry
            .validate_arguments("phone_camera", &too_large)
            .is_err()
    );
}

#[test]
fn phone_camera_schema_separates_phone_and_camera_session_ids() {
    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    let valid = json!({
        "operation": "preview_stop",
        "session_id": "phone-session-1",
        "appshot_id": "shot-1",
        "camera_session_id": "camera-session-1"
    });
    assert!(registry.validate_arguments("phone_camera", &valid).is_ok());

    let ambiguous = json!({
        "operation": "preview_stop",
        "session_id": "phone-session-1",
        "appshot_id": "shot-1"
    });
    assert!(
        registry
            .validate_arguments("phone_camera", &ambiguous)
            .is_err()
    );

    let by_device = json!({
        "operation": "preview_frame",
        "device_id": "device-1",
        "camera_session_id": "camera-session-1"
    });
    assert!(
        registry
            .validate_arguments("phone_camera", &by_device)
            .is_ok()
    );
}

#[test]
fn tool_contract_fixture_matches_generated_registry() {
    let generated = generated_tool_contract();
    let tools = generated["tools"].as_array().expect("contract tools");
    let contract_names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("contract tool name"))
        .collect();
    let registry = build_tool_registry(
        &process_config(true),
        &ModelSessionInfo {
            supports_images: Some(true),
        },
    );
    let advertised_names: Vec<&str> = registry
        .tools
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert_eq!(contract_names, advertised_names);

    assert_fixture_matches(
        "tool_contract.json",
        include_str!("../../../tests/fixtures/tool_contract.json"),
        generated,
    );
}

#[test]
fn call_cases_fixture_matches_contract() {
    let generated = generated_call_cases();
    let cases = generated["cases"].as_array().expect("call cases");
    assert!(
        cases.iter().all(|case| case["valid"].is_object()),
        "every valid call case must be an object"
    );
    assert!(
        cases.iter().all(|case| case["invalid"].is_object()),
        "every invalid call case must be an object"
    );
    let contract_branch_count: usize = generated_tool_contract()["tools"]
        .as_array()
        .expect("contract tools")
        .iter()
        .map(|tool| tool["branches"].as_array().expect("branches").len())
        .sum();
    assert_eq!(cases.len(), contract_branch_count);

    let registry = build_tool_registry(&process_config(true), &ModelSessionInfo::default());
    for case in cases {
        let tool_name = case["tool"].as_str().expect("case tool");
        // Validate against the rich validation schema (what `validate_arguments`
        // enforces), not the flattened advertised schema, which by design no
        // longer carries the per-branch constraints that reject bad calls.
        let schema = registry
            .validation_schemas
            .get(tool_name)
            .unwrap_or_else(|| panic!("missing schema for {tool_name}"));
        assert!(
            schema_accepts(schema, &case["valid"]),
            "call case {}/{} is not valid for its generated schema: {}",
            tool_name,
            case["branch"].as_str().expect("case branch"),
            case["valid"]
        );
        assert!(
            !schema_accepts(schema, &case["invalid"]),
            "call case {}/{} invalid sample was accepted by its generated schema: {}",
            tool_name,
            case["branch"].as_str().expect("case branch"),
            case["invalid"]
        );
    }

    assert_fixture_matches(
        "call_cases.json",
        include_str!("../../../tests/fixtures/call_cases.json"),
        generated,
    );
}

#[test]
fn call_cases_match_grouped_dispatcher() {
    for case in generated_call_cases()["cases"]
        .as_array()
        .expect("call cases")
    {
        let tool_name = case["tool"].as_str().expect("case tool");
        let expected_branch = case["branch"].as_str().expect("case branch");
        let expected_handler = case["handler_id"].as_str().expect("case handler");
        let call = crate::mcp_tools::grouped_handler_call(tool_name, case["valid"].clone())
            .unwrap_or_else(|error| {
                panic!("call case {tool_name}/{expected_branch} did not dispatch: {error}")
            });
        assert_eq!(call.branch, expected_branch, "{tool_name} branch");
        assert_eq!(
            call.handler_name, expected_handler,
            "{tool_name}/{expected_branch} handler"
        );
    }
}

fn assert_fixture_matches(name: &str, expected: &str, generated: Value) {
    let generated_text = serde_json::to_string_pretty(&generated).expect("generated fixture json");
    if std::env::var_os("SKY_CUA_UPDATE_MCP_FIXTURES").is_some() {
        let path = fixture_path(name);
        fs::write(&path, format!("{generated_text}\n"))
            .unwrap_or_else(|error| panic!("failed to update {}: {error}", path.display()));
        return;
    }
    let expected: Value = serde_json::from_str(expected)
        .unwrap_or_else(|error| panic!("{name} fixture is invalid json: {error}"));
    let generated: Value =
        serde_json::from_str(&generated_text).expect("generated fixture should parse");
    assert_eq!(
        expected, generated,
        "{name} is stale; rerun with SKY_CUA_UPDATE_MCP_FIXTURES=1"
    );
}

fn assert_schema_keywords_supported(schema: &Value, context: &str) {
    let Some(schema) = schema.as_object() else {
        return;
    };
    for keyword in schema.keys() {
        assert!(
            schema_keyword_is_supported(keyword),
            "{context} inputSchema uses unsupported validation keyword {keyword}"
        );
    }
    if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
        assert!(
            schema_pattern_is_supported(pattern),
            "{context} inputSchema uses unsupported runtime pattern {pattern}"
        );
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, property_schema) in properties {
            assert_schema_keywords_supported(property_schema, &format!("{context}.{name}"));
        }
    }
    if let Some(items) = schema.get("items") {
        assert_schema_keywords_supported(items, &format!("{context}[]"));
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(children) = schema.get(key).and_then(Value::as_array) {
            for (index, child) in children.iter().enumerate() {
                assert_schema_keywords_supported(child, &format!("{context}.{key}[{index}]"));
            }
        }
    }
    for key in ["not", "if", "then"] {
        if let Some(child) = schema.get(key) {
            assert_schema_keywords_supported(child, &format!("{context}.{key}"));
        }
    }
}

fn schema_pattern_is_supported(pattern: &str) -> bool {
    matches!(pattern, "^(https?://[^\\s]+|about:blank)$" | ".*\\S.*")
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}
