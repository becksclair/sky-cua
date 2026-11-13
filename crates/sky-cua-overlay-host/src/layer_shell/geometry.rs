//! Wayland output geometry: builds the host's [`OutputGeometry`] snapshot from
//! SCTK's `OutputInfo`.
//!
//! `OutputState::info()` clones the full `OutputInfo` (make/model strings and
//! the whole mode list) on every call, so this conversion runs only when an
//! output event fires. Per-frame readers use the snapshot.

use smithay_client_toolkit::output::OutputInfo;

use crate::surface_host::geometry::{FALLBACK_PX_PER_MM, OutputGeometry, px_per_mm};

pub(super) fn output_geometry(info: &OutputInfo) -> OutputGeometry {
    OutputGeometry {
        // `logical_position`, falling back to the wl_output location.
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

fn px_per_mm_from_info(info: &OutputInfo) -> f32 {
    let Some((logical_w, logical_h)) = info.logical_size else {
        return FALLBACK_PX_PER_MM;
    };
    let (phys_w_mm, phys_h_mm) = info.physical_size;
    px_per_mm(
        (phys_w_mm as f32, phys_h_mm as f32),
        (logical_w as f32, logical_h as f32),
    )
}
