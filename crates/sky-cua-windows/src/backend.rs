use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::null_mut;

use image::{ImageBuffer, Rgb};
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AppInfo, AppSelector, AppStateSnapshot,
    CaptureBackendKind, CaptureInfo, CaptureScreenMode, CoordinateSpace, DoctorReport, ElementNode,
    EnvironmentInfo, FocusedApp, InputBackendKind, ModelImageFormat, PixelSize, PortalCapabilities,
    RectF, ScrollDirection, SemanticBackendKind, SessionKind, SessionPresenceIntent,
    SessionPresenceStatus, ToolAvailability, ToolCapabilities,
};
use sky_cua_platform::{new_snapshot_id, sky_cua_state_dir};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetWindowDC, HBITMAP, HDC, HGDIOBJ,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, SendInput, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN,
    VK_SHIFT, VK_TAB, mouse_event,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GUITHREADINFO, GetCursorPos, GetDesktopWindow, GetForegroundWindow,
    GetGUIThreadInfo, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SetCursorPos, SetForegroundWindow,
    WM_CHAR, WM_KEYDOWN, WM_KEYUP,
};

use crate::{session_presence::SessionPresenceManager, uia};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseButton {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsInputBackend {
    SendInput,
    WindowMessages,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureBlankFrame {
    Black,
    White,
}

impl WindowsInputBackend {
    fn model_kind(self) -> InputBackendKind {
        match self {
            Self::SendInput => InputBackendKind::SendInput,
            Self::WindowMessages => InputBackendKind::WindowsMessages,
            Self::None => InputBackendKind::None,
        }
    }
}

#[derive(Debug, Clone)]
struct WindowInfo {
    hwnd: usize,
    title: String,
    pid: Option<u32>,
    executable: Option<String>,
    bounds: RectF,
    is_foreground: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WindowsDesktopBackend {
    session_presence: SessionPresenceManager,
}

impl WindowsDesktopBackend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            session_presence: SessionPresenceManager::new(),
        }
    }

    fn capabilities(environment: &EnvironmentInfo) -> ToolCapabilities {
        let physical_ready = matches!(
            environment.input_backend,
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages
        );
        let semantic_ready = environment.semantic_backend == SemanticBackendKind::Uia;
        let listing_ready = environment.session_kind == SessionKind::Windows;
        let input_reason = if is_rdp_session() {
            "Windows RDP message input is unavailable"
        } else {
            "SendInput is unavailable"
        };

        ToolCapabilities {
            list_apps: availability(
                listing_ready,
                "Windows top-level window enumeration is unavailable",
            ),
            get_app_state: availability(
                listing_ready,
                "Windows top-level window enumeration is unavailable",
            ),
            focus_element: availability(semantic_ready, "UI Automation is unavailable"),
            activate_element: availability(semantic_ready, "UI Automation is unavailable"),
            select_element: availability(semantic_ready, "UI Automation is unavailable"),
            expand_element: availability(semantic_ready, "UI Automation is unavailable"),
            collapse_element: availability(semantic_ready, "UI Automation is unavailable"),
            toggle_element: availability(semantic_ready, "UI Automation is unavailable"),
            click: availability(semantic_ready || physical_ready, input_reason),
            perform_action: availability(semantic_ready, "UI Automation is unavailable"),
            perform_secondary_action: availability(physical_ready, input_reason),
            scroll: availability(physical_ready, input_reason),
            supported_scroll_directions: vec![ScrollDirection::Up, ScrollDirection::Down],
            drag: availability(physical_ready, input_reason),
            type_text: availability(physical_ready, input_reason),
            press_key: availability(physical_ready, input_reason),
            set_value: availability(
                semantic_ready || physical_ready,
                "UI Automation and physical input are unavailable for set_value in this session",
            ),
        }
    }

    fn focused_from_app(app: &AppInfo) -> FocusedApp {
        FocusedApp {
            app_id: app.app_id.clone(),
            name: app.name.clone(),
            pid: app.pid,
            desktop_file_id: app.desktop_file_id.clone(),
            app_user_model_id: app.app_user_model_id.clone(),
            window_handle: app.window_handle.clone(),
            toolkit_guess: app.toolkit_guess.clone(),
            window_title: app.window_title.clone(),
        }
    }

    fn window_to_app(window: &WindowInfo) -> AppInfo {
        let executable = window.executable.clone();
        let name = executable
            .as_deref()
            .and_then(|path| {
                PathBuf::from(path)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
            })
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| window.title.clone());
        AppInfo {
            app_id: format!("hwnd:0x{:x}", window.hwnd),
            name,
            pid: window.pid,
            executable,
            desktop_file_id: None,
            app_user_model_id: None,
            window_handle: Some(format!("0x{:x}", window.hwnd)),
            toolkit_guess: Some("win32".to_string()),
            window_title: Some(window.title.clone()),
            is_focused_candidate: window.is_foreground,
        }
    }
}

#[async_trait::async_trait]
impl DesktopBackend for WindowsDesktopBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        let input_backend = select_input_backend().model_kind();
        let semantic_backend = if uia::is_available() {
            SemanticBackendKind::Uia
        } else {
            SemanticBackendKind::None
        };
        Ok(EnvironmentInfo {
            session_kind: SessionKind::Windows,
            compositor: Some("windows-desktop".to_string()),
            desktop_environment: std::env::var("SESSIONNAME").ok(),
            capture_backend: CaptureBackendKind::WindowsGdi,
            input_backend,
            semantic_backend,
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
        })
    }

    async fn doctor(&self) -> Result<DoctorReport, BackendError> {
        let environment = self.probe_environment().await?;
        Ok(crate::session_presence::windows_doctor_report(
            environment,
            self.session_presence.doctor_report(),
        ))
    }

    async fn ensure_session_presence(
        &self,
        intent: SessionPresenceIntent,
    ) -> Result<SessionPresenceStatus, BackendError> {
        Ok(self.session_presence.ensure(intent).await)
    }

    async fn release_session_presence(
        &self,
        relock: bool,
    ) -> Result<SessionPresenceStatus, BackendError> {
        Ok(self.session_presence.release(relock).await)
    }

    async fn session_presence_status(&self) -> SessionPresenceStatus {
        self.session_presence.status().await
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        Ok(enumerate_windows()
            .into_iter()
            .map(|window| Self::window_to_app(&window))
            .collect())
    }

    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        let snapshot_id = new_snapshot_id();
        let environment = self.probe_environment().await?;
        let capabilities = Self::capabilities(&environment);
        let mut diagnostics = DiagnosticBuilder::new();
        match environment.input_backend {
            InputBackendKind::WindowsMessages => diagnostics.push(
                BackendErrorCode::ActionRequiresPhysicalInput,
                "Windows rejected SendInput during environment probing; using RDP-safe per-window message input fallback",
                None,
            ),
            InputBackendKind::None => diagnostics.push(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "Windows rejected SendInput and no valid RDP input fallback was detected; physical input actions are unavailable in this session",
                None,
            ),
            _ => {}
        }

        let windows = enumerate_windows();
        let selected = selector
            .as_ref()
            .and_then(|selector| select_window(&windows, selector))
            .or_else(|| windows.iter().find(|window| window.is_foreground).cloned())
            .or_else(|| windows.first().cloned());

        let capture_result = if capture_screen == CaptureScreenMode::Never {
            None
        } else {
            match capture_desktop(&snapshot_id, selected.as_ref()).await {
                Ok(result) => Some(result),
                Err(error) => {
                    diagnostics.push(
                        BackendErrorCode::Internal,
                        "Windows GDI screenshot capture failed",
                        Some(error.message),
                    );
                    Some(CaptureResult {
                        capture: empty_capture(),
                        blank_frame: None,
                    })
                }
            }
        };

        if let Some(blank_frame) = capture_result
            .as_ref()
            .and_then(|result| result.blank_frame)
        {
            let details = capture_result
                .as_ref()
                .and_then(|result| result.capture.screenshot_path.clone())
                .map(|path| format!("kind={} path={path}", blank_frame.as_str()))
                .unwrap_or_else(|| format!("kind={}", blank_frame.as_str()));
            diagnostics.push(
                BackendErrorCode::CaptureFrameBlank,
                "Windows GDI screenshot appears blank; browser GPU or protected surfaces may not be capturable through GDI",
                Some(details),
            );
        }

        let (focused_app, elements) = if let Some(window) = selected.as_ref() {
            let app = Self::window_to_app(window);
            let focused_app = Some(Self::focused_from_app(&app));
            let uia_elements = if environment.semantic_backend == SemanticBackendKind::Uia {
                uia::collect_elements_for_hwnd(
                    window.hwnd,
                    &window.title,
                    &window.bounds,
                    capture_result.as_ref().map(|result| &result.capture),
                )
            } else {
                Err(BackendError::new(
                    BackendErrorCode::AccessibilityUnavailable,
                    "Windows UI Automation is unavailable in this session",
                ))
            };

            match uia_elements {
                Ok(elements)
                    if elements.len() > 1
                        || elements
                            .iter()
                            .any(|element| !element.semantic_actions.is_empty()) =>
                {
                    (focused_app, elements)
                }
                Ok(_) => {
                    diagnostics.push(
                        BackendErrorCode::AccessibilityCoverageLimited,
                        "Windows UI Automation returned no useful child elements; using Win32 window fallback tree",
                        Some(window.title.clone()),
                    );
                    (focused_app, vec![window_element(window, 0)])
                }
                Err(error) => {
                    diagnostics.push(
                        BackendErrorCode::AccessibilityCoverageLimited,
                        "Windows UI Automation tree collection failed; using Win32 window fallback tree",
                        Some(error.message),
                    );
                    (focused_app, vec![window_element(window, 0)])
                }
            }
        } else {
            diagnostics.push(
                BackendErrorCode::AccessibilityCoverageLimited,
                "No visible top-level Windows application window was found",
                None,
            );
            (None, Vec::new())
        };

        Ok(AppStateSnapshot {
            snapshot_id,
            created_at: chrono::Utc::now(),
            environment,
            capabilities,
            focused_app,
            capture: capture_result.map(|result| result.capture),
            elements,
            diagnostics: diagnostics.finish(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        })
    }

    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let uia_target = uia_backend_ref_for_fallback(&request)
            .filter(|_| {
                matches!(
                    request.action,
                    ActionName::Click
                        | ActionName::FocusElement
                        | ActionName::ActivateElement
                        | ActionName::SelectElement
                        | ActionName::ExpandElement
                        | ActionName::CollapseElement
                        | ActionName::ToggleElement
                        | ActionName::SetValue
                )
            })
            .map(ToOwned::to_owned);
        if let Some(outcome) = uia::try_execute_semantic_action(&request)? {
            return Ok(outcome);
        }

        let input_backend = request
            .environment
            .as_ref()
            .map(|environment| environment.input_backend.clone())
            .unwrap_or_else(|| select_input_backend().model_kind());
        let mut outcome = match input_backend {
            InputBackendKind::SendInput => execute_send_input_action(&request),
            InputBackendKind::WindowsMessages => execute_window_message_action(&request),
            _ => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "Windows physical input is unavailable in this session",
            )),
        }?;
        if let Some(reference) = uia_target {
            outcome.diagnostics.push(
                sky_cua_platform::model::DiagnosticEntry {
                    code: BackendErrorCode::AccessibilityCoverageLimited
                        .as_str()
                        .to_string(),
                    message:
                        "UI Automation semantic action was unavailable; physical input fallback was used"
                            .to_string(),
                    details: Some(reference),
                },
            );
        }
        Ok(outcome)
    }
}

fn execute_send_input_action(request: &ActionRequest) -> Result<ActionOutcome, BackendError> {
    match request.action {
        ActionName::FocusElement
        | ActionName::ActivateElement
        | ActionName::SelectElement
        | ActionName::ExpandElement
        | ActionName::CollapseElement
        | ActionName::ToggleElement
        | ActionName::PerformAction => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "this semantic automation primitive requires Windows UI Automation",
        )),
        ActionName::Click => {
            focus_request_window(request);
            let (x, y) = desktop_action_point(request)?;
            click_at(x, y, MouseButton::Left)?;
            Ok(success("SendInput click completed"))
        }
        ActionName::PerformSecondaryAction => {
            focus_request_window(request);
            let (x, y) = desktop_action_point(request)?;
            click_at(x, y, MouseButton::Right)?;
            Ok(success("SendInput secondary click completed"))
        }
        ActionName::Scroll => {
            focus_request_window(request);
            if let Ok((x, y)) = desktop_action_point(request) {
                move_pointer(x, y)?;
            }
            let delta_y = scroll_delta_y(request);
            wheel(delta_y)?;
            Ok(success("SendInput scroll completed"))
        }
        ActionName::Drag => {
            focus_request_window(request);
            let (from_x, from_y) = desktop_drag_from_point(request)?;
            let (to_x, to_y) = desktop_target_point(request)?;
            drag(from_x, from_y, to_x, to_y)?;
            Ok(success("SendInput drag completed"))
        }
        ActionName::TypeText => {
            focus_request_window(request);
            let text = required_text_arg(request, "text", "type_text requires text")?;
            send_text(text)?;
            Ok(success("SendInput text completed"))
        }
        ActionName::PressKey => {
            focus_request_window(request);
            let keys = parse_keys(request)?;
            press_keys(&keys)?;
            Ok(success("SendInput key sequence completed"))
        }
        ActionName::SetValue => {
            focus_request_window(request);
            let value = required_text_arg(request, "value", "set_value requires value")?;
            press_keys(&["Ctrl".to_string(), "A".to_string()])?;
            send_text(value)?;
            Ok(ActionOutcome {
                success: true,
                message: "Windows v1 used SendInput focus/select-all/type fallback for set_value"
                    .to_string(),
                code: "Completed".to_string(),
                diagnostics: vec![sky_cua_platform::model::DiagnosticEntry {
                    code: BackendErrorCode::AccessibilityCoverageLimited
                        .as_str()
                        .to_string(),
                    message:
                        "UI Automation ValuePattern was unavailable; physical fallback was used"
                            .to_string(),
                    details: None,
                }],
                agent_cursor: None,
            })
        }
    }
}

fn execute_window_message_action(request: &ActionRequest) -> Result<ActionOutcome, BackendError> {
    match request.action {
        ActionName::FocusElement
        | ActionName::ActivateElement
        | ActionName::SelectElement
        | ActionName::ExpandElement
        | ActionName::CollapseElement
        | ActionName::ToggleElement
        | ActionName::PerformAction => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "this semantic automation primitive requires Windows UI Automation",
        )),
        ActionName::Click => {
            let hwnd = request_hwnd(request)?;
            let (x, y) = desktop_action_point(request)?;
            legacy_cursor_click(hwnd, x, y, MouseButton::Left)?;
            Ok(success("Windows RDP cursor click completed"))
        }
        ActionName::PerformSecondaryAction => {
            let hwnd = request_hwnd(request)?;
            let (x, y) = desktop_action_point(request)?;
            legacy_cursor_click(hwnd, x, y, MouseButton::Right)?;
            Ok(success("Windows RDP cursor secondary click completed"))
        }
        ActionName::Scroll => {
            let hwnd = request_hwnd(request)?;
            let point = desktop_action_point(request).ok();
            let delta_y = scroll_delta_y(request);
            legacy_cursor_scroll(hwnd, point, delta_y)?;
            Ok(success("Windows RDP cursor scroll completed"))
        }
        ActionName::Drag => {
            let hwnd = request_hwnd(request)?;
            let (from_x, from_y) = desktop_drag_from_point(request)?;
            let (to_x, to_y) = desktop_target_point(request)?;
            legacy_cursor_drag(hwnd, from_x, from_y, to_x, to_y)?;
            Ok(success("Windows RDP cursor drag completed"))
        }
        ActionName::TypeText => {
            let hwnd = request_hwnd(request)?;
            let text = required_text_arg(request, "text", "type_text requires text")?;
            post_text(hwnd, text)?;
            Ok(success("Windows RDP message text completed"))
        }
        ActionName::PressKey => {
            let hwnd = request_hwnd(request)?;
            let keys = parse_keys(request)?;
            post_keys(hwnd, &keys)?;
            Ok(success("Windows RDP message key sequence completed"))
        }
        ActionName::SetValue => {
            let hwnd = request_hwnd(request)?;
            let value = required_text_arg(request, "value", "set_value requires value")?;
            post_keys(hwnd, &["Ctrl".to_string(), "A".to_string()])?;
            post_text(hwnd, value)?;
            Ok(ActionOutcome {
                success: true,
                message: "Windows v1 used RDP message focus/select-all/type fallback for set_value"
                    .to_string(),
                code: "Completed".to_string(),
                diagnostics: vec![sky_cua_platform::model::DiagnosticEntry {
                    code: BackendErrorCode::AccessibilityCoverageLimited
                        .as_str()
                        .to_string(),
                    message:
                        "UI Automation ValuePattern was unavailable; RDP message fallback was used"
                            .to_string(),
                    details: None,
                }],
                agent_cursor: None,
            })
        }
    }
}

fn uia_backend_ref_for_fallback(request: &ActionRequest) -> Option<&str> {
    request
        .resolved_element
        .as_ref()
        .and_then(|element| element.backend_ref.as_deref())
        .filter(|backend_ref| backend_ref.starts_with("uia:"))
}

fn required_text_arg<'a>(
    request: &'a ActionRequest,
    name: &str,
    message: &str,
) -> Result<&'a str, BackendError> {
    request
        .arguments
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| BackendError::new(BackendErrorCode::InvalidRequest, message))
}

fn availability(available: bool, reason: &str) -> ToolAvailability {
    ToolAvailability {
        available,
        reason: (!available).then(|| reason.to_string()),
    }
}

fn success(message: &str) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: message.to_string(),
        code: "Completed".to_string(),
        diagnostics: Vec::new(),
        agent_cursor: None,
    }
}

impl CaptureBlankFrame {
    fn as_str(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::White => "white",
        }
    }
}

fn scroll_delta_y(request: &ActionRequest) -> f64 {
    if let Some(delta_y) = request
        .arguments
        .get("delta_y")
        .and_then(serde_json::Value::as_f64)
    {
        return delta_y;
    }
    let pages = request
        .arguments
        .get("pages")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(1.0)
        .max(1.0);
    match request
        .arguments
        .get("direction")
        .and_then(serde_json::Value::as_str)
    {
        Some("up") => 120.0 * pages,
        _ => -120.0 * pages,
    }
}

fn select_window(windows: &[WindowInfo], selector: &AppSelector) -> Option<WindowInfo> {
    windows
        .iter()
        .find(|window| {
            selector.app_id.as_ref().is_some_and(|app_id| {
                app_id.eq_ignore_ascii_case(&format!("hwnd:0x{:x}", window.hwnd))
            }) || selector.window_title.as_ref().is_some_and(|title| {
                window.title.eq_ignore_ascii_case(title) || window.title.contains(title)
            }) || selector.name.as_ref().is_some_and(|name| {
                window.executable.as_deref().is_some_and(|exe| {
                    exe.to_ascii_lowercase()
                        .contains(&name.to_ascii_lowercase())
                }) || window
                    .title
                    .to_ascii_lowercase()
                    .contains(&name.to_ascii_lowercase())
            })
        })
        .cloned()
}

fn window_element(window: &WindowInfo, element_index: usize) -> ElementNode {
    ElementNode {
        element_index,
        parent_index: None,
        role: "window".to_string(),
        name: Some(window.title.clone()),
        description: Some(
            "Top-level Win32 window fallback with screenshot-local bounds".to_string(),
        ),
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags: if window.is_foreground {
            vec!["focused".to_string(), "win32_fallback".to_string()]
        } else {
            vec!["win32_fallback".to_string()]
        },
        semantic_actions: Vec::new(),
        bounds: Some(RectF {
            x: 0.0,
            y: 0.0,
            width: window.bounds.width,
            height: window.bounds.height,
            space: CoordinateSpace::StreamPixels,
        }),
        backend_ref: Some(format!("hwnd:0x{:x}", window.hwnd)),
    }
}

#[derive(Debug, Clone)]
struct CaptureSource {
    hwnd: Option<usize>,
    width: i32,
    height: i32,
    logical_rect: Option<RectF>,
}

#[derive(Debug, Clone)]
struct CaptureResult {
    capture: CaptureInfo,
    blank_frame: Option<CaptureBlankFrame>,
}

#[derive(Debug, Clone)]
struct CaptureImageResult {
    pixel_size: PixelSize,
    blank_frame: Option<CaptureBlankFrame>,
}

async fn capture_desktop(
    snapshot_id: &str,
    window: Option<&WindowInfo>,
) -> Result<CaptureResult, BackendError> {
    let output_path = capture_output_path(snapshot_id)?;
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to create Windows capture directory {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    let path = output_path.clone();
    let source = capture_source(window)?;
    let logical_rect = source.logical_rect.clone();
    let image_result = tokio::task::spawn_blocking(move || capture_desktop_blocking(&path, source))
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("Windows capture task failed to join cleanly: {error}"),
            )
        })??;

    let capture = CaptureInfo {
        backend: CaptureBackendKind::WindowsGdi,
        image_backend: Some(CaptureBackendKind::WindowsGdi),
        coordinate_space: Some(CoordinateSpace::StreamPixels),
        stream_id: None,
        source_type: None,
        mapping_id: None,
        logical_rect,
        pixel_size: Some(image_result.pixel_size.clone()),
        original_pixel_size: Some(image_result.pixel_size),
        logical_to_pixel_scale: Some(1.0),
        screenshot_path: Some(output_path.display().to_string()),
        original_screenshot_path: None,
        model_image_format: Some(ModelImageFormat::Jpeg),
        model_image_quality: Some(85),
        model_image_bytes: std::fs::metadata(&output_path).ok().map(|meta| meta.len()),
        model_image_encode_ms: None,
    };
    Ok(CaptureResult {
        capture,
        blank_frame: image_result.blank_frame,
    })
}

fn capture_source(window: Option<&WindowInfo>) -> Result<CaptureSource, BackendError> {
    if let Some(window) = window {
        return Ok(CaptureSource {
            hwnd: Some(window.hwnd),
            width: rounded_positive_i32(window.bounds.width, "window width")?,
            height: rounded_positive_i32(window.bounds.height, "window height")?,
            logical_rect: Some(window.bounds.clone()),
        });
    }

    unsafe {
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if width <= 0 || height <= 0 {
            return Err(BackendError::new(
                BackendErrorCode::Internal,
                "Windows virtual screen dimensions are invalid",
            ));
        }
        Ok(CaptureSource {
            hwnd: None,
            width,
            height,
            logical_rect: Some(RectF {
                x: f64::from(GetSystemMetrics(SM_XVIRTUALSCREEN)),
                y: f64::from(GetSystemMetrics(SM_YVIRTUALSCREEN)),
                width: f64::from(width),
                height: f64::from(height),
                space: CoordinateSpace::DesktopLogical,
            }),
        })
    }
}

fn rounded_positive_i32(value: f64, label: &str) -> Result<i32, BackendError> {
    if value <= 0.0 || value > f64::from(i32::MAX) {
        return Err(BackendError::new(
            BackendErrorCode::Internal,
            format!("Windows capture {label} is invalid: {value}"),
        ));
    }
    Ok(value.round() as i32)
}

fn empty_capture() -> CaptureInfo {
    CaptureInfo {
        backend: CaptureBackendKind::WindowsGdi,
        image_backend: None,
        coordinate_space: Some(CoordinateSpace::StreamPixels),
        stream_id: None,
        source_type: None,
        mapping_id: None,
        logical_rect: None,
        pixel_size: None,
        original_pixel_size: None,
        logical_to_pixel_scale: Some(1.0),
        screenshot_path: None,
        original_screenshot_path: None,
        model_image_format: None,
        model_image_quality: None,
        model_image_bytes: None,
        model_image_encode_ms: None,
    }
}

fn capture_output_path(snapshot_id: &str) -> Result<PathBuf, BackendError> {
    let dir = sky_cua_state_dir().map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to resolve Windows capture state directory: {error}"),
        )
    })?;
    Ok(dir.join("captures").join(format!("{snapshot_id}.jpg")))
}

fn capture_desktop_blocking(
    path: &PathBuf,
    source: CaptureSource,
) -> Result<CaptureImageResult, BackendError> {
    unsafe {
        let capture_window = if let Some(hwnd) = source.hwnd {
            hwnd as HWND
        } else {
            GetDesktopWindow()
        };
        let width = source.width;
        let height = source.height;
        let screen_dc = GetWindowDC(capture_window);
        if screen_dc.is_null() {
            return Err(win32_error("GetDC failed for the virtual screen"));
        }
        let memory_dc = CreateCompatibleDC(screen_dc);
        if memory_dc.is_null() {
            ReleaseDC(capture_window, screen_dc);
            return Err(win32_error(
                "CreateCompatibleDC failed for screenshot capture",
            ));
        }
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        if bitmap.is_null() {
            cleanup_capture_dc(capture_window, screen_dc, memory_dc, null_mut());
            return Err(win32_error(
                "CreateCompatibleBitmap failed for screenshot capture",
            ));
        }
        let old = SelectObject(memory_dc, bitmap as HGDIOBJ);
        let copied = if source.hwnd.is_some() {
            PrintWindow(capture_window, memory_dc, 0)
        } else {
            BitBlt(memory_dc, 0, 0, width, height, screen_dc, 0, 0, SRCCOPY)
        };
        if copied == 0 {
            SelectObject(memory_dc, old);
            cleanup_capture_dc(capture_window, screen_dc, memory_dc, bitmap);
            return Err(win32_error(
                "Windows GDI copy failed for screenshot capture",
            ));
        }

        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let byte_len = usize::try_from(width)
            .ok()
            .and_then(|w| usize::try_from(height).ok().map(|h| w * h * 4))
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    "screenshot dimensions overflowed",
                )
            })?;
        let mut pixels = vec![0u8; byte_len];
        let rows = GetDIBits(
            memory_dc,
            bitmap,
            0,
            height as u32,
            pixels.as_mut_ptr().cast::<c_void>(),
            &mut info,
            DIB_RGB_COLORS,
        );
        SelectObject(memory_dc, old);
        cleanup_capture_dc(capture_window, screen_dc, memory_dc, bitmap);
        if rows == 0 {
            return Err(win32_error("GetDIBits failed for screenshot capture"));
        }

        let mut rgb_pixels = Vec::with_capacity(
            usize::try_from(width)
                .unwrap_or_default()
                .saturating_mul(usize::try_from(height).unwrap_or_default())
                .saturating_mul(3),
        );
        for pixel in pixels.chunks_exact(4) {
            rgb_pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        let blank_frame = detect_blank_rgb_frame(&rgb_pixels);
        let image =
            ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width as u32, height as u32, rgb_pixels)
                .ok_or_else(|| {
                    BackendError::new(
                        BackendErrorCode::Internal,
                        "failed to build image buffer from Windows screenshot bytes",
                    )
                })?;
        image
            .save_with_format(path, image::ImageFormat::Jpeg)
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!(
                        "failed to write Windows screenshot {}: {error}",
                        path.display()
                    ),
                )
            })?;
        Ok(CaptureImageResult {
            pixel_size: PixelSize {
                width: width as u32,
                height: height as u32,
            },
            blank_frame,
        })
    }
}

fn detect_blank_rgb_frame(rgb_pixels: &[u8]) -> Option<CaptureBlankFrame> {
    let mut chunks = rgb_pixels.chunks_exact(3);
    if !chunks.remainder().is_empty() {
        return None;
    }
    let pixel_count = chunks.len();
    if pixel_count == 0 {
        return None;
    }

    let mut dark = 0usize;
    let mut light = 0usize;
    for pixel in chunks.by_ref() {
        let [red, green, blue]: [u8; 3] = [pixel[0], pixel[1], pixel[2]];
        if red <= 8 && green <= 8 && blue <= 8 {
            dark += 1;
        }
        if red >= 247 && green >= 247 && blue >= 247 {
            light += 1;
        }
    }

    let threshold = ((pixel_count as f64) * 0.995).ceil() as usize;
    if dark >= threshold {
        Some(CaptureBlankFrame::Black)
    } else if light >= threshold {
        Some(CaptureBlankFrame::White)
    } else {
        None
    }
}

unsafe fn cleanup_capture_dc(window: HWND, screen_dc: HDC, memory_dc: HDC, bitmap: HBITMAP) {
    if !bitmap.is_null() {
        unsafe { DeleteObject(bitmap as HGDIOBJ) };
    }
    if !memory_dc.is_null() {
        unsafe { DeleteDC(memory_dc) };
    }
    if !screen_dc.is_null() {
        unsafe { ReleaseDC(window, screen_dc) };
    }
}

fn enumerate_windows() -> Vec<WindowInfo> {
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> i32 {
        let windows = unsafe { &mut *(lparam as *mut Vec<WindowInfo>) };
        if let Some(window) = unsafe { window_info(hwnd) } {
            windows.push(window);
        }
        1
    }

    let mut windows = Vec::new();
    unsafe {
        EnumWindows(
            Some(callback),
            &mut windows as *mut Vec<WindowInfo> as LPARAM,
        );
    }
    windows
}

unsafe fn window_info(hwnd: HWND) -> Option<WindowInfo> {
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return None;
    }
    let title = unsafe { window_title(hwnd) };
    if title.trim().is_empty() {
        return None;
    }
    let mut rect: RECT = unsafe { zeroed() };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return None;
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let pid = (pid != 0).then_some(pid);
    let executable = pid.and_then(|pid| unsafe { executable_for_pid(pid) });
    Some(WindowInfo {
        hwnd: hwnd as usize,
        title,
        pid,
        executable,
        bounds: RectF {
            x: f64::from(rect.left),
            y: f64::from(rect.top),
            width: f64::from(width),
            height: f64::from(height),
            space: CoordinateSpace::DesktopLogical,
        },
        is_foreground: std::ptr::eq(hwnd, unsafe { GetForegroundWindow() }),
    })
}

unsafe fn window_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; len as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

unsafe fn executable_for_pid(pid: u32) -> Option<String> {
    let handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32768];
    let len = unsafe {
        K32GetModuleFileNameExW(handle, null_mut(), buffer.as_mut_ptr(), buffer.len() as u32)
    };
    unsafe { CloseHandle(handle) };
    (len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
}

fn action_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    if let Some(element) = request.resolved_element.as_ref()
        && let Some(bounds) = element.bounds.as_ref()
    {
        return Ok((
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ));
    }
    let x = request
        .arguments
        .get("x")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "action requires x or element_index",
            )
        })?;
    let y = request
        .arguments
        .get("y")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "action requires y or element_index",
            )
        })?;
    Ok((x, y))
}

fn desktop_action_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    stream_to_desktop_point(request, action_point(request)?)
}

fn drag_from_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    if let Some(point) = point_from_fields(&request.arguments, "from_x", "from_y") {
        return Ok(point);
    }
    action_point(request)
}

fn desktop_drag_from_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    stream_to_desktop_point(request, drag_from_point(request)?)
}

fn target_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    if let Some(element) = request.resolved_target_element.as_ref()
        && let Some(bounds) = element.bounds.as_ref()
    {
        return Ok((
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ));
    }
    let x = request
        .arguments
        .get("to_x")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "drag requires to_x or to_element_index",
            )
        })?;
    let y = request
        .arguments
        .get("to_y")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "drag requires to_y or to_element_index",
            )
        })?;
    Ok((x, y))
}

fn point_from_fields(
    arguments: &serde_json::Value,
    x_field: &str,
    y_field: &str,
) -> Option<(f64, f64)> {
    let x = arguments.get(x_field).and_then(serde_json::Value::as_f64)?;
    let y = arguments.get(y_field).and_then(serde_json::Value::as_f64)?;
    Some((x, y))
}

fn desktop_target_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    stream_to_desktop_point(request, target_point(request)?)
}

fn stream_to_desktop_point(
    request: &ActionRequest,
    (x, y): (f64, f64),
) -> Result<(f64, f64), BackendError> {
    let Some(capture) = request.resolved_capture.as_ref() else {
        return Ok((x, y));
    };
    if capture.coordinate_space != Some(CoordinateSpace::StreamPixels) {
        return Ok((x, y));
    }
    let Some(rect) = capture.logical_rect.as_ref() else {
        return Ok((x, y));
    };
    let scale = capture.logical_to_pixel_scale.unwrap_or(1.0);
    if scale <= 0.0 {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("invalid Windows capture scale: {scale}"),
        ));
    }
    Ok((rect.x + x / scale, rect.y + y / scale))
}

fn focus_request_window(request: &ActionRequest) {
    let Some(handle) = request
        .resolved_focused_app
        .as_ref()
        .and_then(|app| app.window_handle.as_deref())
        .and_then(parse_hwnd)
    else {
        return;
    };
    unsafe {
        SetForegroundWindow(handle);
    }
}

fn request_hwnd(request: &ActionRequest) -> Result<HWND, BackendError> {
    request
        .resolved_focused_app
        .as_ref()
        .and_then(|app| app.window_handle.as_deref())
        .and_then(parse_hwnd)
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "Windows RDP message input requires a focused app window_handle from a fresh snapshot",
            )
        })
}

fn parse_hwnd(value: &str) -> Option<HWND> {
    usize::from_str_radix(value.trim_start_matches("0x"), 16)
        .ok()
        .map(|value| value as HWND)
}

fn legacy_cursor_click(
    hwnd: HWND,
    x: f64,
    y: f64,
    button: MouseButton,
) -> Result<(), BackendError> {
    focus_window_for_cursor_input(hwnd);
    set_cursor_pos(x, y)?;
    legacy_mouse_button(button, true);
    std::thread::sleep(std::time::Duration::from_millis(80));
    legacy_mouse_button(button, false);
    Ok(())
}

fn legacy_cursor_drag(
    hwnd: HWND,
    from_x: f64,
    from_y: f64,
    to_x: f64,
    to_y: f64,
) -> Result<(), BackendError> {
    focus_window_for_cursor_input(hwnd);
    set_cursor_pos(from_x, from_y)?;
    legacy_mouse_button(MouseButton::Left, true);
    std::thread::sleep(std::time::Duration::from_millis(80));
    set_cursor_pos(to_x, to_y)?;
    std::thread::sleep(std::time::Duration::from_millis(80));
    legacy_mouse_button(MouseButton::Left, false);
    Ok(())
}

fn legacy_cursor_scroll(
    hwnd: HWND,
    point: Option<(f64, f64)>,
    delta_y: f64,
) -> Result<(), BackendError> {
    focus_window_for_cursor_input(hwnd);
    if let Some((x, y)) = point {
        set_cursor_pos(x, y)?;
    }
    unsafe { mouse_event(MOUSEEVENTF_WHEEL, 0, 0, wheel_data(delta_y), 0) };
    Ok(())
}

fn focus_window_for_cursor_input(hwnd: HWND) {
    unsafe {
        SetForegroundWindow(hwnd);
    }
    std::thread::sleep(std::time::Duration::from_millis(150));
}

fn set_cursor_pos(x: f64, y: f64) -> Result<(), BackendError> {
    let target_x = x.round() as i32;
    let target_y = y.round() as i32;
    if unsafe { SetCursorPos(target_x, target_y) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(0) {
            return Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("SetCursorPos failed: {error}"),
            ));
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    let Some(actual) = cursor_pos()? else {
        return Ok(());
    };
    if (actual.x - target_x).abs() <= 1 && (actual.y - target_y).abs() <= 1 {
        Ok(())
    } else {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!(
                "SetCursorPos did not reach target: requested {target_x},{target_y}; actual {},{}",
                actual.x, actual.y
            ),
        ))
    }
}

fn cursor_pos() -> Result<Option<POINT>, BackendError> {
    let mut point = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&raw mut point) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(5) {
            Ok(None)
        } else {
            Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("GetCursorPos failed: {error}"),
            ))
        }
    } else {
        Ok(Some(point))
    }
}

fn legacy_mouse_button(button: MouseButton, pressed: bool) {
    let flags = match (button, pressed) {
        (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
        (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
        (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
        (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
    };
    unsafe { mouse_event(flags, 0, 0, 0, 0) };
}

fn post_text(hwnd: HWND, text: &str) -> Result<(), BackendError> {
    let target = focused_message_hwnd().unwrap_or(hwnd);
    for unit in text.encode_utf16() {
        post_window_message(target, WM_CHAR, unit as WPARAM, 0)?;
    }
    Ok(())
}

fn post_keys(hwnd: HWND, keys: &[String]) -> Result<(), BackendError> {
    let target = focused_message_hwnd().unwrap_or(hwnd);
    if keys.is_empty() {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "press_key requires at least one key",
        ));
    }
    let virtual_keys = keys
        .iter()
        .map(|key| virtual_key(key))
        .collect::<Option<Vec<_>>>();
    if let Some(virtual_keys) = virtual_keys {
        for key in &virtual_keys {
            post_window_message(target, WM_KEYDOWN, *key as WPARAM, 0)?;
        }
        for key in virtual_keys.iter().rev() {
            post_window_message(target, WM_KEYUP, *key as WPARAM, 0)?;
        }
        return Ok(());
    }
    if keys.len() == 1 {
        post_text(target, &keys[0])
    } else {
        Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("unsupported Windows key chord: {}", keys.join("+")),
        ))
    }
}

fn focused_message_hwnd() -> Option<HWND> {
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(0, &raw mut info) } == 0 {
        return None;
    }
    (!info.hwndFocus.is_null())
        .then_some(info.hwndFocus)
        .or_else(|| (!info.hwndCaret.is_null()).then_some(info.hwndCaret))
}

fn post_window_message(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Result<(), BackendError> {
    if unsafe { PostMessageW(hwnd, message, wparam, lparam) } == 0 {
        Err(win32_error("PostMessageW failed"))
    } else {
        Ok(())
    }
}

fn click_at(x: f64, y: f64, button: MouseButton) -> Result<(), BackendError> {
    move_pointer(x, y)?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    mouse_button(button, true)?;
    std::thread::sleep(std::time::Duration::from_millis(90));
    mouse_button(button, false)
}

fn drag(from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> Result<(), BackendError> {
    move_pointer(from_x, from_y)?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    mouse_button(MouseButton::Left, true)?;
    std::thread::sleep(std::time::Duration::from_millis(90));
    move_pointer(to_x, to_y)?;
    std::thread::sleep(std::time::Duration::from_millis(90));
    mouse_button(MouseButton::Left, false)
}

fn move_pointer(x: f64, y: f64) -> Result<(), BackendError> {
    let (absolute_x, absolute_y) = absolute_pointer_coords(x, y);
    send_mouse(
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        absolute_x,
        absolute_y,
        0,
    )
}

fn mouse_button(button: MouseButton, pressed: bool) -> Result<(), BackendError> {
    let flags = match (button, pressed) {
        (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
        (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
        (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
        (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
    };
    send_mouse(flags, 0, 0, 0)
}

fn wheel(delta_y: f64) -> Result<(), BackendError> {
    send_mouse(MOUSEEVENTF_WHEEL, 0, 0, wheel_data(delta_y))
}

fn wheel_data(delta_y: f64) -> i32 {
    if delta_y == 0.0 {
        0
    } else if delta_y.is_sign_positive() {
        120
    } else {
        -120
    }
}

fn send_mouse(flags: u32, dx: i32, dy: i32, mouse_data: i32) -> Result<(), BackendError> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_inputs(&[input])
}

fn absolute_pointer_coords(x: f64, y: f64) -> (i32, i32) {
    unsafe {
        let left = f64::from(GetSystemMetrics(SM_XVIRTUALSCREEN));
        let top = f64::from(GetSystemMetrics(SM_YVIRTUALSCREEN));
        let width = f64::from(GetSystemMetrics(SM_CXVIRTUALSCREEN));
        let height = f64::from(GetSystemMetrics(SM_CYVIRTUALSCREEN));
        let absolute_x = absolute_pointer_coord(x, left, width);
        let absolute_y = absolute_pointer_coord(y, top, height);
        (absolute_x, absolute_y)
    }
}

fn absolute_pointer_coord(value: f64, origin: f64, span: f64) -> i32 {
    if span <= 1.0 {
        return 0;
    }
    (((value - origin) * 65535.0) / (span - 1.0))
        .round()
        .clamp(0.0, 65535.0) as i32
}

fn send_text(text: &str) -> Result<(), BackendError> {
    for unit in text.encode_utf16() {
        send_key_unicode(unit, false)?;
        send_key_unicode(unit, true)?;
    }
    Ok(())
}

fn parse_keys(request: &ActionRequest) -> Result<Vec<String>, BackendError> {
    if let Some(key) = request
        .arguments
        .get("key")
        .and_then(serde_json::Value::as_str)
    {
        return Ok(key.split('+').map(|part| part.trim().to_string()).collect());
    }
    let Some(keys) = request
        .arguments
        .get("keys")
        .and_then(serde_json::Value::as_array)
    else {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "press_key requires key or keys",
        ));
    };
    Ok(keys
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect())
}

fn press_keys(keys: &[String]) -> Result<(), BackendError> {
    if keys.is_empty() {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "press_key requires at least one key",
        ));
    }
    let virtual_keys = keys
        .iter()
        .map(|key| virtual_key(key))
        .collect::<Option<Vec<_>>>();
    if let Some(virtual_keys) = virtual_keys {
        for key in &virtual_keys {
            send_key(*key, false)?;
        }
        for key in virtual_keys.iter().rev() {
            send_key(*key, true)?;
        }
        return Ok(());
    }
    if keys.len() == 1 {
        send_text(&keys[0])
    } else {
        Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("unsupported Windows key chord: {}", keys.join("+")),
        ))
    }
}

fn virtual_key(key: &str) -> Option<VIRTUAL_KEY> {
    match key.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(VK_CONTROL),
        "alt" => Some(VK_MENU),
        "shift" => Some(VK_SHIFT),
        "enter" | "return" => Some(VK_RETURN),
        "esc" | "escape" => Some(VK_ESCAPE),
        "tab" => Some(VK_TAB),
        "backspace" => Some(VK_BACK),
        value if value.len() == 1 => value
            .chars()
            .next()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .map(|ch| ch.to_ascii_uppercase() as u16),
        _ => None,
    }
}

fn send_key(key: VIRTUAL_KEY, released: bool) -> Result<(), BackendError> {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if released { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_inputs(&[input])
}

fn send_key_unicode(unit: u16, released: bool) -> Result<(), BackendError> {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: unit,
                dwFlags: KEYEVENTF_UNICODE | if released { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_inputs(&[input])
}

fn send_inputs(inputs: &[INPUT]) -> Result<(), BackendError> {
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(win32_error("SendInput failed"))
    }
}

fn send_input_available() -> bool {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(1, &raw const input, size_of::<INPUT>() as i32) };
    sent == 1
}

fn select_input_backend() -> WindowsInputBackend {
    if send_input_available() {
        WindowsInputBackend::SendInput
    } else if is_rdp_session() {
        WindowsInputBackend::WindowMessages
    } else {
        WindowsInputBackend::None
    }
}

fn is_rdp_session() -> bool {
    std::env::var("SESSIONNAME")
        .map(|session| session.to_ascii_lowercase().starts_with("rdp-"))
        .unwrap_or(false)
}

fn win32_error(message: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!("{message}: {}", std::io::Error::last_os_error()),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureBlankFrame, WindowInfo, WindowsInputBackend, absolute_pointer_coord,
        absolute_pointer_coords, desktop_action_point, desktop_drag_from_point,
        detect_blank_rgb_frame, parse_hwnd, scroll_delta_y, uia_backend_ref_for_fallback,
        virtual_key, window_element,
    };
    use sky_cua_platform::model::{
        ActionName, ActionRequest, CaptureBackendKind, CaptureInfo, CoordinateSpace, ElementNode,
        InputBackendKind, PixelSize, RectF,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_RETURN};

    #[test]
    fn parses_hex_window_handle() {
        assert!(parse_hwnd("0x10").is_some());
    }

    #[test]
    fn maps_common_virtual_keys() {
        assert_eq!(virtual_key("Ctrl"), Some(VK_CONTROL));
        assert_eq!(virtual_key("Enter"), Some(VK_RETURN));
    }

    #[test]
    fn maps_window_message_backend_to_wire_value() {
        assert_eq!(
            WindowsInputBackend::WindowMessages.model_kind(),
            InputBackendKind::WindowsMessages
        );
    }

    #[test]
    fn maps_stream_pixel_action_coordinates_to_desktop_coordinates() {
        let request = action_request(
            serde_json::json!({ "x": 21.0, "y": 305.0 }),
            Some(capture_with_rect(420.0, 184.0, 1732.0, 1070.0, 1.0)),
            None,
        );

        assert_eq!(desktop_action_point(&request).unwrap(), (441.0, 489.0));
    }

    #[test]
    fn maps_scaled_stream_pixel_action_coordinates_to_desktop_coordinates() {
        let request = action_request(
            serde_json::json!({ "x": 1280.0, "y": 720.0 }),
            Some(capture_with_rect(1920.0, 0.0, 1280.0, 720.0, 2.0)),
            None,
        );

        assert_eq!(desktop_action_point(&request).unwrap(), (2560.0, 360.0));
    }

    #[test]
    fn maps_element_center_from_stream_pixels_to_desktop_coordinates() {
        let request = action_request(
            serde_json::json!({}),
            Some(capture_with_rect(420.0, 184.0, 1732.0, 1070.0, 1.0)),
            Some(ElementNode {
                element_index: 0,
                parent_index: None,
                role: "button".to_string(),
                name: Some("Settings".to_string()),
                description: None,
                value: None,
                text: None,
                numeric_value: None,
                supports_editable_text: false,
                state_flags: Vec::new(),
                semantic_actions: Vec::new(),
                bounds: Some(RectF {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                    space: CoordinateSpace::StreamPixels,
                }),
                backend_ref: None,
            }),
        );

        assert_eq!(desktop_action_point(&request).unwrap(), (445.0, 224.0));
    }

    #[test]
    fn identifies_uia_backend_refs_for_fallback_diagnostics() {
        let request = action_request(
            serde_json::json!({}),
            None,
            Some(ElementNode {
                element_index: 0,
                parent_index: None,
                role: "button".to_string(),
                name: Some("Open".to_string()),
                description: None,
                value: None,
                text: None,
                numeric_value: None,
                supports_editable_text: false,
                state_flags: Vec::new(),
                semantic_actions: vec!["click".to_string()],
                bounds: None,
                backend_ref: Some("uia:hwnd=0x10;path=0".to_string()),
            }),
        );

        assert_eq!(
            uia_backend_ref_for_fallback(&request),
            Some("uia:hwnd=0x10;path=0")
        );
    }

    #[test]
    fn maps_drag_from_coordinates_from_stream_pixels_to_desktop_coordinates() {
        let request = action_request(
            serde_json::json!({ "from_x": 21.0, "from_y": 305.0 }),
            Some(capture_with_rect(420.0, 184.0, 1732.0, 1070.0, 1.0)),
            None,
        );

        assert_eq!(desktop_drag_from_point(&request).unwrap(), (441.0, 489.0));
    }

    #[test]
    fn window_fallback_element_uses_screenshot_local_bounds() {
        let window = WindowInfo {
            hwnd: 0x10,
            title: "Sumwall Browser".to_string(),
            pid: Some(42),
            executable: None,
            bounds: RectF {
                x: 420.0,
                y: 184.0,
                width: 1732.0,
                height: 1070.0,
                space: CoordinateSpace::DesktopLogical,
            },
            is_foreground: true,
        };

        let bounds = window_element(&window, 0).bounds.unwrap();
        assert_eq!(bounds.x, 0.0);
        assert_eq!(bounds.y, 0.0);
        assert_eq!(bounds.width, 1732.0);
        assert_eq!(bounds.height, 1070.0);
        assert_eq!(bounds.space, CoordinateSpace::StreamPixels);
    }

    #[test]
    fn absolute_pointer_coords_are_clamped() {
        let (x, y) = absolute_pointer_coords(-100000.0, -100000.0);
        assert!(x >= 0);
        assert!(y >= 0);
    }

    #[test]
    fn absolute_pointer_coord_handles_degenerate_spans() {
        assert_eq!(absolute_pointer_coord(0.0, 0.0, 0.0), 0);
        assert_eq!(absolute_pointer_coord(0.0, 0.0, 1.0), 0);
    }

    #[test]
    fn scroll_delta_uses_direction_and_pages() {
        let request = action_request(
            serde_json::json!({ "direction": "up", "pages": 2 }),
            None,
            None,
        );

        assert_eq!(scroll_delta_y(&request), 240.0);

        let request = action_request(serde_json::json!({ "direction": "down" }), None, None);

        assert_eq!(scroll_delta_y(&request), -120.0);
    }

    #[test]
    fn explicit_scroll_delta_takes_precedence() {
        let request = action_request(
            serde_json::json!({ "direction": "up", "pages": 2, "delta_y": -360.0 }),
            None,
            None,
        );

        assert_eq!(scroll_delta_y(&request), -360.0);
    }

    #[test]
    fn detects_black_and_white_blank_capture_frames() {
        let black = vec![0u8; 300];
        let white = vec![255u8; 300];

        assert_eq!(
            detect_blank_rgb_frame(&black),
            Some(CaptureBlankFrame::Black)
        );
        assert_eq!(
            detect_blank_rgb_frame(&white),
            Some(CaptureBlankFrame::White)
        );
    }

    #[test]
    fn does_not_mark_varied_capture_frames_as_blank() {
        let mut pixels = vec![0u8; 300];
        for (index, value) in pixels.iter_mut().enumerate() {
            *value = (index % 251) as u8;
        }

        assert_eq!(detect_blank_rgb_frame(&pixels), None);
    }

    fn action_request(
        arguments: serde_json::Value,
        resolved_capture: Option<CaptureInfo>,
        resolved_element: Option<ElementNode>,
    ) -> ActionRequest {
        ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot".to_string()),
            element_index: None,
            arguments,
            resolved_element,
            resolved_target_element: None,
            resolved_capture,
            resolved_focused_app: None,
            environment: None,
        }
    }

    fn capture_with_rect(x: f64, y: f64, width: f64, height: f64, scale: f64) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::WindowsGdi,
            image_backend: Some(CaptureBackendKind::WindowsGdi),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: Some(RectF {
                x,
                y,
                width,
                height,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: (width * scale) as u32,
                height: (height * scale) as u32,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale: Some(scale),
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }
}
