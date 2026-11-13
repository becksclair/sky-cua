//! The platform-neutral overlay surface host.
//!
//! [`OverlaySurfaceHost`] is the visible-overlay backend shared by every
//! desktop that renders with wgpu: it speaks the host protocol, owns the
//! cursor target state and the motion driver, maps the motion frame onto one
//! surface per output, and drives the renderer. Everything native (surface
//! creation, the frame hand-off, the capture barrier mechanism, system-cursor
//! and pointer-telemetry adapters) sits behind [`SurfacePlatform`].
//!
//! The motion driver steps exactly once per rendered frame at the top of
//! [`OverlaySurfaceHost::draw`]. Every render path funnels through `draw`, so
//! stepping it anywhere else would double-integrate.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use sky_cua_platform::model::{AgentCursorState, AgentOverlayHostLifecycleState, DiagnosticEntry};

use crate::{
    GestureEventTracker, OVERLAY_HOST_PROTOCOL_VERSION, OverlayArrivalCondition,
    OverlayArrivalOutcome, OverlayArrivalWaitReply, OverlayArrivalWaitRequest, OverlayHostMessage,
    OverlayHostMessageKind, OverlayHostReply, OverlayMotionStatus, cursor_asset,
    cursor_motion::{
        CursorMotionDriver, MotionBounds, MotionFrame, MotionGesture, MotionStepInput,
    },
    diagnostic,
    motion::MotionPoint,
    renderer::{
        CursorImage, SurfaceDrawRequest, SurfaceDrawSpec, SurfaceGuard, WgpuOverlayInstance,
    },
};

mod capabilities;
mod cursor_state;
pub(crate) mod geometry;
mod motion_adapter;
pub(crate) mod platform;
mod renderer_selection;

use cursor_state::{
    apply_system_pointer_position, cursor_point, state_needs_system_pointer_update,
};
use geometry::{
    FALLBACK_PX_PER_MM, PointerTrackingBounds, box_reaches_output, translate_to_output_local,
};
use platform::{SurfaceChange, SurfacePlatform, SurfaceSlot};
use renderer_selection::SelectedRenderer;

/// Upper bound on one held `wait_for_arrival` request.
const MAX_ARRIVAL_WAIT_MS: u64 = 8_000;

/// How long a request that makes the overlay visible waits for a surface to
/// take its frame before replying anyway. A freshly started macOS host needs
/// about 250 ms before its first panel can take a frame, and the service's
/// request timeout is 2 s.
const PRESENT_WAIT: Duration = Duration::from_secs(1);
const PRESENT_POLL: Duration = Duration::from_millis(1);

/// What a frame did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameOutcome {
    /// A surface took the frame, or the overlay is hidden and had nothing to
    /// show.
    Settled,
    /// The overlay should be visible, but no surface could take a frame yet
    /// (a platform still bringing its surfaces on screen, or surfaces the
    /// windowing system reports as occluded).
    AwaitingSurfaces,
}

// Field order is drop order: the renderer and the wgpu surfaces go before the
// instance, and all of them before the platform's native surfaces.
#[derive(Debug)]
pub struct OverlaySurfaceHost<P: SurfacePlatform> {
    renderer: SelectedRenderer,
    /// wgpu surfaces, index-aligned with the platform's surfaces.
    surface_guards: Vec<Option<SurfaceGuard>>,
    instance: WgpuOverlayInstance,
    cursor: CursorImage,
    state: Option<AgentCursorState>,
    lifecycle_state: AgentOverlayHostLifecycleState,
    gesture_tracker: GestureEventTracker,
    /// The vehicle-steering motion driver: owns the drawn cursor pose between
    /// frames. `state` is only ever its target.
    motion: CursorMotionDriver,
    /// The driver's latest frame, for `should_animate` and the reply echo.
    last_motion: Option<MotionFrame>,
    frames_submitted: u64,
    last_frame_submission_us: Option<u128>,
    /// The latest frame should have been visible but no surface took it. The
    /// motion echo and the arrival wait never report an arrival meanwhile.
    awaiting_surfaces: bool,
    /// A wait for a surface to take a frame already ran out (a sleeping or
    /// locked display). Later requests do not wait again until a frame lands,
    /// so a display that shows nothing costs one wait, not one per request.
    presentation_stalled: bool,
    platform: P,
}

impl<P: SurfacePlatform> OverlaySurfaceHost<P> {
    /// Builds the host around a platform whose surfaces already exist:
    /// creates the wgpu surfaces, waits for the platform to make them
    /// drawable, selects the renderer, and renders the first (hidden) frame.
    pub(crate) fn start(platform: P) -> Result<Self> {
        let cursor = CursorImage::load()?;
        let instance = WgpuOverlayInstance::new();
        let surface_guards = create_surface_guards(&platform, &instance);
        let mut host = Self {
            renderer: SelectedRenderer::Unsupported {
                reason: Some("overlay renderer has not been selected yet".to_string()),
            },
            surface_guards,
            instance,
            cursor,
            state: None,
            lifecycle_state: AgentOverlayHostLifecycleState::BackendInitializing,
            gesture_tracker: GestureEventTracker::default(),
            motion: CursorMotionDriver::new(),
            last_motion: None,
            frames_submitted: 0,
            last_frame_submission_us: None,
            awaiting_surfaces: false,
            presentation_stalled: false,
            platform,
        };
        host.platform.prime()?;
        host.select_renderer()?;
        let bounds = host.pointer_tracking_bounds();
        host.platform.start_pointer_tracking(bounds);
        host.render_current()?;
        Ok(host)
    }

    pub(crate) fn handle_message(&mut self, message: OverlayHostMessage) -> OverlayHostReply {
        if message.version != OVERLAY_HOST_PROTOCOL_VERSION {
            return self.reply(
                false,
                vec![diagnostic(
                    "OverlayProtocolVersionMismatch",
                    "Overlay host protocol version mismatch.",
                    Some(format!(
                        "expected={} got={}",
                        OVERLAY_HOST_PROTOCOL_VERSION, message.version
                    )),
                )],
            );
        }

        match message.kind {
            OverlayHostMessageKind::Hello
            | OverlayHostMessageKind::Ping
            | OverlayHostMessageKind::Capabilities => {
                let _ = self.follow_tracked_pointer();
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::AnimateGesture => {
                let _ = self.follow_tracked_pointer();
                let (ok, gesture, diagnostics) =
                    crate::validate_gesture_message(message.gesture, &mut self.gesture_tracker);
                if ok && let Some(gesture) = gesture {
                    self.motion.start_gesture(MotionGesture {
                        kind: gesture.kind,
                        points: gesture
                            .points
                            .iter()
                            .map(|point| MotionPoint {
                                x: point.x as f32,
                                y: point.y as f32,
                            })
                            .collect(),
                        space: gesture.coordinate_space,
                        duration_ms: gesture.duration_ms,
                    });
                    return self.render_reply_with_diagnostics(diagnostics);
                }
                self.reply(ok, diagnostics)
            }
            OverlayHostMessageKind::WaitForArrival => self.wait_for_arrival(message.arrival_wait),
            OverlayHostMessageKind::Shutdown => {
                let _ = self.hide_visible_cursor();
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::SetCursor => {
                self.state = message.state;
                self.render_reply()
            }
            OverlayHostMessageKind::Hide => self.hide(message.sequence, message.reason),
            OverlayHostMessageKind::Show => {
                self.state = message.state;
                if let Some(state) = self.state.as_mut() {
                    state.visible = true;
                }
                self.platform.clear_capture_barrier();
                self.render_reply()
            }
        }
    }

    /// Hides the cursor. A sequence-bearing hide is a capture barrier: the
    /// reply is held until the platform confirms the hidden frame is what a
    /// capture would read, and carries the sequence as `applied_sequence`.
    fn hide(&mut self, sequence: Option<u64>, reason: Option<String>) -> OverlayHostReply {
        if let Some(state) = self.state.as_mut() {
            state.visible = false;
        }
        // A capture hide freezes the motion driver so the restore resumes
        // from the same pose; a plain hide drops the gesture pipeline and
        // marks the next show cold.
        self.motion.hide(sequence.is_some());
        // With no drawable surface there is nothing to hide and nothing to
        // confirm, so the barrier is not reported as applied.
        let barrier = sequence.filter(|_| self.active_surface_count() > 0);
        if barrier.is_some() {
            self.platform.begin_capture_barrier();
        }
        let mut reply = self.render_reply();
        if barrier.is_some() {
            if !reply.ok {
                // The hidden frame was never committed, so there is nothing
                // to wait for.
                self.platform.clear_capture_barrier();
            } else if let Err(error) = self.platform.wait_for_capture_barrier() {
                reply.ok = false;
                reply.diagnostics.push(diagnostic(
                    "OverlayCaptureBarrierTimeout",
                    "Overlay host capture barrier timed out before the hidden frame was applied.",
                    Some(error.to_string()),
                ));
            } else {
                // Reported only once the barrier holds.
                reply.applied_sequence = barrier;
            }
        }
        if let Some(reason) = reason.filter(|value| !value.trim().is_empty()) {
            reply.diagnostics.push(diagnostic(
                "OverlayCursorHidden",
                "Overlay host hid the cursor.",
                Some(reason),
            ));
        }
        reply
    }

    fn wait_for_arrival(&mut self, wait: Option<OverlayArrivalWaitRequest>) -> OverlayHostReply {
        let Some(wait) = wait else {
            return self.reply(
                false,
                vec![diagnostic(
                    "OverlayArrivalWaitMissing",
                    "WaitForArrival message did not include an arrival wait request.",
                    None,
                )],
            );
        };
        if wait.timeout_ms == 0 || wait.timeout_ms > MAX_ARRIVAL_WAIT_MS {
            return self.reply(
                false,
                vec![diagnostic(
                    "OverlayArrivalWaitInvalid",
                    "WaitForArrival timeout must be within the supported range.",
                    Some(format!(
                        "sequence={} timeout_ms={} max_ms={MAX_ARRIVAL_WAIT_MS}",
                        wait.sequence, wait.timeout_ms
                    )),
                )],
            );
        }

        let deadline = Instant::now() + Duration::from_millis(wait.timeout_ms);
        let mut unpresented_since: Option<Instant> = None;
        let outcome = loop {
            match self.arrival_status(wait) {
                ArrivalStatus::Complete(outcome) => break outcome,
                ArrivalStatus::Pending => {}
            }
            let now = Instant::now();
            if now >= deadline {
                break OverlayArrivalOutcome::DeadlineElapsed;
            }
            // Surfaces that stay unable to take a frame (a sleeping or locked
            // display) cannot prove an arrival. Say so promptly instead of
            // holding input for the whole deadline.
            if self.awaiting_surfaces {
                let waited = now.duration_since(*unpresented_since.get_or_insert(now));
                if self.presentation_stalled || waited >= PRESENT_WAIT {
                    self.presentation_stalled = true;
                    break OverlayArrivalOutcome::Unavailable;
                }
            } else {
                unpresented_since = None;
            }
            self.tick();
            if let ArrivalStatus::Complete(outcome) = self.arrival_status(wait) {
                break outcome;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break OverlayArrivalOutcome::DeadlineElapsed;
            }
            let pause = self.pointer_tick_interval().min(remaining);
            self.platform.wait(pause);
        };

        let mut diagnostics = Vec::new();
        match outcome {
            OverlayArrivalOutcome::Arrived => {}
            OverlayArrivalOutcome::DeadlineElapsed => diagnostics.push(diagnostic(
                "OverlayArrivalDeadlineElapsed",
                "Overlay host did not reach the requested arrival condition before its deadline.",
                Some(format!("sequence={}", wait.sequence)),
            )),
            OverlayArrivalOutcome::Superseded => diagnostics.push(diagnostic(
                "OverlayArrivalSuperseded",
                "Overlay arrival wait was superseded by a newer cursor action.",
                Some(format!("sequence={}", wait.sequence)),
            )),
            OverlayArrivalOutcome::Unavailable => diagnostics.push(diagnostic(
                "OverlayArrivalUnavailable",
                "Overlay host could not prove the requested arrival condition.",
                Some(format!("sequence={}", wait.sequence)),
            )),
        }
        let mut reply = self.reply(true, diagnostics);
        reply.arrival_wait = Some(OverlayArrivalWaitReply {
            sequence: wait.sequence,
            outcome,
        });
        reply
    }

    fn arrival_status(&self, wait: OverlayArrivalWaitRequest) -> ArrivalStatus {
        if !self.visible_overlay_supported()
            || !self.state.as_ref().is_some_and(|state| state.visible)
        {
            return ArrivalStatus::Complete(OverlayArrivalOutcome::Unavailable);
        }
        if self.awaiting_surfaces {
            // Nothing the driver reports has been shown yet.
            return ArrivalStatus::Pending;
        }
        match wait.condition {
            OverlayArrivalCondition::GestureFeedbackStarted => {
                let highest = self.gesture_tracker.highest_sequence;
                if highest > wait.sequence {
                    ArrivalStatus::Complete(OverlayArrivalOutcome::Superseded)
                } else if highest < wait.sequence {
                    ArrivalStatus::Complete(OverlayArrivalOutcome::Unavailable)
                } else if self.motion.pending_gesture_feedback() {
                    ArrivalStatus::Pending
                } else {
                    ArrivalStatus::Complete(OverlayArrivalOutcome::Arrived)
                }
            }
            OverlayArrivalCondition::MotionSettled => {
                let Some(state) = self.state.as_ref() else {
                    return ArrivalStatus::Complete(OverlayArrivalOutcome::Unavailable);
                };
                if state.sequence > wait.sequence {
                    ArrivalStatus::Complete(OverlayArrivalOutcome::Superseded)
                } else if state.sequence < wait.sequence {
                    ArrivalStatus::Complete(OverlayArrivalOutcome::Unavailable)
                } else if self.motion_status().is_some_and(|motion| motion.settled) {
                    ArrivalStatus::Complete(OverlayArrivalOutcome::Arrived)
                } else {
                    ArrivalStatus::Pending
                }
            }
        }
    }

    pub(crate) fn tick(&mut self) {
        let _ = self.follow_tracked_pointer();
        if self.should_animate() {
            let _ = self.render_current();
        }
    }

    /// Tick cadence matched to the fastest connected display while visible or
    /// animating, so the cursor updates at the panel's refresh rate (e.g.
    /// 4.2 ms on a 240 Hz screen) instead of a fixed 60 Hz; otherwise
    /// `IDLE_TICK_INTERVAL`. The refresh rate is clamped to [60, 240] Hz and
    /// defaults to 60 Hz when no refresh rate is advertised.
    pub(crate) fn pointer_tick_interval(&self) -> Duration {
        // Reads the platform's geometry snapshot: this is re-derived on every
        // timer fire and must not query the windowing system.
        let max_mhz = (0..self.platform.surface_count())
            .filter_map(|index| self.platform.slot(index).geometry)
            .filter_map(|geometry| geometry.refresh_mhz)
            .max();
        let hz = max_mhz
            .map(|mhz| (f64::from(mhz) / 1000.0).clamp(60.0, 240.0))
            .unwrap_or(60.0);
        let refresh_interval = Duration::from_secs_f64(1.0 / hz);
        let visible = self.state.as_ref().is_some_and(|state| state.visible);
        tick_interval_for(visible, self.should_animate(), refresh_interval)
    }

    /// Waits out `duration` between ticks, servicing the platform's native
    /// event loop if it has one.
    pub(crate) fn idle(&mut self, duration: Duration) {
        self.platform.wait(duration);
    }

    /// The platform, for the native event work its own backend wrapper does
    /// between requests.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }

    fn render_reply(&mut self) -> OverlayHostReply {
        self.render_reply_with_diagnostics(Vec::new())
    }

    fn render_reply_with_diagnostics(
        &mut self,
        mut diagnostics: Vec<DiagnosticEntry>,
    ) -> OverlayHostReply {
        match self.render_for_reply() {
            Ok(()) => self.reply(true, diagnostics),
            Err(error) => {
                let detail = error.to_string();
                self.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
                // A renderer that already failed closed keeps its reason:
                // from then on `detail` is that same reason coming back out
                // of `draw`, and wrapping it again on every request would
                // grow the reason, and each reply carrying it, without bound.
                if !matches!(self.renderer, SelectedRenderer::Unsupported { .. }) {
                    self.renderer = SelectedRenderer::Unsupported {
                        reason: Some(format!("overlay render failed closed: {detail}")),
                    };
                }
                let _ = self.platform.system_cursor_mut().set_hidden(false);
                self.platform.fail_closed();
                diagnostics.push(diagnostic(
                    "OverlayRenderFailed",
                    "Overlay host failed to render the agent cursor.",
                    Some(detail),
                ));
                self.reply(false, diagnostics)
            }
        }
    }

    /// Renders the frame a reply reports on. The reply's motion echo is what
    /// the service releases physical input on, so it must describe something
    /// a surface has actually shown: while the overlay should be visible but
    /// no surface can take a frame yet, the motion driver is not advanced and
    /// this waits, within `PRESENT_WAIT`, for the first frame that lands.
    /// Once such a wait has run out it is not repeated until a frame lands.
    fn render_for_reply(&mut self) -> Result<()> {
        let deadline = Instant::now() + PRESENT_WAIT;
        loop {
            if self.render_current()? == FrameOutcome::Settled {
                return Ok(());
            }
            if self.presentation_stalled || Instant::now() >= deadline {
                self.presentation_stalled = true;
                return Ok(());
            }
            self.platform.wait(PRESENT_POLL);
        }
    }

    fn render_current(&mut self) -> Result<FrameOutcome> {
        self.apply_surface_change();
        if self.open_surface_count() == 0 {
            bail!("overlay surfaces are closed");
        }
        let should_hide_system_cursor = self.visible_overlay_supported()
            && self.state.as_ref().is_some_and(|state| state.visible);
        self.platform
            .system_cursor_mut()
            .set_hidden(should_hide_system_cursor)
            .context("failed to update the system cursor adapter")?;
        let outcome = self.draw()?;
        self.awaiting_surfaces = outcome == FrameOutcome::AwaitingSurfaces;
        if !self.awaiting_surfaces {
            self.presentation_stalled = false;
        }
        // A hot-plug the platform saw while committing takes effect now, so
        // the reply for this frame already reports it.
        self.apply_surface_change();
        Ok(outcome)
    }

    fn apply_surface_change(&mut self) {
        match self.platform.take_surface_change() {
            SurfaceChange::None => {}
            SurfaceChange::CoverageLost => {
                self.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
            }
            SurfaceChange::Replaced => {
                // Assigning drops the old wgpu surfaces only after the new
                // ones exist; the platform keeps the replaced native surfaces
                // alive until its next change.
                self.surface_guards = create_surface_guards(&self.platform, &self.instance);
            }
        }
    }

    fn hide_visible_cursor(&mut self) -> Result<()> {
        if let Some(state) = self.state.as_mut() {
            state.visible = false;
        }
        self.render_current().map(|_| ())
    }

    fn follow_tracked_pointer(&mut self) -> Result<()> {
        if !self.state.as_ref().is_some_and(|state| state.visible) {
            return Ok(());
        }
        let Some(position) = self.platform.latest_pointer_position() else {
            return Ok(());
        };
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        if !state_needs_system_pointer_update(state, position) {
            return Ok(());
        }
        apply_system_pointer_position(state, position);
        self.render_current().map(|_| ())
    }

    fn reply(&self, ok: bool, diagnostics: Vec<DiagnosticEntry>) -> OverlayHostReply {
        OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok,
            capabilities: Some(self.capabilities()),
            lifecycle_state: Some(self.lifecycle_state),
            applied_sequence: None,
            state: self.state.clone(),
            motion: self.motion_status(),
            arrival_wait: None,
            diagnostics,
        }
    }

    fn draw(&mut self) -> Result<FrameOutcome> {
        let now_ms = current_epoch_ms();
        if matches!(self.renderer, SelectedRenderer::Wgpu(..)) {
            self.ensure_wgpu_output_coverage()
                .context("wgpu output coverage changed before rendering")?;
        }
        // The single motion-stepping site: every render path (tick timer,
        // message replies, pointer follow) funnels through draw(), so the
        // driver integrates exactly once per rendered frame.
        let visible = self.state.as_ref().is_some_and(|state| state.visible);
        let target = self
            .state
            .as_ref()
            .filter(|state| state.visible)
            .and_then(|state| {
                let point = state.native_point.as_ref().or(state.model_point.as_ref())?;
                let (x, y) = cursor_point(state)?;
                Some((x, y, point.coordinate_space.clone()))
            });
        // A renderer that failed closed presents nothing, so the platform is
        // not asked to bring its surfaces on screen for it.
        if let SelectedRenderer::Unsupported { reason } = &self.renderer {
            bail!(
                "{}",
                reason
                    .as_deref()
                    .unwrap_or("overlay renderer is unsupported")
            );
        }
        let presenting = self.platform.begin_frame(visible);
        if visible && !presenting {
            // The platform is still bringing its surfaces on screen. The
            // motion driver is not advanced for a frame nobody can see:
            // otherwise a cold show would snap, and a gesture would start its
            // feedback, before the cursor exists, and the service would
            // release input on that.
            self.platform.finish_frame()?;
            return Ok(FrameOutcome::AwaitingSurfaces);
        }
        let bounds_space = self
            .motion
            .upcoming_space(target.as_ref().map(|(_, _, space)| space.clone()));
        let bounds = self.motion_bounds(bounds_space.as_ref());
        let motion_frame = self.motion.step(MotionStepInput {
            now: Instant::now(),
            now_ms,
            visible,
            target,
            bounds,
        });
        let surface_count = self.platform.surface_count();
        let slots = (0..surface_count)
            .map(|index| self.platform.slot(index))
            .collect::<Vec<_>>();
        // The mover pose and the gesture scene are each in global
        // desktop-logical space; map them into EVERY output they reach (the
        // adapter translates unclipped and the shader clips per-pixel), so a
        // glide or a boundary-spanning ripple/trail renders continuously
        // across the monitor arrangement rather than popping at an edge.
        let cursor_pos = motion_frame.pos.filter(|_| visible);
        let cursors = (0..surface_count)
            .map(|index| {
                cursor_pos.and_then(|pos| {
                    self.cursor_for_surface(index, pos, motion_frame.space.as_ref())
                })
            })
            .collect::<Vec<_>>();
        let mut effects = (0..surface_count)
            .map(|index| self.feedback_scene_for_surface(index, &motion_frame))
            .collect::<Vec<_>>();
        let cursor_rotation_deg = motion_frame.rotation_deg;
        let cursor_cloud_alpha = motion_frame.cloud_alpha;
        self.last_motion = Some(motion_frame);
        // Frame-constant agent-in-control lease (see `Self::glow_active`): gates
        // the ambient edge glow / inward waves on every surface, distinct from
        // per-surface cursor presence.
        let glow_active = self.glow_active();

        if !presenting {
            // The overlay is hidden and the platform has its surfaces off
            // screen; there is nothing to present, but it still gets its
            // end-of-frame turn.
            self.platform.finish_frame()?;
            return Ok(FrameOutcome::Settled);
        }

        let mut requests: Vec<SurfaceDrawRequest> = Vec::with_capacity(surface_count);
        for (index, slot) in slots.iter().enumerate() {
            if !slot.active() {
                requests.push(None);
                continue;
            }
            let width = slot.width.max(1);
            let height = slot.height.max(1);
            // Per-output buffer scale: render at physical resolution so the
            // cursor and effects stay crisp on hidpi / fractionally-scaled
            // outputs.
            let render_scale = slot.geometry.map_or(1.0, |geometry| geometry.render_scale);
            self.platform
                .prepare_surface(index, width, height, render_scale);
            requests.push(Some(SurfaceDrawSpec {
                width,
                height,
                cursor: cursors[index],
                // `effects` is dead after this loop; move the scene out
                // instead of cloning its trail-points Vec every frame.
                effect: effects[index].take(),
                glow_active,
                // Per-output physical density so the edge glow sizes its rim
                // and containment band in millimetres regardless of DPI.
                px_per_mm: slot
                    .geometry
                    .map_or(FALLBACK_PX_PER_MM, |geometry| geometry.px_per_mm),
                render_scale,
                cursor_rotation_deg,
                cursor_cloud_alpha,
            }));
        }

        let frame_started = Instant::now();
        let presented = match &mut self.renderer {
            SelectedRenderer::Wgpu(renderer, _) => {
                renderer.draw(&mut self.surface_guards, &requests)?
            }
            SelectedRenderer::Unsupported { .. } => 0,
        };
        if presented > 0 {
            self.last_frame_submission_us = Some(frame_started.elapsed().as_micros());
            self.frames_submitted = self.frames_submitted.saturating_add(1);
        }

        self.platform.finish_frame()?;
        // Every surface skipped the frame (occluded): nothing was shown.
        Ok(if visible && presented == 0 {
            FrameOutcome::AwaitingSurfaces
        } else {
            FrameOutcome::Settled
        })
    }

    fn should_animate(&self) -> bool {
        // The driver keeps `animating` true while the mover is unsettled, a
        // gesture is pending arrival or playing feedback, or the cloud is
        // mid-bloom; pending gestures have no epoch expiry, so no duration
        // reaper may run here.
        self.last_motion
            .as_ref()
            .is_some_and(|frame| frame.animating)
            || self.glow_active()
    }

    /// The agent-in-control lease: a visible overlay state on a renderer that
    /// actually presents a visible overlay at full coverage. This is the desktop
    /// analogue of Android's `glowActive` and gates the ambient edge glow and
    /// inward waves. Deliberately distinct from per-surface cursor presence so
    /// the glow does not light merely because a one-shot effect is present.
    fn glow_active(&self) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| state.visible && self.visible_overlay_supported())
    }

    fn first_active_surface_index(&self) -> Option<usize> {
        (0..self.platform.surface_count()).find(|index| self.platform.slot(*index).active())
    }

    fn open_surface_count(&self) -> usize {
        (0..self.platform.surface_count())
            .filter(|index| self.platform.slot(*index).open)
            .count()
    }

    fn active_surface_count(&self) -> usize {
        (0..self.platform.surface_count())
            .filter(|index| self.platform.slot(*index).active())
            .count()
    }

    /// The structured motion echo for replies: where the vehicle-steered
    /// glyph actually is, so clients can assert glide behavior from fields
    /// instead of prose. `None` until the mover has ever been placed.
    fn motion_status(&self) -> Option<OverlayMotionStatus> {
        let frame = self.last_motion.as_ref()?;
        let pos = frame.pos?;
        Some(OverlayMotionStatus {
            x: f64::from(pos.x),
            y: f64::from(pos.y),
            heading_deg: f64::from(frame.heading_deg),
            speed: f64::from(frame.speed),
            // While no surface has taken the latest frame, the pose is one
            // nobody was shown (it may predate a hide), so it is not reported
            // as an arrival.
            settled: frame.settled && !self.awaiting_surfaces,
            pending_gesture_feedback: self.motion.pending_gesture_feedback()
                || self.awaiting_surfaces,
        })
    }
}

/// One wgpu surface per platform surface, index-aligned. A surface whose
/// handles or wgpu surface cannot be created is `None`, which the coverage
/// check reports instead of drawing a partial overlay.
fn create_surface_guards<P: SurfacePlatform>(
    platform: &P,
    instance: &WgpuOverlayInstance,
) -> Vec<Option<SurfaceGuard>> {
    (0..platform.surface_count())
        .map(|index| {
            let created = platform.raw_handles(index).and_then(|(display, window)| {
                SurfaceGuard::from_raw_handles(instance, display, window)
            });
            match created {
                Ok(guard) => Some(guard),
                Err(error) => {
                    eprintln!("sky-cua overlay: failed to create wgpu surface {index}: {error:#}");
                    None
                }
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
enum ArrivalStatus {
    Pending,
    Complete(OverlayArrivalOutcome),
}

/// The event-loop tick period: the display refresh period while the overlay
/// is visible or the motion driver is animating, otherwise the idle period.
pub(crate) fn tick_interval_for(
    visible: bool,
    animating: bool,
    refresh_interval: Duration,
) -> Duration {
    if visible || animating {
        refresh_interval
    } else {
        crate::IDLE_TICK_INTERVAL
    }
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crate::{
        cursor_asset,
        renderer::{CursorImage, draw_cursor_asset},
    };

    #[test]
    fn tick_interval_is_refresh_rate_while_visible_or_animating_and_idle_otherwise() {
        let refresh = std::time::Duration::from_micros(4_167); // 240 Hz
        assert_eq!(super::tick_interval_for(true, false, refresh), refresh);
        assert_eq!(super::tick_interval_for(false, true, refresh), refresh);
        assert_eq!(super::tick_interval_for(true, true, refresh), refresh);
        assert_eq!(
            super::tick_interval_for(false, false, refresh),
            crate::IDLE_TICK_INTERVAL
        );
        assert!(crate::IDLE_TICK_INTERVAL >= std::time::Duration::from_millis(50));
    }

    #[test]
    fn cursor_asset_draw_keeps_background_fully_transparent() {
        let cursor = CursorImage::load().expect("load cursor");
        let mut canvas = vec![0_u8; (cursor.width * cursor.height * 4) as usize];

        draw_cursor_asset(&mut canvas, cursor.width, cursor.height, &cursor, 0, 0);

        // Top-left corner stays transparent (it is now smoke margin); the
        // hotspot lands on the opaque body. The texture covers the glyph
        // footprint plus margin, so the scale divides by the FOOTPRINT width and
        // the hotspot is the footprint hotspot.
        let corner_alpha = canvas[3];
        let texture_scale = cursor.width / cursor_asset::AGENT_CURSOR_FOOTPRINT_WIDTH;
        let hotspot_x = cursor_asset::AGENT_CURSOR_FOOTPRINT_HOTSPOT_X as u32 * texture_scale;
        let hotspot_y = cursor_asset::AGENT_CURSOR_FOOTPRINT_HOTSPOT_Y as u32 * texture_scale;
        let hotspot_offset = ((hotspot_y * cursor.width + hotspot_x) * 4) as usize;
        assert_eq!(corner_alpha, 0);
        assert!(canvas[hotspot_offset + 3] > 0);
    }
}
