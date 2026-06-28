use std::{
    ptr::NonNull,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPoint,
    AgentCursorPointerTrackingBackendKind, AgentCursorRendererBackendKind, AgentCursorState,
    AgentOverlayCoverageKind, AgentOverlayEffectsCapabilities, AgentOverlayGestureEvent,
    AgentOverlayHostLifecycleState, CoordinateSpace, DiagnosticEntry, Point2,
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
    OverlayHostReply, cursor_asset, diagnostic,
    pointer_tracking::{PointerTracker, PointerTrackingBounds},
    renderer::{
        CursorImage, CursorPoint, EffectScene, SurfaceDrawRequest, SurfaceDrawSpec, SurfaceGuard,
        WgpuOverlayInstance, WgpuOverlayRenderer,
    },
    system_cursor::{SystemCursorAdapter, SystemPointerPosition},
};

const INITIAL_ROUNDTRIPS: usize = 4;
const LAYER_ENV: &str = "SKY_CUA_LAYER_SHELL_LAYER";
const RENDERER_ENV: &str = "SKY_CUA_LAYER_SHELL_RENDERER";

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

        let app = LayerShellApp {
            renderer: LayerShellRenderer::Unsupported {
                reason: Some("layer-shell renderer has not been selected yet".to_string()),
            },
            output_state,
            layers,
            instance: Some(instance),
            surface_guards,
            cursor,
            state: None,
            lifecycle_state: AgentOverlayHostLifecycleState::BackendInitializing,
            capture_barrier: None,
            gesture_tracker: GestureEventTracker::default(),
            active_effect: None,
            frames_submitted: 0,
            last_frame_submission_us: None,
        };
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
                    self.app.start_effect(gesture);
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
                if let Some(sequence) = message.sequence {
                    self.app.start_capture_barrier(sequence);
                }
                let mut reply = self.render_reply();
                if message.sequence.is_some() {
                    if let Err(error) = self.wait_for_capture_barrier() {
                        reply.ok = false;
                        reply.diagnostics.push(diagnostic(
                            "OverlayCaptureBarrierTimeout",
                            "Overlay host capture barrier timed out before the hidden frame was applied.",
                            Some(error.to_string()),
                        ));
                    }
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
        let max_mhz = self
            .app
            .output_state
            .outputs()
            .filter_map(|output| self.app.output_state.info(&output))
            .flat_map(|info| info.modes)
            .filter(|mode| mode.current && mode.refresh_rate > 0)
            .map(|mode| mode.refresh_rate)
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

    fn wait_for_capture_barrier(&mut self) -> Result<()> {
        use std::time::{Duration, Instant};
        const BARRIER_TIMEOUT: Duration = Duration::from_millis(1500);
        let deadline = Instant::now() + BARRIER_TIMEOUT;
        while Instant::now() < deadline {
            if self.app.capture_barrier.is_none() {
                return Ok(());
            }
            self.event_queue
                .roundtrip(&mut self.app)
                .context("Wayland roundtrip failed while waiting for capture barrier")?;
            if self.app.capture_barrier.is_none() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.app.clear_capture_barrier();
        bail!("capture barrier timed out waiting for compositor frame acknowledgement")
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
            diagnostics,
        }
    }

    fn capabilities(&self) -> AgentCursorCapabilities {
        layer_shell_capabilities(
            self.app.open_layer_count(),
            self.app.has_open_layer(),
            self.app.renderer_kind(),
            self.app.renderer_reason(),
            self.app.coverage_kind(),
            self.app.active_output_count(),
            self.app.rendered_output_count(),
            self.app.adapter_name(),
            self.app.last_frame_submission_us,
            self.app.frames_submitted,
            self.pointer_tracker.backend(),
            self.pointer_tracker.exact(),
            self.pointer_tracker.reason(),
            &self.system_cursor,
        )
    }
}

fn layer_shell_capabilities(
    open_layer_count: usize,
    has_open_layer: bool,
    renderer_backend: AgentCursorRendererBackendKind,
    renderer_reason: Option<&str>,
    coverage: AgentOverlayCoverageKind,
    active_output_count: usize,
    rendered_output_count: usize,
    adapter_name: Option<&str>,
    last_frame_submission_us: Option<u128>,
    frames_submitted: u64,
    pointer_tracking_backend: AgentCursorPointerTrackingBackendKind,
    pointer_tracking_exact: bool,
    pointer_tracking_reason: Option<&str>,
    system_cursor: &SystemCursorAdapter,
) -> AgentCursorCapabilities {
    let mut reason = format!(
        "zwlr_layer_shell_v1 visible overlay active on {} output surface(s)",
        open_layer_count
    );
    if let Some(system_cursor_reason) = system_cursor.reason() {
        if system_cursor.supported() {
            reason.push_str("; system cursor: ");
        } else {
            reason.push_str("; system cursor hide unsupported: ");
        }
        reason.push_str(system_cursor_reason);
    }
    if let Some(renderer_reason) = renderer_reason {
        reason.push_str("; renderer: ");
        reason.push_str(renderer_reason);
    }
    if let Some(pointer_tracking_reason) = pointer_tracking_reason {
        reason.push_str("; pointer tracking: ");
        reason.push_str(pointer_tracking_reason);
    }
    if let Some(last_frame_submission_us) = last_frame_submission_us {
        reason.push_str(&format!(
            "; frame pacing: last_cpu_submit_us={last_frame_submission_us} frames_submitted={frames_submitted}"
        ));
    }
    let visible_overlay = has_open_layer
        && coverage == AgentOverlayCoverageKind::Full
        && renderer_backend == AgentCursorRendererBackendKind::Wgpu;
    AgentCursorCapabilities {
        backend: AgentCursorBackendKind::WaylandLayerShell,
        renderer_backend,
        visible_overlay,
        screenshot_synthetic_cursor: false,
        click_through: true,
        capture_exclusion: false,
        pointer_tracking_backend,
        pointer_tracking_exact,
        system_cursor_hide_supported: system_cursor.supported(),
        system_cursor_hidden: system_cursor.hidden(),
        system_cursor_backend: system_cursor.backend(),
        needs_user_install: false,
        reason: Some(reason),
        effects: Some(AgentOverlayEffectsCapabilities {
            glide: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            rotation: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            halo: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            ripple: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            trail: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            edge_glow: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            inward_wave: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            no_no_render: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            hit_test: false,
            sound: sky_cua_platform::overlay_spec::sound::ENABLED,
        }),
        coverage: Some(coverage),
        supported_coordinate_spaces: vec![
            CoordinateSpace::DesktopLogical,
            CoordinateSpace::StreamLogical,
            CoordinateSpace::StreamPixels,
        ],
        max_gesture_points: Some(
            sky_cua_platform::overlay_spec::shared::effects::MAX_GESTURE_POINTS,
        ),
        protocol_version: Some(OVERLAY_HOST_PROTOCOL_VERSION),
        effect_schema_version: Some(sky_cua_platform::overlay_spec::SCHEMA_VERSION),
        active_output_count: Some(active_output_count.min(u32::MAX as usize) as u32),
        rendered_output_count: Some(rendered_output_count.min(u32::MAX as usize) as u32),
        adapter_name: adapter_name.map(str::to_string),
        ..Default::default()
    }
}

#[derive(Debug)]
struct LayerShellApp {
    renderer: LayerShellRenderer,
    output_state: OutputState,
    layers: Vec<LayerSurfaceEntry>,
    surface_guards: Vec<Option<SurfaceGuard>>,
    instance: Option<WgpuOverlayInstance>,
    cursor: CursorImage,
    state: Option<AgentCursorState>,
    lifecycle_state: AgentOverlayHostLifecycleState,
    capture_barrier: Option<CaptureBarrierState>,
    gesture_tracker: GestureEventTracker,
    active_effect: Option<LayerEffectEvent>,
    frames_submitted: u64,
    last_frame_submission_us: Option<u128>,
}

#[derive(Debug, Clone, Copy)]
struct CaptureBarrierState {
    sequence: u64,
}

#[derive(Debug, Clone)]
struct LayerEffectEvent {
    gesture: AgentOverlayGestureEvent,
    started_at_ms: u64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestedLayerShellRenderer {
    Auto,
    Wgpu,
    UnsupportedLegacy(&'static str),
}

fn requested_renderer() -> RequestedLayerShellRenderer {
    match std::env::var(RENDERER_ENV)
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "shm" | "wayland_shm" | "wayland-shm" => RequestedLayerShellRenderer::UnsupportedLegacy(
            "Wayland SHM visible rendering was retired; WGPU is required",
        ),
        "wgpu" | "gpu" => RequestedLayerShellRenderer::Wgpu,
        "auto" | "" => RequestedLayerShellRenderer::Auto,
        _ => RequestedLayerShellRenderer::Auto,
    }
}

#[derive(Debug)]
enum LayerShellRenderer {
    Wgpu(WgpuOverlayRenderer, String),
    Unsupported { reason: Option<String> },
}

impl LayerShellRenderer {
    fn kind(&self) -> AgentCursorRendererBackendKind {
        match self {
            Self::Wgpu(_, _) => AgentCursorRendererBackendKind::Wgpu,
            Self::Unsupported { .. } => AgentCursorRendererBackendKind::None,
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Wgpu(_, reason) => Some(reason.as_str()),
            Self::Unsupported { reason } => reason.as_deref(),
        }
    }

    fn adapter_name(&self) -> Option<&str> {
        match self {
            Self::Wgpu(renderer, _) => Some(renderer.info().adapter_name.as_str()),
            Self::Unsupported { .. } => None,
        }
    }

    fn supports_visible_overlay(&self) -> bool {
        matches!(self, Self::Wgpu(..))
    }
}

/// Extract a raw display handle from the Wayland connection.
///
/// # Safety
/// The returned handle is valid only while `conn` remains connected.
/// Integer buffer scale for an output: `ceil(native_mode / logical_size)`,
/// clamped to `>= 1`. Rendering the surface at `logical * this` physical pixels
/// makes the compositor downsample a sharp buffer instead of upscaling a soft
/// logical one on hidpi / fractionally-scaled outputs. `1.0` when geometry is
/// unknown or the output is unscaled.
pub(crate) fn output_render_scale(info: &smithay_client_toolkit::output::OutputInfo) -> f32 {
    let Some((logical_w, logical_h)) = info.logical_size else {
        return 1.0;
    };
    let Some(mode) = info
        .modes
        .iter()
        .find(|mode| mode.current)
        .or(info.modes.first())
    else {
        return 1.0;
    };
    let (native_w, native_h) = mode.dimensions;
    if logical_w <= 0 || logical_h <= 0 || native_w <= 0 || native_h <= 0 {
        return 1.0;
    }
    let scale_x = native_w as f32 / logical_w as f32;
    let scale_y = native_h as f32 / logical_h as f32;
    scale_x.max(scale_y).ceil().max(1.0)
}

pub(crate) fn wayland_display_handle(conn: &Connection) -> Result<wgpu::rwh::RawDisplayHandle> {
    let display = NonNull::new(conn.backend().display_ptr() as *mut _)
        .context("Wayland display pointer was null")?;
    Ok(wgpu::rwh::RawDisplayHandle::Wayland(
        wgpu::rwh::WaylandDisplayHandle::new(display),
    ))
}

/// Extract a raw window handle from a `wl_surface`.
///
/// # Safety
/// The returned handle is valid only while `surface` remains alive.
pub(crate) fn wayland_window_handle(
    surface: &wl_surface::WlSurface,
) -> Result<wgpu::rwh::RawWindowHandle> {
    let surface_ptr = NonNull::new(surface.id().as_ptr() as *mut _)
        .context("Wayland surface pointer was null")?;
    Ok(wgpu::rwh::RawWindowHandle::Wayland(
        wgpu::rwh::WaylandWindowHandle::new(surface_ptr),
    ))
}

impl LayerShellApp {
    fn select_renderer(&mut self, _conn: &Connection) -> Result<()> {
        match requested_renderer() {
            RequestedLayerShellRenderer::UnsupportedLegacy(reason) => {
                self.renderer = LayerShellRenderer::Unsupported {
                    reason: Some(format!("{RENDERER_ENV}: {reason}")),
                };
                self.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
                Ok(())
            }
            RequestedLayerShellRenderer::Wgpu => {
                self.ensure_wgpu_output_coverage().context(
                    "explicit wgpu layer-shell renderer failed output coverage validation",
                )?;
                let instance = self.instance.as_ref().context("wgpu instance is missing")?;
                let renderer =
                    WgpuOverlayRenderer::new(instance, &mut self.surface_guards, &self.cursor)
                        .context("explicit wgpu layer-shell renderer failed to initialize")?;
                let reason = format!(
                    "wgpu renderer active on {} via {}",
                    renderer.info().adapter_name,
                    renderer.info().backend
                );
                self.renderer = LayerShellRenderer::Wgpu(renderer, reason);
                self.lifecycle_state = AgentOverlayHostLifecycleState::BackendReady;
                Ok(())
            }
            RequestedLayerShellRenderer::Auto => {
                let wgpu_result = self.ensure_wgpu_output_coverage().and_then(|()| {
                    let instance = self.instance.as_ref().context("wgpu instance is missing")?;
                    WgpuOverlayRenderer::new(instance, &mut self.surface_guards, &self.cursor)
                });
                match wgpu_result {
                    Ok(renderer) => {
                        let reason = format!(
                            "wgpu renderer active on {} via {}",
                            renderer.info().adapter_name,
                            renderer.info().backend
                        );
                        self.renderer = LayerShellRenderer::Wgpu(renderer, reason);
                        self.lifecycle_state = AgentOverlayHostLifecycleState::BackendReady;
                    }
                    Err(error) => {
                        self.renderer = LayerShellRenderer::Unsupported {
                            reason: Some(format!(
                                "wgpu unavailable; visible overlay failed closed: {error}"
                            )),
                        };
                        self.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
                    }
                }
                Ok(())
            }
        }
    }

    fn renderer_kind(&self) -> AgentCursorRendererBackendKind {
        self.renderer.kind()
    }

    fn renderer_reason(&self) -> Option<&str> {
        self.renderer.reason()
    }

    fn adapter_name(&self) -> Option<&str> {
        self.renderer.adapter_name()
    }

    fn visible_overlay_supported(&self) -> bool {
        self.renderer.supports_visible_overlay()
            && self.coverage_kind() == AgentOverlayCoverageKind::Full
    }

    fn active_output_count(&self) -> usize {
        self.output_state
            .outputs()
            .count()
            .max(self.layers.iter().filter(|entry| !entry.closed).count())
    }

    fn rendered_output_count(&self) -> usize {
        match self.renderer {
            LayerShellRenderer::Wgpu(..) => self
                .layers
                .iter()
                .enumerate()
                .filter(|(index, entry)| {
                    !entry.closed
                        && entry.configured
                        && self
                            .surface_guards
                            .get(*index)
                            .is_some_and(|guard| guard.is_some())
                })
                .count(),
            LayerShellRenderer::Unsupported { .. } => 0,
        }
    }

    fn coverage_kind(&self) -> AgentOverlayCoverageKind {
        let active_outputs = self.active_output_count();
        let rendered_outputs = self.rendered_output_count();
        if active_outputs > 0 && active_outputs == rendered_outputs {
            AgentOverlayCoverageKind::Full
        } else {
            AgentOverlayCoverageKind::None
        }
    }

    fn ensure_wgpu_output_coverage(&self) -> Result<()> {
        let active_outputs = self.active_output_count();
        let rendered_outputs = self
            .layers
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                !entry.closed
                    && entry.configured
                    && self
                        .surface_guards
                        .get(*index)
                        .is_some_and(|guard| guard.is_some())
            })
            .count();
        if active_outputs == 0 {
            bail!("no active Wayland outputs are available");
        }
        if active_outputs != rendered_outputs {
            bail!(
                "incomplete wgpu output coverage: active_outputs={active_outputs} rendered_outputs={rendered_outputs}"
            );
        }
        Ok(())
    }

    fn pointer_tracking_bounds(&self) -> Option<PointerTrackingBounds> {
        let mut left = i32::MAX;
        let mut top = i32::MAX;
        let mut right = i32::MIN;
        let mut bottom = i32::MIN;
        for entry in self
            .layers
            .iter()
            .filter(|entry| !entry.closed && entry.configured)
        {
            let Some(output) = entry.output.as_ref() else {
                left = left.min(0);
                top = top.min(0);
                right = right.max(i32::try_from(entry.width).unwrap_or(i32::MAX));
                bottom = bottom.max(i32::try_from(entry.height).unwrap_or(i32::MAX));
                continue;
            };
            let Some(info) = self.output_state.info(output) else {
                continue;
            };
            let position = info.logical_position.unwrap_or(info.location);
            let Some(size) = info.logical_size else {
                continue;
            };
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

    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        let now_ms = current_epoch_ms();
        self.clear_expired_effect(now_ms);
        if matches!(self.renderer, LayerShellRenderer::Wgpu(..)) {
            self.ensure_wgpu_output_coverage()
                .context("wgpu output coverage changed before rendering")?;
        }
        let visible_target = self
            .state
            .as_ref()
            .filter(|state| state.visible)
            .and_then(|state| self.cursor_target(state));
        let effects = (0..self.layers.len())
            .map(|index| self.effect_scene_for_layer(index, now_ms))
            .collect::<Vec<_>>();
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

            let cursor = visible_target
                .as_ref()
                .filter(|target| target.layer_index == index)
                .map(|target| CursorPoint {
                    x: target.x,
                    y: target.y,
                });
            let effect = effects[index].clone();
            requests.push(Some(SurfaceDrawSpec {
                width,
                height,
                cursor,
                effect,
                glow_active,
                px_per_mm: px_per_mm[index],
                render_scale: scale,
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

    fn cursor_target(&self, state: &AgentCursorState) -> Option<LayerCursorTarget> {
        let point = state.native_point.as_ref().or(state.model_point.as_ref())?;
        let (x, y) = cursor_point(state)?;
        if point.coordinate_space == CoordinateSpace::DesktopLogical
            && let Some(target) = self.desktop_logical_target(x, y)
        {
            return Some(target);
        }
        self.first_open_layer_index()
            .map(|layer_index| LayerCursorTarget { layer_index, x, y })
    }

    fn start_effect(&mut self, gesture: AgentOverlayGestureEvent) {
        self.active_effect = Some(LayerEffectEvent {
            gesture,
            started_at_ms: current_epoch_ms(),
        });
    }

    fn has_active_effect(&self, now_ms: u64) -> bool {
        self.active_effect.as_ref().is_some_and(|effect| {
            now_ms.saturating_sub(effect.started_at_ms) <= effect.gesture.duration_ms
        })
    }

    fn should_animate(&self, now_ms: u64) -> bool {
        self.has_active_effect(now_ms) || self.glow_active()
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

    fn clear_expired_effect(&mut self, now_ms: u64) {
        if !self.has_active_effect(now_ms) {
            self.active_effect = None;
        }
    }

    fn effect_scene_for_layer(&self, layer_index: usize, now_ms: u64) -> Option<EffectScene> {
        let active = self.active_effect.as_ref()?;
        if now_ms.saturating_sub(active.started_at_ms) > active.gesture.duration_ms {
            return None;
        }
        let first_point = active.gesture.points.first()?;
        let coordinate_space = active.gesture.coordinate_space.clone();
        let target = self.gesture_point_target(coordinate_space.clone(), first_point)?;
        if target.layer_index != layer_index {
            return None;
        }
        let points = active
            .gesture
            .points
            .iter()
            .filter_map(|point| {
                self.gesture_point_for_layer(layer_index, coordinate_space.clone(), point)
            })
            .collect::<Vec<_>>();
        if points.is_empty() {
            return None;
        }
        Some(EffectScene {
            kind: active.gesture.kind,
            started_at_ms: active.started_at_ms,
            duration_ms: active.gesture.duration_ms,
            points,
        })
    }

    fn gesture_point_target(
        &self,
        coordinate_space: CoordinateSpace,
        point: &Point2,
    ) -> Option<LayerCursorTarget> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return None;
        }
        if coordinate_space == CoordinateSpace::DesktopLogical
            && let Some(target) = self.desktop_logical_target(point.x, point.y)
        {
            return Some(target);
        }
        self.first_open_layer_index()
            .map(|layer_index| LayerCursorTarget {
                layer_index,
                x: point.x,
                y: point.y,
            })
    }

    fn gesture_point_for_layer(
        &self,
        layer_index: usize,
        coordinate_space: CoordinateSpace,
        point: &Point2,
    ) -> Option<CursorPoint> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return None;
        }
        if coordinate_space == CoordinateSpace::DesktopLogical {
            return self.desktop_logical_point_for_layer(layer_index, point.x, point.y);
        }
        Some(CursorPoint {
            x: point.x,
            y: point.y,
        })
    }

    /// Logical pixels per physical millimetre for a layer's output, from its
    /// `wl_output` physical size (mm) and logical size (px). Uses the diagonal
    /// so it stays correct under per-axis DPI differences and output rotation.
    /// Falls back to a representative logical density (~120 logical DPI) when
    /// the geometry is unknown or degenerate, so the edge glow always has a
    /// sane real-world scale.
    fn layer_px_per_mm(&self, index: usize) -> f32 {
        const FALLBACK_PX_PER_MM: f32 = 4.7;
        let Some(entry) = self.layers.get(index) else {
            return FALLBACK_PX_PER_MM;
        };
        let Some(output) = entry.output.as_ref() else {
            return FALLBACK_PX_PER_MM;
        };
        let Some(info) = self.output_state.info(output) else {
            return FALLBACK_PX_PER_MM;
        };
        let (phys_w_mm, phys_h_mm) = info.physical_size;
        let Some((logical_w, logical_h)) = info.logical_size else {
            return FALLBACK_PX_PER_MM;
        };
        let phys_diag_mm = ((phys_w_mm as f32).powi(2) + (phys_h_mm as f32).powi(2)).sqrt();
        let logical_diag_px = ((logical_w as f32).powi(2) + (logical_h as f32).powi(2)).sqrt();
        if phys_diag_mm < 1.0 || logical_diag_px < 1.0 {
            return FALLBACK_PX_PER_MM;
        }
        logical_diag_px / phys_diag_mm
    }

    /// Integer buffer scale for a layer's output (see [`output_render_scale`]).
    fn layer_render_scale(&self, index: usize) -> f32 {
        self.layers
            .get(index)
            .and_then(|entry| entry.output.as_ref())
            .and_then(|output| self.output_state.info(output))
            .map(|info| output_render_scale(&info))
            .unwrap_or(1.0)
    }

    fn desktop_logical_target(&self, x: f64, y: f64) -> Option<LayerCursorTarget> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.closed)
            .filter(|(_, entry)| entry.configured)
            .filter_map(|(layer_index, entry)| {
                let output = entry.output.as_ref()?;
                let info = self.output_state.info(output)?;
                let position = info.logical_position.unwrap_or(info.location);
                let size = info.logical_size?;
                output_local_point((x, y), position, size).map(|(x, y)| LayerCursorTarget {
                    layer_index,
                    x,
                    y,
                })
            })
            .next()
    }

    fn desktop_logical_point_for_layer(
        &self,
        layer_index: usize,
        x: f64,
        y: f64,
    ) -> Option<CursorPoint> {
        let entry = self.layers.get(layer_index)?;
        let output = entry.output.as_ref()?;
        let info = self.output_state.info(output)?;
        let position = info.logical_position.unwrap_or(info.location);
        let size = info.logical_size?;
        output_local_point((x, y), position, size).map(|(x, y)| CursorPoint { x, y })
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

    fn applied_sequence(&self) -> Option<u64> {
        self.capture_barrier.map(|barrier| barrier.sequence)
    }

    fn start_capture_barrier(&mut self, sequence: u64) {
        let frames = sky_cua_platform::overlay_spec::shared::effects::CAPTURE_BARRIER_FRAMES;
        let mut active_surfaces = 0;
        for entry in &mut self.layers {
            if !entry.closed && entry.configured {
                entry.capture_barrier_frames_remaining = frames;
                active_surfaces += 1;
            } else {
                entry.capture_barrier_frames_remaining = 0;
            }
        }
        self.capture_barrier = (active_surfaces > 0).then_some(CaptureBarrierState { sequence });
    }

    fn capture_barrier_complete(&self) -> bool {
        self.layers
            .iter()
            .filter(|entry| !entry.closed && entry.configured)
            .all(|entry| entry.capture_barrier_frames_remaining == 0)
    }

    fn clear_capture_barrier(&mut self) {
        self.capture_barrier = None;
        for entry in &mut self.layers {
            entry.capture_barrier_frames_remaining = 0;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LayerCursorTarget {
    layer_index: usize,
    x: f64,
    y: f64,
}

fn create_cursor_layer(
    compositor: &CompositorState,
    layer_shell: &LayerShell,
    qh: &QueueHandle<LayerShellApp>,
    output: Option<&wl_output::WlOutput>,
) -> LayerSurface {
    let surface = compositor.create_surface(qh);
    let layer = layer_shell.create_layer_surface(
        qh,
        surface,
        requested_layer(),
        Some("sky-cua-agent-cursor"),
        output,
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT | Anchor::BOTTOM);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // The production cursor overlay must cover compositor-global logical
    // coordinates, not the panel-constrained work area. KWin otherwise offsets
    // click-through layer surfaces away from exclusive panel edges.
    layer.set_exclusive_zone(-1);
    layer.set_size(0, 0);
    set_empty_input_region(compositor, &layer, qh);
    layer.commit();
    layer
}

fn requested_layer() -> Layer {
    match std::env::var(LAYER_ENV)
        .unwrap_or_else(|_| "overlay".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "background" => Layer::Background,
        "bottom" => Layer::Bottom,
        "top" => Layer::Top,
        "overlay" => Layer::Overlay,
        _ => Layer::Overlay,
    }
}

fn set_empty_input_region(
    compositor: &CompositorState,
    layer: &LayerSurface,
    qh: &QueueHandle<LayerShellApp>,
) {
    // An empty Wayland input region makes the overlay click-through.
    let region = compositor.wl_compositor().create_region(qh, ());
    layer.set_input_region(Some(&region));
    region.destroy();
}

fn cursor_point(state: &AgentCursorState) -> Option<(f64, f64)> {
    state
        .native_point
        .as_ref()
        .or(state.model_point.as_ref())
        .and_then(point_to_overlay_coordinates)
}

fn point_to_overlay_coordinates(point: &AgentCursorPoint) -> Option<(f64, f64)> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }
    match point.coordinate_space {
        CoordinateSpace::DesktopLogical
        | CoordinateSpace::StreamLogical
        | CoordinateSpace::StreamPixels => Some((point.x, point.y)),
    }
}

fn state_needs_system_pointer_update(
    state: &AgentCursorState,
    position: SystemPointerPosition,
) -> bool {
    let Some(point) = state.native_point.as_ref() else {
        return true;
    };
    if point.coordinate_space != CoordinateSpace::DesktopLogical {
        return true;
    }
    (point.x - position.x).abs() >= 0.5 || (point.y - position.y).abs() >= 0.5
}

fn apply_system_pointer_position(state: &mut AgentCursorState, position: SystemPointerPosition) {
    state.native_point = Some(AgentCursorPoint {
        x: position.x,
        y: position.y,
        coordinate_space: CoordinateSpace::DesktopLogical,
        mapping_id: None,
    });
    state.sequence = state.sequence.saturating_add(1);
    state.updated_at_ms = current_epoch_ms();
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn output_local_point(
    point: (f64, f64),
    position: (i32, i32),
    size: (i32, i32),
) -> Option<(f64, f64)> {
    if size.0 <= 0 || size.1 <= 0 {
        return None;
    }
    let x = point.0 - f64::from(position.0);
    let y = point.1 - f64::from(position.1);
    (x >= 0.0 && y >= 0.0 && x < f64::from(size.0) && y < f64::from(size.1)).then_some((x, y))
}

impl CompositorHandler for LayerShellApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if let Some(entry) = self
            .layers
            .iter_mut()
            .find(|entry| entry.layer.wl_surface().id() == surface.id())
        {
            if entry.capture_barrier_frames_remaining > 0 {
                entry.capture_barrier_frames_remaining =
                    entry.capture_barrier_frames_remaining.saturating_sub(1);
                if entry.capture_barrier_frames_remaining > 0 {
                    surface.frame(qh, surface.clone());
                    surface.commit();
                }
            }
        }
        if self.capture_barrier.is_some() && self.capture_barrier_complete() {
            self.capture_barrier = None;
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for LayerShellApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(entry) = self.layers.iter_mut().find(|entry| &entry.layer == layer) {
            entry.closed = true;
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if let Some(entry) = self.layers.iter_mut().find(|entry| &entry.layer == layer) {
            entry.configured = true;
            entry.width = if configure.new_size.0 == 0 {
                cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH
            } else {
                configure.new_size.0
            };
            entry.height = if configure.new_size.1 == 0 {
                cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT
            } else {
                configure.new_size.1
            };
        }
    }
}

impl OutputHandler for LayerShellApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        for entry in &mut self.layers {
            if entry
                .output
                .as_ref()
                .is_some_and(|entry_output| entry_output.id() == output.id())
            {
                entry.closed = true;
                entry.capture_barrier_frames_remaining = 0;
            }
        }
        if self.capture_barrier.is_some() && self.capture_barrier_complete() {
            self.capture_barrier = None;
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for LayerShellApp {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_region::WlRegion, ()> for LayerShellApp {
    fn event(
        _state: &mut Self,
        _region: &wl_region::WlRegion,
        _event: wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

delegate_compositor!(LayerShellApp);
delegate_layer!(LayerShellApp);
delegate_output!(LayerShellApp);

#[cfg(test)]
mod tests {
    use super::{
        OVERLAY_HOST_PROTOCOL_VERSION, RequestedLayerShellRenderer, apply_system_pointer_position,
        cursor_point, layer_shell_capabilities, output_local_point, requested_renderer,
        state_needs_system_pointer_update,
    };
    use crate::{
        cursor_asset,
        renderer::{CursorImage, draw_cursor_asset},
        system_cursor::{SystemCursorAdapter, SystemPointerPosition},
    };
    use sky_cua_platform::model::{
        AgentCursorBackendKind, AgentCursorPoint, AgentCursorPointerTrackingBackendKind,
        AgentCursorRendererBackendKind, AgentCursorState, AgentCursorSystemCursorBackendKind,
        AgentOverlayCoverageKind, CoordinateSpace,
    };

    #[test]
    fn cursor_point_prefers_native_coordinates_for_visible_overlay() {
        let state = AgentCursorState {
            visible: true,
            sequence: 1,
            model_point: Some(AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: None,
            }),
            native_point: Some(AgentCursorPoint {
                x: 100.0,
                y: 200.0,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
            }),
            snapshot_id: None,
            source_action: None,
            updated_at_ms: 1,
        };

        assert_eq!(cursor_point(&state), Some((100.0, 200.0)));
    }

    #[test]
    fn system_pointer_update_moves_visible_state_to_desktop_coordinates() {
        let mut state = AgentCursorState {
            visible: true,
            sequence: 7,
            model_point: Some(AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: Some("stream".to_string()),
            }),
            native_point: None,
            snapshot_id: None,
            source_action: None,
            updated_at_ms: 0,
        };
        let position = SystemPointerPosition { x: 300.0, y: 400.0 };

        assert!(state_needs_system_pointer_update(&state, position));
        apply_system_pointer_position(&mut state, position);

        assert_eq!(state.sequence, 8);
        assert_eq!(
            state.native_point,
            Some(AgentCursorPoint {
                x: 300.0,
                y: 400.0,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
            })
        );
        assert_eq!(cursor_point(&state), Some((300.0, 400.0)));
        assert!(!state_needs_system_pointer_update(
            &state,
            SystemPointerPosition {
                x: 300.25,
                y: 400.25
            }
        ));
        assert!(state_needs_system_pointer_update(
            &state,
            SystemPointerPosition { x: 301.0, y: 400.0 }
        ));
    }

    #[test]
    fn desktop_output_point_maps_to_output_local_coordinates() {
        assert_eq!(
            output_local_point((2020.0, 90.0), (1920, 0), (1280, 720)),
            Some((100.0, 90.0))
        );
        assert_eq!(
            output_local_point((1919.0, 90.0), (1920, 0), (1280, 720)),
            None
        );
        assert_eq!(
            output_local_point((3200.0, 90.0), (1920, 0), (1280, 720)),
            None
        );
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

    #[test]
    fn layer_shell_capabilities_report_kwin_system_cursor_split_path() {
        let capabilities = layer_shell_capabilities(
            1,
            true,
            AgentCursorRendererBackendKind::Wgpu,
            Some("wgpu renderer active"),
            AgentOverlayCoverageKind::Full,
            2,
            2,
            Some("llvmpipe"),
            Some(120),
            3,
            AgentCursorPointerTrackingBackendKind::KwinEffectSignal,
            true,
            Some("KWin signal tracker active"),
            &SystemCursorAdapter::test_kwin_effect(true),
        );

        assert_eq!(
            capabilities.backend,
            AgentCursorBackendKind::WaylandLayerShell
        );
        assert_eq!(
            capabilities.renderer_backend,
            AgentCursorRendererBackendKind::Wgpu
        );
        assert!(capabilities.visible_overlay);
        assert!(capabilities.click_through);
        assert_eq!(
            capabilities.pointer_tracking_backend,
            AgentCursorPointerTrackingBackendKind::KwinEffectSignal
        );
        assert!(capabilities.pointer_tracking_exact);
        assert!(capabilities.system_cursor_hide_supported);
        assert!(capabilities.system_cursor_hidden);
        assert_eq!(
            capabilities.system_cursor_backend,
            AgentCursorSystemCursorBackendKind::KwinEffect
        );
        assert_eq!(capabilities.coverage, Some(AgentOverlayCoverageKind::Full));
        assert_eq!(capabilities.active_output_count, Some(2));
        assert_eq!(capabilities.rendered_output_count, Some(2));
        assert_eq!(capabilities.adapter_name.as_deref(), Some("llvmpipe"));
        assert_eq!(
            capabilities.protocol_version,
            Some(OVERLAY_HOST_PROTOCOL_VERSION)
        );
        assert!(
            capabilities
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("last_cpu_submit_us=120")
        );
    }

    #[test]
    fn layer_shell_capabilities_fail_closed_for_incomplete_output_coverage() {
        let capabilities = layer_shell_capabilities(
            2,
            true,
            AgentCursorRendererBackendKind::Wgpu,
            Some("wgpu renderer active"),
            AgentOverlayCoverageKind::None,
            2,
            1,
            Some("llvmpipe"),
            None,
            0,
            AgentCursorPointerTrackingBackendKind::None,
            false,
            None,
            &SystemCursorAdapter::test_kwin_effect(false),
        );

        assert_eq!(capabilities.coverage, Some(AgentOverlayCoverageKind::None));
        assert_eq!(capabilities.active_output_count, Some(2));
        assert_eq!(capabilities.rendered_output_count, Some(1));
        assert!(!capabilities.visible_overlay);
    }

    #[test]
    fn layer_shell_renderer_env_selects_wgpu_and_rejects_legacy_shm() {
        unsafe { std::env::set_var(super::RENDERER_ENV, "wgpu") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Wgpu);
        unsafe { std::env::set_var(super::RENDERER_ENV, "shm") };
        assert!(matches!(
            requested_renderer(),
            RequestedLayerShellRenderer::UnsupportedLegacy(reason)
                if reason.contains("SHM visible rendering was retired")
        ));
        unsafe { std::env::set_var(super::RENDERER_ENV, "auto") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Auto);
        unsafe { std::env::remove_var(super::RENDERER_ENV) };
    }
}
