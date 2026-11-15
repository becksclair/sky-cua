//! AX window lookup by CoreGraphics identity, window raising, the key window
//! check, and the priming point. Runs on a lane thread only.

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::RectF;

use super::element::{
    AxElement, AxErr, as_bool, as_element, as_elements, as_point, as_size, as_string, cf_string,
};
use super::lane::{JobGuard, LaneContext};
use super::tree::{Names, ax_error};
use crate::types::MacWindow;
use crate::window_match::{self, AxWindowCandidate, WindowMatch};

/// Finds the AX element of exactly `window` among its application's
/// `AXWindows`. Fails closed with `AccessibilityCoverageLimited` when the
/// correlation is not exact. Stops between windows once the caller has
/// stopped waiting, so an abandoned lane does not keep querying an
/// application that is already slow.
pub(crate) fn find_window(
    context: &LaneContext,
    guard: &JobGuard<'_>,
    names: &Names,
    window: &MacWindow,
    app_windows: &[MacWindow],
) -> Result<AxElement, BackendError> {
    let windows = context
        .app
        .copy_attribute(&names.windows)
        .map_err(|error| {
            ax_error(
                error,
                &format!("failed to read AXWindows of pid {}", context.pid),
            )
        })?;
    let elements = as_elements(&windows, usize::MAX);
    let mut candidates = Vec::with_capacity(elements.len());
    for element in &elements {
        guard.ensure_wanted()?;
        // One round trip per window: title, frame, minimized, main.
        let values = match element.copy_multiple(&names.window_batch) {
            Ok(values) => values,
            // A window that closed after `AXWindows` was read cannot be the
            // target. It keeps its slot, frameless, so candidate indexes stay
            // aligned with `elements` and the other windows still correlate.
            Err(error) if error == AxErr::INVALID_ELEMENT => Vec::new(),
            Err(error) => return Err(ax_error(error, "failed to read an AX window's frame")),
        };
        let get = |index: usize| values.get(index).and_then(|value| value.as_ref().ok());
        candidates.push(AxWindowCandidate {
            title: get(0).and_then(|value| as_string(value)),
            position: get(1).and_then(|value| as_point(value)),
            size: get(2).and_then(|value| as_size(value)),
            minimized: get(3).and_then(|value| as_bool(value)).unwrap_or(false),
            main: get(4).and_then(|value| as_bool(value)).unwrap_or(false),
        });
    }
    let target = || {
        format!(
            "CoreGraphics window {} of {} (pid {}) at ({}, {}) {}x{}",
            window.window_id,
            window.owner_name,
            window.pid,
            window.bounds.x,
            window.bounds.y,
            window.bounds.width,
            window.bounds.height
        )
    };
    match window_match::match_ax_window(window, app_windows, &candidates) {
        WindowMatch::Exact(index) => Ok(elements[index].clone()),
        WindowMatch::NotFound => Err(BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!(
                "no Accessibility window matches {} among {} AX windows (not found); the window may be moving, minimized, or not exposed to Accessibility",
                target(),
                candidates.len()
            ),
        )),
        WindowMatch::Ambiguous(indices) => Err(BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!(
                "several Accessibility windows match {} (ambiguous: {} candidates share its frame and title); refusing to guess",
                target(),
                indices.len()
            ),
        )),
    }
}

/// Raises `window` within its application and brings the application to the
/// front: `AXRaise` and `AXMain` on the window, then `AXFrontmost` on the
/// application, so the requested window is already on top when the
/// application activates. Every step is attempted; any failure is reported.
pub(crate) fn raise(
    context: &LaneContext,
    guard: &JobGuard<'_>,
    names: &Names,
    window: &MacWindow,
    app_windows: &[MacWindow],
) -> Result<(), BackendError> {
    let element = find_window(context, guard, names, window, app_windows)?;
    // Finding the window can outlast the caller's wait when the application
    // answers slowly; a raise sent after that would steal focus from whatever
    // request runs next.
    guard.ensure_wanted()?;
    let mut failures = Vec::new();
    if let Err(error) = element.perform(&cf_string("AXRaise")) {
        if error == AxErr::INVALID_ELEMENT {
            return Err(ax_error(error, "the AX window vanished before AXRaise"));
        }
        failures.push(format!("AXRaise: {error}"));
    }
    guard.ensure_wanted()?;
    if let Err(error) = element.set_bool(&cf_string("AXMain"), true) {
        failures.push(format!("AXMain: {error}"));
    }
    guard.ensure_wanted()?;
    if let Err(error) = context.app.set_bool(&cf_string("AXFrontmost"), true) {
        failures.push(format!("AXFrontmost: {error}"));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!(
                "macOS Accessibility could not fully raise window {} of pid {}: {}",
                window.window_id,
                context.pid,
                failures.join("; ")
            ),
        ))
    }
}

/// Whether `window` is the window its application hands key events to: the
/// application's `AXFocusedWindow`. An application that is not active still
/// reports one, and it is where key events posted to the process arrive. An
/// application that reports none has no key window.
pub(crate) fn receives_keys(
    context: &LaneContext,
    guard: &JobGuard<'_>,
    names: &Names,
    window: &MacWindow,
    app_windows: &[MacWindow],
) -> Result<bool, BackendError> {
    let element = find_window(context, guard, names, window, app_windows)?;
    guard.ensure_wanted()?;
    let focused = match context.app.copy_attribute(&cf_string("AXFocusedWindow")) {
        Ok(focused) => focused,
        Err(error) if error.is_unsupported() => return Ok(false),
        Err(error) => {
            return Err(ax_error(
                error,
                &format!("failed to read AXFocusedWindow of pid {}", context.pid),
            ));
        }
    };
    Ok(as_element(focused).is_some_and(|focused| focused == element))
}

/// Actions an element may offer and still do nothing when left-clicked.
const INERT_ACTIONS: &[&str] = &["AXRaise", "AXShowMenu", "AXScrollToVisible", "AXCancel"];

/// A point of `window` where a left click does nothing but make the window
/// key: the window's `AXActivationPoint`, when it lies inside the window and
/// the element under it belongs to that window and offers no action a click
/// would trigger. `None` when the window offers no such point; applications
/// that are not built on AppKit report an activation point outside their
/// window.
pub(crate) fn priming_point(
    context: &LaneContext,
    guard: &JobGuard<'_>,
    names: &Names,
    window: &MacWindow,
    app_windows: &[MacWindow],
) -> Result<Option<(f64, f64)>, BackendError> {
    let element = find_window(context, guard, names, window, app_windows)?;
    guard.ensure_wanted()?;
    let point = match element.copy_attribute(&cf_string("AXActivationPoint")) {
        Ok(value) => as_point(&value),
        Err(error) if error.is_unsupported() => None,
        Err(error) => return Err(ax_error(error, "failed to read AXActivationPoint")),
    };
    let Some((x, y)) = point.filter(|&point| contains(&window.bounds, point)) else {
        return Ok(None);
    };
    guard.ensure_wanted()?;
    let under = match context.app.element_at_position(x, y) {
        Ok(under) => under,
        Err(error) if error.is_unsupported() => return Ok(None),
        Err(error) => return Err(ax_error(error, "failed to hit-test the activation point")),
    };
    // The hit-test covers every window of the application, so a sibling
    // window in front would be checked instead of this one.
    let in_window = under == element
        || under
            .copy_attribute(&cf_string("AXWindow"))
            .ok()
            .and_then(as_element)
            .is_some_and(|owner| owner == element);
    if !in_window {
        return Ok(None);
    }
    let actions = under
        .action_names()
        .map_err(|error| ax_error(error, "failed to read the actions at the activation point"))?;
    Ok(only_inert(&actions).then_some((x, y)))
}

fn contains(bounds: &RectF, (x, y): (f64, f64)) -> bool {
    x >= bounds.x && x < bounds.right() && y >= bounds.y && y < bounds.bottom()
}

/// Whether a left click on an element offering `actions` does nothing. An
/// element with no actions, such as the window or a hosting view, is inert.
fn only_inert(actions: &[String]) -> bool {
    actions
        .iter()
        .all(|action| INERT_ACTIONS.contains(&action.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::desktop_rect;

    #[test]
    fn a_priming_point_must_lie_inside_the_window() {
        let bounds = desktop_rect(100.0, 50.0, 400.0, 300.0);

        assert!(contains(&bounds, (130.0, 66.0)));
        assert!(contains(&bounds, (100.0, 50.0)));
        assert!(!contains(&bounds, (500.0, 66.0)));
        assert!(!contains(&bounds, (130.0, 350.0)));
        assert!(!contains(&bounds, (99.0, 66.0)));
    }

    #[test]
    fn only_an_element_whose_actions_are_all_inert_can_be_primed() {
        let actions = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert!(only_inert(&[]));
        assert!(only_inert(&actions(&["AXRaise", "AXShowMenu"])));
        assert!(!only_inert(&actions(&["AXPress"])));
        assert!(!only_inert(&actions(&["AXRaise", "AXPress"])));
    }
}
