//! The macOS panel platform for the overlay surface host.
//!
//! One borderless, non-activating `NSPanel` per screen: transparent,
//! click-through (`ignoresMouseEvents`), non-shareable (left out of screen
//! captures), above every normal window, present on every Space and beside
//! full-screen applications. Each panel's content view
//! carries the Metal layer wgpu renders into. This module owns the
//! application object, the panels, the per-screen geometry snapshot, and the
//! capture barrier. The protocol, motion, scene mapping, and rendering live in
//! [`crate::surface_host`].
//!
//! AppKit is main-thread only, so the platform exists only on the main thread
//! and the whole host (socket handling included) runs there. Only public
//! AppKit, CoreGraphics, and CoreFoundation interfaces are used.
//!
//! The host's coordinate space is `desktop_logical`: global points with a
//! top-left origin, the space of CoreGraphics display bounds, Accessibility,
//! and CGEvent. AppKit screen frames (bottom-left origin) are used only to
//! place each panel on its own screen; all scene geometry comes from
//! `CGDisplayBounds`.

use std::{
    cell::Cell,
    ptr::NonNull,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use block2::RcBlock;
use objc2::{
    MainThreadMarker, MainThreadOnly,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy,
    NSApplicationDidChangeScreenParametersNotification, NSBackingStoreType, NSColor, NSPanel,
    NSScreenSaverWindowLevel, NSView, NSWindowAnimationBehavior, NSWindowCollectionBehavior,
    NSWindowSharingType, NSWindowStyleMask,
};
use objc2_core_graphics::CGSessionCopyCurrentDictionary;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorPointerTrackingBackendKind,
    AgentCursorSystemCursorBackendKind,
};

use crate::{
    surface_host::{
        OverlaySurfaceHost,
        geometry::{OutputGeometry, PointerTrackingBounds},
        platform::{
            PointerTrackingInfo, RendererRequest, SurfaceChange, SurfacePlatform, SurfaceSlot,
        },
    },
    system_cursor::{SystemCursorAdapter, SystemPointerPosition},
};

mod display;
mod presence;
pub mod run_loop;

use display::ScreenInfo;
use presence::{FrameStart, PanelPresence};

/// How long a capture hide waits for the window server to report the panels
/// off screen.
const BARRIER_TIMEOUT: Duration = Duration::from_millis(1500);
const BARRIER_POLL_INTERVAL: Duration = Duration::from_millis(1);

const SYSTEM_CURSOR_REASON: &str = "macOS lets only the frontmost application hide the pointer through public interfaces, and the click-through overlay never becomes frontmost, so the system pointer stays visible";
const POINTER_TRACKING_REASON: &str =
    "not used on macOS: the system pointer stays visible, so the agent cursor does not follow it";

pub type MacosPanelOverlayBackend = OverlaySurfaceHost<MacosPanelPlatform>;

/// Creates one overlay panel per screen and starts the surface host on them.
/// Fails off the main thread and outside a window-server login session.
pub fn connect() -> Result<MacosPanelOverlayBackend> {
    OverlaySurfaceHost::start(MacosPanelPlatform::connect()?)
}

pub struct MacosPanelPlatform {
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    panels: Vec<OverlayPanel>,
    /// Panels a screen change replaced. They are already off screen and are
    /// kept only until the host has dropped the wgpu surfaces made from them.
    retired: Vec<OverlayPanel>,
    presence: PanelPresence,
    /// The visibility the current frame was begun with.
    frame_visible: bool,
    /// Set by the screen-parameters observer; consumed by the next frame.
    screens_changed: Rc<Cell<bool>>,
    screens_observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    system_cursor: SystemCursorAdapter,
}

struct OverlayPanel {
    panel: Retained<NSPanel>,
    view: Retained<NSView>,
    /// The panel's window-server identifier.
    window_id: Option<u32>,
    geometry: OutputGeometry,
    width: u32,
    height: u32,
}

impl std::fmt::Debug for MacosPanelPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacosPanelPlatform")
            .field(
                "screens",
                &self
                    .panels
                    .iter()
                    .map(|panel| panel.geometry)
                    .collect::<Vec<_>>(),
            )
            .field("presence", &self.presence)
            .finish_non_exhaustive()
    }
}

impl MacosPanelPlatform {
    fn connect() -> Result<Self> {
        let mtm = MainThreadMarker::new()
            .context("the macOS overlay panel must be created on the main thread")?;
        // Outside a login session (an SSH shell, a launch daemon) there is no
        // window server to connect to, and creating the application object
        // would abort the process instead of returning an error.
        if CGSessionCopyCurrentDictionary().is_none() {
            bail!("no window server session is available to this process");
        }
        let app = NSApplication::sharedApplication(mtm);
        // An accessory application has no Dock icon and no menu bar, and is
        // never activated by showing its panels.
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();

        let panels = create_panels(mtm)?;
        let screens_changed = Rc::new(Cell::new(false));
        let screens_observer = observe_screen_changes(Rc::clone(&screens_changed));
        Ok(Self {
            mtm,
            app,
            panels,
            retired: Vec::new(),
            presence: PanelPresence::OrderedOut,
            frame_visible: false,
            screens_changed,
            screens_observer,
            system_cursor: SystemCursorAdapter::unsupported_with_backend(
                AgentCursorSystemCursorBackendKind::MacosNative,
                SYSTEM_CURSOR_REASON,
            ),
        })
    }

    /// Delivers the application's queued events. The panels take no input, so
    /// this is housekeeping that keeps the window server's view of the process
    /// responsive.
    pub fn dispatch_app_events(&mut self) {
        run_loop::dispatch_app_events(&self.app);
    }

    fn order_out(&mut self) {
        for entry in &self.panels {
            entry.panel.orderOut(None);
        }
        self.presence = PanelPresence::OrderedOut;
    }

    fn window_ids(&self) -> Vec<u32> {
        self.panels
            .iter()
            .filter_map(|entry| entry.window_id)
            .collect()
    }

    /// Replaces every panel after the screen arrangement changed. Keeps the
    /// existing panels when no screen is attached, and retries on a later
    /// frame.
    fn rebuild_panels(&mut self) -> bool {
        let Ok(panels) = create_panels(self.mtm) else {
            return false;
        };
        self.order_out();
        self.retired = std::mem::replace(&mut self.panels, panels);
        true
    }
}

impl Drop for MacosPanelPlatform {
    fn drop(&mut self) {
        let observer: &AnyObject = (*self.screens_observer).as_ref();
        // SAFETY: the observer token came from this notification center.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(observer) };
        self.order_out();
    }
}

impl SurfacePlatform for MacosPanelPlatform {
    fn backend_kind(&self) -> AgentCursorBackendKind {
        AgentCursorBackendKind::MacosPanel
    }

    fn describe(&self, open_surfaces: usize) -> String {
        format!("macOS overlay panel active on {open_surfaces} screen(s)")
    }

    fn surface_count(&self) -> usize {
        self.panels.len()
    }

    fn slot(&self, index: usize) -> SurfaceSlot {
        let Some(entry) = self.panels.get(index) else {
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
            open: true,
            ready: true,
            has_output: true,
            width: entry.width,
            height: entry.height,
            geometry: Some(entry.geometry),
        }
    }

    fn advertised_output_count(&self) -> usize {
        // Every screen has a panel: a change in the screen set replaces the
        // panels before the next frame is drawn.
        self.panels.len()
    }

    fn raw_handles(
        &self,
        index: usize,
    ) -> Result<(wgpu::rwh::RawDisplayHandle, wgpu::rwh::RawWindowHandle)> {
        let entry = self
            .panels
            .get(index)
            .context("overlay panel index is out of range")?;
        let view = NonNull::from(&*entry.view).cast();
        Ok((
            wgpu::rwh::RawDisplayHandle::AppKit(wgpu::rwh::AppKitDisplayHandle::new()),
            wgpu::rwh::RawWindowHandle::AppKit(wgpu::rwh::AppKitWindowHandle::new(view)),
        ))
    }

    fn requested_renderer(&self) -> RendererRequest {
        RendererRequest::Auto
    }

    fn prime(&mut self) -> Result<()> {
        // A panel's content view is drawable as soon as it exists.
        Ok(())
    }

    fn take_surface_change(&mut self) -> SurfaceChange {
        if self.screens_changed.get() && self.rebuild_panels() {
            self.screens_changed.set(false);
            return SurfaceChange::Replaced;
        }
        SurfaceChange::None
    }

    fn begin_frame(&mut self, visible: bool) -> bool {
        // The host has replaced its wgpu surfaces by the time it draws.
        self.retired.clear();
        self.frame_visible = visible;
        let (presence, start) = self.presence.begin_frame(visible, || {
            // An unavailable window list must not keep the overlay dark; the
            // renderer's own occlusion check still applies.
            let window_ids = self.window_ids();
            display::windows_on_screen(&window_ids).is_none_or(|count| count == window_ids.len())
        });
        self.presence = presence;
        match start {
            FrameStart::Draw => true,
            FrameStart::Skip => false,
            FrameStart::OrderIn => {
                for entry in &self.panels {
                    entry.panel.orderFrontRegardless();
                }
                false
            }
        }
    }

    fn prepare_surface(&mut self, _index: usize, _width: u32, _height: u32, _render_scale: f32) {}

    fn finish_frame(&mut self) -> Result<()> {
        if self.presence.orders_out_after_frame(self.frame_visible) {
            self.order_out();
        }
        Ok(())
    }

    fn excluded_from_capture(&self) -> bool {
        // Every panel is created non-shareable; see `create_panel`.
        true
    }

    fn fail_closed(&mut self) {
        self.order_out();
    }

    fn begin_capture_barrier(&mut self) {}

    /// The hidden frame's `finish_frame` ordered the panels out. A capture
    /// reads the window server's window list, so the barrier holds once that
    /// list no longer has the panels on screen. AppKit commits window
    /// ordering to the window server when the run loop turns, so the wait
    /// runs the run loop instead of sleeping.
    fn wait_for_capture_barrier(&mut self) -> Result<()> {
        let window_ids = self.window_ids();
        let deadline = Instant::now() + BARRIER_TIMEOUT;
        loop {
            run_loop::run_main_loop_for(BARRIER_POLL_INTERVAL);
            // An unavailable window list confirms nothing, so it keeps
            // waiting and the bound reports the failure.
            if display::windows_on_screen(&window_ids) == Some(0) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "capture barrier timed out waiting for the overlay panels to leave the screen"
                );
            }
        }
    }

    fn clear_capture_barrier(&mut self) {}

    fn system_cursor(&self) -> &SystemCursorAdapter {
        &self.system_cursor
    }

    fn system_cursor_mut(&mut self) -> &mut SystemCursorAdapter {
        &mut self.system_cursor
    }

    fn start_pointer_tracking(&mut self, _bounds: Option<PointerTrackingBounds>) {}

    fn pointer_tracking(&self) -> PointerTrackingInfo<'_> {
        PointerTrackingInfo {
            backend: AgentCursorPointerTrackingBackendKind::None,
            exact: false,
            reason: Some(POINTER_TRACKING_REASON),
        }
    }

    fn latest_pointer_position(&mut self) -> Option<SystemPointerPosition> {
        None
    }

    fn wait(&mut self, duration: Duration) {
        run_loop::run_main_loop_for(duration);
        self.dispatch_app_events();
    }
}

fn create_panels(mtm: MainThreadMarker) -> Result<Vec<OverlayPanel>> {
    let screens = display::screens(mtm).context("an attached screen has no display identifier")?;
    if screens.is_empty() {
        bail!("no screen is attached");
    }
    screens
        .iter()
        .map(|screen| create_panel(mtm, screen))
        .collect()
}

fn create_panel(mtm: MainThreadMarker, screen: &ScreenInfo) -> Result<OverlayPanel> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        screen.frame,
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    // SAFETY: the panel is owned by a `Retained` and must not release itself
    // when closed.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setLevel(NSScreenSaverWindowLevel);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(true);
    // Deliberate: a non-shareable window is left out of screen captures, so
    // a capture never contains the overlay and the service does not have to
    // blink it off around every observation. The cost, accepted on purpose,
    // is that the overlay is also absent from the user's own screenshots,
    // recordings, and screen sharing. See "Capture exclusion" in
    // docs/features/agent-cursor-overlay.md.
    panel.setSharingType(NSWindowSharingType::None);
    // A panel hides when its application is inactive by default; this
    // application is never active.
    panel.setHidesOnDeactivate(false);
    panel.setCanHide(false);
    panel.setAnimationBehavior(NSWindowAnimationBehavior::None);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    // The initializer may place a borderless panel relative to the main
    // screen; pin it to its own screen's frame.
    panel.setFrame_display(screen.frame, false);
    let view = panel
        .contentView()
        .context("overlay panel has no content view")?;
    let (width, height) = screen
        .geometry
        .logical_size
        .context("screen has no logical size")?;
    Ok(OverlayPanel {
        window_id: u32::try_from(panel.windowNumber()).ok(),
        panel,
        view,
        geometry: screen.geometry,
        width: u32::try_from(width).unwrap_or(1).max(1),
        height: u32::try_from(height).unwrap_or(1).max(1),
    })
}

/// Marks `changed` whenever a display is attached, detached, rearranged, or
/// changes mode. The notification is delivered on the main thread, where the
/// flag is read.
fn observe_screen_changes(
    changed: Rc<Cell<bool>>,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        changed.set(true);
    });
    // SAFETY: the notification name is the AppKit constant, and the block
    // takes the notification argument the center passes.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidChangeScreenParametersNotification),
            None,
            None,
            &block,
        )
    }
}
