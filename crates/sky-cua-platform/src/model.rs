use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Wayland,
    X11,
    Windows,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackendKind {
    PortalPipeWire,
    PortalScreenshot,
    X11,
    WindowsGdi,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputBackendKind {
    PortalRemoteDesktop,
    LinuxVirtualInput,
    XTest,
    SendInput,
    WindowsMessages,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticBackendKind {
    Atspi,
    Uia,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateSpace {
    DesktopLogical,
    StreamLogical,
    StreamPixels,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCursorBackendKind {
    None,
    ScreenshotSynthetic,
    WaylandLayerShell,
    KwinEffect,
    X11ShapedWindow,
    WindowsLayeredWindow,
    MacosPanel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentCursorSystemCursorBackendKind {
    #[default]
    None,
    Unsupported,
    WaylandClientUnsupported,
    X11Xfixes,
    KwinEffect,
    WindowsWin32,
    MacosNative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCursorPlane {
    UserVisible,
    ScreenshotSynthetic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCursorPoint {
    pub x: f64,
    pub y: f64,
    pub coordinate_space: CoordinateSpace,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCursorState {
    pub visible: bool,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_point: Option<AgentCursorPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_point: Option<AgentCursorPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action: Option<ActionName>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCursorCapabilities {
    pub backend: AgentCursorBackendKind,
    pub visible_overlay: bool,
    pub screenshot_synthetic_cursor: bool,
    pub click_through: bool,
    pub capture_exclusion: bool,
    #[serde(default)]
    pub system_cursor_hide_supported: bool,
    #[serde(default)]
    pub system_cursor_hidden: bool,
    #[serde(default)]
    pub system_cursor_backend: AgentCursorSystemCursorBackendKind,
    pub needs_user_install: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelImageFormat {
    Jpeg,
    Webp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureScreenMode {
    Auto,
    #[default]
    IfChanged,
    Always,
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub space: CoordinateSpace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolAvailability {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCapabilities {
    pub list_apps: ToolAvailability,
    pub get_app_state: ToolAvailability,
    pub focus_element: ToolAvailability,
    pub activate_element: ToolAvailability,
    pub select_element: ToolAvailability,
    pub expand_element: ToolAvailability,
    pub collapse_element: ToolAvailability,
    pub toggle_element: ToolAvailability,
    pub click: ToolAvailability,
    pub perform_action: ToolAvailability,
    pub perform_secondary_action: ToolAvailability,
    pub scroll: ToolAvailability,
    pub drag: ToolAvailability,
    pub type_text: ToolAvailability,
    pub press_key: ToolAvailability,
    pub set_value: ToolAvailability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortalCapabilities {
    pub screencast_version: Option<u32>,
    pub remote_desktop_version: Option<u32>,
    pub screenshot_version: Option<u32>,
    pub available_source_types: Option<u32>,
    pub available_cursor_modes: Option<u32>,
    pub available_device_types: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentInfo {
    pub session_kind: SessionKind,
    pub compositor: Option<String>,
    pub desktop_environment: Option<String>,
    pub capture_backend: CaptureBackendKind,
    pub input_backend: InputBackendKind,
    pub semantic_backend: SemanticBackendKind,
    pub portal_capabilities: PortalCapabilities,
    pub xdg_session_type: Option<String>,
    pub display: Option<String>,
    pub wayland_display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppInfo {
    pub app_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_user_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_handle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolkit_guess: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    pub is_focused_candidate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusedApp {
    pub app_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_user_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_handle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolkit_guess: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AppSelector {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptureInfo {
    pub backend: CaptureBackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_backend: Option<CaptureBackendKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinate_space: Option<CoordinateSpace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_rect: Option<RectF>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixel_size: Option<PixelSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_pixel_size: Option<PixelSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_to_pixel_scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_screenshot_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_image_format: Option<ModelImageFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_image_quality: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_image_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_image_encode_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementNode {
    pub element_index: usize,
    pub parent_index: Option<usize>,
    pub role: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub state_flags: Vec<String>,
    pub semantic_actions: Vec<String>,
    pub bounds: Option<RectF>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticEntry {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorReadiness {
    pub can_register_mcp_tools: bool,
    pub can_build_accessibility_tree: bool,
    pub can_capture_screen: bool,
    pub can_send_input: bool,
    #[serde(default)]
    pub can_list_windows: bool,
    #[serde(default)]
    pub can_target_windows: bool,
    pub recommended_next_step: String,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorPlatformReport {
    pub os: String,
    pub arch: String,
    pub session_kind: SessionKind,
    pub xdg_session_type: Option<String>,
    pub desktop_environment: Option<String>,
    pub compositor: Option<String>,
    pub display: Option<String>,
    pub wayland_display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorPortalReport {
    pub screencast_version: Option<u32>,
    pub remote_desktop_version: Option<u32>,
    pub screenshot_version: Option<u32>,
    pub input_capture_version: Option<u32>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorAccessibilityReport {
    pub atspi_bus: DoctorCheck,
    pub toolkit_accessibility: DoctorCheck,
    pub at_spi_enabled: DoctorCheck,
    pub screen_reader: DoctorCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowBackendProbe {
    pub id: String,
    pub ok: bool,
    pub can_list_windows: bool,
    pub can_focus_apps: bool,
    pub can_focus_windows: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorWindowingReport {
    pub probes: Vec<WindowBackendProbe>,
    pub can_list_windows: bool,
    pub can_focus_windows: bool,
    pub detail: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorInputReport {
    pub backend: InputBackendKind,
    pub ydotool: DoctorCheck,
    pub ydotoold: DoctorCheck,
    pub ydotool_socket: DoctorCheck,
    pub xdotool: DoctorCheck,
    pub uinput: DoctorCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserIntegrationReport {
    pub chrome: DoctorCheck,
    pub chromium: DoctorCheck,
    pub brave: DoctorCheck,
    pub native_host_manifest: DoctorCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DoctorReport {
    pub environment: EnvironmentInfo,
    pub checks: Vec<DoctorCheck>,
    pub readiness: DoctorReadiness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<DoctorPlatformReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal: Option<DoctorPortalReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility: Option<DoctorAccessibilityReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windowing: Option<DoctorWindowingReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<DoctorInputReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_integration: Option<BrowserIntegrationReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupCommandReport {
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessibilitySetupReport {
    pub before: Box<DoctorReport>,
    pub accessibility_command: SetupCommandReport,
    pub after: Box<DoctorReport>,
    pub changed: bool,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowTargetingSetupReport {
    pub extension_dir: String,
    pub wrote_files: bool,
    pub enable_command: SetupCommandReport,
    pub windows: Vec<WindowInfo>,
    pub windows_error: Option<String>,
    pub requires_shell_reload: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalProcessInfo {
    pub pid: u32,
    pub command_name: String,
    pub command_line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalWindowInfo {
    pub tty: String,
    pub root_process: TerminalProcessInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_process: Option<TerminalProcessInfo>,
    pub process_count: usize,
    pub confidence: String,
    pub match_reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wm_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<RectF>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<i32>,
    pub focused: bool,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_type: Option<String>,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalWindowInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wm_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl WindowTarget {
    #[must_use]
    pub fn has_target(&self) -> bool {
        self.window_id.as_deref().is_some_and(non_empty)
            || self.pid.is_some()
            || self.tty.as_deref().is_some_and(non_empty)
            || self.terminal_pid.is_some()
            || self.terminal_command.as_deref().is_some_and(non_empty)
            || self.terminal_cwd.as_deref().is_some_and(non_empty)
            || self.app_id.as_deref().is_some_and(non_empty)
            || self.wm_class.as_deref().is_some_and(non_empty)
            || self.title.as_deref().is_some_and(non_empty)
    }
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeuristicMatch {
    pub key: String,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppStateSnapshot {
    pub snapshot_id: String,
    pub created_at: DateTime<Utc>,
    pub environment: EnvironmentInfo,
    pub capabilities: ToolCapabilities,
    pub focused_app: Option<FocusedApp>,
    pub capture: Option<CaptureInfo>,
    pub elements: Vec<ElementNode>,
    pub diagnostics: Vec<DiagnosticEntry>,
    pub app_guidance: Option<HeuristicMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doctor_report: Option<DoctorReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_cursor: Option<AgentCursorState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionName {
    FocusElement,
    ActivateElement,
    SelectElement,
    ExpandElement,
    CollapseElement,
    ToggleElement,
    Click,
    PerformAction,
    PerformSecondaryAction,
    Scroll,
    Drag,
    TypeText,
    PressKey,
    SetValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionRequest {
    pub action: ActionName,
    pub snapshot_id: Option<String>,
    pub element_index: Option<usize>,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_element: Option<ElementNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_target_element: Option<ElementNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_capture: Option<CaptureInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_focused_app: Option<FocusedApp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionOutcome {
    pub success: bool,
    pub message: String,
    pub code: String,
    pub diagnostics: Vec<DiagnosticEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_cursor: Option<AgentCursorState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortalTokenResetOutcome {
    pub token_path: String,
    pub cleared: bool,
    pub dropped_cached_session: bool,
}

impl ActionOutcome {
    #[must_use]
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            code: "NotImplemented".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        }
    }
}

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
        desktop_env: std::collections::BTreeMap<String, String>,
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
    use super::{
        ActionName, ActionOutcome, ActionRequest, AgentCursorBackendKind, AgentCursorCapabilities,
        AgentCursorPlane, AgentCursorPoint, AgentCursorState, AgentCursorSystemCursorBackendKind,
        AppStateSnapshot, CaptureBackendKind, CaptureInfo, CoordinateSpace, DoctorCheck,
        DoctorReadiness, DoctorReport, EnvironmentInfo, InputBackendKind, ModelImageFormat,
        PixelSize, PortalCapabilities, RectF, SemanticBackendKind, ServiceRequest, ServiceResponse,
        SessionKind, SetupCommandReport, ToolAvailability, ToolCapabilities, WindowInfo,
        WindowTargetingSetupReport,
    };
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn boxed_execute_action_preserves_wire_shape() {
        let rendered = serde_json::to_value(ServiceRequest::ExecuteAction {
            request: Box::new(ActionRequest {
                action: ActionName::Click,
                snapshot_id: Some("snap-1".to_string()),
                element_index: Some(7),
                arguments: json!({"x": 10, "y": 20}),
                resolved_element: None,
                resolved_target_element: None,
                resolved_capture: None,
                resolved_focused_app: None,
                environment: None,
            }),
        })
        .expect("service request should serialize");

        assert_eq!(
            rendered,
            json!({
                "type": "execute_action",
                "request": {
                    "action": "click",
                    "snapshot_id": "snap-1",
                    "element_index": 7,
                    "arguments": {"x": 10, "y": 20}
                }
            })
        );
    }

    #[test]
    fn get_app_state_capture_screen_defaults_to_if_changed_on_wire() {
        let rendered = serde_json::to_value(ServiceRequest::GetAppState {
            selector: None,
            capture_screen: Default::default(),
        })
        .expect("service request should serialize");

        assert_eq!(rendered, json!({"type": "get_app_state"}));

        let parsed: ServiceRequest =
            serde_json::from_value(json!({"type": "get_app_state"})).expect("request parses");
        assert_eq!(
            parsed,
            ServiceRequest::GetAppState {
                selector: None,
                capture_screen: Default::default(),
            }
        );
    }

    #[test]
    fn boxed_get_app_state_preserves_wire_shape() {
        let rendered = serde_json::to_value(ServiceResponse::GetAppState {
            snapshot: Box::new(AppStateSnapshot {
                snapshot_id: "snap-1".to_string(),
                created_at: Utc::now(),
                focused_app: None,
                environment: EnvironmentInfo {
                    session_kind: SessionKind::Wayland,
                    compositor: Some("KWin".to_string()),
                    desktop_environment: Some("KDE".to_string()),
                    wayland_display: Some("wayland-0".to_string()),
                    display: None,
                    xdg_session_type: Some("wayland".to_string()),
                    capture_backend: CaptureBackendKind::PortalPipeWire,
                    input_backend: InputBackendKind::PortalRemoteDesktop,
                    semantic_backend: SemanticBackendKind::Atspi,
                    portal_capabilities: PortalCapabilities {
                        screencast_version: Some(5),
                        remote_desktop_version: Some(2),
                        screenshot_version: Some(1),
                        available_source_types: None,
                        available_cursor_modes: None,
                        available_device_types: None,
                    },
                },
                capabilities: available_capabilities(),
                elements: Vec::new(),
                diagnostics: Vec::new(),
                capture: Some(CaptureInfo {
                    backend: CaptureBackendKind::PortalPipeWire,
                    image_backend: Some(CaptureBackendKind::PortalPipeWire),
                    stream_id: Some("42".to_string()),
                    source_type: Some(1),
                    mapping_id: None,
                    screenshot_path: Some("/tmp/snap.jpg".to_string()),
                    original_screenshot_path: Some("/tmp/snap.png".to_string()),
                    pixel_size: Some(PixelSize {
                        width: 1920,
                        height: 1080,
                    }),
                    original_pixel_size: Some(PixelSize {
                        width: 3840,
                        height: 2160,
                    }),
                    coordinate_space: Some(CoordinateSpace::StreamPixels),
                    logical_rect: Some(RectF {
                        x: 0.0,
                        y: 0.0,
                        width: 3840.0,
                        height: 2160.0,
                        space: CoordinateSpace::DesktopLogical,
                    }),
                    logical_to_pixel_scale: Some(0.5),
                    model_image_format: Some(ModelImageFormat::Jpeg),
                    model_image_quality: Some(85),
                    model_image_bytes: Some(1234),
                    model_image_encode_ms: Some(7),
                }),
                app_guidance: None,
                doctor_report: None,
                agent_cursor: None,
            }),
        })
        .expect("service response should serialize");

        assert_eq!(rendered["type"], "get_app_state");
        assert_eq!(rendered["snapshot"]["snapshot_id"], "snap-1");
        assert_eq!(
            rendered["snapshot"]["capture"]["screenshot_path"],
            "/tmp/snap.jpg"
        );
        assert!(rendered.get("snapshot").is_some());
        assert!(rendered["snapshot"].get("doctor_report").is_none());
        assert!(rendered["snapshot"].get("agent_cursor").is_none());
    }

    #[test]
    fn boxed_get_app_state_includes_doctor_report_when_present() {
        let report = DoctorReport {
            environment: EnvironmentInfo {
                session_kind: SessionKind::Wayland,
                compositor: None,
                desktop_environment: None,
                capture_backend: CaptureBackendKind::None,
                input_backend: InputBackendKind::None,
                semantic_backend: SemanticBackendKind::None,
                portal_capabilities: PortalCapabilities {
                    screencast_version: None,
                    remote_desktop_version: None,
                    screenshot_version: None,
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: None,
                display: None,
                wayland_display: None,
            },
            checks: vec![DoctorCheck {
                name: "semantic_backend".to_string(),
                ok: true,
                detail: "Atspi".to_string(),
            }],
            readiness: DoctorReadiness {
                can_register_mcp_tools: true,
                can_build_accessibility_tree: true,
                can_capture_screen: true,
                can_send_input: true,
                can_list_windows: false,
                can_target_windows: false,
                recommended_next_step: "Ready".to_string(),
                blockers: Vec::new(),
            },
            platform: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
        };
        let rendered = serde_json::to_value(ServiceResponse::GetAppState {
            snapshot: Box::new(AppStateSnapshot {
                snapshot_id: "snap-1".to_string(),
                created_at: Utc::now(),
                focused_app: None,
                environment: report.environment.clone(),
                capabilities: available_capabilities(),
                elements: Vec::new(),
                diagnostics: Vec::new(),
                capture: None,
                app_guidance: None,
                doctor_report: Some(report),
                agent_cursor: None,
            }),
        })
        .expect("service response should serialize");

        assert!(rendered["snapshot"].get("doctor_report").is_some());
        assert_eq!(
            rendered["snapshot"]["doctor_report"]["readiness"]["can_build_accessibility_tree"],
            true
        );
    }

    #[test]
    fn window_targeting_report_skips_permissions_hint_when_none() {
        let report = WindowTargetingSetupReport {
            extension_dir: "/tmp/ext".to_string(),
            wrote_files: true,
            enable_command: SetupCommandReport {
                ok: true,
                detail: "enabled".to_string(),
            },
            windows: vec![WindowInfo {
                window_id: "w1".to_string(),
                title: Some("Test".to_string()),
                app_id: Some("app".to_string()),
                wm_class: None,
                pid: Some(42),
                bounds: None,
                workspace: None,
                focused: false,
                hidden: false,
                client_type: None,
                backend: "gnome".to_string(),
                terminal: None,
            }],
            windows_error: None,
            requires_shell_reload: false,
            message: "ok".to_string(),
            permissions_hint: None,
        };
        let rendered = serde_json::to_value(&report).expect("serialize");
        assert!(rendered.get("permissions_hint").is_none());
    }

    #[test]
    fn window_targeting_report_includes_permissions_hint_when_present() {
        let report = WindowTargetingSetupReport {
            extension_dir: "/tmp/ext".to_string(),
            wrote_files: true,
            enable_command: SetupCommandReport {
                ok: true,
                detail: "enabled".to_string(),
            },
            windows: Vec::new(),
            windows_error: Some("dbus error".to_string()),
            requires_shell_reload: false,
            message: "failed".to_string(),
            permissions_hint: Some("Check permissions".to_string()),
        };
        let rendered = serde_json::to_value(&report).expect("serialize");
        assert_eq!(
            rendered["permissions_hint"].as_str(),
            Some("Check permissions")
        );
    }

    #[test]
    fn agent_cursor_contract_serializes_snake_case_and_skips_absent_optional_fields() {
        let state = AgentCursorState {
            visible: true,
            sequence: 7,
            model_point: Some(AgentCursorPoint {
                x: 40.0,
                y: 25.5,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: Some("stream-1".to_string()),
            }),
            native_point: None,
            snapshot_id: Some("snap-1".to_string()),
            source_action: Some(ActionName::Click),
            updated_at_ms: 1_714_000_000_000,
        };
        let rendered = serde_json::to_value(&state).expect("cursor state should serialize");

        assert_eq!(rendered["model_point"]["coordinate_space"], "stream_pixels");
        assert_eq!(rendered["source_action"], "click");
        assert!(rendered.get("native_point").is_none());
        assert_eq!(
            serde_json::to_value(AgentCursorPlane::ScreenshotSynthetic).expect("serialize plane"),
            json!("screenshot_synthetic")
        );
    }

    #[test]
    fn agent_cursor_capabilities_report_backend_as_snake_case() {
        let rendered = serde_json::to_value(AgentCursorCapabilities {
            backend: AgentCursorBackendKind::WaylandLayerShell,
            visible_overlay: true,
            screenshot_synthetic_cursor: true,
            click_through: true,
            capture_exclusion: false,
            system_cursor_hide_supported: false,
            system_cursor_hidden: false,
            system_cursor_backend: AgentCursorSystemCursorBackendKind::WaylandClientUnsupported,
            needs_user_install: false,
            reason: None,
        })
        .expect("capabilities should serialize");

        assert_eq!(rendered["backend"], "wayland_layer_shell");
        assert_eq!(rendered["system_cursor_hide_supported"], false);
        assert_eq!(rendered["system_cursor_hidden"], false);
        assert_eq!(
            rendered["system_cursor_backend"],
            "wayland_client_unsupported"
        );
        assert!(rendered.get("reason").is_none());

        let old: AgentCursorCapabilities = serde_json::from_value(json!({
            "backend": "wayland_layer_shell",
            "visible_overlay": true,
            "screenshot_synthetic_cursor": true,
            "click_through": true,
            "capture_exclusion": false,
            "needs_user_install": false
        }))
        .expect("old capabilities without system cursor fields should deserialize");
        assert!(!old.system_cursor_hide_supported);
        assert!(!old.system_cursor_hidden);
        assert_eq!(
            old.system_cursor_backend,
            AgentCursorSystemCursorBackendKind::None
        );
    }

    #[test]
    fn action_outcome_skips_absent_cursor_and_accepts_old_wire_shape() {
        let rendered = serde_json::to_value(ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        })
        .expect("outcome should serialize");

        assert!(rendered.get("agent_cursor").is_none());

        let old: ActionOutcome = serde_json::from_value(json!({
            "success": true,
            "message": "ok",
            "code": "Ok",
            "diagnostics": []
        }))
        .expect("old outcomes without cursor should deserialize");
        assert_eq!(old.agent_cursor, None);
    }

    #[test]
    fn app_state_snapshot_accepts_old_wire_shape_without_agent_cursor() {
        let old = json!({
            "snapshot_id": "snap-old",
            "created_at": "2026-05-14T19:00:00Z",
            "environment": {
                "session_kind": "wayland",
                "compositor": "KWin",
                "desktop_environment": "KDE",
                "capture_backend": "portal_pipe_wire",
                "input_backend": "portal_remote_desktop",
                "semantic_backend": "atspi",
                "portal_capabilities": {
                    "screencast_version": 5,
                    "remote_desktop_version": 2,
                    "screenshot_version": 1,
                    "available_source_types": null,
                    "available_cursor_modes": null,
                    "available_device_types": null
                },
                "xdg_session_type": "wayland",
                "display": null,
                "wayland_display": "wayland-0"
            },
            "capabilities": available_capabilities(),
            "focused_app": null,
            "capture": null,
            "elements": [],
            "diagnostics": [],
            "app_guidance": null
        });

        let snapshot: AppStateSnapshot =
            serde_json::from_value(old).expect("old snapshot should deserialize");
        assert_eq!(snapshot.agent_cursor, None);
    }

    #[test]
    fn agent_cursor_service_requests_preserve_json_wire_shape() {
        let state = AgentCursorState {
            visible: true,
            sequence: 1,
            model_point: Some(AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: None,
            }),
            native_point: None,
            snapshot_id: Some("snap-1".to_string()),
            source_action: Some(ActionName::Click),
            updated_at_ms: 42,
        };
        let rendered = serde_json::to_value(ServiceRequest::SetAgentCursor { state })
            .expect("request should serialize");

        assert_eq!(rendered["type"], "set_agent_cursor");
        assert_eq!(
            rendered["state"]["model_point"]["coordinate_space"],
            "stream_pixels"
        );
        assert!(rendered["state"]["model_point"].get("mapping_id").is_none());
    }

    fn available_capabilities() -> ToolCapabilities {
        let available = || ToolAvailability {
            available: true,
            reason: None,
        };
        ToolCapabilities {
            list_apps: available(),
            get_app_state: available(),
            focus_element: available(),
            activate_element: available(),
            select_element: available(),
            expand_element: available(),
            collapse_element: available(),
            toggle_element: available(),
            click: available(),
            perform_action: available(),
            perform_secondary_action: available(),
            scroll: available(),
            drag: available(),
            type_text: available(),
            press_key: available(),
            set_value: available(),
        }
    }
}
