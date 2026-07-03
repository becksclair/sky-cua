//! Pure vehicle-steering motion math for the desktop agent cursor.
//!
//! Op-for-op port of the Android companion's `OverlayMath` mover and path
//! helpers (`android/phone-companion/.../overlay/OverlayMath.kt`) plus the
//! controller's trail resampler (`AgentOverlayController.sampleTrail`). Free of
//! Wayland/wgpu/smithay types so the logic is unit-testable without a
//! compositor and portable to future backends.
//!
//! Numeric recipe (cross-language parity contract): all state is `f32`; every
//! transcendental (`sqrt`, `atan2`, `cos`, `sin`) is evaluated in `f64` and
//! truncated back to `f32` per assignment, exactly as the Kotlin source does
//! `Math.sqrt((...).toDouble()).toFloat()`. Non-transcendental arithmetic
//! between `f32` values stays in `f32`. The cross-language motion fixtures in
//! `resources/overlay/agent_overlay_motion_fixtures.json` depend on this
//! recipe holding everywhere in this module.

use sky_cua_platform::overlay_spec::shared::{effects, motion};

/// A point in the mover's logical-pixel coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionPoint {
    pub x: f32,
    pub y: f32,
}

/// Tunable parameters for [`Mover2D`], mirroring the Kotlin constructor
/// arguments one for one.
#[derive(Debug, Clone, Copy)]
pub struct MoverParams {
    /// Glide-speed cap (logical px/s).
    pub max_speed: f32,
    /// Forward acceleration/deceleration (logical px/s^2).
    pub accel: f32,
    /// Max heading change rate (rad/s); lower = wider curves.
    pub turn_rate_rad: f32,
    /// Distance (logical px) within which speed ramps down to a stop.
    pub arrive_radius: f32,
    /// Distance (logical px) within which the turn rate ramps up so the
    /// cursor curls tightly into the target instead of orbiting it; 0
    /// disables homing.
    pub homing_radius: f32,
    /// Peak extra turn-rate multiplier at the target (0 = off). Without it a
    /// wide-curve approach whose turn radius exceeds the remaining distance
    /// can never curve inward and orbits the point forever.
    pub homing_boost: f32,
    /// Heading (radians) the cursor resets to when it parks, so the next move
    /// launches along the pointer's resting nose and curves toward the target.
    pub default_heading_rad: f32,
}

impl MoverParams {
    /// Production parameters from the shared overlay spec, dp read as logical
    /// px 1:1 (strict constant parity with the phone). Degree constants
    /// convert through `f64::to_radians` before the `f32` truncation,
    /// mirroring the Kotlin `Math.toRadians` on `Double`.
    #[must_use]
    pub fn from_spec() -> Self {
        Self {
            max_speed: motion::CURSOR_MAX_SPEED_DP_PER_S as f32,
            accel: motion::CURSOR_ACCEL_DP_PER_S2 as f32,
            turn_rate_rad: motion::CURSOR_TURN_RATE_DEG_PER_S.to_radians() as f32,
            arrive_radius: motion::CURSOR_ARRIVE_RADIUS_DP as f32,
            homing_radius: motion::CURSOR_HOMING_RADIUS_DP as f32,
            homing_boost: motion::CURSOR_HOMING_TURN_BOOST as f32,
            default_heading_rad: motion::CURSOR_NOSE_DEG.to_radians() as f32,
        }
    }
}

/// A vehicle-steering mover for the agent cursor. It keeps a heading and a
/// forward speed: each step it turns the heading toward the target at a
/// bounded rate and thrusts forward, so it cannot pivot instantly — a
/// direction change carries momentum and bends the path into a curve. The
/// forward speed ramps to zero inside the arrive radius, so it eases to a
/// clean stop at the target and never overshoots or springs back.
///
/// Port of `OverlayMath.Mover2D` (OverlayMath.kt:392-496) with one extension:
/// bounds are a rect (`min_x/min_y/max_x/max_y`) instead of `(width, height)`
/// because desktop-global logical space can have negative origins (a monitor
/// left of primary). Kotlin `setBounds(w, h)` equals `set_bounds(0, 0, w, h)`.
#[derive(Debug, Clone)]
pub struct Mover2D {
    params: MoverParams,
    x: f32,
    y: f32,
    heading_rad: f32,
    speed: f32,
    initialized: bool,
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl Mover2D {
    /// Starts uninitialized (the first [`Self::step`] snaps) with the Kotlin
    /// default bounds `[0, f32::MAX]` on both axes.
    #[must_use]
    pub fn new(params: MoverParams) -> Self {
        Self {
            params,
            x: 0.0,
            y: 0.0,
            heading_rad: 0.0,
            speed: 0.0,
            initialized: false,
            min_x: 0.0,
            min_y: 0.0,
            max_x: f32::MAX,
            max_y: f32::MAX,
        }
    }

    /// Constrains the cursor to the rect so momentum can never carry it
    /// outside. Clamps the current position immediately too.
    pub fn set_bounds(&mut self, min_x: f32, min_y: f32, max_x: f32, max_y: f32) {
        self.min_x = min_x;
        self.min_y = min_y;
        self.max_x = max_x;
        self.max_y = max_y;
        self.x = self.x.clamp(min_x, max_x);
        self.y = self.y.clamp(min_y, max_y);
    }

    /// Jumps to the clamped target, stops, and resets heading to the resting
    /// nose.
    pub fn snap_to(&mut self, tx: f32, ty: f32) {
        self.x = tx.clamp(self.min_x, self.max_x);
        self.y = ty.clamp(self.min_y, self.max_y);
        self.speed = 0.0;
        self.heading_rad = self.params.default_heading_rad;
        self.initialized = true;
    }

    /// Advances toward the target by `dt_seconds`. The first-ever call snaps.
    /// The step is clamped to `CURSOR_MAX_STEP_S` so a stalled frame cannot
    /// fling the cursor, and a step that would pass the target lands exactly
    /// on it.
    ///
    /// The settle branch and the pass-target branch are distinct on purpose:
    /// only the settle branch resets the heading to the resting nose, so a
    /// landing keeps its travel heading until the next step settles.
    pub fn step(&mut self, tx: f32, ty: f32, dt_seconds: f32) {
        if !self.initialized {
            self.snap_to(tx, ty);
            return;
        }
        let dt = dt_seconds.clamp(0.0, motion::CURSOR_MAX_STEP_S as f32);
        if dt <= 0.0 {
            return;
        }
        let dx = tx - self.x;
        let dy = ty - self.y;
        let dist = f64::from(dx * dx + dy * dy).sqrt() as f32;
        if dist <= motion::CURSOR_SETTLE_PX as f32 {
            // Close enough: settle exactly on the target and stop, and reset
            // the heading to the resting nose so the next move launches along
            // the pointer's diagonal and curves toward its target.
            self.x = tx;
            self.y = ty;
            self.speed = 0.0;
            self.heading_rad = self.params.default_heading_rad;
            return;
        }
        {
            // Steer the heading toward the target, bounded by the turn rate.
            // The turn rate ramps up within the homing radius so the cursor
            // curls tightly into the target instead of orbiting it: far away
            // the wide cruise rate is untouched, but near the target the
            // shrinking turn radius lets the heading out-turn the bearing
            // sweep and spiral in.
            let homing = if self.params.homing_radius > 0.0 && dist < self.params.homing_radius {
                self.params.homing_boost * (1.0 - dist / self.params.homing_radius)
            } else {
                0.0
            };
            let target_angle = f64::from(dy).atan2(f64::from(dx)) as f32;
            let max_turn = self.params.turn_rate_rad * (1.0 + homing) * dt;
            let turn = wrap_radians(target_angle - self.heading_rad).clamp(-max_turn, max_turn);
            self.heading_rad = wrap_radians(self.heading_rad + turn);
        }
        // Forward speed ramps down on arrival so the cursor decelerates in.
        let desired_speed = if dist < self.params.arrive_radius {
            self.params.max_speed * (dist / self.params.arrive_radius)
        } else {
            self.params.max_speed
        };
        let ds =
            (desired_speed - self.speed).clamp(-self.params.accel * dt, self.params.accel * dt);
        self.speed = (self.speed + ds).max(0.0);
        let step_len = self.speed * dt;
        if step_len >= dist {
            // Would reach/pass the target this frame: land exactly on it.
            // The heading is deliberately NOT reset here; the next step's
            // settle branch does that.
            self.x = tx;
            self.y = ty;
            self.speed = 0.0;
            return;
        }
        // Integrate, clamped to the bounds so momentum never carries it out.
        self.x = (self.x + f64::from(self.heading_rad).cos() as f32 * step_len)
            .clamp(self.min_x, self.max_x);
        self.y = (self.y + f64::from(self.heading_rad).sin() as f32 * step_len)
            .clamp(self.min_y, self.max_y);
    }

    #[must_use]
    pub fn x(&self) -> f32 {
        self.x
    }

    #[must_use]
    pub fn y(&self) -> f32 {
        self.y
    }

    /// Current heading in radians (0 = +x / right, PI/2 = +y / down).
    #[must_use]
    pub fn heading_rad(&self) -> f32 {
        self.heading_rad
    }

    /// Current forward speed in logical px/s.
    #[must_use]
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Whether any snap/step has placed the mover yet.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
}

/// Wraps an angle (radians) to `(-PI, PI]`. Port of OverlayMath.kt:158-164;
/// the boundary comparisons run in `f64` against `f64` PI exactly as Kotlin
/// compares a `Float` against `Math.PI`.
#[must_use]
pub fn wrap_radians(angle: f32) -> f32 {
    let two_pi = (2.0 * std::f64::consts::PI) as f32;
    let mut a = angle % two_pi;
    if f64::from(a) <= -std::f64::consts::PI {
        a += two_pi;
    }
    if f64::from(a) > std::f64::consts::PI {
        a -= two_pi;
    }
    a
}

/// Moves `current` (degrees) toward `target` (degrees) by at most `max_delta`
/// along the shortest angular path, returning the new angle folded to
/// `(-180, 180]`. Port of OverlayMath.kt:171-180.
#[must_use]
pub fn approach_angle_deg(current: f32, target: f32, max_delta: f32) -> f32 {
    let mut diff = (target - current) % 360.0;
    if diff < -180.0 {
        diff += 360.0;
    }
    if diff > 180.0 {
        diff -= 360.0;
    }
    let step = diff.clamp(-max_delta, max_delta);
    let mut result = (current + step) % 360.0;
    if result <= -180.0 {
        result += 360.0;
    }
    if result > 180.0 {
        result -= 360.0;
    }
    result
}

/// Total length of the polyline through `points` in logical px. Returns 0 for
/// fewer than two points. Port of OverlayMath.kt:314-321.
#[must_use]
pub fn path_length(points: &[MotionPoint]) -> f32 {
    if points.len() < 2 {
        return 0.0;
    }
    let mut total = 0.0f32;
    for i in 1..points.len() {
        total += distance(points[i - 1], points[i]);
    }
    total
}

/// Position along the polyline at normalized `progress` in `[0, 1]`,
/// interpolated by arc length so motion is even regardless of how the input
/// points are spaced. Returns the first point for an empty-ish path and the
/// last point at progress >= 1. Port of OverlayMath.kt:329-351.
#[must_use]
pub fn point_at_progress(points: &[MotionPoint], progress: f32) -> MotionPoint {
    let Some(&first) = points.first() else {
        return MotionPoint { x: 0.0, y: 0.0 };
    };
    if points.len() == 1 {
        return first;
    }
    let p = clamp01(progress);
    if p <= 0.0 {
        return first;
    }
    let last = points[points.len() - 1];
    if p >= 1.0 {
        return last;
    }
    let total = path_length(points);
    if total <= 0.0 {
        return first;
    }
    let target = total * p;
    let mut travelled = 0.0f32;
    for i in 1..points.len() {
        let seg = distance(points[i - 1], points[i]);
        if seg <= 0.0 {
            continue;
        }
        if travelled + seg >= target {
            let local_t = (target - travelled) / seg;
            return lerp(points[i - 1], points[i], local_t);
        }
        travelled += seg;
    }
    last
}

/// Resamples the swept portion of the path (start..head at `progress`) into
/// `TRAIL_SAMPLES` arc-length-even points, replacing the contents of `out`.
/// Port of `AgentOverlayController.sampleTrail` (AgentOverlayController.kt:
/// 719-729); the sample count comes from the shared spec so phone and desktop
/// trails stay identical.
pub fn sample_trail(points: &[MotionPoint], progress: f32, out: &mut Vec<MotionPoint>) {
    let n = effects::TRAIL_SAMPLES as usize;
    out.clear();
    out.reserve(n);
    for i in 0..n {
        let frac = progress * (i as f32 / (n - 1) as f32);
        out.push(point_at_progress(points, frac));
    }
}

/// Euclidean distance between two points: `f32` squared-sum widened to `f64`
/// for the sqrt, truncated back to `f32` (Kotlin `OverlayMath.distance`).
/// `pub(crate)` so the motion driver's arrival/settle/takeover checks use the
/// same parity-critical recipe instead of re-inlining it.
pub(crate) fn distance(a: MotionPoint, b: MotionPoint) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    f64::from(dx * dx + dy * dy).sqrt() as f32
}

/// Clamps to `[0, 1]` (Kotlin `OverlayMath.clamp01`).
fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Linear interpolation between two points at clamped `t` (Kotlin
/// `OverlayMath.lerp`).
fn lerp(a: MotionPoint, b: MotionPoint, t: f32) -> MotionPoint {
    let x = clamp01(t);
    MotionPoint {
        x: a.x + (b.x - a.x) * x,
        y: a.y + (b.y - a.y) * x,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::Value;

    use super::*;

    const EPS: f32 = 1e-4;
    const DT: f32 = 1.0 / 60.0;

    fn motion_fixture_root() -> Value {
        serde_json::from_str(include_str!(
            "../../../resources/overlay/agent_overlay_motion_fixtures.json"
        ))
        .expect("parse motion fixtures JSON")
    }

    fn fixture_f32(value: &Value, key: &str) -> f32 {
        value[key]
            .as_f64()
            .unwrap_or_else(|| panic!("fixture field {key} must be a number: {value}"))
            as f32
    }

    fn fixture_point(value: &Value) -> MotionPoint {
        MotionPoint {
            x: fixture_f32(value, "x"),
            y: fixture_f32(value, "y"),
        }
    }

    fn fixture_cases<'a>(root: &'a Value, family: &str) -> &'a [Value] {
        let cases = root["fixtures"][family]
            .as_array()
            .unwrap_or_else(|| panic!("missing fixture family {family}"));
        assert!(
            !cases.is_empty(),
            "fixture family {family} must not be empty"
        );
        cases
    }

    /// Headings compare across the wrap: `|wrap_radians(expected - actual)| <=
    /// tol`, never raw subtraction (fixture-file contract).
    #[track_caller]
    fn assert_heading(context: &str, expected: f32, actual: f32, tol: f32) {
        let diff = wrap_radians(expected - actual).abs();
        assert!(
            diff <= tol,
            "{context}: |wrap_radians({expected} - {actual})| = {diff} > {tol}"
        );
    }

    /// Replays each generated `mover_trajectory` case against the production
    /// spec-constant mover: optional bounds, optional snap start, then stepping
    /// toward each segment target for its step count at the case dt. Mid-flight
    /// samples compare at `tolerance.mover`; samples at or past `settled_step`
    /// at `tolerance.default`. From `settled_step` on, the mover must hold the
    /// final target exactly with zero speed (the Kotlin reference proves the
    /// same invariant bit-for-bit). Trajectory inputs are exact float32
    /// decimals, so the replay is bit-exact.
    #[test]
    fn motion_fixtures_match_reference() {
        let root = motion_fixture_root();
        let mover_tol = fixture_f32(&root["tolerance"], "mover");
        let default_tol = fixture_f32(&root["tolerance"], "default");
        for case in fixture_cases(&root, "mover_trajectory") {
            let name = case["name"].as_str().expect("mover_trajectory case name");
            let mut mover = Mover2D::new(MoverParams::from_spec());
            if let Some(bounds) = case.get("bounds") {
                mover.set_bounds(
                    fixture_f32(bounds, "min_x"),
                    fixture_f32(bounds, "min_y"),
                    fixture_f32(bounds, "max_x"),
                    fixture_f32(bounds, "max_y"),
                );
            }
            if let Some(start) = case.get("start") {
                mover.snap_to(fixture_f32(start, "x"), fixture_f32(start, "y"));
            }
            let dt = fixture_f32(case, "dt");
            let segments = case["segments"]
                .as_array()
                .unwrap_or_else(|| panic!("{name} missing segments"));
            let samples_by_step: HashMap<u64, &Value> = case["samples"]
                .as_array()
                .unwrap_or_else(|| panic!("{name} missing samples"))
                .iter()
                .map(|sample| {
                    (
                        sample["step"]
                            .as_u64()
                            .expect("sample step must be an integer"),
                        sample,
                    )
                })
                .collect();
            let settled_step = case.get("settled_step").and_then(Value::as_u64);
            let final_target =
                fixture_point(&segments.last().expect("at least one segment")["target"]);
            let mut step: u64 = 0;
            for segment in segments {
                let target = fixture_point(&segment["target"]);
                let steps = segment["steps"].as_u64().expect("segment steps");
                for _ in 0..steps {
                    mover.step(target.x, target.y, dt);
                    step += 1;
                    if let Some(sample) = samples_by_step.get(&step) {
                        let tol = if settled_step.is_some_and(|settled| step >= settled) {
                            default_tol
                        } else {
                            mover_tol
                        };
                        let (ex, ey) = (fixture_f32(sample, "x"), fixture_f32(sample, "y"));
                        assert!(
                            (ex - mover.x()).abs() <= tol,
                            "{name} x at step {step}: expected {ex}, got {}",
                            mover.x()
                        );
                        assert!(
                            (ey - mover.y()).abs() <= tol,
                            "{name} y at step {step}: expected {ey}, got {}",
                            mover.y()
                        );
                        assert_heading(
                            &format!("{name} heading at step {step}"),
                            fixture_f32(sample, "heading_rad"),
                            mover.heading_rad(),
                            tol,
                        );
                        let espeed = fixture_f32(sample, "speed");
                        assert!(
                            (espeed - mover.speed()).abs() <= tol,
                            "{name} speed at step {step}: expected {espeed}, got {}",
                            mover.speed()
                        );
                    }
                    if settled_step.is_some_and(|settled| step >= settled) {
                        // Settled means landed: from settled_step on, the
                        // mover holds the final target exactly with zero
                        // speed.
                        assert_eq!(mover.x(), final_target.x, "{name} settled x at step {step}");
                        assert_eq!(mover.y(), final_target.y, "{name} settled y at step {step}");
                        assert_eq!(mover.speed(), 0.0, "{name} settled speed at step {step}");
                    }
                }
            }
            let unreplayed: Vec<u64> = samples_by_step
                .keys()
                .copied()
                .filter(|sampled| *sampled > step)
                .collect();
            assert!(
                unreplayed.is_empty(),
                "{name} has samples beyond the replayed steps: {unreplayed:?}"
            );
        }
    }

    #[test]
    fn approach_angle_matches_fixtures() {
        let root = motion_fixture_root();
        let tol = fixture_f32(&root["tolerance"], "default");
        for case in fixture_cases(&root, "approach_angle") {
            let current = fixture_f32(case, "current");
            let target = fixture_f32(case, "target");
            let max_delta = fixture_f32(case, "max_delta");
            let expected = fixture_f32(case, "expected");
            let actual = approach_angle_deg(current, target, max_delta);
            assert!(
                (expected - actual).abs() <= tol,
                "approach angle current={current} target={target} max_delta={max_delta}: \
                 expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn wrap_radians_matches_fixtures() {
        let root = motion_fixture_root();
        let tol = fixture_f32(&root["tolerance"], "default");
        for case in fixture_cases(&root, "wrap_radians") {
            let value = fixture_f32(case, "value");
            let expected = fixture_f32(case, "expected");
            // Wrapped comparison: the fixture folds boundary values to
            // whichever side of the +-PI seam the reference produced.
            assert_heading(
                &format!("wrap radians of {value}"),
                expected,
                wrap_radians(value),
                tol,
            );
        }
    }

    #[test]
    fn trail_resample_matches_fixtures() {
        let root = motion_fixture_root();
        let tol = fixture_f32(&root["tolerance"], "default");
        for case in fixture_cases(&root, "trail_resample") {
            let points: Vec<MotionPoint> = case["points"]
                .as_array()
                .expect("trail_resample points")
                .iter()
                .map(fixture_point)
                .collect();
            let progress = fixture_f32(case, "progress");
            assert_eq!(
                case["sample_count"].as_u64().expect("sample_count"),
                u64::from(effects::TRAIL_SAMPLES),
                "trail_resample sample_count must match the spec"
            );
            let expected: Vec<MotionPoint> = case["expected"]
                .as_array()
                .expect("trail_resample expected samples")
                .iter()
                .map(fixture_point)
                .collect();
            assert_eq!(
                expected.len(),
                effects::TRAIL_SAMPLES as usize,
                "trail_resample expected sample list size"
            );
            let mut out = Vec::new();
            sample_trail(&points, progress, &mut out);
            assert_eq!(out.len(), expected.len());
            for (i, (want, got)) in expected.iter().zip(&out).enumerate() {
                assert!(
                    (want.x - got.x).abs() <= tol,
                    "trail sample {i} x at progress {progress}: expected {}, got {}",
                    want.x,
                    got.x
                );
                assert!(
                    (want.y - got.y).abs() <= tol,
                    "trail sample {i} y at progress {progress}: expected {}, got {}",
                    want.y,
                    got.y
                );
            }
        }
    }

    /// The AgentOverlayTest.kt test vehicle: same params, default resting
    /// nose 0 (+x) as the Kotlin constructor default.
    fn vehicle_params() -> MoverParams {
        MoverParams {
            max_speed: 3000.0,
            accel: 18000.0,
            turn_rate_rad: 300.0f64.to_radians() as f32,
            arrive_radius: 350.0,
            homing_radius: 750.0,
            homing_boost: 3.5,
            default_heading_rad: 0.0,
        }
    }

    fn vehicle() -> Mover2D {
        Mover2D::new(vehicle_params())
    }

    #[test]
    fn first_step_snaps_uninitialized_mover() {
        let mut m = vehicle();
        m.step(50.0, 70.0, DT);
        assert!((m.x() - 50.0).abs() <= EPS);
        assert!((m.y() - 70.0).abs() <= EPS);
        assert!(m.speed().abs() <= EPS);
        assert!(m.is_initialized());
    }

    #[test]
    fn arrives_exactly_without_overshoot() {
        let mut m = vehicle();
        m.snap_to(0.0, 0.0);
        let mut max_x = 0.0f32;
        for _ in 0..400 {
            m.step(500.0, 0.0, DT);
            if m.x() > max_x {
                max_x = m.x();
            }
        }
        // A straight move decelerates to a clean stop ON the target — never
        // past it (no spring-back) — and lands exactly.
        assert!(max_x <= 500.0 + EPS, "overshot to {max_x}");
        assert_eq!(m.x(), 500.0);
        assert_eq!(m.speed(), 0.0);
    }

    #[test]
    fn curves_on_direction_change_with_momentum() {
        // Build rightward momentum, then retarget straight DOWN from the turn
        // point. The bounded turn rate keeps it moving right while it swings
        // to face down, so the path bows out instead of dropping straight.
        let mut m = vehicle();
        m.snap_to(0.0, 0.0);
        for _ in 0..30 {
            m.step(4000.0, 0.0, DT);
        }
        let x_at_turn = m.x();
        for _ in 0..15 {
            m.step(x_at_turn, 4000.0, DT);
        }
        assert!(
            m.x() > x_at_turn + 1.0,
            "expected rightward carry past {x_at_turn}, got {}",
            m.x()
        );
        assert!(m.y() > 1.0, "expected downward progress, got {}", m.y());
    }

    #[test]
    fn homing_boost_required_to_settle_off_axis_approach() {
        // The launch heading (reset to default = +x) points away from a target
        // to the left, forcing a wide curving approach. Without the near-target
        // turn-rate boost (homing) the turn radius stays larger than the
        // remaining distance, so the cursor orbits the point forever; with it
        // the cursor curls in and settles exactly. This asserts the boost is
        // load-bearing, not merely that the tuned mover happens to converge.
        let tx = 120.0f32;
        let ty = 470.0f32;

        let mut no_homing = Mover2D::new(MoverParams {
            homing_radius: 0.0,
            homing_boost: 0.0,
            ..vehicle_params()
        });
        no_homing.snap_to(500.0, 500.0);
        let mut settled = false;
        for _ in 0..2000 {
            no_homing.step(tx, ty, DT);
            let dist = f64::from(no_homing.x() - tx).hypot(f64::from(no_homing.y() - ty));
            if no_homing.speed() == 0.0 && dist < 2.0 {
                settled = true;
            }
        }
        assert!(
            !settled,
            "without homing the cursor must orbit and never settle"
        );

        let mut with_homing = vehicle();
        with_homing.snap_to(500.0, 500.0);
        for _ in 0..600 {
            with_homing.step(tx, ty, DT);
        }
        assert!(
            (with_homing.x() - tx).abs() <= EPS,
            "homing must converge: x = {}",
            with_homing.x()
        );
        assert!(
            (with_homing.y() - ty).abs() <= EPS,
            "homing must converge: y = {}",
            with_homing.y()
        );
        assert!(with_homing.speed().abs() <= EPS);
    }

    #[test]
    fn clamps_huge_frame_steps() {
        let mut m = vehicle();
        m.snap_to(0.0, 0.0);
        // A 10 s "frame" must not fling the cursor; the step is clamped.
        m.step(500.0, 0.0, 10.0);
        assert!(
            m.x().is_finite() && (0.0..=500.0).contains(&m.x()),
            "x must stay bounded, got {}",
            m.x()
        );
    }

    #[test]
    fn never_leaves_bounds() {
        let mut m = vehicle();
        m.set_bounds(0.0, 0.0, 1000.0, 2000.0);
        m.snap_to(500.0, 500.0);
        // Aim far outside the bounds; momentum must never carry it out.
        for _ in 0..200 {
            m.step(9000.0, 9000.0, DT);
        }
        assert!(
            (0.0..=1000.0).contains(&m.x()),
            "x out of bounds: {}",
            m.x()
        );
        assert!(
            (0.0..=2000.0).contains(&m.y()),
            "y out of bounds: {}",
            m.y()
        );
    }

    #[test]
    fn wrap_radians_folds_full_turn_to_zero() {
        assert!(wrap_radians((2.0 * std::f64::consts::PI) as f32).abs() <= 1e-3);
    }

    #[test]
    fn approach_angle_takes_shortest_path_across_the_wrap() {
        // 170 -> -170 is +20 the short way (through 180), not -340.
        assert!((approach_angle_deg(170.0, -170.0, 5.0) - 175.0).abs() <= EPS);
    }

    #[test]
    fn approach_angle_clamps_to_max_delta() {
        assert!((approach_angle_deg(0.0, 90.0, 10.0) - 10.0).abs() <= EPS);
    }

    #[test]
    fn path_length_sums_segments_and_zero_for_degenerate() {
        let pts = [
            MotionPoint { x: 0.0, y: 0.0 },
            MotionPoint { x: 3.0, y: 4.0 },  // 5
            MotionPoint { x: 3.0, y: 14.0 }, // 10
        ];
        assert!((path_length(&pts) - 15.0).abs() <= EPS);
        assert_eq!(path_length(&[]), 0.0);
        assert_eq!(path_length(&[MotionPoint { x: 5.0, y: 5.0 }]), 0.0);
    }

    #[test]
    fn point_at_progress_returns_endpoints() {
        let pts = [
            MotionPoint { x: 0.0, y: 0.0 },
            MotionPoint { x: 10.0, y: 0.0 },
        ];
        assert!(point_at_progress(&pts, 0.0).x.abs() <= EPS);
        assert!((point_at_progress(&pts, 1.0).x - 10.0).abs() <= EPS);
    }

    #[test]
    fn point_at_progress_interpolates_by_arc_length() {
        // A bent path where geometric midpoint != index midpoint. First
        // segment length 10, second segment length 30; total 40. At progress
        // 0.5 we are 20 along: 10 into the second segment.
        let pts = [
            MotionPoint { x: 0.0, y: 0.0 },
            MotionPoint { x: 10.0, y: 0.0 },
            MotionPoint { x: 40.0, y: 0.0 },
        ];
        assert!((point_at_progress(&pts, 0.5).x - 20.0).abs() <= EPS);
    }

    #[test]
    fn point_at_progress_clamps_out_of_range() {
        let pts = [
            MotionPoint { x: 0.0, y: 0.0 },
            MotionPoint { x: 10.0, y: 0.0 },
        ];
        assert!(point_at_progress(&pts, -1.0).x.abs() <= EPS);
        assert!((point_at_progress(&pts, 5.0).x - 10.0).abs() <= EPS);
    }

    #[test]
    fn point_at_progress_handles_zero_length_path() {
        let pts = [
            MotionPoint { x: 7.0, y: 7.0 },
            MotionPoint { x: 7.0, y: 7.0 },
        ];
        let p = point_at_progress(&pts, 0.5);
        assert!((p.x - 7.0).abs() <= EPS);
        assert!((p.y - 7.0).abs() <= EPS);
    }

    #[test]
    fn sample_trail_spans_start_to_head_with_12_samples() {
        let pts = [
            MotionPoint { x: 0.0, y: 0.0 },
            MotionPoint { x: 10.0, y: 0.0 },
            MotionPoint { x: 40.0, y: 0.0 },
        ];
        let progress = 0.5;
        let mut out = Vec::new();
        sample_trail(&pts, progress, &mut out);
        assert_eq!(out.len(), 12);
        assert_eq!(out.len(), effects::TRAIL_SAMPLES as usize);
        let first = out[0];
        assert!((first.x - pts[0].x).abs() <= EPS);
        assert!((first.y - pts[0].y).abs() <= EPS);
        let head = point_at_progress(&pts, progress);
        let last = out[out.len() - 1];
        assert!((last.x - head.x).abs() <= EPS);
        assert!((last.y - head.y).abs() <= EPS);
    }
}
