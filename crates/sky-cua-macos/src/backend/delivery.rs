//! Choosing how one physical input reaches the window its request names.
//!
//! Input that takes effect in an application that is not active is posted
//! straight to the window's process, so the user's frontmost application and
//! pointer are left alone. Everything else brings the window to the front
//! first and goes through the HID tap. The choice is reported as a
//! diagnostic on the action outcome.

use sky_cua_platform::diagnostics::BackendError;
use sky_cua_platform::model::{ActionRequest, DiagnosticEntry};

use super::actions::PhysicalInput;
use super::focus::{self, FocusNeed};
use crate::ax::AxRuntime;
use crate::input::Route;
use crate::types::MacWindow;
use crate::windows;

/// Diagnostic code: the input was posted to the target process and its
/// window was not brought to the front.
pub(super) const PROCESS_TARGETED_INPUT: &str = "ProcessTargetedInput";
/// Diagnostic code: the input could have been posted to the target process
/// but was not, so the window was brought to the front instead.
pub(super) const PROCESS_TARGETED_INPUT_UNAVAILABLE: &str = "ProcessTargetedInputUnavailable";

/// The route one input is posted on, with the diagnostics of choosing it.
#[derive(Debug)]
pub(super) struct Delivery {
    pub route: Route,
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// Prepares the request's window for `input` and picks the route.
///
/// A request that names no window posts to the current focus, and an
/// already-focused window is left alone; both use the HID tap. A vanished
/// window fails the action so the input never lands on whatever window is in
/// front instead, and so does a window that moved or was resized since the
/// snapshot when the input is posted at a snapshot point. An unfocused window
/// receives the input in its process when [`process_route`] allows it;
/// otherwise it is brought to the front, and the action fails if that does
/// not happen.
pub(super) async fn prepare(
    ax: &AxRuntime,
    request: &ActionRequest,
    input: &PhysicalInput,
) -> Result<Delivery, BackendError> {
    let hid = |diagnostics| Delivery {
        route: Route::HidTap,
        diagnostics,
    };
    let Some(window_id) = focus::request_window_id(request) else {
        return Ok(hid(Vec::new()));
    };
    let windows_front_to_back = windows::enumerate_windows();
    let window = match focus::physical_input_focus_need(
        window_id,
        &windows_front_to_back,
        focus::captured_window_frame(request).filter(|_| input.uses_snapshot_point()),
    )? {
        FocusNeed::AlreadyFocused => return Ok(hid(Vec::new())),
        FocusNeed::Activate(window) => window,
    };

    let mut unavailable = None;
    if input.reaches_inactive_application()? {
        let app_windows = focus::app_windows(&windows_front_to_back, window.pid);
        let receives_keys = ax.window_receives_keys(window, &app_windows).await;
        match process_route(window, receives_keys) {
            Ok(delivery) => return Ok(delivery),
            Err(diagnostic) => unavailable = Some(diagnostic),
        }
    }
    let attempt = focus::focus_window(ax, window, &windows_front_to_back).await;
    match focus::physical_input_focus_result(window_id, attempt) {
        Ok(focus_diagnostics) => Ok(hid(unavailable
            .into_iter()
            .chain(focus_diagnostics)
            .collect())),
        Err(error) => Err(with_process_route_reason(error, unavailable.as_ref())),
    }
}

/// Adds why the process route was not taken to the error of a failed
/// fallback, so the reason is not lost with the diagnostics.
fn with_process_route_reason(
    mut error: BackendError,
    unavailable: Option<&DiagnosticEntry>,
) -> BackendError {
    if let Some(reason) = unavailable.and_then(|diagnostic| diagnostic.details.as_deref()) {
        error.message.push_str(&format!(
            "; posting the input to the window's process was not possible either: {reason}"
        ));
    }
    error
}

/// The process route for keyboard input to an unfocused `window`, or the
/// diagnostic for not taking it.
///
/// A process hands posted key events to its own key window, whichever window
/// the request named. `receives_keys` is whether Accessibility confirmed
/// `window` is that window; anything else, including a failed check, would
/// type into a different window of the same application.
fn process_route(
    window: &MacWindow,
    receives_keys: Result<bool, BackendError>,
) -> Result<Delivery, DiagnosticEntry> {
    let unavailable = |details: String| DiagnosticEntry {
        code: PROCESS_TARGETED_INPUT_UNAVAILABLE.to_string(),
        message: format!(
            "Keyboard input for macOS window {} could not be posted to its process (pid {}), so the window is brought to the front",
            window.window_id, window.pid
        ),
        details: Some(details),
    };
    match receives_keys {
        Ok(true) => Ok(Delivery {
            route: Route::Process(window.pid),
            diagnostics: vec![DiagnosticEntry {
                code: PROCESS_TARGETED_INPUT.to_string(),
                message: format!(
                    "Keyboard input was posted to the process of macOS window {} (pid {}) without bringing the window to the front",
                    window.window_id, window.pid
                ),
                details: None,
            }],
        }),
        Ok(false) => Err(unavailable(
            "the window is not the one its application sends key events to".to_string(),
        )),
        Err(error) => Err(unavailable(format!("{}: {}", error.code, error.message))),
    }
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::diagnostics::BackendErrorCode;

    use super::*;
    use crate::coords::desktop_rect;

    fn window() -> MacWindow {
        MacWindow {
            window_id: 30,
            pid: 7,
            owner_name: "App".to_string(),
            bundle_id: None,
            executable: None,
            title: None,
            bounds: desktop_rect(0.0, 0.0, 10.0, 10.0),
            layer: 0,
            on_screen: true,
            focused: false,
            z_index: 1,
        }
    }

    #[test]
    fn the_key_window_of_an_unfocused_application_takes_the_process_route() {
        let delivery = process_route(&window(), Ok(true)).unwrap();

        assert_eq!(delivery.route, Route::Process(7));
        assert_eq!(delivery.diagnostics.len(), 1);
        assert_eq!(delivery.diagnostics[0].code, PROCESS_TARGETED_INPUT);
        assert!(delivery.diagnostics[0].message.contains("window 30"));
    }

    #[test]
    fn another_window_of_the_application_is_brought_to_the_front_instead() {
        let diagnostic = process_route(&window(), Ok(false)).unwrap_err();

        assert_eq!(diagnostic.code, PROCESS_TARGETED_INPUT_UNAVAILABLE);
        assert!(diagnostic.message.contains("pid 7"));
        assert!(diagnostic.details.unwrap().contains("not the one"));
    }

    #[test]
    fn a_failed_fallback_keeps_the_reason_the_process_route_was_refused() {
        let focus_error = || {
            BackendError::new(
                BackendErrorCode::ActionRequiresPhysicalInput,
                "macOS window 30 could not be brought to the front",
            )
        };
        let unavailable = process_route(&window(), Ok(false)).unwrap_err();

        let error = with_process_route_reason(focus_error(), Some(&unavailable));
        assert_eq!(
            error.code,
            BackendErrorCode::ActionRequiresPhysicalInput.as_str()
        );
        assert!(error.message.contains("could not be brought to the front"));
        assert!(error.message.contains("not the one its application"));

        // Pointer input never tries the process route, so nothing is added.
        let untouched = with_process_route_reason(focus_error(), None);
        assert_eq!(untouched.message, focus_error().message);
    }

    #[test]
    fn an_unconfirmed_key_window_is_brought_to_the_front_instead() {
        let diagnostic = process_route(
            &window(),
            Err(BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                "no Accessibility window matches",
            )),
        )
        .unwrap_err();

        assert_eq!(diagnostic.code, PROCESS_TARGETED_INPUT_UNAVAILABLE);
        let details = diagnostic.details.unwrap();
        assert!(details.contains("AccessibilityCoverageLimited"));
        assert!(details.contains("no Accessibility window matches"));
    }
}
