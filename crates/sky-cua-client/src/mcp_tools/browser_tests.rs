use std::cell::RefCell;
use std::collections::VecDeque;

use serde_json::json;
use sky_cua_platform::model::{
    BrowserActionResponse, BrowserClaimTabResponse, BrowserListTabsResponse,
    BrowserMoveMouseResponse, BrowserOpenResponse, BrowserRequest, BrowserResponse,
    BrowserSnapshotResponse, BrowserStatusReport, BrowserTab, BrowserTargetAvailability,
    BrowserTargetKind, DiagnosticEntry, ServiceRequest, ServiceResponse,
};

use crate::heuristics::HeuristicsRegistry;
use crate::mcp_server::ModelSessionInfo;

use super::browser::{
    BrowserTabTextFilter, browser_list_tabs_summary, browser_open_summary,
    browser_snapshot_summary, browser_status_summary, parse_browser_open_url, parse_browser_point,
    parse_browser_scroll, parse_browser_tab_id, parse_browser_target, parse_required_browser_url,
    parse_required_literal_string, parse_required_string,
};
use super::{McpService, build_tool_definitions, handle_tool_call};

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
    let definitions = build_tool_definitions(false);
    let names = tool_names(&definitions);
    assert!(names.contains(&"browser_status"));
    assert!(names.contains(&"browser_list_tabs"));
    assert!(names.contains(&"browser_open"));
    assert!(names.contains(&"browser_claim_tab"));
    assert!(names.contains(&"browser_move_mouse"));
    assert!(names.contains(&"browser_navigate"));
    assert!(names.contains(&"browser_snapshot"));
    assert!(names.contains(&"browser_screenshot"));
    assert!(names.contains(&"browser_click"));
    assert!(names.contains(&"browser_type_text"));
    assert!(names.contains(&"browser_press_key"));
    assert!(names.contains(&"browser_scroll"));
}

#[test]
fn browser_scroll_schema_matches_delta_defaults() {
    let definitions = build_tool_definitions(false);
    let scroll_tool = definitions
        .as_array()
        .expect("tools should be an array")
        .iter()
        .find(|tool| tool["name"] == "browser_scroll")
        .expect("browser_scroll tool is advertised");

    assert_eq!(scroll_tool["inputSchema"]["required"], json!(["tab_id"]));
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
        parse_browser_open_url(&json!({"url": " http://127.0.0.1:8080/page "})).unwrap(),
        Some("http://127.0.0.1:8080/page".to_string())
    );
    assert_eq!(
        parse_browser_open_url(&json!({"url": "about:blank"})).unwrap(),
        Some("about:blank".to_string())
    );
    assert_eq!(parse_browser_open_url(&json!({"url": ""})).unwrap(), None);
    assert_eq!(parse_browser_open_url(&json!({"url": null})).unwrap(), None);
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
    assert_eq!(
        parse_browser_tab_id(&json!({"tab_id": 456})).unwrap(),
        "456"
    );
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
    assert_eq!(
        parse_required_browser_url(
            &json!({"url": " https://example.test/ "}),
            "browser_navigate"
        )
        .unwrap(),
        "https://example.test/"
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
        (0.0, 500.0, 10.0, 20.0)
    );
    assert!(parse_browser_scroll(&json!({"delta_x": 0, "delta_y": 0})).is_err());
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
            target: BrowserTargetKind::Managed,
            available: true,
            detail: "Available browser binaries: chromium.".to_string(),
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
    assert!(summary.contains("managed=available"));
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
    assert!(summary.contains("devicePixelRatio=1.25"));
    assert!(summary.contains("Visible text: \"New session Update Available"));
    assert!(summary.contains("[3] tag=button role=button name=\"Update Available\""));
    assert!(summary.contains("bounds=x:241.60 y:971.20 w:120 h:32"));
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
