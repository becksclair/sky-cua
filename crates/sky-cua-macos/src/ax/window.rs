//! AX window lookup by CoreGraphics identity, and window raising. Runs on a
//! lane thread only.

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

use super::element::{
    AxElement, AxErr, as_bool, as_elements, as_point, as_size, as_string, cf_string,
};
use super::lane::LaneContext;
use super::tree::{Names, ax_error};
use crate::types::MacWindow;
use crate::window_match::{self, AxWindowCandidate, WindowMatch};

/// Finds the AX element of exactly `window` among its application's
/// `AXWindows`. Fails closed with `AccessibilityCoverageLimited` when the
/// correlation is not exact.
pub(crate) fn find_window(
    context: &LaneContext,
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
    names: &Names,
    window: &MacWindow,
    app_windows: &[MacWindow],
) -> Result<(), BackendError> {
    let element = find_window(context, names, window, app_windows)?;
    let mut failures = Vec::new();
    if let Err(error) = element.perform(&cf_string("AXRaise")) {
        if error == AxErr::INVALID_ELEMENT {
            return Err(ax_error(error, "the AX window vanished before AXRaise"));
        }
        failures.push(format!("AXRaise: {error}"));
    }
    if let Err(error) = element.set_bool(&cf_string("AXMain"), true) {
        failures.push(format!("AXMain: {error}"));
    }
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
