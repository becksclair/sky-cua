//! Wayland plumbing for the layer-shell host: surface creation, raw handle
//! extraction, and the SCTK handler/dispatch impls. Protocol state changes
//! land here (configure/close, output add/change/destroy, capture-barrier
//! frame callbacks); the output handlers rebuild the geometry snapshot the
//! per-frame paths read.

use super::*;

const LAYER_ENV: &str = "SKY_CUA_LAYER_SHELL_LAYER";

/// Extract a raw display handle from the Wayland connection.
///
/// # Safety
/// The returned handle is valid only while `conn` remains connected.
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
pub(super) fn create_cursor_layer(
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
        self.refresh_output_geometry();
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        // Geometry (position, size, scale, mode) may have changed; rebuild
        // the snapshot the per-frame paths read.
        self.refresh_output_geometry();
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
        self.refresh_output_geometry();
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
