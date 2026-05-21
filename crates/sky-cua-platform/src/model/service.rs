use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    AccessibilitySetupReport, ActionOutcome, ActionRequest, AgentCursorCapabilities,
    AgentCursorState, AppInfo, AppSelector, AppStateSnapshot, CaptureScreenMode, DiagnosticEntry,
    DoctorReport, EnvironmentInfo, WindowInfo, WindowTarget, WindowTargetingSetupReport,
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
    ExecuteAction {
        outcome: ActionOutcome,
    },
    Error {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{
        action_outcome, app_state_snapshot, cursor_capabilities, cursor_state, doctor_report,
        environment_info, setup_command_report, window_info,
    };
    use super::*;
    use crate::{AccessibilitySetupReport, ActionName, WindowTargetingSetupReport};
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
}
