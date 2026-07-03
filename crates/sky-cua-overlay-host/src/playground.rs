//! Interactive desktop effects playground (Wayland layer-shell + wgpu).
//!
//! The desktop analogue of the Android pointer playground: a maximized,
//! input-capturing layer-shell window that renders the *production effect
//! shader* (smoky edge glow, cursor halo, gesture ripple/trail/no-no, the agent
//! cursor glyph) so the computer-use overlay can be driven and reviewed live
//! without executing any real desktop input.
//!
//! Unlike the production overlay (an empty-input-region, click-through surface
//! driven over the serve protocol), this surface owns pointer input and runs its
//! own blocking event loop, reusing [`WgpuOverlayRenderer`] so what you see is
//! pixel-identical to the deployed overlay.
//!
//! Controls (pointer only; Ctrl-C in the terminal quits):
//! - move           — the cursor, its smoky halo, and the edge glow follow.
//! - left click      — tap (ripple + halo + cursor bounce).
//! - left drag       — drag (smoky trail + cursor rotation).
//! - right drag      — swipe (smoky trail).
//! - right click     — no-no (head-shake + mark).
//! - middle click    — toggle the ambient edge glow on/off.
//!
//! Backdrop is chosen at launch (`--backdrop transparent|dark|light`): a solid
//! canvas makes the pink effects pop, while `transparent` overlays the effects
//! on the live desktop.

use anyhow::{Context, Result, bail};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_seat,
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
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_pointer, wl_registry, wl_seat, wl_surface},
};

use crate::{
    cursor_motion::{CursorMotionDriver, MotionBounds, MotionGesture, MotionStepInput},
    layer_shell::{wayland_display_handle, wayland_window_handle},
    motion::MotionPoint,
    renderer::{
        CursorImage, CursorPoint, EffectScene, SurfaceDrawRequest, SurfaceDrawSpec, SurfaceGuard,
        WgpuOverlayInstance, WgpuOverlayRenderer,
    },
};
use sky_cua_platform::{
    model::{AgentOverlayGestureKind, CoordinateSpace},
    overlay_spec,
};

const INITIAL_ROUNDTRIPS: usize = 4;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
/// Pointer travel (surface px) beyond which a press→release reads as a drag
/// rather than a click.
const DRAG_THRESHOLD_PX: f64 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backdrop {
    /// Fully transparent: the effects render over live desktop content.
    Transparent,
    /// Opaque near-black canvas so the pink effects pop.
    Dark,
    /// Opaque light canvas for previewing on a bright background.
    Light,
}

impl Backdrop {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "transparent" | "none" | "live" => Some(Self::Transparent),
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }

    /// Render-pass clear color. Values are chosen to read dark/light after the
    /// surface's sRGB encoding; the exact shade is not load-bearing.
    fn clear_color(self) -> ::wgpu::Color {
        match self {
            Self::Transparent => ::wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            Self::Dark => ::wgpu::Color {
                r: 0.015,
                g: 0.015,
                b: 0.022,
                a: 1.0,
            },
            Self::Light => ::wgpu::Color {
                r: 0.80,
                g: 0.80,
                b: 0.82,
                a: 1.0,
            },
        }
    }
}

/// Parse `playground [--backdrop transparent|dark|light]` and run the loop.
pub(crate) fn run_from_args(args: Vec<String>) -> Result<()> {
    let mut backdrop = Backdrop::Dark;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--backdrop" => {
                let value = iter
                    .next()
                    .context("--backdrop requires a value: transparent|dark|light")?;
                backdrop = Backdrop::parse(&value)
                    .with_context(|| format!("unknown backdrop: {value}"))?;
            }
            other => bail!(
                "usage: sky-cua-overlay-host playground [--backdrop transparent|dark|light] \
                 (got unexpected argument: {other})"
            ),
        }
    }
    run(backdrop)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0)
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
                output,
                configured: false,
                closed: false,
                width: 1,
                height: 1,
            }
        })
        .collect::<Vec<_>>();

    // Build a wgpu surface guard per layer surface, mirroring the production
    // layer-shell host so the rendered effects are pixel-identical.
    let instance = WgpuOverlayInstance::new();
    let display_handle = wayland_display_handle(&conn)?;
    let mut surface_guards: Vec<Option<SurfaceGuard>> = Vec::with_capacity(surfaces.len());
    for surface in &surfaces {
        match wayland_window_handle(surface.layer.wl_surface()) {
            Ok(window_handle) => surface_guards.push(
                SurfaceGuard::from_raw_handles(&instance, display_handle, window_handle).ok(),
            ),
            Err(error) => {
                eprintln!("sky-cua playground: failed to create surface guard: {error:#}");
                surface_guards.push(None);
            }
        }
    }
    let mut renderer = WgpuOverlayRenderer::new(&instance, &mut surface_guards, &cursor)
        .context("failed to initialize wgpu renderer for the effects playground")?;
    renderer.set_clear_color(backdrop.clear_color());

    let mut app = PlaygroundApp {
        output_state,
        seat_state,
        surfaces,
        surface_guards,
        renderer,
        pointer: None,
        pointer_surface: None,
        cursor_pos: None,
        left_press: None,
        right_press: None,
        motion: CursorMotionDriver::new(),
        glow_active: true,
        needs_redraw: true,
    };

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

    eprintln!(
        "sky-cua effects playground: backdrop={backdrop:?}, renderer={} — move/click/drag to \
         drive the overlay; left=tap/drag, right=no-no/swipe, middle=toggle glow; Ctrl-C to quit.",
        app.renderer.info().backend
    );
    app.render(&qh)?;
    let _ = conn.flush();

    loop {
        event_queue
            .blocking_dispatch(&mut app)
            .context("Wayland dispatch failed in the playground loop")?;
        if !app.has_open() {
            break;
        }
        if app.needs_redraw {
            app.render(&qh)?;
            app.needs_redraw = false;
            let _ = conn.flush();
        }
    }
    Ok(())
}

struct PlaygroundApp {
    output_state: OutputState,
    seat_state: SeatState,
    surfaces: Vec<PlaygroundSurface>,
    surface_guards: Vec<Option<SurfaceGuard>>,
    renderer: WgpuOverlayRenderer,
    pointer: Option<wl_pointer::WlPointer>,
    /// Index of the surface the pointer is currently over.
    pointer_surface: Option<usize>,
    /// Surface-local pointer position on `pointer_surface`.
    cursor_pos: Option<(f64, f64)>,
    left_press: Option<(f64, f64)>,
    right_press: Option<(f64, f64)>,
    /// The production motion driver: the glyph glides after the physical
    /// pointer with the same vehicle steering, arrival-gated feedback, and
    /// trail resampling as the deployed overlay.
    motion: CursorMotionDriver,
    glow_active: bool,
    needs_redraw: bool,
}

struct PlaygroundSurface {
    layer: LayerSurface,
    output: Option<wl_output::WlOutput>,
    configured: bool,
    closed: bool,
    width: u32,
    height: u32,
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

    /// Logical pixels per physical millimetre for a surface's output (diagonal
    /// based, robust to per-axis DPI and rotation); falls back to a
    /// representative logical density when geometry is unknown.
    fn px_per_mm(&self, index: usize) -> f32 {
        const FALLBACK_PX_PER_MM: f32 = 4.7;
        let Some(output) = self.surfaces.get(index).and_then(|s| s.output.as_ref()) else {
            return FALLBACK_PX_PER_MM;
        };
        let Some(info) = self.output_state.info(output) else {
            return FALLBACK_PX_PER_MM;
        };
        let (phys_w_mm, phys_h_mm) = info.physical_size;
        let Some((logical_w, logical_h)) = info.logical_size else {
            return FALLBACK_PX_PER_MM;
        };
        let phys = ((phys_w_mm as f32).powi(2) + (phys_h_mm as f32).powi(2)).sqrt();
        let logical = ((logical_w as f32).powi(2) + (logical_h as f32).powi(2)).sqrt();
        if phys < 1.0 || logical < 1.0 {
            return FALLBACK_PX_PER_MM;
        }
        logical / phys
    }

    /// Integer buffer scale for a surface's output (see
    /// [`crate::layer_shell::output_render_scale`]).
    fn render_scale(&self, index: usize) -> f32 {
        self.surfaces
            .get(index)
            .and_then(|surface| surface.output.as_ref())
            .and_then(|output| self.output_state.info(output))
            .map(|info| crate::layer_shell::output_render_scale(&info))
            .unwrap_or(1.0)
    }

    /// Start a gesture animation at the given surface-local points. Drives only
    /// the overlay shader — no real desktop input is performed. The gesture
    /// runs the production pipeline: the glyph sails to the start point, the
    /// feedback (ripple/squash/trail) fires on arrival.
    fn start_gesture(&mut self, kind: AgentOverlayGestureKind, points: Vec<CursorPoint>) {
        use overlay_spec::shared::timing::{
            CLICK_FEEDBACK_MS, NO_NO_WIGGLE_MS, SWIPE_VISUAL_MIN_MS,
        };
        let duration_ms = match kind {
            AgentOverlayGestureKind::Tap => CLICK_FEEDBACK_MS,
            AgentOverlayGestureKind::NoNo => NO_NO_WIGGLE_MS,
            AgentOverlayGestureKind::Drag | AgentOverlayGestureKind::Swipe => SWIPE_VISUAL_MIN_MS,
        };
        self.motion.start_gesture(MotionGesture {
            kind,
            points: points
                .iter()
                .map(|point| MotionPoint {
                    x: point.x as f32,
                    y: point.y as f32,
                })
                .collect(),
            space: CoordinateSpace::StreamLogical,
            duration_ms,
        });
        self.needs_redraw = true;
    }

    fn on_button_release(&mut self, button: u32) {
        let Some(release) = self.cursor_pos else {
            self.left_press = None;
            self.right_press = None;
            return;
        };
        let release_point = CursorPoint {
            x: release.0,
            y: release.1,
        };
        match button {
            BTN_LEFT => {
                if let Some((sx, sy)) = self.left_press.take() {
                    if (release.0 - sx).hypot(release.1 - sy) > DRAG_THRESHOLD_PX {
                        self.start_gesture(
                            AgentOverlayGestureKind::Drag,
                            vec![CursorPoint { x: sx, y: sy }, release_point],
                        );
                    } else {
                        self.start_gesture(AgentOverlayGestureKind::Tap, vec![release_point]);
                    }
                }
            }
            BTN_RIGHT => {
                if let Some((sx, sy)) = self.right_press.take() {
                    if (release.0 - sx).hypot(release.1 - sy) > DRAG_THRESHOLD_PX {
                        self.start_gesture(
                            AgentOverlayGestureKind::Swipe,
                            vec![CursorPoint { x: sx, y: sy }, release_point],
                        );
                    } else {
                        self.start_gesture(AgentOverlayGestureKind::NoNo, vec![release_point]);
                    }
                }
            }
            BTN_MIDDLE => {
                self.glow_active = !self.glow_active;
                self.needs_redraw = true;
            }
            _ => {}
        }
    }

    fn render(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        // Step the production motion driver once per rendered frame, exactly
        // like the serve host's draw(): the physical pointer is the target and
        // the drawn glyph is the steered pursuit.
        let pointer_index = self.pointer_surface;
        let bounds = pointer_index
            .and_then(|index| self.surfaces.get(index))
            .map(|surface| MotionBounds {
                min_x: 0.0,
                min_y: 0.0,
                max_x: surface.width.max(1) as f32,
                max_y: surface.height.max(1) as f32,
            })
            .unwrap_or(MotionBounds {
                min_x: 0.0,
                min_y: 0.0,
                max_x: f32::MAX,
                max_y: f32::MAX,
            });
        let motion_frame = self.motion.step(MotionStepInput {
            now: Instant::now(),
            now_ms: now_ms(),
            visible: true,
            target: self
                .cursor_pos
                .map(|(x, y)| (x, y, CoordinateSpace::StreamLogical)),
            bounds,
        });
        let drawn_cursor = motion_frame.pos.map(|pos| CursorPoint {
            x: f64::from(pos.x),
            y: f64::from(pos.y),
        });
        let feedback_effect = motion_frame.feedback.as_ref().map(|feedback| EffectScene {
            kind: feedback.kind,
            started_at_ms: feedback.started_at_ms,
            duration_ms: feedback.duration_ms,
            points: feedback
                .scene_points()
                .iter()
                .map(|point| CursorPoint {
                    x: f64::from(point.x),
                    y: f64::from(point.y),
                })
                .collect(),
        });

        let mut requests: Vec<SurfaceDrawRequest> = Vec::with_capacity(self.surfaces.len());
        for (index, surface) in self.surfaces.iter().enumerate() {
            if surface.closed || !surface.configured {
                requests.push(None);
                continue;
            }
            let width = surface.width.max(1);
            let height = surface.height.max(1);
            // Render at physical buffer resolution so the cursor and effects are
            // crisp on hidpi / fractionally-scaled outputs.
            let render_scale = self.render_scale(index);
            surface.layer.set_size(width, height);
            surface
                .layer
                .wl_surface()
                .set_buffer_scale((render_scale as i32).max(1));
            let buffer_w = (width as f32 * render_scale).round() as i32;
            let buffer_h = (height as f32 * render_scale).round() as i32;
            surface
                .layer
                .wl_surface()
                .damage_buffer(0, 0, buffer_w, buffer_h);
            // Request a frame callback so the breathing glow and gesture
            // animations keep advancing without further input.
            surface
                .layer
                .wl_surface()
                .frame(qh, surface.layer.wl_surface().clone());

            let is_pointer_surface = pointer_index == Some(index);
            let cursor = if is_pointer_surface {
                drawn_cursor
            } else {
                None
            };
            let effect = if is_pointer_surface {
                feedback_effect.clone()
            } else {
                None
            };
            requests.push(Some(SurfaceDrawSpec {
                width,
                height,
                cursor,
                effect,
                glow_active: self.glow_active,
                px_per_mm: self.px_per_mm(index),
                render_scale,
                cursor_rotation_deg: motion_frame.rotation_deg,
                cursor_cloud_alpha: motion_frame.cloud_alpha,
            }));
        }

        self.renderer
            .draw(&mut self.surface_guards, &requests)
            .context("wgpu draw failed in the effects playground")?;
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
        Some("sky-cua-effects-playground"),
        output,
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT | Anchor::BOTTOM);
    // No keyboard focus is needed; pointer drives everything and Ctrl-C quits.
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(0);
    layer.set_size(0, 0);
    // Leave the default (full) input region so the surface captures pointer
    // input — the inverse of the production overlay's empty, click-through
    // region.
    layer.commit();
    layer
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
                    // Hide the system cursor over our surface; the shader draws
                    // the agent cursor glyph instead.
                    pointer.set_cursor(*serial, None, 0, 0);
                    self.pointer_surface = index;
                    self.cursor_pos = Some(event.position);
                    self.needs_redraw = true;
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_surface = index;
                    self.cursor_pos = Some(event.position);
                    self.needs_redraw = true;
                }
                PointerEventKind::Leave { .. } => {
                    if self.pointer_surface == index {
                        self.pointer_surface = None;
                        self.cursor_pos = None;
                    }
                    self.needs_redraw = true;
                }
                PointerEventKind::Press { button, .. } => {
                    self.pointer_surface = index;
                    self.cursor_pos = Some(event.position);
                    match *button {
                        BTN_LEFT => self.left_press = Some(event.position),
                        BTN_RIGHT => self.right_press = Some(event.position),
                        _ => {}
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    self.pointer_surface = index;
                    self.cursor_pos = Some(event.position);
                    self.on_button_release(*button);
                }
                PointerEventKind::Axis { .. } => {}
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
        // Drive continuous animation: every presented frame schedules the next.
        self.needs_redraw = true;
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
delegate_seat!(PlaygroundApp);
delegate_pointer!(PlaygroundApp);

#[cfg(test)]
mod tests {
    use super::Backdrop;

    #[test]
    fn backdrop_parses_known_aliases_and_rejects_unknown() {
        assert_eq!(Backdrop::parse("transparent"), Some(Backdrop::Transparent));
        assert_eq!(Backdrop::parse("LIVE"), Some(Backdrop::Transparent));
        assert_eq!(Backdrop::parse(" Dark "), Some(Backdrop::Dark));
        assert_eq!(Backdrop::parse("light"), Some(Backdrop::Light));
        assert_eq!(Backdrop::parse("grid"), None);
        assert_eq!(Backdrop::parse("rainbow"), None);
    }

    #[test]
    fn dark_and_light_backdrops_are_opaque_transparent_is_not() {
        assert_eq!(Backdrop::Transparent.clear_color().a, 0.0);
        assert_eq!(Backdrop::Dark.clear_color().a, 1.0);
        assert_eq!(Backdrop::Light.clear_color().a, 1.0);
    }
}
