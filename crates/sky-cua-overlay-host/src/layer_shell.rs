use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPoint, AgentCursorState,
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
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_region, wl_registry, wl_shm, wl_surface},
};

use crate::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    cursor_asset, diagnostic, system_cursor::SystemCursorAdapter,
};

const INITIAL_ROUNDTRIPS: usize = 4;
const DEBUG_FILL_ENV: &str = "SKY_CUA_LAYER_SHELL_DEBUG_FILL";
const LAYER_ENV: &str = "SKY_CUA_LAYER_SHELL_LAYER";
const BUFFER_SLOTS_PER_LAYER: usize = 2;

#[derive(Debug)]
pub struct LayerShellOverlayBackend {
    event_queue: wayland_client::EventQueue<LayerShellApp>,
    app: LayerShellApp,
    system_cursor: SystemCursorAdapter,
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

        let pool_size = layer_buffer_pool_size(cursor.width, cursor.height, layers.len())
            .context("failed to size initial layer-shell shared-memory pool")?;
        let pool = SlotPool::new(pool_size, &shm)
            .context("failed to create Wayland shared-memory pool")?;
        let app = LayerShellApp {
            shm,
            output_state,
            pool,
            layers,
            cursor,
            state: None,
        };
        let mut backend = Self {
            event_queue,
            app,
            system_cursor: SystemCursorAdapter::for_wayland_session(),
        };
        backend.prime()?;
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
            | OverlayHostMessageKind::Capabilities => self.reply(true, Vec::new()),
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
        let mut reason = format!(
            "zwlr_layer_shell_v1 visible overlay active on {} output surface(s)",
            self.app.open_layer_count()
        );
        if let Some(system_cursor_reason) = self.system_cursor.reason() {
            if self.system_cursor.supported() {
                reason.push_str("; system cursor: ");
            } else {
                reason.push_str("; system cursor hide unsupported: ");
            }
            reason.push_str(system_cursor_reason);
        }
        AgentCursorCapabilities {
            backend: AgentCursorBackendKind::WaylandLayerShell,
            visible_overlay: self.app.has_open_layer(),
            screenshot_synthetic_cursor: false,
            click_through: true,
            capture_exclusion: false,
            system_cursor_hide_supported: self.system_cursor.supported(),
            system_cursor_hidden: self.system_cursor.hidden(),
            system_cursor_backend: self.system_cursor.backend(),
            needs_user_install: false,
            reason: Some(reason),
        }
    }
}

#[derive(Debug)]
struct LayerShellApp {
    shm: Shm,
    output_state: OutputState,
    pool: SlotPool,
    layers: Vec<LayerSurfaceEntry>,
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

impl LayerShellApp {
    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        let visible_target = self
            .state
            .as_ref()
            .filter(|state| state.visible)
            .and_then(|state| self.cursor_target(state));
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
                let left = x.round() as i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_X;
                let top = y.round() as i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_Y;
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
    layer.set_exclusive_zone(0);
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

fn ensure_layer_pool_capacity(
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

fn layer_buffer_pool_size(width: u32, height: u32, layer_count: usize) -> Result<usize> {
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

fn draw_cursor_asset(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    cursor: &CursorImage,
    left: i32,
    top: i32,
) {
    for source_y in 0..cursor.height {
        let dest_y = top + i32::try_from(source_y).expect("cursor source y fits i32");
        if dest_y < 0 || dest_y >= i32::try_from(height).expect("surface height fits i32") {
            continue;
        }
        for source_x in 0..cursor.width {
            let dest_x = left + i32::try_from(source_x).expect("cursor source x fits i32");
            if dest_x < 0 || dest_x >= i32::try_from(width).expect("surface width fits i32") {
                continue;
            }
            let source_offset = ((source_y * cursor.width + source_x) * 4) as usize;
            let r = cursor.rgba[source_offset];
            let g = cursor.rgba[source_offset + 1];
            let b = cursor.rgba[source_offset + 2];
            let a = cursor.rgba[source_offset + 3];
            if a == 0 {
                continue;
            };
            let dest_x = u32::try_from(dest_x).expect("nonnegative destination x");
            let dest_y = u32::try_from(dest_y).expect("nonnegative destination y");
            let offset = ((dest_y * width + dest_x) * 4) as usize;
            let r = premultiply(r, a);
            let g = premultiply(g, a);
            let b = premultiply(b, a);
            let color = ((u32::from(a)) << 24)
                | ((u32::from(r)) << 16)
                | ((u32::from(g)) << 8)
                | u32::from(b);
            canvas[offset..offset + 4].copy_from_slice(&color.to_le_bytes());
        }
    }
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

fn premultiply(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}

#[derive(Debug)]
struct CursorImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl CursorImage {
    fn load() -> Result<Self> {
        let image = image::load_from_memory(cursor_asset::AGENT_CURSOR_PNG)
            .context("failed to decode bundled agent cursor image")?
            .to_rgba8();
        let (width, height) = image.dimensions();
        if width != cursor_asset::AGENT_CURSOR_SOURCE_WIDTH
            || height != cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT
        {
            bail!(
                "bundled agent cursor image changed size: expected {}x{} got {}x{}",
                cursor_asset::AGENT_CURSOR_SOURCE_WIDTH,
                cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT,
                width,
                height
            );
        }
        let image = image::imageops::resize(
            &image,
            cursor_asset::AGENT_CURSOR_WIDTH,
            cursor_asset::AGENT_CURSOR_HEIGHT,
            FilterType::Lanczos3,
        );
        let (width, height) = image.dimensions();
        Ok(Self {
            width,
            height,
            rgba: image.into_raw(),
        })
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
                cursor_asset::AGENT_CURSOR_WIDTH
            } else {
                configure.new_size.0
            };
            entry.height = if configure.new_size.1 == 0 {
                cursor_asset::AGENT_CURSOR_HEIGHT
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
        BUFFER_SLOTS_PER_LAYER, CursorImage, cursor_point, draw_cursor_asset,
        layer_buffer_pool_size, output_local_point,
    };
    use crate::cursor_asset;
    use sky_cua_platform::model::{AgentCursorPoint, AgentCursorState, CoordinateSpace};

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

        let corner_alpha = canvas[3];
        let source_black = (8 * cursor_asset::AGENT_CURSOR_WIDTH + 8) as usize * 4;
        let source_black_alpha = canvas[source_black + 3];
        assert_eq!(corner_alpha, 0);
        assert_eq!(source_black_alpha, 255);
    }
}
