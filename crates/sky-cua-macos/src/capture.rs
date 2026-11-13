//! ScreenCaptureKit window and display capture.

mod content_cache;
mod pixels;
mod sck;

use std::path::{Path, PathBuf};
use std::time::Instant;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    CaptureBackendKind, CaptureInfo, CaptureScope, CoordinateSpace, DisplayInfo, DisplayRef,
    ModelImageFormat, PixelSize,
};

use crate::permissions::{self, Permission};
use crate::types::MacWindow;

/// What to capture.
#[derive(Debug, Clone)]
pub enum CaptureSource {
    /// One window's own content, independent of occlusion.
    Window(MacWindow),
    /// One whole display.
    Display(DisplayInfo),
}

/// Captures `source`, prepares the model image through `sky-cua-capture`, and
/// returns a fully populated `CaptureInfo`: `coordinate_space` is
/// `StreamPixels`, `logical_rect` is the captured desktop rectangle in global
/// points, and `logical_to_pixel_scale` is derived with
/// `sky_cua_capture::update_model_capture_scale`.
///
/// Never blocks a tokio runtime thread on the capture callback.
pub async fn capture(
    snapshot_id: &str,
    source: CaptureSource,
    scope: CaptureScope,
) -> Result<CaptureInfo, BackendError> {
    if !permissions::status().screen_capture {
        return Err(permissions::missing_permission_error(
            Permission::ScreenCapture,
        ));
    }
    let (frame, display) = match source {
        CaptureSource::Window(window) => {
            let display =
                crate::windows::window_to_model(&window, &crate::displays::enumerate_displays())
                    .display;
            (
                sck::capture_window(window.window_id, window.pid).await?,
                display,
            )
        }
        CaptureSource::Display(display) => {
            let display_id = crate::displays::display_id(&display)?;
            (
                sck::capture_display(display_id).await?,
                Some(DisplayRef::from(&display)),
            )
        }
    };
    build_capture_info(&captures_dir()?, snapshot_id, frame, scope, display).await
}

/// A `CaptureInfo` describing "no image" for snapshots taken with capture
/// disabled or after a capture failure.
#[must_use]
pub fn empty_capture() -> CaptureInfo {
    CaptureInfo {
        backend: CaptureBackendKind::ScreenCaptureKit,
        image_backend: None,
        capture_scope: CaptureScope::Unknown,
        display: None,
        coordinate_space: Some(CoordinateSpace::StreamPixels),
        stream_id: None,
        source_type: None,
        mapping_id: None,
        logical_rect: None,
        source_logical_rect: None,
        pixel_size: None,
        original_pixel_size: None,
        logical_to_pixel_scale: Some(1.0),
        screenshot_path: None,
        original_screenshot_path: None,
        model_image_format: None,
        model_image_quality: None,
        model_image_bytes: None,
        model_image_encode_ms: None,
    }
}

fn captures_dir() -> Result<PathBuf, BackendError> {
    let dir = sky_cua_platform::sky_cua_state_dir().map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to resolve macOS capture state directory: {error}"),
        )
    })?;
    Ok(dir.join("captures"))
}

/// Converts the frame and writes the model image on the blocking pool, then
/// assembles the `CaptureInfo`.
///
/// Only the model image is written: no consumer reads a full-resolution raw
/// file, and encoding one (PNG at Retina sizes) would dominate capture
/// latency, so `original_screenshot_path` stays `None`.
async fn build_capture_info(
    captures_dir: &Path,
    snapshot_id: &str,
    frame: sck::CapturedFrame,
    capture_scope: CaptureScope,
    display: Option<DisplayRef>,
) -> Result<CaptureInfo, BackendError> {
    let logical_rect = frame.logical_rect.clone();
    let timings = frame.timings;
    let captures_dir = captures_dir.to_path_buf();
    let snapshot = snapshot_id.to_owned();
    let (model, convert_ms) = tokio::task::spawn_blocking(move || {
        let convert_started = Instant::now();
        let rgb = pixels::cg_image_to_rgb(&frame.image).map_err(|message| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to read ScreenCaptureKit image: {message}"),
            )
        })?;
        drop(frame.image);
        let (width, height) = rgb.dimensions();
        if (width as usize, height as usize) != (frame.requested_width, frame.requested_height) {
            return Err(BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "ScreenCaptureKit returned a {width}x{height} image for a {}x{} request \
                     (scale {}); the pixels would not map onto the captured rectangle",
                    frame.requested_width, frame.requested_height, frame.point_pixel_scale
                ),
            ));
        }
        let convert_ms = convert_started.elapsed().as_millis();
        // The raw path is never written; the explicit original pixel size
        // keeps `sky-cua-capture` from probing it.
        let raw_path = captures_dir.join(format!("{snapshot}.png"));
        let model = sky_cua_capture::prepare_model_capture_from_image(
            &captures_dir,
            &snapshot,
            image::DynamicImage::ImageRgb8(rgb),
            &raw_path,
            Some(PixelSize { width, height }),
        )?;
        Ok((model, convert_ms))
    })
    .await
    .map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("macOS capture task failed to join cleanly: {error}"),
        )
    })??;
    tracing::debug!(
        snapshot_id,
        shareable_content_ms = timings.shareable_content.as_millis(),
        shareable_content_reused = timings.content_reused,
        screenshot_ms = timings.screenshot.as_millis(),
        convert_ms,
        encode_ms = model.encode_ms,
        "macOS ScreenCaptureKit capture"
    );

    let mut capture = CaptureInfo {
        backend: CaptureBackendKind::ScreenCaptureKit,
        image_backend: Some(CaptureBackendKind::ScreenCaptureKit),
        capture_scope,
        display,
        coordinate_space: Some(CoordinateSpace::StreamPixels),
        stream_id: None,
        source_type: None,
        mapping_id: None,
        source_logical_rect: Some(logical_rect.clone()),
        logical_rect: Some(logical_rect),
        pixel_size: model.pixel_size,
        original_pixel_size: model.original_pixel_size,
        // Derived below from the model pixel size and logical rect; the model
        // image may be downscaled, so the scale is not the backing scale.
        logical_to_pixel_scale: None,
        screenshot_path: Some(model.path.display().to_string()),
        original_screenshot_path: None,
        model_image_format: Some(match model.format {
            sky_cua_capture::ModelScreenshotFormat::Jpeg => ModelImageFormat::Jpeg,
            sky_cua_capture::ModelScreenshotFormat::Webp => ModelImageFormat::Webp,
        }),
        model_image_quality: Some(model.quality),
        model_image_bytes: model.bytes,
        model_image_encode_ms: Some(model.encode_ms),
    };
    sky_cua_capture::update_model_capture_scale(&mut capture);
    Ok(capture)
}

#[cfg(test)]
mod tests;
