//! CGEvent pointer and keyboard injection.
//!
//! Every function is synchronous and may sleep between events; callers run
//! them on a blocking thread. All points are global points with a top-left
//! origin (`DesktopLogical`).
//!
//! Event source and tap: events are created from a
//! `kCGEventSourceStateHIDSystemState` source and posted at `kCGHIDEventTap`,
//! the point where hardware events enter the window server. From there they
//! take the same path as a physical device: they move the visible cursor,
//! update the global button and modifier state, pass through every
//! session-level event tap, and are routed to whatever window is under the
//! pointer or holds key focus. That routing is independent of the posting
//! process, which matters because the daemon is never the active application
//! and owns no windows. `kCGSessionEventTap` enters after the HID-level state
//! update, and `CGEventPostToPid` bypasses window-server routing (no hover,
//! no focus change on click), so neither suits a physical-input fallback. The
//! HID-system state source matches that tap; a private-state source would
//! keep its own button and modifier bookkeeping that diverges from what the
//! window server sees. The source's local-event suppression interval is set
//! to zero so the user's own keyboard and mouse are never suppressed after an
//! injected event. Every event's flags are set explicitly, so modifiers the
//! user physically holds never leak into injected clicks or keystrokes.

mod plan;

use std::thread::sleep;
use std::time::Duration;

use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventType, CGMouseButton, CGScrollEventUnit,
};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

use crate::keymap;
use crate::permissions::{self, Permission};
use crate::types::MouseButton;

use plan::{KeyStroke, StrokeKind, TextSegment};

/// Pointer rest after moving onto a target before pressing, so hover-driven
/// controls (menus, toolbar buttons, web hover states) update first.
const HOVER_DWELL: Duration = Duration::from_millis(40);
/// Button hold between press and release of one click.
const CLICK_HOLD: Duration = Duration::from_millis(30);
/// Gap between the clicks of a multi-click; well inside the system
/// double-click interval (0.5 s by default).
const MULTI_CLICK_GAP: Duration = Duration::from_millis(40);
/// Hold after pressing before the first dragged event, and before releasing
/// after the last, so drag sources and drop targets register both ends.
const DRAG_EDGE_DWELL: Duration = Duration::from_millis(80);
/// Gap between interpolated dragged events.
const DRAG_STEP_GAP: Duration = Duration::from_millis(8);
/// Gap between scroll events.
const SCROLL_STEP_GAP: Duration = Duration::from_millis(8);
/// Gap between keyboard events, so applications that sample modifier state
/// asynchronously see each transition in order.
const KEY_EVENT_GAP: Duration = Duration::from_millis(4);

pub fn move_pointer(x: f64, y: f64) -> Result<(), BackendError> {
    let poster = Poster::new()?;
    poster.mouse(CGEventType::MouseMoved, (x, y), CGMouseButton::Left, 0)
}

/// Clicks `count` times (1 = single, 2 = double) at a point.
pub fn click(x: f64, y: f64, button: MouseButton, count: u32) -> Result<(), BackendError> {
    if count == 0 {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "click count must be at least 1",
        ));
    }
    let poster = Poster::new()?;
    let (down, up, cg_button) = button_events(button);
    poster.mouse(CGEventType::MouseMoved, (x, y), CGMouseButton::Left, 0)?;
    sleep(HOVER_DWELL);
    for click_state in 1..=i64::from(count) {
        if click_state > 1 {
            sleep(MULTI_CLICK_GAP);
        }
        // The click-state field is what AppKit reports as `clickCount`; the
        // window server does not derive it for synthetic events.
        poster.mouse(down, (x, y), cg_button, click_state)?;
        sleep(CLICK_HOLD);
        poster.mouse(up, (x, y), cg_button, click_state)?;
    }
    Ok(())
}

/// Presses at `from`, moves through interpolated points, and releases at `to`.
pub fn drag(from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
    let poster = Poster::new()?;
    poster.mouse(CGEventType::MouseMoved, from, CGMouseButton::Left, 0)?;
    sleep(HOVER_DWELL);
    poster.mouse(CGEventType::LeftMouseDown, from, CGMouseButton::Left, 1)?;
    let dragged = (|| {
        sleep(DRAG_EDGE_DWELL);
        for point in plan::drag_path(from, to) {
            poster.mouse(CGEventType::LeftMouseDragged, point, CGMouseButton::Left, 1)?;
            sleep(DRAG_STEP_GAP);
        }
        sleep(DRAG_EDGE_DWELL);
        Ok(())
    })();
    // Release even when a dragged event failed, so the button is never left
    // held on the user's desktop.
    let released = poster.mouse(CGEventType::LeftMouseUp, to, CGMouseButton::Left, 1);
    dragged.and(released)
}

/// Scrolls at an optional point, in wheel lines, using the CGEvent wheel
/// convention: positive `delta_y` moves the content down (the view scrolls
/// toward the top) and positive `delta_x` moves the content right (the view
/// scrolls toward the left).
pub fn scroll(at: Option<(f64, f64)>, delta_x: i32, delta_y: i32) -> Result<(), BackendError> {
    let poster = Poster::new()?;
    if let Some(point) = at {
        poster.mouse(CGEventType::MouseMoved, point, CGMouseButton::Left, 0)?;
        sleep(HOVER_DWELL);
    }
    for (index, (wheel1, wheel2)) in plan::scroll_steps(delta_x, delta_y).into_iter().enumerate() {
        if index > 0 {
            sleep(SCROLL_STEP_GAP);
        }
        let event = poster.scroll_event(at, wheel1, wheel2)?;
        poster.post(&event);
    }
    Ok(())
}

/// Types text as Unicode key events, independent of the keyboard layout.
pub fn type_text(text: &str) -> Result<(), BackendError> {
    if text.is_empty() {
        return Ok(());
    }
    let poster = Poster::new()?;
    for (index, segment) in plan::text_segments(text).into_iter().enumerate() {
        if index > 0 {
            sleep(KEY_EVENT_GAP);
        }
        let (key_code, units, implied) = match &segment {
            TextSegment::Unicode(units) => (0, Some(units.as_slice()), 0),
            TextSegment::Key(code) => (*code, None, plan::implied_key_flags(*code)),
        };
        let down = poster.key_event(key_code, true, implied, units)?;
        let up = poster.key_event(key_code, false, implied, units)?;
        poster.post(&down);
        poster.post(&up);
    }
    Ok(())
}

/// Presses one chord; see `keymap::parse_chord` for accepted key names.
pub fn press_keys(keys: &[String]) -> Result<(), BackendError> {
    let chord = keymap::parse_chord(keys)?;
    let poster = Poster::new()?;
    let strokes = plan::chord_plan(&chord);
    for (index, stroke) in strokes.iter().enumerate() {
        if index > 0 {
            sleep(KEY_EVENT_GAP);
        }
        if let Err(error) = poster.stroke(stroke) {
            // Release whatever this chord still holds before reporting, so no
            // modifier stays latched on the user's desktop.
            for release in strokes[index + 1..].iter().filter(|stroke| !stroke.down) {
                let _ = poster.stroke(release);
            }
            return Err(error);
        }
    }
    Ok(())
}

fn button_events(button: MouseButton) -> (CGEventType, CGEventType, CGMouseButton) {
    match button {
        MouseButton::Left => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGMouseButton::Left,
        ),
        MouseButton::Right => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            CGMouseButton::Right,
        ),
    }
}

/// An event source configured for posting; see the module docs.
struct Poster {
    source: CFRetained<CGEventSource>,
}

impl Poster {
    /// Checks the event-synthesis grant and creates the event source.
    fn new() -> Result<Self, BackendError> {
        if !permissions::status().post_event {
            return Err(permissions::missing_permission_error(Permission::PostEvent));
        }
        Self::unchecked()
    }

    /// Creates the event source without the permission check. Building events
    /// needs no grant; only posting does.
    fn unchecked() -> Result<Self, BackendError> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .ok_or_else(|| null_event("CGEventSourceCreate"))?;
        CGEventSource::set_local_events_suppression_interval(Some(&source), 0.0);
        Ok(Self { source })
    }

    fn post(&self, event: &CGEvent) {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(event));
    }

    fn mouse(
        &self,
        event_type: CGEventType,
        point: (f64, f64),
        button: CGMouseButton,
        click_state: i64,
    ) -> Result<(), BackendError> {
        let event = self.mouse_event(event_type, point, button, click_state)?;
        self.post(&event);
        Ok(())
    }

    fn mouse_event(
        &self,
        event_type: CGEventType,
        (x, y): (f64, f64),
        button: CGMouseButton,
        click_state: i64,
    ) -> Result<CFRetained<CGEvent>, BackendError> {
        let event =
            CGEvent::new_mouse_event(Some(&self.source), event_type, CGPoint { x, y }, button)
                .ok_or_else(|| null_event("CGEventCreateMouseEvent"))?;
        CGEvent::set_flags(Some(&event), CGEventFlags::empty());
        if click_state > 0 {
            CGEvent::set_integer_value_field(
                Some(&event),
                CGEventField::MouseEventClickState,
                click_state,
            );
        }
        Ok(event)
    }

    fn scroll_event(
        &self,
        at: Option<(f64, f64)>,
        wheel1: i32,
        wheel2: i32,
    ) -> Result<CFRetained<CGEvent>, BackendError> {
        let event = CGEvent::new_scroll_wheel_event2(
            Some(&self.source),
            CGScrollEventUnit::Line,
            2,
            wheel1,
            wheel2,
            0,
        )
        .ok_or_else(|| null_event("CGEventCreateScrollWheelEvent2"))?;
        CGEvent::set_flags(Some(&event), CGEventFlags::empty());
        if let Some((x, y)) = at {
            CGEvent::set_location(Some(&event), CGPoint { x, y });
        }
        Ok(event)
    }

    /// A key event with explicit flags and an optional Unicode payload.
    fn key_event(
        &self,
        key_code: u16,
        down: bool,
        flags: u64,
        units: Option<&[u16]>,
    ) -> Result<CFRetained<CGEvent>, BackendError> {
        let event = CGEvent::new_keyboard_event(Some(&self.source), key_code, down)
            .ok_or_else(|| null_event("CGEventCreateKeyboardEvent"))?;
        CGEvent::set_flags(Some(&event), CGEventFlags::from_bits_retain(flags));
        if let Some(units) = units {
            debug_assert!(units.len() <= plan::MAX_UTF16_PER_EVENT);
            // SAFETY: `units` is a live slice and the length passed is its
            // length; CoreGraphics copies the string into the event.
            unsafe {
                CGEvent::keyboard_set_unicode_string(
                    Some(&event),
                    units.len() as _,
                    units.as_ptr(),
                );
            }
        }
        Ok(event)
    }

    fn stroke_event(&self, stroke: &KeyStroke) -> Result<CFRetained<CGEvent>, BackendError> {
        let mut buffer = [0u16; 2];
        let units = stroke.character.map(|ch| &*ch.encode_utf16(&mut buffer));
        let event = self.key_event(stroke.key_code, stroke.down, stroke.flags, units)?;
        if stroke.kind == StrokeKind::FlagsChanged {
            // Hardware reports modifier keys as flags-changed events, not key
            // downs; applications that watch modifiers listen for these.
            CGEvent::set_type(Some(&event), CGEventType::FlagsChanged);
        }
        Ok(event)
    }

    fn stroke(&self, stroke: &KeyStroke) -> Result<(), BackendError> {
        let event = self.stroke_event(stroke)?;
        self.post(&event);
        Ok(())
    }
}

fn null_event(function: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::Internal,
        format!("{function} returned null"),
    )
}

#[cfg(test)]
mod tests {
    //! These tests build CGEvents and read them back. They never post.

    use super::*;
    use crate::keymap::vk;

    #[test]
    fn plan_flag_bits_match_coregraphics() {
        assert_eq!(plan::flag::SHIFT, CGEventFlags::MaskShift.bits());
        assert_eq!(plan::flag::CONTROL, CGEventFlags::MaskControl.bits());
        assert_eq!(plan::flag::ALTERNATE, CGEventFlags::MaskAlternate.bits());
        assert_eq!(plan::flag::COMMAND, CGEventFlags::MaskCommand.bits());
        assert_eq!(plan::flag::NUMERIC_PAD, CGEventFlags::MaskNumericPad.bits());
        assert_eq!(
            plan::flag::SECONDARY_FN,
            CGEventFlags::MaskSecondaryFn.bits()
        );
    }

    #[test]
    fn builds_mouse_events_with_click_state_and_location() {
        let poster = Poster::unchecked().unwrap();
        let event = poster
            .mouse_event(
                CGEventType::RightMouseDown,
                (123.5, 456.25),
                CGMouseButton::Right,
                2,
            )
            .unwrap();
        assert_eq!(CGEvent::r#type(Some(&event)), CGEventType::RightMouseDown);
        let location = CGEvent::location(Some(&event));
        assert_eq!((location.x, location.y), (123.5, 456.25));
        assert_eq!(
            CGEvent::integer_value_field(Some(&event), CGEventField::MouseEventClickState),
            2
        );
        assert_eq!(CGEvent::flags(Some(&event)), CGEventFlags::empty());
    }

    #[test]
    fn builds_unicode_key_events_with_the_full_payload() {
        let poster = Poster::unchecked().unwrap();
        let units: Vec<u16> = "héllo 😀 wörld!!".encode_utf16().collect();
        assert!(units.len() <= plan::MAX_UTF16_PER_EVENT);
        let event = poster.key_event(0, true, 0, Some(&units)).unwrap();
        let mut read = [0u16; 32];
        let mut length = 0;
        // SAFETY: both pointers reference live locals sized as declared.
        unsafe {
            CGEvent::keyboard_get_unicode_string(
                Some(&event),
                read.len() as _,
                &mut length,
                read.as_mut_ptr(),
            );
        }
        assert_eq!(&read[..length as usize], units.as_slice());
        assert_eq!(CGEvent::r#type(Some(&event)), CGEventType::KeyDown);
    }

    #[test]
    fn builds_chord_strokes_with_explicit_flags_and_types() {
        let poster = Poster::unchecked().unwrap();
        let keys = vec!["Ctrl".to_string(), "Shift".to_string(), "Tab".to_string()];
        let strokes = plan::chord_plan(&keymap::parse_chord(&keys).unwrap());
        let events: Vec<_> = strokes
            .iter()
            .map(|stroke| poster.stroke_event(stroke).unwrap())
            .collect();
        let summary: Vec<_> = events
            .iter()
            .map(|event| {
                (
                    CGEvent::r#type(Some(event)),
                    CGEvent::integer_value_field(Some(event), CGEventField::KeyboardEventKeycode),
                    CGEvent::flags(Some(event)),
                )
            })
            .collect();
        let control = CGEventFlags::MaskControl;
        let both = CGEventFlags::MaskControl | CGEventFlags::MaskShift;
        assert_eq!(
            summary,
            vec![
                (CGEventType::FlagsChanged, i64::from(vk::CONTROL), control),
                (CGEventType::FlagsChanged, i64::from(vk::SHIFT), both),
                (CGEventType::KeyDown, i64::from(vk::TAB), both),
                (CGEventType::KeyUp, i64::from(vk::TAB), both),
                (CGEventType::FlagsChanged, i64::from(vk::SHIFT), control),
                (
                    CGEventType::FlagsChanged,
                    i64::from(vk::CONTROL),
                    CGEventFlags::empty()
                ),
            ]
        );
    }

    #[test]
    fn builds_line_scroll_events() {
        let poster = Poster::unchecked().unwrap();
        let event = poster.scroll_event(Some((10.0, 20.0)), -3, 2).unwrap();
        assert_eq!(CGEvent::r#type(Some(&event)), CGEventType::ScrollWheel);
        assert_eq!(
            CGEvent::integer_value_field(Some(&event), CGEventField::ScrollWheelEventDeltaAxis1),
            -3
        );
        assert_eq!(
            CGEvent::integer_value_field(Some(&event), CGEventField::ScrollWheelEventDeltaAxis2),
            2
        );
        assert_eq!(
            CGEvent::integer_value_field(Some(&event), CGEventField::ScrollWheelEventIsContinuous),
            0
        );
    }

    #[test]
    fn zero_click_count_is_rejected_before_posting() {
        let error = click(0.0, 0.0, MouseButton::Left, 0).unwrap_err();
        assert_eq!(error.code, "InvalidRequest");
    }

    #[test]
    fn invalid_chord_is_rejected_before_posting() {
        let error = press_keys(&["Ctrl".to_string(), "Nope".to_string()]).unwrap_err();
        assert_eq!(error.code, "InvalidRequest");
    }
}
