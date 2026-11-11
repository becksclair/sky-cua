//! Bringing a window forward before physical input or a targeted capture.
//!
//! Activation is application-level (`windows::activate_application`), and
//! raising the specific window is an AX call (`AxRuntime::raise_window`).
//! macOS applies activation asynchronously, so the result is verified by
//! polling the CoreGraphics window list for a short, bounded settle period.

use std::time::{Duration, Instant};

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{ActionRequest, DiagnosticEntry};

use crate::ax::AxRuntime;
use crate::types::MacWindow;
use crate::windows;

/// Longest wait for the window server to report the window as focused.
const FOCUS_SETTLE_TIMEOUT: Duration = Duration::from_millis(300);
const FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(15);

/// Diagnostic code for a focus request whose effect could not be confirmed.
pub(super) const WINDOW_FOCUS_UNVERIFIED: &str = "WindowFocusUnverified";

/// What happened while bringing one window forward.
#[derive(Debug)]
pub(super) struct FocusReport {
    pub window_id: u32,
    /// The window list reported the window as focused before the deadline.
    pub focused: bool,
    /// `NSRunningApplication` activation was declined; macOS activation is
    /// cooperative, so the AX raise still ran and may have succeeded.
    pub activation_error: Option<BackendError>,
    /// `AXRaise` failed; the application may still have been activated.
    pub raise_error: Option<BackendError>,
}

impl FocusReport {
    pub fn diagnostics(&self) -> Vec<DiagnosticEntry> {
        let mut diagnostics = Vec::new();
        if let Some(error) = self.activation_error.as_ref() {
            diagnostics.push(DiagnosticEntry {
                code: error.code.to_string(),
                message: format!(
                    "Activating the application of macOS window {} was declined; the window was raised through Accessibility",
                    self.window_id
                ),
                details: Some(error.message.clone()),
            });
        }
        if let Some(error) = self.raise_error.as_ref() {
            diagnostics.push(DiagnosticEntry {
                code: error.code.to_string(),
                message: format!(
                    "Raising macOS window {} through Accessibility failed; only its application was activated",
                    self.window_id
                ),
                details: Some(error.message.clone()),
            });
        }
        if !self.focused {
            diagnostics.push(DiagnosticEntry {
                code: WINDOW_FOCUS_UNVERIFIED.to_string(),
                message: format!(
                    "macOS window {} was not reported as focused within {} ms of activation",
                    self.window_id,
                    FOCUS_SETTLE_TIMEOUT.as_millis()
                ),
                details: None,
            });
        }
        diagnostics
    }
}

/// Activates `window`'s application, raises the window, and waits briefly for
/// the window server to report it focused. Activation is cooperative on
/// macOS and may be declined, so the AX raise runs either way. The call fails
/// only when activation failed and the window still did not come forward;
/// every other problem is reported in the result.
pub(super) async fn focus_window(
    ax: &AxRuntime,
    window: &MacWindow,
    windows_front_to_back: &[MacWindow],
) -> Result<FocusReport, BackendError> {
    let activation_error = windows::activate_application(window).err();
    let app_windows = app_windows(windows_front_to_back, window.pid);
    let raise_error = ax.raise_window(window, &app_windows).await.err();
    let focused = wait_until_focused(window.window_id).await;
    if !focused && let Some(error) = activation_error {
        return Err(error);
    }
    Ok(FocusReport {
        window_id: window.window_id,
        focused,
        activation_error,
        raise_error,
    })
}

/// Brings the window named by the request's snapshot forward before physical
/// input. A request that names no window posts to the current focus; an
/// already-focused window is left alone. A vanished window, or one that
/// could not be brought to the front, fails the action so the input never
/// lands on whatever window is in front instead. Returns diagnostics for the
/// focus outcome.
pub(super) async fn focus_request_window(
    ax: &AxRuntime,
    request: &ActionRequest,
) -> Result<Vec<DiagnosticEntry>, BackendError> {
    let Some(window_id) = request_window_id(request) else {
        return Ok(Vec::new());
    };
    let windows_front_to_back = windows::enumerate_windows();
    match physical_input_focus_need(window_id, &windows_front_to_back)? {
        FocusNeed::AlreadyFocused => Ok(Vec::new()),
        FocusNeed::Activate(window) => {
            let attempt = focus_window(ax, window, &windows_front_to_back).await;
            physical_input_focus_result(window_id, attempt)
        }
    }
}

/// What the request's window needs before physical input may be posted.
#[derive(Debug)]
enum FocusNeed<'a> {
    AlreadyFocused,
    Activate(&'a MacWindow),
}

/// Whether the requested window still exists and must be brought forward.
/// A window missing from the list is a stale snapshot.
fn physical_input_focus_need(
    window_id: u32,
    windows_front_to_back: &[MacWindow],
) -> Result<FocusNeed<'_>, BackendError> {
    let window = windows_front_to_back
        .iter()
        .find(|window| window.window_id == window_id)
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::SnapshotStale,
                format!(
                    "macOS window {window_id} targeted by this action is gone; observe again with get_app_state before acting"
                ),
            )
        })?;
    Ok(if window.focused {
        FocusNeed::AlreadyFocused
    } else {
        FocusNeed::Activate(window)
    })
}

/// Gates physical input on the focus attempt: input is posted only when the
/// window server reports the requested window focused.
fn physical_input_focus_result(
    window_id: u32,
    attempt: Result<FocusReport, BackendError>,
) -> Result<Vec<DiagnosticEntry>, BackendError> {
    let activation_error = match attempt {
        Ok(report) if report.focused => return Ok(report.diagnostics()),
        Ok(report) => report.activation_error.map(|error| error.message),
        Err(error) => Some(error.message),
    };
    let mut message = format!(
        "macOS window {window_id} could not be brought to the front within {} ms, so physical input was not sent",
        FOCUS_SETTLE_TIMEOUT.as_millis()
    );
    if let Some(activation_error) = activation_error {
        message.push_str(&format!("; activation error: {activation_error}"));
    }
    Err(BackendError::new(
        BackendErrorCode::ActionRequiresPhysicalInput,
        message,
    ))
}

/// Every window of `pid` in front-to-back order.
pub(super) fn app_windows(windows_front_to_back: &[MacWindow], pid: i32) -> Vec<MacWindow> {
    let mut app_windows: Vec<MacWindow> = windows_front_to_back
        .iter()
        .filter(|window| window.pid == pid)
        .cloned()
        .collect();
    app_windows.sort_by_key(|window| window.z_index);
    app_windows
}

/// The `CGWindowID` the request's snapshot targeted, from its focused app's
/// `window_handle`.
fn request_window_id(request: &ActionRequest) -> Option<u32> {
    request
        .resolved_focused_app
        .as_ref()
        .and_then(|app| app.window_handle.as_deref())
        .and_then(parse_window_handle)
}

/// Parses a `CGWindowID` written in decimal (the contract `window_id` form)
/// or as `0x`-prefixed hexadecimal.
fn parse_window_handle(value: &str) -> Option<u32> {
    let value = value.trim();
    match value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => value.parse().ok(),
    }
}

async fn wait_until_focused(window_id: u32) -> bool {
    let deadline = Instant::now() + FOCUS_SETTLE_TIMEOUT;
    loop {
        if windows::enumerate_windows()
            .iter()
            .any(|window| window.window_id == window_id && window.focused)
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(FOCUS_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::desktop_rect;

    #[test]
    fn parses_decimal_and_hex_window_handles() {
        assert_eq!(parse_window_handle("1234"), Some(1234));
        assert_eq!(parse_window_handle(" 0x4d2 "), Some(1234));
        assert_eq!(parse_window_handle("hwnd:0x10"), None);
        assert_eq!(parse_window_handle(""), None);
    }

    #[test]
    fn app_windows_keep_one_process_in_front_to_back_order() {
        let windows = vec![
            window(30, 7, 2),
            window(10, 7, 0),
            window(20, 8, 1),
            window(40, 7, 3),
        ];

        let ids: Vec<u32> = app_windows(&windows, 7)
            .iter()
            .map(|window| window.window_id)
            .collect();

        assert_eq!(ids, vec![10, 30, 40]);
    }

    #[test]
    fn focus_report_diagnostics_are_structured() {
        let report = FocusReport {
            window_id: 5,
            focused: false,
            activation_error: None,
            raise_error: Some(BackendError::new(
                sky_cua_platform::diagnostics::BackendErrorCode::PermissionRequired,
                "grant Accessibility",
            )),
        };

        let codes: Vec<String> = report
            .diagnostics()
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect();

        assert_eq!(codes, vec!["PermissionRequired", WINDOW_FOCUS_UNVERIFIED]);
    }

    #[test]
    fn request_naming_no_window_posts_to_current_focus() {
        let mut request = ActionRequest {
            action: sky_cua_platform::model::ActionName::TypeText,
            appshot_id: None,
            snapshot_id: None,
            element_index: None,
            arguments: serde_json::Value::Null,
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: None,
        };
        assert_eq!(request_window_id(&request), None);

        request.resolved_focused_app = Some(sky_cua_platform::model::FocusedApp {
            app_id: "com.apple.TextEdit".to_string(),
            name: "TextEdit".to_string(),
            pid: Some(7),
            desktop_file_id: None,
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: None,
            window_title: None,
            display: None,
        });
        assert_eq!(request_window_id(&request), None);

        if let Some(app) = request.resolved_focused_app.as_mut() {
            app.window_handle = Some("30".to_string());
        }
        assert_eq!(request_window_id(&request), Some(30));
    }

    #[test]
    fn vanished_request_window_is_a_stale_snapshot() {
        let windows = vec![window(10, 7, 0)];

        let error = physical_input_focus_need(30, &windows).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::SnapshotStale.as_str());
        assert!(error.message.contains("30"));
        assert!(error.message.contains("observe again"));
    }

    #[test]
    fn focused_request_window_is_not_reactivated() {
        let mut focused = window(30, 7, 1);
        focused.focused = true;
        let windows = vec![window(10, 8, 0), focused];

        assert!(matches!(
            physical_input_focus_need(30, &windows),
            Ok(FocusNeed::AlreadyFocused)
        ));
        assert!(matches!(
            physical_input_focus_need(10, &windows),
            Ok(FocusNeed::Activate(window)) if window.window_id == 10
        ));
    }

    #[test]
    fn request_window_focused_by_the_attempt_allows_input() {
        let report = FocusReport {
            window_id: 30,
            focused: true,
            activation_error: Some(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "declined",
            )),
            raise_error: None,
        };

        let diagnostics = physical_input_focus_result(30, Ok(report)).unwrap();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].code,
            BackendErrorCode::ActionUnsupportedForEnvironment.as_str()
        );
    }

    #[test]
    fn request_window_left_unfocused_blocks_input() {
        let declined = physical_input_focus_result(
            30,
            Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "macOS declined to activate application pid 7 for window 30",
            )),
        )
        .unwrap_err();
        assert_eq!(
            declined.code,
            BackendErrorCode::ActionRequiresPhysicalInput.as_str()
        );
        assert!(declined.message.contains("physical input was not sent"));
        assert!(declined.message.contains("macOS declined to activate"));

        let unverified = physical_input_focus_result(
            30,
            Ok(FocusReport {
                window_id: 30,
                focused: false,
                activation_error: None,
                raise_error: None,
            }),
        )
        .unwrap_err();
        assert_eq!(
            unverified.code,
            BackendErrorCode::ActionRequiresPhysicalInput.as_str()
        );
        assert!(!unverified.message.contains("activation error"));
    }

    fn window(window_id: u32, pid: i32, z_index: usize) -> MacWindow {
        MacWindow {
            window_id,
            pid,
            owner_name: "App".to_string(),
            bundle_id: None,
            executable: None,
            title: None,
            bounds: desktop_rect(0.0, 0.0, 10.0, 10.0),
            layer: 0,
            on_screen: true,
            focused: false,
            z_index,
        }
    }
}
