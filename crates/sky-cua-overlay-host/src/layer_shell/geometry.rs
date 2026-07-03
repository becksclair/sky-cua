//! Per-output geometry snapshots for the layer-shell host.
//!
//! SCTK's `OutputState::info()` clones the full `OutputInfo` — make/model
//! `String`s and the whole mode list — on every call. The 60–240 Hz draw path
//! reads output geometry for cursor routing, motion bounds, per-output density
//! and buffer scale, and the tick timer re-derives its cadence from the mode
//! list on every fire. This module snapshots the handful of scalar fields the
//! host actually consumes into [`OutputGeometry`], rebuilt only when an output
//! event fires, so steady-state frames allocate nothing for geometry.

use smithay_client_toolkit::output::OutputInfo;

/// Representative logical density (~120 logical DPI) used when the output
/// geometry is unknown or degenerate, so physical-unit effects always have a
/// sane real-world scale.
pub(crate) const FALLBACK_PX_PER_MM: f32 = 4.7;

/// The scalar geometry the per-frame paths read for one layer's output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OutputGeometry {
    /// Desktop-logical origin (`logical_position`, falling back to the
    /// wl_output location).
    pub position: (i32, i32),
    /// Desktop-logical size; `None` until the compositor advertises it.
    pub logical_size: Option<(i32, i32)>,
    /// Logical pixels per physical millimetre (diagonal-based, robust to
    /// per-axis DPI and rotation); [`FALLBACK_PX_PER_MM`] when unknown.
    pub px_per_mm: f32,
    /// Integer buffer scale (see [`output_render_scale`]).
    pub render_scale: f32,
    /// Current mode refresh in mHz, for the tick cadence.
    pub refresh_mhz: Option<i32>,
}

impl OutputGeometry {
    pub(crate) fn from_info(info: &OutputInfo) -> Self {
        Self {
            position: info.logical_position.unwrap_or(info.location),
            logical_size: info.logical_size,
            px_per_mm: px_per_mm_from_info(info),
            render_scale: output_render_scale(info),
            refresh_mhz: info
                .modes
                .iter()
                .find(|mode| mode.current && mode.refresh_rate > 0)
                .map(|mode| mode.refresh_rate),
        }
    }
}

/// Integer buffer scale for an output: `ceil(native_mode / logical_size)`,
/// clamped to `>= 1`. Rendering the surface at `logical * this` physical pixels
/// makes the compositor downsample a sharp buffer instead of upscaling a soft
/// logical one on hidpi / fractionally-scaled outputs. `1.0` when geometry is
/// unknown or the output is unscaled.
pub(crate) fn output_render_scale(info: &OutputInfo) -> f32 {
    let Some((logical_w, logical_h)) = info.logical_size else {
        return 1.0;
    };
    let Some(mode) = info
        .modes
        .iter()
        .find(|mode| mode.current)
        .or(info.modes.first())
    else {
        return 1.0;
    };
    let (native_w, native_h) = mode.dimensions;
    if logical_w <= 0 || logical_h <= 0 || native_w <= 0 || native_h <= 0 {
        return 1.0;
    }
    let scale_x = native_w as f32 / logical_w as f32;
    let scale_y = native_h as f32 / logical_h as f32;
    scale_x.max(scale_y).ceil().max(1.0)
}

/// Logical pixels per physical millimetre from the output's physical size (mm)
/// and logical size (px), diagonal-based.
fn px_per_mm_from_info(info: &OutputInfo) -> f32 {
    let (phys_w_mm, phys_h_mm) = info.physical_size;
    let Some((logical_w, logical_h)) = info.logical_size else {
        return FALLBACK_PX_PER_MM;
    };
    let phys_diag_mm = ((phys_w_mm as f32).powi(2) + (phys_h_mm as f32).powi(2)).sqrt();
    let logical_diag_px = ((logical_w as f32).powi(2) + (logical_h as f32).powi(2)).sqrt();
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
    use super::{box_reaches_output, translate_to_output_local};

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
