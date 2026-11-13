//! Action argument parsing and physical CGEvent execution.
//!
//! Parsing is pure: `plan_action` turns an `ActionRequest` into a
//! `PhysicalInput` with every point already in desktop points, using the same
//! argument names and defaults as the other desktop backends. `perform` posts
//! the events and is synchronous, so callers run it on a blocking thread.

use std::time::Duration;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, CoordinateSpace, ElementNode, RectF,
};

use crate::coords::{action_point_to_desktop, desktop_rect};
use crate::input;
use crate::types::MouseButton;

/// Wheel units per notch in the shared action contract (`delta_y`, and one
/// notch per page), the same unit the Windows and Linux backends use.
const WHEEL_UNITS_PER_NOTCH: f64 = 120.0;

/// Wheel lines one notch scrolls (the Windows default the shared contract's
/// notch unit was defined against).
const LINES_PER_NOTCH: f64 = 3.0;

/// Pause after the focusing click of a `set_value` fallback, so the target
/// has taken key focus before select-all is sent (the Linux fallback waits
/// the same).
const FOCUS_CLICK_SETTLE: Duration = Duration::from_millis(40);

/// One physical input operation with every point in desktop points.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum PhysicalInput {
    Move {
        at: (f64, f64),
    },
    Click {
        at: (f64, f64),
        button: MouseButton,
        count: u32,
    },
    /// Deltas use the `input::scroll` sign convention.
    Scroll {
        at: Option<(f64, f64)>,
        delta_x: i32,
        delta_y: i32,
    },
    Drag {
        from: (f64, f64),
        to: (f64, f64),
    },
    TypeText(String),
    PressKeys(Vec<String>),
    /// Select-all followed by typing the value. `at` is clicked first so the
    /// target holds key focus; `None` replaces the content of whatever
    /// already holds it.
    SetValue {
        at: Option<(f64, f64)>,
        value: String,
    },
}

/// What physical input can do for one request.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ActionPlan {
    Physical(PhysicalInput),
    /// The action is a semantic primitive with no physical equivalent.
    RequiresAccessibility,
}

pub(super) fn plan_action(request: &ActionRequest) -> Result<ActionPlan, BackendError> {
    let input = match request.action {
        ActionName::FocusElement
        | ActionName::ActivateElement
        | ActionName::SelectElement
        | ActionName::ExpandElement
        | ActionName::CollapseElement
        | ActionName::ToggleElement
        | ActionName::PerformAction => return Ok(ActionPlan::RequiresAccessibility),
        ActionName::Click => PhysicalInput::Click {
            at: desktop_action_point(request)?,
            button: mouse_button(request)?,
            count: click_count(request),
        },
        ActionName::Move => PhysicalInput::Move {
            at: desktop_action_point(request)?,
        },
        ActionName::PerformSecondaryAction => PhysicalInput::Click {
            at: desktop_action_point(request)?,
            button: MouseButton::Right,
            count: 1,
        },
        ActionName::Scroll => {
            let (delta_x, delta_y) = scroll_deltas(request)?;
            PhysicalInput::Scroll {
                at: optional_action_point(request)?,
                delta_x,
                delta_y,
            }
        }
        ActionName::Drag => PhysicalInput::Drag {
            from: desktop_drag_from_point(request)?,
            to: desktop_target_point(request)?,
        },
        ActionName::TypeText => PhysicalInput::TypeText(
            required_text_arg(request, "text", "type_text requires text")?.to_owned(),
        ),
        ActionName::PressKey => PhysicalInput::PressKeys(parse_keys(request)?),
        ActionName::SetValue => {
            if let Some(element) = request.resolved_element.as_ref()
                && !accepts_typed_value(element)
            {
                return Err(BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!(
                        "set_value was not performed through Accessibility and has no physical fallback for a {} element, which is not a text field; observe again if its value should be settable",
                        element.role
                    ),
                ));
            }
            PhysicalInput::SetValue {
                value: required_text_arg(request, "value", "set_value requires value")?.to_owned(),
                at: optional_action_point(request)?,
            }
        }
    };
    Ok(ActionPlan::Physical(input))
}

/// Whether clicking `element` and typing can stand in for a value write.
///
/// The physical `set_value` fallback clicks its target, selects all, and
/// types. On anything but a text field the click is the control's own
/// action (it presses the button, opens the pop-up menu) or takes no key
/// focus (a label), and the typing lands in whatever held key focus. Text
/// readback is present exactly on text fields, text areas, combo boxes, and
/// search fields; the shared `text` and `combo_box` roles also cover static
/// labels and pop-up buttons, so the role is not the test.
fn accepts_typed_value(element: &ElementNode) -> bool {
    element.supports_editable_text || element.text.is_some()
}

/// Posts `input` through CGEvent. Blocks; run on a blocking thread.
pub(super) fn perform(input: PhysicalInput) -> Result<ActionOutcome, BackendError> {
    match input {
        PhysicalInput::Move { at: (x, y) } => {
            input::move_pointer(x, y)?;
            Ok(success("CGEvent pointer move completed"))
        }
        PhysicalInput::Click {
            at: (x, y),
            button,
            count,
        } => {
            input::click(x, y, button, count)?;
            Ok(success(match button {
                MouseButton::Left => "CGEvent click completed",
                MouseButton::Right => "CGEvent secondary click completed",
            }))
        }
        PhysicalInput::Scroll {
            at,
            delta_x,
            delta_y,
        } => {
            input::scroll(at, delta_x, delta_y)?;
            Ok(success("CGEvent scroll completed"))
        }
        PhysicalInput::Drag { from, to } => {
            input::drag(from, to)?;
            Ok(success("CGEvent drag completed"))
        }
        PhysicalInput::TypeText(text) => {
            input::type_text(&text)?;
            Ok(success("CGEvent text completed"))
        }
        PhysicalInput::PressKeys(keys) => {
            input::press_keys(&keys)?;
            Ok(success("CGEvent key sequence completed"))
        }
        PhysicalInput::SetValue { at, value } => {
            if let Some((x, y)) = at {
                input::click(x, y, MouseButton::Left, 1)?;
                std::thread::sleep(FOCUS_CLICK_SETTLE);
            }
            input::press_keys(&["Cmd".to_string(), "A".to_string()])?;
            if value.is_empty() {
                // Typing nothing would leave the old content selected.
                input::press_keys(&["Backspace".to_string()])?;
            } else {
                input::type_text(&value)?;
            }
            let mut outcome =
                success("macOS used CGEvent focus/select-all/type fallback for set_value");
            outcome.diagnostics.push(
                BackendError::new(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    "Accessibility value setting was unavailable; physical fallback was used",
                )
                .diagnostic(),
            );
            Ok(outcome)
        }
    }
}

pub(super) fn success(message: &str) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: message.to_string(),
        code: "Completed".to_string(),
        diagnostics: Vec::new(),
        agent_cursor: None,
    }
}

/// True for the actions an AX semantic attempt could have handled, so a
/// physical fallback for them is worth a diagnostic.
pub(super) fn has_semantic_equivalent(action: &ActionName) -> bool {
    matches!(
        action,
        ActionName::Click
            | ActionName::FocusElement
            | ActionName::ActivateElement
            | ActionName::SelectElement
            | ActionName::ExpandElement
            | ActionName::CollapseElement
            | ActionName::ToggleElement
            | ActionName::SetValue
    )
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

/// The desktop-point action point of a resolved element: the center of the
/// part of it that is visible in the snapshot's capture.
///
/// Bounds tagged `DesktopLogical` (an AX tree read without a capture, as the
/// AppShot path does) are already desktop points; any other space is the
/// snapshot's capture space and is mapped through the request's capture.
fn element_desktop_center(
    request: &ActionRequest,
    element: Option<&ElementNode>,
) -> Option<Result<(f64, f64), BackendError>> {
    let bounds = element?.bounds.as_ref()?;
    Some(element_desktop_rect(request, bounds).and_then(|rect| visible_center(request, &rect)))
}

fn element_desktop_rect(request: &ActionRequest, bounds: &RectF) -> Result<RectF, BackendError> {
    if bounds.space == CoordinateSpace::DesktopLogical {
        return Ok(bounds.clone());
    }
    let (x, y) = to_desktop(request, (bounds.x, bounds.y))?;
    let (right, bottom) = to_desktop(request, (bounds.right(), bounds.bottom()))?;
    Ok(desktop_rect(x, y, right - x, bottom - y))
}

/// Clips an element rectangle to the captured area before taking its center.
///
/// AX reports the full frame of a scrolled view (a text view's frame is its
/// whole document), so the raw center can lie far outside the window or the
/// screen. An element with no visible part is an error, never a click on
/// whatever happens to be at that point.
fn visible_center(request: &ActionRequest, rect: &RectF) -> Result<(f64, f64), BackendError> {
    let center = |rect: &RectF| (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let Some(visible) = request
        .resolved_capture
        .as_ref()
        .and_then(|capture| capture.logical_rect.as_ref())
        .filter(|visible| visible.space == CoordinateSpace::DesktopLogical && visible.area() > 0.0)
    else {
        return Ok(center(rect));
    };
    if let Some(clipped) = rect.intersection(visible) {
        return Ok(center(&clipped));
    }
    // A zero-area element has no intersection rectangle; its point still counts.
    let (x, y) = center(rect);
    if x >= visible.x && x <= visible.right() && y >= visible.y && y <= visible.bottom() {
        return Ok((x, y));
    }
    Err(BackendError::new(
        BackendErrorCode::InvalidRequest,
        "the target element lies outside the visible area of its window; scroll it into view and observe again",
    ))
}

fn required_f64(request: &ActionRequest, name: &str, message: &str) -> Result<f64, BackendError> {
    request
        .arguments
        .get(name)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| BackendError::new(BackendErrorCode::InvalidRequest, message))
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

fn to_desktop(request: &ActionRequest, point: (f64, f64)) -> Result<(f64, f64), BackendError> {
    action_point_to_desktop(point, request.resolved_capture.as_ref())
}

/// The action point in desktop points: the resolved element's center, else
/// `x`/`y` in the snapshot's coordinate space.
pub(super) fn desktop_action_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    if let Some(center) = element_desktop_center(request, request.resolved_element.as_ref()) {
        return center;
    }
    let point = (
        required_f64(request, "x", "action requires x or element_index")?,
        required_f64(request, "y", "action requires y or element_index")?,
    );
    to_desktop(request, point)
}

/// The action point of a request that names one (a resolved element with
/// bounds, or `x`/`y`), else `None`. A named point that cannot be resolved is
/// an error: the input never falls back to wherever the pointer or the key
/// focus happens to be.
fn optional_action_point(request: &ActionRequest) -> Result<Option<(f64, f64)>, BackendError> {
    let names_element = request
        .resolved_element
        .as_ref()
        .is_some_and(|element| element.bounds.is_some());
    let names_point = ["x", "y"].iter().any(|field| {
        request
            .arguments
            .get(field)
            .is_some_and(|value| !value.is_null())
    });
    if names_element || names_point {
        desktop_action_point(request).map(Some)
    } else {
        Ok(None)
    }
}

fn desktop_drag_from_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    match point_from_fields(&request.arguments, "from_x", "from_y") {
        Some(point) => to_desktop(request, point),
        None => desktop_action_point(request),
    }
}

fn desktop_target_point(request: &ActionRequest) -> Result<(f64, f64), BackendError> {
    if let Some(center) = element_desktop_center(request, request.resolved_target_element.as_ref())
    {
        return center;
    }
    let point = (
        required_f64(request, "to_x", "drag requires to_x or to_element_index")?,
        required_f64(request, "to_y", "drag requires to_y or to_element_index")?,
    );
    to_desktop(request, point)
}

fn mouse_button(request: &ActionRequest) -> Result<MouseButton, BackendError> {
    match request
        .arguments
        .get("mouse_button")
        .and_then(serde_json::Value::as_str)
    {
        None | Some("left" | "l") => Ok(MouseButton::Left),
        Some("right" | "r") => Ok(MouseButton::Right),
        Some(other) => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("macOS physical input does not support mouse_button {other}"),
        )),
    }
}

fn click_count(request: &ActionRequest) -> u32 {
    request
        .arguments
        .get("click_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|count| u32::try_from(count).ok())
        .unwrap_or(1)
        .max(1)
}

/// Scroll deltas for `input::scroll`, from `delta_x`/`delta_y` (wheel units;
/// positive `delta_y` scrolls up and positive `delta_x` scrolls right, as on
/// the other backends) or from `direction` plus `pages` (one notch per page,
/// default one page, default direction down).
fn scroll_deltas(request: &ActionRequest) -> Result<(i32, i32), BackendError> {
    let arguments = &request.arguments;
    let explicit_x = arguments.get("delta_x").and_then(serde_json::Value::as_f64);
    let explicit_y = arguments.get("delta_y").and_then(serde_json::Value::as_f64);
    if explicit_x.is_some() || explicit_y.is_some() {
        return Ok((
            wheel_units_to_lines(explicit_x.unwrap_or(0.0)).saturating_neg(),
            wheel_units_to_lines(explicit_y.unwrap_or(0.0)),
        ));
    }
    let pages = arguments
        .get("pages")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(1.0)
        .max(1.0);
    let units = WHEEL_UNITS_PER_NOTCH * pages;
    match arguments
        .get("direction")
        .and_then(serde_json::Value::as_str)
    {
        Some("up" | "u") => Ok((0, wheel_units_to_lines(units))),
        Some("down" | "d") | None => Ok((0, wheel_units_to_lines(-units))),
        Some("left" | "l") => Ok((wheel_units_to_lines(units), 0)),
        Some("right" | "r") => Ok((wheel_units_to_lines(-units), 0)),
        Some(direction) => Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("unsupported scroll direction: {direction}"),
        )),
    }
}

/// Converts contract wheel units to `input::scroll` lines. `input::scroll`
/// moves content down for positive `delta_y` and right for positive
/// `delta_x` (the CGEvent wheel convention), so a positive (upward) wheel
/// delta keeps its sign. A non-zero delta always moves at least one line.
fn wheel_units_to_lines(units: f64) -> i32 {
    if units == 0.0 || !units.is_finite() {
        return 0;
    }
    let lines = (units / WHEEL_UNITS_PER_NOTCH * LINES_PER_NOTCH).round();
    let lines = if lines == 0.0 { units.signum() } else { lines };
    lines.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn parse_keys(request: &ActionRequest) -> Result<Vec<String>, BackendError> {
    let keys = if let Some(key) = request
        .arguments
        .get("key")
        .and_then(serde_json::Value::as_str)
    {
        key.split('+').map(|part| part.trim().to_string()).collect()
    } else if let Some(keys) = request
        .arguments
        .get("keys")
        .and_then(serde_json::Value::as_array)
    {
        keys.iter()
            .filter_map(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    } else {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "press_key requires key or keys",
        ));
    };
    if keys.is_empty() {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "press_key requires at least one key",
        ));
    }
    Ok(keys)
}

#[cfg(test)]
mod tests;
