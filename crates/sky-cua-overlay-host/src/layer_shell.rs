use std::{
    ptr::NonNull,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPoint,
    AgentCursorPointerTrackingBackendKind, AgentCursorRendererBackendKind, AgentCursorState,
    CoordinateSpace, DiagnosticEntry,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_shm,
    output::{OutputHandler, OutputState},
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_region, wl_registry, wl_shm, wl_surface},
};

use crate::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    cursor_asset, diagnostic,
    pointer_tracking::{PointerTracker, PointerTrackingBounds},
    renderer::{
        CursorImage, CursorPoint, SurfaceDrawRequest, SurfaceDrawSpec, SurfaceGuard,
        WgpuOverlayInstance, WgpuOverlayRenderer, draw_cursor_asset,
    },
    system_cursor::{SystemCursorAdapter, SystemPointerPosition},
};

const INITIAL_ROUNDTRIPS: usize = 4;
const DEBUG_FILL_ENV: &str = "SKY_CUA_LAYER_SHELL_DEBUG_FILL";
const LAYER_ENV: &str = "SKY_CUA_LAYER_SHELL_LAYER";
const RENDERER_ENV: &str = "SKY_CUA_LAYER_SHELL_RENDERER";
const BUFFER_SLOTS_PER_LAYER: usize = 2;

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
        let shm = Shm::bind(&globals, &qh).context("wl_shm is unavailable")?;
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
                    buffer: None,
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

        let pool_size = layer_buffer_pool_size(cursor.width, cursor.height, layers.len())
            .context("failed to size initial layer-shell shared-memory pool")?;
        let pool = SlotPool::new(pool_size, &shm)
            .context("failed to create Wayland shared-memory pool")?;
        let app = LayerShellApp {
            renderer: LayerShellRenderer::Shm {
                reason: Some("layer-shell renderer has not been selected yet".to_string()),
            },
            shm,
            output_state,
            pool,
            layers,
            instance: Some(instance),
            surface_guards,
            cursor,
            state: None,
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
            | OverlayHostMessageKind::Capabilities
            | OverlayHostMessageKind::AnimateGesture => {
                let _ = self.follow_tracked_pointer();
                self.reply(true, Vec::new())
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
                let mut reply = self.render_reply();
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
                self.render_reply()
            }
        }
    }

    pub fn tick(&mut self) {
        let _ = self.follow_tracked_pointer();
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
        match self.render_current() {
            Ok(()) => self.reply(true, Vec::new()),
            Err(error) => self.reply(
                false,
                vec![diagnostic(
                    "OverlayRenderFailed",
                    "Layer-shell overlay failed to render the agent cursor.",
                    Some(error.to_string()),
                )],
            ),
        }
    }

    fn render_current(&mut self) -> Result<()> {
        if !self.app.has_open_layer() {
            bail!("layer-shell overlay surfaces are closed");
        }
        self.system_cursor
            .set_hidden(self.app.state.as_ref().is_some_and(|state| state.visible))
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
    AgentCursorCapabilities {
        backend: AgentCursorBackendKind::WaylandLayerShell,
        renderer_backend,
        visible_overlay: has_open_layer,
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
        ..Default::default()
    }
}

#[derive(Debug)]
struct LayerShellApp {
    renderer: LayerShellRenderer,
    shm: Shm,
    output_state: OutputState,
    pool: SlotPool,
    layers: Vec<LayerSurfaceEntry>,
    surface_guards: Vec<Option<SurfaceGuard>>,
    instance: Option<WgpuOverlayInstance>,
    cursor: CursorImage,
    state: Option<AgentCursorState>,
}

#[derive(Debug)]
struct LayerSurfaceEntry {
    output: Option<wl_output::WlOutput>,
    layer: LayerSurface,
    configured: bool,
    closed: bool,
    width: u32,
    height: u32,
    buffer: Option<Buffer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestedLayerShellRenderer {
    Auto,
    Wgpu,
    Shm,
}

fn requested_renderer() -> RequestedLayerShellRenderer {
    match std::env::var(RENDERER_ENV)
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "shm" | "wayland_shm" | "wayland-shm" => RequestedLayerShellRenderer::Shm,
        "wgpu" | "gpu" => RequestedLayerShellRenderer::Wgpu,
        "auto" | "" => RequestedLayerShellRenderer::Auto,
        _ => RequestedLayerShellRenderer::Auto,
    }
}

#[derive(Debug)]
enum LayerShellRenderer {
    Shm { reason: Option<String> },
    Wgpu(WgpuOverlayRenderer, String),
}

impl LayerShellRenderer {
    fn kind(&self) -> AgentCursorRendererBackendKind {
        match self {
            Self::Shm { .. } => AgentCursorRendererBackendKind::WaylandShm,
            Self::Wgpu(_, _) => AgentCursorRendererBackendKind::Wgpu,
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Shm { reason } => reason.as_deref(),
            Self::Wgpu(_, reason) => Some(reason.as_str()),
        }
    }
}

/// Extract a raw display handle from the Wayland connection.
///
/// # Safety
/// The returned handle is valid only while `conn` remains connected.
fn wayland_display_handle(conn: &Connection) -> Result<wgpu::rwh::RawDisplayHandle> {
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
fn wayland_window_handle(surface: &wl_surface::WlSurface) -> Result<wgpu::rwh::RawWindowHandle> {
    let surface_ptr = NonNull::new(surface.id().as_ptr() as *mut _)
        .context("Wayland surface pointer was null")?;
    Ok(wgpu::rwh::RawWindowHandle::Wayland(
        wgpu::rwh::WaylandWindowHandle::new(surface_ptr),
    ))
}

impl LayerShellApp {
    fn select_renderer(&mut self, _conn: &Connection) -> Result<()> {
        match requested_renderer() {
            RequestedLayerShellRenderer::Shm => {
                self.renderer = LayerShellRenderer::Shm {
                    reason: Some(format!("{RENDERER_ENV}=shm")),
                };
                Ok(())
            }
            RequestedLayerShellRenderer::Wgpu => {
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
                Ok(())
            }
            RequestedLayerShellRenderer::Auto => {
                if debug_fill_enabled() {
                    self.renderer = LayerShellRenderer::Shm {
                        reason: Some(format!(
                            "{DEBUG_FILL_ENV} is set, using shm renderer for debug fill support"
                        )),
                    };
                    return Ok(());
                }
                let instance = self.instance.as_ref().context("wgpu instance is missing")?;
                match WgpuOverlayRenderer::new(instance, &mut self.surface_guards, &self.cursor) {
                    Ok(renderer) => {
                        let reason = format!(
                            "wgpu renderer active on {} via {}",
                            renderer.info().adapter_name,
                            renderer.info().backend
                        );
                        self.renderer = LayerShellRenderer::Wgpu(renderer, reason);
                    }
                    Err(error) => {
                        self.renderer = LayerShellRenderer::Shm {
                            reason: Some(format!("wgpu unavailable, using shm fallback: {error}")),
                        };
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
        let visible_target = self
            .state
            .as_ref()
            .filter(|state| state.visible)
            .and_then(|state| self.cursor_target(state));

        let mut requests: Vec<SurfaceDrawRequest> = Vec::with_capacity(self.layers.len());
        for (index, entry) in self.layers.iter_mut().enumerate() {
            if entry.closed || !entry.configured {
                requests.push(None);
                continue;
            }
            let width = entry.width.max(1);
            let height = entry.height.max(1);
            entry.layer.set_size(width, height);
            entry.layer.set_margin(0, 0, 0, 0);
            entry
                .layer
                .wl_surface()
                .damage_buffer(0, 0, width as i32, height as i32);
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
            requests.push(Some(SurfaceDrawSpec {
                width,
                height,
                cursor,
            }));
        }

        match &mut self.renderer {
            LayerShellRenderer::Wgpu(renderer, _) => {
                renderer.draw(&mut self.surface_guards, &requests)?;
            }
            LayerShellRenderer::Shm { .. } => {
                self.draw_shm(qh, visible_target)?;
            }
        }

        for entry in self.layers.iter_mut() {
            if entry.closed || !entry.configured {
                continue;
            }
            entry.layer.commit();
        }
        Ok(())
    }

    fn draw_shm(
        &mut self,
        qh: &QueueHandle<Self>,
        visible_target: Option<LayerCursorTarget>,
    ) -> Result<()> {
        let layer_count = self.layers.len();

        for (index, entry) in self.layers.iter_mut().enumerate() {
            if entry.closed || !entry.configured {
                continue;
            }
            let visible_point = visible_target
                .as_ref()
                .filter(|target| target.layer_index == index)
                .map(|target| (target.x, target.y));
            let width = entry.width.max(1);
            let height = entry.height.max(1);
            entry.layer.set_size(width, height);
            entry.layer.set_margin(0, 0, 0, 0);

            let stride = width as i32 * 4;
            ensure_layer_pool_capacity(&mut self.pool, width, height, layer_count)
                .context("failed to resize layer-shell cursor shared-memory pool")?;
            let (buffer, canvas) = self
                .pool
                .create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                )
                .context("failed to create layer-shell cursor buffer")?;
            canvas.fill(0);
            if debug_fill_enabled() {
                draw_debug_fill(canvas, width, height, visible_point);
            }
            if let Some((x, y)) = visible_point {
                let left = x.round() as i32 - cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X;
                let top = y.round() as i32 - cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y;
                draw_cursor_asset(canvas, width, height, &self.cursor, left, top);
            }

            entry
                .layer
                .wl_surface()
                .damage_buffer(0, 0, width as i32, height as i32);
            entry
                .layer
                .wl_surface()
                .frame(qh, entry.layer.wl_surface().clone());
            buffer
                .attach_to(entry.layer.wl_surface())
                .context("failed to attach layer-shell cursor buffer")?;
            entry.layer.commit();
            entry.buffer = Some(buffer);
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

pub(crate) fn ensure_layer_pool_capacity(
    pool: &mut SlotPool,
    width: u32,
    height: u32,
    layer_count: usize,
) -> Result<()> {
    let required = layer_buffer_pool_size(width, height, layer_count)?;
    if pool.len() < required {
        pool.resize(required)?;
    }
    Ok(())
}

pub(crate) fn layer_buffer_pool_size(width: u32, height: u32, layer_count: usize) -> Result<usize> {
    let layer_count = layer_count.max(1);
    let bytes_per_buffer = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("layer-shell cursor buffer dimensions overflowed usize")?;
    bytes_per_buffer
        .checked_mul(layer_count)
        .and_then(|bytes| bytes.checked_mul(BUFFER_SLOTS_PER_LAYER))
        .map(|bytes| bytes.max(4096))
        .context("layer-shell cursor buffer pool size overflowed usize")
}

fn debug_fill_enabled() -> bool {
    std::env::var(DEBUG_FILL_ENV)
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "full" | "rect"))
}

fn draw_debug_fill(canvas: &mut [u8], width: u32, height: u32, visible_point: Option<(f64, f64)>) {
    let fill_full_surface =
        std::env::var(DEBUG_FILL_ENV).is_ok_and(|value| value.trim().eq_ignore_ascii_case("full"));
    let (left, top, right, bottom) = if fill_full_surface {
        (0, 0, width as i32, height as i32)
    } else if let Some((x, y)) = visible_point {
        let left = x.round() as i32 - 48;
        let top = y.round() as i32 - 48;
        (left, top, left + 96, top + 96)
    } else {
        (0, 0, 96, 96)
    };
    let color = ((u32::from(224_u8)) << 24)
        | ((u32::from(224_u8)) << 16)
        | ((u32::from(24_u8)) << 8)
        | u32::from(128_u8);
    let bytes = color.to_le_bytes();
    for y in top.max(0)..bottom.min(height as i32) {
        for x in left.max(0)..right.min(width as i32) {
            let x = u32::try_from(x).expect("nonnegative x");
            let y = u32::try_from(y).expect("nonnegative y");
            let offset = ((y * width + x) * 4) as usize;
            canvas[offset..offset + 4].copy_from_slice(&bytes);
        }
    }
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
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
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

impl ShmHandler for LayerShellApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
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
        _output: wl_output::WlOutput,
    ) {
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
delegate_shm!(LayerShellApp);

#[cfg(test)]
mod tests {
    use super::{
        BUFFER_SLOTS_PER_LAYER, RequestedLayerShellRenderer, apply_system_pointer_position,
        cursor_point, layer_buffer_pool_size, layer_shell_capabilities, output_local_point,
        requested_renderer, state_needs_system_pointer_update,
    };
    use crate::{
        cursor_asset,
        renderer::{CursorImage, draw_cursor_asset},
        system_cursor::{SystemCursorAdapter, SystemPointerPosition},
    };
    use sky_cua_platform::model::{
        AgentCursorBackendKind, AgentCursorPoint, AgentCursorPointerTrackingBackendKind,
        AgentCursorRendererBackendKind, AgentCursorState, AgentCursorSystemCursorBackendKind,
        CoordinateSpace,
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
    fn layer_pool_size_tracks_full_surface_buffers() {
        assert_eq!(
            layer_buffer_pool_size(1920, 1080, 2).expect("pool size"),
            1920 * 1080 * 4 * 2 * BUFFER_SLOTS_PER_LAYER
        );
    }

    #[test]
    fn cursor_asset_draw_keeps_background_fully_transparent() {
        let cursor = CursorImage::load().expect("load cursor");
        let mut canvas = vec![0_u8; (cursor.width * cursor.height * 4) as usize];

        draw_cursor_asset(&mut canvas, cursor.width, cursor.height, &cursor, 0, 0);

        // Top-left corner stays transparent; the hotspot lands on the opaque body.
        let corner_alpha = canvas[3];
        let hotspot_offset = ((cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y as u32 * cursor.width
            + cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X as u32)
            * 4) as usize;
        assert_eq!(corner_alpha, 0);
        assert_eq!(canvas[hotspot_offset + 3], 255);
    }

    #[test]
    fn layer_shell_capabilities_report_kwin_system_cursor_split_path() {
        let capabilities = layer_shell_capabilities(
            1,
            true,
            AgentCursorRendererBackendKind::Wgpu,
            Some("wgpu renderer active"),
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
    }

    #[test]
    fn layer_shell_renderer_env_selects_wgpu_and_shm() {
        unsafe { std::env::set_var(super::RENDERER_ENV, "wgpu") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Wgpu);
        unsafe { std::env::set_var(super::RENDERER_ENV, "shm") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Shm);
        unsafe { std::env::set_var(super::RENDERER_ENV, "auto") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Auto);
        unsafe { std::env::remove_var(super::RENDERER_ENV) };
    }
}
