//! Per-output geometry snapshots for the surface host.
//!
//! The 60-240 Hz draw path reads output geometry for cursor routing, motion
//! bounds, per-output density and buffer scale, and the tick cadence. Asking
//! the windowing system for it per frame is never free (SCTK's
//! `OutputState::info()` clones make/model strings and the whole mode list;
//! AppKit allocates a screen array), so each platform snapshots the handful
//! of scalar fields the host consumes into [`OutputGeometry`] and rebuilds the
//! snapshot only when an output event fires. Steady-state frames allocate
//! nothing for geometry.

use serde::Serialize;

/// Representative logical density (~120 logical DPI) used when the output
/// geometry is unknown or degenerate, so physical-unit effects always have a
/// sane real-world scale.
pub(crate) const FALLBACK_PX_PER_MM: f32 = 4.7;

/// The scalar geometry the per-frame paths read for one surface's output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputGeometry {
    /// Desktop-logical origin of the output.
    pub position: (i32, i32),
    /// Desktop-logical size; `None` until the windowing system advertises it.
    pub logical_size: Option<(i32, i32)>,
    /// Logical pixels per physical millimetre (see [`px_per_mm`]).
    pub px_per_mm: f32,
    /// Buffer scale: the surface renders at `logical * render_scale` physical
    /// pixels.
    pub render_scale: f32,
    /// Current refresh rate in mHz, for the tick cadence.
    pub refresh_mhz: Option<i32>,
}

/// The union of the output rects in desktop-logical space.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct PointerTrackingBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_milli: u32,
}

/// Logical pixels per physical millimetre from an output's physical size (mm)
/// and logical size (px). Diagonal-based, so it is robust to per-axis DPI and
/// rotation. [`FALLBACK_PX_PER_MM`] when either size is unknown or degenerate.
pub(crate) fn px_per_mm(physical_mm: (f32, f32), logical_px: (f32, f32)) -> f32 {
    let phys_diag_mm = (physical_mm.0.powi(2) + physical_mm.1.powi(2)).sqrt();
    let logical_diag_px = (logical_px.0.powi(2) + logical_px.1.powi(2)).sqrt();
    if phys_diag_mm < 1.0 || logical_diag_px < 1.0 {
        return FALLBACK_PX_PER_MM;
    }
    logical_diag_px / phys_diag_mm
}

/// Translates a desktop-logical point into an output's local coordinates,
/// **without** clipping to the output rect. Every output receives the full
/// motion scene in its own coordinates and the WGSL clips per-pixel, so a
/// glyph / ripple / trail spanning a monitor boundary renders continuously on
/// both sides. Because logical positions encode the compositor's arrangement,
/// a point on the A|B seam maps to A's right edge and B's left edge — the
/// stroke crosses the seam with no jump. Per-output `render_scale` is applied
/// later in `build_effect_uniform`, so outputs at different scales stay
/// visually continuous.
pub(crate) fn translate_to_output_local(point: (f64, f64), position: (i32, i32)) -> (f64, f64) {
    (
        point.0 - f64::from(position.0),
        point.1 - f64::from(position.1),
    )
}

/// Whether the axis-aligned box `[min, max]` expanded by `margin` intersects
/// the output rect at `position` with `size`. Used to skip outputs a motion
/// element (glyph footprint or gesture scene) cannot reach, so per-frame
/// glyph/effect shader work stays on the 1-2 outputs actually touched.
/// `margin` guarantees an element straddling a seam still reaches the
/// neighbour; over-inclusion is harmless (the shader draws nothing off-bounds)
/// while under-inclusion would clip a visible element, so callers pass a
/// generous margin.
pub(crate) fn box_reaches_output(
    min: (f64, f64),
    max: (f64, f64),
    position: (i32, i32),
    size: (i32, i32),
    margin: f64,
) -> bool {
    if size.0 <= 0 || size.1 <= 0 {
        return false;
    }
    let left = f64::from(position.0);
    let top = f64::from(position.1);
    let right = left + f64::from(size.0);
    let bottom = top + f64::from(size.1);
    (min.0 - margin) < right
        && (max.0 + margin) > left
        && (min.1 - margin) < bottom
        && (max.1 + margin) > top
}

#[cfg(test)]
mod tests {
    use super::{FALLBACK_PX_PER_MM, box_reaches_output, px_per_mm, translate_to_output_local};

    #[test]
    fn px_per_mm_uses_diagonals_and_falls_back_on_degenerate_sizes() {
        // A 3:4:5 triangle on both sides: 500 logical px across 100 mm.
        assert!((px_per_mm((60.0, 80.0), (300.0, 400.0)) - 5.0).abs() < 1e-6);
        assert_eq!(px_per_mm((0.0, 0.0), (300.0, 400.0)), FALLBACK_PX_PER_MM);
        assert_eq!(px_per_mm((60.0, 80.0), (0.0, 0.0)), FALLBACK_PX_PER_MM);
    }

    #[test]
    fn seam_point_maps_continuously_across_two_outputs() {
        // Output A: 1920x1080 at origin; output B at (1920, 0). A point on the
        // shared seam (x = 1920) maps to A's right edge and B's left edge, so
        // a stroke crossing it is seamless.
        assert_eq!(
            translate_to_output_local((1920.0, 90.0), (0, 0)),
            (1920.0, 90.0)
        );
        assert_eq!(
            translate_to_output_local((1920.0, 90.0), (1920, 0)),
            (0.0, 90.0)
        );
        // A negative-origin monitor (left of primary) translates too.
        assert_eq!(
            translate_to_output_local((-100.0, 50.0), (-1920, 0)),
            (1820.0, 50.0)
        );
    }

    #[test]
    fn box_reach_includes_neighbour_within_margin_only() {
        let a = ((0, 0), (1920, 1080));
        let b = ((1920, 0), (1280, 720));
        // A point 40px left of the A|B seam, margin 64: reaches BOTH outputs.
        let p = (1880.0, 400.0);
        assert!(box_reaches_output(p, p, a.0, a.1, 64.0));
        assert!(box_reaches_output(p, p, b.0, b.1, 64.0));
        // The same point with a small margin reaches only A.
        assert!(box_reaches_output(p, p, a.0, a.1, 8.0));
        assert!(!box_reaches_output(p, p, b.0, b.1, 8.0));
        // A far point reaches neither the neighbour nor a degenerate output.
        let far = (200.0, 400.0);
        assert!(!box_reaches_output(far, far, b.0, b.1, 64.0));
        assert!(!box_reaches_output(p, p, (1920, 0), (0, 0), 64.0));
    }
}
