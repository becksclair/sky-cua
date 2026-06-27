use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::Once;

use image::{ImageBuffer, Rgb};
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AppInfo, AppSelector, AppStateSnapshot,
    CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace, DisplayInfo,
    DisplayIntersection, DisplayRef, DisplayTarget, DoctorReport, ElementNode, EnvironmentInfo,
    FocusedApp, InputBackendKind, ModelImageFormat, PixelSize, PortalCapabilities, RectF,
    ScrollDirection, SemanticBackendKind, SessionKind, SessionPresenceIntent,
    SessionPresenceStatus, ToolAvailability, ToolCapabilities, WindowInfo as ModelWindowInfo,
    WindowTarget,
};
use sky_cua_platform::{new_snapshot_id, sky_cua_state_dir};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDIBits, GetMonitorInfoW,
    GetWindowDC, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
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
    GetWindowThreadProcessId, IsWindowVisible, MONITORINFOF_PRIMARY, PostMessageW,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SetCursorPos,
    SetForegroundWindow, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
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
    display: Option<DisplayRef>,
    display_intersections: Vec<DisplayIntersection>,
    is_foreground: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WindowsDesktopBackend {
    session_presence: SessionPresenceManager,
}

impl WindowsDesktopBackend {
    #[must_use]
    pub fn new() -> Self {
        ensure_dpi_awareness();
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
            display: None,
        }
    }

    fn focused_from_window(window: &WindowInfo) -> FocusedApp {
        let app = Self::window_to_app(window);
        FocusedApp {
            display: window.display.clone(),
            ..Self::focused_from_app(&app)
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
        ensure_dpi_awareness();
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
            displays: enumerate_displays(),
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

    async fn list_windows(&self) -> Result<Vec<ModelWindowInfo>, BackendError> {
        Ok(enumerate_windows()
            .into_iter()
            .map(window_to_model)
            .collect())
    }

    async fn focused_window(&self) -> Result<Option<ModelWindowInfo>, BackendError> {
        Ok(enumerate_windows()
            .into_iter()
            .find(|window| window.is_foreground)
            .map(window_to_model))
    }

    async fn activate_window(&self, target: WindowTarget) -> Result<ActionOutcome, BackendError> {
        let windows = enumerate_windows();
        let window = resolve_window_target(&windows, &target)?;
        focus_window(window.hwnd)?;
        Ok(success(&format!(
            "Activated Windows window 0x{:x}.",
            window.hwnd
        )))
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
            let focused_app = Some(Self::focused_from_window(window));
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

    async fn screenshot(
        &self,
        target: Option<WindowTarget>,
        display_target: Option<DisplayTarget>,
    ) -> Result<AppStateSnapshot, BackendError> {
        let snapshot_id = new_snapshot_id();
        let environment = self.probe_environment().await?;
        let capabilities = Self::capabilities(&environment);
        let mut diagnostics = DiagnosticBuilder::new();

        let mut focused_app = None;
        let (source, scope, display) = if let Some(target) = target {
            let windows = enumerate_windows();
            let window = resolve_window_target(&windows, &target)?;
            focus_window(window.hwnd)?;
            diagnostics.push_code(
                "WindowFocusRequested",
                format!(
                    "Requested foreground focus for Windows window 0x{:x}.",
                    window.hwnd
                ),
                None,
            );
            focused_app = Some(Self::focused_from_window(&window));
            (
                capture_source_for_window(&window)?,
                CaptureScope::Window,
                window.display.clone(),
            )
        } else if let Some(display_target) = display_target {
            let display = resolve_display_target(&environment.displays, &display_target)?;
            let display_ref = DisplayRef::from(&display);
            (
                capture_source_for_rect(&display.logical_rect, Some(display_ref.clone()))?,
                CaptureScope::Display,
                Some(display_ref),
            )
        } else if let Some(display) = primary_display(&environment.displays) {
            let display_ref = DisplayRef::from(&display);
            (
                capture_source_for_rect(&display.logical_rect, Some(display_ref.clone()))?,
                CaptureScope::PrimaryDisplay,
                Some(display_ref),
            )
        } else {
            diagnostics.push(
                BackendErrorCode::CaptureBackendDowngraded,
                "Windows monitor topology is unavailable, so screenshot fell back to the virtual desktop capture for an omitted selector.",
                None,
            );
            (
                virtual_desktop_capture_source()?,
                CaptureScope::Unknown,
                None,
            )
        };

        let capture_result = capture_desktop_with_source(&snapshot_id, source, scope, display)
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!("Windows GDI screenshot capture failed: {}", error.message),
                )
            })?;

        Ok(AppStateSnapshot {
            snapshot_id,
            created_at: chrono::Utc::now(),
            environment,
            capabilities,
            focused_app,
            capture: Some(capture_result.capture),
            elements: Vec::new(),
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

fn resolve_window_target(
    windows: &[WindowInfo],
    target: &WindowTarget,
) -> Result<WindowInfo, BackendError> {
    if let Some(window_id) = normalized_target(target.window_id.as_deref()) {
        let matches = windows
            .iter()
            .filter(|window| window_id_matches(window, &window_id))
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("window_id {window_id}"));
    }

    if let Some(app_id) = normalized_target(target.app_id.as_deref()) {
        let matches = windows
            .iter()
            .filter(|window| window_id_matches(window, &app_id))
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("app_id {app_id}"));
    }

    if let Some(pid) = target.pid {
        let matches = windows
            .iter()
            .filter(|window| window.pid == Some(pid))
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("pid {pid}"));
    }

    if let Some(wm_class) = normalized_target(target.wm_class.as_deref()) {
        let wm_class = wm_class.to_ascii_lowercase();
        let matches = windows
            .iter()
            .filter(|window| {
                window
                    .executable
                    .as_deref()
                    .is_some_and(|exe| exe.to_ascii_lowercase().contains(&wm_class))
            })
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("wm_class containing {wm_class}"));
    }

    if let Some(title) = normalized_target(target.title.as_deref()) {
        let title_lower = title.to_ascii_lowercase();
        let matches = windows
            .iter()
            .filter(|window| {
                window.title.eq_ignore_ascii_case(&title)
                    || window.title.to_ascii_lowercase().contains(&title_lower)
            })
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("title containing {title}"));
    }

    Err(BackendError::new(
        BackendErrorCode::InvalidRequest,
        "Pass window_id, pid, app_id, wm_class, or title to target a Windows window.",
    ))
}

fn window_id_matches(window: &WindowInfo, value: &str) -> bool {
    let normalized = value.trim();
    normalized.eq_ignore_ascii_case(&format!("hwnd:0x{:x}", window.hwnd))
        || normalized.eq_ignore_ascii_case(&format!("0x{:x}", window.hwnd))
}

fn unique_windows_match(
    matches: Vec<&WindowInfo>,
    description: &str,
) -> Result<WindowInfo, BackendError> {
    match matches.as_slice() {
        [window] => Ok((*window).clone()),
        [] => Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("No Windows window matched {description}."),
        )),
        windows => {
            let ids = windows
                .iter()
                .map(|window| format!("hwnd:0x{:x}", window.hwnd))
                .collect::<Vec<_>>()
                .join(", ");
            Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!(
                    "{description} matched multiple Windows windows ({ids}); add window_id to disambiguate."
                ),
            ))
        }
    }
}

fn normalized_target(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn resolve_display_target(
    displays: &[DisplayInfo],
    target: &DisplayTarget,
) -> Result<DisplayInfo, BackendError> {
    let matches = displays
        .iter()
        .filter(|display| display_matches_target(display, target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [display] => Ok((*display).clone()),
        [] => Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("no Windows display matched requested screenshot target: {target:?}"),
        )),
        _ => Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("Windows display target is ambiguous: {target:?}"),
        )),
    }
}

fn display_matches_target(display: &DisplayInfo, target: &DisplayTarget) -> bool {
    let mut matched = false;
    if let Some(value) = target.display_id.as_ref() {
        if !display.display_id.eq_ignore_ascii_case(value.trim()) {
            return false;
        }
        matched = true;
    }
    if let Some(value) = target.display_name.as_ref() {
        if !{
            let value = value.trim();
            display
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(value))
                || display.display_id.eq_ignore_ascii_case(value)
        } {
            return false;
        }
        matched = true;
    }
    if let Some(index) = target.display_index {
        if display.index != index {
            return false;
        }
        matched = true;
    }
    matched
}

fn primary_display(displays: &[DisplayInfo]) -> Option<DisplayInfo> {
    sky_cua_platform::model::primary_flagged_display(displays)
        .or_else(|| displays.first())
        .cloned()
}

fn window_to_model(window: WindowInfo) -> ModelWindowInfo {
    ModelWindowInfo {
        window_id: format!("hwnd:0x{:x}", window.hwnd),
        title: Some(window.title),
        app_id: Some(format!("hwnd:0x{:x}", window.hwnd)),
        wm_class: window.executable.clone(),
        pid: window.pid,
        bounds: Some(window.bounds),
        display: window.display,
        display_intersections: window.display_intersections,
        workspace: None,
        focused: window.is_foreground,
        hidden: false,
        client_type: Some("win32".to_string()),
        backend: "windows".to_string(),
        terminal: None,
    }
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
    source_x: i32,
    source_y: i32,
    width: i32,
    height: i32,
    logical_rect: Option<RectF>,
    display: Option<DisplayRef>,
}

#[derive(Debug, Clone)]
struct CaptureResult {
    capture: CaptureInfo,
    blank_frame: Option<CaptureBlankFrame>,
}

#[derive(Debug, Clone)]
struct CaptureImageResult {
    model: sky_cua_capture::ModelCaptureImage,
    blank_frame: Option<CaptureBlankFrame>,
}

async fn capture_desktop(
    snapshot_id: &str,
    window: Option<&WindowInfo>,
) -> Result<CaptureResult, BackendError> {
    let source = match window {
        Some(window) => capture_source_for_window(window)?,
        None => virtual_desktop_capture_source()?,
    };
    let scope = if window.is_some() {
        CaptureScope::Window
    } else {
        CaptureScope::Unknown
    };
    let display = source.display.clone();
    capture_desktop_with_source(snapshot_id, source, scope, display).await
}

async fn capture_desktop_with_source(
    snapshot_id: &str,
    source: CaptureSource,
    capture_scope: CaptureScope,
    display: Option<DisplayRef>,
) -> Result<CaptureResult, BackendError> {
    let captures_dir = captures_dir()?;
    tokio::fs::create_dir_all(&captures_dir)
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to create Windows capture directory {}: {error}",
                    captures_dir.display()
                ),
            )
        })?;
    let raw_path = raw_capture_output_path(snapshot_id)?;
    let logical_rect = source.logical_rect.clone();
    let snapshot = snapshot_id.to_owned();
    let captures_dir_for_task = captures_dir.clone();
    let image_result = tokio::task::spawn_blocking(move || {
        capture_desktop_blocking(&snapshot, &captures_dir_for_task, &raw_path, source)
    })
    .await
    .map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("Windows capture task failed to join cleanly: {error}"),
        )
    })??;

    let CaptureImageResult { model, blank_frame } = image_result;
    let mut capture = CaptureInfo {
        backend: CaptureBackendKind::WindowsGdi,
        image_backend: Some(CaptureBackendKind::WindowsGdi),
        capture_scope,
        display,
        coordinate_space: Some(CoordinateSpace::StreamPixels),
        stream_id: None,
        source_type: None,
        mapping_id: None,
        source_logical_rect: logical_rect.clone(),
        logical_rect,
        pixel_size: model.pixel_size,
        original_pixel_size: model.original_pixel_size,
        // Derived below from the model pixel size and logical rect; the model
        // image is downscaled, so the scale is no longer the identity.
        logical_to_pixel_scale: None,
        screenshot_path: Some(model.path.display().to_string()),
        original_screenshot_path: model
            .original_path
            .as_ref()
            .map(|path| path.display().to_string()),
        model_image_format: Some(match model.format {
            sky_cua_capture::ModelScreenshotFormat::Jpeg => ModelImageFormat::Jpeg,
            sky_cua_capture::ModelScreenshotFormat::Webp => ModelImageFormat::Webp,
        }),
        model_image_quality: Some(model.quality),
        model_image_bytes: model.bytes,
        model_image_encode_ms: Some(model.encode_ms),
    };
    sky_cua_capture::update_model_capture_scale(&mut capture);
    Ok(CaptureResult {
        capture,
        blank_frame,
    })
}

fn capture_source_for_window(window: &WindowInfo) -> Result<CaptureSource, BackendError> {
    Ok(CaptureSource {
        hwnd: Some(window.hwnd),
        source_x: 0,
        source_y: 0,
        width: rounded_positive_i32(window.bounds.width, "window width")?,
        height: rounded_positive_i32(window.bounds.height, "window height")?,
        logical_rect: Some(window.bounds.clone()),
        display: window.display.clone(),
    })
}

fn capture_source_for_rect(
    rect: &RectF,
    display: Option<DisplayRef>,
) -> Result<CaptureSource, BackendError> {
    Ok(CaptureSource {
        hwnd: None,
        source_x: rounded_i32(rect.x, "display x")?,
        source_y: rounded_i32(rect.y, "display y")?,
        width: rounded_positive_i32(rect.width, "display width")?,
        height: rounded_positive_i32(rect.height, "display height")?,
        logical_rect: Some(rect.clone()),
        display,
    })
}

fn virtual_desktop_capture_source() -> Result<CaptureSource, BackendError> {
    unsafe {
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if width <= 0 || height <= 0 {
            return Err(BackendError::new(
                BackendErrorCode::Internal,
                "Windows virtual screen dimensions are invalid",
            ));
        }
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        Ok(CaptureSource {
            hwnd: None,
            source_x: x,
            source_y: y,
            width,
            height,
            logical_rect: Some(RectF {
                x: f64::from(x),
                y: f64::from(y),
                width: f64::from(width),
                height: f64::from(height),
                space: CoordinateSpace::DesktopLogical,
            }),
            display: None,
        })
    }
}

fn rounded_i32(value: f64, label: &str) -> Result<i32, BackendError> {
    if value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(BackendError::new(
            BackendErrorCode::Internal,
            format!("Windows capture {label} is invalid: {value}"),
        ));
    }
    Ok(value.round() as i32)
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
        capture_scope: CaptureScope::Unknown,
        display: None,
        coordinate_space: Some(CoordinateSpace::StreamPixels),
        stream_id: None,
        source_type: None,
        mapping_id: None,
        logical_rect: None,
        source_logical_rect: None,
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

fn captures_dir() -> Result<PathBuf, BackendError> {
    let dir = sky_cua_state_dir().map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to resolve Windows capture state directory: {error}"),
        )
    })?;
    Ok(dir.join("captures"))
}

fn raw_capture_output_path(snapshot_id: &str) -> Result<PathBuf, BackendError> {
    Ok(captures_dir()?.join(format!("{snapshot_id}.png")))
}

fn capture_desktop_blocking(
    snapshot_id: &str,
    captures_dir: &Path,
    raw_path: &Path,
    source: CaptureSource,
) -> Result<CaptureImageResult, BackendError> {
    ensure_dpi_awareness();
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
            BitBlt(
                memory_dc,
                0,
                0,
                width,
                height,
                screen_dc,
                source.source_x,
                source.source_y,
                SRCCOPY,
            )
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
        // Persist the raw full-resolution capture as PNG, then derive the bounded,
        // re-encoded model image from it. Coordinate remapping relies on the raw
        // size being reported as the original pixel size.
        image
            .save_with_format(raw_path, image::ImageFormat::Png)
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!(
                        "failed to write Windows raw screenshot {}: {error}",
                        raw_path.display()
                    ),
                )
            })?;
        let original_pixel_size = PixelSize {
            width: width as u32,
            height: height as u32,
        };
        let model = sky_cua_capture::prepare_model_capture_from_image(
            captures_dir,
            snapshot_id,
            image::DynamicImage::ImageRgb8(image),
            raw_path,
            Some(original_pixel_size),
        )?;
        Ok(CaptureImageResult { model, blank_frame })
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
    ensure_dpi_awareness();
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
    let displays = enumerate_displays();
    assign_window_displays(&mut windows, &displays);
    windows
}

fn enumerate_displays() -> Vec<DisplayInfo> {
    ensure_dpi_awareness();
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> i32 {
        let displays = unsafe { &mut *(lparam as *mut Vec<DisplayInfo>) };
        if let Some(display) = unsafe { display_info(monitor, displays.len()) } {
            displays.push(display);
        }
        1
    }

    let mut displays = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            std::ptr::null(),
            Some(callback),
            &mut displays as *mut Vec<DisplayInfo> as LPARAM,
        );
    }
    normalize_displays(displays)
}

unsafe fn display_info(monitor: HMONITOR, index: usize) -> Option<DisplayInfo> {
    let mut info: MONITORINFOEXW = unsafe { zeroed() };
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    } == 0
    {
        return None;
    }
    let rect = info.monitorInfo.rcMonitor;
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return None;
    }
    let name = utf16_null_terminated(&info.szDevice)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("DISPLAY{}", index + 1));
    let scale_factor = monitor_scale_factor(monitor);
    Some(DisplayInfo {
        display_id: format!("windows:{name}"),
        name: Some(name),
        index: u32::try_from(index).unwrap_or(u32::MAX),
        primary: (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
        logical_rect: RectF {
            x: f64::from(rect.left),
            y: f64::from(rect.top),
            width: f64::from(width),
            height: f64::from(height),
            space: CoordinateSpace::DesktopLogical,
        },
        pixel_size: Some(PixelSize {
            width: width as u32,
            height: height as u32,
        }),
        scale_factor: Some(scale_factor),
        backend: "windows".to_string(),
    })
}

fn ensure_dpi_awareness() {
    static DPI_AWARENESS: Once = Once::new();
    DPI_AWARENESS.call_once(|| unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0 {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE);
        }
    });
}

fn monitor_scale_factor(monitor: HMONITOR) -> f64 {
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    let ok = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    if ok == 0 && dpi_x > 0 && dpi_y > 0 {
        f64::from(dpi_x.max(dpi_y)) / 96.0
    } else {
        1.0
    }
}

fn utf16_null_terminated(value: &[u16]) -> Option<String> {
    let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
    (len > 0).then(|| String::from_utf16_lossy(&value[..len]))
}

fn normalize_displays(mut displays: Vec<DisplayInfo>) -> Vec<DisplayInfo> {
    displays.retain(|display| {
        display.logical_rect.width > 0.0
            && display.logical_rect.height > 0.0
            && display.logical_rect.space == CoordinateSpace::DesktopLogical
    });
    displays.sort_by(|left, right| {
        left.index
            .cmp(&right.index)
            .then_with(|| left.display_id.cmp(&right.display_id))
    });
    if !displays.iter().any(|display| display.primary)
        && let Some(first) = displays.first_mut()
    {
        first.primary = true;
    }
    for (index, display) in displays.iter_mut().enumerate() {
        display.index = u32::try_from(index).unwrap_or(u32::MAX);
    }
    displays
}

fn assign_window_displays(windows: &mut [WindowInfo], displays: &[DisplayInfo]) {
    for window in windows {
        let mut intersections = displays
            .iter()
            .filter_map(|display| DisplayIntersection::from_bounds(display, &window.bounds))
            .collect::<Vec<_>>();
        intersections.sort_by(|left, right| {
            right
                .intersection_area
                .total_cmp(&left.intersection_area)
                .then_with(|| left.display.index.cmp(&right.display.index))
        });
        window.display = intersections
            .first()
            .map(|intersection| intersection.display.clone());
        window.display_intersections = intersections;
    }
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
        display: None,
        display_intersections: Vec::new(),
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

fn focus_window(hwnd: usize) -> Result<(), BackendError> {
    ensure_dpi_awareness();
    let ok = unsafe { SetForegroundWindow(hwnd as HWND) };
    if ok == 0 {
        return Err(win32_error(
            "SetForegroundWindow failed for Windows window target",
        ));
    }
    std::thread::sleep(std::time::Duration::from_millis(150));
    let foreground = unsafe { GetForegroundWindow() };
    if !std::ptr::eq(foreground, hwnd as HWND) {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("Windows focus verification failed after activating window 0x{hwnd:x}"),
        ));
    }
    Ok(())
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
        absolute_pointer_coords, assign_window_displays, capture_source_for_rect,
        desktop_action_point, desktop_drag_from_point, detect_blank_rgb_frame, parse_hwnd,
        primary_display, resolve_display_target, resolve_window_target, scroll_delta_y,
        uia_backend_ref_for_fallback, virtual_key, window_element,
    };
    use sky_cua_platform::diagnostics::BackendErrorCode;
    use sky_cua_platform::model::{
        ActionName, ActionRequest, CaptureBackendKind, CaptureInfo, CaptureScope, CoordinateSpace,
        DisplayInfo, DisplayTarget, ElementNode, InputBackendKind, PixelSize, RectF, WindowTarget,
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
    fn maps_secondary_negative_origin_stream_pixels_to_desktop_coordinates() {
        let request = action_request(
            serde_json::json!({ "x": 640.0, "y": 360.0 }),
            Some(capture_with_rect(-1280.0, 0.0, 1280.0, 720.0, 1.0)),
            None,
        );

        assert_eq!(desktop_action_point(&request).unwrap(), (-640.0, 360.0));
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
            display: None,
            display_intersections: Vec::new(),
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

    #[test]
    fn assigns_windows_to_largest_monitor_intersection() {
        let displays = vec![
            display("windows:\\\\.\\DISPLAY1", 0, true, 0.0, 0.0, 1920.0, 1080.0),
            display(
                "windows:\\\\.\\DISPLAY2",
                1,
                false,
                -1280.0,
                0.0,
                1280.0,
                720.0,
            ),
        ];
        let mut windows = vec![WindowInfo {
            hwnd: 0x20,
            title: "Tool".to_string(),
            pid: Some(22),
            executable: None,
            bounds: RectF {
                x: -900.0,
                y: 10.0,
                width: 1000.0,
                height: 500.0,
                space: CoordinateSpace::DesktopLogical,
            },
            display: None,
            display_intersections: Vec::new(),
            is_foreground: false,
        }];

        assign_window_displays(&mut windows, &displays);

        assert_eq!(
            windows[0]
                .display
                .as_ref()
                .map(|display| display.display_id.as_str()),
            Some("windows:\\\\.\\DISPLAY2")
        );
        assert_eq!(windows[0].display_intersections.len(), 2);
    }

    #[test]
    fn resolves_primary_and_explicit_display_targets() {
        let displays = vec![
            display("windows:\\\\.\\DISPLAY1", 0, true, 0.0, 0.0, 1920.0, 1080.0),
            display(
                "windows:\\\\.\\DISPLAY2",
                1,
                false,
                1920.0,
                0.0,
                1280.0,
                720.0,
            ),
        ];

        assert_eq!(
            primary_display(&displays).map(|display| display.display_id),
            Some("windows:\\\\.\\DISPLAY1".to_string())
        );
        let resolved = resolve_display_target(
            &displays,
            &DisplayTarget {
                display_id: None,
                display_name: None,
                display_index: Some(1),
            },
        )
        .unwrap();
        assert_eq!(resolved.display_id, "windows:\\\\.\\DISPLAY2");
    }

    #[test]
    fn display_target_fields_must_match_same_windows_display() {
        let displays = vec![
            display("windows:\\\\.\\DISPLAY1", 0, true, 0.0, 0.0, 1920.0, 1080.0),
            display(
                "windows:\\\\.\\DISPLAY2",
                1,
                false,
                1920.0,
                0.0,
                1280.0,
                720.0,
            ),
        ];

        let resolved = resolve_display_target(
            &displays,
            &DisplayTarget {
                display_id: Some("windows:\\\\.\\DISPLAY2".to_string()),
                display_name: Some("\\\\.\\DISPLAY2".to_string()),
                display_index: Some(1),
            },
        )
        .unwrap();
        assert_eq!(resolved.display_id, "windows:\\\\.\\DISPLAY2");

        let error = resolve_display_target(
            &displays,
            &DisplayTarget {
                display_id: Some("windows:missing".to_string()),
                display_name: None,
                display_index: Some(0),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
    }

    #[test]
    fn resolves_exact_window_id_before_broad_metadata() {
        let windows = vec![
            window_for_target(0x10, "Shared Title", Some(10), Some("shared.exe")),
            window_for_target(0x20, "Shared Title", Some(20), Some("other.exe")),
        ];

        let resolved = resolve_window_target(
            &windows,
            &WindowTarget {
                window_id: Some("hwnd:0x20".to_string()),
                title: Some("Shared Title".to_string()),
                wm_class: Some("shared".to_string()),
                ..WindowTarget::default()
            },
        )
        .unwrap();

        assert_eq!(resolved.hwnd, 0x20);
    }

    #[test]
    fn reports_ambiguous_windows_title_targets() {
        let windows = vec![
            window_for_target(0x10, "Shared Title", Some(10), None),
            window_for_target(0x20, "Shared Title", Some(20), None),
        ];

        let error = resolve_window_target(
            &windows,
            &WindowTarget {
                title: Some("Shared Title".to_string()),
                ..WindowTarget::default()
            },
        )
        .unwrap_err();

        assert!(error.message.contains("matched multiple Windows windows"));
    }

    #[test]
    fn resolves_windows_title_substrings_case_insensitively() {
        let windows = vec![window_for_target(
            0x10,
            "Untitled - Notepad",
            Some(10),
            None,
        )];

        let resolved = resolve_window_target(
            &windows,
            &WindowTarget {
                title: Some("notepad".to_string()),
                ..WindowTarget::default()
            },
        )
        .unwrap();

        assert_eq!(resolved.hwnd, 0x10);
    }

    #[test]
    fn display_capture_source_preserves_nonzero_origin() {
        let rect = RectF {
            x: -1280.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
            space: CoordinateSpace::DesktopLogical,
        };

        let source = capture_source_for_rect(&rect, None).unwrap();

        assert_eq!(source.source_x, -1280);
        assert_eq!(source.source_y, 0);
        assert_eq!(source.width, 1280);
        assert_eq!(source.height, 720);
        assert_eq!(source.logical_rect, Some(rect));
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
            capture_scope: CaptureScope::Window,
            display: None,
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
            source_logical_rect: None,
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

    fn display(
        id: &str,
        index: u32,
        primary: bool,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> DisplayInfo {
        DisplayInfo {
            display_id: id.to_string(),
            name: id.rsplit(':').next().map(ToOwned::to_owned),
            index,
            primary,
            logical_rect: RectF {
                x,
                y,
                width,
                height,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: Some(PixelSize {
                width: width as u32,
                height: height as u32,
            }),
            scale_factor: Some(1.0),
            backend: "windows".to_string(),
        }
    }

    fn window_for_target(
        hwnd: usize,
        title: &str,
        pid: Option<u32>,
        executable: Option<&str>,
    ) -> WindowInfo {
        WindowInfo {
            hwnd,
            title: title.to_string(),
            pid,
            executable: executable.map(ToOwned::to_owned),
            bounds: RectF {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
                space: CoordinateSpace::DesktopLogical,
            },
            display: None,
            display_intersections: Vec::new(),
            is_foreground: false,
        }
    }
}
