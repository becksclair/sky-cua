//! Frame scene description passed from the host to the renderer.
//!
//! The scene is intentionally minimal for Phase 3: a list of per-surface draw
//! requests, each carrying only geometry and an optional cursor point. The
//! renderer never sees Wayland layer handles or output state.

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceDrawSpec {
    pub width: u32,
    pub height: u32,
    pub cursor: Option<CursorPoint>,
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
