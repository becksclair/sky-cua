use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::config::BrowserControlMode;

use super::browser::BrowserRequestContext;

use super::{
    AccessibilitySetupReport, ActionOutcome, ActionRequest, AgentCursorCapabilities,
    AgentCursorState, AppInfo, AppSelector, AppStateSnapshot, BrowserRequest, BrowserResponse,
    BrowserSessionIdentity, CaptureScreenMode, DiagnosticEntry, DisplayTarget, DoctorReport,
    EnvironmentInfo, InputBackendKind, PhoneRequest, PhoneResponse, SessionPresenceIntent,
    SessionPresenceStatus, WindowInfo, WindowTarget, WindowTargetingSetupReport,
};

pub const CUA_SERVICE_PROTOCOL_VERSION: u32 = 1;
pub const CUA_SERVICE_VERSION: &str = "0.1.0";
pub const CUA_SERVICE_MAX_DEADLINE_MS: u32 = 30_000;
pub const CUA_SERVICE_DEFAULT_MOUSE_SIZE_PX: u32 = 12;
pub const BROWSER_CONTROL_CAPABILITY_V1: &str = "browser_control.v1";
const BROWSER_CONTROL_MODE_CAPABILITY_PREFIX: &str = "browser_control.mode=";

pub const CUA_SERVICE_CAPABILITIES: &[&str] = &[
    "action.held_key",
    "action.post_action_sleep_ms",
    "linux.activate_window",
    "linux.click",
    "linux.click.button",
    "linux.click.count",
    "linux.drag",
    "linux.get_screenshot",
    "linux.move",
    "linux.press_key",
    "linux.scroll",
    "linux.scroll.direction",
    "linux.scroll.origin",
    "linux.scroll.pixels",
    "linux.type_text",
    "screen.cursor_size",
    "screenshot.webp",
    "transport.max_frame_64_mib",
    "transport.ndjson",
    "turn.cancel",
];

#[must_use]
pub fn cua_service_capabilities() -> Vec<String> {
    CUA_SERVICE_CAPABILITIES
        .iter()
        .map(|capability| (*capability).to_string())
        .collect()
}

#[must_use]
pub fn cua_service_capabilities_for_input_backend(backend: &InputBackendKind) -> Vec<String> {
    let supports_linux_input = matches!(
        backend,
        InputBackendKind::PortalRemoteDesktop
            | InputBackendKind::XTest
            | InputBackendKind::LinuxVirtualInput
    );
    CUA_SERVICE_CAPABILITIES
        .iter()
        .filter(|capability| {
            ((**capability == "linux.activate_window" || **capability == "linux.get_screenshot")
                || !capability.starts_with("linux.")
                || supports_linux_input)
                && (**capability != "linux.scroll.pixels"
                    || *backend == InputBackendKind::PortalRemoteDesktop)
        })
        .map(|capability| (*capability).to_string())
        .collect()
}

#[must_use]
pub fn browser_control_mode_capability(mode: BrowserControlMode) -> String {
    format!(
        "{BROWSER_CONTROL_MODE_CAPABILITY_PREFIX}{}",
        match mode {
            BrowserControlMode::Legacy => "legacy",
            BrowserControlMode::Hybrid => "hybrid",
            BrowserControlMode::Strict => "strict",
        }
    )
}

#[must_use]
pub fn browser_control_mode_from_capabilities(
    capabilities: &[String],
) -> Option<BrowserControlMode> {
    capabilities.iter().find_map(|capability| {
        match capability.strip_prefix(BROWSER_CONTROL_MODE_CAPABILITY_PREFIX)? {
            "legacy" => Some(BrowserControlMode::Legacy),
            "hybrid" => Some(BrowserControlMode::Hybrid),
            "strict" => Some(BrowserControlMode::Strict),
            _ => None,
        }
    })
}

fn default_cua_protocol_version() -> u32 {
    CUA_SERVICE_PROTOCOL_VERSION
}

fn default_cua_service_version() -> String {
    CUA_SERVICE_VERSION.to_string()
}

fn default_cua_capabilities() -> Vec<String> {
    cua_service_capabilities()
}

fn default_cua_deadline_ms() -> u32 {
    CUA_SERVICE_MAX_DEADLINE_MS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CuaRequestContext {
    pub session_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u32>,
}

impl CuaRequestContext {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.trim().is_empty() || self.turn_id.trim().is_empty() {
            return Err("session_id and turn_id must be non-empty");
        }
        if let Some(deadline_ms) = self.deadline_ms
            && !(1..=CUA_SERVICE_MAX_DEADLINE_MS).contains(&deadline_ms)
        {
            return Err("deadline_ms must be between 1 and 30000");
        }
        Ok(())
    }

    #[must_use]
    pub fn deadline_ms(&self) -> u32 {
        self.deadline_ms.unwrap_or_else(default_cua_deadline_ms)
    }

    #[must_use]
    pub fn turn_key(&self) -> (String, String) {
        (self.session_id.clone(), self.turn_id.clone())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CuaMouseButton {
    Left,
    Right,
    Middle,
    #[serde(rename = "l")]
    L,
    #[serde(rename = "r")]
    R,
    #[serde(rename = "m")]
    M,
}

impl CuaMouseButton {
    #[must_use]
    pub fn canonical(self) -> Self {
        match self {
            Self::L => Self::Left,
            Self::R => Self::Right,
            Self::M => Self::Middle,
            button => button,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CuaScrollDirection {
    Up,
    Down,
    Left,
    Right,
    #[serde(rename = "u")]
    U,
    #[serde(rename = "d")]
    D,
    #[serde(rename = "l")]
    L,
    #[serde(rename = "r")]
    R,
}

impl CuaScrollDirection {
    #[must_use]
    pub fn canonical(self) -> Self {
        match self {
            Self::U => Self::Up,
            Self::D => Self::Down,
            Self::L => Self::Left,
            Self::R => Self::Right,
            direction => direction,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CuaActionRequest {
    Click {
        context: CuaRequestContext,
        x: f64,
        y: f64,
        mouse_button: Option<CuaMouseButton>,
        click_count: Option<u32>,
        key: Option<String>,
        post_action_sleep_ms: Option<u32>,
    },
    Drag {
        context: CuaRequestContext,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        key: Option<String>,
        post_action_sleep_ms: Option<u32>,
    },
    Move {
        context: CuaRequestContext,
        x: f64,
        y: f64,
        key: Option<String>,
        post_action_sleep_ms: Option<u32>,
    },
    PressKey {
        context: CuaRequestContext,
        key: String,
        post_action_sleep_ms: Option<u32>,
    },
    Scroll {
        context: CuaRequestContext,
        direction: CuaScrollDirection,
        pixels: Option<u32>,
        x: Option<f64>,
        y: Option<f64>,
        key: Option<String>,
        post_action_sleep_ms: Option<u32>,
    },
    TypeText {
        context: CuaRequestContext,
        text: String,
        post_action_sleep_ms: Option<u32>,
    },
}

impl CuaActionRequest {
    #[must_use]
    pub fn context(&self) -> &CuaRequestContext {
        match self {
            Self::Click { context, .. }
            | Self::Drag { context, .. }
            | Self::Move { context, .. }
            | Self::PressKey { context, .. }
            | Self::Scroll { context, .. }
            | Self::TypeText { context, .. } => context,
        }
    }

    #[must_use]
    pub fn post_action_sleep_ms(&self) -> Option<u32> {
        match self {
            Self::Click {
                post_action_sleep_ms,
                ..
            }
            | Self::Drag {
                post_action_sleep_ms,
                ..
            }
            | Self::Move {
                post_action_sleep_ms,
                ..
            }
            | Self::PressKey {
                post_action_sleep_ms,
                ..
            }
            | Self::Scroll {
                post_action_sleep_ms,
                ..
            }
            | Self::TypeText {
                post_action_sleep_ms,
                ..
            } => *post_action_sleep_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CuaCancelStatus {
    CancelRequested,
    AlreadyCancelled,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CuaScreenshot {
    pub filepath: String,
    pub bytes_base64: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CuaBackendResponse {
    Action,
    Screenshots(Vec<CuaScreenshot>),
}

#[derive(Debug, Clone)]
pub struct CuaCancellation(Arc<AtomicBool>);

impl CuaCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

impl Default for CuaCancellation {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceRequest {
    Health,
    Click {
        context: CuaRequestContext,
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mouse_button: Option<CuaMouseButton>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        click_count: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_action_sleep_ms: Option<u32>,
    },
    Drag {
        context: CuaRequestContext,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_action_sleep_ms: Option<u32>,
    },
    GetScreenshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<CuaRequestContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mouse_size_px: Option<u32>,
    },
    Move {
        context: CuaRequestContext,
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_action_sleep_ms: Option<u32>,
    },
    PressKey {
        context: CuaRequestContext,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_action_sleep_ms: Option<u32>,
    },
    Scroll {
        context: CuaRequestContext,
        direction: CuaScrollDirection,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pixels: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_action_sleep_ms: Option<u32>,
    },
    TypeText {
        context: CuaRequestContext,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_action_sleep_ms: Option<u32>,
    },
    CancelTurn {
        session_id: String,
        turn_id: String,
        reason: String,
    },
    Doctor,
    SetupAccessibility,
    SetupWindowTargeting,
    LaunchApplication {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    ListApps,
    ListWindows,
    FocusedWindow,
    ActivateWindow {
        #[serde(default)]
        target: WindowTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<CuaRequestContext>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<BrowserSessionIdentity>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<BrowserRequestContext>,
    },
    CancelBrowserOperation {
        connection_id: String,
        operation_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    BrowserClientDisconnected {
        connection_id: String,
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

impl ServiceRequest {
    /// Whether re-sending this exact request after an ambiguous failure (one
    /// where the daemon may already have received and executed it) is safe.
    ///
    /// Idempotent requests converge to the same observable state no matter how
    /// many times they run (reads, status/listing, focus-set, cursor-set,
    /// setup convergence). Non-idempotent requests perform an action whose
    /// effect compounds on repetition (a click, a keystroke, launching a
    /// process) — retrying those blind after a lost response can double-execute
    /// them. New variants are forced through this match at compile time so an
    /// addition can never silently default to "safe to retry".
    #[must_use]
    pub fn is_idempotent(&self) -> bool {
        match self {
            Self::Health
            | Self::GetScreenshot { .. }
            | Self::Doctor
            | Self::ListApps
            | Self::ListWindows
            | Self::FocusedWindow
            | Self::GetAppState { .. }
            | Self::Screenshot { .. }
            | Self::AgentCursorStatus
            | Self::SetAgentCursor { .. }
            | Self::HideAgentCursor { .. }
            | Self::ShowAgentCursor
            | Self::SetupAccessibility
            | Self::SetupWindowTargeting
            | Self::CancelBrowserOperation { .. }
            | Self::BrowserClientDisconnected { .. }
            // Focus-set converges: activating the same window twice ends in
            // the same focused state as activating it once.
            | Self::ActivateWindow { .. } => true,
            Self::Move { .. } => true,
            Self::LaunchApplication { .. }
            | Self::ResetPortalTokens
            | Self::SessionPresence { .. }
            | Self::ExecuteAction { .. }
            | Self::Click { .. }
            | Self::Drag { .. }
            | Self::PressKey { .. }
            | Self::Scroll { .. }
            | Self::TypeText { .. } => false,
            Self::CancelTurn { .. } => true,
            Self::Browser { request, .. } => request.is_idempotent(),
            Self::Phone { request } => request.is_idempotent(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
// boxing would churn the wire/model contract; revisit if size matters
#[allow(clippy::large_enum_variant)]
pub enum ServiceResponse {
    Health {
        ok: bool,
        service_socket: String,
        #[serde(default = "default_cua_protocol_version")]
        protocol_version: u32,
        #[serde(default = "default_cua_service_version")]
        service_version: String,
        #[serde(default = "default_cua_capabilities")]
        capabilities: Vec<String>,
        #[serde(default)]
        desktop_env: BTreeMap<String, String>,
        #[serde(default)]
        browser_env: BTreeMap<String, String>,
    },
    Click {
        ok: bool,
        session_id: String,
        turn_id: String,
    },
    Drag {
        ok: bool,
        session_id: String,
        turn_id: String,
    },
    GetScreenshot {
        ok: bool,
        screenshots: Vec<CuaScreenshot>,
    },
    Move {
        ok: bool,
        session_id: String,
        turn_id: String,
    },
    PressKey {
        ok: bool,
        session_id: String,
        turn_id: String,
    },
    Scroll {
        ok: bool,
        session_id: String,
        turn_id: String,
    },
    TypeText {
        ok: bool,
        session_id: String,
        turn_id: String,
    },
    CancelTurn {
        ok: bool,
        session_id: String,
        turn_id: String,
        status: CuaCancelStatus,
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
    LaunchApplication {
        pid: u32,
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
        #[serde(default, skip_serializing)]
        ok: bool,
        code: String,
        message: String,
        #[serde(default, skip_serializing)]
        session_id: Option<String>,
        #[serde(default, skip_serializing)]
        turn_id: Option<String>,
        #[serde(default, skip_serializing)]
        retry: Option<String>,
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
        PhoneResponse, PhoneSessionSelector, PhoneStatusReport, PhoneStatusRequest,
        PhoneTapRequest, WindowTargetingSetupReport,
    };
    use serde_json::json;

    #[test]
    fn browser_control_mode_capability_round_trips_and_ignores_legacy_health() {
        for mode in [
            BrowserControlMode::Legacy,
            BrowserControlMode::Hybrid,
            BrowserControlMode::Strict,
        ] {
            let capabilities = vec![
                BROWSER_CONTROL_CAPABILITY_V1.to_owned(),
                browser_control_mode_capability(mode),
            ];
            assert_eq!(
                browser_control_mode_from_capabilities(&capabilities),
                Some(mode)
            );
        }
        assert_eq!(browser_control_mode_from_capabilities(&[]), None);
    }

    #[test]
    fn service_request_idempotency_matches_the_classification_table() {
        let idempotent = [
            ServiceRequest::Health,
            ServiceRequest::Doctor,
            ServiceRequest::SetupAccessibility,
            ServiceRequest::SetupWindowTargeting,
            ServiceRequest::ListApps,
            ServiceRequest::ListWindows,
            ServiceRequest::FocusedWindow,
            ServiceRequest::ActivateWindow {
                target: WindowTarget::default(),
                context: None,
            },
            ServiceRequest::GetAppState {
                selector: None,
                capture_screen: CaptureScreenMode::default(),
            },
            ServiceRequest::Screenshot {
                target: None,
                display_target: None,
            },
            ServiceRequest::AgentCursorStatus,
            ServiceRequest::SetAgentCursor {
                state: cursor_state(),
            },
            ServiceRequest::HideAgentCursor { reason: None },
            ServiceRequest::ShowAgentCursor,
            ServiceRequest::Browser {
                request: BrowserRequest::Status,
                identity: None,
                context: None,
            },
            ServiceRequest::Phone {
                request: PhoneRequest::Status(PhoneStatusRequest::default()),
            },
        ];
        for request in idempotent {
            assert!(request.is_idempotent(), "expected idempotent: {request:?}");
        }

        let non_idempotent = [
            ServiceRequest::LaunchApplication {
                command: "kcalc".to_string(),
                args: Vec::new(),
            },
            ServiceRequest::ResetPortalTokens,
            ServiceRequest::SessionPresence {
                action: SessionPresenceAction::Status,
            },
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
            ServiceRequest::Browser {
                identity: None,
                context: None,
                request: BrowserRequest::Click {
                    target: Some(BrowserTargetKind::UserChrome),
                    tab_id: "123".to_string(),
                    x: 10.0,
                    y: 10.0,
                },
            },
            ServiceRequest::Phone {
                request: PhoneRequest::Tap(PhoneTapRequest {
                    session: PhoneSessionSelector::default(),
                    phone_snapshot_id: None,
                    x: 10.0,
                    y: 10.0,
                    use_device_coordinates: false,
                }),
            },
        ];
        for request in non_idempotent {
            assert!(
                !request.is_idempotent(),
                "expected non-idempotent: {request:?}"
            );
        }
    }

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
            (
                ServiceRequest::LaunchApplication {
                    command: "kcalc".to_string(),
                    args: vec!["--help".to_string()],
                },
                "launch_application",
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
                    context: None,
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
                    identity: None,
                    context: None,
                },
                "browser",
            ),
            (
                ServiceRequest::Browser {
                    identity: None,
                    context: None,
                    request: BrowserRequest::ListTabs {
                        target: Some(BrowserTargetKind::UserChrome),
                    },
                },
                "browser",
            ),
            (
                ServiceRequest::Browser {
                    identity: None,
                    context: None,
                    request: BrowserRequest::Open {
                        target: Some(BrowserTargetKind::UserChrome),
                        url: Some("https://example.test/".to_string()),
                    },
                },
                "browser",
            ),
            (
                ServiceRequest::Browser {
                    identity: None,
                    context: None,
                    request: BrowserRequest::ClaimTab {
                        target: Some(BrowserTargetKind::UserChrome),
                        tab_id: "123".to_string(),
                    },
                },
                "browser",
            ),
            (
                ServiceRequest::Browser {
                    identity: None,
                    context: None,
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
            identity: None,
            context: None,
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
    fn browser_cancellation_and_disconnect_requests_have_stable_wire_shapes() {
        let cancellation = ServiceRequest::CancelBrowserOperation {
            connection_id: "mcp-connection".to_string(),
            operation_id: "op-mcp-connection-0001".to_string(),
            reason: Some("caller cancelled".to_string()),
        };
        let rendered = serde_json::to_value(&cancellation).expect("cancellation serializes");
        assert_eq!(
            rendered,
            json!({
                "type": "cancel_browser_operation",
                "connection_id": "mcp-connection",
                "operation_id": "op-mcp-connection-0001",
                "reason": "caller cancelled"
            })
        );
        assert_eq!(
            serde_json::from_value::<ServiceRequest>(rendered).expect("cancellation round trip"),
            cancellation
        );

        let without_reason = serde_json::to_value(ServiceRequest::CancelBrowserOperation {
            connection_id: "mcp-connection".to_string(),
            operation_id: "op-mcp-connection-0002".to_string(),
            reason: None,
        })
        .expect("reasonless cancellation serializes");
        assert!(without_reason.get("reason").is_none());

        let disconnected = ServiceRequest::BrowserClientDisconnected {
            connection_id: "mcp-connection".to_string(),
        };
        let rendered = serde_json::to_value(&disconnected).expect("disconnect serializes");
        assert_eq!(
            rendered,
            json!({
                "type": "browser_client_disconnected",
                "connection_id": "mcp-connection"
            })
        );
        assert_eq!(
            serde_json::from_value::<ServiceRequest>(rendered).expect("disconnect round trip"),
            disconnected
        );
        assert!(cancellation.is_idempotent());
        assert!(disconnected.is_idempotent());
    }

    #[test]
    fn browser_service_request_context_round_trips_and_legacy_request_stays_readable() {
        let request = ServiceRequest::Browser {
            request: BrowserRequest::Status,
            identity: Some(BrowserSessionIdentity {
                session_id: "codex-session".to_string(),
                turn_id: "codex-turn".to_string(),
                thread_id: Some("codex-thread".to_string()),
            }),
            context: Some(BrowserRequestContext {
                provenance: super::super::browser::BrowserCallerProvenance {
                    caller: super::super::browser::BrowserCallerKind::CodexDesktop,
                    source: super::super::browser::BrowserProvenanceSource::InstallerDeclaration,
                    connection_id: "mcp-connection".to_string(),
                    declared_caller: Some("codex_desktop".to_string()),
                    client_info: Some(super::super::browser::BrowserMcpClientInfo {
                        name: "codex".to_string(),
                        version: "1.2.3".to_string(),
                        title: Some("Codex Desktop".to_string()),
                    }),
                },
                logical_identity: super::super::browser::BrowserLogicalIdentity {
                    session_id: "codex-session".to_string(),
                    thread_id: Some("codex-thread".to_string()),
                    turn_id: Some("codex-turn".to_string()),
                },
                operation_identity: super::super::browser::BrowserOperationIdentity {
                    operation_id: "mcp:mcp-connection:abcd".to_string(),
                    request_id_fingerprint: "abcd".to_string(),
                },
            }),
        };

        let rendered = serde_json::to_value(&request).expect("browser context should serialize");
        assert_eq!(rendered["identity"]["session_id"], "codex-session");
        assert_eq!(rendered["context"]["provenance"]["caller"], "codex_desktop");
        assert_eq!(
            rendered["context"]["provenance"]["connection_id"],
            "mcp-connection"
        );
        assert!(
            rendered["context"]["provenance"]
                .get("mcp_connection_id")
                .is_none()
        );
        assert_eq!(
            rendered["context"]["provenance"]["client_info"]["title"],
            "Codex Desktop"
        );
        assert_eq!(
            rendered["context"]["operation_identity"]["operation_id"],
            "mcp:mcp-connection:abcd"
        );
        assert_eq!(
            serde_json::from_value::<ServiceRequest>(rendered).expect("new request round trip"),
            request
        );

        let legacy = serde_json::from_value::<ServiceRequest>(json!({
            "type": "browser",
            "request": { "type": "status" },
            "identity": {
                "session_id": "legacy-session",
                "turn_id": "legacy-turn"
            }
        }))
        .expect("legacy browser request should remain readable");
        assert!(matches!(
            legacy,
            ServiceRequest::Browser {
                context: None,
                identity: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn activate_window_context_round_trips_and_legacy_request_stays_readable() {
        let context = CuaRequestContext {
            session_id: "node-repl-session".to_string(),
            turn_id: "node-repl-turn".to_string(),
            deadline_ms: Some(1_234),
        };
        let request = ServiceRequest::ActivateWindow {
            target: WindowTarget {
                window_id: Some("window-1".to_string()),
                ..Default::default()
            },
            context: Some(context.clone()),
        };

        let rendered = serde_json::to_value(&request).expect("window context should serialize");
        assert_eq!(rendered["context"]["session_id"], "node-repl-session");
        assert_eq!(rendered["context"]["turn_id"], "node-repl-turn");
        assert_eq!(rendered["context"]["deadline_ms"], 1_234);
        assert_eq!(
            serde_json::from_value::<ServiceRequest>(rendered)
                .expect("context-bearing window request should round trip"),
            request
        );

        let legacy = serde_json::from_value::<ServiceRequest>(json!({
            "type": "activate_window",
            "target": { "window_id": "legacy-window" }
        }))
        .expect("legacy context-free window request should remain readable");
        assert_eq!(
            legacy,
            ServiceRequest::ActivateWindow {
                target: WindowTarget {
                    window_id: Some("legacy-window".to_string()),
                    ..Default::default()
                },
                context: None,
            }
        );
        let rendered_legacy =
            serde_json::to_value(legacy).expect("legacy window request should serialize");
        assert!(rendered_legacy.get("context").is_none());
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
                    protocol_version: CUA_SERVICE_PROTOCOL_VERSION,
                    service_version: CUA_SERVICE_VERSION.to_string(),
                    capabilities: cua_service_capabilities(),
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
                ServiceResponse::LaunchApplication { pid: 4321 },
                "launch_application",
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
                    ok: false,
                    code: "Failed".to_string(),
                    message: "boom".to_string(),
                    session_id: None,
                    turn_id: None,
                    retry: None,
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

    #[test]
    fn cua_service_fixture_and_backward_health_shape_round_trip() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/service-protocol-cua-js.json"
        )))
        .expect("frozen CUA service fixture must remain valid JSON");
        assert_eq!(fixture["schema_version"], 1);
        assert_eq!(fixture["protocol"]["version"], CUA_SERVICE_PROTOCOL_VERSION);
        assert_eq!(fixture["wire"]["max_frame_bytes"], 64 * 1024 * 1024);
        assert_eq!(
            fixture["health"]["example"]["capabilities"],
            serde_json::to_value(cua_service_capabilities()).expect("capabilities serialize")
        );

        let health_request: ServiceRequest =
            serde_json::from_value(fixture["health"]["request"].clone())
                .expect("health request should decode");
        assert_eq!(health_request, ServiceRequest::Health);

        let health_response: ServiceResponse =
            serde_json::from_value(fixture["health"]["example"].clone())
                .expect("health response example should decode");
        let ServiceResponse::Health {
            protocol_version,
            service_version,
            capabilities,
            ..
        } = health_response
        else {
            panic!("fixture health response should use the health tag");
        };
        assert_eq!(protocol_version, CUA_SERVICE_PROTOCOL_VERSION);
        assert_eq!(service_version, CUA_SERVICE_VERSION);
        assert_eq!(capabilities, cua_service_capabilities());

        let old_health: ServiceResponse = serde_json::from_value(json!({
            "type": "health",
            "ok": true,
            "service_socket": "/tmp/sky-cua/service.sock"
        }))
        .expect("old health response must remain decodable");
        let ServiceResponse::Health {
            protocol_version,
            service_version,
            capabilities,
            ..
        } = old_health
        else {
            panic!("old health response should use the health tag");
        };
        assert_eq!(protocol_version, CUA_SERVICE_PROTOCOL_VERSION);
        assert_eq!(service_version, CUA_SERVICE_VERSION);
        assert_eq!(capabilities, cua_service_capabilities());
    }

    #[test]
    fn cua_screenshot_serializes_one_base64_payload_and_accepts_legacy_data_url() {
        let screenshot = CuaScreenshot {
            filepath: "/tmp/capture.webp".to_string(),
            bytes_base64: "UklGRg==".to_string(),
            mime_type: "image/webp".to_string(),
            width: 800,
            height: 600,
        };
        let rendered = serde_json::to_value(&screenshot).expect("screenshot should serialize");
        assert_eq!(rendered["bytes_base64"], "UklGRg==");
        assert!(rendered.get("data_url").is_none());

        let legacy: CuaScreenshot = serde_json::from_value(json!({
            "filepath": "/tmp/capture.webp",
            "bytes_base64": "UklGRg==",
            "data_url": "data:image/webp;base64,UklGRg==",
            "mime_type": "image/webp",
            "width": 800,
            "height": 600
        }))
        .expect("legacy duplicated data_url should remain accepted");
        assert_eq!(legacy, screenshot);
    }

    #[test]
    fn cua_requests_encode_context_and_aliases_without_wire_drift() {
        let context = CuaRequestContext {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            deadline_ms: Some(30_000),
        };
        let rendered = serde_json::to_value(ServiceRequest::Click {
            context: context.clone(),
            x: 10.5,
            y: 20.25,
            mouse_button: Some(CuaMouseButton::M),
            click_count: Some(2),
            key: Some("Ctrl".to_string()),
            post_action_sleep_ms: Some(0),
        })
        .expect("CUA click should serialize");
        assert_eq!(rendered["type"], "click");
        assert_eq!(rendered["context"]["session_id"], "session-1");
        assert_eq!(rendered["context"]["deadline_ms"], 30_000);
        assert_eq!(rendered["mouse_button"], "m");
        assert_eq!(rendered["click_count"], 2);

        let cancel: ServiceRequest = serde_json::from_value(json!({
            "type": "cancel_turn",
            "session_id": "session-1",
            "turn_id": "turn-1",
            "reason": "deadline"
        }))
        .expect("CancelTurn should decode");
        assert!(cancel.is_idempotent());
        assert!(
            ServiceRequest::Move {
                context,
                x: 1.0,
                y: 2.0,
                key: None,
                post_action_sleep_ms: None,
            }
            .is_idempotent()
        );
    }

    #[test]
    fn cua_scroll_pixel_capability_tracks_active_input_backend() {
        let portal =
            cua_service_capabilities_for_input_backend(&InputBackendKind::PortalRemoteDesktop);
        assert!(portal.iter().any(|value| value == "linux.scroll.pixels"));

        for backend in [InputBackendKind::XTest, InputBackendKind::LinuxVirtualInput] {
            let capabilities = cua_service_capabilities_for_input_backend(&backend);
            assert!(
                !capabilities
                    .iter()
                    .any(|value| value == "linux.scroll.pixels")
            );
            assert!(
                capabilities
                    .iter()
                    .any(|value| value == "linux.scroll.direction")
            );
        }

        let unavailable = cua_service_capabilities_for_input_backend(&InputBackendKind::None);
        assert!(!unavailable.iter().any(|value| value == "linux.scroll"));
        assert!(
            unavailable
                .iter()
                .any(|value| value == "linux.activate_window")
        );
        assert!(
            unavailable
                .iter()
                .any(|value| value == "linux.get_screenshot")
        );
    }

    #[test]
    fn protocol_v1_error_serialization_preserves_legacy_shape() {
        let rendered = serde_json::to_value(ServiceResponse::Error {
            ok: false,
            code: "SKY_CUA_INVALID_CONTEXT".to_string(),
            message: "invalid".to_string(),
            session_id: Some("session".to_string()),
            turn_id: Some("turn".to_string()),
            retry: Some("never".to_string()),
        })
        .expect("error response should serialize");
        assert_eq!(
            rendered,
            json!({
                "type": "error",
                "code": "SKY_CUA_INVALID_CONTEXT",
                "message": "invalid"
            })
        );
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
                    control_plane: None,
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
