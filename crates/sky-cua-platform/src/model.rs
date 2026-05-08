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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelImageFormat {
    Jpeg,
    Webp,
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
    pub pid: Option<u32>,
    pub executable: Option<String>,
    pub desktop_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_user_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_handle: Option<String>,
    pub toolkit_guess: Option<String>,
    pub window_title: Option<String>,
    pub is_focused_candidate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusedApp {
    pub app_id: String,
    pub name: String,
    pub pid: Option<u32>,
    pub desktop_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_user_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_handle: Option<String>,
    pub toolkit_guess: Option<String>,
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
    pub stream_id: Option<String>,
    pub source_type: Option<u32>,
    pub mapping_id: Option<String>,
    pub logical_rect: Option<RectF>,
    pub pixel_size: Option<PixelSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_pixel_size: Option<PixelSize>,
    pub logical_to_pixel_scale: Option<f64>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionOutcome {
    pub success: bool,
    pub message: String,
    pub code: String,
    pub diagnostics: Vec<DiagnosticEntry>,
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceRequest {
    Health,
    ListApps,
    GetAppState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selector: Option<AppSelector>,
    },
    ResetPortalTokens,
    ExecuteAction {
        request: Box<ActionRequest>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceResponse {
    Health {
        ok: bool,
        service_socket: String,
    },
    ListApps {
        environment: EnvironmentInfo,
        apps: Vec<AppInfo>,
        diagnostics: Vec<DiagnosticEntry>,
    },
    GetAppState {
        snapshot: Box<AppStateSnapshot>,
    },
    ResetPortalTokens {
        cleared: bool,
        token_path: String,
        dropped_cached_session: bool,
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
        ActionName, ActionRequest, AppStateSnapshot, CaptureBackendKind, CaptureInfo,
        CoordinateSpace, EnvironmentInfo, InputBackendKind, ModelImageFormat, PixelSize,
        PortalCapabilities, RectF, SemanticBackendKind, ServiceRequest, ServiceResponse,
        SessionKind, ToolAvailability, ToolCapabilities,
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
            perform_secondary_action: available(),
            scroll: available(),
            drag: available(),
            type_text: available(),
            press_key: available(),
            set_value: available(),
        }
    }
}
