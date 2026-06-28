use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace, DisplayRef,
    EnvironmentInfo, InputBackendKind, ModelImageFormat, PixelSize, RectF, SessionKind,
};

use crate::coords::{logical_to_pixel, rect_contains_rect};
use crate::portal::remote_desktop::RemoteDesktopSessionManager;
use crate::portal::screenshot::{self, PixelRect};
use crate::x11::capture as x11_capture;

#[derive(Debug)]
pub(crate) struct CapturePlanOutcome {
    pub(crate) capture: Option<CaptureInfo>,
    pub(crate) portal_session_error: Option<BackendError>,
    pub(crate) capture_error: Option<BackendError>,
}

const CAPTURE_SOURCE_GEOMETRY_MISSING_STEM: &str =
    "targeted screenshot requires capture source geometry";
const CAPTURE_SOURCE_GEOMETRY_MISSING_MESSAGE: &str = "targeted screenshot requires capture source geometry; refresh the window/display state and retry the targeted capture once. Captures are single-screen; there is no broader capture to fall back to.";
const CAPTURE_BACKEND_DOWNGRADED_MESSAGE: &str =
    "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback";
const CAPTURE_BACKEND_DOWNGRADED_DETAILS: &str =
    "primary_backend=portal_pipe_wire image_backend=portal_screenshot";

pub(crate) fn is_capture_source_geometry_missing(error: &BackendError) -> bool {
    error.code == BackendErrorCode::CaptureSourceGeometryMissing.as_str()
        || error.message.contains(CAPTURE_SOURCE_GEOMETRY_MISSING_STEM)
}

pub(crate) fn outcome_missing_capture_source_geometry(outcome: &CapturePlanOutcome) -> bool {
    if outcome.capture.as_ref().is_some_and(|capture_info| {
        capture_info.screenshot_path.is_some()
            && capture_info.backend == CaptureBackendKind::PortalPipeWire
            && capture_info.image_backend == Some(CaptureBackendKind::PortalScreenshot)
            && capture_info.source_logical_rect.is_none()
    }) {
        return true;
    }
    outcome
        .capture_error
        .as_ref()
        .is_some_and(is_capture_source_geometry_missing)
        && outcome
            .capture
            .as_ref()
            .is_none_or(|capture_info| capture_info.screenshot_path.is_none())
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CaptureRegionTarget {
    pub(crate) desktop_logical_rect: RectF,
    pub(crate) capture_scope: CaptureScope,
    pub(crate) display: Option<DisplayRef>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn plan_capture(
    portal: &RemoteDesktopSessionManager,
    snapshot_id: &str,
    capture_screen: CaptureScreenMode,
    environment: &EnvironmentInfo,
    region_target: Option<&CaptureRegionTarget>,
    capture_scope: CaptureScope,
    display: Option<DisplayRef>,
    defer_source_geometry_fallback: bool,
    diagnostics: &mut DiagnosticBuilder,
) -> Result<CapturePlanOutcome, BackendError> {
    let should_capture_screen = capture_screen != CaptureScreenMode::Never;
    let mut capture =
        initial_capture_with_scope(capture_screen, environment, capture_scope, display);
    let mut portal_session_error: Option<BackendError> = None;
    let mut capture_error: Option<BackendError> = None;

    if should_capture_screen && environment.input_backend == InputBackendKind::PortalRemoteDesktop {
        match portal.ensure_started().await {
            Ok(Some(stream)) => {
                if let Some(capture_info) = capture.as_mut() {
                    capture_info.stream_id = Some(
                        stream
                            .stream_id
                            .unwrap_or_else(|| stream.node_id.to_string()),
                    );
                    capture_info.source_type = stream.source_type;
                    capture_info.mapping_id = stream.mapping_id;
                    capture_info.source_logical_rect = stream.logical_rect.clone();
                    capture_info.logical_rect = stream.logical_rect;
                }
            }
            Ok(None) => diagnostics.push(
                BackendErrorCode::PortalCapabilityMissing,
                "RemoteDesktop started without an associated screencast stream",
                None,
            ),
            Err(error) => portal_session_error = Some(error),
        }
    }

    if should_capture_screen
        && environment.capture_backend == CaptureBackendKind::PortalPipeWire
        && environment.input_backend == InputBackendKind::PortalRemoteDesktop
        && portal_session_error.is_none()
    {
        // Capture the frame even when the portal stream omitted its logical
        // position. xdg-desktop-portal-kde does not populate the ScreenCast
        // stream position the way Mutter does, so `source_logical_rect` is None
        // here; `apply_model_capture` then recovers it from the captured frame's
        // pixel size against the display topology, so a targeted crop still
        // resolves. It only errors when the streamed monitor genuinely cannot
        // satisfy the requested target (e.g. the target lives on a different
        // monitor than the one the single-monitor RemoteDesktop session streams).
        match portal.capture_frame(snapshot_id).await {
            Ok(frame) => {
                if let Some(capture_info) = capture.as_mut() {
                    capture_info.image_backend = Some(CaptureBackendKind::PortalPipeWire);
                    if let Err(error) = apply_model_capture(
                        capture_info,
                        snapshot_id,
                        &frame.path,
                        frame.pixel_size,
                        region_target,
                        environment,
                    ) {
                        capture_info.clear_image_fields();
                        capture_error = Some(error);
                    }
                }
            }
            Err(error) => {
                capture_error = Some(error);
            }
        }
    } else if should_attempt_x11_capture(capture_screen, environment) {
        match x11_capture::capture_still(snapshot_id).await {
            Ok(frame) => {
                if let Some(capture_info) = capture.as_mut() {
                    capture_info.image_backend = Some(CaptureBackendKind::X11);
                    apply_model_capture(
                        capture_info,
                        snapshot_id,
                        &frame.path,
                        frame.pixel_size,
                        region_target,
                        environment,
                    )?;
                }
            }
            Err(error) => diagnostics.push(
                BackendErrorCode::Internal,
                "X11 capture failed while building the app-state snapshot",
                Some(error.message),
            ),
        }
    }

    if should_fallback_to_screenshot(
        capture.as_ref(),
        environment,
        portal_session_error.as_ref(),
        capture_error.as_ref(),
        region_target,
        defer_source_geometry_fallback,
    ) {
        match screenshot::capture_still(snapshot_id).await {
            Ok(path) => {
                if let Some(capture_info) = capture.as_mut() {
                    capture_info.image_backend = Some(CaptureBackendKind::PortalScreenshot);
                    let original_pixel_size = screenshot::pixel_size_from_path(&path);
                    apply_independent_model_capture(
                        capture_info,
                        snapshot_id,
                        &path,
                        original_pixel_size,
                        region_target,
                        environment,
                    )?;
                }
            }
            Err(error) => diagnostics.push(
                BackendErrorCode::PortalRequestDenied,
                "Still capture fallback through the Screenshot portal failed",
                Some(error.message),
            ),
        }
    }

    Ok(CapturePlanOutcome {
        capture,
        portal_session_error,
        capture_error,
    })
}

#[cfg(test)]
pub(crate) fn initial_capture(
    capture_screen: CaptureScreenMode,
    environment: &EnvironmentInfo,
) -> Option<CaptureInfo> {
    initial_capture_with_scope(capture_screen, environment, CaptureScope::Unknown, None)
}

pub(crate) fn initial_capture_with_scope(
    capture_screen: CaptureScreenMode,
    environment: &EnvironmentInfo,
    capture_scope: CaptureScope,
    display: Option<DisplayRef>,
) -> Option<CaptureInfo> {
    (capture_screen != CaptureScreenMode::Never
        && environment.capture_backend != CaptureBackendKind::None)
        .then_some(CaptureInfo {
            backend: environment.capture_backend.clone(),
            image_backend: None,
            capture_scope,
            display,
            coordinate_space: None,
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: None,
            source_logical_rect: None,
            pixel_size: None,
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        })
}

pub(crate) fn should_attempt_x11_capture(
    capture_screen: CaptureScreenMode,
    environment: &EnvironmentInfo,
) -> bool {
    capture_screen != CaptureScreenMode::Never
        && environment.capture_backend == CaptureBackendKind::X11
}

fn should_fallback_to_screenshot(
    capture: Option<&CaptureInfo>,
    environment: &EnvironmentInfo,
    portal_session_error: Option<&BackendError>,
    capture_error: Option<&BackendError>,
    region_target: Option<&CaptureRegionTarget>,
    defer_source_geometry_fallback: bool,
) -> bool {
    let has_unfilled_capture = capture
        .as_ref()
        .is_some_and(|capture_info| capture_info.screenshot_path.is_none());
    if defer_source_geometry_fallback
        && region_target.is_some()
        && capture_error
            .as_ref()
            .is_some_and(|error| is_capture_source_geometry_missing(error))
    {
        return false;
    }
    let has_retryable_target_error = region_target.is_some() && capture_error.is_some();
    let targeted_pipewire_attempt_can_still_fill_capture = region_target.is_some()
        && environment.capture_backend == CaptureBackendKind::PortalPipeWire
        && environment.input_backend == InputBackendKind::PortalRemoteDesktop;
    if targeted_pipewire_attempt_can_still_fill_capture
        && portal_session_error.is_none()
        && !has_retryable_target_error
    {
        return false;
    }
    has_unfilled_capture
        && environment.portal_capabilities.screenshot_version.is_some()
        && !portal_approval_pending(portal_session_error)
        && !portal_approval_pending(capture_error)
        && matches!(environment.session_kind, SessionKind::Wayland)
}

fn apply_independent_model_capture(
    capture_info: &mut CaptureInfo,
    snapshot_id: &str,
    raw_path: &std::path::Path,
    raw_pixel_size: Option<PixelSize>,
    region_target: Option<&CaptureRegionTarget>,
    environment: &EnvironmentInfo,
) -> Result<(), BackendError> {
    let dispatch_source = capture_info.source_logical_rect.clone();
    capture_info.logical_rect = None;
    capture_info.source_logical_rect = None;
    apply_model_capture(
        capture_info,
        snapshot_id,
        raw_path,
        raw_pixel_size,
        region_target,
        environment,
    )?;
    capture_info.source_logical_rect =
        compatible_dispatch_source(dispatch_source, capture_info.logical_rect.as_ref());
    Ok(())
}

fn compatible_dispatch_source(
    dispatch_source: Option<RectF>,
    final_logical_rect: Option<&RectF>,
) -> Option<RectF> {
    let dispatch_source = dispatch_source?;
    let final_logical_rect = final_logical_rect?;
    rect_contains_rect(&dispatch_source, final_logical_rect).then_some(dispatch_source)
}

fn apply_model_capture(
    capture_info: &mut CaptureInfo,
    snapshot_id: &str,
    raw_path: &std::path::Path,
    raw_pixel_size: Option<PixelSize>,
    region_target: Option<&CaptureRegionTarget>,
    environment: &EnvironmentInfo,
) -> Result<(), BackendError> {
    let source_logical_rect = capture_info.logical_rect.clone();
    if capture_info.source_logical_rect.is_none() {
        capture_info.source_logical_rect = source_logical_rect.clone();
    }
    let model_capture = match region_target {
        Some(target) => {
            let raw_pixel_size = raw_pixel_size.clone().ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    "targeted screenshot capture requires raw capture pixel dimensions",
                )
            })?;
            let source_logical_rect = source_logical_rect
                .or_else(|| infer_capture_source_rect(target, environment, &raw_pixel_size));
            if capture_info.source_logical_rect.is_none() {
                capture_info.source_logical_rect = source_logical_rect.clone();
            }
            let crop = pixel_crop_for_target(
                &target.desktop_logical_rect,
                source_logical_rect.as_ref(),
                &raw_pixel_size,
                capture_info.image_backend.as_ref(),
            )?;
            let (cropped_path, cropped_image) =
                screenshot::crop_capture(snapshot_id, raw_path, crop.pixel_rect)?;
            capture_info.logical_rect = Some(crop.logical_rect);
            capture_info.capture_scope = target.capture_scope.clone();
            capture_info.display = target.display.clone();
            let cropped_pixel_size = PixelSize {
                width: crop.pixel_rect.width,
                height: crop.pixel_rect.height,
            };
            screenshot::prepare_model_capture_from_image(
                snapshot_id,
                cropped_image,
                &cropped_path,
                Some(cropped_pixel_size),
            )?
        }
        None => screenshot::prepare_model_capture(snapshot_id, raw_path)?,
    };
    capture_info.coordinate_space = Some(CoordinateSpace::StreamPixels);
    capture_info.screenshot_path = Some(model_capture.path.display().to_string());
    let model_pixel_size = model_capture.pixel_size.clone();
    capture_info.pixel_size = model_capture.pixel_size;
    capture_info.original_screenshot_path = model_capture
        .original_path
        .map(|path| path.display().to_string());
    capture_info.original_pixel_size = if region_target.is_some() {
        model_capture
            .original_pixel_size
            .or_else(|| model_pixel_size.clone())
    } else {
        raw_pixel_size.or(model_capture.original_pixel_size)
    };
    capture_info.model_image_format = Some(match model_capture.format {
        screenshot::ModelScreenshotFormat::Jpeg => ModelImageFormat::Jpeg,
        screenshot::ModelScreenshotFormat::Webp => ModelImageFormat::Webp,
    });
    capture_info.model_image_quality = Some(model_capture.quality);
    capture_info.model_image_bytes = model_capture.bytes;
    capture_info.model_image_encode_ms = Some(model_capture.encode_ms);
    sky_cua_capture::update_model_capture_scale(capture_info);
    Ok(())
}

fn infer_capture_source_rect(
    target: &CaptureRegionTarget,
    environment: &EnvironmentInfo,
    raw_pixel_size: &PixelSize,
) -> Option<RectF> {
    if let Some(target_display) = &target.display
        && let Some(display) = environment
            .displays
            .iter()
            .find(|display| display.display_id == target_display.display_id)
        && display_matches_raw_size(display, raw_pixel_size)
    {
        return Some(display.logical_rect.clone());
    }

    if pixel_size_matches_rect(raw_pixel_size, &target.desktop_logical_rect) {
        return Some(target.desktop_logical_rect.clone());
    }

    let union = virtual_desktop_rect(&environment.displays)?;
    pixel_size_matches_rect(raw_pixel_size, &union).then_some(union)
}

fn display_matches_raw_size(
    display: &sky_cua_platform::model::DisplayInfo,
    raw: &PixelSize,
) -> bool {
    if let Some(pixel_size) = display.pixel_size.as_ref() {
        return pixel_size.width.abs_diff(raw.width) <= 2
            && pixel_size.height.abs_diff(raw.height) <= 2;
    }
    pixel_size_matches_rect(raw, &display.logical_rect)
}

fn pixel_size_matches_rect(raw: &PixelSize, rect: &RectF) -> bool {
    (f64::from(raw.width) - rect.width).abs() <= 2.0
        && (f64::from(raw.height) - rect.height).abs() <= 2.0
}

fn virtual_desktop_rect(displays: &[sky_cua_platform::model::DisplayInfo]) -> Option<RectF> {
    let mut iter = displays.iter();
    let first = iter.next()?.logical_rect.clone();
    let (mut left, mut top, mut right, mut bottom) = (
        first.x,
        first.y,
        first.x + first.width,
        first.y + first.height,
    );
    for display in iter {
        let rect = &display.logical_rect;
        left = left.min(rect.x);
        top = top.min(rect.y);
        right = right.max(rect.x + rect.width);
        bottom = bottom.max(rect.y + rect.height);
    }
    Some(RectF {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
        space: CoordinateSpace::DesktopLogical,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct TargetCrop {
    pixel_rect: PixelRect,
    logical_rect: RectF,
}

fn pixel_crop_for_target(
    target: &RectF,
    source_logical_rect: Option<&RectF>,
    raw_pixel_size: &PixelSize,
    image_backend: Option<&CaptureBackendKind>,
) -> Result<TargetCrop, BackendError> {
    if raw_pixel_size.width == 0 || raw_pixel_size.height == 0 {
        return Err(BackendError::new(
            BackendErrorCode::Internal,
            "targeted screenshot cannot crop a zero-sized raw capture",
        ));
    }

    let (left_top, right_bottom, source) = if image_backend == Some(&CaptureBackendKind::X11)
        && source_logical_rect.is_none()
    {
        (
            (target.x, target.y),
            (target.x + target.width, target.y + target.height),
            None,
        )
    } else {
        let source = source_logical_rect.ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::CaptureSourceGeometryMissing,
                CAPTURE_SOURCE_GEOMETRY_MISSING_MESSAGE,
            )
        })?;
        (
            logical_to_pixel((target.x, target.y), source, raw_pixel_size).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    "targeted screenshot could not map window origin into capture pixels",
                )
            })?,
            logical_to_pixel(
                (target.x + target.width, target.y + target.height),
                source,
                raw_pixel_size,
            )
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    "targeted screenshot could not map window extent into capture pixels",
                )
            })?,
            Some(source),
        )
    };

    let left = left_top.0.floor().max(0.0);
    let top = left_top.1.floor().max(0.0);
    let right = right_bottom.0.ceil().min(f64::from(raw_pixel_size.width));
    let bottom = right_bottom.1.ceil().min(f64::from(raw_pixel_size.height));
    if right <= left || bottom <= top {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "targeted screenshot window bounds do not intersect the captured source",
        ));
    }
    let pixel_rect = PixelRect {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    };
    let logical_rect = source
        .map(|source| logical_rect_for_crop(&pixel_rect, source, raw_pixel_size))
        .unwrap_or(RectF {
            x: f64::from(pixel_rect.x),
            y: f64::from(pixel_rect.y),
            width: f64::from(pixel_rect.width),
            height: f64::from(pixel_rect.height),
            space: CoordinateSpace::DesktopLogical,
        });
    Ok(TargetCrop {
        pixel_rect,
        logical_rect,
    })
}

fn logical_rect_for_crop(crop: &PixelRect, source: &RectF, raw_pixel_size: &PixelSize) -> RectF {
    let scale_x = source.width / f64::from(raw_pixel_size.width);
    let scale_y = source.height / f64::from(raw_pixel_size.height);
    RectF {
        x: source.x + f64::from(crop.x) * scale_x,
        y: source.y + f64::from(crop.y) * scale_y,
        width: f64::from(crop.width) * scale_x,
        height: f64::from(crop.height) * scale_y,
        space: CoordinateSpace::DesktopLogical,
    }
}

pub(crate) fn push_diagnostics(
    environment: &EnvironmentInfo,
    capture: Option<&CaptureInfo>,
    portal_session_error: Option<&BackendError>,
    capture_error: Option<&BackendError>,
    diagnostics: &mut DiagnosticBuilder,
) {
    if let Some(error) = portal_session_error {
        if portal_approval_pending(Some(error)) {
            diagnostics.push_code(
                error.code,
                "Waiting on portal approval before live screen control can start",
                Some(error.message.clone()),
            );
        } else {
            diagnostics.push_code(
                error.code,
                "Combined RemoteDesktop and ScreenCast session could not be started",
                Some(error.message.clone()),
            );
        }
    }

    if let Some(error) = capture_error {
        if portal_approval_pending(Some(error)) {
            diagnostics.push_code(
                error.code,
                "Waiting on portal approval before a live frame can be captured for this snapshot",
                Some(error.message.clone()),
            );
            return;
        }
        let used_screenshot_fallback = capture.is_some_and(|capture_info| {
            capture_info.image_backend == Some(CaptureBackendKind::PortalScreenshot)
        });
        if is_capture_source_geometry_missing(error) {
            diagnostics.push_code(
                error.code,
                if used_screenshot_fallback {
                    "Capture source geometry was unavailable before the fallback snapshot image was produced"
                } else {
                    "Capture source geometry is unavailable for this targeted screenshot"
                },
                Some(error.message.clone()),
            );
            if used_screenshot_fallback {
                diagnostics.push(
                    BackendErrorCode::CaptureBackendDowngraded,
                    CAPTURE_BACKEND_DOWNGRADED_MESSAGE,
                    Some(CAPTURE_BACKEND_DOWNGRADED_DETAILS.to_string()),
                );
            }
            return;
        }
        let diagnostic_message = if error.code == BackendErrorCode::PipeWireStreamFailed.as_str() {
            if used_screenshot_fallback {
                "Live PipeWire frame capture failed before the snapshot image was produced"
            } else {
                "Live PipeWire frame capture failed and no fallback image was produced"
            }
        } else if used_screenshot_fallback {
            "Targeted capture failed before the fallback snapshot image was produced"
        } else {
            "Targeted capture failed and no fallback image was produced"
        };
        diagnostics.push_code(error.code, diagnostic_message, Some(error.message.clone()));
        if used_screenshot_fallback {
            diagnostics.push(
                BackendErrorCode::CaptureBackendDowngraded,
                CAPTURE_BACKEND_DOWNGRADED_MESSAGE,
                Some(CAPTURE_BACKEND_DOWNGRADED_DETAILS.to_string()),
            );
        }
    } else if portal_session_error.is_none()
        && environment.capture_backend == CaptureBackendKind::PortalPipeWire
    {
        let image_backend = capture.and_then(|capture_info| capture_info.image_backend.as_ref());
        if image_backend == Some(&CaptureBackendKind::PortalScreenshot) {
            diagnostics.push(
                BackendErrorCode::PipeWireStreamFailed,
                "Live PipeWire frame capture did not produce the snapshot image",
                Some("no PipeWire frame image was available for this snapshot".to_string()),
            );
            diagnostics.push(
                BackendErrorCode::CaptureBackendDowngraded,
                CAPTURE_BACKEND_DOWNGRADED_MESSAGE,
                Some(CAPTURE_BACKEND_DOWNGRADED_DETAILS.to_string()),
            );
        } else if capture.is_some_and(|capture_info| capture_info.screenshot_path.is_none()) {
            diagnostics.push(
                BackendErrorCode::PipeWireStreamFailed,
                "ScreenCast metadata is live, but no frame image could be produced for this snapshot",
                None,
            );
        }
    }
}

fn portal_approval_pending(error: Option<&BackendError>) -> bool {
    error.is_some_and(|error| error.code == BackendErrorCode::PortalApprovalPending.as_str())
}

#[cfg(test)]
mod tests {
    use super::{
        CAPTURE_BACKEND_DOWNGRADED_DETAILS, CapturePlanOutcome, CaptureRegionTarget,
        compatible_dispatch_source, display_matches_raw_size, infer_capture_source_rect,
        initial_capture, outcome_missing_capture_source_geometry, pixel_crop_for_target,
        push_diagnostics, should_fallback_to_screenshot, virtual_desktop_rect,
    };
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
    use sky_cua_platform::model::test_support::wayland_pipewire_environment;
    use sky_cua_platform::model::{
        CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace,
        DisplayInfo, DisplayRef, EnvironmentInfo, InputBackendKind, PixelSize, RectF,
    };

    fn capture_with_backend(
        backend: CaptureBackendKind,
        image_backend: Option<CaptureBackendKind>,
        screenshot_path: Option<&str>,
    ) -> CaptureInfo {
        CaptureInfo {
            backend,
            image_backend,
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: None,
            pixel_size: None,
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: screenshot_path.map(str::to_string),
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }

    fn test_display(
        id: &str,
        index: u32,
        primary: bool,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> sky_cua_platform::model::DisplayInfo {
        sky_cua_platform::model::DisplayInfo {
            display_id: id.to_string(),
            name: Some(id.to_string()),
            index,
            primary,
            logical_rect: RectF {
                x,
                y,
                width,
                height,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: Some(PixelSize {
                width: width as u32,
                height: height as u32,
            }),
            scale_factor: Some(1.0),
            backend: "test".to_string(),
        }
    }

    #[test]
    fn initial_capture_is_disabled_for_never_mode() {
        let environment = wayland_pipewire_environment();

        assert!(initial_capture(CaptureScreenMode::Never, &environment).is_none());
    }

    #[test]
    fn initial_capture_preserves_primary_backend_until_image_backend_is_known() {
        let environment = wayland_pipewire_environment();

        let capture = initial_capture(CaptureScreenMode::Always, &environment)
            .expect("capture should be planned");

        assert_eq!(capture.backend, CaptureBackendKind::PortalPipeWire);
        assert_eq!(capture.image_backend, None);
        assert_eq!(capture.screenshot_path, None);
    }

    #[test]
    fn screenshot_fallback_waits_for_unfilled_wayland_capture_without_portal_approval() {
        let environment = wayland_pipewire_environment();
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);

        assert!(should_fallback_to_screenshot(
            Some(&capture),
            &environment,
            None,
            None,
            None,
            false,
        ));
    }

    #[test]
    fn screenshot_fallback_is_suppressed_while_portal_approval_is_pending() {
        let environment = wayland_pipewire_environment();
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);
        let error = BackendError::new(
            BackendErrorCode::PortalApprovalPending,
            "operator has not approved the portal dialog",
        );

        assert!(!should_fallback_to_screenshot(
            Some(&capture),
            &environment,
            Some(&error),
            None,
            None,
            false,
        ));
    }

    #[test]
    fn screenshot_fallback_is_suppressed_for_targeted_captures() {
        let mut environment = wayland_pipewire_environment();
        environment.displays = vec![sky_cua_platform::model::DisplayInfo {
            display_id: "kwin:HDMI-A-1".to_string(),
            name: Some("HDMI-A-1".to_string()),
            index: 0,
            primary: true,
            logical_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: Some(sky_cua_platform::model::PixelSize {
                width: 1920,
                height: 1080,
            }),
            scale_factor: Some(1.0),
            backend: "kwin".to_string(),
        }];
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);
        let target = CaptureRegionTarget {
            desktop_logical_rect: environment.displays[0].logical_rect.clone(),
            capture_scope: CaptureScope::Display,
            display: Some(sky_cua_platform::model::DisplayRef::from(
                &environment.displays[0],
            )),
        };

        assert!(!should_fallback_to_screenshot(
            Some(&capture),
            &environment,
            None,
            None,
            Some(&target),
            false,
        ));
    }

    #[test]
    fn screenshot_fallback_is_allowed_for_targeted_screenshot_portal_backend() {
        let mut environment = wayland_pipewire_environment();
        environment.capture_backend = CaptureBackendKind::PortalScreenshot;
        environment.input_backend = InputBackendKind::LinuxVirtualInput;
        environment.displays = vec![test_display(
            "kwin:eDP-1",
            0,
            true,
            0.0,
            0.0,
            1920.0,
            1080.0,
        )];
        let capture = capture_with_backend(CaptureBackendKind::PortalScreenshot, None, None);
        let target = CaptureRegionTarget {
            desktop_logical_rect: environment.displays[0].logical_rect.clone(),
            capture_scope: CaptureScope::PrimaryDisplay,
            display: Some(sky_cua_platform::model::DisplayRef::from(
                &environment.displays[0],
            )),
        };

        assert!(should_fallback_to_screenshot(
            Some(&capture),
            &environment,
            None,
            None,
            Some(&target),
            false,
        ));
    }

    #[test]
    fn screenshot_fallback_is_allowed_for_targeted_capture_after_session_failure() {
        let mut environment = wayland_pipewire_environment();
        environment.displays = vec![test_display(
            "kwin:eDP-1",
            0,
            true,
            0.0,
            0.0,
            1920.0,
            1080.0,
        )];
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);
        let target = CaptureRegionTarget {
            desktop_logical_rect: environment.displays[0].logical_rect.clone(),
            capture_scope: CaptureScope::PrimaryDisplay,
            display: Some(sky_cua_platform::model::DisplayRef::from(
                &environment.displays[0],
            )),
        };
        let session_error = BackendError::new(
            BackendErrorCode::PortalUnavailable,
            "RemoteDesktop session failed before PipeWire capture could start",
        );

        assert!(should_fallback_to_screenshot(
            Some(&capture),
            &environment,
            Some(&session_error),
            None,
            Some(&target),
            false,
        ));
    }

    #[test]
    fn screenshot_fallback_is_allowed_for_targeted_capture_after_source_mismatch() {
        let mut environment = wayland_pipewire_environment();
        environment.displays = vec![sky_cua_platform::model::DisplayInfo {
            display_id: "kwin:HDMI-A-1".to_string(),
            name: Some("HDMI-A-1".to_string()),
            index: 0,
            primary: true,
            logical_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: Some(sky_cua_platform::model::PixelSize {
                width: 1920,
                height: 1080,
            }),
            scale_factor: Some(1.0),
            backend: "kwin".to_string(),
        }];
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);
        let target = CaptureRegionTarget {
            desktop_logical_rect: environment.displays[0].logical_rect.clone(),
            capture_scope: CaptureScope::Display,
            display: Some(sky_cua_platform::model::DisplayRef::from(
                &environment.displays[0],
            )),
        };
        let crop_error = BackendError::new(
            BackendErrorCode::InvalidRequest,
            "targeted screenshot window bounds do not intersect the captured source",
        );

        assert!(should_fallback_to_screenshot(
            Some(&capture),
            &environment,
            None,
            Some(&crop_error),
            Some(&target),
            false,
        ));
    }

    #[test]
    fn screenshot_fallback_is_deferred_for_targeted_missing_source_geometry_before_retry() {
        let mut environment = wayland_pipewire_environment();
        environment.displays = vec![test_display(
            "kwin:eDP-1",
            0,
            true,
            0.0,
            0.0,
            1920.0,
            1080.0,
        )];
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);
        let target = CaptureRegionTarget {
            desktop_logical_rect: environment.displays[0].logical_rect.clone(),
            capture_scope: CaptureScope::PrimaryDisplay,
            display: Some(sky_cua_platform::model::DisplayRef::from(
                &environment.displays[0],
            )),
        };
        let source_geometry_error = BackendError::new(
            BackendErrorCode::CaptureSourceGeometryMissing,
            super::CAPTURE_SOURCE_GEOMETRY_MISSING_MESSAGE,
        );

        assert!(!should_fallback_to_screenshot(
            Some(&capture),
            &environment,
            None,
            Some(&source_geometry_error),
            Some(&target),
            true,
        ));
        assert!(should_fallback_to_screenshot(
            Some(&capture),
            &environment,
            None,
            Some(&source_geometry_error),
            Some(&target),
            false,
        ));
    }

    #[test]
    fn independent_capture_drops_incompatible_dispatch_source() {
        let dispatch_source = RectF {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let final_rect = RectF {
            x: 1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            space: CoordinateSpace::DesktopLogical,
        };

        assert_eq!(
            compatible_dispatch_source(Some(dispatch_source), Some(&final_rect)),
            None
        );
    }

    #[test]
    fn independent_capture_keeps_dispatch_source_that_covers_final_rect() {
        let dispatch_source = RectF {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let final_rect = RectF {
            x: 500.0,
            y: 200.0,
            width: 800.0,
            height: 600.0,
            space: CoordinateSpace::DesktopLogical,
        };

        assert_eq!(
            compatible_dispatch_source(Some(dispatch_source.clone()), Some(&final_rect)),
            Some(dispatch_source)
        );
    }

    #[test]
    fn clipped_target_crop_reports_visible_logical_rect() {
        let source = RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let target = RectF {
            x: -25.0,
            y: 10.0,
            width: 50.0,
            height: 40.0,
            space: CoordinateSpace::DesktopLogical,
        };

        let crop = pixel_crop_for_target(
            &target,
            Some(&source),
            &sky_cua_platform::model::PixelSize {
                width: 100,
                height: 100,
            },
            Some(&CaptureBackendKind::PortalPipeWire),
        )
        .expect("partially visible target should crop");

        assert_eq!(crop.pixel_rect.x, 0);
        assert_eq!(crop.pixel_rect.width, 25);
        assert_eq!(crop.logical_rect.x, 0.0);
        assert_eq!(crop.logical_rect.width, 25.0);
        assert_eq!(crop.logical_rect.y, 10.0);
        assert_eq!(crop.logical_rect.height, 40.0);
    }

    #[test]
    fn target_crop_missing_source_geometry_has_specific_error_code() {
        let target = RectF {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        };

        let error = pixel_crop_for_target(
            &target,
            None,
            &PixelSize {
                width: 1920,
                height: 1080,
            },
            Some(&CaptureBackendKind::PortalPipeWire),
        )
        .expect_err("missing source geometry should be machine-readable");

        assert_eq!(error.code, "CaptureSourceGeometryMissing");
        assert!(super::is_capture_source_geometry_missing(&error));
    }

    #[test]
    fn missing_source_geometry_outcome_is_retryable_when_image_is_unfilled() {
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);
        let outcome = CapturePlanOutcome {
            capture: Some(capture),
            portal_session_error: None,
            capture_error: Some(BackendError::new(
                BackendErrorCode::CaptureSourceGeometryMissing,
                super::CAPTURE_SOURCE_GEOMETRY_MISSING_MESSAGE,
            )),
        };

        assert!(outcome_missing_capture_source_geometry(&outcome));
    }

    #[test]
    fn missing_source_geometry_outcome_is_retryable_for_independent_fallback_without_dispatch_source()
     {
        let capture = capture_with_backend(
            CaptureBackendKind::PortalPipeWire,
            Some(CaptureBackendKind::PortalScreenshot),
            Some("/tmp/fallback.png"),
        );
        let outcome = CapturePlanOutcome {
            capture: Some(capture),
            portal_session_error: None,
            capture_error: Some(BackendError::new(
                BackendErrorCode::CaptureSourceGeometryMissing,
                super::CAPTURE_SOURCE_GEOMETRY_MISSING_MESSAGE,
            )),
        };

        assert!(outcome_missing_capture_source_geometry(&outcome));
    }

    #[test]
    fn missing_source_geometry_outcome_is_not_retryable_for_direct_screenshot_capture() {
        let capture = capture_with_backend(
            CaptureBackendKind::PortalScreenshot,
            Some(CaptureBackendKind::PortalScreenshot),
            Some("/tmp/screenshot.png"),
        );
        let outcome = CapturePlanOutcome {
            capture: Some(capture),
            portal_session_error: None,
            capture_error: None,
        };

        assert!(!outcome_missing_capture_source_geometry(&outcome));
    }

    #[test]
    fn missing_source_geometry_outcome_is_not_retryable_for_fallback_with_dispatch_source() {
        let mut capture = capture_with_backend(
            CaptureBackendKind::PortalPipeWire,
            Some(CaptureBackendKind::PortalScreenshot),
            Some("/tmp/fallback.png"),
        );
        capture.source_logical_rect = Some(RectF {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            space: CoordinateSpace::DesktopLogical,
        });
        let outcome = CapturePlanOutcome {
            capture: Some(capture),
            portal_session_error: None,
            capture_error: Some(BackendError::new(
                BackendErrorCode::CaptureSourceGeometryMissing,
                super::CAPTURE_SOURCE_GEOMETRY_MISSING_MESSAGE,
            )),
        };

        assert!(!outcome_missing_capture_source_geometry(&outcome));
    }

    #[test]
    fn display_source_inference_trusts_physical_pixel_size_when_present() {
        let display = sky_cua_platform::model::DisplayInfo {
            display_id: "hyprland:DP-1".to_string(),
            name: Some("DP-1".to_string()),
            index: 0,
            primary: true,
            logical_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: Some(PixelSize {
                width: 2560,
                height: 1440,
            }),
            scale_factor: Some(2.0),
            backend: "hyprland".to_string(),
        };

        assert!(!super::display_matches_raw_size(
            &display,
            &PixelSize {
                width: 1280,
                height: 720,
            },
        ));
        assert!(super::display_matches_raw_size(
            &display,
            &PixelSize {
                width: 2560,
                height: 1440,
            },
        ));
    }

    #[test]
    fn capture_diagnostics_surface_downgrade_when_screenshot_fallback_is_used() {
        let environment = wayland_pipewire_environment();
        let capture = capture_with_backend(
            CaptureBackendKind::PortalPipeWire,
            Some(CaptureBackendKind::PortalScreenshot),
            Some("/tmp/fallback.png"),
        );
        let error = BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            "remote fd closed unexpectedly",
        );
        let mut diagnostics = DiagnosticBuilder::new();

        push_diagnostics(
            &environment,
            Some(&capture),
            None,
            Some(&error),
            &mut diagnostics,
        );

        let entries = diagnostics.finish();
        assert!(
            entries
                .iter()
                .any(|entry| entry.code == "PipeWireStreamFailed")
        );
        let downgrade = entries
            .iter()
            .find(|entry| entry.code == "CaptureBackendDowngraded")
            .expect("expected a capture downgrade diagnostic");
        assert!(downgrade.message.contains("downgraded from PipeWire"));
        assert_eq!(
            downgrade.details.as_deref(),
            Some(CAPTURE_BACKEND_DOWNGRADED_DETAILS)
        );
    }

    #[test]
    fn capture_diagnostics_do_not_claim_downgrade_without_a_fallback_image() {
        let environment = wayland_pipewire_environment();
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);
        let error = BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            "capture timed out on cached stream",
        );
        let mut diagnostics = DiagnosticBuilder::new();

        push_diagnostics(
            &environment,
            Some(&capture),
            None,
            Some(&error),
            &mut diagnostics,
        );

        let entries = diagnostics.finish();
        let pipewire = entries
            .iter()
            .find(|entry| entry.code == "PipeWireStreamFailed")
            .expect("expected a PipeWire failure diagnostic");
        assert!(pipewire.message.contains("no fallback image was produced"));
        assert!(
            !entries
                .iter()
                .any(|entry| entry.code == "CaptureBackendDowngraded")
        );
    }

    #[test]
    fn capture_diagnostics_surface_missing_source_geometry_without_pipewire_claim() {
        let environment = wayland_pipewire_environment();
        let capture = capture_with_backend(CaptureBackendKind::PortalPipeWire, None, None);
        let error = BackendError::new(
            BackendErrorCode::CaptureSourceGeometryMissing,
            super::CAPTURE_SOURCE_GEOMETRY_MISSING_MESSAGE,
        );
        let mut diagnostics = DiagnosticBuilder::new();

        push_diagnostics(
            &environment,
            Some(&capture),
            None,
            Some(&error),
            &mut diagnostics,
        );

        let entries = diagnostics.finish();
        assert!(
            entries
                .iter()
                .any(|entry| entry.code == "CaptureSourceGeometryMissing")
        );
        assert!(
            !entries
                .iter()
                .any(|entry| entry.code == "PipeWireStreamFailed")
        );
    }

    #[test]
    fn capture_diagnostics_preserve_crop_error_code_with_fallback_image() {
        let environment = wayland_pipewire_environment();
        let capture = capture_with_backend(
            CaptureBackendKind::PortalPipeWire,
            Some(CaptureBackendKind::PortalScreenshot),
            Some("/tmp/fallback.png"),
        );
        let error = BackendError::new(
            BackendErrorCode::InvalidRequest,
            "targeted screenshot window bounds do not intersect the captured source",
        );
        let mut diagnostics = DiagnosticBuilder::new();

        push_diagnostics(
            &environment,
            Some(&capture),
            None,
            Some(&error),
            &mut diagnostics,
        );

        let entries = diagnostics.finish();
        assert!(entries.iter().any(|entry| entry.code == "InvalidRequest"));
        assert!(
            !entries
                .iter()
                .any(|entry| entry.code == "PipeWireStreamFailed")
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.code == "CaptureBackendDowngraded")
        );
    }

    fn env_with_displays(displays: Vec<DisplayInfo>) -> EnvironmentInfo {
        EnvironmentInfo {
            displays,
            ..wayland_pipewire_environment()
        }
    }

    fn display(id: &str, logical_rect: RectF, pixel_size: Option<PixelSize>) -> DisplayInfo {
        DisplayInfo {
            display_id: id.to_string(),
            name: Some(id.to_string()),
            index: 0,
            primary: id == "primary",
            logical_rect,
            pixel_size,
            scale_factor: None,
            backend: "test".to_string(),
        }
    }

    fn display_ref(id: &str) -> DisplayRef {
        DisplayRef {
            display_id: id.to_string(),
            name: Some(id.to_string()),
            index: 0,
            primary: id == "primary",
            backend: "test".to_string(),
        }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
            space: CoordinateSpace::DesktopLogical,
        }
    }

    #[test]
    fn infer_capture_source_rect_prefers_matching_display() {
        let target_rect = rect(0.0, 0.0, 1920.0, 1080.0);
        let display_pixel = PixelSize {
            width: 1920,
            height: 1080,
        };
        let env = env_with_displays(vec![
            display("left", target_rect.clone(), Some(display_pixel.clone())),
            display("right", rect(1920.0, 0.0, 1920.0, 1080.0), None),
        ]);
        let target = CaptureRegionTarget {
            desktop_logical_rect: target_rect.clone(),
            capture_scope: CaptureScope::Display,
            display: Some(display_ref("left")),
        };

        assert_eq!(
            infer_capture_source_rect(&target, &env, &display_pixel),
            Some(target_rect)
        );
    }

    #[test]
    fn infer_capture_source_rect_falls_back_to_desktop_logical_rect() {
        let desktop_rect = rect(0.0, 0.0, 1920.0, 1080.0);
        let env = env_with_displays(vec![display(
            "primary",
            desktop_rect.clone(),
            Some(PixelSize {
                width: 3840,
                height: 2160,
            }),
        )]);
        let target = CaptureRegionTarget {
            desktop_logical_rect: desktop_rect.clone(),
            capture_scope: CaptureScope::Display,
            display: None,
        };

        assert_eq!(
            infer_capture_source_rect(
                &target,
                &env,
                &PixelSize {
                    width: 1920,
                    height: 1080,
                }
            ),
            Some(desktop_rect)
        );
    }

    #[test]
    fn infer_capture_source_rect_falls_back_to_virtual_desktop_union() {
        let left = rect(0.0, 0.0, 1920.0, 1080.0);
        let right = rect(1920.0, 0.0, 1920.0, 1080.0);
        let union = rect(0.0, 0.0, 3840.0, 1080.0);
        let env = env_with_displays(vec![
            display("left", left, None),
            display("right", right, None),
        ]);
        let target = CaptureRegionTarget {
            desktop_logical_rect: union.clone(),
            capture_scope: CaptureScope::Unknown,
            display: None,
        };

        assert_eq!(
            infer_capture_source_rect(
                &target,
                &env,
                &PixelSize {
                    width: 3840,
                    height: 1080,
                }
            ),
            Some(union)
        );
    }

    #[test]
    fn infer_capture_source_rect_returns_none_when_nothing_matches() {
        let env = env_with_displays(vec![display(
            "primary",
            rect(0.0, 0.0, 1920.0, 1080.0),
            Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
        )]);
        let target = CaptureRegionTarget {
            desktop_logical_rect: rect(0.0, 0.0, 800.0, 600.0),
            capture_scope: CaptureScope::Display,
            display: None,
        };

        assert_eq!(
            infer_capture_source_rect(
                &target,
                &env,
                &PixelSize {
                    width: 1024,
                    height: 768,
                }
            ),
            None
        );
    }

    #[test]
    fn display_matches_raw_size_uses_pixel_size_within_tolerance() {
        let d = display(
            "d",
            rect(0.0, 0.0, 1920.0, 1080.0),
            Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
        );
        assert!(display_matches_raw_size(
            &d,
            &PixelSize {
                width: 1921,
                height: 1080,
            }
        ));
        assert!(!display_matches_raw_size(
            &d,
            &PixelSize {
                width: 2000,
                height: 1080,
            }
        ));
    }

    #[test]
    fn display_matches_raw_size_falls_back_to_logical_rect() {
        let d = display("d", rect(0.0, 0.0, 1920.0, 1080.0), None);
        assert!(display_matches_raw_size(
            &d,
            &PixelSize {
                width: 1920,
                height: 1080,
            }
        ));
    }

    #[test]
    fn virtual_desktop_rect_unions_all_display_logical_rects() {
        let rects = [
            rect(-100.0, 0.0, 100.0, 100.0),
            rect(0.0, -50.0, 200.0, 200.0),
        ];
        let displays = vec![
            display("a", rects[0].clone(), None),
            display("b", rects[1].clone(), None),
        ];
        assert_eq!(
            virtual_desktop_rect(&displays),
            Some(rect(-100.0, -50.0, 300.0, 200.0))
        );
    }
}
