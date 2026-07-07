use std::{
    ptr::NonNull,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPoint,
    AgentCursorPointerTrackingBackendKind, AgentCursorRendererBackendKind, AgentCursorState,
    AgentOverlayCoverageKind, AgentOverlayEffectsCapabilities, AgentOverlayHostLifecycleState,
    CoordinateSpace, DiagnosticEntry,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output,
    output::{OutputHandler, OutputState},
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_region, wl_registry, wl_surface},
};

use crate::{
    GestureEventTracker, OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind,
    OverlayHostReply, OverlayMotionStatus, cursor_asset,
    cursor_motion::{
        CursorMotionDriver, MotionBounds, MotionFrame, MotionGesture, MotionStepInput,
    },
    diagnostic,
    motion::MotionPoint,
    pointer_tracking::{PointerTracker, PointerTrackingBounds},
    renderer::{
        CursorImage, CursorPoint, EffectScene, SurfaceDrawRequest, SurfaceDrawSpec, SurfaceGuard,
        WgpuOverlayInstance, WgpuOverlayRenderer,
    },
    system_cursor::{SystemCursorAdapter, SystemPointerPosition},
};

mod capabilities;
mod capture_barrier;
mod cursor_state;
mod geometry;
mod motion_adapter;
mod renderer_selection;
mod wayland;

use capture_barrier::CaptureBarrierState;
use cursor_state::{
    apply_system_pointer_position, cursor_point, state_needs_system_pointer_update,
};
use renderer_selection::LayerShellRenderer;

use wayland::create_cursor_layer;
pub(crate) use wayland::{wayland_display_handle, wayland_window_handle};

pub(crate) use geometry::output_render_scale;
use geometry::{FALLBACK_PX_PER_MM, OutputGeometry, box_reaches_output, translate_to_output_local};

const INITIAL_ROUNDTRIPS: usize = 4;

#[derive(Debug)]
pub struct LayerShellOverlayBackend {
    event_queue: wayland_client::EventQueue<LayerShellApp>,
    app: LayerShellApp,
    system_cursor: SystemCursorAdapter,
    pointer_tracker: PointerTracker,
    conn: Connection,
}

impl LayerShellOverlayBackend {
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let (globals, event_queue) =
            registry_queue_init(&conn).context("failed to initialize Wayland registry")?;
        let qh = event_queue.handle();
        let compositor =
            CompositorState::bind(&globals, &qh).context("wl_compositor is unavailable")?;
        let layer_shell =
            LayerShell::bind(&globals, &qh).context("zwlr_layer_shell_v1 is unavailable")?;
        let output_state = OutputState::new(&globals, &qh);
        let cursor = CursorImage::load()?;
        let outputs: Vec<Option<wl_output::WlOutput>> = {
            let advertised_outputs: Vec<_> = output_state.outputs().collect();
            if advertised_outputs.is_empty() {
                vec![None]
            } else {
                advertised_outputs.into_iter().map(Some).collect()
            }
        };
        let layers = outputs
            .into_iter()
            .map(|output| {
                let layer = create_cursor_layer(&compositor, &layer_shell, &qh, output.as_ref());
                LayerSurfaceEntry {
                    output,
                    layer,
                    configured: false,
                    closed: false,
                    width: 1,
                    height: 1,
                    capture_barrier_frames_remaining: 0,
                }
            })
            .collect::<Vec<_>>();

        let instance = WgpuOverlayInstance::new();
        let display_handle = wayland_display_handle(&conn)?;
        let mut surface_guards = Vec::with_capacity(layers.len());
        for entry in &layers {
            match wayland_window_handle(entry.layer.wl_surface()) {
                Ok(window_handle) => surface_guards.push(
                    SurfaceGuard::from_raw_handles(&instance, display_handle, window_handle).ok(),
                ),
                Err(error) => {
                    eprintln!("sky-cua layer-shell: failed to create surface guard: {error:#}");
                    surface_guards.push(None);
                }
            }
        }

        let mut app = LayerShellApp {
            renderer: LayerShellRenderer::Unsupported {
                reason: Some("layer-shell renderer has not been selected yet".to_string()),
            },
            output_state,
            layers,
            output_geometry: Vec::new(),
            instance: Some(instance),
            surface_guards,
            cursor,
            state: None,
            lifecycle_state: AgentOverlayHostLifecycleState::BackendInitializing,
            capture_barrier: None,
            gesture_tracker: GestureEventTracker::default(),
            motion: CursorMotionDriver::new(),
            last_motion: None,
            frames_submitted: 0,
            last_frame_submission_us: None,
        };
        app.refresh_output_geometry();
        let mut backend = Self {
            event_queue,
            app,
            system_cursor: SystemCursorAdapter::for_wayland_session(),
            pointer_tracker: PointerTracker::none("pointer tracker has not been selected yet"),
            conn,
        };
        backend.prime()?;
        backend.app.select_renderer(&backend.conn)?;
        backend.pointer_tracker =
            PointerTracker::for_wayland_session(backend.app.pointer_tracking_bounds());
        backend.render_current()?;
        Ok(backend)
    }

    pub fn handle_message(&mut self, message: OverlayHostMessage) -> OverlayHostReply {
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
                    crate::validate_gesture_message(message.gesture, &mut self.app.gesture_tracker);
                if ok && let Some(gesture) = gesture {
                    self.app.motion.start_gesture(MotionGesture {
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
            OverlayHostMessageKind::Shutdown => {
                let _ = self.hide_visible_cursor();
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::SetCursor => {
                self.app.state = message.state;
                self.render_reply()
            }
            OverlayHostMessageKind::Hide => {
                if let Some(state) = self.app.state.as_mut() {
                    state.visible = false;
                }
                // A capture hide (sequence-bearing barrier) freezes the motion
                // driver so the restore resumes from the same pose; a plain
                // hide drops the gesture pipeline and marks the next show cold.
                self.app.motion.hide(message.sequence.is_some());
                if let Some(sequence) = message.sequence {
                    self.app.start_capture_barrier(sequence);
                }
                let mut reply = self.render_reply();
                if message.sequence.is_some()
                    && let Err(error) = self.wait_for_capture_barrier()
                {
                    reply.ok = false;
                    reply.diagnostics.push(diagnostic(
                        "OverlayCaptureBarrierTimeout",
                        "Overlay host capture barrier timed out before the hidden frame was applied.",
                        Some(error.to_string()),
                    ));
                }
                if let Some(reason) = message.reason.filter(|value| !value.trim().is_empty()) {
                    reply.diagnostics.push(diagnostic(
                        "OverlayCursorHidden",
                        "Overlay host hid the cursor.",
                        Some(reason),
                    ));
                }
                reply
            }
            OverlayHostMessageKind::Show => {
                self.app.state = message.state;
                if let Some(state) = self.app.state.as_mut() {
                    state.visible = true;
                }
                self.app.clear_capture_barrier();
                self.render_reply()
            }
        }
    }

    pub fn tick(&mut self) {
        let _ = self.follow_tracked_pointer();
        if self.app.should_animate(current_epoch_ms()) {
            let _ = self.render_current();
        }
    }

    /// Tick cadence matched to the fastest connected display's current mode, so
    /// the agent-cursor follow updates at the panel's refresh rate (e.g. 4.2 ms
    /// on a 240 Hz screen) instead of a fixed 60 Hz. Clamped to [60, 240] Hz and
    /// defaulting to 60 Hz when no refresh rate is advertised.
    pub fn pointer_tick_interval(&self) -> std::time::Duration {
        // Reads the geometry snapshot, not OutputState: this re-arms on every
        // timer fire, and `OutputState::info` clones the full mode list.
        let max_mhz = self
            .app
            .output_geometry
            .iter()
            .flatten()
            .filter_map(|geometry| geometry.refresh_mhz)
            .max();
        let hz = max_mhz
            .map(|mhz| (f64::from(mhz) / 1000.0).clamp(60.0, 240.0))
            .unwrap_or(60.0);
        std::time::Duration::from_secs_f64(1.0 / hz)
    }

    fn prime(&mut self) -> Result<()> {
        for _ in 0..INITIAL_ROUNDTRIPS {
            self.event_queue
                .roundtrip(&mut self.app)
                .context("Wayland roundtrip failed while priming layer-shell overlay")?;
            if self.app.has_configured_layer() || !self.app.has_open_layer() {
                break;
            }
        }
        if !self.app.has_open_layer() {
            bail!("layer-shell compositor closed all overlay surfaces during startup");
        }
        if !self.app.has_configured_layer() {
            bail!("layer-shell overlay surfaces were not configured by the compositor");
        }
        Ok(())
    }

    fn render_reply(&mut self) -> OverlayHostReply {
        self.render_reply_with_diagnostics(Vec::new())
    }

    fn render_reply_with_diagnostics(
        &mut self,
        mut diagnostics: Vec<DiagnosticEntry>,
    ) -> OverlayHostReply {
        match self.render_current() {
            Ok(()) => self.reply(true, diagnostics),
            Err(error) => {
                let detail = error.to_string();
                self.app.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
                self.app.renderer = LayerShellRenderer::Unsupported {
                    reason: Some(format!("layer-shell render failed closed: {detail}")),
                };
                let _ = self.system_cursor.set_hidden(false);
                diagnostics.push(diagnostic(
                    "OverlayRenderFailed",
                    "Layer-shell overlay failed to render the agent cursor.",
                    Some(detail),
                ));
                self.reply(false, diagnostics)
            }
        }
    }

    fn render_current(&mut self) -> Result<()> {
        if !self.app.has_open_layer() {
            bail!("layer-shell overlay surfaces are closed");
        }
        let should_hide_system_cursor = self.app.visible_overlay_supported()
            && self.app.state.as_ref().is_some_and(|state| state.visible);
        self.system_cursor
            .set_hidden(should_hide_system_cursor)
            .context("failed to update layer-shell system cursor adapter")?;
        let qh = self.event_queue.handle();
        self.app.draw(&qh)?;
        self.event_queue
            .roundtrip(&mut self.app)
            .context("Wayland roundtrip failed after drawing layer-shell overlay")?;
        Ok(())
    }

    fn hide_visible_cursor(&mut self) -> Result<()> {
        if let Some(state) = self.app.state.as_mut() {
            state.visible = false;
        }
        self.render_current()
    }

    fn follow_tracked_pointer(&mut self) -> Result<()> {
        if !self.app.state.as_ref().is_some_and(|state| state.visible) {
            return Ok(());
        }
        let Some(position) = self.pointer_tracker.latest_position() else {
            return Ok(());
        };
        let Some(state) = self.app.state.as_mut() else {
            return Ok(());
        };
        if !state_needs_system_pointer_update(state, position) {
            return Ok(());
        }
        apply_system_pointer_position(state, position);
        self.render_current()
    }

    fn reply(&self, ok: bool, diagnostics: Vec<DiagnosticEntry>) -> OverlayHostReply {
        OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok,
            capabilities: Some(self.capabilities()),
            lifecycle_state: Some(self.app.lifecycle_state()),
            applied_sequence: self.app.applied_sequence(),
            state: self.app.state.clone(),
            motion: self.app.motion_status(),
            diagnostics,
        }
    }
}

#[derive(Debug)]
struct LayerShellApp {
    renderer: LayerShellRenderer,
    output_state: OutputState,
    layers: Vec<LayerSurfaceEntry>,
    /// Per-layer output geometry snapshot, index-aligned with `layers`.
    /// Rebuilt only on output events — SCTK's `OutputState::info()` clones
    /// the full `OutputInfo` per call, which the 60-240 Hz draw path must
    /// never pay per frame.
    output_geometry: Vec<Option<OutputGeometry>>,
    surface_guards: Vec<Option<SurfaceGuard>>,
    instance: Option<WgpuOverlayInstance>,
    cursor: CursorImage,
    state: Option<AgentCursorState>,
    lifecycle_state: AgentOverlayHostLifecycleState,
    capture_barrier: Option<CaptureBarrierState>,
    gesture_tracker: GestureEventTracker,
    /// The vehicle-steering motion driver: owns the drawn cursor pose between
    /// frames. `state` is only ever its target.
    motion: CursorMotionDriver,
    /// The driver's latest frame, for `should_animate` and the reply echo.
    last_motion: Option<MotionFrame>,
    frames_submitted: u64,
    last_frame_submission_us: Option<u128>,
}

#[derive(Debug)]
struct LayerSurfaceEntry {
    output: Option<wl_output::WlOutput>,
    layer: LayerSurface,
    configured: bool,
    closed: bool,
    width: u32,
    height: u32,
    capture_barrier_frames_remaining: u32,
}

impl LayerShellApp {
    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        let now_ms = current_epoch_ms();
        if matches!(self.renderer, LayerShellRenderer::Wgpu(..)) {
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
        // The mover pose and the gesture scene are each in global
        // desktop-logical space; map them into EVERY output they reach (the
        // adapter translates unclipped and the shader clips per-pixel), so a
        // glide or a boundary-spanning ripple/trail renders continuously
        // across the monitor arrangement rather than popping at an edge.
        let cursor_pos = motion_frame.pos.filter(|_| visible);
        let cursors = (0..self.layers.len())
            .map(|index| {
                cursor_pos
                    .and_then(|pos| self.cursor_for_layer(index, pos, motion_frame.space.as_ref()))
            })
            .collect::<Vec<_>>();
        let mut effects = (0..self.layers.len())
            .map(|index| self.feedback_scene_for_layer(index, &motion_frame))
            .collect::<Vec<_>>();
        let cursor_rotation_deg = motion_frame.rotation_deg;
        let cursor_cloud_alpha = motion_frame.cloud_alpha;
        self.last_motion = Some(motion_frame);
        // Per-output physical density so the edge glow sizes its rim and
        // containment band in millimetres regardless of monitor DPI.
        let px_per_mm = (0..self.layers.len())
            .map(|index| self.layer_px_per_mm(index))
            .collect::<Vec<_>>();
        // Per-output integer buffer scale: render at physical resolution so the
        // cursor and effects stay crisp on hidpi / fractionally-scaled outputs.
        let render_scale = (0..self.layers.len())
            .map(|index| self.layer_render_scale(index))
            .collect::<Vec<_>>();
        // Frame-constant agent-in-control lease (see `Self::glow_active`): gates
        // the ambient edge glow / inward waves on every surface, distinct from
        // per-surface cursor presence.
        let glow_active = self.glow_active();

        let mut requests: Vec<SurfaceDrawRequest> = Vec::with_capacity(self.layers.len());
        for (index, entry) in self.layers.iter_mut().enumerate() {
            if entry.closed || !entry.configured {
                requests.push(None);
                continue;
            }
            let width = entry.width.max(1);
            let height = entry.height.max(1);
            let scale = render_scale[index];
            entry.layer.set_size(width, height);
            entry.layer.set_margin(0, 0, 0, 0);
            entry
                .layer
                .wl_surface()
                .set_buffer_scale((scale as i32).max(1));
            let buffer_w = (width as f32 * scale).round() as i32;
            let buffer_h = (height as f32 * scale).round() as i32;
            entry
                .layer
                .wl_surface()
                .damage_buffer(0, 0, buffer_w, buffer_h);
            entry
                .layer
                .wl_surface()
                .frame(qh, entry.layer.wl_surface().clone());

            let cursor = cursors[index];
            // `effects` is dead after this loop; move the scene out instead
            // of cloning its trail-points Vec every frame.
            let effect = effects[index].take();
            requests.push(Some(SurfaceDrawSpec {
                width,
                height,
                cursor,
                effect,
                glow_active,
                px_per_mm: px_per_mm[index],
                render_scale: scale,
                cursor_rotation_deg,
                cursor_cloud_alpha,
            }));
        }

        let frame_started = Instant::now();
        match &mut self.renderer {
            LayerShellRenderer::Wgpu(renderer, _) => {
                renderer.draw(&mut self.surface_guards, &requests)?;
            }
            LayerShellRenderer::Unsupported { reason } => {
                bail!(
                    "{}",
                    reason
                        .as_deref()
                        .unwrap_or("layer-shell renderer is unsupported")
                );
            }
        }
        self.last_frame_submission_us = Some(frame_started.elapsed().as_micros());
        self.frames_submitted = self.frames_submitted.saturating_add(1);

        for entry in self.layers.iter_mut() {
            if entry.closed || !entry.configured {
                continue;
            }
            entry.layer.commit();
        }
        Ok(())
    }

    fn should_animate(&self, _now_ms: u64) -> bool {
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

    fn first_open_layer_index(&self) -> Option<usize> {
        self.layers
            .iter()
            .enumerate()
            .find_map(|(index, entry)| (!entry.closed && entry.configured).then_some(index))
    }

    fn open_layer_count(&self) -> usize {
        self.layers.iter().filter(|entry| !entry.closed).count()
    }

    fn has_open_layer(&self) -> bool {
        self.open_layer_count() > 0
    }

    fn has_configured_layer(&self) -> bool {
        self.layers
            .iter()
            .any(|entry| !entry.closed && entry.configured)
    }

    fn lifecycle_state(&self) -> AgentOverlayHostLifecycleState {
        self.lifecycle_state
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
            settled: frame.settled,
            pending_gesture_feedback: self.motion.pending_gesture_feedback(),
        })
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
