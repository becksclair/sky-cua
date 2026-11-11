//! The single point/pixel conversion layer for the macOS backend.
//!
//! macOS has one global coordinate space shared by AX, CGEvent, and the
//! CoreGraphics window list: points, top-left origin, y down. This crate
//! calls it `DesktopLogical`. The model sees a screenshot, and addresses it in
//! model-image pixels (`StreamPixels`). A capture is therefore fully described
//! by the desktop rectangle it covers and one scale factor, both carried on
//! `CaptureInfo` (`logical_rect`, `logical_to_pixel_scale`), so conversion is
//! a subtraction and a multiplication with no per-display lookups: the display
//! backing scale and any model-image downscale are already folded into
//! `logical_to_pixel_scale` by `sky_cua_capture::update_model_capture_scale`.

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{CaptureInfo, CoordinateSpace, RectF};

/// The mapping between one capture's model-image pixels and desktop points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureGeometry {
    /// Desktop point at the capture's top-left pixel.
    pub origin_x: f64,
    pub origin_y: f64,
    /// Model-image pixels per desktop point.
    pub scale: f64,
}

impl CaptureGeometry {
    /// Returns the mapping for a capture addressed in model-image pixels, or
    /// `None` when the capture carries no usable pixel mapping (no image, a
    /// non-pixel coordinate space, or a missing rectangle). An invalid scale
    /// is an error rather than a silent identity mapping.
    pub fn from_capture(capture: &CaptureInfo) -> Result<Option<Self>, BackendError> {
        if capture.coordinate_space != Some(CoordinateSpace::StreamPixels) {
            return Ok(None);
        }
        let (Some(rect), Some(scale)) = (
            capture.logical_rect.as_ref(),
            capture.logical_to_pixel_scale,
        ) else {
            return Ok(None);
        };
        if !scale.is_finite() || scale <= 0.0 {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!("invalid macOS capture scale: {scale}"),
            ));
        }
        Ok(Some(Self {
            origin_x: rect.x,
            origin_y: rect.y,
            scale,
        }))
    }

    #[must_use]
    pub fn stream_to_desktop(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            self.origin_x + x / self.scale,
            self.origin_y + y / self.scale,
        )
    }

    #[must_use]
    pub fn desktop_to_stream(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            (x - self.origin_x) * self.scale,
            (y - self.origin_y) * self.scale,
        )
    }

    #[must_use]
    pub fn desktop_rect_to_stream(&self, rect: &RectF) -> RectF {
        let (x, y) = self.desktop_to_stream((rect.x, rect.y));
        RectF {
            x,
            y,
            width: rect.width * self.scale,
            height: rect.height * self.scale,
            space: CoordinateSpace::StreamPixels,
        }
    }
}

/// A desktop rectangle in global points.
#[must_use]
pub fn desktop_rect(x: f64, y: f64, width: f64, height: f64) -> RectF {
    RectF {
        x,
        y,
        width,
        height,
        space: CoordinateSpace::DesktopLogical,
    }
}

/// Converts an element's desktop-point bounds into the space the snapshot
/// reports: model-image pixels when the snapshot has a pixel-addressed
/// capture, desktop points otherwise.
pub fn element_bounds(
    desktop: &RectF,
    capture: Option<&CaptureInfo>,
) -> Result<RectF, BackendError> {
    match capture.map(CaptureGeometry::from_capture).transpose()? {
        Some(Some(geometry)) => Ok(geometry.desktop_rect_to_stream(desktop)),
        _ => Ok(desktop.clone()),
    }
}

/// Converts an action point expressed in the snapshot's coordinate space back
/// into desktop points for AX hit-testing or CGEvent input.
pub fn action_point_to_desktop(
    point: (f64, f64),
    capture: Option<&CaptureInfo>,
) -> Result<(f64, f64), BackendError> {
    match capture.map(CaptureGeometry::from_capture).transpose()? {
        Some(Some(geometry)) => Ok(geometry.stream_to_desktop(point)),
        _ => Ok(point),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::{CaptureBackendKind, CaptureScope, PixelSize};

    fn capture(x: f64, y: f64, width: f64, height: f64, pixel_width: u32) -> CaptureInfo {
        let mut capture = CaptureInfo {
            backend: CaptureBackendKind::ScreenCaptureKit,
            image_backend: Some(CaptureBackendKind::ScreenCaptureKit),
            capture_scope: CaptureScope::Window,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: Some(desktop_rect(x, y, width, height)),
            source_logical_rect: Some(desktop_rect(x, y, width, height)),
            pixel_size: Some(PixelSize {
                width: pixel_width,
                height: (f64::from(pixel_width) * height / width).round() as u32,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        };
        capture.logical_to_pixel_scale = Some(f64::from(pixel_width) / width);
        capture
    }

    #[test]
    fn retina_window_pixels_map_to_desktop_points() {
        let capture = capture(100.0, 50.0, 800.0, 600.0, 1600);
        let point = action_point_to_desktop((400.0, 300.0), Some(&capture)).unwrap();
        assert_eq!(point, (300.0, 200.0));
    }

    #[test]
    fn downscaled_model_image_uses_the_folded_scale() {
        // A 2x Retina capture of a 1600pt-wide window downscaled to 1280px.
        let capture = capture(0.0, 0.0, 1600.0, 900.0, 1280);
        let point = action_point_to_desktop((640.0, 360.0), Some(&capture)).unwrap();
        assert_eq!(point, (800.0, 450.0));
    }

    #[test]
    fn negative_origin_secondary_display_round_trips() {
        let capture = capture(-1920.0, -200.0, 960.0, 540.0, 1920);
        let geometry = CaptureGeometry::from_capture(&capture).unwrap().unwrap();
        let desktop = geometry.stream_to_desktop((10.0, 20.0));
        assert_eq!(desktop, (-1915.0, -190.0));
        assert_eq!(geometry.desktop_to_stream(desktop), (10.0, 20.0));
    }

    #[test]
    fn element_bounds_become_screenshot_local_pixels() {
        let capture = capture(100.0, 50.0, 800.0, 600.0, 1600);
        let bounds =
            element_bounds(&desktop_rect(150.0, 70.0, 40.0, 20.0), Some(&capture)).unwrap();
        assert_eq!(
            bounds,
            RectF {
                x: 100.0,
                y: 40.0,
                width: 80.0,
                height: 40.0,
                space: CoordinateSpace::StreamPixels,
            }
        );
    }

    #[test]
    fn without_a_pixel_capture_points_pass_through() {
        assert_eq!(
            action_point_to_desktop((12.0, 34.0), None).unwrap(),
            (12.0, 34.0)
        );
        let mut imageless = capture(0.0, 0.0, 10.0, 10.0, 20);
        imageless.clear_image_fields();
        let rect = desktop_rect(1.0, 2.0, 3.0, 4.0);
        assert_eq!(element_bounds(&rect, Some(&imageless)).unwrap(), rect);
    }

    #[test]
    fn invalid_scale_is_rejected() {
        let mut capture = capture(0.0, 0.0, 10.0, 10.0, 20);
        capture.logical_to_pixel_scale = Some(0.0);
        assert!(action_point_to_desktop((1.0, 1.0), Some(&capture)).is_err());
    }
}
