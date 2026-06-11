use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod browser;
mod service;

pub use browser::{
    BROWSER_EVAL_ENV, BrowserActionResponse, BrowserClaimTabResponse, BrowserEvalResponse,
    BrowserListTabsResponse, BrowserMoveMouseResponse, BrowserNavigateResponse,
    BrowserOpenResponse, BrowserRequest, BrowserResponse, BrowserScreenshotResponse,
    BrowserSnapshotResponse, BrowserStatusReport, BrowserTab, BrowserTargetAvailability,
    BrowserTargetKind, browser_diagnostic_is_error_code, browser_eval_enabled,
    normalize_browser_open_url,
};
pub use service::{ServiceRequest, ServiceResponse};

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
    GnomeShellExtension,
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
    GnomeShellExtension,
    HyprlandConfig,
    CosmicCompBridge,
    CosmicTransparentXcursor,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_scroll_directions: Vec<ScrollDirection>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<ElementTextReadback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_value: Option<ElementNumericValueReadback>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub supports_editable_text: bool,
    pub state_flags: Vec<String>,
    pub semantic_actions: Vec<String>,
    pub bounds: Option<RectF>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementTextReadback {
    pub character_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caret_offset: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub content_suppressed: bool,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selections: Vec<ElementTextSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ElementTextSelection {
    pub start_offset: i32,
    pub end_offset: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementNumericValueReadback {
    pub current: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub minimum_increment: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
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
pub struct DoctorSessionEnvRepair {
    pub key: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorSessionEnvReport {
    #[serde(default)]
    pub repaired: Vec<DoctorSessionEnvRepair>,
    #[serde(default)]
    pub path_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_path: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl DoctorSessionEnvReport {
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.repaired.is_empty() || self.path_changed
    }
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
    pub session_env: Option<DoctorSessionEnvReport>,
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
    pub const FIELD_NAMES: &'static [&'static str] = &[
        "window_id",
        "pid",
        "tty",
        "terminal_pid",
        "terminal_command",
        "terminal_cwd",
        "app_id",
        "wm_class",
        "title",
    ];

    pub fn from_argument_fields(
        arguments: &serde_json::Value,
    ) -> Result<Option<Self>, serde_json::Error> {
        let Some(arguments) = arguments.as_object() else {
            return Ok(None);
        };

        let mut target_arguments = serde_json::Map::new();
        for field in Self::FIELD_NAMES {
            if let Some(value) = arguments.get(*field)
                && target_argument_is_present(value)
            {
                target_arguments.insert((*field).to_string(), value.clone());
            }
        }

        if target_arguments.is_empty() {
            return Ok(None);
        }

        let mut target: Self = serde_json::from_value(serde_json::Value::Object(target_arguments))?;
        target.normalize_empty_fields();
        Ok(target.has_target().then_some(target))
    }

    #[must_use]
    pub fn has_target(&self) -> bool {
        self.window_id.as_deref().is_some_and(non_empty)
            || self.pid.is_some_and(non_zero)
            || self.tty.as_deref().is_some_and(non_empty)
            || self.terminal_pid.is_some_and(non_zero)
            || self.terminal_command.as_deref().is_some_and(non_empty)
            || self.terminal_cwd.as_deref().is_some_and(non_empty)
            || self.app_id.as_deref().is_some_and(non_empty)
            || self.wm_class.as_deref().is_some_and(non_empty)
            || self.title.as_deref().is_some_and(non_empty)
    }

    pub fn normalize_empty_fields(&mut self) {
        self.window_id = normalize_optional_string(self.window_id.take());
        self.pid = self.pid.filter(|pid| non_zero(*pid));
        self.tty = normalize_optional_string(self.tty.take());
        self.terminal_pid = self.terminal_pid.filter(|pid| non_zero(*pid));
        self.terminal_command = normalize_optional_string(self.terminal_command.take());
        self.terminal_cwd = normalize_optional_string(self.terminal_cwd.take());
        self.app_id = normalize_optional_string(self.app_id.take());
        self.wm_class = normalize_optional_string(self.wm_class.take());
        self.title = normalize_optional_string(self.title.take());
    }
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn non_zero(value: u32) -> bool {
    value != 0
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.len() == value.len() {
            Some(value)
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn target_argument_is_present(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Number(value) => value.as_u64().is_none_or(|value| value != 0),
        _ => true,
    }
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

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;
