//! The seam between the platform-neutral surface host and a native windowing
//! adapter.
//!
//! The host owns everything that is the same on every desktop: the protocol,
//! the motion driver, the per-output scene mapping, the wgpu renderer, and
//! capability reporting. A [`SurfacePlatform`] owns only the native surfaces
//! (one per output), their geometry, the frame hand-off to the windowing
//! system, the hide-for-capture barrier mechanism, and the system-cursor and
//! pointer-telemetry adapters that exist on that desktop.

use std::time::Duration;

use anyhow::Result;
use sky_cua_platform::model::{AgentCursorBackendKind, AgentCursorPointerTrackingBackendKind};

use super::geometry::{OutputGeometry, PointerTrackingBounds};
use crate::system_cursor::{SystemCursorAdapter, SystemPointerPosition};

/// One native overlay surface as the host sees it for a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSlot {
    /// The native surface still exists.
    pub open: bool,
    /// The surface is open and the windowing system has made it drawable.
    pub ready: bool,
    /// The surface is bound to one output. `false` only for an output-less
    /// fallback surface, which takes desktop-logical points raw.
    pub has_output: bool,
    /// Surface size in logical pixels.
    pub width: u32,
    pub height: u32,
    /// Geometry of the output this surface covers, when known.
    pub geometry: Option<OutputGeometry>,
}

impl SurfaceSlot {
    /// Open and drawable: the surface takes part in rendering and coverage.
    pub fn active(&self) -> bool {
        self.open && self.ready
    }
}

/// Which renderer the platform's configuration asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
// Only the layer-shell platform has a renderer setting.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum RendererRequest {
    /// Use wgpu when it initializes; otherwise report unsupported.
    Auto,
    /// Require wgpu; an initialization failure is an error.
    Wgpu,
    /// A retired renderer was requested; the reason names it.
    Unsupported(String),
}

/// What changed in the native surface set since the host last asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum SurfaceChange {
    None,
    /// An output appeared that has no surface. The host fails closed.
    CoverageLost,
    /// The platform replaced every surface. The host rebuilds its wgpu
    /// surfaces from fresh raw handles and keeps its renderer.
    Replaced,
}

/// Structured pointer-telemetry state for capability reporting.
#[derive(Debug, Clone, Copy)]
pub struct PointerTrackingInfo<'a> {
    pub backend: AgentCursorPointerTrackingBackendKind,
    pub exact: bool,
    pub reason: Option<&'a str>,
}

pub trait SurfacePlatform: std::fmt::Debug {
    /// The backend kind this platform reports in capabilities.
    fn backend_kind(&self) -> AgentCursorBackendKind;

    /// Lead sentence of the capability reason, naming the native mechanism.
    fn describe(&self, open_surfaces: usize) -> String;

    fn surface_count(&self) -> usize;

    /// Cheap snapshot of one surface. Called several times per frame, so it
    /// must not allocate or query the windowing system.
    fn slot(&self, index: usize) -> SurfaceSlot;

    /// Outputs the windowing system currently advertises. It can exceed the
    /// surface count after a hot-plug the platform has not covered.
    fn advertised_output_count(&self) -> usize;

    /// Raw handles for one surface's wgpu surface.
    ///
    /// The platform keeps the native objects behind them alive until the host
    /// has dropped the wgpu surfaces, which the host does before it drops or
    /// replaces the platform's surfaces.
    fn raw_handles(
        &self,
        index: usize,
    ) -> Result<(wgpu::rwh::RawDisplayHandle, wgpu::rwh::RawWindowHandle)>;

    fn requested_renderer(&self) -> RendererRequest;

    /// Blocks until at least one surface is drawable, or fails.
    fn prime(&mut self) -> Result<()>;

    /// Reports and clears any surface-set change since the last call.
    fn take_surface_change(&mut self) -> SurfaceChange;

    /// Called once per frame before any surface is prepared. `visible` is
    /// whether the overlay holds a visible state. Returns whether the host
    /// should render this frame; a platform that takes its surfaces off
    /// screen while hidden returns `false` to skip the GPU work entirely.
    fn begin_frame(&mut self, visible: bool) -> bool;

    /// Per-surface hand-off before the renderer draws into it.
    fn prepare_surface(&mut self, index: usize, width: u32, height: u32, render_scale: f32);

    /// Called after the renderer has presented every surface: commit the
    /// frame to the windowing system and process its events.
    fn finish_frame(&mut self) -> Result<()>;

    /// Whether the windowing system keeps these surfaces out of screen
    /// captures while they stay on screen. When it does, the service does not
    /// hide the overlay around a capture.
    fn excluded_from_capture(&self) -> bool {
        false
    }

    /// The host failed closed after a render error and will present nothing
    /// more. A platform whose surfaces keep showing their last frame takes
    /// them off screen here.
    fn fail_closed(&mut self) {}

    /// Arms the hide-for-capture barrier for the frame about to be rendered.
    fn begin_capture_barrier(&mut self);

    /// Blocks until the hidden frame is what a capture would read, or fails
    /// after the platform's bound. A failure leaves the barrier cleared.
    fn wait_for_capture_barrier(&mut self) -> Result<()>;

    fn clear_capture_barrier(&mut self);

    fn system_cursor(&self) -> &SystemCursorAdapter;

    fn system_cursor_mut(&mut self) -> &mut SystemCursorAdapter;

    /// Starts pointer telemetry once the host knows the desktop bounds.
    fn start_pointer_tracking(&mut self, bounds: Option<PointerTrackingBounds>);

    fn pointer_tracking(&self) -> PointerTrackingInfo<'_>;

    /// The newest system pointer position since the last call, if telemetry
    /// is running and the pointer moved.
    fn latest_pointer_position(&mut self) -> Option<SystemPointerPosition>;

    /// Waits between frames: inside a held request (the arrival barrier),
    /// and between ticks in a serve loop that has no event source of its own.
    /// A platform with a native event loop services it here instead of
    /// sleeping.
    fn wait(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
