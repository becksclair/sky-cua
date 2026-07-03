//! Output-geometry and motion-scene mapping for the layer-shell host.
//!
//! The pure adapter between the motion driver's frame (desktop-logical or
//! stream-space poses, feedback scenes) and per-layer renderer inputs. On a
//! multi-output desktop it treats every output as a window into the shared
//! desktop-logical scene: the glyph, ripple, and trail are handed to every
//! output they reach, each translated (unclipped) into that output's local
//! coordinates, and the WGSL clips per-pixel — so a boundary-spanning glide or
//! gesture renders continuously across the arrangement, each output applying
//! its own `render_scale`. There is no "route to the containing output" step
//! and thus no edge-pop when a stroke crosses a seam. Everything here reads
//! only `layers` + the [`geometry::OutputGeometry`] snapshot — no Wayland
//! dispatch, no protocol state — and runs on the 60-240 Hz draw path, so it
//! must never call SCTK's cloning `OutputState::info()` (see `geometry`).

use super::*;

/// Reach margin around the glyph hotspot in logical px: the cursor footprint
/// (arrow + smoke margin) so the aura is handed to a neighbouring output as
/// the glyph nears a seam and never clips there.
fn cursor_reach_margin() -> f64 {
    f64::from(
        cursor_asset::AGENT_CURSOR_FOOTPRINT_WIDTH.max(cursor_asset::AGENT_CURSOR_FOOTPRINT_HEIGHT),
    )
}

/// Reach margin around a gesture scene in logical px: the ripple is the widest
/// element (max radius + ring stroke); the trail stroke is far smaller.
fn scene_reach_margin() -> f64 {
    sky_cua_platform::overlay_spec::desktop::geometry::RIPPLE_MAX_LOGICAL_PX
        + sky_cua_platform::overlay_spec::desktop::geometry::RIPPLE_STROKE_LOGICAL_PX
}

fn point_bbox(points: &[MotionPoint]) -> ((f64, f64), (f64, f64)) {
    let mut min = (f64::MAX, f64::MAX);
    let mut max = (f64::MIN, f64::MIN);
    for p in points {
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        min.0 = min.0.min(x);
        min.1 = min.1.min(y);
        max.0 = max.0.max(x);
        max.1 = max.1.max(y);
    }
    (min, max)
}

impl LayerShellApp {
    /// Rebuilds the per-layer output geometry snapshot. Called from output
    /// events only (plus once at connect); per-frame readers use
    /// [`Self::layer_geometry`] so the draw path never pays SCTK's
    /// `OutputInfo` clone.
    pub(super) fn refresh_output_geometry(&mut self) {
        self.output_geometry = self
            .layers
            .iter()
            .map(|entry| {
                entry
                    .output
                    .as_ref()
                    .and_then(|output| self.output_state.info(output))
                    .map(|info| OutputGeometry::from_info(&info))
            })
            .collect();
    }
    pub(super) fn layer_geometry(&self, index: usize) -> Option<&OutputGeometry> {
        self.output_geometry.get(index).and_then(|g| g.as_ref())
    }
    pub(super) fn pointer_tracking_bounds(&self) -> Option<PointerTrackingBounds> {
        let mut left = i32::MAX;
        let mut top = i32::MAX;
        let mut right = i32::MIN;
        let mut bottom = i32::MIN;
        for (index, entry) in self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.closed && entry.configured)
        {
            if entry.output.is_none() {
                left = left.min(0);
                top = top.min(0);
                right = right.max(i32::try_from(entry.width).unwrap_or(i32::MAX));
                bottom = bottom.max(i32::try_from(entry.height).unwrap_or(i32::MAX));
                continue;
            }
            let Some(geometry) = self.layer_geometry(index) else {
                continue;
            };
            let Some(size) = geometry.logical_size else {
                continue;
            };
            let position = geometry.position;
            left = left.min(position.0);
            top = top.min(position.1);
            right = right.max(position.0.saturating_add(size.0));
            bottom = bottom.max(position.1.saturating_add(size.1));
        }
        if right <= left || bottom <= top {
            return None;
        }
        Some(PointerTrackingBounds {
            x: left,
            y: top,
            width: u32::try_from(right - left).ok()?,
            height: u32::try_from(bottom - top).ok()?,
            scale_milli: 1000,
        })
    }
    /// Bounds for the motion driver in the space it will resolve targets in:
    /// the union of output logical rects for desktop-logical space, or the
    /// first open layer's rect for stream spaces / unknown geometry. Clamping
    /// targets into these bounds is what guarantees the arrival gate always
    /// fires (an off-screen target would otherwise never settle).
    pub(super) fn motion_bounds(&self, space: Option<&CoordinateSpace>) -> MotionBounds {
        if space == Some(&CoordinateSpace::DesktopLogical)
            && let Some(bounds) = self.pointer_tracking_bounds()
        {
            return MotionBounds {
                min_x: bounds.x as f32,
                min_y: bounds.y as f32,
                max_x: bounds.x as f32 + bounds.width as f32,
                max_y: bounds.y as f32 + bounds.height as f32,
            };
        }
        let (width, height) = self
            .first_open_layer_index()
            .and_then(|index| self.layers.get(index))
            .map(|entry| (entry.width.max(1) as f32, entry.height.max(1) as f32))
            .unwrap_or((f32::MAX, f32::MAX));
        MotionBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: width,
            max_y: height,
        }
    }
    /// Whether any configured layer has placed output geometry (the normal
    /// multi-output config). `false` for the output-less single-surface
    /// fallback, which routes desktop-logical points raw to the sole layer.
    fn has_output_geometry(&self) -> bool {
        self.output_geometry
            .iter()
            .flatten()
            .any(|g| g.logical_size.is_some())
    }
    /// The glyph position for one open layer, or `None` if the cursor footprint
    /// does not reach this output. A desktop-logical pose is translated
    /// (unclipped) into every output the footprint overlaps, so the glyph and
    /// its smoke aura render continuously across a seam; stream-space or
    /// output-less poses land raw on the first open layer.
    pub(super) fn cursor_for_layer(
        &self,
        index: usize,
        pos: MotionPoint,
        space: Option<&CoordinateSpace>,
    ) -> Option<CursorPoint> {
        let (x, y) = (f64::from(pos.x), f64::from(pos.y));
        if space == Some(&CoordinateSpace::DesktopLogical) && self.has_output_geometry() {
            let geometry = self.layer_geometry(index)?;
            let size = geometry.logical_size?;
            if !box_reaches_output(
                (x, y),
                (x, y),
                geometry.position,
                size,
                cursor_reach_margin(),
            ) {
                return None;
            }
            let (lx, ly) = translate_to_output_local((x, y), geometry.position);
            return Some(CursorPoint { x: lx, y: ly });
        }
        (self.first_open_layer_index() == Some(index)).then_some(CursorPoint { x, y })
    }
    /// The gesture feedback scene for one open layer. For a desktop-logical
    /// gesture, the ripple center (taps/no-no) or resampled trail (slides) is
    /// translated (unclipped) into this output's coordinates when the scene
    /// reaches it, so a boundary-spanning ripple/trail renders on every output
    /// it crosses; stream-space scenes land raw on the first open layer.
    /// `started_at_ms` is the ARRIVAL time, so the shader's ripple/squash
    /// timelines begin when the glyph lands.
    pub(super) fn feedback_scene_for_layer(
        &self,
        index: usize,
        frame: &MotionFrame,
    ) -> Option<EffectScene> {
        let feedback = frame.feedback.as_ref()?;
        let raw = feedback.scene_points();
        if raw.is_empty() {
            return None;
        }
        let space = frame
            .space
            .clone()
            .unwrap_or(CoordinateSpace::DesktopLogical);

        let points: Vec<CursorPoint> =
            if space == CoordinateSpace::DesktopLogical && self.has_output_geometry() {
                let geometry = self.layer_geometry(index)?;
                let size = geometry.logical_size?;
                let (min, max) = point_bbox(raw);
                if !box_reaches_output(min, max, geometry.position, size, scene_reach_margin()) {
                    return None;
                }
                raw.iter()
                    .map(|p| {
                        let (x, y) = translate_to_output_local(
                            (f64::from(p.x), f64::from(p.y)),
                            geometry.position,
                        );
                        CursorPoint { x, y }
                    })
                    .collect()
            } else if self.first_open_layer_index() == Some(index) {
                raw.iter()
                    .map(|p| CursorPoint {
                        x: f64::from(p.x),
                        y: f64::from(p.y),
                    })
                    .collect()
            } else {
                return None;
            };

        Some(EffectScene {
            kind: feedback.kind,
            started_at_ms: feedback.started_at_ms,
            duration_ms: feedback.duration_ms,
            points,
        })
    }
    /// Logical pixels per physical millimetre for a layer's output, from the
    /// geometry snapshot (diagonal-based; see `geometry::px_per_mm_from_info`).
    pub(super) fn layer_px_per_mm(&self, index: usize) -> f32 {
        self.layer_geometry(index)
            .map(|geometry| geometry.px_per_mm)
            .unwrap_or(FALLBACK_PX_PER_MM)
    }
    /// Integer buffer scale for a layer's output (see [`output_render_scale`]).
    pub(super) fn layer_render_scale(&self, index: usize) -> f32 {
        self.layer_geometry(index)
            .map(|geometry| geometry.render_scale)
            .unwrap_or(1.0)
    }
}
