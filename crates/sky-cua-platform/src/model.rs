use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod browser;
mod phone;
mod service;

pub use browser::{
    BROWSER_EVAL_ENV, BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT, BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT,
    BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT, BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserActionResponse,
    BrowserClaimTabResponse, BrowserEvalResponse, BrowserListTabsResponse,
    BrowserMoveMouseResponse, BrowserNavigateResponse, BrowserOpenResponse, BrowserRequest,
    BrowserResponse, BrowserScreenshotResponse, BrowserSessionIdentity, BrowserSnapshotResponse,
    BrowserStatusReport, BrowserTab, BrowserTargetAvailability, BrowserTargetKind,
    browser_diagnostic_is_error_code, browser_eval_enabled, normalize_browser_open_url,
};
pub use phone::{
    PhoneAccessibilityNode, PhoneAccessibilitySummary, PhoneAccessibilityTreeRequest,
    PhoneAccessibilityTreeResponse, PhoneActionResponse, PhoneAppCurrentRequest,
    PhoneAppForceStopRequest, PhoneAppInfo, PhoneAppInstallMode, PhoneAppInstallRequest,
    PhoneAppLaunchRequest, PhoneAppListRequest, PhoneAppOpenIntentRequest, PhoneAppResponse,
    PhoneAppResponseKind, PhoneAvailableAction, PhoneBackendCapabilities, PhoneBackendKind,
    PhoneCapabilityProfile, PhoneCapabilityRefreshState, PhoneCompanionCapabilities,
    PhoneCompanionStatusRequest, PhoneCompanionStatusResponse, PhoneConnectRequest,
    PhoneConnectionKind, PhoneCoordinateMapping, PhoneCursorCapabilities, PhoneCursorState,
    PhoneDevice, PhoneDeviceState, PhoneDisconnectRequest, PhoneDisconnectResponse, PhoneImage,
    PhoneInstallCompanionRequest, PhoneInstallStrategy, PhoneListDevicesRequest,
    PhoneListDevicesResponse, PhoneNotificationAction, PhoneNotificationActionRequest,
    PhoneNotificationDismissRequest, PhoneNotificationEvent, PhoneNotificationOpenRequest,
    PhoneNotificationRedaction, PhoneNotificationReplyRequest, PhoneNotificationsRequest,
    PhoneNotificationsResponse, PhoneObserveRequest, PhoneObserveResponse,
    PhoneOpenSettingsRequest, PhonePairWirelessRequest, PhonePairWirelessResponse, PhonePoint,
    PhonePressKeyRequest, PhoneRefreshCapabilitiesRequest, PhoneRequest, PhoneResponse,
    PhoneScrcpyCapabilities, PhoneScreenshotRequest, PhoneScreenshotResponse, PhoneSession,
    PhoneSessionSelector, PhoneSettingsScreen, PhoneStatusReport, PhoneStatusRequest,
    PhoneSwipeRequest, PhoneTapRequest, PhoneTargetDeviceKind, PhoneTypeTextRequest,
    PhoneUnavailableAction,
};
pub use service::{
    CUA_SERVICE_CAPABILITIES, CUA_SERVICE_DEFAULT_MOUSE_SIZE_PX, CUA_SERVICE_MAX_DEADLINE_MS,
    CUA_SERVICE_PROTOCOL_VERSION, CUA_SERVICE_VERSION, CuaActionRequest, CuaBackendResponse,
    CuaCancelStatus, CuaCancellation, CuaMouseButton, CuaRequestContext, CuaScreenshot,
    CuaScrollDirection, ServiceRequest, ServiceResponse, SessionPresenceAction,
    cua_service_capabilities, cua_service_capabilities_for_input_backend,
};

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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCursorBackendKind {
    #[default]
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
pub enum AgentCursorRendererBackendKind {
    #[default]
    None,
    WaylandShm,
    Wgpu,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentCursorPointerTrackingBackendKind {
    #[default]
    None,
    KwinEffectSignal,
    PrivilegedInputHelper,
    X11Query,
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

/// A generic 2D point used for gesture coordinates. The coordinate space is
/// carried by the container, not the point itself.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentOverlayGestureKind {
    Tap,
    Drag,
    Swipe,
    NoNo,
}

/// A one-shot animation event sent to the overlay host. Events are not
/// persistent cursor state: a host restart must not replay old gesture events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentOverlayGestureEvent {
    pub event_id: String,
    pub sequence: u64,
    pub kind: AgentOverlayGestureKind,
    pub coordinate_space: CoordinateSpace,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping_id: Option<String>,
    pub points: Vec<Point2>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action: Option<ActionName>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentOverlayCoverageKind {
    #[default]
    None,
    Full,
    Partial,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentOverlayHostLifecycleState {
    /// The overlay host process has not been spawned or has exited.
    #[default]
    ProcessUnavailable,
    /// The host process is running and the IPC endpoint is reachable, but the
    /// backend has not finished initialization yet.
    SocketReady,
    /// The backend is actively initializing (adapter/device setup, surface
    /// validation, etc.).
    BackendInitializing,
    /// The backend finished initialization and is ready to render.
    BackendReady,
    /// The backend finished initialization but visible overlay is unsupported
    /// in this session.
    BackendUnsupported,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentOverlayEffectsCapabilities {
    pub glide: bool,
    pub rotation: bool,
    pub halo: bool,
    pub ripple: bool,
    pub trail: bool,
    pub edge_glow: bool,
    pub inward_wave: bool,
    pub no_no_render: bool,
    pub hit_test: bool,
    pub sound: bool,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCursorCapabilities {
    pub backend: AgentCursorBackendKind,
    #[serde(default)]
    pub renderer_backend: AgentCursorRendererBackendKind,
    pub visible_overlay: bool,
    pub screenshot_synthetic_cursor: bool,
    pub click_through: bool,
    pub capture_exclusion: bool,
    #[serde(default)]
    pub pointer_tracking_backend: AgentCursorPointerTrackingBackendKind,
    #[serde(default)]
    pub pointer_tracking_exact: bool,
    #[serde(default)]
    pub system_cursor_hide_supported: bool,
    #[serde(default)]
    pub system_cursor_hidden: bool,
    #[serde(default)]
    pub system_cursor_backend: AgentCursorSystemCursorBackendKind,
    pub needs_user_install: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<AgentOverlayEffectsCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<AgentOverlayCoverageKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_coordinate_spaces: Vec<CoordinateSpace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gesture_points: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect_schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_output_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_output_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_name: Option<String>,
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

impl RectF {
    #[must_use]
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    #[must_use]
    pub fn area(&self) -> f64 {
        if self.width <= 0.0 || self.height <= 0.0 {
            0.0
        } else {
            self.width * self.height
        }
    }

    #[must_use]
    pub fn intersection(&self, other: &RectF) -> Option<RectF> {
        if self.space != other.space {
            return None;
        }
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > left && bottom > top).then(|| RectF {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
            space: self.space.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisplayInfo {
    pub display_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub index: u32,
    pub primary: bool,
    pub logical_rect: RectF,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_size: Option<PixelSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_factor: Option<f64>,
    pub backend: String,
}

/// The display whose `primary` flag is set, if any.
///
/// Resolves only the primary flag so the "which display is primary" rule lives
/// in one place rather than being re-spelled per backend. Callers layer their
/// own fallback (first display, tallest, ...) when nothing is flagged primary.
/// Returns a borrow; clone if an owned value is needed.
pub fn primary_flagged_display(displays: &[DisplayInfo]) -> Option<&DisplayInfo> {
    displays.iter().find(|display| display.primary)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplayRef {
    pub display_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub index: u32,
    pub primary: bool,
    pub backend: String,
}

impl From<&DisplayInfo> for DisplayRef {
    fn from(display: &DisplayInfo) -> Self {
        Self {
            display_id: display.display_id.clone(),
            name: display.name.clone(),
            index: display.index,
            primary: display.primary,
            backend: display.backend.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisplayIntersection {
    pub display: DisplayRef,
    pub intersection_rect: RectF,
    pub intersection_area: f64,
    pub coverage_ratio: f64,
}

impl DisplayIntersection {
    #[must_use]
    pub fn from_bounds(display: &DisplayInfo, bounds: &RectF) -> Option<Self> {
        let intersection_rect = bounds.intersection(&display.logical_rect)?;
        let intersection_area = intersection_rect.area();
        if intersection_area <= 0.0 {
            return None;
        }
        let coverage_ratio = if bounds.area() > 0.0 {
            intersection_area / bounds.area()
        } else {
            0.0
        };
        Some(Self {
            display: DisplayRef::from(display),
            intersection_rect,
            intersection_area,
            coverage_ratio,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureScope {
    PrimaryDisplay,
    Display,
    Window,
    #[default]
    Unknown,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub displays: Vec<DisplayInfo>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayRef>,
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
    #[serde(default, skip_serializing_if = "is_unknown_capture_scope")]
    pub capture_scope: CaptureScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayRef>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_logical_rect: Option<RectF>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixel_size: Option<PixelSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_pixel_size: Option<PixelSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_to_pixel_scale: Option<f64>,
    #[serde(
        rename = "inspection_image_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub screenshot_path: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
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

impl CaptureInfo {
    pub fn clear_image_fields(&mut self) {
        self.image_backend = None;
        self.screenshot_path = None;
        self.pixel_size = None;
        self.original_screenshot_path = None;
        self.original_pixel_size = None;
        self.logical_to_pixel_scale = None;
        self.model_image_format = None;
        self.model_image_quality = None;
        self.model_image_bytes = None;
        self.model_image_encode_ms = None;
    }
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionPresenceIntent {
    #[serde(default)]
    pub unlock: bool,
    #[serde(default)]
    pub inhibit_lock: bool,
    #[serde(default)]
    pub inhibit_suspend: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionPresenceStatus {
    pub backend: String,
    pub supported: bool,
    pub unlock_supported: bool,
    pub locked: Option<bool>,
    pub lock_inhibited: bool,
    pub suspend_inhibited: bool,
    pub detail: String,
}

impl SessionPresenceStatus {
    #[must_use]
    pub fn unsupported(backend: &str) -> Self {
        Self {
            backend: backend.to_string(),
            supported: false,
            unlock_supported: false,
            locked: None,
            lock_inhibited: false,
            suspend_inhibited: false,
            detail: "session presence is not available for this backend".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorSessionPresenceReport {
    pub backend: String,
    pub unlock: DoctorCheck,
    pub inhibit_lock: DoctorCheck,
    pub inhibit_suspend: DoctorCheck,
    pub lock_state_readable: DoctorCheck,
}

impl DoctorSessionPresenceReport {
    #[must_use]
    pub fn unsupported(backend: &str) -> Self {
        let unsupported_detail = "session presence is not available for this backend".to_string();
        Self {
            backend: backend.to_string(),
            unlock: DoctorCheck {
                name: "unlock".to_string(),
                ok: false,
                detail: unsupported_detail.clone(),
            },
            inhibit_lock: DoctorCheck {
                name: "inhibit_lock".to_string(),
                ok: false,
                detail: unsupported_detail.clone(),
            },
            inhibit_suspend: DoctorCheck {
                name: "inhibit_suspend".to_string(),
                ok: false,
                detail: unsupported_detail.clone(),
            },
            lock_state_readable: DoctorCheck {
                name: "lock_state_readable".to_string(),
                ok: false,
                detail: unsupported_detail,
            },
        }
    }
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
    #[serde(default)]
    pub can_inhibit_presence: bool,
    #[serde(default)]
    pub can_unlock_session: bool,
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
pub struct DoctorDisplayProbeReport {
    pub provider: String,
    pub attempted: bool,
    pub ok: bool,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<i32>,
    #[serde(default)]
    pub stdout_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_snippet: Option<String>,
    #[serde(default)]
    pub display_count: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorDisplayTopologyReport {
    pub display_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_provider: Option<String>,
    #[serde(default)]
    pub probes: Vec<DoctorDisplayProbeReport>,
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
    #[serde(default = "default_linux_virtual_input_check")]
    pub linux_virtual_input: DoctorCheck,
    pub ydotool: DoctorCheck,
    pub ydotoold: DoctorCheck,
    pub ydotool_socket: DoctorCheck,
    pub xdotool: DoctorCheck,
    pub uinput: DoctorCheck,
}

fn default_linux_virtual_input_check() -> DoctorCheck {
    DoctorCheck {
        name: "linux_virtual_input".to_string(),
        ok: false,
        detail: "not reported".to_string(),
    }
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
    pub display_topology: Option<DoctorDisplayTopologyReport>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_presence: Option<DoctorSessionPresenceReport>,
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
    pub display: Option<DisplayRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub display_intersections: Vec<DisplayIntersection>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_index: Option<u32>,
}

impl DisplayTarget {
    pub const FIELD_NAMES: &'static [&'static str] =
        &["display_id", "display_name", "display_index"];

    pub fn from_argument_fields(
        arguments: &serde_json::Value,
    ) -> Result<Option<Self>, serde_json::Error> {
        let Some(arguments) = arguments.as_object() else {
            return Ok(None);
        };

        let mut target_arguments = serde_json::Map::new();
        for &field in Self::FIELD_NAMES {
            if let Some(value) = arguments.get(field)
                && display_target_argument_is_present(field, value)
            {
                target_arguments.insert(field.to_string(), value.clone());
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
        self.display_id.as_deref().is_some_and(non_empty)
            || self.display_name.as_deref().is_some_and(non_empty)
            || self.display_index.is_some()
    }

    pub fn normalize_empty_fields(&mut self) {
        self.display_id = normalize_optional_string(self.display_id.take());
        self.display_name = normalize_optional_string(self.display_name.take());
    }
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

fn display_target_argument_is_present(field: &str, value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Number(_) => field == "display_index",
        _ => true,
    }
}

fn is_unknown_capture_scope(value: &CaptureScope) -> bool {
    *value == CaptureScope::Unknown
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
    Move,
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

/// An application the backend launched into its desktop session.
///
/// The process is detached and outlives the launch request; the only fact the
/// caller learns is the operating-system process id of the spawned child.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchedApplication {
    pub pid: u32,
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

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;
