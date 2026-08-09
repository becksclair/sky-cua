//! Tests for the Phone Use MCP tool surface. Mirrors `browser_tests.rs`: a
//! `FakeService` records requests and replays scripted responses so each tool's
//! arg->request mapping and response->result shaping can be asserted without a
//! live service.

use std::cell::RefCell;
use std::collections::VecDeque;

use serde_json::{Value, json};
use sky_cua_platform::model::{
    AppShotActionSnapshot, AppShotCapture, AppShotConsistency, AppShotCoverage, AppShotEnvelope,
    AppShotTrigger, ContentPersistence, ContentRef, ContentSource, CoordinateSpace,
    DiagnosticEntry, PhoneAccessibilityNode, PhoneAccessibilityTreeResponse, PhoneActionResponse,
    PhoneAppInfo, PhoneAppInstallMode, PhoneAppResponse, PhoneAppResponseKind,
    PhoneBackendCapabilities, PhoneBackendKind, PhoneCallerProvenance, PhoneCapabilityProfile,
    PhoneCapabilityRefreshState, PhoneCompanionCapabilities, PhoneConnectionIdentity,
    PhoneConnectionKind, PhoneCoordinateMapping, PhoneImage, PhoneListDevicesResponse,
    PhoneMcpClientInfo, PhoneNotificationsResponse, PhoneObserveResponse, PhonePairWirelessRequest,
    PhoneRequest, PhoneRequestContext, PhoneResponse, PhoneScrcpyCapabilities,
    PhoneScreenshotResponse, PhoneSession, PhoneStatusReport, PhoneTargetDeviceKind, PixelSize,
    RectF, ServiceRequest, ServiceResponse,
};

use crate::heuristics::HeuristicsRegistry;
use crate::mcp_server::{ModelSessionInfo, with_phone_request_context};

use super::{build_tool_definitions, handle_tool_call, validation_tool_definitions};

const PHONE_TOOL_NAMES: &[&str] = &[
    "status",
    "list_resources",
    "observe",
    "capture_screen",
    "phone_accessibility_tree",
    "phone_notifications",
    "phone_connection",
    "phone_pair_wireless",
    "phone_setup",
    "phone_pointer",
    "phone_keyboard",
    "phone_notification_action",
    "phone_notification_reply",
    "phone_app_action",
    "phone_app_force_stop",
    "phone_app_install",
];

#[derive(Default)]
struct FakeService {
    requests: RefCell<Vec<ServiceRequest>>,
    responses: RefCell<VecDeque<ServiceResponse>>,
}

impl FakeService {
    fn with_response(response: ServiceResponse) -> Self {
        Self {
            requests: RefCell::new(Vec::new()),
            responses: RefCell::new(VecDeque::from([response])),
        }
    }

    fn take_requests(&self) -> Vec<ServiceRequest> {
        self.requests.take()
    }
}

impl super::McpService for FakeService {
    fn call(&self, request: &ServiceRequest) -> anyhow::Result<ServiceResponse> {
        self.requests.borrow_mut().push(request.clone());
        self.responses
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("fake service response queue exhausted"))
    }
}

macro_rules! phone_service_response {
    ($variant:ident ( $inner:expr )) => {
        ServiceResponse::Phone {
            response: PhoneResponse::$variant($inner),
        }
    };
}

fn heuristics() -> HeuristicsRegistry {
    HeuristicsRegistry::load_from_repo().expect("heuristics load")
}

fn image_model() -> ModelSessionInfo {
    ModelSessionInfo {
        supports_images: Some(true),
    }
}

fn text_only_model() -> ModelSessionInfo {
    ModelSessionInfo {
        supports_images: Some(false),
    }
}

fn call(
    service: &FakeService,
    model: &ModelSessionInfo,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    handle_tool_call(service, &heuristics(), model, name, arguments).expect("phone tool call")
}

fn tool_names(tools: &serde_json::Value) -> Vec<String> {
    tools
        .as_array()
        .expect("tools should be an array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name").to_string())
        .collect()
}

fn find_tool<'a>(tools: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    tools
        .as_array()
        .expect("tools should be an array")
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("{name} tool is advertised"))
}

fn status_report() -> PhoneStatusReport {
    PhoneStatusReport {
        enabled: true,
        adb_available: false,
        adb_path: None,
        adb_version: None,
        adb_server_running: None,
        scrcpy_available: false,
        scrcpy_path: None,
        scrcpy_version: None,
        companion_enabled: true,
        mdns_available: false,
        default_serial: None,
        default_backend: PhoneBackendKind::Auto,
        sessions: Vec::new(),
        devices: Vec::new(),
        diagnostics: vec![DiagnosticEntry {
            code: "PhoneAdbNotImplemented".to_string(),
            message: "adb probing is not implemented yet".to_string(),
            details: None,
        }],
    }
}

// ---------------------------------------------------------------------------
// advertisement + schema
// ---------------------------------------------------------------------------

#[test]
fn all_phone_tools_are_advertised() {
    let definitions = build_tool_definitions(false, false);
    let names = tool_names(&definitions);
    for expected in PHONE_TOOL_NAMES {
        assert!(
            names.iter().any(|name| name == expected),
            "phone tool {expected} must be advertised"
        );
    }
    assert_eq!(
        PHONE_TOOL_NAMES.len(),
        16,
        "exactly 16 grouped phone-capable tools"
    );
}

#[test]
fn phone_tools_carry_session_selector_and_strict_schema() {
    let definitions = build_tool_definitions(true, false);
    // Every post-connect device-bound tool exposes session_id, rejects raw
    // serial selectors, and forbids unknown properties.
    for name in [
        "phone_accessibility_tree",
        "phone_notifications",
        "phone_pointer",
        "phone_keyboard",
        "phone_setup",
        "phone_app_action",
    ] {
        let tool = find_tool(&definitions, name);
        let schema = &tool["inputSchema"];
        assert_eq!(
            schema["additionalProperties"], false,
            "{name} must reject unknown properties"
        );
        assert!(
            schema["properties"].get("session_id").is_some(),
            "{name} exposes session_id"
        );
        assert!(
            schema["properties"].get("serial").is_none(),
            "{name} rejects serial"
        );
    }
}

#[test]
fn phone_action_schemas_pin_required_fields() {
    // Per-branch required-field constraints live in the validation schema now.
    let definitions = validation_tool_definitions(false, false);
    let operation_branch = |schema: &Value, operation: &str| -> Value {
        schema["allOf"]
            .as_array()
            .and_then(|all_of| {
                all_of.iter().find_map(|constraint| {
                    constraint["oneOf"].as_array().and_then(|one_of| {
                        one_of
                            .iter()
                            .find(|branch| branch["properties"]["operation"]["const"] == operation)
                    })
                })
            })
            .unwrap_or_else(|| panic!("missing operation={operation} branch"))
            .clone()
    };

    let pointer = find_tool(&definitions, "phone_pointer");
    assert_eq!(
        pointer["inputSchema"]["required"],
        json!(["operation", "appshot_id"])
    );
    assert!(
        pointer["inputSchema"]["properties"]
            .get("phone_snapshot_id")
            .is_some()
    );

    let tap_branch = operation_branch(&pointer["inputSchema"], "tap");
    assert!(tap_branch["required"].is_null());
    assert!(tap_branch["oneOf"].as_array().is_some_and(|selectors| {
        selectors.iter().any(|branch| {
            branch["required"] == json!(["session_id", "operation", "appshot_id", "x", "y"])
        }) && selectors.iter().any(|branch| {
            branch["required"] == json!(["device_id", "operation", "appshot_id", "x", "y"])
        })
    }));
    assert_eq!(tap_branch["additionalProperties"], json!(false));
    assert!(
        tap_branch["anyOf"].as_array().is_some_and(|any_of| {
            any_of
                .iter()
                .any(|constraint| constraint["required"] == json!(["phone_snapshot_id"]))
                && any_of.iter().any(|constraint| {
                    constraint["properties"]["use_device_coordinates"]["const"] == true
                        && constraint["required"] == json!(["use_device_coordinates"])
                })
        }),
        "phone_pointer must require snapshot provenance or raw device coordinates"
    );

    let keyboard = find_tool(&definitions, "phone_keyboard");
    assert_eq!(
        keyboard["inputSchema"]["required"],
        json!(["operation", "appshot_id"])
    );
    let type_text_branch = operation_branch(&keyboard["inputSchema"], "type_text");
    assert!(
        type_text_branch["oneOf"]
            .as_array()
            .is_some_and(|selectors| {
                selectors.iter().any(|branch| {
                    branch["required"] == json!(["session_id", "operation", "appshot_id", "text"])
                }) && selectors.iter().any(|branch| {
                    branch["required"] == json!(["device_id", "operation", "appshot_id", "text"])
                })
            })
    );
    assert_eq!(type_text_branch["additionalProperties"], json!(false));

    let pair = find_tool(&definitions, "phone_pair_wireless");
    assert_eq!(
        pair["inputSchema"]["required"],
        json!(["host_port", "pairing_code"])
    );
    // The pairing tool never advertises a session selector — pairing precedes a
    // session.
    assert!(
        pair["inputSchema"]["properties"]
            .get("session_id")
            .is_none()
    );

    let install = find_tool(&definitions, "phone_app_install");
    assert!(
        install["inputSchema"]["allOf"]
            .as_array()
            .and_then(|all_of| all_of.first())
            .and_then(|branch| branch["oneOf"].as_array())
            .is_some_and(|selectors| {
                selectors.iter().any(|branch| {
                    branch["required"] == json!(["session_id", "appshot_id", "apk_paths"])
                }) && selectors.iter().any(|branch| {
                    branch["required"] == json!(["device_id", "appshot_id", "apk_paths"])
                })
            })
    );
    assert!(
        install["inputSchema"]["properties"]
            .get("apk_path")
            .is_none(),
        "phone_app_install no longer advertises apk_path"
    );
    assert_eq!(
        install["inputSchema"]["properties"]["mode"]["anyOf"][0]["enum"],
        json!(["single", "multiple", "multi_package"])
    );
    assert_eq!(install["annotations"]["destructiveHint"], true);
    assert_eq!(install["annotations"]["idempotentHint"], false);

    let setup = find_tool(&definitions, "phone_setup");
    assert_eq!(setup["inputSchema"]["required"], json!(["operation"]));
}

#[test]
fn grouped_phone_feature_schemas_require_appshots_only_for_mutations() {
    fn mutation_conditional(schema: &serde_json::Value) -> Option<&serde_json::Value> {
        if schema
            .pointer("/if/properties/operation/not/enum")
            .is_some()
        {
            return Some(schema);
        }
        schema
            .get("allOf")?
            .as_array()?
            .iter()
            .find_map(mutation_conditional)
    }

    let definitions = validation_tool_definitions(false, false);
    for (name, reads) in [
        ("phone_content", vec!["describe"]),
        ("phone_clipboard", vec!["get", "changes"]),
        ("phone_editor", vec!["context"]),
        (
            "phone_camera",
            vec!["enumerate", "capabilities", "preview_frame"],
        ),
        (
            "phone_storage",
            vec![
                "roots",
                "list",
                "stat",
                "read",
                "hash",
                "search",
                "thumbnail",
                "metadata",
                "list_saf_roots",
            ],
        ),
    ] {
        let schema = &find_tool(&definitions, name)["inputSchema"];
        let conditional =
            mutation_conditional(schema).unwrap_or_else(|| panic!("{name} mutation conditional"));
        assert_eq!(
            conditional["if"]["properties"]["operation"]["not"]["enum"],
            json!(reads)
        );
        assert_eq!(conditional["then"]["required"], json!(["appshot_id"]));
    }
}

#[test]
fn grouped_phone_mutation_without_appshot_fails_before_service_dispatch() {
    let service = FakeService::default();
    let result = handle_tool_call(
        &service,
        &heuristics(),
        &image_model(),
        "phone_clipboard",
        json!({
            "session_id": "sess-1",
            "operation": "set",
            "payload": {"items": [{"text": "hello"}], "sensitive": false}
        }),
    )
    .expect("invalid request is returned as a structured MCP result");
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "InvalidRequest");
    assert!(
        result["structuredContent"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("appshot_id is required"))
    );
    assert!(service.take_requests().is_empty());
}

// ---------------------------------------------------------------------------
// request mapping + response shaping
// ---------------------------------------------------------------------------

#[test]
fn phone_status_maps_request_and_summarizes_report() {
    let service = FakeService::with_response(phone_service_response!(Status(status_report())));
    let result = call(&service, &image_model(), "phone_status", json!({}));

    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("Phone Use tools are enabled"));
    assert!(text.contains("adb=unavailable"));
    assert_eq!(result["structuredContent"]["enabled"], true);

    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Phone {
            request: PhoneRequest::Status(sky_cua_platform::model::PhoneStatusRequest {
                refresh_devices: false,
            }),
            context: None,
        }
    );
}

#[test]
fn phone_status_timeout_diagnostic_is_error() {
    let mut report = status_report();
    report.diagnostics = vec![DiagnosticEntry {
        code: "PhoneCommandTimedOut".to_string(),
        message: "adb timed out".to_string(),
        details: None,
    }];
    let service = FakeService::with_response(phone_service_response!(Status(report)));
    let result = call(&service, &image_model(), "phone_status", json!({}));

    assert_eq!(result["isError"], true);
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Phone {
            request: PhoneRequest::Status(sky_cua_platform::model::PhoneStatusRequest {
                refresh_devices: false,
            }),
            context: None,
        }
    );
}

#[test]
fn phone_service_request_propagates_scoped_context() {
    let service = FakeService::with_response(phone_service_response!(Status(status_report())));
    let context = PhoneRequestContext {
        session_id: Some("opencode-session".to_string()),
        turn_id: Some("opencode-turn".to_string()),
        caller_provenance: Some(PhoneCallerProvenance::OpenCode),
        identity_synthetic: Some(true),
        client_info: Some(PhoneMcpClientInfo {
            name: "opencode".to_string(),
            version: "3.0".to_string(),
            title: None,
        }),
    };

    with_phone_request_context(context.clone(), || {
        call(&service, &image_model(), "phone_status", json!({}));
    });

    match service.take_requests().remove(0) {
        ServiceRequest::Phone {
            request: PhoneRequest::Status(_),
            context: Some(recorded),
        } => assert_eq!(recorded, context),
        other => panic!("expected contextual phone status request, got {other:?}"),
    }
}

#[test]
fn phone_observe_requests_image_data_only_for_image_models() {
    let service = FakeService::with_response(phone_service_response!(Status(status_report())));
    call(
        &service,
        &text_only_model(),
        "phone_observe",
        json!({"serial": "ABC123", "include_accessibility": true}),
    );
    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::Observe(request),
            ..
        } => {
            assert_eq!(request.session.serial.as_deref(), Some("ABC123"));
            assert!(request.include_accessibility);
            assert!(
                !request.include_image_data,
                "text-only model must not request image data"
            );
        }
        other => panic!("expected observe request, got {other:?}"),
    }
}

#[test]
fn phone_list_devices_maps_include_mdns() {
    let response = PhoneListDevicesResponse {
        devices: Vec::new(),
        adb_path: None,
        adb_version: None,
        diagnostics: vec![DiagnosticEntry {
            code: "PhoneAdbNotImplemented".to_string(),
            message: "not implemented".to_string(),
            details: None,
        }],
    };
    let service = FakeService::with_response(phone_service_response!(Devices(response)));
    let result = call(
        &service,
        &image_model(),
        "phone_list_devices",
        json!({"include_mdns": true}),
    );

    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("Discovered 0 Android devices")
    );
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Phone {
            request: PhoneRequest::ListDevices(sky_cua_platform::model::PhoneListDevicesRequest {
                include_mdns: true,
            }),
            context: None,
        }
    );
}

#[test]
fn phone_pair_wireless_maps_code_but_never_echoes_it() {
    let service = FakeService::with_response(phone_service_response!(PairedWireless(
        sky_cua_platform::model::PhonePairWirelessResponse {
            paired: false,
            host_port: "192.168.1.5:37000".to_string(),
            serial: None,
            diagnostics: vec![DiagnosticEntry {
                code: "PhoneNotImplemented".to_string(),
                message: "pairing not implemented".to_string(),
                details: None,
            }],
        }
    )));
    let result = call(
        &service,
        &image_model(),
        "phone_pair_wireless",
        json!({"host_port": "192.168.1.5:37000", "pairing_code": "424242"}),
    );

    assert_eq!(result["isError"], true);
    let serialized = serde_json::to_string(&result).expect("serialize result");
    assert!(
        !serialized.contains("424242"),
        "pairing code must never appear in the tool result"
    );
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Phone {
            request: PhoneRequest::PairWireless(PhonePairWirelessRequest {
                host_port: "192.168.1.5:37000".to_string(),
                pairing_code: "424242".to_string(),
            }),
            context: None,
        }
    );
}

#[test]
fn phone_tap_maps_snapshot_and_coordinates() {
    let service = FakeService::with_response(phone_service_response!(Action(action_response())));
    call(
        &service,
        &image_model(),
        "phone_tap",
        json!({"phone_snapshot_id": "snap-7", "x": 120.0, "y": 240.0}),
    );
    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::Tap(request),
            ..
        } => {
            assert_eq!(request.phone_snapshot_id.as_deref(), Some("snap-7"));
            assert_eq!(request.x, 120.0);
            assert_eq!(request.y, 240.0);
            assert!(!request.use_device_coordinates);
        }
        other => panic!("expected tap request, got {other:?}"),
    }
}

#[test]
fn phone_tap_rejects_missing_coordinates() {
    let service = FakeService::default();
    let result = handle_tool_call(
        &service,
        &heuristics(),
        &image_model(),
        "phone_tap",
        json!({"phone_snapshot_id": "snap-7"}),
    )
    .expect("invalid request is a tool error, not a hard error");
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "InvalidRequest");
    assert!(service.take_requests().is_empty());
}

#[test]
fn phone_app_install_maps_paths_and_mode() {
    let service = FakeService::with_response(phone_service_response!(App(app_response(
        PhoneAppResponseKind::Install
    ))));
    call(
        &service,
        &image_model(),
        "phone_app_install",
        json!({
            "session_id": "phone-1",
            "apk_paths": ["/tmp/base.apk", "/tmp/split.apk"],
            "mode": "multiple",
            "grant_runtime_permissions": true
        }),
    );
    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::AppInstall(request),
            ..
        } => {
            assert_eq!(request.apk_paths, vec!["/tmp/base.apk", "/tmp/split.apk"]);
            assert_eq!(request.mode, PhoneAppInstallMode::Multiple);
            assert!(request.grant_runtime_permissions);
        }
        other => panic!("expected app install request, got {other:?}"),
    }
}

#[test]
fn phone_open_settings_maps_screen_enum() {
    let service = FakeService::with_response(phone_service_response!(App(app_response(
        PhoneAppResponseKind::OpenSettings
    ))));
    call(
        &service,
        &image_model(),
        "phone_open_settings",
        json!({"screen": "accessibility"}),
    );
    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::OpenSettings(request),
            ..
        } => {
            assert_eq!(
                request.screen,
                sky_cua_platform::model::PhoneSettingsScreen::Accessibility
            );
        }
        other => panic!("expected open settings request, got {other:?}"),
    }
}

#[test]
fn phone_tap_rejected_for_stale_snapshot_is_mcp_error() {
    // A coordinate action that never reached a backend (backend = None) did not
    // happen, so the MCP result must be an error even though the rejection code
    // is a snapshot-safety code rather than a not-implemented stub. Without this
    // the agent would read a rejected tap as a success.
    let response = PhoneActionResponse {
        session_id: "sess-1".to_string(),
        serial: "ABC".to_string(),
        action: "phone_tap".to_string(),
        backend: PhoneBackendKind::None,
        capability_profile_id: "profile-1".to_string(),
        profile_refresh_state: PhoneCapabilityRefreshState::Detected,
        phone_snapshot_id: None,
        cursor: None,
        diagnostics: vec![DiagnosticEntry {
            code: "PhoneSnapshotUnknown".to_string(),
            message: "snapshot not found".to_string(),
            details: None,
        }],
    };
    let service = FakeService::with_response(phone_service_response!(Action(response)));
    let result = call(
        &service,
        &image_model(),
        "phone_tap",
        json!({"phone_snapshot_id": "bogus", "x": 10.0, "y": 10.0}),
    );
    assert_eq!(result["isError"], true);
}

#[test]
fn phone_snapshot_orientation_resolution_rejection_codes_flip_error() {
    // The snapshot orientation/resolution mismatch codes must be in the client
    // error allowlist, not merely caught by the `backend == None` backstop.
    // Use a non-None backend so the assertion depends on the allowlist entry
    // alone — this locks the "every emitted code is listed" invariant for the
    // two codes added with the snapshot orientation/resolution rejection.
    for code in [
        "PhoneSnapshotOrientationMismatch",
        "PhoneSnapshotResolutionMismatch",
    ] {
        let response = PhoneActionResponse {
            session_id: "sess-1".to_string(),
            serial: "ABC".to_string(),
            action: "phone_tap".to_string(),
            backend: PhoneBackendKind::Adb,
            capability_profile_id: "profile-1".to_string(),
            profile_refresh_state: PhoneCapabilityRefreshState::Reused,
            phone_snapshot_id: None,
            cursor: None,
            diagnostics: vec![DiagnosticEntry {
                code: code.to_string(),
                message: "snapshot device geometry changed".to_string(),
                details: None,
            }],
        };
        let service = FakeService::with_response(phone_service_response!(Action(response)));
        let result = call(
            &service,
            &image_model(),
            "phone_tap",
            json!({"phone_snapshot_id": "snap-7", "x": 10.0, "y": 10.0}),
        );
        assert_eq!(result["isError"], true, "code {code} must flip isError");
    }
}

#[test]
fn phone_tap_dispatched_to_adb_is_not_error() {
    let mut response = action_response();
    response.backend = PhoneBackendKind::Adb;
    response.diagnostics = Vec::new();
    let service = FakeService::with_response(phone_service_response!(Action(response)));
    let result = call(
        &service,
        &image_model(),
        "phone_tap",
        json!({"phone_snapshot_id": "snap-7", "x": 10.0, "y": 10.0}),
    );
    assert_eq!(result["isError"], false);
}

#[test]
fn phone_notifications_maps_and_flags_error_on_diagnostic() {
    let response = PhoneNotificationsResponse {
        session_id: "sess-1".to_string(),
        serial: "ABC".to_string(),
        backend: PhoneBackendKind::None,
        listener_enabled: false,
        events: Vec::new(),
        truncated: false,
        diagnostics: vec![DiagnosticEntry {
            code: "PhoneNotImplemented".to_string(),
            message: "notifications not implemented".to_string(),
            details: None,
        }],
    };
    let service = FakeService::with_response(phone_service_response!(Notifications(response)));
    let result = call(
        &service,
        &image_model(),
        "phone_notifications",
        json!({"session_id": "sess-1", "limit": 5}),
    );

    assert_eq!(result["isError"], true);
    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::Notifications(request),
            ..
        } => {
            assert_eq!(request.session.session_id.as_deref(), Some("sess-1"));
            assert_eq!(request.limit, Some(5));
        }
        other => panic!("expected notifications request, got {other:?}"),
    }

    let whitespace_service = FakeService::with_response(phone_service_response!(Notifications(
        PhoneNotificationsResponse {
            session_id: " ".to_string(),
            serial: "ABC".to_string(),
            backend: PhoneBackendKind::Adb,
            listener_enabled: true,
            events: Vec::new(),
            truncated: false,
            diagnostics: Vec::new(),
        }
    )));
    let result = call(
        &whitespace_service,
        &image_model(),
        "phone_notifications",
        json!({"session_id": " "}),
    );

    assert_eq!(result["isError"], false);
    match &whitespace_service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::Notifications(request),
            ..
        } => assert_eq!(request.session.session_id.as_deref(), Some(" ")),
        other => panic!("expected notifications request, got {other:?}"),
    }
}

#[test]
fn phone_screenshot_capture_failure_with_no_backend_is_mcp_error() {
    // A capture that never produced a frame routes through `screenshot_failure`,
    // which sets `backend = None` and attaches the classified diagnostic (e.g.
    // `PhoneAdbCommandFailed` when `adb screencap` itself failed). That is the
    // only screenshot error shape: no frame was captured, so `isError` flips and
    // the summary takes the honest could-not-capture branch.
    let mut response = screenshot_response(None);
    response.backend = PhoneBackendKind::None;
    response.phone_snapshot_id = String::new();
    response.diagnostics = vec![DiagnosticEntry {
        code: "PhoneAdbCommandFailed".to_string(),
        message: "`adb exec-out screencap -p` exited with status 1".to_string(),
        details: None,
    }];
    let service = FakeService::with_response(phone_service_response!(Screenshot(response)));
    let result = call(
        &service,
        &image_model(),
        "phone_screenshot",
        json!({"session_id": "sess-1"}),
    );
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().expect("text");
    assert!(
        text.contains("Could not capture"),
        "error branch must produce the honest could-not-capture summary, got: {text}"
    );
}

#[test]
fn phone_screenshot_adb_fallback_after_companion_failure_is_success() {
    // The companion screenshot throttled and the request transparently fell back
    // to ADB, which produced a real frame (`backend = Adb`, a minted snapshot id).
    // The companion-failure diagnostic (`throttled`) rides along as informational
    // context on a *successful* capture: `isError` must stay false, and the
    // diagnostic must survive into structuredContent rather than being dropped.
    let mut response = screenshot_response(Some("aGVsbG8=".to_string()));
    response.backend = PhoneBackendKind::Adb;
    response.diagnostics = vec![DiagnosticEntry {
        code: "throttled".to_string(),
        message: "companion screenshot failed; fell back to adb screencap".to_string(),
        details: None,
    }];
    let service = FakeService::with_response(phone_service_response!(Screenshot(response)));
    let result = call(
        &service,
        &image_model(),
        "phone_screenshot",
        json!({"session_id": "sess-1"}),
    );
    assert_eq!(result["isError"], false);
    let diagnostics = result["structuredContent"]["diagnostics"]
        .as_array()
        .expect("diagnostics array survives on a successful fallback capture");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "throttled"),
        "the informational companion-failure diagnostic must not be dropped, got: {diagnostics:?}"
    );
}

#[test]
fn phone_observe_adb_fallback_after_companion_failure_is_success() {
    // Mirror of the screenshot fallback case for `phone_observe`: a companion
    // failure that fell back to ADB still produced a usable observation, so the
    // throttled diagnostic is informational and `isError` must stay false.
    let response = observe_response(
        PhoneBackendKind::Adb,
        vec![DiagnosticEntry {
            code: "throttled".to_string(),
            message: "companion observe failed; fell back to adb".to_string(),
            details: None,
        }],
    );
    let service = FakeService::with_response(phone_service_response!(Observe(response)));
    let result = call(
        &service,
        &image_model(),
        "phone_observe",
        json!({"session_id": "sess-1"}),
    );
    assert_eq!(result["isError"], false);
}

#[test]
fn phone_observe_for_text_only_model_omits_image_block_and_base64() {
    let mut response = observe_response(PhoneBackendKind::Adb, vec![]);
    response.appshot = Some(Box::new(phone_appshot()));
    response.inline_image = Some(PhoneImage {
        mime_type: "image/png".to_string(),
        data_base64: "aGVsbG8=".to_string(),
        width: Some(1080),
        height: Some(2400),
    });
    let service = FakeService::with_response(phone_service_response!(Observe(response)));

    let result = call(
        &service,
        &text_only_model(),
        "phone_observe",
        json!({"session_id": "sess-1"}),
    );

    assert_eq!(result["isError"], false);
    assert_eq!(result["content"].as_array().expect("content").len(), 1);
    assert_eq!(result["content"][0]["type"], "text");
    let text = result["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("Model-facing phone accessibility projection"));
    assert!(text.contains("\"text\":\"Continue\""));
    assert!(
        result["structuredContent"]["inline_image"]
            .as_object()
            .expect("inline_image object")
            .get("data_base64")
            .is_none(),
        "base64 must not escape through structuredContent"
    );

    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::Observe(request),
            ..
        } => assert!(
            !request.include_image_data,
            "text-only model must not request observe image data"
        ),
        other => panic!("expected observe request, got {other:?}"),
    }
}

#[test]
fn phone_accessibility_tree_includes_nodes_in_model_facing_text() {
    let response = PhoneAccessibilityTreeResponse {
        session_id: "sess-1".into(),
        serial: "ABC".into(),
        backend: PhoneBackendKind::Companion,
        package_name: Some("com.example".into()),
        activity: Some("MainActivity".into()),
        nodes: vec![PhoneAccessibilityNode {
            node_index: 0,
            parent_index: None,
            class_name: Some("android.widget.Button".into()),
            package_name: Some("com.example".into()),
            text: Some("Continue".into()),
            content_description: None,
            bounds: None,
            clickable: true,
            focusable: true,
            enabled: true,
            redacted: false,
        }],
        truncated: false,
        redacted: false,
        diagnostics: Vec::new(),
    };
    let service = FakeService::with_response(phone_service_response!(AccessibilityTree(response)));
    let result = call(
        &service,
        &text_only_model(),
        "phone_accessibility_tree",
        json!({"session_id": "sess-1"}),
    );

    let text = result["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("Model-facing phone accessibility tree"));
    assert!(text.contains("\"text\":\"Continue\""));
    assert!(text.contains("\"clickable\":true"));
}

#[test]
fn phone_observe_for_image_model_attaches_image_block_and_strips_base64() {
    let mut response = observe_response(PhoneBackendKind::Adb, vec![]);
    response.inline_image = Some(PhoneImage {
        mime_type: "image/png".to_string(),
        data_base64: "aGVsbG8=".to_string(),
        width: Some(1080),
        height: Some(2400),
    });
    let service = FakeService::with_response(phone_service_response!(Observe(response)));

    let result = call(
        &service,
        &image_model(),
        "phone_observe",
        json!({"session_id": "sess-1"}),
    );

    assert_eq!(result["isError"], false);
    assert_eq!(result["content"][1]["type"], "image");
    assert_eq!(result["content"][1]["data"], "aGVsbG8=");
    assert_eq!(result["content"][1]["mimeType"], "image/png");
    assert!(
        result["structuredContent"]["inline_image"]
            .as_object()
            .expect("inline_image object")
            .get("data_base64")
            .is_none(),
        "base64 must travel only in the image content block"
    );

    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::Observe(request),
            ..
        } => assert!(request.include_image_data),
        other => panic!("expected observe request, got {other:?}"),
    }
}

#[test]
fn phone_notification_op_failure_with_no_backend_is_mcp_error() {
    // A notification operation that never reached a backend (backend = None,
    // e.g. companion required/unavailable or an op rejection) did not happen and
    // must flip `isError` even when the diagnostic code is not in the allowlist,
    // mirroring `phone_action_result`.
    let response = PhoneNotificationsResponse {
        session_id: "sess-1".to_string(),
        serial: "ABC".to_string(),
        backend: PhoneBackendKind::None,
        listener_enabled: false,
        events: Vec::new(),
        truncated: false,
        diagnostics: vec![DiagnosticEntry {
            code: "PhoneNotificationOpRejected".to_string(),
            message: "companion rejected the notification operation".to_string(),
            details: None,
        }],
    };
    let service = FakeService::with_response(phone_service_response!(Notifications(response)));
    let result = call(
        &service,
        &image_model(),
        "phone_notification_dismiss",
        json!({"session_id": "sess-1", "event_id": "evt-1"}),
    );
    assert_eq!(result["isError"], true);
}

#[test]
fn phone_screenshot_attaches_image_block_and_strips_base64() {
    let service = FakeService::with_response(phone_service_response!(Screenshot(
        screenshot_response(Some("aGVsbG8=".to_string()))
    )));
    let result = call(
        &service,
        &image_model(),
        "phone_screenshot",
        json!({"session_id": "sess-1"}),
    );

    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("1080x2400"));
    assert!(text.contains("snap-1"));
    assert!(text.contains("attached"));

    assert_eq!(result["content"][1]["type"], "image");
    assert_eq!(result["content"][1]["data"], "aGVsbG8=");
    assert_eq!(result["content"][1]["mimeType"], "image/png");

    // The base64 payload must not ride structuredContent.
    let structured = &result["structuredContent"];
    assert!(
        structured["inline_image"]
            .as_object()
            .expect("inline_image object")
            .get("data_base64")
            .is_none(),
        "data_base64 must be stripped from structuredContent"
    );
    assert_eq!(structured["inline_image"]["mime_type"], "image/png");
    assert_eq!(structured["phone_snapshot_id"], "snap-1");

    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::Screenshot(request),
            ..
        } => assert!(request.include_image_data),
        other => panic!("expected screenshot request, got {other:?}"),
    }
}

#[test]
fn phone_screenshot_text_only_omits_image_block() {
    let service = FakeService::with_response(phone_service_response!(Screenshot(
        screenshot_response(None)
    )));
    let result = call(
        &service,
        &text_only_model(),
        "phone_screenshot",
        json!({"session_id": "sess-1"}),
    );

    assert_eq!(result["isError"], false);
    assert_eq!(result["content"].as_array().expect("content").len(), 1);
    let text = result["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("does not support image input"));
    assert!(result["structuredContent"]["inline_image"].is_null());

    match &service.take_requests()[0] {
        ServiceRequest::Phone {
            request: PhoneRequest::Screenshot(request),
            ..
        } => assert!(
            !request.include_image_data,
            "text-only model must not request image data"
        ),
        other => panic!("expected screenshot request, got {other:?}"),
    }
}

#[test]
fn phone_service_error_becomes_tool_error() {
    let service = FakeService::with_response(ServiceResponse::Error {
        ok: false,
        code: "PhoneServiceFailure".to_string(),
        message: "service exploded".to_string(),
        session_id: None,
        turn_id: None,
        retry: None,
    });
    let result = call(&service, &image_model(), "phone_status", json!({}));
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "PhoneServiceFailure");
}

// ---------------------------------------------------------------------------
// response fixtures
// ---------------------------------------------------------------------------

fn action_response() -> PhoneActionResponse {
    PhoneActionResponse {
        session_id: "sess-1".to_string(),
        serial: "ABC".to_string(),
        action: "phone_tap".to_string(),
        backend: PhoneBackendKind::None,
        capability_profile_id: String::new(),
        profile_refresh_state: PhoneCapabilityRefreshState::Stale,
        phone_snapshot_id: None,
        cursor: None,
        diagnostics: vec![DiagnosticEntry {
            code: "PhoneNotImplemented".to_string(),
            message: "not implemented".to_string(),
            details: None,
        }],
    }
}

fn app_response(kind: PhoneAppResponseKind) -> PhoneAppResponse {
    PhoneAppResponse {
        session_id: "sess-1".to_string(),
        serial: "ABC".to_string(),
        kind,
        backend: PhoneBackendKind::None,
        success: false,
        destination_appshot: None,
        current_app: Some(PhoneAppInfo {
            package_name: "com.android.settings".to_string(),
            label: None,
            activity: None,
            version_name: None,
            version_code: None,
            launchable: true,
            system_app: true,
        }),
        apps: Vec::new(),
        truncated: false,
        install_strategy: None,
        diagnostics: vec![DiagnosticEntry {
            code: "PhoneNotImplemented".to_string(),
            message: "not implemented".to_string(),
            details: None,
        }],
    }
}

fn screenshot_response(data_base64: Option<String>) -> PhoneScreenshotResponse {
    let inline_image = data_base64.map(|data_base64| PhoneImage {
        mime_type: "image/png".to_string(),
        data_base64,
        width: Some(1080),
        height: Some(2400),
    });
    PhoneScreenshotResponse {
        session_id: "sess-1".to_string(),
        serial: "ABC".to_string(),
        phone_snapshot_id: "snap-1".to_string(),
        backend: PhoneBackendKind::Adb,
        capability_profile_id: "profile-1".to_string(),
        profile_refresh_state: PhoneCapabilityRefreshState::Detected,
        screenshot_path: Some("/tmp/sky-cua/phone/snap-1.png".to_string()),
        inline_image,
        device_size: PixelSize {
            width: 1080,
            height: 2400,
        },
        coordinate_mapping: identity_mapping(),
        cursor: None,
        cursor_capabilities: sky_cua_platform::model::PhoneCursorCapabilities {
            host_visible_overlay: false,
            screenshot_synthetic_cursor: false,
            phone_native_overlay: false,
            visible_overlay_reason: None,
        },
        capture_contains_native_overlay: false,
        diagnostics: Vec::new(),
    }
}

fn phone_session() -> PhoneSession {
    PhoneSession {
        session_id: "sess-1".to_string(),
        serial: "ABC".to_string(),
        connection: Some(PhoneConnectionIdentity::Adb {
            serial: "ABC".to_string(),
            name: None,
        }),
        connection_kind: PhoneConnectionKind::Usb,
        backend: PhoneBackendKind::Adb,
        capabilities: PhoneBackendCapabilities {
            adb: true,
            companion: false,
            scrcpy: false,
            screenshot: true,
            gestures: true,
            text_input: true,
            key_input: true,
            accessibility_tree: false,
            notifications: false,
            app_management: true,
            host_visible_overlay: false,
            screenshot_synthetic_cursor: false,
            phone_native_overlay: false,
        },
        capability_profile: PhoneCapabilityProfile {
            profile_id: "profile-1".to_string(),
            session_id: "sess-1".to_string(),
            serial: "ABC".to_string(),
            detected_at_ms: 0,
            stale: false,
            refresh_state: PhoneCapabilityRefreshState::Detected,
            manufacturer: None,
            brand: None,
            model: None,
            device: None,
            target_device_kind: PhoneTargetDeviceKind::UnknownAndroid,
            hyperos_version: None,
            android_sdk: None,
            android_release: None,
            display_size: None,
            density_dpi: None,
            orientation: None,
            display_rotation_degrees: None,
            connection_kind: PhoneConnectionKind::Usb,
            companion: PhoneCompanionCapabilities::absent("com.example.companion"),
            scrcpy: PhoneScrcpyCapabilities::absent(),
            root_available: false,
            shizuku_available: false,
            device_owner: false,
            available_actions: Vec::new(),
            unavailable_actions: Vec::new(),
            routes: Vec::new(),
        },
        companion: None,
        managed_process: false,
        window_title: None,
        created_at_ms: 0,
    }
}

/// A `PhoneObserveResponse` carrying the supplied backend and diagnostics. Used
/// to exercise the `phone_observe_result` is-error rule, which keys solely on
/// `backend == None`.
fn observe_response(
    backend: PhoneBackendKind,
    diagnostics: Vec<DiagnosticEntry>,
) -> PhoneObserveResponse {
    PhoneObserveResponse {
        session: phone_session(),
        appshot: None,
        phone_snapshot_id: Some("snap-1".to_string()),
        screenshot_path: None,
        inline_image: None,
        current_app: None,
        accessibility_summary: None,
        recent_notifications: Vec::new(),
        cursor: None,
        backend,
        capability_profile_id: "profile-1".to_string(),
        profile_refresh_state: PhoneCapabilityRefreshState::Reused,
        available_actions: Vec::new(),
        unavailable_actions: Vec::new(),
        diagnostics,
    }
}

fn phone_appshot() -> AppShotEnvelope {
    let content_ref = ContentRef {
        content_id: "content-1".into(),
        device_id: Some("device-1".into()),
        link_epoch: Some(1),
        mime_type: "application/json".into(),
        filename: None,
        size_bytes: 1,
        sha256: "00".repeat(32),
        source: ContentSource::HostPrivateArtifact,
        expires_at_ms: Some(1_000),
        persistence: ContentPersistence::Temporary,
    };
    AppShotEnvelope {
        appshot_id: "appshot-1".into(),
        trigger: AppShotTrigger::Observe,
        captured_at: chrono::Utc::now(),
        consistency: AppShotConsistency::Stable,
        capture: AppShotCapture::Phone {
            device_id: "device-1".into(),
            link_epoch: 1,
            package_name: Some("com.example".into()),
            activity_name: Some("MainActivity".into()),
            display_id: 0,
            window_ids: vec![1],
            semantic_projection: json!({
                "nodes": [{
                    "node_index": 0,
                    "class_name": "android.widget.Button",
                    "text": "Continue",
                    "clickable": true
                }]
            }),
            event_sequence_before: 1,
            event_sequence_after: 1,
            full_tree_artifact: content_ref.clone(),
        },
        image: content_ref,
        action_snapshot: AppShotActionSnapshot {
            snapshot_id: "snapshot-1".into(),
            session_id: Some("sess-1".into()),
            subject_generation: Some(1),
        },
        coverage: AppShotCoverage {
            pixels_complete: true,
            semantics_complete: true,
            secure_regions_redacted: false,
            projection_truncated: false,
            total_semantic_nodes: Some(1),
            projected_semantic_nodes: Some(1),
        },
        capability_profile_id: "profile-1".into(),
        diagnostics: Vec::new(),
    }
}

fn identity_mapping() -> PhoneCoordinateMapping {
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: 1080.0,
        height: 2400.0,
        space: CoordinateSpace::DesktopLogical,
    };
    PhoneCoordinateMapping {
        mapping_id: "map-1".to_string(),
        session_id: "sess-1".to_string(),
        serial: "ABC".to_string(),
        device_rect: rect.clone(),
        screenshot_rect: rect,
        host_window_rect: None,
        host_content_rect: None,
        rotation_degrees: 0,
        captured_at_ms: 0,
    }
}
