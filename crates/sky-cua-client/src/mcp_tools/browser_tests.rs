use std::cell::RefCell;
use std::collections::VecDeque;

use serde_json::json;
use sky_cua_platform::model::{
    BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT, BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
    BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserActionResponse, BrowserClaimTabResponse,
    BrowserEvalResponse, BrowserListTabsResponse, BrowserMoveMouseResponse, BrowserOpenResponse,
    BrowserRequest, BrowserResponse, BrowserSnapshotResponse, BrowserStatusReport, BrowserTab,
    BrowserTargetAvailability, BrowserTargetKind, DiagnosticEntry, ServiceRequest, ServiceResponse,
};

use crate::heuristics::HeuristicsRegistry;
use crate::mcp_server::ModelSessionInfo;

use super::browser::{
    BROWSER_EVAL_ENV, BrowserTabTextFilter, browser_list_tabs_summary, browser_open_summary,
    browser_snapshot_summary, browser_status_summary, parse_browser_open_url, parse_browser_point,
    parse_browser_scroll, parse_browser_tab_id, parse_browser_target, parse_required_browser_url,
    parse_required_literal_string, parse_required_string,
};
use super::{McpService, build_tool_definitions, handle_tool_call};

/// Serializes the tests that toggle `SKY_CUA_BROWSER_EVAL`.
static EVAL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

impl McpService for FakeService {
    fn call(&self, request: &ServiceRequest) -> anyhow::Result<ServiceResponse> {
        self.requests.borrow_mut().push(request.clone());
        self.responses
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("fake service response queue exhausted"))
    }
}

macro_rules! browser_service_response {
    ($variant:ident { $($body:tt)* }) => {
        ServiceResponse::Browser {
            response: BrowserResponse::$variant { $($body)* },
        }
    };
}

fn tool_names(tools: &serde_json::Value) -> Vec<&str> {
    tools
        .as_array()
        .expect("tools should be an array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect()
}

#[test]
fn browser_tool_definitions_are_always_advertised() {
    let definitions = build_tool_definitions(false, false);
    let names = tool_names(&definitions);
    assert!(names.contains(&"status"));
    assert!(names.contains(&"list_resources"));
    assert!(names.contains(&"observe"));
    assert!(names.contains(&"capture_screen"));
    assert!(names.contains(&"browser_open"));
    assert!(names.contains(&"browser_claim_tab"));
    assert!(names.contains(&"browser_move_mouse"));
    assert!(names.contains(&"browser_navigate"));
    assert!(names.contains(&"browser_input"));
    assert!(names.contains(&"browser_scroll"));
    assert!(
        !names.contains(&"browser_eval"),
        "browser_eval must stay opt-in"
    );
}

#[test]
fn browser_scroll_schema_matches_delta_defaults() {
    let definitions = build_tool_definitions(false, false);
    let scroll_tool = definitions
        .as_array()
        .expect("tools should be an array")
        .iter()
        .find(|tool| tool["name"] == "browser_scroll")
        .expect("browser_scroll tool is advertised");

    assert_eq!(scroll_tool["inputSchema"]["required"], json!(["tab_id"]));
    let description = scroll_tool["description"]
        .as_str()
        .expect("browser_scroll description");
    assert!(description.contains("non-zero delta_x or delta_y"));
    let properties = &scroll_tool["inputSchema"]["properties"];
    assert!(
        properties["delta_x"]["description"]
            .as_str()
            .expect("delta_x description")
            .contains("at least one delta must be non-zero")
    );
    assert!(
        properties["delta_y"]["description"]
            .as_str()
            .expect("delta_y description")
            .contains("at least one delta must be non-zero")
    );
}

#[test]
fn browser_action_schemas_explain_simple_control_contract() {
    let definitions = build_tool_definitions(false, false);
    let tools = definitions.as_array().expect("tools should be an array");
    let find_tool = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} tool is advertised"))
    };

    let list_description = find_tool("list_resources")["description"]
        .as_str()
        .expect("list_resources description");
    assert!(list_description.contains("browser tabs"));

    let claim_description = find_tool("browser_claim_tab")["description"]
        .as_str()
        .expect("browser_claim_tab description");
    assert!(claim_description.contains("controllable"));

    let observe_description = find_tool("observe")["description"]
        .as_str()
        .expect("observe description");
    assert!(observe_description.contains("Browser requires tab_id"));

    let screenshot_description = find_tool("capture_screen")["description"]
        .as_str()
        .expect("capture_screen description");
    assert!(screenshot_description.contains("browser-tab or phone image"));

    let input_description = find_tool("browser_input")["description"]
        .as_str()
        .expect("browser_input description");
    assert!(input_description.contains("click"));
    assert!(input_description.contains("type text"));

    let move_description = find_tool("browser_move_mouse")["description"]
        .as_str()
        .expect("browser_move_mouse description");
    assert!(move_description.contains("without clicking"));
    assert!(move_description.contains("without clicking"));

    let scroll_description = find_tool("browser_scroll")["description"]
        .as_str()
        .expect("browser_scroll description");
    assert!(scroll_description.contains("move the visible browser agent cursor"));
    assert!(scroll_description.contains("Omit x/y for viewport scroll"));
    assert!(scroll_description.contains("non-zero delta_x or delta_y"));

    let scroll_properties = &find_tool("browser_scroll")["inputSchema"]["properties"];
    let scroll_x_number_schema = scroll_properties["x"]["anyOf"]
        .as_array()
        .expect("browser_scroll x should be nullable")
        .iter()
        .find(|schema| schema["type"] == "number")
        .expect("browser_scroll x numeric branch");
    assert!(
        !scroll_x_number_schema["description"]
            .as_str()
            .expect("browser_scroll x description")
            .contains("Wheel event")
    );
}

#[test]
fn browser_input_schema_accepts_element_ref_alternative() {
    use super::definitions::{schema_accepts, validation_tool_definitions};

    let advertised = build_tool_definitions(false, false);
    let advertised_input = advertised
        .as_array()
        .expect("tools should be an array")
        .iter()
        .find(|tool| tool["name"] == "browser_input")
        .expect("browser_input tool is advertised");

    // The per-branch either/or constraints live on the rich validation schema;
    // the advertised (flattened) schema drops the root oneOf.
    let validation = validation_tool_definitions(false, false);
    let input_tool = validation
        .as_array()
        .expect("validation tools should be an array")
        .iter()
        .find(|tool| tool["name"] == "browser_input")
        .expect("browser_input validation schema present");
    let schema = &input_tool["inputSchema"];

    // The opaque `ref` property is advertised with its guidance description.
    let ref_property = &advertised_input["inputSchema"]["properties"]["ref"];
    assert_eq!(ref_property["type"], "string");
    assert!(
        ref_property["description"]
            .as_str()
            .expect("ref description")
            .contains("opaque element reference from observe(surface=browser)")
    );

    // click accepts either coordinates or a ref.
    assert!(
        schema_accepts(
            schema,
            &json!({"operation": "click", "tab_id": "tab-1", "ref": "opaque-token"})
        ),
        "click by ref must be accepted"
    );
    assert!(
        schema_accepts(
            schema,
            &json!({"operation": "click", "tab_id": "tab-1", "x": 1, "y": 1})
        ),
        "click by coordinates must remain accepted"
    );

    // type_text accepts an optional ref alongside the required text.
    assert!(
        schema_accepts(
            schema,
            &json!({"operation": "type_text", "tab_id": "tab-1", "text": "hi", "ref": "opaque-token"})
        ),
        "type_text with a ref must be accepted"
    );
    assert!(
        schema_accepts(
            schema,
            &json!({"operation": "type_text", "tab_id": "tab-1", "text": "hi"})
        ),
        "type_text without a ref must remain accepted"
    );

    // click must supply exactly one of {x, y} or {ref}: neither and both are rejected.
    assert!(
        !schema_accepts(schema, &json!({"operation": "click", "tab_id": "tab-1"})),
        "click with neither coordinates nor a ref must be rejected"
    );
    assert!(
        !schema_accepts(
            schema,
            &json!({"operation": "click", "tab_id": "tab-1", "x": 1, "y": 1, "ref": "opaque-token"})
        ),
        "click with both coordinates and a ref must be rejected"
    );
}

#[test]
fn browser_snapshot_schema_advertises_element_filtering() {
    let definitions = build_tool_definitions(false, false);
    let observe_tool = definitions
        .as_array()
        .expect("tools should be an array")
        .iter()
        .find(|tool| tool["name"] == "observe")
        .expect("observe tool is advertised");

    let properties = &observe_tool["inputSchema"]["properties"];
    assert!(properties.get("element_query").is_some());
    assert!(properties.get("element_offset").is_some());
    assert!(properties.get("element_limit").is_some());
    assert_eq!(
        properties["element_limit"]["anyOf"][0]["maximum"],
        BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT
    );
    assert_eq!(
        properties["text_limit"]["anyOf"][0]["maximum"],
        BROWSER_SNAPSHOT_MAX_TEXT_LIMIT
    );
}

#[test]
fn parses_browser_list_tabs_target() {
    assert_eq!(
        parse_browser_target(&json!({"target": "user_chrome"})).unwrap(),
        Some(BrowserTargetKind::UserChrome)
    );
    assert_eq!(parse_browser_target(&json!({"target": ""})).unwrap(), None);
    assert_eq!(
        parse_browser_target(&json!({"target": null})).unwrap(),
        None
    );
    assert!(parse_browser_target(&json!({"target": 123})).is_err());
    assert!(parse_browser_target(&json!({"target": {}})).is_err());
    assert!(parse_browser_target(&json!({"target": "managed"})).is_err());
    assert!(parse_browser_target(&json!({"target": "firefox"})).is_err());
}

#[test]
fn parses_browser_open_url_allowlist() {
    assert_eq!(
        parse_browser_open_url(&json!({"url": "https://example.test/"})).unwrap(),
        Some("https://example.test/".to_string())
    );
    assert_eq!(
        parse_browser_open_url(&json!({"url": "about:blank"})).unwrap(),
        Some("about:blank".to_string())
    );
    assert_eq!(parse_browser_open_url(&json!({"url": ""})).unwrap(), None);
    assert_eq!(parse_browser_open_url(&json!({"url": null})).unwrap(), None);
    assert!(parse_browser_open_url(&json!({"url": " http://127.0.0.1:8080/page "})).is_err());
    assert!(parse_browser_open_url(&json!({"url": 123})).is_err());
    assert!(parse_browser_open_url(&json!({"url": {}})).is_err());
    assert!(parse_browser_open_url(&json!({"url": "file:///etc/passwd"})).is_err());
    assert!(parse_browser_open_url(&json!({"url": "javascript:alert(1)"})).is_err());
}

#[test]
fn parses_browser_tab_id_and_point() {
    assert_eq!(
        parse_browser_tab_id(&json!({"tab_id": " 123 "})).unwrap(),
        "123"
    );
    assert!(parse_browser_tab_id(&json!({"tab_id": 456})).is_err());
    assert!(parse_browser_tab_id(&json!({"tab_id": ""})).is_err());
    assert!(parse_browser_tab_id(&json!({"tab_id": null})).is_err());

    assert_eq!(
        parse_browser_point(&json!({"x": 240, "y": 160.5}), "browser_click").unwrap(),
        (240.0, 160.5)
    );
    assert!(parse_browser_point(&json!({"x": -1, "y": 160}), "browser_click").is_err());
    let error = parse_browser_point(&json!({"x": "240", "y": 160}), "browser_click")
        .expect_err("string x should be invalid");
    assert!(
        error
            .to_string()
            .contains("browser_click x must be a number")
    );
}

#[test]
fn parses_browser_action_arguments() {
    assert!(
        parse_required_browser_url(
            &json!({"url": " https://example.test/ "}),
            "browser_navigate"
        )
        .is_err()
    );
    assert!(
        parse_required_browser_url(&json!({"url": "file:///tmp/nope"}), "browser_navigate")
            .is_err()
    );
    assert_eq!(
        parse_required_string(
            &json!({"text": " hello "}),
            "text",
            "browser_type_text text"
        )
        .unwrap(),
        "hello"
    );
    assert_eq!(
        parse_required_literal_string(
            &json!({"text": " hello "}),
            "text",
            "browser_type_text text"
        )
        .unwrap(),
        " hello "
    );
    assert_eq!(
        parse_required_literal_string(&json!({"text": "   "}), "text", "browser_type_text text")
            .unwrap(),
        "   "
    );
    assert!(parse_required_string(&json!({"text": ""}), "text", "browser_type_text text").is_err());
    assert!(
        parse_required_literal_string(&json!({"text": ""}), "text", "browser_type_text text")
            .is_err()
    );
    assert_eq!(
        parse_browser_scroll(&json!({"delta_y": 500, "x": 10, "y": 20})).unwrap(),
        (0.0, 500.0, Some(10.0), Some(20.0))
    );
    assert_eq!(
        parse_browser_scroll(&json!({"delta_y": 500})).unwrap(),
        (0.0, 500.0, None, None)
    );
    assert!(parse_browser_scroll(&json!({"delta_x": 0, "delta_y": 0})).is_err());
    assert!(parse_browser_scroll(&json!({"delta_y": 500, "x": 10})).is_err());
}

#[test]
fn browser_type_text_preserves_literal_text() {
    let service = FakeService::with_response(browser_service_response!(TypeText {
        response: BrowserActionResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "123".to_string(),
            action: "type_text".to_string(),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_type_text",
        json!({"target": "user_chrome", "tab_id": "123", "text": " hello "}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Browser {
            request: BrowserRequest::TypeText {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                text: " hello ".to_string(),
            },
        }
    );
}

#[test]
fn browser_claim_tab_routes_to_service_and_returns_tab() {
    let service = FakeService::with_response(browser_service_response!(ClaimTab {
        response: BrowserClaimTabResponse {
            target: BrowserTargetKind::UserChrome,
            tab: Some(BrowserTab {
                tab_id: "123".to_string(),
                target: BrowserTargetKind::UserChrome,
                title: Some("Example".to_string()),
                url: Some("https://example.test/".to_string()),
                active: true,
            }),
            diagnostics: Vec::new(),
        },
    }));
    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_claim_tab",
        json!({"target": "user_chrome", "tab_id": "123"}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Claimed browser tab 123")
    );
    assert_eq!(result["structuredContent"]["tab"]["tab_id"], "123");
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Browser {
            request: BrowserRequest::ClaimTab {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
            },
        }
    );
}

#[test]
fn browser_move_mouse_routes_to_service_and_returns_coordinates() {
    let service = FakeService::with_response(browser_service_response!(MoveMouse {
        response: BrowserMoveMouseResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "123".to_string(),
            x: 240.0,
            y: 160.0,
            wait_for_arrival: true,
            diagnostics: Vec::new(),
        },
    }));
    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_move_mouse",
        json!({"target": "user_chrome", "tab_id": "123", "x": 240, "y": 160}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Moved browser cursor")
    );
    assert_eq!(result["structuredContent"]["tab_id"], "123");
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Browser {
            request: BrowserRequest::MoveMouse {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                x: 240.0,
                y: 160.0,
                wait_for_arrival: true,
            },
        }
    );
}

#[test]
fn browser_move_mouse_rejects_non_boolean_wait_for_arrival() {
    let service = FakeService::default();
    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_move_mouse",
        json!({
            "target": "user_chrome",
            "tab_id": "123",
            "x": 240,
            "y": 160,
            "wait_for_arrival": "false"
        }),
    )
    .unwrap();

    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("wait_for_arrival must be a boolean when provided")
    );
    assert!(service.take_requests().is_empty());
}

#[test]
fn browser_open_routes_to_service_and_returns_tab() {
    let service = FakeService::with_response(browser_service_response!(Open {
        response: BrowserOpenResponse {
            target: BrowserTargetKind::UserChrome,
            tab: Some(BrowserTab {
                tab_id: "123".to_string(),
                target: BrowserTargetKind::UserChrome,
                title: Some("Example".to_string()),
                url: Some("https://example.test/".to_string()),
                active: true,
            }),
            diagnostics: Vec::new(),
        },
    }));
    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_open",
        json!({"target": "user_chrome", "url": "https://example.test/"}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Opened browser tab 123")
    );
    assert_eq!(result["structuredContent"]["tab"]["tab_id"], "123");

    let requests = service.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0],
        ServiceRequest::Browser {
            request: BrowserRequest::Open {
                target: Some(BrowserTargetKind::UserChrome),
                url: Some("https://example.test/".to_string()),
            },
        }
    );
}

#[test]
fn browser_open_summary_reports_failure_without_tab() {
    let response = BrowserOpenResponse {
        target: BrowserTargetKind::UserChrome,
        tab: None,
        diagnostics: vec![DiagnosticEntry {
            code: "BrowserBridgeDisconnected".to_string(),
            message: "No browser bridge is connected.".to_string(),
            details: None,
        }],
    };

    let summary = browser_open_summary(&response);
    assert!(summary.contains("Could not open browser tab"));
    assert!(summary.contains("No browser bridge is connected"));
}

#[test]
fn browser_open_partial_response_is_mcp_error_with_tab() {
    let service = FakeService::with_response(browser_service_response!(Open {
        response: BrowserOpenResponse {
            target: BrowserTargetKind::UserChrome,
            tab: Some(BrowserTab {
                tab_id: "616".to_string(),
                target: BrowserTargetKind::UserChrome,
                title: Some("Partial Tab".to_string()),
                url: Some("about:blank".to_string()),
                active: true,
            }),
            diagnostics: vec![DiagnosticEntry {
                code: "BrowserOpenPartial".to_string(),
                message: "Created browser tab 616, but browser_open could not complete attach."
                    .to_string(),
                details: Some("source_code=BrowserBridgeRequestFailed".to_string()),
            }],
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_open",
        json!({"target": "user_chrome"}),
    )
    .unwrap();

    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["tab"]["tab_id"], "616");
    assert_eq!(
        result["structuredContent"]["diagnostics"][0]["code"],
        "BrowserOpenPartial"
    );
    let summary = result["content"][0]["text"].as_str().unwrap();
    assert!(summary.contains("Created browser tab 616"));
    assert!(summary.contains("browser_open did not complete"));
    assert!(!summary.contains("Opened browser tab 616"));
}

#[test]
fn browser_status_summary_mentions_targets_and_diagnostics() {
    let report = BrowserStatusReport {
        enabled: true,
        available_targets: vec![BrowserTargetAvailability {
            target: BrowserTargetKind::UserChrome,
            available: true,
            detail: "Chrome native-host browser bridge is responsive.".to_string(),
        }],
        tabs_known: None,
        browser_integration: None,
        diagnostics: vec![DiagnosticEntry {
            code: "BrowserBridgeDisconnected".to_string(),
            message: "No browser bridge is connected.".to_string(),
            details: None,
        }],
    };

    let summary = browser_status_summary(&report);
    assert!(summary.contains("Browser MCP tools are available"));
    assert!(summary.contains("user_chrome=available"));
    assert!(summary.contains("Tabs known: unknown"));
    assert!(summary.contains("No browser bridge is connected"));
}

#[test]
fn browser_list_tabs_summary_mentions_target_and_diagnostics() {
    let response = BrowserListTabsResponse {
        target: Some(BrowserTargetKind::UserChrome),
        tabs: vec![BrowserTab {
            tab_id: "tab-1".to_string(),
            target: BrowserTargetKind::UserChrome,
            title: Some("Example".to_string()),
            url: Some("https://example.com".to_string()),
            active: true,
        }],
        diagnostics: vec![DiagnosticEntry {
            code: "BrowserBridgeDisconnected".to_string(),
            message: "No browser bridge is connected.".to_string(),
            details: None,
        }],
    };

    let summary = browser_list_tabs_summary(&response, &BrowserTabTextFilter::default());
    assert!(summary.contains("Discovered 1 browser tabs for user_chrome"));
    assert!(summary.contains("[tab-1]"));
    assert!(summary.contains("title=\"Example\""));
    assert!(summary.contains("url=\"https://example.com\""));
    assert!(summary.contains("No browser bridge is connected"));
}

#[test]
fn browser_list_tabs_summary_defaults_to_user_chrome_target() {
    let response = BrowserListTabsResponse {
        target: None,
        tabs: Vec::new(),
        diagnostics: Vec::new(),
    };

    let summary = browser_list_tabs_summary(&response, &BrowserTabTextFilter::default());

    assert!(summary.contains("for user_chrome"));
    assert!(!summary.contains("all browser targets"));
}

#[test]
fn browser_list_tabs_summary_filters_text_visible_matches() {
    let response = BrowserListTabsResponse {
        target: Some(BrowserTargetKind::UserChrome),
        tabs: vec![
            BrowserTab {
                tab_id: "tab-1".to_string(),
                target: BrowserTargetKind::UserChrome,
                title: Some("Docs".to_string()),
                url: Some("https://example.com/docs".to_string()),
                active: false,
            },
            BrowserTab {
                tab_id: "tab-2".to_string(),
                target: BrowserTargetKind::UserChrome,
                title: Some("Dot Agents | OpenChamber".to_string()),
                url: Some("https://chamber.heliasar.com/".to_string()),
                active: true,
            },
        ],
        diagnostics: Vec::new(),
    };

    let summary = browser_list_tabs_summary(
        &response,
        &BrowserTabTextFilter {
            title_contains: None,
            url_contains: Some("CHAMBER.heliasar".to_string()),
        },
    );

    assert!(summary.contains("2 browser tabs"));
    assert!(summary.contains("1 matched"));
    assert!(summary.contains("[tab-2]"));
    assert!(summary.contains("OpenChamber"));
    assert!(!summary.contains("[tab-1]"));
}

#[test]
fn browser_list_tabs_filters_structured_content_when_filter_is_present() {
    let service = FakeService::with_response(browser_service_response!(ListTabs {
        response: BrowserListTabsResponse {
            target: Some(BrowserTargetKind::UserChrome),
            tabs: vec![
                BrowserTab {
                    tab_id: "tab-1".to_string(),
                    target: BrowserTargetKind::UserChrome,
                    title: Some("Private unrelated tab".to_string()),
                    url: Some("https://unrelated.example/".to_string()),
                    active: false,
                },
                BrowserTab {
                    tab_id: "tab-2".to_string(),
                    target: BrowserTargetKind::UserChrome,
                    title: Some("Dot Agents | OpenChamber".to_string()),
                    url: Some("https://chamber.heliasar.com/".to_string()),
                    active: true,
                },
            ],
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_list_tabs",
        json!({"target": "user_chrome", "url_contains": "chamber.heliasar.com"}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert_eq!(
        result["structuredContent"]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(result["structuredContent"]["tabs"][0]["tab_id"], "tab-2");
    assert!(!result.to_string().contains("Private unrelated tab"));
}

#[test]
fn browser_list_tabs_truncates_structured_content_to_limit() {
    let tabs: Vec<BrowserTab> = (0..5)
        .map(|index| BrowserTab {
            tab_id: format!("tab-{index}"),
            target: BrowserTargetKind::UserChrome,
            title: Some(format!("Tab {index}")),
            url: Some(format!("https://example.test/{index}")),
            active: index == 0,
        })
        .collect();
    let service = FakeService::with_response(browser_service_response!(ListTabs {
        response: BrowserListTabsResponse {
            target: Some(BrowserTargetKind::UserChrome),
            tabs,
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_list_tabs",
        json!({"target": "user_chrome", "limit": 2}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    let returned = result["structuredContent"]["tabs"].as_array().unwrap();
    assert_eq!(returned.len(), 2);
    assert_eq!(returned[0]["tab_id"], "tab-0");
    assert_eq!(returned[1]["tab_id"], "tab-1");
}

#[test]
fn browser_list_tabs_limit_zero_returns_all_tabs() {
    let tabs: Vec<BrowserTab> = (0..3)
        .map(|index| BrowserTab {
            tab_id: format!("tab-{index}"),
            target: BrowserTargetKind::UserChrome,
            title: Some(format!("Tab {index}")),
            url: Some(format!("https://example.test/{index}")),
            active: index == 0,
        })
        .collect();
    let service = FakeService::with_response(browser_service_response!(ListTabs {
        response: BrowserListTabsResponse {
            target: Some(BrowserTargetKind::UserChrome),
            tabs,
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_list_tabs",
        json!({"limit": 0}),
    )
    .unwrap();

    assert_eq!(
        result["structuredContent"]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn browser_list_tabs_limit_summary_reports_true_total_not_capped_count() {
    let tabs: Vec<BrowserTab> = (0..5)
        .map(|index| BrowserTab {
            tab_id: format!("tab-{index}"),
            target: BrowserTargetKind::UserChrome,
            title: Some(format!("Tab {index}")),
            url: Some(format!("https://example.test/{index}")),
            active: index == 0,
        })
        .collect();
    let service = FakeService::with_response(browser_service_response!(ListTabs {
        response: BrowserListTabsResponse {
            target: Some(BrowserTargetKind::UserChrome),
            tabs,
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_list_tabs",
        json!({"target": "user_chrome", "limit": 2}),
    )
    .unwrap();

    // The cap returns 2 tabs but must not tell the model only 2 exist: the
    // summary reports the true discovered total (5) and that it is showing 2.
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Discovered 5 browser tabs"),
        "summary should report the true total, got: {text}"
    );
    assert!(
        text.contains("Showing first 2 tab"),
        "summary should note the cap, got: {text}"
    );
    assert_eq!(
        result["structuredContent"]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn browser_snapshot_summary_exposes_page_details_for_text_only_agents() {
    let response = BrowserSnapshotResponse {
        target: BrowserTargetKind::UserChrome,
        tab_id: "tab-1".to_string(),
        title: Some("Dot Agents | OpenChamber".to_string()),
        url: Some("https://chamber.heliasar.com/".to_string()),
        snapshot: Some(json!({
            "title": "Dot Agents | OpenChamber",
            "url": "https://chamber.heliasar.com/",
            "viewport": {"width": 1440, "height": 900, "devicePixelRatio": 1.25},
            "text": "New session\nUpdate Available\nSky Cua browser smoke test",
            "elements": [
                {
                    "index": 3,
                    "tag": "button",
                    "role": "button",
                    "name": "Update Available",
                    "href": null,
                    "bounds": {"x": 241.6, "y": 971.2, "width": 120, "height": 32}
                }
            ]
        })),
        diagnostics: Vec::new(),
    };

    let summary = browser_snapshot_summary(&response);

    assert!(summary.contains("tab tab-1"));
    assert!(summary.contains("Title: \"Dot Agents | OpenChamber\""));
    assert!(summary.contains("URL: https://chamber.heliasar.com/"));
    assert!(summary.contains("Viewport: width=1440 height=900."));
    // The device pixel ratio is deliberately not surfaced: coordinates are a
    // single normalized CSS-pixel space, so exposing the ratio only tempts a
    // model to double-correct for scaling.
    assert!(!summary.contains("devicePixelRatio"));
    assert!(summary.contains("Visible text: \"New session Update Available"));
    assert!(summary.contains("[3] tag=button role=button name=\"Update Available\""));
    assert!(summary.contains("bounds=x:241.60 y:971.20 w:120 h:32"));
}

#[test]
fn browser_snapshot_caps_structured_elements_by_default() {
    let many_elements: Vec<_> = (0..1000)
        .map(|index| json!({"index": index, "tag": "button", "role": "button"}))
        .collect();
    let service = FakeService::with_response(browser_service_response!(Snapshot {
        response: BrowserSnapshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            title: Some("Dense".to_string()),
            url: Some("https://dense.example/".to_string()),
            snapshot: Some(json!({
                "title": "Dense",
                "url": "https://dense.example/",
                "viewport": {"width": 1440, "height": 900, "devicePixelRatio": 1},
                "text": "Dense",
                "elementCount": 1000,
                "elements": many_elements,
            })),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({"tab_id": "tab-1"}),
    )
    .unwrap();

    let elements = result["structuredContent"]["snapshot"]["elements"]
        .as_array()
        .expect("elements array");
    // Untuned calls are capped so dense pages do not overflow host budgets,
    // but the true total stays visible via elementCount.
    assert_eq!(elements.len(), 200);
    assert_eq!(
        result["structuredContent"]["snapshot"]["elementCount"],
        1000
    );
}

#[test]
fn browser_snapshot_limits_visible_text_by_default_and_preserves_metadata() {
    let long_text = "abcdef".repeat(900);
    let service_text = long_text
        .chars()
        .take(BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT)
        .collect::<String>();
    let service = FakeService::with_response(browser_service_response!(Snapshot {
        response: BrowserSnapshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            title: Some("Long".to_string()),
            url: Some("https://long.example/".to_string()),
            snapshot: Some(json!({
                "title": "Long",
                "url": "https://long.example/",
                "viewport": {"width": 1440, "height": 900, "devicePixelRatio": 1},
                "text": service_text,
                "textCharCount": null,
                "textLimit": BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT,
                "textTruncated": true,
                "elementCount": 0,
                "elements": [],
            })),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({"tab_id": "tab-1"}),
    )
    .unwrap();

    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: None,
                tab_id: "tab-1".to_string(),
                text_limit: Some(BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT),
                element_offset: None,
                element_limit: Some(200),
                element_query: None,
            },
        }
    );
    let text = result["structuredContent"]["snapshot"]["text"]
        .as_str()
        .expect("snapshot text");
    assert_eq!(text.chars().count(), BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT);
    assert_eq!(
        result["structuredContent"]["snapshot"]["textCharCount"],
        serde_json::Value::Null
    );
    assert_eq!(
        result["structuredContent"]["snapshot"]["textLimit"],
        BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT
    );
    assert_eq!(
        result["structuredContent"]["snapshot"]["textTruncated"],
        true
    );
}

#[test]
fn browser_snapshot_accepts_zero_text_limit_to_omit_visible_text() {
    let service = FakeService::with_response(browser_service_response!(Snapshot {
        response: BrowserSnapshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            title: Some("No Text".to_string()),
            url: Some("https://notext.example/".to_string()),
            snapshot: Some(json!({
                "title": "No Text",
                "url": "https://notext.example/",
                "text": "",
                "textCharCount": null,
                "textTruncated": null,
                "elements": [],
            })),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({"tab_id": "tab-1", "text_limit": 0}),
    )
    .unwrap();

    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: None,
                tab_id: "tab-1".to_string(),
                text_limit: Some(0),
                element_offset: None,
                element_limit: Some(200),
                element_query: None,
            },
        }
    );
    assert_eq!(result["structuredContent"]["snapshot"]["text"], "");
    assert_eq!(
        result["structuredContent"]["snapshot"]["textTruncated"],
        serde_json::Value::Null
    );
    assert!(
        !result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Visible text:")
    );
}

#[test]
fn browser_snapshot_rejects_oversized_text_limit() {
    let service = FakeService::default();
    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({"tab_id": "tab-1", "text_limit": BROWSER_SNAPSHOT_MAX_TEXT_LIMIT + 1}),
    )
    .unwrap();

    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(&format!(
                "text_limit must be at most {BROWSER_SNAPSHOT_MAX_TEXT_LIMIT}"
            ))
    );
    assert!(service.take_requests().is_empty());
}

#[test]
fn browser_snapshot_rejects_oversized_element_limit() {
    let service = FakeService::default();
    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({"tab_id": "tab-1", "element_limit": BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT + 1}),
    )
    .unwrap();

    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(&format!(
                "element_limit must be at most {BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT}"
            ))
    );
    assert!(service.take_requests().is_empty());
}

#[test]
fn browser_snapshot_filters_structured_elements_for_deep_sidebar_controls() {
    let service = FakeService::with_response(browser_service_response!(Snapshot {
        response: BrowserSnapshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            title: Some("Dot Agents | OpenChamber".to_string()),
            url: Some("https://chamber.heliasar.com/".to_string()),
            snapshot: Some(json!({
                "title": "Dot Agents | OpenChamber",
                "url": "https://chamber.heliasar.com/",
                "viewport": {"width": 1440, "height": 900, "devicePixelRatio": 1},
                "text": "OpenChamber",
                "elementCount": 200,
                "elements": [
                    {"index": 0, "tag": "button", "role": "button", "name": "New Session"},
                    {"index": 184, "tag": "button", "role": "button", "name": "Update Available"},
                    {"index": 185, "tag": "button", "role": "button", "name": "Settings"}
                ]
            })),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({"tab_id": "tab-1", "element_query": "update", "element_limit": 1}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    let elements = result["structuredContent"]["snapshot"]["elements"]
        .as_array()
        .expect("filtered elements");
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0]["index"], 184);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Update Available")
    );
    assert!(!result.to_string().contains("New Session"));
    assert_eq!(
        service.take_requests(),
        vec![ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: None,
                tab_id: "tab-1".to_string(),
                text_limit: Some(BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT),
                element_offset: None,
                element_limit: Some(1),
                element_query: Some("update".to_string()),
            },
        }]
    );
}

#[test]
fn browser_snapshot_sends_offset_to_service_without_double_skipping_results() {
    let service = FakeService::with_response(browser_service_response!(Snapshot {
        response: BrowserSnapshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            title: Some("Paged".to_string()),
            url: Some("https://paged.example/".to_string()),
            snapshot: Some(json!({
                "title": "Paged",
                "url": "https://paged.example/",
                "text": "",
                "elementCount": 100,
                "elements": [
                    {"index": 7, "tag": "button", "role": "button", "name": "Result 7"},
                    {"index": 8, "tag": "button", "role": "button", "name": "Result 8"}
                ]
            })),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({
            "tab_id": "tab-1",
            "element_query": "result",
            "element_offset": 7,
            "element_limit": 2
        }),
    )
    .unwrap();

    let elements = result["structuredContent"]["snapshot"]["elements"]
        .as_array()
        .expect("paged elements");
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[0]["index"], 7);
    assert_eq!(elements[1]["index"], 8);
    assert_eq!(
        service.take_requests(),
        vec![ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: None,
                tab_id: "tab-1".to_string(),
                text_limit: Some(BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT),
                element_offset: Some(7),
                element_limit: Some(2),
                element_query: Some("result".to_string()),
            },
        }]
    );
}

#[test]
fn browser_snapshot_accepts_zero_element_limit_to_omit_elements() {
    let service = FakeService::with_response(browser_service_response!(Snapshot {
        response: BrowserSnapshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            title: Some("Text Only".to_string()),
            url: Some("https://text.example/".to_string()),
            snapshot: Some(json!({
                "title": "Text Only",
                "url": "https://text.example/",
                "text": "Visible text",
                "elementCount": 2,
                "elements": [
                    {"index": 0, "tag": "button", "role": "button", "name": "One"},
                    {"index": 1, "tag": "button", "role": "button", "name": "Two"}
                ]
            })),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({"tab_id": "tab-1", "element_offset": 7, "element_limit": 0}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert_eq!(
        result["structuredContent"]["snapshot"]["elements"]
            .as_array()
            .expect("elements")
            .len(),
        0
    );
    assert_eq!(
        service.take_requests(),
        vec![ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: None,
                tab_id: "tab-1".to_string(),
                text_limit: Some(BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT),
                element_offset: None,
                element_limit: Some(0),
                element_query: None,
            },
        }]
    );
}

#[test]
fn browser_snapshot_offset_past_service_cap_requests_no_elements() {
    let service = FakeService::with_response(browser_service_response!(Snapshot {
        response: BrowserSnapshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            title: Some("Past Cap".to_string()),
            url: Some("https://past-cap.example/".to_string()),
            snapshot: Some(json!({
                "title": "Past Cap",
                "url": "https://past-cap.example/",
                "text": "",
                "elementCount": BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
                "elements": []
            })),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({"tab_id": "tab-1", "element_offset": BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert_eq!(
        service.take_requests(),
        vec![ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: None,
                tab_id: "tab-1".to_string(),
                text_limit: Some(BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT),
                element_offset: None,
                element_limit: Some(0),
                element_query: None,
            },
        }]
    );
}

#[test]
fn browser_snapshot_offset_near_service_cap_clamps_requested_window() {
    let service = FakeService::with_response(browser_service_response!(Snapshot {
        response: BrowserSnapshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            title: Some("Near Cap".to_string()),
            url: Some("https://near-cap.example/".to_string()),
            snapshot: Some(json!({
                "title": "Near Cap",
                "url": "https://near-cap.example/",
                "text": "",
                "elementCount": BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
                "elements": [
                    {
                        "index": BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT - 1,
                        "tag": "button",
                        "role": "button",
                        "name": "Last reachable control"
                    }
                ]
            })),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_snapshot",
        json!({
            "tab_id": "tab-1",
            "element_offset": BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT - 1
        }),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert_eq!(
        result["structuredContent"]["snapshot"]["elements"]
            .as_array()
            .expect("elements")
            .len(),
        1
    );
    assert_eq!(
        service.take_requests(),
        vec![ServiceRequest::Browser {
            request: BrowserRequest::Snapshot {
                target: None,
                tab_id: "tab-1".to_string(),
                text_limit: Some(BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT),
                element_offset: Some(BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT - 1),
                element_limit: Some(1),
                element_query: None,
            },
        }]
    );
}

#[test]
fn browser_eval_tool_visibility_follows_the_enabled_flag() {
    let disabled = build_tool_definitions(false, false);
    assert!(
        !disabled
            .as_array()
            .expect("tools")
            .iter()
            .any(|tool| tool["name"] == "browser_eval")
    );

    let enabled = build_tool_definitions(false, true);
    assert!(
        enabled
            .as_array()
            .expect("tools")
            .iter()
            .any(|tool| tool["name"] == "browser_eval")
    );
}

#[test]
fn browser_eval_enabled_by_default_routes_to_service() {
    let _guard = EVAL_ENV_LOCK.lock().unwrap();
    let previous = std::env::var_os(BROWSER_EVAL_ENV);
    unsafe { std::env::remove_var(BROWSER_EVAL_ENV) };

    let service = FakeService::with_response(browser_service_response!(Eval {
        response: BrowserEvalResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            value: Some(json!({"ok": true})),
            diagnostics: Vec::new(),
        },
    }));
    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_eval",
        json!({"tab_id": "tab-1", "expression": "1"}),
    )
    .unwrap();

    if let Some(value) = previous {
        unsafe { std::env::set_var(BROWSER_EVAL_ENV, value) };
    }
    assert_eq!(result["isError"], false);
    assert!(!service.take_requests().is_empty());
}

#[test]
fn browser_eval_rejected_when_explicitly_disabled() {
    let _guard = EVAL_ENV_LOCK.lock().unwrap();
    let previous = std::env::var_os(BROWSER_EVAL_ENV);
    unsafe { std::env::set_var(BROWSER_EVAL_ENV, "off") };

    let service = FakeService::default();
    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_eval",
        json!({"tab_id": "tab-1", "expression": "1"}),
    )
    .unwrap();

    match previous {
        Some(value) => unsafe { std::env::set_var(BROWSER_EVAL_ENV, value) },
        None => unsafe { std::env::remove_var(BROWSER_EVAL_ENV) },
    }
    assert_eq!(result["isError"], true);
    assert!(result.to_string().contains("SKY_CUA_BROWSER_EVAL"));
    assert!(service.take_requests().is_empty());
}

#[test]
fn browser_eval_routes_expression_to_service() {
    let _guard = EVAL_ENV_LOCK.lock().unwrap();
    let previous = std::env::var_os(BROWSER_EVAL_ENV);
    unsafe { std::env::set_var(BROWSER_EVAL_ENV, "on") };
    let service = FakeService::with_response(browser_service_response!(Eval {
        response: BrowserEvalResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "tab-1".to_string(),
            value: Some(json!({"ok": true})),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_eval",
        json!({"tab_id": "tab-1", "expression": "(() => ({ok: true}))()"}),
    )
    .unwrap();

    match previous {
        Some(value) => unsafe { std::env::set_var(BROWSER_EVAL_ENV, value) },
        None => unsafe { std::env::remove_var(BROWSER_EVAL_ENV) },
    }
    assert_eq!(result["isError"], false);
    assert_eq!(result["structuredContent"]["value"]["ok"], true);
    assert_eq!(
        service.take_requests(),
        vec![ServiceRequest::Browser {
            request: BrowserRequest::Eval {
                target: None,
                tab_id: "tab-1".to_string(),
                expression: "(() => ({ok: true}))()".to_string(),
            },
        }]
    );
}

#[test]
fn browser_list_tabs_marks_bridge_failure_as_mcp_error() {
    let service = FakeService::with_response(browser_service_response!(ListTabs {
        response: BrowserListTabsResponse {
            target: Some(BrowserTargetKind::UserChrome),
            tabs: Vec::new(),
            diagnostics: vec![DiagnosticEntry {
                code: "BrowserBridgeDisconnected".to_string(),
                message: "No browser bridge is connected.".to_string(),
                details: None,
            }],
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_list_tabs",
        json!({"target": "user_chrome"}),
    )
    .unwrap();

    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["diagnostics"][0]["code"],
        "BrowserBridgeDisconnected"
    );
}

#[test]
fn browser_list_tabs_marks_unsupported_bridge_as_mcp_error() {
    let service = FakeService::with_response(browser_service_response!(ListTabs {
        response: BrowserListTabsResponse {
            target: Some(BrowserTargetKind::UserChrome),
            tabs: Vec::new(),
            diagnostics: vec![DiagnosticEntry {
                code: "BrowserBridgeUnsupported".to_string(),
                message: "Browser MCP tools require the native-host socket bridge.".to_string(),
                details: None,
            }],
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo::default(),
        "browser_list_tabs",
        json!({"target": "user_chrome"}),
    )
    .unwrap();

    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["diagnostics"][0]["code"],
        "BrowserBridgeUnsupported"
    );
}

fn screenshot_service_response() -> ServiceResponse {
    browser_service_response!(Screenshot {
        response: sky_cua_platform::model::BrowserScreenshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "123".to_string(),
            mime_type: "image/jpeg".to_string(),
            data_base64: "aGVsbG8=".to_string(),
            screenshot_path: Some("/tmp/sky-cua/captures/browser-123-1.jpg".to_string()),
            width: Some(1280),
            height: Some(720),
            diagnostics: Vec::new(),
        },
    })
}

#[test]
fn browser_screenshot_attaches_image_block_and_strips_base64() {
    let service = FakeService::with_response(screenshot_service_response());

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo {
            supports_images: Some(true),
        },
        "browser_screenshot",
        json!({"target": "user_chrome", "tab_id": "123"}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("1280x720"));
    assert!(text.contains("/tmp/sky-cua/captures/browser-123-1.jpg"));
    assert!(text.contains("CSS-pixel"));

    assert_eq!(result["content"][1]["type"], "image");
    assert_eq!(result["content"][1]["data"], "aGVsbG8=");
    assert_eq!(result["content"][1]["mimeType"], "image/jpeg");
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Browser {
            request: BrowserRequest::Screenshot {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                include_image_data: true,
            },
        }
    );

    let structured = &result["structuredContent"];
    assert!(structured.get("data_base64").is_none());
    assert_eq!(
        structured["screenshot_path"],
        "/tmp/sky-cua/captures/browser-123-1.jpg"
    );
    assert_eq!(structured["width"], 1280);
    assert_eq!(structured["height"], 720);
}

#[test]
fn browser_screenshot_for_text_only_model_omits_image_block() {
    let service = FakeService::with_response(screenshot_service_response());

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo {
            supports_images: Some(false),
        },
        "browser_screenshot",
        json!({"target": "user_chrome", "tab_id": "123"}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert_eq!(result["content"].as_array().unwrap().len(), 1);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("does not support image input"));
    assert!(text.contains("browser_snapshot"));
    assert!(result["structuredContent"].get("data_base64").is_none());
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Browser {
            request: BrowserRequest::Screenshot {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                include_image_data: false,
            },
        }
    );
}

#[test]
fn browser_screenshot_text_only_accepts_path_without_image_data() {
    let service = FakeService::with_response(browser_service_response!(Screenshot {
        response: sky_cua_platform::model::BrowserScreenshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "123".to_string(),
            mime_type: "image/jpeg".to_string(),
            data_base64: String::new(),
            screenshot_path: Some("/tmp/sky-cua/captures/browser-123-1.jpg".to_string()),
            width: Some(1280),
            height: Some(720),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo {
            supports_images: Some(false),
        },
        "browser_screenshot",
        json!({"target": "user_chrome", "tab_id": "123"}),
    )
    .unwrap();

    assert_eq!(result["isError"], false);
    assert_eq!(result["content"].as_array().unwrap().len(), 1);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Image data was omitted"));
    assert!(result["structuredContent"].get("data_base64").is_none());
    assert_eq!(
        result["structuredContent"]["screenshot_path"],
        "/tmp/sky-cua/captures/browser-123-1.jpg"
    );
}

#[test]
fn browser_screenshot_text_only_rejects_base64_without_path() {
    let service = FakeService::with_response(browser_service_response!(Screenshot {
        response: sky_cua_platform::model::BrowserScreenshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "123".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "aGVsbG8=".to_string(),
            screenshot_path: None,
            width: Some(1280),
            height: Some(720),
            diagnostics: Vec::new(),
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo {
            supports_images: Some(false),
        },
        "browser_screenshot",
        json!({"target": "user_chrome", "tab_id": "123"}),
    )
    .unwrap();

    assert_eq!(result["isError"], true);
    assert_eq!(result["content"].as_array().unwrap().len(), 1);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Could not capture browser screenshot")
    );
    assert!(result["structuredContent"].get("data_base64").is_none());
    assert!(result["structuredContent"].get("screenshot_path").is_none());
    assert_eq!(
        service.take_requests()[0],
        ServiceRequest::Browser {
            request: BrowserRequest::Screenshot {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                include_image_data: false,
            },
        }
    );
}

#[test]
fn browser_screenshot_with_empty_data_reports_error() {
    let service = FakeService::with_response(browser_service_response!(Screenshot {
        response: sky_cua_platform::model::BrowserScreenshotResponse {
            target: BrowserTargetKind::UserChrome,
            tab_id: "123".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            screenshot_path: None,
            width: None,
            height: None,
            diagnostics: vec![DiagnosticEntry {
                code: "BrowserBridgeRequestFailed".to_string(),
                message: "Browser screenshot CDP response did not include image data.".to_string(),
                details: None,
            }],
        },
    }));

    let result = handle_tool_call(
        &service,
        &HeuristicsRegistry::load_from_repo().expect("heuristics load"),
        &ModelSessionInfo {
            supports_images: Some(true),
        },
        "browser_screenshot",
        json!({"target": "user_chrome", "tab_id": "123"}),
    )
    .unwrap();

    assert_eq!(result["isError"], true);
    assert_eq!(result["content"].as_array().unwrap().len(), 1);
}
