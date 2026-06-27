use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    AccessibilitySetupReport, ActionOutcome, ActionRequest, AgentCursorCapabilities,
    AgentCursorState, AppInfo, AppSelector, AppStateSnapshot, BrowserRequest, BrowserResponse,
    CaptureScreenMode, DiagnosticEntry, DisplayTarget, DoctorReport, EnvironmentInfo, PhoneRequest,
    PhoneResponse, SessionPresenceIntent, SessionPresenceStatus, WindowInfo, WindowTarget,
    WindowTargetingSetupReport,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceRequest {
    Health,
    Doctor,
    SetupAccessibility,
    SetupWindowTargeting,
    ListApps,
    ListWindows,
    FocusedWindow,
    ActivateWindow {
        #[serde(default)]
        target: WindowTarget,
    },
    GetAppState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selector: Option<AppSelector>,
        #[serde(default, skip_serializing_if = "is_default_capture_screen")]
        capture_screen: CaptureScreenMode,
    },
    Screenshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<WindowTarget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_target: Option<DisplayTarget>,
    },
    ResetPortalTokens,
    AgentCursorStatus,
    SetAgentCursor {
        state: AgentCursorState,
    },
    HideAgentCursor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    ShowAgentCursor,
    Browser {
        request: BrowserRequest,
    },
    Phone {
        request: PhoneRequest,
    },
    SessionPresence {
        action: SessionPresenceAction,
    },
    ExecuteAction {
        request: Box<ActionRequest>,
    },
}

fn is_default_capture_screen(mode: &CaptureScreenMode) -> bool {
    *mode == CaptureScreenMode::default()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceResponse {
    Health {
        ok: bool,
        service_socket: String,
        #[serde(default)]
        desktop_env: BTreeMap<String, String>,
        #[serde(default)]
        browser_env: BTreeMap<String, String>,
    },
    Doctor {
        report: Box<DoctorReport>,
    },
    SetupAccessibility {
        report: Box<AccessibilitySetupReport>,
    },
    SetupWindowTargeting {
        report: Box<WindowTargetingSetupReport>,
    },
    ListApps {
        environment: EnvironmentInfo,
        apps: Vec<AppInfo>,
        diagnostics: Vec<DiagnosticEntry>,
    },
    ListWindows {
        environment: EnvironmentInfo,
        windows: Vec<WindowInfo>,
        diagnostics: Vec<DiagnosticEntry>,
    },
    FocusedWindow {
        environment: EnvironmentInfo,
        window: Option<Box<WindowInfo>>,
        diagnostics: Vec<DiagnosticEntry>,
    },
    ActivateWindow {
        outcome: ActionOutcome,
    },
    GetAppState {
        snapshot: Box<AppStateSnapshot>,
    },
    Screenshot {
        snapshot: Box<AppStateSnapshot>,
    },
    ResetPortalTokens {
        cleared: bool,
        token_path: String,
        dropped_cached_session: bool,
    },
    AgentCursorStatus {
        capabilities: AgentCursorCapabilities,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<AgentCursorState>,
        diagnostics: Vec<DiagnosticEntry>,
    },
    SetAgentCursor {
        capabilities: AgentCursorCapabilities,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<AgentCursorState>,
        diagnostics: Vec<DiagnosticEntry>,
    },
    HideAgentCursor {
        capabilities: AgentCursorCapabilities,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<AgentCursorState>,
        diagnostics: Vec<DiagnosticEntry>,
    },
    ShowAgentCursor {
        capabilities: AgentCursorCapabilities,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<AgentCursorState>,
        diagnostics: Vec<DiagnosticEntry>,
    },
    Browser {
        response: BrowserResponse,
    },
    Phone {
        response: PhoneResponse,
    },
    SessionPresence {
        status: SessionPresenceStatus,
    },
    ExecuteAction {
        outcome: ActionOutcome,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionPresenceAction {
    Ensure(SessionPresenceIntent),
    Release {
        #[serde(default)]
        relock: bool,
    },
    Status,
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{
        action_outcome, app_state_snapshot, cursor_capabilities, cursor_state, doctor_report,
        environment_info, setup_command_report, window_info,
    };
    use super::*;
    use crate::{
        AccessibilitySetupReport, ActionName, BrowserClaimTabResponse, BrowserListTabsResponse,
        BrowserMoveMouseResponse, BrowserOpenResponse, BrowserRequest, BrowserResponse,
        BrowserStatusReport, BrowserTab, BrowserTargetAvailability, BrowserTargetKind,
        PhoneBackendKind, PhoneListDevicesRequest, PhoneListDevicesResponse, PhoneRequest,
        PhoneResponse, PhoneStatusReport, PhoneStatusRequest, WindowTargetingSetupReport,
    };
    use serde_json::json;

    #[test]
    fn service_request_variants_preserve_type_tags() {
        let requests = [
            (ServiceRequest::Health, "health"),
            (ServiceRequest::Doctor, "doctor"),
            (ServiceRequest::SetupAccessibility, "setup_accessibility"),
            (
                ServiceRequest::SetupWindowTargeting,
                "setup_window_targeting",
            ),
            (ServiceRequest::ListApps, "list_apps"),
            (ServiceRequest::ListWindows, "list_windows"),
            (ServiceRequest::FocusedWindow, "focused_window"),
            (
                ServiceRequest::ActivateWindow {
                    target: WindowTarget {
                        window_id: Some("w1".to_string()),
                        ..Default::default()
                    },
                },
                "activate_window",
            ),
            (
                ServiceRequest::GetAppState {
                    selector: None,
                    capture_screen: CaptureScreenMode::Always,
                },
                "get_app_state",
            ),
            (
                ServiceRequest::Screenshot {
                    target: Some(WindowTarget {
                        window_id: Some("w1".to_string()),
                        ..Default::default()
                    }),
                    display_target: None,
                },
                "screenshot",
            ),
            (ServiceRequest::ResetPortalTokens, "reset_portal_tokens"),
            (ServiceRequest::AgentCursorStatus, "agent_cursor_status"),
            (
                ServiceRequest::SetAgentCursor {
                    state: cursor_state(),
                },
                "set_agent_cursor",
            ),
            (
                ServiceRequest::HideAgentCursor {
                    reason: Some("test".to_string()),
                },
                "hide_agent_cursor",
            ),
            (ServiceRequest::ShowAgentCursor, "show_agent_cursor"),
            (
                ServiceRequest::Browser {
                    request: BrowserRequest::Status,
                },
                "browser",
            ),
            (
                ServiceRequest::Browser {
                    request: BrowserRequest::ListTabs {
                        target: Some(BrowserTargetKind::UserChrome),
                    },
                },
                "browser",
            ),
            (
                ServiceRequest::Browser {
                    request: BrowserRequest::Open {
                        target: Some(BrowserTargetKind::UserChrome),
                        url: Some("https://example.test/".to_string()),
                    },
                },
                "browser",
            ),
            (
                ServiceRequest::Browser {
                    request: BrowserRequest::ClaimTab {
                        target: Some(BrowserTargetKind::UserChrome),
                        tab_id: "123".to_string(),
                    },
                },
                "browser",
            ),
            (
                ServiceRequest::Browser {
                    request: BrowserRequest::MoveMouse {
                        target: Some(BrowserTargetKind::UserChrome),
                        tab_id: "123".to_string(),
                        x: 240.0,
                        y: 160.0,
                        wait_for_arrival: true,
                    },
                },
                "browser",
            ),
            (
                ServiceRequest::Phone {
                    request: PhoneRequest::Status(PhoneStatusRequest::default()),
                },
                "phone",
            ),
            (
                ServiceRequest::Phone {
                    request: PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
                },
                "phone",
            ),
            (
                ServiceRequest::SessionPresence {
                    action: SessionPresenceAction::Ensure(SessionPresenceIntent {
                        unlock: true,
                        inhibit_lock: true,
                        inhibit_suspend: true,
                    }),
                },
                "session_presence",
            ),
            (
                ServiceRequest::ExecuteAction {
                    request: Box::new(ActionRequest {
                        action: ActionName::Click,
                        snapshot_id: None,
                        element_index: None,
                        arguments: json!({}),
                        resolved_element: None,
                        resolved_target_element: None,
                        resolved_capture: None,
                        resolved_focused_app: None,
                        environment: None,
                    }),
                },
                "execute_action",
            ),
        ];

        for (request, expected_type) in requests {
            let rendered = serde_json::to_value(request).expect("request should serialize");
            assert_eq!(rendered["type"], expected_type);
        }
    }

    #[test]
    fn screenshot_request_serializes_display_selectors() {
        let rendered = serde_json::to_value(ServiceRequest::Screenshot {
            target: None,
            display_target: Some(DisplayTarget {
                display_id: Some("kwin:HDMI-A-1".to_string()),
                display_name: None,
                display_index: None,
            }),
        })
        .expect("screenshot request should serialize");

        assert_eq!(rendered["type"], "screenshot");
        assert_eq!(rendered["display_target"]["display_id"], "kwin:HDMI-A-1");
        assert!(rendered.get("capture_all_displays").is_none());

        let rendered = serde_json::to_value(ServiceRequest::Screenshot {
            target: None,
            display_target: None,
        })
        .expect("primary screenshot request should serialize");

        assert_eq!(rendered["type"], "screenshot");
        assert!(rendered.get("display_target").is_none());
    }

    #[test]
    fn browser_service_request_uses_nested_type_tag() {
        let rendered = serde_json::to_value(ServiceRequest::Browser {
            request: BrowserRequest::Open {
                target: Some(BrowserTargetKind::UserChrome),
                url: Some("https://example.test/".to_string()),
            },
        })
        .expect("browser request should serialize");

        assert_eq!(rendered["type"], "browser");
        assert_eq!(rendered["request"]["type"], "open");
        assert_eq!(rendered["request"]["target"], "user_chrome");
        assert_eq!(rendered["request"]["url"], "https://example.test/");
    }

    #[test]
    fn browser_scroll_request_preserves_optional_target_point() {
        let viewport_scroll = BrowserRequest::Scroll {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "123".to_string(),
            delta_x: 0.0,
            delta_y: 400.0,
            x: None,
            y: None,
        };
        let rendered =
            serde_json::to_value(&viewport_scroll).expect("browser request should serialize");
        assert!(rendered.get("x").is_none());
        assert!(rendered.get("y").is_none());

        let parsed: BrowserRequest = serde_json::from_value(json!({
            "type": "scroll",
            "target": "user_chrome",
            "tab_id": "123",
            "delta_x": 0.0,
            "delta_y": 400.0
        }))
        .expect("missing target point should deserialize");
        assert_eq!(parsed, viewport_scroll);

        let targeted_scroll = BrowserRequest::Scroll {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "123".to_string(),
            delta_x: 0.0,
            delta_y: 400.0,
            x: Some(10.0),
            y: Some(20.0),
        };
        let rendered =
            serde_json::to_value(targeted_scroll).expect("browser request should serialize");
        assert_eq!(rendered["x"], 10.0);
        assert_eq!(rendered["y"], 20.0);
    }

    #[test]
    fn browser_snapshot_request_preserves_optional_text_limit() {
        let default_snapshot = BrowserRequest::Snapshot {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "123".to_string(),
            text_limit: None,
            element_offset: None,
            element_limit: None,
            element_query: None,
        };
        let rendered =
            serde_json::to_value(&default_snapshot).expect("browser request should serialize");
        assert!(rendered.get("text_limit").is_none());
        assert!(rendered.get("element_offset").is_none());
        assert!(rendered.get("element_limit").is_none());
        assert!(rendered.get("element_query").is_none());

        let parsed: BrowserRequest = serde_json::from_value(json!({
            "type": "snapshot",
            "target": "user_chrome",
            "tab_id": "123"
        }))
        .expect("missing text_limit should deserialize");
        assert_eq!(parsed, default_snapshot);

        let limited_snapshot = BrowserRequest::Snapshot {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "123".to_string(),
            text_limit: Some(4000),
            element_offset: Some(5),
            element_limit: Some(25),
            element_query: Some("settings".to_string()),
        };
        let rendered =
            serde_json::to_value(limited_snapshot).expect("browser request should serialize");
        assert_eq!(rendered["text_limit"], 4000);
        assert_eq!(rendered["element_offset"], 5);
        assert_eq!(rendered["element_limit"], 25);
        assert_eq!(rendered["element_query"], "settings");
    }

    #[test]
    fn browser_screenshot_request_defaults_to_returning_image_data() {
        let default_screenshot = BrowserRequest::Screenshot {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "123".to_string(),
            include_image_data: true,
        };
        let rendered =
            serde_json::to_value(&default_screenshot).expect("browser request should serialize");
        assert!(rendered.get("include_image_data").is_none());

        let parsed: BrowserRequest = serde_json::from_value(json!({
            "type": "screenshot",
            "target": "user_chrome",
            "tab_id": "123"
        }))
        .expect("missing include_image_data should deserialize");
        assert_eq!(parsed, default_screenshot);

        let path_only_screenshot = BrowserRequest::Screenshot {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "123".to_string(),
            include_image_data: false,
        };
        let rendered =
            serde_json::to_value(path_only_screenshot).expect("browser request should serialize");
        assert_eq!(rendered["include_image_data"], false);
    }

    #[test]
    fn browser_move_mouse_request_defaults_to_waiting_for_arrival() {
        let parsed: BrowserRequest = serde_json::from_value(json!({
            "type": "move_mouse",
            "target": "user_chrome",
            "tab_id": "123",
            "x": 240.0,
            "y": 160.0
        }))
        .expect("missing wait_for_arrival should deserialize");

        assert_eq!(
            parsed,
            BrowserRequest::MoveMouse {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                x: 240.0,
                y: 160.0,
                wait_for_arrival: true,
            }
        );
    }

    #[test]
    fn service_response_variants_preserve_type_tags() {
        let environment = environment_info();
        let diagnostics = Vec::new();
        let cursor_capabilities = cursor_capabilities();
        let cursor_state = Some(cursor_state());
        let responses = [
            (
                ServiceResponse::Health {
                    ok: true,
                    service_socket: "/tmp/socket".to_string(),
                    desktop_env: Default::default(),
                    browser_env: Default::default(),
                },
                "health",
            ),
            (
                ServiceResponse::Doctor {
                    report: Box::new(doctor_report()),
                },
                "doctor",
            ),
            (
                ServiceResponse::SetupAccessibility {
                    report: Box::new(AccessibilitySetupReport {
                        before: Box::new(doctor_report()),
                        accessibility_command: setup_command_report(),
                        after: Box::new(doctor_report()),
                        changed: false,
                        requires_restart: false,
                    }),
                },
                "setup_accessibility",
            ),
            (
                ServiceResponse::SetupWindowTargeting {
                    report: Box::new(WindowTargetingSetupReport {
                        extension_dir: "/tmp/ext".to_string(),
                        wrote_files: false,
                        enable_command: setup_command_report(),
                        windows: Vec::new(),
                        windows_error: None,
                        requires_shell_reload: false,
                        message: "ok".to_string(),
                        permissions_hint: None,
                    }),
                },
                "setup_window_targeting",
            ),
            (
                ServiceResponse::ListApps {
                    environment: environment.clone(),
                    apps: Vec::new(),
                    diagnostics: diagnostics.clone(),
                },
                "list_apps",
            ),
            (
                ServiceResponse::ListWindows {
                    environment: environment.clone(),
                    windows: vec![window_info()],
                    diagnostics: diagnostics.clone(),
                },
                "list_windows",
            ),
            (
                ServiceResponse::FocusedWindow {
                    environment: environment.clone(),
                    window: Some(Box::new(window_info())),
                    diagnostics: diagnostics.clone(),
                },
                "focused_window",
            ),
            (
                ServiceResponse::ActivateWindow {
                    outcome: action_outcome(),
                },
                "activate_window",
            ),
            (
                ServiceResponse::GetAppState {
                    snapshot: Box::new(app_state_snapshot()),
                },
                "get_app_state",
            ),
            (
                ServiceResponse::Screenshot {
                    snapshot: Box::new(app_state_snapshot()),
                },
                "screenshot",
            ),
            (
                ServiceResponse::ResetPortalTokens {
                    cleared: true,
                    token_path: "/tmp/tokens".to_string(),
                    dropped_cached_session: true,
                },
                "reset_portal_tokens",
            ),
            (
                ServiceResponse::AgentCursorStatus {
                    capabilities: cursor_capabilities.clone(),
                    state: cursor_state.clone(),
                    diagnostics: diagnostics.clone(),
                },
                "agent_cursor_status",
            ),
            (
                ServiceResponse::SetAgentCursor {
                    capabilities: cursor_capabilities.clone(),
                    state: cursor_state.clone(),
                    diagnostics: diagnostics.clone(),
                },
                "set_agent_cursor",
            ),
            (
                ServiceResponse::HideAgentCursor {
                    capabilities: cursor_capabilities.clone(),
                    state: cursor_state.clone(),
                    diagnostics: diagnostics.clone(),
                },
                "hide_agent_cursor",
            ),
            (
                ServiceResponse::ShowAgentCursor {
                    capabilities: cursor_capabilities,
                    state: cursor_state,
                    diagnostics,
                },
                "show_agent_cursor",
            ),
            (browser_status_response(), "browser"),
            (
                ServiceResponse::Browser {
                    response: BrowserResponse::ListTabs {
                        response: BrowserListTabsResponse {
                            target: None,
                            tabs: Vec::new(),
                            diagnostics: Vec::new(),
                        },
                    },
                },
                "browser",
            ),
            (
                ServiceResponse::Browser {
                    response: BrowserResponse::Open {
                        response: BrowserOpenResponse {
                            target: BrowserTargetKind::UserChrome,
                            tab: Some(browser_tab()),
                            diagnostics: Vec::new(),
                        },
                    },
                },
                "browser",
            ),
            (
                ServiceResponse::Browser {
                    response: BrowserResponse::ClaimTab {
                        response: BrowserClaimTabResponse {
                            target: BrowserTargetKind::UserChrome,
                            tab: Some(browser_tab()),
                            diagnostics: Vec::new(),
                        },
                    },
                },
                "browser",
            ),
            (
                ServiceResponse::Browser {
                    response: BrowserResponse::MoveMouse {
                        response: BrowserMoveMouseResponse {
                            target: BrowserTargetKind::UserChrome,
                            tab_id: "tab-1".to_string(),
                            x: 240.0,
                            y: 160.0,
                            wait_for_arrival: true,
                            diagnostics: Vec::new(),
                        },
                    },
                },
                "browser",
            ),
            (phone_status_response(), "phone"),
            (
                ServiceResponse::Phone {
                    response: PhoneResponse::Devices(PhoneListDevicesResponse {
                        devices: Vec::new(),
                        adb_path: None,
                        adb_version: None,
                        diagnostics: Vec::new(),
                    }),
                },
                "phone",
            ),
            (
                ServiceResponse::SessionPresence {
                    status: SessionPresenceStatus::unsupported("none"),
                },
                "session_presence",
            ),
            (
                ServiceResponse::ExecuteAction {
                    outcome: action_outcome(),
                },
                "execute_action",
            ),
            (
                ServiceResponse::Error {
                    code: "Failed".to_string(),
                    message: "boom".to_string(),
                },
                "error",
            ),
        ];

        for (response, expected_type) in responses {
            let rendered = serde_json::to_value(response).expect("response should serialize");
            assert_eq!(rendered["type"], expected_type);
        }
    }

    #[test]
    fn browser_service_response_uses_nested_type_tag() {
        let rendered = serde_json::to_value(browser_status_response())
            .expect("browser response should serialize");

        assert_eq!(rendered["type"], "browser");
        assert_eq!(rendered["response"]["type"], "status");
        assert_eq!(rendered["response"]["report"]["enabled"], true);
    }

    #[test]
    fn phone_service_request_uses_nested_type_tag() {
        let rendered = serde_json::to_value(ServiceRequest::Phone {
            request: PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
        })
        .expect("phone request should serialize");

        assert_eq!(rendered["type"], "phone");
        assert_eq!(rendered["request"]["type"], "list_devices");
    }

    #[test]
    fn phone_service_response_uses_nested_type_tag() {
        let rendered =
            serde_json::to_value(phone_status_response()).expect("phone response should serialize");

        // `ServiceResponse::Phone` wraps the inner enum under `response`. The
        // inner `PhoneResponse` is an internally-tagged newtype enum, so the
        // payload's own fields flatten next to its `type` tag.
        assert_eq!(rendered["type"], "phone");
        assert_eq!(rendered["response"]["type"], "status");
        assert_eq!(rendered["response"]["enabled"], true);
    }

    fn phone_status_response() -> ServiceResponse {
        ServiceResponse::Phone {
            response: PhoneResponse::Status(PhoneStatusReport {
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
                diagnostics: Vec::new(),
            }),
        }
    }

    fn browser_status_response() -> ServiceResponse {
        ServiceResponse::Browser {
            response: BrowserResponse::Status {
                report: BrowserStatusReport {
                    enabled: true,
                    available_targets: vec![BrowserTargetAvailability {
                        target: BrowserTargetKind::UserChrome,
                        available: true,
                        detail: "ok".to_string(),
                    }],
                    tabs_known: Some(0),
                    browser_integration: None,
                    diagnostics: Vec::new(),
                },
            },
        }
    }

    fn browser_tab() -> BrowserTab {
        BrowserTab {
            tab_id: "tab-1".to_string(),
            target: BrowserTargetKind::UserChrome,
            title: Some("Example".to_string()),
            url: Some("https://example.test/".to_string()),
            active: true,
        }
    }
}
