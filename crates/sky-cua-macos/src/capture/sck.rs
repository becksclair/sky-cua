//! ScreenCaptureKit bridging: shareable-content lookup, content filters, and
//! the single-frame `SCScreenshotManager` capture.
//!
//! Every ScreenCaptureKit call here is asynchronous with a completion block.
//! Blocks forward their result through a tokio oneshot channel, so the
//! awaiting task yields instead of blocking a runtime thread; Objective-C
//! objects cross the await only inside `Sendable`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2_core_foundation::{CFRetained, CGRect};
use objc2_core_graphics::{CGImage, kCGColorSpaceSRGB};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
    SCStreamErrorCode, SCWindow,
};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::RectF;
use tokio::sync::oneshot;

use super::content_cache;
use crate::coords::desktop_rect;

/// Upper bound for one ScreenCaptureKit callback. Shareable-content queries
/// and single-frame captures normally complete in tens of milliseconds; the
/// bound only keeps a wedged `replayd` from stalling the daemon.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(5);

/// An Objective-C object moved between threads.
///
/// ScreenCaptureKit's shareable-content, window, display, and filter objects
/// are immutable after creation and documented as usable from any queue.
pub(super) struct Sendable<T>(pub T);

// SAFETY: see the type documentation; the wrapped objects are immutable and
// ScreenCaptureKit delivers them on arbitrary dispatch queues itself. `Send`
// is implemented only for the object types this module moves, so wrapping
// anything else does not compile.
unsafe impl Send for Sendable<Retained<SCShareableContent>> {}
unsafe impl Send for Sendable<(Retained<SCContentFilter>, Retained<SCStreamConfiguration>)> {}

/// A frame captured by ScreenCaptureKit with the desktop rectangle it covers.
pub(super) struct CapturedFrame {
    pub image: CFRetained<CGImage>,
    /// Desktop rectangle in global points covered by `image`.
    pub logical_rect: RectF,
    /// Pixels per point of the content filter.
    pub point_pixel_scale: f64,
    pub requested_width: usize,
    pub requested_height: usize,
    pub timings: CaptureTimings,
}

/// Wall-clock cost of each ScreenCaptureKit stage.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct CaptureTimings {
    /// Shareable-content fetch, or the validated reuse of a recent one.
    pub shareable_content: Duration,
    /// True when a recent shareable-content result was reused.
    pub content_reused: bool,
    pub screenshot: Duration,
}

/// A ScreenCaptureKit failure reported through `NSError`.
#[derive(Debug, Clone)]
pub(super) struct ScError {
    pub domain: String,
    pub code: isize,
    pub message: String,
}

impl ScError {
    fn from_ns_error(error: *mut NSError) -> Self {
        // SAFETY: ScreenCaptureKit passes either null or a valid NSError that
        // stays alive for the duration of the completion block.
        match unsafe { error.as_ref() } {
            Some(error) => Self {
                domain: error.domain().to_string(),
                code: error.code(),
                message: error.localizedDescription().to_string(),
            },
            None => Self {
                domain: String::new(),
                code: 0,
                message: "completed without a result or an error".to_owned(),
            },
        }
    }

    /// True when ScreenCaptureKit refused because the Screen Recording grant
    /// is missing.
    pub fn is_permission_denied(&self) -> bool {
        self.domain == "com.apple.ScreenCaptureKit.SCStreamErrorDomain"
            && self.code == SCStreamErrorCode::UserDeclined.0
    }

    fn into_backend_error(self, what: &str) -> BackendError {
        if self.is_permission_denied() {
            return crate::permissions::missing_permission_error(
                crate::permissions::Permission::ScreenCapture,
            );
        }
        BackendError::new(
            BackendErrorCode::Internal,
            format!(
                "ScreenCaptureKit {what} failed ({} {}): {}",
                self.domain, self.code, self.message
            ),
        )
    }
}

/// Captures one window's own content, independent of occlusion, without its
/// shadow. `pid` guards against a reused window id.
pub(super) async fn capture_window(
    window_id: u32,
    pid: i32,
) -> Result<CapturedFrame, BackendError> {
    let started = Instant::now();
    let reused = content_cache::recent().and_then(|content| {
        autoreleasepool(|_| {
            // SAFETY: property reads on immutable ScreenCaptureKit objects.
            let window = unsafe { lookup_window(&content.0, window_id) }?;
            // SAFETY: as above.
            let (owner, frame) = unsafe { (owner_pid(&window), window.frame()) };
            (owner == Some(pid) && content_cache::window_matches(window_id, pid, frame))
                // SAFETY: `window` is a valid shareable window.
                .then(|| unsafe { window_request(&window, window_id) })
        })
    });
    let content_reused = reused.is_some();
    let request = match reused {
        Some(request) => request?,
        None => {
            let content = shareable_content().await?;
            autoreleasepool(|_| {
                // SAFETY: property reads and initialisers on immutable
                // ScreenCaptureKit objects.
                unsafe {
                    let window = find_window(&content.0, window_id, pid)?;
                    window_request(&window, window_id)
                }
            })?
        }
    };
    finish(request, started.elapsed(), content_reused).await
}

/// Captures one whole display, including the menu bar, desktop, and Dock.
pub(super) async fn capture_display(display_id: u32) -> Result<CapturedFrame, BackendError> {
    let started = Instant::now();
    let reused = content_cache::recent().and_then(|content| {
        autoreleasepool(|_| {
            // SAFETY: property reads on immutable ScreenCaptureKit objects.
            let display = unsafe { lookup_display(&content.0, display_id) }?;
            // SAFETY: as above.
            let frame = unsafe { display.frame() };
            content_cache::display_matches(display_id, frame)
                // SAFETY: `display` is a valid shareable display.
                .then(|| unsafe { display_request(&display, display_id) })
        })
    });
    let content_reused = reused.is_some();
    let request = match reused {
        Some(request) => request?,
        None => {
            let content = shareable_content().await?;
            autoreleasepool(|_| {
                // SAFETY: property reads and initialisers on immutable
                // ScreenCaptureKit objects.
                unsafe {
                    let display = lookup_display(&content.0, display_id).ok_or_else(|| {
                        BackendError::new(
                            BackendErrorCode::CaptureSourceGeometryMissing,
                            format!(
                                "macOS display {display_id} is no longer connected or is not \
                                 shareable with ScreenCaptureKit"
                            ),
                        )
                    })?;
                    display_request(&display, display_id)
                }
            })?
        }
    };
    finish(request, started.elapsed(), content_reused).await
}

/// Builds the shadow-free, desktop-independent capture of `window`.
unsafe fn window_request(
    window: &SCWindow,
    window_id: u32,
) -> Result<CaptureRequest, BackendError> {
    // SAFETY: initialiser, property reads, and setters on valid objects.
    unsafe {
        let filter =
            SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), window);
        let frame = window.frame();
        let scale = f64::from(filter.pointPixelScale());
        let (width, height) = output_size(frame, scale, || format!("macOS window {window_id}"))?;
        let config = base_configuration(width, height);
        config.setIgnoreShadowsSingleWindow(true);
        config.setIgnoreGlobalClipSingleWindow(true);
        config.setShouldBeOpaque(true);
        Ok(CaptureRequest {
            objects: Sendable((filter, config)),
            logical_rect: cg_rect_to_desktop(frame),
            scale,
            width,
            height,
        })
    }
}

/// Builds the whole-display capture of `display`.
unsafe fn display_request(
    display: &SCDisplay,
    display_id: u32,
) -> Result<CaptureRequest, BackendError> {
    // SAFETY: initialiser, property reads, and setters on valid objects.
    unsafe {
        let filter = SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            display,
            &NSArray::new(),
        );
        let frame = display.frame();
        let scale = f64::from(filter.pointPixelScale());
        let (width, height) = output_size(frame, scale, || format!("macOS display {display_id}"))?;
        let config = base_configuration(width, height);
        Ok(CaptureRequest {
            objects: Sendable((filter, config)),
            logical_rect: cg_rect_to_desktop(frame),
            scale,
            width,
            height,
        })
    }
}

struct CaptureRequest {
    objects: Sendable<(Retained<SCContentFilter>, Retained<SCStreamConfiguration>)>,
    logical_rect: RectF,
    scale: f64,
    width: usize,
    height: usize,
}

async fn finish(
    request: CaptureRequest,
    shareable_content: Duration,
    content_reused: bool,
) -> Result<CapturedFrame, BackendError> {
    let started = Instant::now();
    let CaptureRequest {
        objects,
        logical_rect,
        scale,
        width,
        height,
    } = request;
    let receiver = start_screenshot(objects);
    let image = await_callback(receiver, "screenshot")
        .await?
        .map_err(|error| error.into_backend_error("screenshot"))?;
    Ok(CapturedFrame {
        image,
        logical_rect,
        point_pixel_scale: scale,
        requested_width: width,
        requested_height: height,
        timings: CaptureTimings {
            shareable_content,
            content_reused,
            screenshot: started.elapsed(),
        },
    })
}

/// Starts a single-frame capture and returns the channel its completion
/// handler answers on. ScreenCaptureKit retains the filter and configuration
/// for the duration of the capture, so they are released here and never held
/// across an await.
fn start_screenshot(
    objects: Sendable<(Retained<SCContentFilter>, Retained<SCStreamConfiguration>)>,
) -> oneshot::Receiver<Result<CFRetained<CGImage>, ScError>> {
    let Sendable((filter, config)) = objects;
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));
    let block = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        let result = match std::ptr::NonNull::new(image) {
            // SAFETY: the image is valid for the duration of the block;
            // retaining it extends its lifetime past the callback.
            Some(image) => Ok(unsafe { CFRetained::retain(image) }),
            None => Err(ScError::from_ns_error(error)),
        };
        if let Some(sender) = sender.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = sender.send(result);
        }
    });
    // SAFETY: filter, configuration, and block are valid; ScreenCaptureKit
    // copies the block and calls it exactly once on its own queue.
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&block),
        );
    }
    receiver
}

/// Opens this process's CoreGraphics window-server connection.
///
/// `SCContentFilter` initialisers assert that the connection exists
/// (`CGS_REQUIRE_INIT`) but do not open it, and a daemon without AppKit has
/// none until its first CoreGraphics display call. `CGMainDisplayID` opens it
/// and is idempotent.
fn ensure_window_server_connection() {
    static CONNECTED: std::sync::Once = std::sync::Once::new();
    CONNECTED.call_once(|| {
        let _ = objc2_core_graphics::CGMainDisplayID();
    });
}

/// Fetches the shareable on-screen windows and displays. Desktop-level
/// windows are excluded and off-screen windows are skipped, which keeps the
/// enumeration as small as the capture targets allow (the backend only
/// targets on-screen, normal-layer windows).
async fn shareable_content() -> Result<Sendable<Retained<SCShareableContent>>, BackendError> {
    ensure_window_server_connection();
    let receiver = {
        let (sender, receiver) =
            oneshot::channel::<Result<Sendable<Retained<SCShareableContent>>, ScError>>();
        let sender = Mutex::new(Some(sender));
        let block = RcBlock::new(
            move |content: *mut SCShareableContent, error: *mut NSError| {
                // SAFETY: the content object is valid for the duration of the
                // block; retaining it extends its lifetime past the callback.
                let result = match unsafe { Retained::retain(content) } {
                    Some(content) => Ok(Sendable(content)),
                    None => Err(ScError::from_ns_error(error)),
                };
                if let Some(sender) = sender.lock().ok().and_then(|mut slot| slot.take()) {
                    let _ = sender.send(result);
                }
            },
        );
        // SAFETY: the block is valid; ScreenCaptureKit copies it and calls it
        // exactly once on its own queue.
        unsafe {
            SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
                true, true, &block,
            );
        }
        receiver
    };
    let content = await_callback(receiver, "shareable content query")
        .await?
        .map_err(|error| error.into_backend_error("shareable content query"))?;
    content_cache::store(&content);
    Ok(content)
}

/// One shareable on-screen window, for live probes.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(super) struct ProbeWindow {
    pub window_id: u32,
    pub pid: i32,
    pub app: String,
    pub title: String,
    pub layer: isize,
    pub frame: RectF,
}

/// Lists shareable on-screen windows, for live probes.
#[cfg(test)]
pub(super) async fn probe_windows() -> Result<Vec<ProbeWindow>, BackendError> {
    let content = shareable_content().await?;
    Ok(autoreleasepool(|_| {
        // SAFETY: property reads on immutable ScreenCaptureKit objects.
        unsafe {
            content
                .0
                .windows()
                .iter()
                .map(|window| {
                    let application = window.owningApplication();
                    ProbeWindow {
                        window_id: window.windowID(),
                        pid: application.as_ref().map_or(-1, |app| app.processID()),
                        app: application
                            .map(|app| app.applicationName().to_string())
                            .unwrap_or_default(),
                        title: window.title().map(|t| t.to_string()).unwrap_or_default(),
                        layer: window.windowLayer(),
                        frame: cg_rect_to_desktop(window.frame()),
                    }
                })
                .collect()
        }
    }))
}

async fn await_callback<T>(receiver: oneshot::Receiver<T>, what: &str) -> Result<T, BackendError> {
    match tokio::time::timeout(CALLBACK_TIMEOUT, receiver).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(BackendError::new(
            BackendErrorCode::Internal,
            format!("ScreenCaptureKit dropped the {what} completion handler without calling it"),
        )),
        Err(_) => Err(BackendError::new(
            BackendErrorCode::Internal,
            format!(
                "ScreenCaptureKit {what} did not complete within {} ms",
                CALLBACK_TIMEOUT.as_millis()
            ),
        )),
    }
}

/// Finds the shareable window with `window_id`, rejecting a window id that
/// now belongs to a different process.
unsafe fn find_window(
    content: &SCShareableContent,
    window_id: u32,
    pid: i32,
) -> Result<Retained<SCWindow>, BackendError> {
    // SAFETY: property reads on immutable ScreenCaptureKit objects.
    let window = unsafe { lookup_window(content, window_id) }.ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::CaptureSourceGeometryMissing,
            format!(
                "macOS window {window_id} is no longer on screen or is not shareable with \
                 ScreenCaptureKit; refresh the window list and retry"
            ),
        )
    })?;
    // SAFETY: as above.
    let owner = unsafe { owner_pid(&window) };
    if owner != Some(pid) {
        return Err(BackendError::new(
            BackendErrorCode::CaptureSourceGeometryMissing,
            format!(
                "macOS window {window_id} now belongs to process {} instead of {pid}; the \
                 original window has closed",
                owner.map_or_else(|| "unknown".to_owned(), |owner| owner.to_string())
            ),
        ));
    }
    Ok(window)
}

unsafe fn lookup_window(
    content: &SCShareableContent,
    window_id: u32,
) -> Option<Retained<SCWindow>> {
    // SAFETY: property reads on immutable ScreenCaptureKit objects.
    unsafe {
        content
            .windows()
            .iter()
            .find(|window| window.windowID() == window_id)
    }
}

unsafe fn owner_pid(window: &SCWindow) -> Option<i32> {
    // SAFETY: property reads on immutable ScreenCaptureKit objects.
    unsafe {
        window
            .owningApplication()
            .map(|application| application.processID())
    }
}

unsafe fn lookup_display(
    content: &SCShareableContent,
    display_id: u32,
) -> Option<Retained<SCDisplay>> {
    // SAFETY: property reads on immutable ScreenCaptureKit objects.
    unsafe {
        content
            .displays()
            .iter()
            .find(|display| display.displayID() == display_id)
    }
}

/// Configuration shared by window and display captures: an explicit output
/// size so pixels map exactly onto the reported rectangle, sRGB output, and
/// no cursor.
unsafe fn base_configuration(width: usize, height: usize) -> Retained<SCStreamConfiguration> {
    // SAFETY: setters on a freshly created configuration; the colour-space
    // name is a static CoreGraphics constant.
    unsafe {
        let config = SCStreamConfiguration::new();
        config.setWidth(width);
        config.setHeight(height);
        config.setShowsCursor(false);
        config.setScalesToFit(false);
        config.setColorSpaceName(kCGColorSpaceSRGB);
        config
    }
}

/// Output pixel size for a rectangle of `frame` points at `scale` pixels per
/// point.
fn output_size(
    frame: CGRect,
    scale: f64,
    describe: impl FnOnce() -> String,
) -> Result<(usize, usize), BackendError> {
    let width = (frame.size.width * scale).round();
    let height = (frame.size.height * scale).round();
    if !(scale.is_finite() && scale > 0.0 && width >= 1.0 && height >= 1.0) {
        return Err(BackendError::new(
            BackendErrorCode::CaptureSourceGeometryMissing,
            format!(
                "{} has no capturable area: {}x{} points at scale {scale}",
                describe(),
                frame.size.width,
                frame.size.height
            ),
        ));
    }
    // The values are finite, positive, and bounded by display geometry.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((width as usize, height as usize))
}

fn cg_rect_to_desktop(rect: CGRect) -> RectF {
    desktop_rect(
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_foundation::{CGPoint, CGSize};

    fn rect(width: f64, height: f64) -> CGRect {
        CGRect::new(CGPoint::new(10.0, 20.0), CGSize::new(width, height))
    }

    #[test]
    fn output_size_scales_points_to_pixels() {
        assert_eq!(
            output_size(rect(800.0, 600.0), 2.0, String::new).unwrap(),
            (1600, 1200)
        );
        assert_eq!(
            output_size(rect(801.5, 600.0), 1.0, String::new).unwrap(),
            (802, 600)
        );
    }

    #[test]
    fn output_size_rejects_empty_or_invalid_geometry() {
        for (width, height, scale) in [(0.0, 10.0, 2.0), (10.0, 10.0, 0.0), (10.0, 10.0, f64::NAN)]
        {
            let error =
                output_size(rect(width, height), scale, || "window 7".to_owned()).unwrap_err();
            assert_eq!(error.code, "CaptureSourceGeometryMissing");
            assert!(error.message.contains("window 7"), "{}", error.message);
        }
    }

    #[test]
    fn user_declined_is_a_permission_error() {
        let declined = ScError {
            domain: "com.apple.ScreenCaptureKit.SCStreamErrorDomain".to_owned(),
            code: -3801,
            message: String::new(),
        };
        assert!(declined.is_permission_denied());
        let other = ScError {
            code: -3815,
            ..declined
        };
        assert!(!other.is_permission_denied());
    }
}
