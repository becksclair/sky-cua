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
}

/// A full frame: one request per host surface guard.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameScene {
    pub surfaces: Vec<SurfaceDrawRequest>,
}

impl FrameScene {
    #[must_use]
    pub fn new(surface_count: usize) -> Self {
        Self {
            surfaces: vec![None; surface_count],
        }
    }
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
