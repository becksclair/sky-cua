//! CoreGraphics display enumeration.
//!
//! Everything here is a synchronous CoreGraphics query, so it is current on
//! every call in a process without a main run loop. `NSScreen` is not used:
//! it is main-thread-only, refreshed by run-loop notifications, and uses a
//! bottom-left origin.

use objc2_core_graphics::{
    CGDirectDisplayID, CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayIsBuiltin,
    CGDisplayMode, CGError, CGGetActiveDisplayList, CGMainDisplayID,
};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{DisplayInfo, DisplayTarget, PixelSize};

use crate::coords::desktop_rect;
use crate::types::WINDOW_BACKEND;

/// The name macOS gives the internal panel. External panels have no name
/// reachable through CoreGraphics; their `name` stays `None` rather than a
/// fabricated label.
const BUILT_IN_DISPLAY_NAME: &str = "Built-in Display";

/// Enumerates active displays. `logical_rect` is in global points with a
/// top-left origin; `scale_factor` is the backing scale (pixels per point).
#[must_use]
pub fn enumerate_displays() -> Vec<DisplayInfo> {
    let main = CGMainDisplayID();
    let mut displays: Vec<DisplayInfo> = active_display_ids()
        .into_iter()
        .filter_map(|id| display_info(id, id == main))
        .collect();
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

/// Active display IDs; CoreGraphics lists the main display first.
fn active_display_ids() -> Vec<CGDirectDisplayID> {
    let mut count = 0u32;
    // SAFETY: a null list with a valid count pointer queries the count only.
    if unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) } != CGError::Success
        || count == 0
    {
        return Vec::new();
    }
    let mut ids = vec![0 as CGDirectDisplayID; count as usize];
    // SAFETY: `ids` holds `count` writable entries.
    if unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) } != CGError::Success {
        return Vec::new();
    }
    ids.truncate(count as usize);
    ids
}

fn display_info(id: CGDirectDisplayID, primary: bool) -> Option<DisplayInfo> {
    let bounds = CGDisplayBounds(id);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return None;
    }
    let mode = CGDisplayCopyDisplayMode(id);
    let mode = mode.as_deref();
    let pixel_width = CGDisplayMode::pixel_width(mode);
    let pixel_height = CGDisplayMode::pixel_height(mode);
    let pixel_size = (pixel_width > 0 && pixel_height > 0).then(|| PixelSize {
        width: u32::try_from(pixel_width).unwrap_or(u32::MAX),
        height: u32::try_from(pixel_height).unwrap_or(u32::MAX),
    });
    Some(DisplayInfo {
        display_id: id.to_string(),
        name: CGDisplayIsBuiltin(id).then(|| BUILT_IN_DISPLAY_NAME.to_string()),
        index: 0,
        primary,
        logical_rect: desktop_rect(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            bounds.size.height,
        ),
        pixel_size,
        scale_factor: backing_scale(pixel_width, CGDisplayMode::width(mode)),
        backend: WINDOW_BACKEND.to_string(),
    })
}

/// Pixels per point from the current mode's pixel and point widths.
fn backing_scale(pixel_width: usize, point_width: usize) -> Option<f64> {
    (pixel_width > 0 && point_width > 0).then(|| pixel_width as f64 / point_width as f64)
}

/// The `CGDirectDisplayID` behind a `DisplayInfo` produced by
/// `enumerate_displays`.
pub fn display_id(display: &DisplayInfo) -> Result<u32, BackendError> {
    display.display_id.trim().parse::<u32>().map_err(|_| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!(
                "macOS display_id {:?} is not a CGDirectDisplayID",
                display.display_id
            ),
        )
    })
}

/// Resolves an explicit display target; every provided field must match the
/// same display.
pub fn resolve_display_target(
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
            format!("no macOS display matched requested screenshot target: {target:?}"),
        )),
        _ => Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("macOS display target is ambiguous: {target:?}"),
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
        let value = value.trim();
        let name_matches = display
            .name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(value))
            || display.display_id.eq_ignore_ascii_case(value);
        if !name_matches {
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

/// The primary display, falling back to the first one.
#[must_use]
pub fn primary_display(displays: &[DisplayInfo]) -> Option<DisplayInfo> {
    sky_cua_platform::model::primary_flagged_display(displays)
        .or_else(|| displays.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::{CoordinateSpace, RectF};

    fn display(id: &str, name: Option<&str>, index: u32, primary: bool, x: f64) -> DisplayInfo {
        DisplayInfo {
            display_id: id.to_string(),
            name: name.map(ToOwned::to_owned),
            index,
            primary,
            logical_rect: RectF {
                x,
                y: 0.0,
                width: 1728.0,
                height: 1117.0,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: Some(PixelSize {
                width: 3456,
                height: 2234,
            }),
            scale_factor: Some(2.0),
            backend: WINDOW_BACKEND.to_string(),
        }
    }

    fn two_displays() -> Vec<DisplayInfo> {
        vec![
            display("1", Some(BUILT_IN_DISPLAY_NAME), 0, true, 0.0),
            display("4280324", None, 1, false, 1728.0),
        ]
    }

    #[test]
    fn resolves_primary_and_explicit_display_targets() {
        let displays = two_displays();
        assert_eq!(
            primary_display(&displays).map(|display| display.display_id),
            Some("1".to_string())
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
        assert_eq!(resolved.display_id, "4280324");
    }

    #[test]
    fn display_target_fields_must_match_same_macos_display() {
        let displays = two_displays();
        let resolved = resolve_display_target(
            &displays,
            &DisplayTarget {
                display_id: Some("1".to_string()),
                display_name: Some("built-in display".to_string()),
                display_index: Some(0),
            },
        )
        .unwrap();
        assert_eq!(resolved.display_id, "1");

        let error = resolve_display_target(
            &displays,
            &DisplayTarget {
                display_id: Some("999".to_string()),
                display_name: None,
                display_index: Some(0),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());

        let empty = resolve_display_target(&displays, &DisplayTarget::default()).unwrap_err();
        assert_eq!(empty.code, BackendErrorCode::InvalidRequest.as_str());
    }

    #[test]
    fn unnamed_display_resolves_by_id_as_name() {
        let resolved = resolve_display_target(
            &two_displays(),
            &DisplayTarget {
                display_name: Some("4280324".to_string()),
                ..DisplayTarget::default()
            },
        )
        .unwrap();
        assert_eq!(resolved.index, 1);
    }

    #[test]
    fn primary_falls_back_to_first_display() {
        let mut displays = two_displays();
        displays[0].primary = false;
        assert_eq!(
            primary_display(&displays).map(|display| display.display_id),
            Some("1".to_string())
        );
        assert_eq!(primary_display(&[]), None);
    }

    #[test]
    fn display_id_parses_the_cg_display_id() {
        assert_eq!(display_id(&two_displays()[1]).unwrap(), 4_280_324);
        let mut foreign = two_displays()[0].clone();
        foreign.display_id = "windows:DISPLAY1".to_string();
        assert!(display_id(&foreign).is_err());
    }

    #[test]
    fn backing_scale_is_pixels_per_point() {
        assert_eq!(backing_scale(3456, 1728), Some(2.0));
        assert_eq!(backing_scale(1920, 1920), Some(1.0));
        assert_eq!(backing_scale(0, 1728), None);
    }

    #[test]
    #[ignore = "live desktop probe"]
    fn live_enumerate_displays() {
        let started = std::time::Instant::now();
        let displays = enumerate_displays();
        eprintln!("enumerate_displays took {:?}", started.elapsed());
        for display in &displays {
            eprintln!("{display:?} cg_id={:?}", display_id(display));
        }
        assert!(primary_display(&displays).is_some());
    }
}
