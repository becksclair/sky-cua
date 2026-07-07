//! Coordinate mapping between device pixels, screenshot pixels, and host desktop
//! (scrcpy-window) pixels.
//!
//! Taps and swipes arrive in screenshot coordinates (the plane the model reads)
//! and must be translated to device coordinates for `adb input` or companion
//! gestures. When scrcpy maps the device into a host window, the same mapping
//! also relates host-desktop pixels to device pixels for the visible overlay.
//! Each snapshot carries a [`PhoneCoordinateMapping`] keyed by `mapping_id`.
//!
//! ## Planes and the transform chain
//!
//! Three rectangles describe a captured frame, all in the same orientation as
//! the bytes the model sees:
//!
//! - `device_rect`: the Android display in device pixels at the snapshot's
//!   rotation. `width`/`height` are the *post-rotation* extent, i.e. what the
//!   user is actually looking at (a portrait device rotated 90° has a landscape
//!   `device_rect`).
//! - `screenshot_rect`: the returned image in screenshot pixels. ADB
//!   `screencap` returns a 1:1 image, so this usually equals `device_rect`, but
//!   scrcpy `--max-size` downscales it.
//! - `host_content_rect`: where the live device video is drawn inside the host
//!   window, in host *desktop* pixels. This is the content box after window
//!   decorations and letterbox bars are removed. `host_window_rect` is the full
//!   window (decorations included) and is informational; the content rect is
//!   what overlay math uses.
//!
//! Screenshot <-> device is an axis-aligned linear scale (no rotation: the
//! capture already baked rotation into both rects). Screenshot <-> host is also
//! a linear scale, between `screenshot_rect` and `host_content_rect`, with the
//! host content rect carrying letterboxing and fractional host scale.
//!
//! `rotation_degrees` is retained for diagnostics and for the rare caller that
//! wants the *unrotated* (natural-orientation) device coordinate; see
//! [`device_point_to_natural`].
//!
//! Every transform rejects non-finite inputs and points that fall outside the
//! source rectangle, returning a structured [`MappingError`] rather than
//! clamping silently. Callers map those into `DiagnosticEntry`s.

use sky_cua_platform::model::{
    CoordinateSpace, PhoneCoordinateMapping, PhonePoint, PixelSize, RectF,
};

/// Why a coordinate could not be translated. Backends map these into
/// structured `DiagnosticEntry`s so the agent never sees a silently-clamped or
/// fabricated coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MappingError {
    /// An input coordinate was NaN or infinite.
    NonFinite { plane: &'static str },
    /// An input coordinate fell outside the source rectangle for its plane.
    OutOfBounds { plane: &'static str },
    /// A source rectangle had zero or negative extent, so no ratio exists.
    DegenerateRect { plane: &'static str },
    /// The mapping has no host content rectangle, so host-plane translation is
    /// impossible (e.g. an ADB-only session with no scrcpy window).
    NoHostMapping,
    /// `rotation_degrees` was not one of 0/90/180/270.
    UnsupportedRotation { rotation_degrees: i32 },
}

impl MappingError {
    /// Stable diagnostic code so callers map errors to structured fields rather
    /// than parsing prose.
    pub(super) fn code(&self) -> &'static str {
        match self {
            MappingError::NonFinite { .. } => "PhoneMappingNonFinite",
            MappingError::OutOfBounds { .. } => "PhoneMappingOutOfBounds",
            MappingError::DegenerateRect { .. } => "PhoneMappingDegenerateRect",
            MappingError::NoHostMapping => "PhoneMappingNoHostSurface",
            MappingError::UnsupportedRotation { .. } => "PhoneMappingUnsupportedRotation",
        }
    }
}

impl std::fmt::Display for MappingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MappingError::NonFinite { plane } => {
                write!(f, "non-finite coordinate in {plane} plane")
            }
            MappingError::OutOfBounds { plane } => {
                write!(f, "coordinate outside the {plane} rectangle")
            }
            MappingError::DegenerateRect { plane } => {
                write!(f, "{plane} rectangle has zero or negative extent")
            }
            MappingError::NoHostMapping => {
                write!(f, "mapping has no host content rectangle")
            }
            MappingError::UnsupportedRotation { rotation_degrees } => {
                write!(
                    f,
                    "unsupported rotation {rotation_degrees} (expected 0/90/180/270)"
                )
            }
        }
    }
}

/// Inputs needed to build a full [`PhoneCoordinateMapping`]. Backends fill this
/// from capture metadata; the constructor validates and normalizes it.
pub(super) struct MappingBuild<'a> {
    pub mapping_id: &'a str,
    pub session_id: &'a str,
    pub serial: &'a str,
    /// Post-rotation device extent in device pixels.
    pub device_size: PixelSize,
    /// Returned image extent in screenshot pixels.
    pub screenshot_size: PixelSize,
    /// 0/90/180/270. Other values are rejected.
    pub rotation_degrees: i32,
    /// Full host window in desktop pixels (decorations included), when a host
    /// surface exists.
    pub host_window_rect: Option<RectF>,
    /// Device-video content box in desktop pixels (letterbox/decorations
    /// removed), when a host surface exists.
    pub host_content_rect: Option<RectF>,
    pub captured_at_ms: u64,
}

/// Build a 1:1 identity mapping for a given device size.
///
/// Preserved from the Phase 1 spine: screenshot pixels equal device pixels with
/// zero rotation and no host surface. Used by the ADB baseline (where
/// `screencap` returns a device-resolution image) and by routing tests.
pub(super) fn identity_mapping(
    mapping_id: &str,
    session_id: &str,
    serial: &str,
    device_size: PixelSize,
    captured_at_ms: u64,
) -> PhoneCoordinateMapping {
    let rect = RectF {
        x: 0.0,
        y: 0.0,
        width: f64::from(device_size.width),
        height: f64::from(device_size.height),
        space: CoordinateSpace::StreamPixels,
    };
    PhoneCoordinateMapping {
        mapping_id: mapping_id.to_string(),
        session_id: session_id.to_string(),
        serial: serial.to_string(),
        device_rect: rect.clone(),
        screenshot_rect: rect,
        host_window_rect: None,
        host_content_rect: None,
        rotation_degrees: 0,
        captured_at_ms,
    }
}

/// Build a validated mapping from capture metadata. Rejects unsupported
/// rotations and degenerate (zero/negative) device or screenshot extents so a
/// mapping can never be constructed in a state that would later divide by zero.
///
/// This is the constructor a downscaled model-image delivery must use instead
/// of [`identity_mapping`]: `screenshot_size` may differ from `device_size`
/// (e.g. a model-bounded capture), and [`screenshot_to_device`] scales through
/// the ratio between them.
pub(super) fn build_mapping(
    build: &MappingBuild<'_>,
) -> Result<PhoneCoordinateMapping, MappingError> {
    if !is_supported_rotation(build.rotation_degrees) {
        return Err(MappingError::UnsupportedRotation {
            rotation_degrees: build.rotation_degrees,
        });
    }
    if build.device_size.width == 0 || build.device_size.height == 0 {
        return Err(MappingError::DegenerateRect { plane: "device" });
    }
    if build.screenshot_size.width == 0 || build.screenshot_size.height == 0 {
        return Err(MappingError::DegenerateRect {
            plane: "screenshot",
        });
    }
    let device_rect = pixel_rect(&build.device_size);
    let screenshot_rect = pixel_rect(&build.screenshot_size);
    // A host content rect with no extent is treated as "no host mapping" rather
    // than a degenerate rect, because callers commonly pass a zero rect to mean
    // "no surface".
    let host_content_rect = build
        .host_content_rect
        .clone()
        .filter(|rect| rect.width > 0.0 && rect.height > 0.0);
    Ok(PhoneCoordinateMapping {
        mapping_id: build.mapping_id.to_string(),
        session_id: build.session_id.to_string(),
        serial: build.serial.to_string(),
        device_rect,
        screenshot_rect,
        host_window_rect: build.host_window_rect.clone(),
        host_content_rect,
        rotation_degrees: normalize_rotation(build.rotation_degrees),
        captured_at_ms: build.captured_at_ms,
    })
}

/// Translate a screenshot-pixel point to a device-pixel point. Both planes are
/// already in the snapshot orientation, so this is an axis-aligned scale.
pub(super) fn screenshot_to_device(
    mapping: &PhoneCoordinateMapping,
    point: PhonePoint,
) -> Result<PhonePoint, MappingError> {
    scale_point(
        point,
        &mapping.screenshot_rect,
        &mapping.device_rect,
        "screenshot",
    )
}

/// Translate a device-pixel point to a screenshot-pixel point.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) fn device_to_screenshot(
    mapping: &PhoneCoordinateMapping,
    point: PhonePoint,
) -> Result<PhonePoint, MappingError> {
    scale_point(
        point,
        &mapping.device_rect,
        &mapping.screenshot_rect,
        "device",
    )
}

/// Translate a host-desktop-pixel point (inside the device video content box)
/// to a device-pixel point. Used when the agent acts on a scrcpy window and the
/// host content mapping is current.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) fn host_to_device(
    mapping: &PhoneCoordinateMapping,
    point: PhonePoint,
) -> Result<PhonePoint, MappingError> {
    let host = mapping
        .host_content_rect
        .as_ref()
        .ok_or(MappingError::NoHostMapping)?;
    scale_point(point, host, &mapping.device_rect, "host")
}

/// Translate a device-pixel point to a host-desktop-pixel point so the visible
/// overlay can be drawn where the agent acted. Errors when no host surface is
/// mapped.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) fn device_to_host(
    mapping: &PhoneCoordinateMapping,
    point: PhonePoint,
) -> Result<PhonePoint, MappingError> {
    let host = mapping
        .host_content_rect
        .as_ref()
        .ok_or(MappingError::NoHostMapping)?;
    scale_point(point, &mapping.device_rect, host, "device")
}

/// Translate a screenshot-pixel point straight to host-desktop pixels for the
/// visible overlay, when a host surface is mapped.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) fn screenshot_to_host(
    mapping: &PhoneCoordinateMapping,
    point: PhonePoint,
) -> Result<PhonePoint, MappingError> {
    let host = mapping
        .host_content_rect
        .as_ref()
        .ok_or(MappingError::NoHostMapping)?;
    scale_point(point, &mapping.screenshot_rect, host, "screenshot")
}

/// Map a device point in the snapshot orientation back to the device's natural
/// (rotation 0) coordinate frame. `adb input` operates in the natural frame on
/// some devices/inputs, so backends that need natural coordinates use this; the
/// returned extent is the natural device size.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) fn device_point_to_natural(
    mapping: &PhoneCoordinateMapping,
    point: PhonePoint,
) -> Result<PhonePoint, MappingError> {
    if !point_is_finite(point) {
        return Err(MappingError::NonFinite { plane: "device" });
    }
    if !point_in_rect(point, &mapping.device_rect) {
        return Err(MappingError::OutOfBounds { plane: "device" });
    }
    let w = mapping.device_rect.width;
    let h = mapping.device_rect.height;
    let (x, y) = (point.x, point.y);
    // Rotate the point from the displayed frame back into the natural frame.
    let natural = match normalize_rotation(mapping.rotation_degrees) {
        0 => (x, y),
        90 => (y, w - x),
        180 => (w - x, h - y),
        270 => (h - y, x),
        other => {
            return Err(MappingError::UnsupportedRotation {
                rotation_degrees: other,
            });
        }
    };
    Ok(PhonePoint {
        x: natural.0,
        y: natural.1,
    })
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn pixel_rect(size: &PixelSize) -> RectF {
    RectF {
        x: 0.0,
        y: 0.0,
        width: f64::from(size.width),
        height: f64::from(size.height),
        space: CoordinateSpace::StreamPixels,
    }
}

fn is_supported_rotation(rotation_degrees: i32) -> bool {
    matches!(normalize_rotation(rotation_degrees), 0 | 90 | 180 | 270)
}

/// Normalize any multiple-of-90 rotation into 0/90/180/270. Non-multiples pass
/// through unchanged so [`is_supported_rotation`] rejects them.
fn normalize_rotation(rotation_degrees: i32) -> i32 {
    rotation_degrees.rem_euclid(360)
}

fn point_is_finite(point: PhonePoint) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

/// Whether `point` lies within `rect` (inclusive of the far edge, with a small
/// epsilon so a point exactly on the boundary after float scaling is accepted).
fn point_in_rect(point: PhonePoint, rect: &RectF) -> bool {
    const EPS: f64 = 1e-6;
    point.x >= rect.x - EPS
        && point.y >= rect.y - EPS
        && point.x <= rect.right() + EPS
        && point.y <= rect.bottom() + EPS
}

/// Linear axis-aligned remap of `point` from `src` to `dst`, validating
/// finiteness, source bounds, and source extent.
fn scale_point(
    point: PhonePoint,
    src: &RectF,
    dst: &RectF,
    plane: &'static str,
) -> Result<PhonePoint, MappingError> {
    if !point_is_finite(point) {
        return Err(MappingError::NonFinite { plane });
    }
    if src.width <= 0.0 || src.height <= 0.0 {
        return Err(MappingError::DegenerateRect { plane });
    }
    if !point_in_rect(point, src) {
        return Err(MappingError::OutOfBounds { plane });
    }
    let fx = (point.x - src.x) / src.width;
    let fy = (point.y - src.y) / src.height;
    Ok(PhonePoint {
        x: dst.x + fx * dst.width,
        y: dst.y + fy * dst.height,
    })
}

#[cfg(test)]
mod tests;
