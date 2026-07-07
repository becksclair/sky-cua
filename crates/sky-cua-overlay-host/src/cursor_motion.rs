//! The stateful cursor-motion driver: the desktop port of the Android
//! companion's ambient overlay loop (`AgentOverlayController`, ambient frame
//! :454-543, gesture pipeline :556-634, capture hide/restore :248-302).
//!
//! The driver owns the drawn cursor pose between frames. Everything upstream —
//! `SetCursor` repositioning, gesture events, compositor pointer telemetry —
//! only moves a *target*; the [`Mover2D`] vehicle does the sailing, so a
//! redirect mid-flight keeps its momentum and bends the path. Gesture visual
//! feedback (ripple, press squash, trail) is arrival-gated: it starts when the
//! mover settles at the gesture point, not when the event arrives.
//!
//! Pure state machine: no Wayland/wgpu types, stepped with an explicit
//! monotonic instant + epoch milliseconds so tests and the offline capture
//! harness can drive it deterministically. The mover's dt must come from the
//! monotonic clock and effect timelines from epoch milliseconds (the renderer
//! compares them against its own epoch clock); mixing the two would let an NTP
//! step fling the cursor or stall a feedback.

use std::time::Instant;

use sky_cua_platform::model::{AgentOverlayGestureKind, CoordinateSpace};
use sky_cua_platform::overlay_spec::desktop::geometry;
use sky_cua_platform::overlay_spec::shared::motion;

use crate::motion::{
    MotionPoint, Mover2D, MoverParams, approach_angle_deg, distance, point_at_progress,
    sample_trail,
};

/// Seconds for the cursor smoke aura to bloom in after a cold show. Mirrors
/// the Android controller-local `CURSOR_CLOUD_FADE_S` (a feel constant kept
/// out of the shared spec on both sides).
pub const CURSOR_CLOUD_FADE_S: f32 = 0.8;

/// A gesture handed to the driver for visual feedback. Coordinates are in the
/// gesture's own space; the driver aims the mover at `points[0]` and starts
/// the feedback timeline on arrival.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionGesture {
    pub kind: AgentOverlayGestureKind,
    pub points: Vec<MotionPoint>,
    pub space: CoordinateSpace,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
struct ActiveGesture {
    gesture: MotionGesture,
    /// Epoch ms when the mover arrived and the feedback timeline started.
    started_at_ms: u64,
    /// The clamped arrival point the ripple centers on.
    arrival: MotionPoint,
}

/// Rect the mover may occupy, in the current target's coordinate space.
/// Desktop-global logical space can have negative origins (a monitor left of
/// primary), hence a full rect instead of a size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionBounds {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl MotionBounds {
    #[must_use]
    pub fn clamp(&self, p: MotionPoint) -> MotionPoint {
        MotionPoint {
            x: p.x.clamp(self.min_x, self.max_x),
            y: p.y.clamp(self.min_y, self.max_y),
        }
    }
}

/// Per-frame input to [`CursorMotionDriver::step`].
#[derive(Debug, Clone)]
pub struct MotionStepInput {
    /// Monotonic clock for the mover's dt.
    pub now: Instant,
    /// Epoch ms for feedback timelines (matches the renderer's clock).
    pub now_ms: u64,
    /// Whether the overlay currently draws. Hidden frames freeze the driver.
    pub visible: bool,
    /// Latest target from cursor state (x, y, space), if any.
    pub target: Option<(f64, f64, CoordinateSpace)>,
    /// Bounds of the target's space (union of outputs for desktop-logical,
    /// the layer size for stream spaces).
    pub bounds: MotionBounds,
}

/// The visual feedback of an arrived gesture, rebuilt every frame while it
/// plays. The shader owns the timeline math; this carries the arrival-based
/// start time, the ripple center, and the resampled trail.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackFrame {
    pub kind: AgentOverlayGestureKind,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub ripple_center: MotionPoint,
    /// Arc-length-even resample of the ideal polyline start..head; empty for
    /// taps and no-no.
    pub trail_samples: Vec<MotionPoint>,
}

impl FeedbackFrame {
    /// The points an effect scene draws: the resampled trail for slides, the
    /// ripple center for taps and no-no. The single source of that selection
    /// rule — the production host, the playground, and the offline capture
    /// harness all consume it, so they cannot drift apart.
    #[must_use]
    pub fn scene_points(&self) -> &[MotionPoint] {
        if self.trail_samples.is_empty() {
            std::slice::from_ref(&self.ripple_center)
        } else {
            &self.trail_samples
        }
    }
}

/// The driver's output for one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionFrame {
    /// Drawn cursor position, `None` until the mover has ever been placed.
    pub pos: Option<MotionPoint>,
    /// Space `pos` lives in.
    pub space: Option<CoordinateSpace>,
    /// CPU-eased glyph rotation in degrees (0 = glyph as authored).
    pub rotation_deg: f32,
    /// Smoke-aura master alpha in `[0, 1]` (blooms in over
    /// [`CURSOR_CLOUD_FADE_S`] after a cold show).
    pub cloud_alpha: f32,
    /// Active gesture feedback, once the arrival gate has fired.
    pub feedback: Option<FeedbackFrame>,
    /// Whether the driver needs further frames (unsettled mover, pending or
    /// active gesture, or a mid-ramp cloud).
    pub animating: bool,
    /// Whether the mover is parked on its most recent target.
    pub settled: bool,
    /// Mover heading in degrees, for the structured motion echo.
    pub heading_deg: f32,
    /// Mover speed in logical px/s, for the structured motion echo.
    pub speed: f32,
}

/// See the module docs. One instance lives on the layer-shell app (and one on
/// the playground demo app); [`CursorMotionDriver::step`] runs exactly once
/// per rendered frame.
#[derive(Debug)]
pub struct CursorMotionDriver {
    mover: Mover2D,
    rotation_deg: f32,
    cloud_alpha: f32,
    last_step: Option<Instant>,
    space: Option<CoordinateSpace>,
    /// Next visible frame re-blooms the cloud from zero.
    cold: bool,
    was_visible: bool,
    /// Set by a capture hide; the next visible frame skips the cold-show
    /// cloud reset (capture hide/restore must be visually seamless).
    capture_suspended: bool,
    pending: Option<MotionGesture>,
    active: Option<ActiveGesture>,
    /// The last clamped aim point, for settled-ness.
    last_aim: Option<MotionPoint>,
    /// Epoch ms of the last visible step; feedback trail sampling reads it.
    now_ms: u64,
    /// Where a retired gesture parked the cursor. Until a state target that
    /// differs from the gesture's stale start point arrives (the service's
    /// post-dispatch update or pointer telemetry), the driver keeps aiming at
    /// the parked head — the phone leaves its cursor target at the gesture
    /// head when the animator ends, and without this the glyph would sail
    /// back toward the pre-dispatch origin while the drag's end-state is
    /// still in flight.
    rest_aim: Option<RestAim>,
}

/// See [`CursorMotionDriver::rest_aim`]. Carries its own coordinate space:
/// the takeover check must never distance-compare across spaces, and the
/// host's bounds handshake ([`CursorMotionDriver::upcoming_space`]) must
/// report the space this aim actually lives in.
#[derive(Debug, Clone)]
struct RestAim {
    parked: MotionPoint,
    /// The gesture's start point — the pre-dispatch state target that must
    /// not reclaim the aim.
    stale_origin: MotionPoint,
    space: CoordinateSpace,
}

impl Default for CursorMotionDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl CursorMotionDriver {
    #[must_use]
    pub fn new() -> Self {
        Self {
            mover: Mover2D::new(MoverParams::from_spec()),
            rotation_deg: 0.0,
            cloud_alpha: 0.0,
            last_step: None,
            space: None,
            cold: true,
            was_visible: false,
            capture_suspended: false,
            pending: None,
            active: None,
            last_aim: None,
            now_ms: 0,
            rest_aim: None,
        }
    }

    /// Queues a gesture for arrival-gated feedback. Cancels any active
    /// feedback and replaces a pending one; the mover is deliberately not
    /// touched, so a redirect mid-flight keeps its momentum (the signature
    /// curve).
    pub fn start_gesture(&mut self, gesture: MotionGesture) {
        if let Some(active) = self.active.take()
            && active.gesture.kind == AgentOverlayGestureKind::NoNo
        {
            // The wiggle owned the rotation; resume ambient easing from
            // the rest pose, as the phone does when the animator ends.
            self.rotation_deg = 0.0;
        }
        self.rest_aim = None;
        self.pending = if gesture.points.is_empty() {
            None
        } else {
            Some(gesture)
        };
    }

    /// Hides the cursor. A capture hide freezes everything (mover, pending
    /// gesture, cloud) so the restore resumes seamlessly; a plain hide drops
    /// the gesture pipeline and marks the next show cold, re-blooming the
    /// cloud. The in-flight feedback tail is dropped either way (phone
    /// parity: `hideForCapture` cancels the running animator).
    pub fn hide(&mut self, capture: bool) {
        if let Some(active) = self.active.take()
            && active.gesture.kind == AgentOverlayGestureKind::NoNo
        {
            self.rotation_deg = 0.0;
        }
        if capture {
            self.capture_suspended = true;
        } else {
            self.pending = None;
            self.cold = true;
            self.rest_aim = None;
            // A plain hide supersedes any outstanding capture freeze; without
            // this, a capture hide whose restore never arrives would eat the
            // next cold show's cloud re-bloom.
            self.capture_suspended = false;
        }
    }

    /// Advances the driver by one frame. Call exactly once per rendered
    /// frame; hidden frames freeze all state and reset the dt baseline so
    /// hidden wall time never integrates into the glide.
    pub fn step(&mut self, input: MotionStepInput) -> MotionFrame {
        if !input.visible {
            // An implicit hide — `SetCursor { visible: false }` or a `Show`
            // without state replaces the stored state without the host ever
            // calling `hide()` — must behave like a plain hide: drop the
            // gesture pipeline and mark the next show cold. Capture hides are
            // exempted by the flag `hide(true)` set before this frame.
            if !self.capture_suspended {
                self.hide(false);
            }
            self.was_visible = false;
            self.last_step = None;
            return self.frame(false);
        }

        let dt = self
            .last_step
            .map(|t| input.now.duration_since(t).as_secs_f32())
            .unwrap_or(0.0);
        self.last_step = Some(input.now);
        self.now_ms = input.now_ms;

        if !self.was_visible {
            self.was_visible = true;
            if self.capture_suspended {
                self.capture_suspended = false;
            } else if self.cold {
                self.cloud_alpha = 0.0;
                self.cold = false;
            }
        }

        // A state target that moved away from the retired gesture's stale
        // start point (the service's post-dispatch update, a new action, or
        // pointer telemetry) takes over from the parked rest aim. A target in
        // a different coordinate space always takes over — heterogeneous
        // spaces cannot be distance-compared.
        if let (Some(rest), Some((x, y, space))) = (&self.rest_aim, &input.target) {
            let takes_over = *space != rest.space
                || distance(
                    MotionPoint {
                        x: *x as f32,
                        y: *y as f32,
                    },
                    rest.stale_origin,
                ) > motion::CURSOR_SETTLE_PX as f32;
            if takes_over {
                self.rest_aim = None;
            }
        }

        // Resolve the aim by fixed precedence: the moving head of an active
        // slide, then a pending gesture's start point, then a retired
        // gesture's parked head, then the latest cursor state. Clamped into
        // bounds before steering — an unreachable target would park the mover
        // at the border with the arrival gate open forever (deliberately
        // stricter than the phone).
        let resolved = self.resolve_target(input.now_ms, input.target);
        if let Some((raw, target_space)) = resolved {
            if self.space.as_ref() != Some(&target_space) {
                // Heterogeneous coordinate spaces cannot be interpolated:
                // adopt the new space with a snap, exactly like a first show.
                self.space = Some(target_space);
                let aim = input.bounds.clamp(raw);
                self.mover.set_bounds(
                    input.bounds.min_x,
                    input.bounds.min_y,
                    input.bounds.max_x,
                    input.bounds.max_y,
                );
                self.mover.snap_to(aim.x, aim.y);
                self.last_aim = Some(aim);
            } else {
                let aim = input.bounds.clamp(raw);
                self.mover.set_bounds(
                    input.bounds.min_x,
                    input.bounds.min_y,
                    input.bounds.max_x,
                    input.bounds.max_y,
                );
                self.mover.step(aim.x, aim.y, dt);
                self.last_aim = Some(aim);
            }
        }
        // No target at all: the mover holds position; rotation and cloud
        // still ease below.

        // Smoke aura blooms toward full presence (phone Controller:480-488).
        if CURSOR_CLOUD_FADE_S > 0.0 {
            self.cloud_alpha = (self.cloud_alpha + dt / CURSOR_CLOUD_FADE_S).min(1.0);
        }

        // Arrival gate: promote the pending gesture once the mover has parked
        // on its (clamped) start point (phone Controller:527-534). The
        // feedback clock starts at arrival, not receipt.
        if let Some(pending) = self.pending.take() {
            let aim = input.bounds.clamp(pending.points[0]);
            let dist = distance(
                aim,
                MotionPoint {
                    x: self.mover.x(),
                    y: self.mover.y(),
                },
            );
            if self.mover.speed() <= 0.0 && dist <= geometry::GESTURE_ARRIVE_LOGICAL_PX as f32 {
                self.active = Some(ActiveGesture {
                    gesture: pending,
                    started_at_ms: input.now_ms,
                    arrival: aim,
                });
            } else {
                self.pending = Some(pending);
            }
        }

        // Retire a finished feedback before building the frame. The parked
        // head becomes the rest aim (see `rest_aim`) so the glyph holds the
        // gesture's landing point instead of sailing back toward a stale
        // pre-dispatch state target.
        if let Some(active) = &self.active
            && input.now_ms.saturating_sub(active.started_at_ms) >= active.gesture.duration_ms
        {
            let was_no_no = active.gesture.kind == AgentOverlayGestureKind::NoNo;
            let parked = if is_slide(&active.gesture) {
                *active.gesture.points.last().unwrap_or(&active.arrival)
            } else {
                active.arrival
            };
            let stale_origin = active
                .gesture
                .points
                .first()
                .copied()
                .unwrap_or(active.arrival);
            self.rest_aim = Some(RestAim {
                parked: input.bounds.clamp(parked),
                stale_origin,
                space: active.gesture.space.clone(),
            });
            self.active = None;
            if was_no_no {
                self.rotation_deg = 0.0;
            }
        }

        // Glyph rotation eases toward the travel heading above the minimum
        // speed and back to rest below it (phone Controller:504-523). The
        // no-no wiggle owns the rotation outright while it plays; the shader
        // adds the waveform on top of a zero base.
        let no_no_active = self
            .active
            .as_ref()
            .is_some_and(|a| a.gesture.kind == AgentOverlayGestureKind::NoNo);
        if no_no_active {
            self.rotation_deg = 0.0;
        } else {
            let target_rot = if self.mover.speed() > motion::CURSOR_ROTATE_MIN_SPEED_DP_PER_S as f32
            {
                (f64::from(self.mover.heading_rad()).to_degrees() as f32)
                    - motion::CURSOR_NOSE_DEG as f32
            } else {
                0.0
            };
            self.rotation_deg = approach_angle_deg(
                self.rotation_deg,
                target_rot,
                motion::CURSOR_ROTATE_RATE_DEG_PER_S as f32 * dt,
            );
        }

        self.frame(true)
    }

    /// The moving head an active slide chases, or the pending/state target.
    fn resolve_target(
        &self,
        now_ms: u64,
        state_target: Option<(f64, f64, CoordinateSpace)>,
    ) -> Option<(MotionPoint, CoordinateSpace)> {
        if let Some(active) = &self.active {
            if is_slide(&active.gesture) {
                let progress = feedback_progress(active, now_ms);
                let head = point_at_progress(&active.gesture.points, progress);
                return Some((head, active.gesture.space.clone()));
            }
            return Some((active.arrival, active.gesture.space.clone()));
        }
        if let Some(pending) = &self.pending {
            return Some((pending.points[0], pending.space.clone()));
        }
        if let Some(rest) = &self.rest_aim {
            return Some((rest.parked, rest.space.clone()));
        }
        state_target.map(|(x, y, space)| {
            (
                MotionPoint {
                    x: x as f32,
                    y: y as f32,
                },
                space,
            )
        })
    }

    fn frame(&self, visible: bool) -> MotionFrame {
        let pos = self.mover.is_initialized().then(|| MotionPoint {
            x: self.mover.x(),
            y: self.mover.y(),
        });
        let settled = self.mover.is_initialized()
            && self.mover.speed() <= 0.0
            && self.last_aim.is_none_or(|aim| {
                distance(
                    aim,
                    MotionPoint {
                        x: self.mover.x(),
                        y: self.mover.y(),
                    },
                ) <= motion::CURSOR_SETTLE_PX as f32
            });
        let feedback = visible.then(|| self.feedback_frame()).flatten();
        let animating = visible
            && (!settled
                || self.pending.is_some()
                || self.active.is_some()
                || self.cloud_alpha < 1.0);
        MotionFrame {
            pos,
            space: self.space.clone(),
            rotation_deg: self.rotation_deg,
            cloud_alpha: self.cloud_alpha,
            feedback,
            animating,
            settled,
            heading_deg: f64::from(self.mover.heading_rad()).to_degrees() as f32,
            speed: self.mover.speed(),
        }
    }

    fn feedback_frame(&self) -> Option<FeedbackFrame> {
        let active = self.active.as_ref()?;
        let mut trail_samples = Vec::new();
        if is_slide(&active.gesture) {
            // The trail traces the ideal polyline start..head while the glyph
            // is the steered pursuit of the head — the phone's signature
            // asymmetry (do not "fix" it).
            sample_trail(
                &active.gesture.points,
                feedback_progress(active, self.now_ms),
                &mut trail_samples,
            );
        }
        Some(FeedbackFrame {
            kind: active.gesture.kind,
            started_at_ms: active.started_at_ms,
            duration_ms: active.gesture.duration_ms,
            ripple_center: active.arrival,
            trail_samples,
        })
    }

    /// Whether a pending gesture is waiting on arrival (for the reply echo).
    #[must_use]
    pub fn pending_gesture_feedback(&self) -> bool {
        self.pending.is_some()
    }

    /// The coordinate space the next [`Self::step`] will resolve its target
    /// in, given the state target's space — the same precedence `step` uses.
    /// Lets the host compute bounds for the right space before stepping.
    #[must_use]
    pub fn upcoming_space(&self, state_space: Option<CoordinateSpace>) -> Option<CoordinateSpace> {
        if let Some(active) = &self.active {
            return Some(active.gesture.space.clone());
        }
        if let Some(pending) = &self.pending {
            return Some(pending.space.clone());
        }
        // The rest-aim arm must mirror resolve_target's precedence: a frame
        // where the parked head wins (e.g. a glow-only state with no cursor
        // point) needs bounds for the space the aim actually lives in, or the
        // host hands the mover a wrong-space rect that yanks the pose.
        if let Some(rest) = &self.rest_aim {
            return Some(rest.space.clone());
        }
        state_space
    }
}

fn is_slide(g: &MotionGesture) -> bool {
    matches!(
        g.kind,
        AgentOverlayGestureKind::Drag | AgentOverlayGestureKind::Swipe
    ) && g.points.len() >= 2
}

fn feedback_progress(active: &ActiveGesture, now_ms: u64) -> f32 {
    let elapsed = now_ms.saturating_sub(active.started_at_ms) as f32;
    let duration = active.gesture.duration_ms.max(1) as f32;
    (elapsed / duration).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const FRAME_MS: u64 = 16;
    const EPOCH_BASE_MS: u64 = 1_000_000;

    fn bounds() -> MotionBounds {
        MotionBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 2000.0,
            max_y: 2000.0,
        }
    }

    fn input_at(base: Instant, frame: u64, target: Option<(f64, f64)>) -> MotionStepInput {
        MotionStepInput {
            now: base + Duration::from_millis(frame * FRAME_MS),
            now_ms: EPOCH_BASE_MS + frame * FRAME_MS,
            visible: true,
            target: target.map(|(x, y)| (x, y, CoordinateSpace::DesktopLogical)),
            bounds: bounds(),
        }
    }

    fn hidden_at(base: Instant, frame: u64) -> MotionStepInput {
        MotionStepInput {
            visible: false,
            ..input_at(base, frame, None)
        }
    }

    fn tap(x: f32, y: f32) -> MotionGesture {
        MotionGesture {
            kind: AgentOverlayGestureKind::Tap,
            points: vec![MotionPoint { x, y }],
            space: CoordinateSpace::DesktopLogical,
            duration_ms: 380,
        }
    }

    /// Snaps the driver to (x, y) via the mover's first-step snap.
    fn placed_at(base: Instant, x: f64, y: f64) -> CursorMotionDriver {
        let mut driver = CursorMotionDriver::new();
        let frame = driver.step(input_at(base, 0, Some((x, y))));
        assert!(frame.settled, "first step must snap");
        driver
    }

    #[test]
    fn pending_feedback_waits_for_settle_within_arrive_radius() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 100.0);
        driver.start_gesture(tap(600.0, 350.0));
        let mut fired_at = None;
        for frame_idx in 1..300u64 {
            // A stale state target must not outrank the pending gesture.
            let frame = driver.step(input_at(base, frame_idx, Some((100.0, 100.0))));
            if let Some(feedback) = &frame.feedback {
                let pos = frame.pos.unwrap();
                let dist = ((pos.x - 600.0).powi(2) + (pos.y - 350.0).powi(2)).sqrt();
                assert!(
                    dist <= geometry::GESTURE_ARRIVE_LOGICAL_PX as f32,
                    "feedback fired {dist}px from the gesture point"
                );
                assert_eq!(frame.speed, 0.0);
                assert_eq!(feedback.ripple_center, MotionPoint { x: 600.0, y: 350.0 });
                fired_at = Some(frame_idx);
                break;
            }
            assert!(
                frame.animating,
                "driver must keep animating while a gesture is pending"
            );
        }
        let fired_at = fired_at.expect("feedback never fired");
        assert!(
            fired_at > 5,
            "a 559px glide cannot settle in {fired_at} frames"
        );
    }

    #[test]
    fn feedback_clock_starts_at_arrival_not_receipt() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 100.0);
        driver.start_gesture(tap(900.0, 900.0));
        let receipt_ms = EPOCH_BASE_MS;
        for frame_idx in 1..300u64 {
            let frame = driver.step(input_at(base, frame_idx, None));
            if let Some(feedback) = &frame.feedback {
                let arrival_ms = EPOCH_BASE_MS + frame_idx * FRAME_MS;
                assert_eq!(feedback.started_at_ms, arrival_ms);
                assert!(feedback.started_at_ms > receipt_ms + 100);
                return;
            }
        }
        panic!("feedback never fired");
    }

    #[test]
    fn redirect_mid_flight_replaces_pending_and_preserves_momentum() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 1000.0);
        driver.start_gesture(tap(1900.0, 1000.0));
        let mut frame = driver.step(input_at(base, 1, None));
        for frame_idx in 2..=25u64 {
            frame = driver.step(input_at(base, frame_idx, None));
        }
        let turn_pos = frame.pos.unwrap();
        let turn_speed = frame.speed;
        assert!(
            turn_speed > 500.0,
            "expected a fast cruise, got {turn_speed}"
        );
        // Redirect straight down: the mover must carry x-momentum through the
        // turn instead of snapping onto the new bearing.
        driver.start_gesture(tap(turn_pos.x, 1900.0));
        let mut carried_x = turn_pos.x;
        let mut y_progress = turn_pos.y;
        for frame_idx in 26..=40u64 {
            let frame = driver.step(input_at(base, frame_idx, None));
            assert!(frame.speed > 0.0, "redirect must not stop the mover");
            assert!(frame.feedback.is_none(), "redirect must not fire feedback");
            let pos = frame.pos.unwrap();
            carried_x = carried_x.max(pos.x);
            y_progress = y_progress.max(pos.y);
        }
        assert!(
            carried_x > turn_pos.x + 1.0,
            "momentum should bow the path past x={} (got {carried_x})",
            turn_pos.x
        );
        assert!(
            y_progress > turn_pos.y + 1.0,
            "turn should progress downward"
        );
    }

    #[test]
    fn redirect_during_active_feedback_cancels_and_reaims() {
        let base = Instant::now();
        let mut driver = placed_at(base, 500.0, 500.0);
        driver.start_gesture(tap(500.0, 500.0));
        let frame = driver.step(input_at(base, 1, None));
        assert!(
            frame.feedback.is_some(),
            "co-located tap should fire at once"
        );
        driver.start_gesture(tap(1200.0, 500.0));
        let frame = driver.step(input_at(base, 2, None));
        assert!(
            frame.feedback.is_none(),
            "redirect must cancel the active feedback"
        );
        assert!(driver.pending_gesture_feedback());
    }

    #[test]
    fn rotation_eases_toward_travel_heading_above_min_speed() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 1000.0);
        let max_step_deg = motion::CURSOR_ROTATE_RATE_DEG_PER_S as f32 * 0.016 + 1e-3;
        let mut last_rotation = 0.0f32;
        for frame_idx in 1..=80u64 {
            let frame = driver.step(input_at(base, frame_idx, Some((1900.0, 1000.0))));
            let delta = (frame.rotation_deg - last_rotation).abs();
            assert!(
                delta <= max_step_deg,
                "rotation stepped {delta} deg in one frame (cap {max_step_deg})"
            );
            last_rotation = frame.rotation_deg;
        }
        // Mid-cruise the rotation must track the travel heading's rest pose
        // (heading - CURSOR_NOSE_DEG) within one easing step. The heading is
        // not exactly 0 here: the resting-nose launch bows the path off-axis
        // and the bearing keeps converging, which is the behavior under test.
        let frame = driver.step(input_at(base, 81, Some((1900.0, 1000.0))));
        assert!(frame.speed > motion::CURSOR_ROTATE_MIN_SPEED_DP_PER_S as f32);
        let target_rot = frame.heading_deg + 135.0;
        let lag = crate::motion::approach_angle_deg(frame.rotation_deg, target_rot, 360.0)
            - frame.rotation_deg;
        assert!(
            lag.abs() <= max_step_deg,
            "rotation {last_rotation} lags heading target {target_rot} by more than one step"
        );
        assert!(
            frame.rotation_deg > 100.0,
            "glyph should be rotated well into the travel heading, got {}",
            frame.rotation_deg
        );
    }

    #[test]
    fn rotation_returns_to_rest_below_min_speed() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 1000.0);
        let mut frame_idx = 1u64;
        loop {
            let frame = driver.step(input_at(base, frame_idx, Some((900.0, 1000.0))));
            if frame.settled && frame_idx > 2 {
                break;
            }
            frame_idx += 1;
            assert!(frame_idx < 500, "glide never settled");
        }
        let mut rotation = f32::MAX;
        for _ in 0..80 {
            frame_idx += 1;
            rotation = driver
                .step(input_at(base, frame_idx, Some((900.0, 1000.0))))
                .rotation_deg;
        }
        assert_eq!(rotation, 0.0, "parked cursor must ease back to rest");
    }

    #[test]
    fn rotation_zeroed_while_no_no_and_hard_reset_after() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 1000.0);
        let mut frame = driver.step(input_at(base, 1, None));
        for frame_idx in 2..=20u64 {
            frame = driver.step(input_at(base, frame_idx, Some((1900.0, 1000.0))));
        }
        assert!(
            frame.rotation_deg.abs() > 10.0,
            "cruise should have rotated the glyph"
        );
        let pos = frame.pos.unwrap();
        driver.start_gesture(MotionGesture {
            kind: AgentOverlayGestureKind::NoNo,
            points: vec![pos],
            space: CoordinateSpace::DesktopLogical,
            duration_ms: 760,
        });
        let mut saw_active = false;
        for frame_idx in 21..=200u64 {
            let frame = driver.step(input_at(base, frame_idx, None));
            if frame
                .feedback
                .as_ref()
                .is_some_and(|f| f.kind == AgentOverlayGestureKind::NoNo)
            {
                saw_active = true;
                assert_eq!(frame.rotation_deg, 0.0, "wiggle owns rotation");
            } else if saw_active {
                assert_eq!(
                    frame.rotation_deg, 0.0,
                    "rotation must hard-reset after no-no"
                );
                return;
            }
        }
        panic!("no-no feedback never played (saw_active={saw_active})");
    }

    #[test]
    fn cloud_alpha_blooms_over_fade_seconds_and_resets_on_cold_show() {
        let base = Instant::now();
        let mut driver = CursorMotionDriver::new();
        let frame = driver.step(input_at(base, 0, Some((500.0, 500.0))));
        assert_eq!(
            frame.cloud_alpha, 0.0,
            "cold show must start the bloom at zero"
        );
        let mut last = 0.0f32;
        let mut full_at = None;
        for frame_idx in 1..=80u64 {
            let frame = driver.step(input_at(base, frame_idx, Some((500.0, 500.0))));
            assert!(frame.cloud_alpha >= last, "bloom must be monotonic");
            last = frame.cloud_alpha;
            if frame.cloud_alpha >= 1.0 && full_at.is_none() {
                full_at = Some(frame_idx);
            }
        }
        let full_at = full_at.expect("cloud never reached full alpha");
        // 0.8s at 16ms frames is 50 frames.
        assert!((45..=55).contains(&full_at), "bloom took {full_at} frames");

        driver.hide(false);
        driver.step(hidden_at(base, 81));
        let frame = driver.step(input_at(base, 82, Some((500.0, 500.0))));
        assert_eq!(frame.cloud_alpha, 0.0, "re-show after plain hide is cold");
    }

    #[test]
    fn capture_hide_preserves_mover_and_pending_without_cloud_reset() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 100.0);
        driver.start_gesture(tap(1500.0, 1500.0));
        let mut frame = driver.step(input_at(base, 1, None));
        for frame_idx in 2..=10u64 {
            frame = driver.step(input_at(base, frame_idx, None));
        }
        let frozen_pos = frame.pos.unwrap();
        let frozen_cloud = frame.cloud_alpha;
        assert!(frame.speed > 0.0, "test needs a mid-flight capture");

        driver.hide(true);
        // Hidden frames simulate a long capture barrier: 10 wall-clock
        // seconds pass while frozen.
        for hidden_idx in 0..5u64 {
            let frame = driver.step(MotionStepInput {
                now: base + Duration::from_secs(2 + hidden_idx * 2),
                now_ms: EPOCH_BASE_MS + (2 + hidden_idx * 2) * 1000,
                visible: false,
                target: None,
                bounds: bounds(),
            });
            assert_eq!(
                frame.pos.unwrap(),
                frozen_pos,
                "hidden frames must freeze the pose"
            );
            assert!(!frame.animating, "hidden frames must not demand redraws");
        }

        let resume = driver.step(MotionStepInput {
            now: base + Duration::from_secs(12),
            now_ms: EPOCH_BASE_MS + 12_000,
            visible: true,
            target: None,
            bounds: bounds(),
        });
        assert_eq!(
            resume.pos.unwrap(),
            frozen_pos,
            "resume frame must not integrate hidden wall time"
        );
        assert!(
            resume.cloud_alpha >= frozen_cloud,
            "capture restore must not re-bloom"
        );
        assert!(
            driver.pending_gesture_feedback(),
            "capture hide must keep the pending gesture"
        );
    }

    #[test]
    fn plain_hide_drops_pending_and_marks_cold() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 100.0);
        driver.start_gesture(tap(1500.0, 1500.0));
        driver.step(input_at(base, 1, None));
        assert!(driver.pending_gesture_feedback());
        driver.hide(false);
        assert!(!driver.pending_gesture_feedback());
    }

    #[test]
    fn swipe_head_chase_follows_point_at_progress() {
        let base = Instant::now();
        let mut driver = placed_at(base, 200.0, 200.0);
        let points = vec![
            MotionPoint { x: 200.0, y: 200.0 },
            MotionPoint { x: 900.0, y: 200.0 },
        ];
        driver.start_gesture(MotionGesture {
            kind: AgentOverlayGestureKind::Drag,
            points: points.clone(),
            space: CoordinateSpace::DesktopLogical,
            duration_ms: 950,
        });
        let frame = driver.step(input_at(base, 1, None));
        let feedback = frame
            .feedback
            .expect("co-located drag start should arrive at once");
        let started = feedback.started_at_ms;
        let mut prev_head_x = 200.0f32;
        let mut chased = false;
        for frame_idx in 2..=60u64 {
            let frame = driver.step(input_at(base, frame_idx, None));
            let Some(feedback) = &frame.feedback else {
                break;
            };
            let now_ms = EPOCH_BASE_MS + frame_idx * FRAME_MS;
            let progress =
                ((now_ms - started) as f32 / feedback.duration_ms as f32).clamp(0.0, 1.0);
            let head = point_at_progress(&points, progress);
            assert_eq!(feedback.trail_samples.len(), 12);
            let trail_head = *feedback.trail_samples.last().unwrap();
            assert!(
                (trail_head.x - head.x).abs() < 1e-3,
                "trail head must ride the ideal head"
            );
            assert!(head.x >= prev_head_x, "head must move monotonically");
            prev_head_x = head.x;
            let pos = frame.pos.unwrap();
            assert!(
                pos.x <= head.x + 1.0,
                "the glyph is a steered pursuit and must not outrun the head"
            );
            if pos.x > 210.0 {
                chased = true;
            }
        }
        assert!(chased, "the glyph never chased the moving head");
    }

    #[test]
    fn off_screen_gesture_target_is_clamped_so_arrival_fires() {
        let base = Instant::now();
        let mut driver = placed_at(base, 1800.0, 900.0);
        driver.start_gesture(tap(5000.0, 900.0));
        for frame_idx in 1..=300u64 {
            let frame = driver.step(input_at(base, frame_idx, None));
            if let Some(feedback) = &frame.feedback {
                assert_eq!(
                    feedback.ripple_center,
                    MotionPoint {
                        x: 2000.0,
                        y: 900.0
                    },
                    "feedback must fire at the clamped border point"
                );
                return;
            }
        }
        panic!("clamped off-screen gesture never arrived");
    }

    #[test]
    fn space_change_snaps_instead_of_gliding() {
        let base = Instant::now();
        let mut driver = placed_at(base, 500.0, 500.0);
        let mut frame = driver.step(input_at(base, 1, Some((1500.0, 500.0))));
        for frame_idx in 2..=6u64 {
            frame = driver.step(input_at(base, frame_idx, Some((1500.0, 500.0))));
        }
        assert!(frame.speed > 0.0, "test needs an in-flight mover");
        let frame = driver.step(MotionStepInput {
            target: Some((300.0, 300.0, CoordinateSpace::StreamLogical)),
            ..input_at(base, 7, None)
        });
        assert_eq!(frame.pos.unwrap(), MotionPoint { x: 300.0, y: 300.0 });
        assert_eq!(frame.speed, 0.0, "a space change cannot be interpolated");
        assert_eq!(frame.space, Some(CoordinateSpace::StreamLogical));
    }

    #[test]
    fn plain_hide_supersedes_capture_freeze_for_cold_show() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 100.0);
        for frame_idx in 1..=60u64 {
            driver.step(input_at(base, frame_idx, Some((100.0, 100.0))));
        }
        // Capture hide whose restore never arrives, then a plain hide.
        driver.hide(true);
        driver.step(hidden_at(base, 61));
        driver.hide(false);
        driver.step(hidden_at(base, 62));
        let frame = driver.step(input_at(base, 63, Some((100.0, 100.0))));
        assert_eq!(
            frame.cloud_alpha, 0.0,
            "the plain hide must win: the next show is cold and re-blooms"
        );
    }

    #[test]
    fn retired_slide_holds_parked_head_against_stale_origin_target() {
        let base = Instant::now();
        let mut driver = placed_at(base, 200.0, 200.0);
        let points = vec![
            MotionPoint { x: 200.0, y: 200.0 },
            MotionPoint { x: 900.0, y: 200.0 },
        ];
        driver.start_gesture(MotionGesture {
            kind: AgentOverlayGestureKind::Drag,
            points,
            space: CoordinateSpace::DesktopLogical,
            duration_ms: 950,
        });
        // Play the whole feedback plus settle time with the STALE pre-dispatch
        // state target (the drag origin) still applied every frame.
        let mut frame = driver.step(input_at(base, 1, Some((200.0, 200.0))));
        for frame_idx in 2..=200u64 {
            frame = driver.step(input_at(base, frame_idx, Some((200.0, 200.0))));
        }
        let parked = frame.pos.unwrap();
        assert!(
            (parked.x - 900.0).abs() <= 1.5 && (parked.y - 200.0).abs() <= 1.5,
            "the glyph must hold the drag end, not sail back to the stale origin (got {parked:?})"
        );
        // A state target that actually moved (the post-dispatch update) takes
        // over again.
        let mut frame = driver.step(input_at(base, 201, Some((600.0, 600.0))));
        for frame_idx in 202..=400u64 {
            frame = driver.step(input_at(base, frame_idx, Some((600.0, 600.0))));
            if frame.settled {
                break;
            }
        }
        let pos = frame.pos.unwrap();
        assert!(
            (pos.x - 600.0).abs() <= 1.5 && (pos.y - 600.0).abs() <= 1.5,
            "a fresh state target must reclaim the aim (got {pos:?})"
        );
    }

    #[test]
    fn rest_aim_survives_target_none_and_reports_its_space_for_bounds() {
        let base = Instant::now();
        let mut driver = placed_at(base, 1800.0, 900.0);
        driver.start_gesture(tap(1800.0, 900.0));
        let frame = driver.step(input_at(base, 1, None));
        assert!(frame.feedback.is_some(), "co-located tap fires at once");
        // Play the feedback out with NO state target (a glow-only visible
        // state has no cursor point), then keep stepping.
        let mut frame = driver.step(input_at(base, 2, None));
        for frame_idx in 3..=40u64 {
            frame = driver.step(input_at(base, frame_idx, None));
        }
        assert!(frame.feedback.is_none(), "feedback must have retired");
        assert_eq!(
            frame.pos.unwrap(),
            MotionPoint {
                x: 1800.0,
                y: 900.0
            },
            "the parked head must hold with no state target"
        );
        // The bounds handshake must report the space the rest aim lives in,
        // or the host computes wrong-space bounds that yank the pose.
        assert_eq!(
            driver.upcoming_space(None),
            Some(CoordinateSpace::DesktopLogical)
        );
    }

    #[test]
    fn rest_aim_yields_to_a_target_in_a_different_space() {
        let base = Instant::now();
        let mut driver = placed_at(base, 500.0, 500.0);
        driver.start_gesture(tap(500.0, 500.0));
        driver.step(input_at(base, 1, None));
        let mut frame = driver.step(input_at(base, 2, None));
        for frame_idx in 3..=40u64 {
            frame = driver.step(input_at(base, frame_idx, None));
        }
        assert!(frame.feedback.is_none(), "feedback must have retired");
        // A stream-space target numerically identical to the stale origin
        // must still take over: heterogeneous spaces cannot be
        // distance-compared, and the space change snaps.
        let frame = driver.step(MotionStepInput {
            target: Some((500.0, 500.0, CoordinateSpace::StreamLogical)),
            ..input_at(base, 41, None)
        });
        assert_eq!(frame.space, Some(CoordinateSpace::StreamLogical));
        assert_eq!(frame.pos.unwrap(), MotionPoint { x: 500.0, y: 500.0 });
        assert_eq!(frame.speed, 0.0, "space adoption snaps");
    }

    #[test]
    fn implicit_hide_without_hide_call_applies_plain_hide_semantics() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 100.0);
        for frame_idx in 1..=60u64 {
            driver.step(input_at(base, frame_idx, Some((100.0, 100.0))));
        }
        driver.start_gesture(tap(1500.0, 1500.0));
        driver.step(input_at(base, 61, None));
        assert!(driver.pending_gesture_feedback());
        // A `SetCursor { visible: false }` replaces state without the host
        // calling hide(); the hidden frame itself must apply plain-hide
        // semantics.
        driver.step(hidden_at(base, 62));
        assert!(
            !driver.pending_gesture_feedback(),
            "an implicit hide must drop the stale pending gesture"
        );
        let frame = driver.step(input_at(base, 63, Some((100.0, 100.0))));
        assert_eq!(
            frame.cloud_alpha, 0.0,
            "re-show after an implicit hide is cold and re-blooms"
        );
    }

    #[test]
    fn hidden_frames_do_not_integrate_dt() {
        let base = Instant::now();
        let mut driver = placed_at(base, 100.0, 100.0);
        let mut frame = driver.step(input_at(base, 1, Some((1900.0, 1900.0))));
        for frame_idx in 2..=5u64 {
            frame = driver.step(input_at(base, frame_idx, Some((1900.0, 1900.0))));
        }
        let before = frame.pos.unwrap();
        driver.step(MotionStepInput {
            now: base + Duration::from_secs(100),
            now_ms: EPOCH_BASE_MS + 100_000,
            visible: false,
            target: None,
            bounds: bounds(),
        });
        let resumed = driver.step(MotionStepInput {
            now: base + Duration::from_millis(100_016),
            now_ms: EPOCH_BASE_MS + 100_016,
            visible: true,
            target: Some((1900.0, 1900.0, CoordinateSpace::DesktopLogical)),
            bounds: bounds(),
        });
        assert_eq!(
            resumed.pos.unwrap(),
            before,
            "the resume frame gets a zero dt slice, not the hidden gap"
        );
    }
}
