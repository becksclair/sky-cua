//! Choosing how one physical input reaches the window its request names.
//!
//! Input that takes effect in an application that is not active is posted
//! straight to the window's process, so the user's frontmost application and
//! pointer are left alone: key events to the process, and left clicks
//! addressed to the window. Everything else brings the window to the front
//! first and goes through the HID tap. The choice is reported as a
//! diagnostic on the action outcome.

use sky_cua_platform::diagnostics::BackendError;
use sky_cua_platform::model::{ActionRequest, DiagnosticEntry};

use super::actions::{Background, PhysicalInput};
use super::focus::{self, FocusNeed};
use crate::ax::{self, AxRuntime};
use crate::input::window_events::{self, Activation, WindowAddress};
use crate::types::MacWindow;
use crate::windows;

/// Diagnostic code: the input was posted to the target process and its
/// window was not brought to the front.
pub(super) const PROCESS_TARGETED_INPUT: &str = "ProcessTargetedInput";
/// Diagnostic code: the input was not posted to the target process, so the
/// window was brought to the front instead; `details` says why.
pub(super) const PROCESS_TARGETED_INPUT_UNAVAILABLE: &str = "ProcessTargetedInputUnavailable";

/// Where one input is posted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DeliveryRoute {
    /// The HID tap: whatever is under the pointer or holds key focus.
    Hid,
    /// Key events to the process with this pid.
    ProcessKeys(i32),
    /// Left clicks addressed to one window of a process.
    WindowClick {
        address: WindowAddress,
        activation: Activation,
    },
}

/// The route one input is posted on, with the diagnostics of choosing it.
#[derive(Debug)]
pub(super) struct Delivery {
    pub route: DeliveryRoute,
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// Prepares the request's window for `input` and picks the route.
///
/// A request that names no window posts to the current focus, and an
/// already-focused window is left alone; both use the HID tap. A vanished
/// window fails the action so the input never lands on whatever window is in
/// front instead, and so does a window that moved or was resized since the
/// snapshot when the input is posted at a snapshot point. An unfocused window
/// receives the input in its process when [`keys_route`] or [`click_route`]
/// allows it; otherwise it is brought to the front, which is reported with a
/// [`PROCESS_TARGETED_INPUT_UNAVAILABLE`] diagnostic, and the action fails if
/// that does not happen.
pub(super) async fn prepare(
    ax: &AxRuntime,
    request: &ActionRequest,
    input: &PhysicalInput,
) -> Result<Delivery, BackendError> {
    let hid = |diagnostics| Delivery {
        route: DeliveryRoute::Hid,
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

    let app_windows = || focus::app_windows(&windows_front_to_back, window.pid);
    let background = match input.background()? {
        Background::Keys => {
            let receives_keys = ax.window_receives_keys(window, &app_windows()).await;
            keys_route(window, receives_keys)
        }
        Background::LeftClick if !window_events::is_available() => Err(unavailable(
            window,
            "Pointer",
            "CGEventSetWindowLocation is not available on this macOS version".to_string(),
        )),
        // A Chromium engine acts on window-addressed clicks while inactive,
        // and loses the first one when told it was activated.
        Background::LeftClick if ax::hosts_chromium_engine(window) => {
            Ok(click_delivery(window, Activation::None))
        }
        // The application is active, so the synthetic deactivation that
        // ends a primed click would leave the user's own application
        // believing it is not.
        Background::LeftClick if app_is_active(&windows_front_to_back, window.pid) => {
            Err(unavailable(
                window,
                "Pointer",
                "the window's application is the active one".to_string(),
            ))
        }
        Background::LeftClick => {
            let priming_point = ax.window_priming_point(window, &app_windows()).await;
            click_route(window, priming_point)
        }
        Background::None => Err(unavailable(window, "Input", NO_PROCESS_ROUTE.to_string())),
    };
    let unavailable = match background {
        Ok(delivery) => return Ok(delivery),
        Err(diagnostic) => diagnostic,
    };
    let attempt = focus::focus_window(ax, window, &windows_front_to_back).await;
    match focus::physical_input_focus_result(window_id, attempt) {
        Ok(focus_diagnostics) => Ok(hid(std::iter::once(unavailable)
            .chain(focus_diagnostics)
            .collect())),
        Err(error) => Err(with_process_route_reason(error, &unavailable)),
    }
}

/// Why input other than keys and left clicks is not posted to the process.
const NO_PROCESS_ROUTE: &str = "only keyboard input without a Command or Control shortcut and left clicks can be posted to an application that is not active";

/// Whether `pid` is the active application, whose front window the window
/// list marks as focused.
fn app_is_active(windows_front_to_back: &[MacWindow], pid: i32) -> bool {
    windows_front_to_back
        .iter()
        .any(|window| window.pid == pid && window.focused)
}

/// Adds why the process route was not taken to the error of a failed
/// fallback, so the reason is not lost with the diagnostics.
fn with_process_route_reason(
    mut error: BackendError,
    unavailable: &DiagnosticEntry,
) -> BackendError {
    if let Some(reason) = unavailable.details.as_deref() {
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
fn keys_route(
    window: &MacWindow,
    receives_keys: Result<bool, BackendError>,
) -> Result<Delivery, DiagnosticEntry> {
    match receives_keys {
        Ok(true) => Ok(Delivery {
            route: DeliveryRoute::ProcessKeys(window.pid),
            diagnostics: vec![delivered(window, "Keyboard")],
        }),
        Ok(false) => Err(unavailable(
            window,
            "Keyboard",
            "the window is not the one its application sends key events to".to_string(),
        )),
        Err(error) => Err(unavailable(window, "Keyboard", error_details(&error))),
    }
}

/// The window-addressed route for a left click on an unfocused AppKit
/// `window`, or the diagnostic for not taking it.
///
/// The application acts on the click only while it believes it is active and
/// the window is key. `priming_point` is the point Accessibility offers for
/// making the window key with a click that does nothing else; without one
/// the first click would be spent on that, or would press whatever it hit.
fn click_route(
    window: &MacWindow,
    priming_point: Result<Option<(f64, f64)>, BackendError>,
) -> Result<Delivery, DiagnosticEntry> {
    match priming_point {
        Ok(Some(at)) => Ok(click_delivery(window, Activation::Primed { at })),
        Ok(None) => Err(unavailable(
            window,
            "Pointer",
            "the window offers no point where a click only makes it the key window".to_string(),
        )),
        Err(error) => Err(unavailable(window, "Pointer", error_details(&error))),
    }
}

fn click_delivery(window: &MacWindow, activation: Activation) -> Delivery {
    Delivery {
        route: DeliveryRoute::WindowClick {
            address: WindowAddress {
                pid: window.pid,
                window_id: window.window_id,
                origin: (window.bounds.x, window.bounds.y),
            },
            activation,
        },
        diagnostics: vec![delivered(window, "Pointer")],
    }
}

fn delivered(window: &MacWindow, kind: &str) -> DiagnosticEntry {
    DiagnosticEntry {
        code: PROCESS_TARGETED_INPUT.to_string(),
        message: format!(
            "{kind} input was posted to the process of macOS window {} (pid {}) without bringing the window to the front",
            window.window_id, window.pid
        ),
        details: None,
    }
}

fn unavailable(window: &MacWindow, kind: &str, details: String) -> DiagnosticEntry {
    DiagnosticEntry {
        code: PROCESS_TARGETED_INPUT_UNAVAILABLE.to_string(),
        message: format!(
            "{kind} input for macOS window {} could not be posted to its process (pid {}), so the window is brought to the front",
            window.window_id, window.pid
        ),
        details: Some(details),
    }
}

fn error_details(error: &BackendError) -> String {
    format!("{}: {}", error.code, error.message)
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
            bounds: desktop_rect(100.0, 50.0, 400.0, 300.0),
            layer: 0,
            on_screen: true,
            focused: false,
            z_index: 1,
        }
    }

    fn ax_error() -> BackendError {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            "no Accessibility window matches",
        )
    }

    #[test]
    fn the_key_window_of_an_unfocused_application_takes_the_process_route() {
        let delivery = keys_route(&window(), Ok(true)).unwrap();

        assert_eq!(delivery.route, DeliveryRoute::ProcessKeys(7));
        assert_eq!(delivery.diagnostics.len(), 1);
        assert_eq!(delivery.diagnostics[0].code, PROCESS_TARGETED_INPUT);
        assert!(delivery.diagnostics[0].message.contains("window 30"));
    }

    #[test]
    fn another_window_of_the_application_is_brought_to_the_front_instead() {
        let diagnostic = keys_route(&window(), Ok(false)).unwrap_err();

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
        let unavailable = keys_route(&window(), Ok(false)).unwrap_err();

        let error = with_process_route_reason(focus_error(), &unavailable);
        assert_eq!(
            error.code,
            BackendErrorCode::ActionRequiresPhysicalInput.as_str()
        );
        assert!(error.message.contains("could not be brought to the front"));
        assert!(error.message.contains("not the one its application"));
    }

    #[test]
    fn an_application_is_active_when_one_of_its_windows_is_focused() {
        let back = window();
        let front = MacWindow {
            window_id: 31,
            focused: true,
            z_index: 0,
            ..window()
        };
        let other = MacWindow {
            pid: 8,
            ..front.clone()
        };

        assert!(app_is_active(&[front, back.clone()], 7));
        assert!(!app_is_active(&[other, back], 7));
    }

    #[test]
    fn input_without_a_process_route_reports_the_foreground_fallback() {
        let diagnostic = unavailable(&window(), "Input", NO_PROCESS_ROUTE.to_string());

        assert_eq!(diagnostic.code, PROCESS_TARGETED_INPUT_UNAVAILABLE);
        assert!(diagnostic.message.contains("brought to the front"));
        assert!(diagnostic.details.unwrap().contains("Command or Control"));
    }

    #[test]
    fn an_unconfirmed_key_window_is_brought_to_the_front_instead() {
        let diagnostic = keys_route(&window(), Err(ax_error())).unwrap_err();

        assert_eq!(diagnostic.code, PROCESS_TARGETED_INPUT_UNAVAILABLE);
        let details = diagnostic.details.unwrap();
        assert!(details.contains("AccessibilityCoverageLimited"));
        assert!(details.contains("no Accessibility window matches"));
    }

    #[test]
    fn a_window_with_a_priming_point_takes_a_primed_click_addressed_to_it() {
        let delivery = click_route(&window(), Ok(Some((130.0, 66.0)))).unwrap();

        assert_eq!(
            delivery.route,
            DeliveryRoute::WindowClick {
                address: WindowAddress {
                    pid: 7,
                    window_id: 30,
                    origin: (100.0, 50.0),
                },
                activation: Activation::Primed { at: (130.0, 66.0) },
            }
        );
        assert_eq!(delivery.diagnostics[0].code, PROCESS_TARGETED_INPUT);
        assert!(delivery.diagnostics[0].message.starts_with("Pointer input"));
    }

    #[test]
    fn a_window_without_a_priming_point_is_brought_to_the_front_instead() {
        let none = click_route(&window(), Ok(None)).unwrap_err();
        assert_eq!(none.code, PROCESS_TARGETED_INPUT_UNAVAILABLE);
        assert!(none.message.starts_with("Pointer input"));
        assert!(none.details.unwrap().contains("no point"));

        let failed = click_route(&window(), Err(ax_error())).unwrap_err();
        assert_eq!(failed.code, PROCESS_TARGETED_INPUT_UNAVAILABLE);
        assert!(
            failed
                .details
                .unwrap()
                .contains("AccessibilityCoverageLimited")
        );
    }
}
