//! Screen enumeration, the per-screen geometry snapshot, and the window-list
//! query behind the capture barrier.
//!
//! Scene geometry is in `desktop_logical` (global points, top-left origin),
//! which is what `CGDisplayBounds` reports. The AppKit frame is carried only
//! so a panel can be placed on its screen.

use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_core_foundation::{CGRect, CGSize};
use objc2_core_graphics::{
    CGDisplayBounds, CGDisplayScreenSize, CGWindowListCreate, CGWindowListOption, kCGNullWindowID,
};
use objc2_foundation::{NSNumber, NSRect, ns_string};

use crate::surface_host::geometry::{OutputGeometry, px_per_mm};

pub(super) struct ScreenInfo {
    /// The screen's frame in AppKit coordinates, for panel placement only.
    pub frame: NSRect,
    pub geometry: OutputGeometry,
}

/// One entry per attached screen, or `None` if any screen's display
/// identifier cannot be read: its global bounds would be unknown, and an
/// overlay that silently left a screen out would still report full coverage.
pub(super) fn screens(mtm: MainThreadMarker) -> Option<Vec<ScreenInfo>> {
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let display_id = screen
                .deviceDescription()
                .objectForKey(ns_string!("NSScreenNumber"))?
                .downcast::<NSNumber>()
                .ok()?
                .unsignedIntValue();
            Some(ScreenInfo {
                frame: screen.frame(),
                geometry: display_geometry(
                    CGDisplayBounds(display_id),
                    CGDisplayScreenSize(display_id),
                    screen.backingScaleFactor(),
                    screen.maximumFramesPerSecond(),
                ),
            })
        })
        .collect()
}

/// Builds the host's geometry snapshot for one display from its global bounds
/// (points), physical size (millimetres), backing scale, and maximum refresh
/// rate (Hz).
fn display_geometry(
    bounds: CGRect,
    physical_mm: CGSize,
    backing_scale: f64,
    max_frames_per_second: isize,
) -> OutputGeometry {
    let width = bounds.size.width.round() as i32;
    let height = bounds.size.height.round() as i32;
    OutputGeometry {
        position: (
            bounds.origin.x.round() as i32,
            bounds.origin.y.round() as i32,
        ),
        logical_size: (width > 0 && height > 0).then_some((width, height)),
        px_per_mm: px_per_mm(
            (physical_mm.width as f32, physical_mm.height as f32),
            (bounds.size.width as f32, bounds.size.height as f32),
        ),
        // The Metal layer renders at the backing scale, so the drawable is
        // exactly the screen's pixel grid.
        render_scale: if backing_scale.is_finite() && backing_scale >= 1.0 {
            backing_scale as f32
        } else {
            1.0
        },
        refresh_mhz: i32::try_from(max_frames_per_second)
            .ok()
            .filter(|hz| *hz > 0)
            .map(|hz| hz.saturating_mul(1000)),
    }
}

/// How many of `window_ids` the window server lists as on screen. `None`
/// when the window list is unavailable.
pub(super) fn windows_on_screen(window_ids: &[u32]) -> Option<usize> {
    if window_ids.is_empty() {
        return Some(0);
    }
    let on_screen = CGWindowListCreate(CGWindowListOption::OptionOnScreenOnly, kCGNullWindowID)?;
    let listed = (0..on_screen.count())
        .filter(|index| {
            // SAFETY: the index is within the array, whose values are window
            // identifiers stored directly in the pointer-sized slots.
            let value = unsafe { on_screen.value_at_index(*index) } as usize;
            u32::try_from(value).is_ok_and(|window_id| window_ids.contains(&window_id))
        })
        .count();
    Some(listed)
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use super::display_geometry;
    use crate::surface_host::geometry::FALLBACK_PX_PER_MM;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(width, height))
    }

    #[test]
    fn display_geometry_snapshots_bounds_scale_density_and_refresh() {
        // A 14-inch built-in Retina panel: 1512 x 982 points at 2x, 120 Hz.
        let geometry = display_geometry(
            rect(0.0, 0.0, 1512.0, 982.0),
            CGSize::new(302.0, 196.0),
            2.0,
            120,
        );
        assert_eq!(geometry.position, (0, 0));
        assert_eq!(geometry.logical_size, Some((1512, 982)));
        assert_eq!(geometry.render_scale, 2.0);
        assert_eq!(geometry.refresh_mhz, Some(120_000));
        assert!((geometry.px_per_mm - 5.0).abs() < 0.05);
    }

    #[test]
    fn display_geometry_keeps_negative_origins_and_falls_back_on_unknown_values() {
        // A display arranged left of and above the main one, reporting no
        // physical size, a zero refresh rate, and an invalid backing scale.
        let geometry = display_geometry(
            rect(-1920.0, -1080.0, 1920.0, 1080.0),
            CGSize::new(0.0, 0.0),
            0.0,
            0,
        );
        assert_eq!(geometry.position, (-1920, -1080));
        assert_eq!(geometry.logical_size, Some((1920, 1080)));
        assert_eq!(geometry.render_scale, 1.0);
        assert_eq!(geometry.refresh_mhz, None);
        assert_eq!(geometry.px_per_mm, FALLBACK_PX_PER_MM);

        let empty = display_geometry(rect(0.0, 0.0, 0.0, 0.0), CGSize::new(0.0, 0.0), 2.0, 60);
        assert_eq!(empty.logical_size, None);
    }
}
