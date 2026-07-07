//! Frame scene description passed from the host to the renderer.
//!
//! The renderer never sees Wayland layer handles or output state. The host
//! normalizes native coordinates into surface-local points and the WGPU shader
//! owns visible cursor/effect animation.

use sky_cua_platform::model::AgentOverlayGestureKind;

/// A point in surface-local pixels where the cursor should be drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorPoint {
    pub x: f64,
    pub y: f64,
}

/// Per-surface draw request.
///
/// `None` means the surface is inactive (closed or not yet configured) and
/// should be skipped. The host builds these in the same order as its surface
/// guards so indices stay aligned.
pub type SurfaceDrawRequest = Option<SurfaceDrawSpec>;

#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceDrawSpec {
    pub width: u32,
    pub height: u32,
    pub cursor: Option<CursorPoint>,
    pub effect: Option<EffectScene>,
    /// True while the agent holds the in-control lease (the host's visible
    /// overlay state). Gates the breathing edge glow and inward waves on this
    /// surface, matching Android's `glowActive`. Distinct from cursor presence.
    pub glow_active: bool,
    /// Integer buffer scale for this output (`ceil(native / logical)`): the
    /// surface renders at `width*render_scale × height*render_scale` physical
    /// pixels so the compositor downsamples a sharp buffer instead of upscaling
    /// a soft logical one. All surface-local coordinates and the cursor footprint
    /// are scaled by this when building the frame uniform. `1.0` = no scaling
    /// (an unscaled output).
    pub render_scale: f32,
    /// Surface-local (logical) pixels per physical millimetre for this output,
    /// derived from its `wl_output` physical size and logical size. Lets the
    /// edge glow size its bright rim and containment band in real-world units
    /// (millimetres/centimetres) so the effect looks identical across monitors
    /// with different DPI. Falls back to a representative logical density when
    /// the output geometry is unknown.
    pub px_per_mm: f32,
    /// CPU-eased glyph rotation in degrees (the motion driver's easing of the
    /// travel heading). The shader adds the no-no wiggle on top; 0.0 draws the
    /// glyph as authored.
    pub cursor_rotation_deg: f32,
    /// Smoke-aura master alpha in `[0, 1]`: the motion driver's cloud bloom.
    /// 1.0 is full presence (the pre-motion behavior).
    pub cursor_cloud_alpha: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectScene {
    pub kind: AgentOverlayGestureKind,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub points: Vec<CursorPoint>,
}

impl EffectScene {
    #[must_use]
    #[allow(dead_code)] // only reachable via the test-only expiry check below
    pub fn is_active_at(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.started_at_ms) <= self.duration_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_scene_expires_after_duration() {
        let effect = EffectScene {
            kind: AgentOverlayGestureKind::Tap,
            started_at_ms: 100,
            duration_ms: 250,
            points: vec![CursorPoint { x: 1.0, y: 2.0 }],
        };

        assert!(effect.is_active_at(350));
        assert!(!effect.is_active_at(351));
    }
}
