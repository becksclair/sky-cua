//! Env-gated dense-frame dump of the REAL cursor-motion driver driving the
//! REAL effect shader: the deterministic offline half of the Phase C motion
//! evidence. Each scenario steps a fresh [`CursorMotionDriver`] on a manual
//! clock and renders selected frames offscreen through
//! [`test_support::render_frame_rgba`], so the dumped `.rgba` frames show
//! exactly what the layer-shell host would draw — glide curves, eased
//! rotation, cloud bloom, arrival-gated ripple, and the head-chase trail.
//!
//! Clock discipline: the mover's dt comes from fabricated `Instant`s spaced
//! exactly 1/60 s apart (`base + sim_step * 16_666_667 ns`); the effect
//! timeline runs on `now_ms = epoch + round(sim_step * 1000 / 60)`. The
//! millisecond rounding never leaks into the mover's dt because dt is
//! Instant-derived — the same never-mix rule the production driver documents.
//! The single `Instant::now()` base is the only wall-clock read anywhere.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use sky_cua_platform::model::{AgentOverlayGestureKind, CoordinateSpace};
use sky_cua_platform::overlay_spec::desktop::geometry;

use crate::cursor_motion::{
    CursorMotionDriver, MotionBounds, MotionFrame, MotionGesture, MotionStepInput,
};
use crate::motion::MotionPoint;
use crate::renderer::scene::{CursorPoint, EffectScene};
use crate::renderer::test_support::{FrameRenderInput, render_frame_rgba, test_device};

const WIDTH: u32 = 768;
const HEIGHT: u32 = 448;
/// Exactly 1/60 s between fabricated instants, in nanoseconds.
const SIM_STEP_NANOS: u64 = 16_666_667;
/// Fixed epoch base for the effect timeline (arbitrary, deterministic).
const EPOCH_BASE_MS: u64 = 1_000_000;
/// Hard cap on simulation steps per scenario so a driver regression that
/// never settles fails loudly instead of spinning forever.
const MAX_SIM_STEPS: u64 = 1_200;

fn bounds() -> MotionBounds {
    MotionBounds {
        min_x: 0.0,
        min_y: 0.0,
        max_x: WIDTH as f32,
        max_y: HEIGHT as f32,
    }
}

fn step_input(base: Instant, sim_step: u64, target: Option<(f64, f64)>) -> MotionStepInput {
    MotionStepInput {
        // Mover dt: Instant-derived, exactly 1/60 s per sim step.
        now: base + Duration::from_nanos(sim_step * SIM_STEP_NANOS),
        // Effect timeline: epoch ms, rounded to the nearest whole ms. The
        // rounding never reaches the mover — its dt comes from `now` above.
        now_ms: EPOCH_BASE_MS + (sim_step * 1000 + 30) / 60,
        visible: true,
        target: target.map(|(x, y)| (x, y, CoordinateSpace::DesktopLogical)),
        bounds: bounds(),
    }
}

/// Builds the renderer scene from the driver's feedback via the shared
/// `FeedbackFrame::scene_points` rule (the same selection production and the
/// playground consume, so the offline evidence cannot drift from them); the
/// arrival-based start time and duration pass through unchanged. (The
/// layer-shell version also maps points into layer-local space; the capture
/// canvas IS the coordinate space here, so that mapping is the identity.)
fn effect_scene(frame: &MotionFrame) -> Option<EffectScene> {
    let feedback = frame.feedback.as_ref()?;
    let raw = feedback.scene_points();
    Some(EffectScene {
        kind: feedback.kind,
        started_at_ms: feedback.started_at_ms,
        duration_ms: feedback.duration_ms,
        points: raw
            .iter()
            .map(|p| CursorPoint {
                x: f64::from(p.x),
                y: f64::from(p.y),
            })
            .collect(),
    })
}

/// Renders one driver frame through the full production shader and writes it
/// as `<scenario>-fNN.rgba`, appending the frame's sim step to the manifest.
struct FrameSink<'a> {
    device: &'a ::wgpu::Device,
    queue: &'a ::wgpu::Queue,
    out: &'a std::path::Path,
    manifest: String,
}

impl FrameSink<'_> {
    fn record(
        &mut self,
        scenario: &str,
        rendered_index: usize,
        sim_step: u64,
        frame: &MotionFrame,
        now_ms: u64,
    ) {
        let effect = effect_scene(frame);
        let rgba = render_frame_rgba(
            self.device,
            self.queue,
            FrameRenderInput {
                width: WIDTH,
                height: HEIGHT,
                now_ms,
                cursor: frame.pos.map(|p| CursorPoint {
                    x: f64::from(p.x),
                    y: f64::from(p.y),
                }),
                effect: effect.as_ref(),
                cursor_rotation_deg: frame.rotation_deg,
                cursor_cloud_alpha: frame.cloud_alpha,
            },
        );
        let name = format!("{scenario}-f{rendered_index:02}.rgba");
        std::fs::write(self.out.join(&name), &rgba).expect("write motion frame");
        writeln!(self.manifest, "{scenario} {name} sim_step={sim_step}").expect("manifest line");
    }

    fn finish_scenario(&mut self, scenario: &str, rendered: usize) {
        writeln!(self.manifest, "{scenario} frames={rendered}").expect("manifest summary");
    }
}

/// Gated visual capture of the motion driver. No-op unless
/// `SKY_CUA_CAPTURE_MOTION=1`, so it never renders in normal CI; output dir
/// defaults to `/tmp/overlay-demo/motion` (override with
/// `SKY_CUA_CAPTURE_DIR`).
#[test]
fn capture_motion_frames_when_requested() {
    if std::env::var("SKY_CUA_CAPTURE_MOTION").ok().as_deref() != Some("1") {
        eprintln!("skipping motion capture: set SKY_CUA_CAPTURE_MOTION=1 to dump frames");
        return;
    }
    let Some((device, queue)) = test_device() else {
        eprintln!("skipping motion capture: no adapter available");
        return;
    };
    let out = std::path::PathBuf::from(
        std::env::var("SKY_CUA_CAPTURE_DIR").unwrap_or_else(|_| "/tmp/overlay-demo/motion".into()),
    );
    std::fs::create_dir_all(&out).expect("create capture dir");
    // The one and only wall-clock read: everything below runs on fabricated
    // offsets from this base.
    let base = Instant::now();
    let mut sink = FrameSink {
        device: &device,
        queue: &queue,
        out: &out,
        manifest: String::new(),
    };

    corner_glide(base, &mut sink);
    redirect(base, &mut sink);
    swipe_chase(base, &mut sink);
    arrival_gated_tap(base, &mut sink);

    std::fs::write(out.join("dims.txt"), format!("{WIDTH} {HEIGHT}\n")).expect("write dims");
    std::fs::write(out.join("manifest.txt"), &sink.manifest).expect("write manifest");
    eprintln!("wrote motion frames to {}", out.display());
}

/// Snap into the top-left corner, then glide to the bottom-right one: the
/// plainest full-length curve (resting-nose launch bow, cruise, arrive-radius
/// deceleration, settle snap) plus the cloud bloom from cold.
fn corner_glide(base: Instant, sink: &mut FrameSink<'_>) {
    let mut driver = CursorMotionDriver::new();
    // Sim step 0: the mover's first-ever step snaps to the park point.
    let frame = driver.step(step_input(base, 0, Some((80.0, 80.0))));
    assert!(frame.settled, "first step must snap");
    let mut rendered = 0usize;
    for sim_step in 1..=MAX_SIM_STEPS {
        let input = step_input(base, sim_step, Some((688.0, 368.0)));
        let now_ms = input.now_ms;
        let frame = driver.step(input);
        let settled = frame.settled;
        // Every 3rd sim step, plus the settle frame itself so the dump ends
        // on the parked pose.
        if (sim_step % 3 == 0 || settled) && rendered < 30 {
            sink.record("corner_glide", rendered, sim_step, &frame, now_ms);
            rendered += 1;
        }
        if settled || rendered >= 30 {
            break;
        }
    }
    assert!(rendered > 5, "corner glide rendered only {rendered} frames");
    sink.finish_scenario("corner_glide", rendered);
}

/// Glide toward the top-right, then retarget to the bottom-left mid-flight at
/// sim step 20: the frames bracketing the retarget are rendered densely
/// (every 2nd step) to show the momentum-preserving bow of the turn.
fn redirect(base: Instant, sink: &mut FrameSink<'_>) {
    const RETARGET_STEP: u64 = 20;
    let mut driver = CursorMotionDriver::new();
    let frame = driver.step(step_input(base, 0, Some((80.0, 80.0))));
    assert!(frame.settled, "first step must snap");
    let mut rendered = 0usize;
    for sim_step in 1..=MAX_SIM_STEPS {
        let target = if sim_step < RETARGET_STEP {
            (688.0, 224.0)
        } else {
            (150.0, 380.0)
        };
        let input = step_input(base, sim_step, Some(target));
        let now_ms = input.now_ms;
        let frame = driver.step(input);
        // Dense sampling across the 10 steps bracketing the retarget, sparse
        // elsewhere.
        let near_turn = (RETARGET_STEP - 5..=RETARGET_STEP + 5).contains(&sim_step);
        let due = if near_turn {
            sim_step % 2 == 0
        } else {
            sim_step % 3 == 0
        };
        let settled = frame.settled && sim_step > RETARGET_STEP;
        if (due || settled) && rendered < 30 {
            sink.record("redirect", rendered, sim_step, &frame, now_ms);
            rendered += 1;
        }
        if settled || rendered >= 30 {
            break;
        }
    }
    assert!(rendered > 5, "redirect rendered only {rendered} frames");
    sink.finish_scenario("redirect", rendered);
}

/// Park away from the gesture, then run a Drag slide: the dump shows the
/// glide to the start point, the arrival, and then the glyph's steered
/// pursuit of the moving head with the arc-length trail behind it.
fn swipe_chase(base: Instant, sink: &mut FrameSink<'_>) {
    let mut driver = CursorMotionDriver::new();
    let frame = driver.step(step_input(base, 0, Some((400.0, 100.0))));
    assert!(frame.settled, "first step must snap");
    driver.start_gesture(MotionGesture {
        kind: AgentOverlayGestureKind::Drag,
        points: vec![
            MotionPoint { x: 150.0, y: 224.0 },
            MotionPoint { x: 618.0, y: 224.0 },
        ],
        space: CoordinateSpace::DesktopLogical,
        duration_ms: 950,
    });
    let mut rendered = 0usize;
    let mut saw_feedback = false;
    for sim_step in 1..=MAX_SIM_STEPS {
        // No state target while the gesture pipeline drives: the pending
        // start point and then the moving head own the aim.
        let input = step_input(base, sim_step, None);
        let now_ms = input.now_ms;
        let frame = driver.step(input);
        if let Some(feedback) = &frame.feedback {
            saw_feedback = true;
            assert_eq!(feedback.trail_samples.len(), 12, "slide trail resample");
        }
        let done = saw_feedback && frame.feedback.is_none() && frame.settled;
        if (sim_step % 3 == 0 || done) && rendered < 40 {
            sink.record("swipe_chase", rendered, sim_step, &frame, now_ms);
            rendered += 1;
        }
        if done || rendered >= 40 {
            break;
        }
    }
    assert!(saw_feedback, "the drag feedback never fired");
    sink.finish_scenario("swipe_chase", rendered);
}

/// The by-eye proof of arrival gating (locked decision 2): a Tap far from the
/// parked cursor. The rendered frames MUST show no ripple until the settle
/// frame; the MotionFrame assertions pin the gate (pixels are for humans).
fn arrival_gated_tap(base: Instant, sink: &mut FrameSink<'_>) {
    let mut driver = CursorMotionDriver::new();
    let frame = driver.step(step_input(base, 0, Some((100.0, 100.0))));
    assert!(frame.settled, "first step must snap");
    driver.start_gesture(MotionGesture {
        kind: AgentOverlayGestureKind::Tap,
        points: vec![MotionPoint { x: 600.0, y: 350.0 }],
        space: CoordinateSpace::DesktopLogical,
        duration_ms: 380,
    });
    let mut rendered = 0usize;
    let mut first_feedback_step: Option<u64> = None;
    for sim_step in 1..=MAX_SIM_STEPS {
        let input = step_input(base, sim_step, None);
        let now_ms = input.now_ms;
        let frame = driver.step(input);
        if frame.feedback.is_some() && first_feedback_step.is_none() {
            first_feedback_step = Some(sim_step);
            // The arrival gate: the frame where feedback first appears is a
            // settled frame — mover parked (speed 0) within the arrive radius
            // of the tap point. This is the structured proof; the ripple-free
            // glide frames are the visual one.
            assert!(frame.settled, "feedback fired on an unsettled frame");
            assert_eq!(frame.speed, 0.0, "feedback fired while still moving");
            let pos = frame.pos.expect("placed mover");
            let dist = ((pos.x - 600.0).powi(2) + (pos.y - 350.0).powi(2)).sqrt();
            assert!(
                dist <= geometry::GESTURE_ARRIVE_LOGICAL_PX as f32,
                "feedback fired {dist}px from the tap point"
            );
        }
        let done = first_feedback_step.is_some() && frame.feedback.is_none();
        if (sim_step % 3 == 0 || done) && rendered < 40 {
            sink.record("arrival_gated_tap", rendered, sim_step, &frame, now_ms);
            rendered += 1;
        }
        if done || rendered >= 40 {
            break;
        }
    }
    let fired = first_feedback_step.expect("tap feedback never fired");
    assert!(
        fired > 10,
        "a 559px glide cannot arrive in {fired} sim steps; the gate did not hold"
    );
    sink.finish_scenario("arrival_gated_tap", rendered);
}
