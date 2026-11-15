//! Left clicks addressed to one window of an application that is not active.
//!
//! A mouse event posted to a process is ignored unless it names one of that
//! process's windows: the window number, the two "window under the pointer"
//! fields, and the event's location in that window's own coordinates. The
//! last of these has no public setter, so `CGEventSetWindowLocation` is
//! resolved at run time and the whole route reports itself unavailable when
//! the symbol is missing.
//!
//! An AppKit application also has to believe it is active before a view
//! acts on a click, and its window has to be key. [`Activation::Primed`]
//! sends the application an "activated" event, clicks a point of the window
//! that does nothing, which makes the window key, delivers the real click,
//! and then sends "deactivated". None of this changes the active application
//! or the window order as the window server sees them. See
//! `docs/research/2026-11-macos-per-window-targeting.md` for the evidence.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread::sleep;
use std::time::Duration;

use objc2::rc::Retained;
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags};
use objc2_foundation::NSPoint;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

use super::{CLICK_HOLD, MULTI_CLICK_GAP};
use crate::permissions::{self, Permission};
use crate::windows;

/// `NSEventSubtypeApplicationActivated` and `...Deactivated`.
const APPLICATION_ACTIVATED: i16 = 1;
const APPLICATION_DEACTIVATED: i16 = 2;
/// Modifier flags the reference activation event carries with a window.
const ACTIVATION_FLAGS: usize = 0xC0000;
/// `NX_SUBTYPE_MOUSE_TOUCH`, the mouse subtype the reference events carry.
const MOUSE_SUBTYPE: i64 = 3;
/// Wait after the activation event before the first mouse event.
const ACTIVATION_SETTLE: Duration = Duration::from_millis(80);
/// Wait after the priming click, so the window is key before the real click.
const PRIMING_SETTLE: Duration = Duration::from_millis(120);
/// Wait after the last click before the application is told it is inactive.
const DEACTIVATION_DELAY: Duration = Duration::from_millis(100);

/// The window a click is addressed to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowAddress {
    pub pid: i32,
    /// `CGWindowID`.
    pub window_id: u32,
    /// The window's top-left corner in desktop points.
    pub origin: (f64, f64),
}

/// What a window needs before a click posted to its process takes effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Activation {
    /// The application acts on window-addressed clicks as it is.
    None,
    /// The application must believe it is active and the window must be key.
    /// `at` is a point of the window where a click does nothing.
    Primed { at: (f64, f64) },
}

type SetWindowLocation = unsafe extern "C" fn(*const CGEvent, CGPoint);

fn set_window_location() -> Option<SetWindowLocation> {
    static SYMBOL: OnceLock<Option<SetWindowLocation>> = OnceLock::new();
    *SYMBOL.get_or_init(|| {
        // SAFETY: a NUL-terminated name looked up in the images already
        // loaded; a null result is handled.
        let symbol =
            unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"CGEventSetWindowLocation".as_ptr()) };
        // SAFETY: the symbol is `void CGEventSetWindowLocation(CGEventRef,
        // CGPoint)` in every CoreGraphics that exports it.
        (!symbol.is_null())
            .then(|| unsafe { std::mem::transmute::<*mut c_void, SetWindowLocation>(symbol) })
    })
}

/// Whether window-addressed events can be built on this system.
#[must_use]
pub fn is_available() -> bool {
    set_window_location().is_some()
}

/// Left-clicks `count` times at a desktop point of `address`'s window
/// without making its application active. Blocks.
pub fn click(
    address: &WindowAddress,
    activation: Activation,
    at: (f64, f64),
    count: u32,
) -> Result<(), BackendError> {
    if count == 0 {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "click count must be at least 1",
        ));
    }
    if !permissions::status().post_event {
        return Err(permissions::missing_permission_error(Permission::PostEvent));
    }
    let set_location = set_window_location().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "CGEventSetWindowLocation is not available on this macOS version",
        )
    })?;
    // The activation sequence is built before anything is posted, and each
    // click's pair before that click, so a failure never leaves the
    // application activated or a button held.
    objc2::rc::autoreleasepool(|_| {
        let mouse = |event_type, point, click_state| {
            mouse_event(address, event_type, point, click_state, set_location)
        };
        let click_pair = |point, click_state| -> Result<_, BackendError> {
            Ok((
                mouse(NSEventType::LeftMouseDown, point, click_state)?,
                mouse(NSEventType::LeftMouseUp, point, click_state)?,
            ))
        };
        let primed = match activation {
            Activation::None => None,
            Activation::Primed { at: inert } => Some((
                application_event(APPLICATION_ACTIVATED, address.window_id)?,
                click_pair(inert, 1)?,
                application_event(APPLICATION_DEACTIVATED, 0)?,
            )),
        };

        let pid = address.pid;
        let press = |(down, up)| {
            post(down, pid);
            sleep(CLICK_HOLD);
            post(up, pid);
        };
        let deactivate = primed.map(|(activate, priming, deactivate)| {
            post(activate, pid);
            sleep(ACTIVATION_SETTLE);
            press(priming);
            sleep(PRIMING_SETTLE);
            deactivate
        });
        let clicked = (1..=i64::from(count)).try_for_each(|click_state| {
            if click_state > 1 {
                sleep(MULTI_CLICK_GAP);
            }
            press(click_pair(at, click_state)?);
            Ok(())
        });
        if let Some(deactivate) = deactivate {
            sleep(DEACTIVATION_DELAY);
            // An application the user brought forward meanwhile is really
            // active; telling it otherwise would take its key focus away.
            if windows::front_process_pid() != Some(pid) {
                post(deactivate, pid);
            }
        }
        clicked
    })
}

fn post(event: Retained<CGEvent>, pid: i32) {
    // SAFETY: plain call with a valid clock id.
    let now = unsafe { clock_gettime_nsec_np(libc::CLOCK_UPTIME_RAW) };
    CGEvent::set_timestamp(Some(&event), now);
    CGEvent::post_to_pid(pid, Some(&event));
}

unsafe extern "C" {
    /// Nanoseconds on `clock_id`, the unit of a CGEvent timestamp.
    fn clock_gettime_nsec_np(clock_id: libc::clockid_t) -> u64;
}

fn next_event_number() -> isize {
    static NEXT: AtomicIsize = AtomicIsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The AppKit-defined event an application receives when it becomes active
/// (naming the window that takes key focus) or stops being active.
fn application_event(subtype: i16, window_id: u32) -> Result<Retained<CGEvent>, BackendError> {
    let flags = if window_id == 0 { 0 } else { ACTIVATION_FLAGS };
    NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::AppKitDefined,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags(flags),
        0.0,
        window_id as isize,
        None,
        subtype,
        0,
        0,
    )
    .and_then(|event| event.CGEvent())
    .ok_or_else(|| null_event("NSEvent otherEventWithType"))
}

fn mouse_event(
    address: &WindowAddress,
    event_type: NSEventType,
    (x, y): (f64, f64),
    click_state: i64,
    set_location: SetWindowLocation,
) -> Result<Retained<CGEvent>, BackendError> {
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        event_type,
        NSPoint::new(x, y),
        NSEventModifierFlags::empty(),
        0.0,
        address.window_id as isize,
        None,
        next_event_number(),
        click_state as isize,
        1.0,
    )
    .and_then(|event| event.CGEvent())
    .ok_or_else(|| null_event("NSEvent mouseEventWithType"))?;
    let window_id = i64::from(address.window_id);
    let set = |field, value| CGEvent::set_integer_value_field(Some(&event), field, value);
    CGEvent::set_flags(Some(&event), CGEventFlags::empty());
    CGEvent::set_location(Some(&event), CGPoint { x, y });
    set(CGEventField::MouseEventButtonNumber, 0);
    set(CGEventField::MouseEventSubtype, MOUSE_SUBTYPE);
    set(CGEventField::MouseEventClickState, click_state);
    set(CGEventField::MouseEventWindowUnderMousePointer, window_id);
    set(
        CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
        window_id,
    );
    let local = CGPoint {
        x: x - address.origin.0,
        y: y - address.origin.1,
    };
    // SAFETY: `event` is a live CGEvent and the function only writes the
    // event's window-location field.
    unsafe { set_location(&raw const *event, local) };
    Ok(event)
}

fn null_event(function: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::Internal,
        format!("{function} returned no event"),
    )
}

#[cfg(test)]
mod tests {
    //! These tests build events and read them back. They never post.

    use objc2_core_graphics::CGEventType;

    use super::*;

    const ADDRESS: WindowAddress = WindowAddress {
        pid: 7,
        window_id: 4242,
        origin: (100.0, 50.0),
    };

    #[test]
    fn a_mouse_event_names_its_window_and_keeps_its_desktop_location() {
        let Some(set_location) = set_window_location() else {
            return;
        };
        let event = mouse_event(
            &ADDRESS,
            NSEventType::LeftMouseDown,
            (130.5, 70.25),
            2,
            set_location,
        )
        .unwrap();

        let field = |field| CGEvent::integer_value_field(Some(&event), field);
        assert_eq!(CGEvent::r#type(Some(&event)), CGEventType::LeftMouseDown);
        let location = CGEvent::location(Some(&event));
        assert_eq!((location.x, location.y), (130.5, 70.25));
        assert_eq!(field(CGEventField::MouseEventWindowUnderMousePointer), 4242);
        assert_eq!(
            field(CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent),
            4242
        );
        assert_eq!(field(CGEventField::MouseEventClickState), 2);
        assert_eq!(field(CGEventField::MouseEventSubtype), MOUSE_SUBTYPE);
        assert_eq!(CGEvent::flags(Some(&event)), CGEventFlags::empty());
    }

    #[test]
    fn application_events_are_appkit_defined() {
        for (subtype, window_id) in [(APPLICATION_ACTIVATED, 4242), (APPLICATION_DEACTIVATED, 0)] {
            let event = application_event(subtype, window_id).unwrap();
            // kCGEventType 13 is the AppKit-defined event class.
            assert_eq!(CGEvent::r#type(Some(&event)).0, 13);
        }
    }

    #[test]
    fn a_zero_click_count_is_rejected_before_anything_is_posted() {
        let error = click(&ADDRESS, Activation::None, (0.0, 0.0), 0).unwrap_err();
        assert_eq!(error.code, "InvalidRequest");
    }
}
