//! Pure waypoint generation for smooth pointer drags.
//!
//! The Linux drag backends historically teleported: press at the origin, emit a
//! single motion to the destination, then release. Toolkit sliders (`Gtk.Scale`,
//! libadwaita) and drag-and-drop gestures track the pointer through continuous
//! motion events held under the button grab, so a single jump is the input they
//! handle worst. [`drag_waypoints`] expands a drag into an eased sequence of
//! intermediate points plus a per-step delay, honoring an optional caller
//! `duration_ms`. Emission stays per-backend (each backend's `drag` loops over
//! the points inside its existing button grab); only generation lives here so it
//! can be unit-tested without a live session.

use std::time::Duration;

/// Minimum number of motion segments in a generated drag path. Eight samples is
/// enough for the pointer to cross a toolkit's drag-and-drop threshold (~8 px on
/// GTK) well before the destination, which is what arms a real drag gesture.
const MIN_STEPS: usize = 8;
/// Maximum number of motion segments in a generated drag path.
const MAX_STEPS: usize = 64;
/// Target on-screen distance, in logical pixels, covered per motion segment.
const PX_PER_STEP: f64 = 24.0;
/// Per-step delay used when the caller does not request a `duration_ms`. Kept
/// small so latency-sensitive smokes that drag without a duration stay fast.
const DEFAULT_STEP_DELAY: Duration = Duration::from_millis(6);
/// Lower bound on the per-step delay so compositors observe discrete motion
/// instead of coalescing the waypoints back into a near-teleport.
const MIN_STEP_DELAY: Duration = Duration::from_millis(2);
/// Upper bound on a caller-requested drag duration. A drag holds the single
/// serialized pointer-input worker (EIS worker / ydotool sequence) for its whole
/// span, so an unbounded `duration_ms` would let one request block all other
/// input. Real UI drags are sub-second to a few seconds; 10s is generous
/// headroom while keeping the worst case bounded. (The phone gesture path caps
/// similarly; see `MAX_GESTURE_DURATION_MS`.)
const MAX_DURATION_MS: u64 = 10_000;

/// An expanded drag path: ordered points (origin first, destination last) plus
/// the delay to sleep between successive motion events.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DragPath {
    pub points: Vec<(f64, f64)>,
    pub per_step_delay: Duration,
}

/// Expand a drag from `from` to `to` into an eased waypoint path.
///
/// The number of segments scales with the straight-line distance, clamped to
/// `[MIN_STEPS, MAX_STEPS]`. Points are placed with a smoothstep ease so the
/// grab accelerates and decelerates like a hand-held drag, which toolkits are
/// far more willing to treat as a real drag-and-drop than an abrupt jump. The
/// first and last points are exactly `from` and `to` so the press and release
/// land on the requested coordinates.
///
/// When `duration_ms` is `Some(value)` and positive the per-step delay paces the
/// whole motion across roughly that wall-clock duration. When it is absent the
/// path still interpolates (so sliders and DnD work by default) but uses the
/// minimum step count and a small fixed per-step delay so existing drag smokes
/// that pass no duration do not regress.
pub(crate) fn drag_waypoints(
    from: (f64, f64),
    to: (f64, f64),
    duration_ms: Option<u64>,
) -> DragPath {
    // Clamp the caller-supplied duration so one drag cannot pin the shared
    // pointer-input worker indefinitely.
    let duration_ms = duration_ms.map(|value| value.min(MAX_DURATION_MS));

    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let distance = dx.hypot(dy);

    let paced = duration_ms.is_some_and(|value| value > 0);
    let steps = if paced {
        ((distance / PX_PER_STEP).round() as usize).clamp(MIN_STEPS, MAX_STEPS)
    } else {
        MIN_STEPS
    };

    let mut points = Vec::with_capacity(steps + 1);
    for index in 0..=steps {
        if index == 0 {
            points.push(from);
        } else if index == steps {
            points.push(to);
        } else {
            let t = index as f64 / steps as f64;
            let eased = t * t * (3.0 - 2.0 * t);
            points.push((from.0 + dx * eased, from.1 + dy * eased));
        }
    }

    let per_step_delay = match duration_ms {
        Some(value) if value > 0 => Duration::from_millis(value / steps as u64).max(MIN_STEP_DELAY),
        _ => DEFAULT_STEP_DELAY,
    };

    DragPath {
        points,
        per_step_delay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments(path: &DragPath) -> usize {
        path.points.len().saturating_sub(1)
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn endpoints_are_exact() {
        let path = drag_waypoints((100.0, 200.0), (640.0, 480.0), Some(500));
        let first = path.points.first().copied().expect("first point");
        let last = path.points.last().copied().expect("last point");
        assert_close(first.0, 100.0);
        assert_close(first.1, 200.0);
        assert_close(last.0, 640.0);
        assert_close(last.1, 480.0);
    }

    #[test]
    fn progress_is_monotonic_along_the_segment() {
        let from = (100.0, 100.0);
        let to = (400.0, 250.0);
        let path = drag_waypoints(from, to, Some(600));
        let dir = (to.0 - from.0, to.1 - from.1);
        let mut previous = f64::NEG_INFINITY;
        for &(x, y) in &path.points {
            // Projection of the point onto the drag direction must not retreat.
            let projection = (x - from.0) * dir.0 + (y - from.1) * dir.1;
            assert!(
                projection >= previous - 1e-9,
                "projection went backwards: {projection} < {previous}"
            );
            previous = projection;
        }
    }

    #[test]
    fn an_intermediate_sample_crosses_the_dnd_threshold() {
        let from = (100.0, 100.0);
        let to = (400.0, 100.0);
        let path = drag_waypoints(from, to, Some(600));
        let interior = &path.points[1..path.points.len() - 1];
        // At least one sample strictly between the endpoints crosses GTK's ~8 px
        // drag threshold without being the teleport to the destination.
        assert!(
            interior
                .iter()
                .any(|&(x, _)| (x - from.0).abs() >= 8.0 && (x - from.0).abs() < 300.0),
            "no intermediate waypoint crossed the drag threshold: {interior:?}"
        );
    }

    #[test]
    fn step_count_is_bounded() {
        let tiny = drag_waypoints((0.0, 0.0), (5.0, 0.0), Some(200));
        assert_eq!(segments(&tiny), MIN_STEPS);

        let huge = drag_waypoints((0.0, 0.0), (100_000.0, 0.0), Some(2000));
        assert_eq!(segments(&huge), MAX_STEPS);
    }

    #[test]
    fn duration_paces_the_total_motion() {
        let path = drag_waypoints((0.0, 0.0), (300.0, 0.0), Some(800));
        let total = path.per_step_delay * segments(&path) as u32;
        let drift = (total.as_millis() as i128 - 800).unsigned_abs();
        assert!(
            drift <= path.per_step_delay.as_millis(),
            "total {} ms drifted from 800 ms by more than one step",
            total.as_millis()
        );
    }

    #[test]
    fn absent_duration_keeps_a_cheap_default() {
        let path = drag_waypoints((0.0, 0.0), (900.0, 600.0), None);
        assert_eq!(segments(&path), MIN_STEPS);
        assert_eq!(path.per_step_delay, DEFAULT_STEP_DELAY);
    }

    #[test]
    fn duration_is_capped_so_a_drag_cannot_pin_the_worker() {
        // An absurd duration must not produce an unbounded hold time.
        let path = drag_waypoints((0.0, 0.0), (300.0, 0.0), Some(u64::MAX));
        let total = path.per_step_delay * segments(&path) as u32;
        assert!(
            total.as_millis() as u64 <= MAX_DURATION_MS,
            "capped drag total {} ms exceeded the cap {} ms",
            total.as_millis(),
            MAX_DURATION_MS
        );
        // A normal duration under the cap is unaffected.
        let normal = drag_waypoints((0.0, 0.0), (300.0, 0.0), Some(600));
        let normal_total = normal.per_step_delay * segments(&normal) as u32;
        let drift = (normal_total.as_millis() as i128 - 600).unsigned_abs();
        assert!(drift <= normal.per_step_delay.as_millis());
    }

    #[test]
    fn degenerate_zero_length_drag_does_not_panic() {
        let path = drag_waypoints((42.0, 42.0), (42.0, 42.0), Some(300));
        assert!(path.points.len() >= 2);
        assert_eq!(path.points.first(), path.points.last());
    }
}
