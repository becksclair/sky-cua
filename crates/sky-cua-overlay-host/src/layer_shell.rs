//! The Wayland layer-shell platform for the overlay surface host.
//!
//! One click-through `zwlr_layer_shell_v1` surface per output. This module
//! owns the Wayland connection and event queue, the layer surfaces, the
//! per-output geometry snapshot, the frame-callback capture barrier, and the
//! compositor-specific system-cursor and pointer-telemetry adapters. The
//! protocol, motion, scene mapping, and rendering live in
//! [`crate::surface_host`].

use std::ptr::NonNull;

use anyhow::{Context, Result, bail};
use sky_cua_platform::model::AgentCursorBackendKind;
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
    cursor_asset,
    pointer_tracking::PointerTracker,
    surface_host::{
        OverlaySurfaceHost,
        geometry::{OutputGeometry, PointerTrackingBounds},
        platform::{
            PointerTrackingInfo, RendererRequest, SurfaceChange, SurfacePlatform, SurfaceSlot,
        },
    },
    system_cursor::{SystemCursorAdapter, SystemPointerPosition},
};

mod capture_barrier;
mod geometry;
mod wayland;

pub(crate) use geometry::output_render_scale;
use wayland::create_cursor_layer;
pub(crate) use wayland::{wayland_display_handle, wayland_window_handle};

const INITIAL_ROUNDTRIPS: usize = 4;
const RENDERER_ENV: &str = "SKY_CUA_LAYER_SHELL_RENDERER";

pub type LayerShellOverlayBackend = OverlaySurfaceHost<LayerShellPlatform>;

/// Connects to the Wayland display, creates one layer surface per output, and
/// starts the surface host on them.
pub fn connect() -> Result<LayerShellOverlayBackend> {
    OverlaySurfaceHost::start(LayerShellPlatform::connect()?)
}

#[derive(Debug)]
pub struct LayerShellPlatform {
    event_queue: wayland_client::EventQueue<LayerShellState>,
    state: LayerShellState,
    system_cursor: SystemCursorAdapter,
    pointer_tracker: PointerTracker,
    conn: Connection,
}

/// The Wayland dispatch state: everything the SCTK handlers mutate.
#[derive(Debug)]
struct LayerShellState {
    output_state: OutputState,
    layers: Vec<LayerSurfaceEntry>,
    /// Per-layer output geometry snapshot, index-aligned with `layers`.
    /// Rebuilt only on output events — SCTK's `OutputState::info()` clones
    /// the full `OutputInfo` per call, which the 60-240 Hz draw path must
    /// never pay per frame.
    output_geometry: Vec<Option<OutputGeometry>>,
    /// A sequence-bearing hide is waiting for its frame callbacks.
    capture_barrier_pending: bool,
    /// An output appeared after the layer surfaces were created.
    output_added: bool,
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

impl LayerShellPlatform {
    fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let (globals, event_queue) =
            registry_queue_init(&conn).context("failed to initialize Wayland registry")?;
        let qh = event_queue.handle();
        let compositor =
            CompositorState::bind(&globals, &qh).context("wl_compositor is unavailable")?;
        let layer_shell =
            LayerShell::bind(&globals, &qh).context("zwlr_layer_shell_v1 is unavailable")?;
        let output_state = OutputState::new(&globals, &qh);
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

        let mut state = LayerShellState {
            output_state,
            layers,
            output_geometry: Vec::new(),
            capture_barrier_pending: false,
            output_added: false,
        };
        state.refresh_output_geometry();
        Ok(Self {
            event_queue,
            state,
            system_cursor: SystemCursorAdapter::for_wayland_session(),
            pointer_tracker: PointerTracker::none("pointer tracker has not been selected yet"),
            conn,
        })
    }
}

impl SurfacePlatform for LayerShellPlatform {
    fn backend_kind(&self) -> AgentCursorBackendKind {
        AgentCursorBackendKind::WaylandLayerShell
    }

    fn describe(&self, open_surfaces: usize) -> String {
        format!("zwlr_layer_shell_v1 visible overlay active on {open_surfaces} output surface(s)")
    }

    fn surface_count(&self) -> usize {
        self.state.layers.len()
    }

    fn slot(&self, index: usize) -> SurfaceSlot {
        let Some(entry) = self.state.layers.get(index) else {
            return SurfaceSlot {
                open: false,
                ready: false,
                has_output: false,
                width: 1,
                height: 1,
                geometry: None,
            };
        };
        SurfaceSlot {
            open: !entry.closed,
            ready: !entry.closed && entry.configured,
            has_output: entry.output.is_some(),
            width: entry.width,
            height: entry.height,
            geometry: self.state.output_geometry.get(index).copied().flatten(),
        }
    }

    fn advertised_output_count(&self) -> usize {
        self.state.output_state.outputs().count()
    }

    fn raw_handles(
        &self,
        index: usize,
    ) -> Result<(wgpu::rwh::RawDisplayHandle, wgpu::rwh::RawWindowHandle)> {
        let entry = self
            .state
            .layers
            .get(index)
            .context("layer surface index is out of range")?;
        Ok((
            wayland_display_handle(&self.conn)?,
            wayland_window_handle(entry.layer.wl_surface())?,
        ))
    }

    fn requested_renderer(&self) -> RendererRequest {
        requested_renderer()
    }

    fn prime(&mut self) -> Result<()> {
        for _ in 0..INITIAL_ROUNDTRIPS {
            self.event_queue
                .roundtrip(&mut self.state)
                .context("Wayland roundtrip failed while priming layer-shell overlay")?;
            if self.state.has_configured_layer() || !self.state.has_open_layer() {
                break;
            }
        }
        if !self.state.has_open_layer() {
            bail!("layer-shell compositor closed all overlay surfaces during startup");
        }
        if !self.state.has_configured_layer() {
            bail!("layer-shell overlay surfaces were not configured by the compositor");
        }
        // SCTK announces the outputs that existed at connect through
        // `new_output` during these roundtrips. Those are the outputs the
        // layer surfaces were created for; only one that appears after
        // startup is a coverage change.
        self.state.output_added = false;
        Ok(())
    }

    fn take_surface_change(&mut self) -> SurfaceChange {
        if std::mem::take(&mut self.state.output_added) {
            SurfaceChange::CoverageLost
        } else {
            SurfaceChange::None
        }
    }

    fn begin_frame(&mut self, _visible: bool) -> bool {
        // A hidden layer surface still has to present a transparent frame.
        true
    }

    fn prepare_surface(&mut self, index: usize, width: u32, height: u32, render_scale: f32) {
        let qh = self.event_queue.handle();
        let Some(entry) = self.state.layers.get(index) else {
            return;
        };
        entry.layer.set_size(width, height);
        entry.layer.set_margin(0, 0, 0, 0);
        let surface = entry.layer.wl_surface();
        surface.set_buffer_scale((render_scale as i32).max(1));
        let buffer_w = (width as f32 * render_scale).round() as i32;
        let buffer_h = (height as f32 * render_scale).round() as i32;
        surface.damage_buffer(0, 0, buffer_w, buffer_h);
        surface.frame(&qh, surface.clone());
    }

    fn finish_frame(&mut self) -> Result<()> {
        for entry in &self.state.layers {
            if entry.closed || !entry.configured {
                continue;
            }
            entry.layer.commit();
        }
        self.event_queue
            .roundtrip(&mut self.state)
            .context("Wayland roundtrip failed after drawing layer-shell overlay")?;
        Ok(())
    }

    fn begin_capture_barrier(&mut self) {
        self.state.start_capture_barrier();
    }

    fn wait_for_capture_barrier(&mut self) -> Result<()> {
        capture_barrier::wait(&mut self.event_queue, &mut self.state)
    }

    fn clear_capture_barrier(&mut self) {
        self.state.clear_capture_barrier();
    }

    fn system_cursor(&self) -> &SystemCursorAdapter {
        &self.system_cursor
    }

    fn system_cursor_mut(&mut self) -> &mut SystemCursorAdapter {
        &mut self.system_cursor
    }

    fn start_pointer_tracking(&mut self, bounds: Option<PointerTrackingBounds>) {
        self.pointer_tracker = PointerTracker::for_wayland_session(bounds);
    }

    fn pointer_tracking(&self) -> PointerTrackingInfo<'_> {
        PointerTrackingInfo {
            backend: self.pointer_tracker.backend(),
            exact: self.pointer_tracker.exact(),
            reason: self.pointer_tracker.reason(),
        }
    }

    fn latest_pointer_position(&mut self) -> Option<SystemPointerPosition> {
        self.pointer_tracker.latest_position()
    }
}

impl LayerShellState {
    /// Rebuilds the per-layer output geometry snapshot. Called from output
    /// events only (plus once at connect); per-frame readers use the snapshot
    /// so the draw path never pays SCTK's `OutputInfo` clone.
    fn refresh_output_geometry(&mut self) {
        self.output_geometry = self
            .layers
            .iter()
            .map(|entry| {
                entry
                    .output
                    .as_ref()
                    .and_then(|output| self.output_state.info(output))
                    .map(|info| geometry::output_geometry(&info))
            })
            .collect();
    }

    fn has_open_layer(&self) -> bool {
        self.layers.iter().any(|entry| !entry.closed)
    }

    fn has_configured_layer(&self) -> bool {
        self.layers
            .iter()
            .any(|entry| !entry.closed && entry.configured)
    }
}

fn requested_renderer() -> RendererRequest {
    match std::env::var(RENDERER_ENV)
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "shm" | "wayland_shm" | "wayland-shm" => RendererRequest::Unsupported(format!(
            "{RENDERER_ENV}: Wayland SHM visible rendering was retired; WGPU is required"
        )),
        "wgpu" | "gpu" => RendererRequest::Wgpu,
        _ => RendererRequest::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::{RENDERER_ENV, requested_renderer};
    use crate::surface_host::platform::RendererRequest;

    #[test]
    fn layer_shell_renderer_env_selects_wgpu_and_rejects_legacy_shm() {
        unsafe { std::env::set_var(RENDERER_ENV, "wgpu") };
        assert_eq!(requested_renderer(), RendererRequest::Wgpu);
        unsafe { std::env::set_var(RENDERER_ENV, "shm") };
        assert!(matches!(
            requested_renderer(),
            RendererRequest::Unsupported(reason)
                if reason.contains("SHM visible rendering was retired")
        ));
        unsafe { std::env::set_var(RENDERER_ENV, "auto") };
        assert_eq!(requested_renderer(), RendererRequest::Auto);
        unsafe { std::env::remove_var(RENDERER_ENV) };
    }
}
