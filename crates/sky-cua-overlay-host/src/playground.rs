//! Interactive desktop pointer playground (Wayland layer-shell).
//!
//! The desktop analogue of the Android `PointerPlaygroundActivity`: a maximized,
//! input-capturing layer-shell surface that hides the system cursor and draws the
//! real agent cursor glyph wherever the pointer moves, so the computer-use pointer
//! can be previewed over live desktop content (transparent backdrop) or a
//! controlled backdrop (grid / dark / light) for contrast testing.
//!
//! Unlike the production overlay (an empty-input-region, click-through surface
//! driven over the serve protocol), this surface owns pointer input and runs its
//! own blocking event loop. It reuses the production `CursorImage` decode and
//! `draw_cursor_asset` blit so the previewed glyph is pixel-identical to the agent
//! cursor. Quit with Ctrl-C; the compositor restores the real cursor on exit.

use anyhow::{Context, Result, bail};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_seat,
    delegate_shm,
    output::{OutputHandler, OutputState},
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
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
    protocol::{wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_surface},
};

use crate::{
    cursor_asset,
    layer_shell::{
        CursorImage, draw_cursor_asset, ensure_layer_pool_capacity, layer_buffer_pool_size,
    },
};

const INITIAL_ROUNDTRIPS: usize = 4;
const GRID_CELL_PX: u32 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backdrop {
    /// Fully transparent: live desktop content shows through behind the cursor.
    Transparent,
    /// Opaque near-black surface for previewing the glyph on a dark background.
    Dark,
    /// Opaque light surface for previewing the glyph on a light background.
    Light,
    /// White surface with a light grid, mirroring the Android playground backdrop.
    Grid,
}

impl Backdrop {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "transparent" | "none" | "live" => Some(Self::Transparent),
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "grid" => Some(Self::Grid),
            _ => None,
        }
    }
}

/// Parse `playground [--backdrop transparent|grid|dark|light]` and run the loop.
pub(crate) fn run_from_args(args: Vec<String>) -> Result<()> {
    let mut backdrop = Backdrop::Transparent;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--backdrop" => {
                let value = iter
                    .next()
                    .context("--backdrop requires a value: transparent|grid|dark|light")?;
                backdrop = Backdrop::parse(&value)
                    .with_context(|| format!("unknown backdrop: {value}"))?;
            }
            other => bail!(
                "usage: sky-cua-overlay-host playground [--backdrop transparent|grid|dark|light] \
                 (got unexpected argument: {other})"
            ),
        }
    }
    run(backdrop)
}

fn run(backdrop: Backdrop) -> Result<()> {
    let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).context("failed to initialize Wayland registry")?;
    let qh = event_queue.handle();
    let compositor =
        CompositorState::bind(&globals, &qh).context("wl_compositor is unavailable")?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).context("zwlr_layer_shell_v1 is unavailable")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is unavailable")?;
    let output_state = OutputState::new(&globals, &qh);
    let seat_state = SeatState::new(&globals, &qh);
    let cursor = CursorImage::load()?;

    let outputs: Vec<Option<wl_output::WlOutput>> = {
        let advertised: Vec<_> = output_state.outputs().collect();
        if advertised.is_empty() {
            vec![None]
        } else {
            advertised.into_iter().map(Some).collect()
        }
    };
    let surfaces = outputs
        .into_iter()
        .map(|output| {
            let layer = create_playground_layer(&compositor, &layer_shell, &qh, output.as_ref());
            PlaygroundSurface {
                layer,
                configured: false,
                closed: false,
                width: 1,
                height: 1,
                buffer: None,
                pointer_pos: None,
            }
        })
        .collect::<Vec<_>>();

    let pool_size = layer_buffer_pool_size(cursor.width, cursor.height, surfaces.len())
        .context("failed to size playground shared-memory pool")?;
    let pool =
        SlotPool::new(pool_size, &shm).context("failed to create Wayland shared-memory pool")?;

    let mut app = PlaygroundApp {
        shm,
        output_state,
        seat_state,
        pool,
        surfaces,
        cursor,
        backdrop,
        pointer: None,
        needs_redraw: true,
    };

    // Prime: roundtrip until the compositor configures at least one surface.
    for _ in 0..INITIAL_ROUNDTRIPS {
        event_queue
            .roundtrip(&mut app)
            .context("Wayland roundtrip failed while priming the playground surface")?;
        if app.has_configured() || !app.has_open() {
            break;
        }
    }
    if !app.has_open() {
        bail!("layer-shell compositor closed all playground surfaces during startup");
    }
    if !app.has_configured() {
        bail!("layer-shell playground surfaces were not configured by the compositor");
    }

    app.draw(&qh)?;
    let _ = conn.flush();
    eprintln!(
        "sky-cua pointer playground: backdrop={backdrop:?} — move the mouse to preview the agent cursor; Ctrl-C to quit."
    );

    loop {
        event_queue
            .blocking_dispatch(&mut app)
            .context("Wayland dispatch failed in the playground loop")?;
        if app.needs_redraw {
            app.draw(&qh)?;
            app.needs_redraw = false;
            let _ = conn.flush();
        }
    }
}

struct PlaygroundApp {
    shm: Shm,
    output_state: OutputState,
    seat_state: SeatState,
    pool: SlotPool,
    surfaces: Vec<PlaygroundSurface>,
    cursor: CursorImage,
    backdrop: Backdrop,
    pointer: Option<wl_pointer::WlPointer>,
    needs_redraw: bool,
}

struct PlaygroundSurface {
    layer: LayerSurface,
    configured: bool,
    closed: bool,
    width: u32,
    height: u32,
    buffer: Option<Buffer>,
    /// Surface-local pointer position while the pointer is over this surface.
    pointer_pos: Option<(f64, f64)>,
}

impl PlaygroundApp {
    fn has_open(&self) -> bool {
        self.surfaces.iter().any(|surface| !surface.closed)
    }

    fn has_configured(&self) -> bool {
        self.surfaces
            .iter()
            .any(|surface| !surface.closed && surface.configured)
    }

    fn surface_index(&self, wl_surface: &wl_surface::WlSurface) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|surface| surface.layer.wl_surface() == wl_surface)
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        let surface_count = self.surfaces.len();
        let backdrop = self.backdrop;
        for surface in self.surfaces.iter_mut() {
            if surface.closed || !surface.configured {
                continue;
            }
            let width = surface.width.max(1);
            let height = surface.height.max(1);
            surface.layer.set_size(width, height);
            let stride = width as i32 * 4;
            ensure_layer_pool_capacity(&mut self.pool, width, height, surface_count)
                .context("failed to resize playground shared-memory pool")?;
            let (buffer, canvas) = self
                .pool
                .create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                )
                .context("failed to create playground buffer")?;
            draw_backdrop(canvas, width, height, backdrop);
            if let Some((x, y)) = surface.pointer_pos {
                let left = x.round() as i32 - cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X;
                let top = y.round() as i32 - cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y;
                draw_cursor_asset(canvas, width, height, &self.cursor, left, top);
            }
            surface
                .layer
                .wl_surface()
                .damage_buffer(0, 0, width as i32, height as i32);
            buffer
                .attach_to(surface.layer.wl_surface())
                .context("failed to attach playground buffer")?;
            surface.layer.commit();
            surface.buffer = Some(buffer);
        }
        let _ = qh;
        Ok(())
    }
}

fn create_playground_layer(
    compositor: &CompositorState,
    layer_shell: &LayerShell,
    qh: &QueueHandle<PlaygroundApp>,
    output: Option<&wl_output::WlOutput>,
) -> LayerSurface {
    let surface = compositor.create_surface(qh);
    let layer = layer_shell.create_layer_surface(
        qh,
        surface,
        Layer::Overlay,
        Some("sky-cua-pointer-playground"),
        output,
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT | Anchor::BOTTOM);
    // No keyboard focus is needed; Ctrl-C in the terminal quits the playground.
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(0);
    layer.set_size(0, 0);
    // Leave the default (full) input region so the surface captures pointer
    // motion — the inverse of the production overlay's empty, click-through region.
    layer.commit();
    layer
}

fn draw_backdrop(canvas: &mut [u8], width: u32, height: u32, backdrop: Backdrop) {
    match backdrop {
        Backdrop::Transparent => canvas.fill(0),
        Backdrop::Dark => fill_solid(canvas, argb(255, 0x1e, 0x1e, 0x2a)),
        Backdrop::Light => fill_solid(canvas, argb(255, 0xf0, 0xf0, 0xf0)),
        Backdrop::Grid => {
            fill_solid(canvas, argb(255, 255, 255, 255));
            let line = argb(255, 0xd2, 0xd2, 0xd2);
            let mut y = 0u32;
            while y < height {
                let row_start = (y as usize) * (width as usize) * 4;
                for x in 0..width as usize {
                    let offset = row_start + x * 4;
                    canvas[offset..offset + 4].copy_from_slice(&line);
                }
                y += GRID_CELL_PX;
            }
            let mut x = 0u32;
            while x < width {
                for yy in 0..height as usize {
                    let offset = (yy * width as usize + x as usize) * 4;
                    canvas[offset..offset + 4].copy_from_slice(&line);
                }
                x += GRID_CELL_PX;
            }
        }
    }
}

/// Pack an opaque color as premultiplied ARGB8888 little-endian bytes. Opaque
/// colors are unchanged by premultiplication.
fn argb(a: u8, r: u8, g: u8, b: u8) -> [u8; 4] {
    let color = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    color.to_le_bytes()
}

fn fill_solid(canvas: &mut [u8], bytes: [u8; 4]) {
    for pixel in canvas.chunks_exact_mut(4) {
        pixel.copy_from_slice(&bytes);
    }
}

impl SeatHandler for PlaygroundApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && self.pointer.is_none()
            && let Ok(pointer) = self.seat_state.get_pointer(qh, &seat)
        {
            self.pointer = Some(pointer);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.pointer = None;
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl PointerHandler for PlaygroundApp {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let index = self.surface_index(&event.surface);
            match &event.kind {
                PointerEventKind::Enter { serial } => {
                    // Hide the real system cursor over our surface; we draw the agent one.
                    pointer.set_cursor(*serial, None, 0, 0);
                    if let Some(index) = index {
                        self.surfaces[index].pointer_pos = Some(event.position);
                    }
                    self.needs_redraw = true;
                }
                PointerEventKind::Motion { .. } => {
                    if let Some(index) = index {
                        self.surfaces[index].pointer_pos = Some(event.position);
                    }
                    self.needs_redraw = true;
                }
                PointerEventKind::Leave { .. } => {
                    if let Some(index) = index {
                        self.surfaces[index].pointer_pos = None;
                    }
                    self.needs_redraw = true;
                }
                _ => {}
            }
        }
    }
}

impl CompositorHandler for PlaygroundApp {
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

impl LayerShellHandler for PlaygroundApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(surface) = self
            .surfaces
            .iter_mut()
            .find(|surface| &surface.layer == layer)
        {
            surface.closed = true;
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
        if let Some(surface) = self
            .surfaces
            .iter_mut()
            .find(|surface| &surface.layer == layer)
        {
            surface.configured = true;
            surface.width = configure.new_size.0.max(1);
            surface.height = configure.new_size.1.max(1);
            self.needs_redraw = true;
        }
    }
}

impl ShmHandler for PlaygroundApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl OutputHandler for PlaygroundApp {
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

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PlaygroundApp {
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

delegate_compositor!(PlaygroundApp);
delegate_layer!(PlaygroundApp);
delegate_output!(PlaygroundApp);
delegate_shm!(PlaygroundApp);
delegate_seat!(PlaygroundApp);
delegate_pointer!(PlaygroundApp);

#[cfg(test)]
mod tests {
    use super::Backdrop;

    #[test]
    fn backdrop_parses_known_aliases_and_rejects_unknown() {
        assert_eq!(Backdrop::parse("transparent"), Some(Backdrop::Transparent));
        assert_eq!(Backdrop::parse("LIVE"), Some(Backdrop::Transparent));
        assert_eq!(Backdrop::parse("grid"), Some(Backdrop::Grid));
        assert_eq!(Backdrop::parse(" Dark "), Some(Backdrop::Dark));
        assert_eq!(Backdrop::parse("light"), Some(Backdrop::Light));
        assert_eq!(Backdrop::parse("rainbow"), None);
    }
}
