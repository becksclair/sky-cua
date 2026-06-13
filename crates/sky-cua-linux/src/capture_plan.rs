use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace, DisplayRef,
    EnvironmentInfo, InputBackendKind, ModelImageFormat, PixelSize, RectF, SessionKind,
};

use crate::coords::logical_to_pixel;
use crate::portal::remote_desktop::RemoteDesktopSessionManager;
use crate::portal::screenshot::{self, PixelRect};
use crate::x11::capture as x11_capture;

#[derive(Debug)]
pub(crate) struct CapturePlanOutcome {
    pub(crate) capture: Option<CaptureInfo>,
    pub(crate) portal_session_error: Option<BackendError>,
    pub(crate) capture_error: Option<BackendError>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CaptureRegionTarget {
    pub(crate) desktop_logical_rect: RectF,
    pub(crate) capture_scope: CaptureScope,
    pub(crate) display: Option<DisplayRef>,
}

pub(crate) async fn plan_capture(
    portal: &RemoteDesktopSessionManager,
    snapshot_id: &str,
    capture_screen: CaptureScreenMode,
    environment: &EnvironmentInfo,
    region_target: Option<&CaptureRegionTarget>,
    capture_scope: CaptureScope,
    display: Option<DisplayRef>,
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
        match portal.capture_frame(snapshot_id).await {
            Ok(frame) => {
                if let Some(capture_info) = capture.as_mut() {
                    capture_info.image_backend = Some(CaptureBackendKind::PortalPipeWire);
                    if !pipewire_source_covers_all_displays(capture_info, environment) {
                        clear_failed_image_capture(capture_info);
                        capture_error = Some(BackendError::new(
                            BackendErrorCode::PipeWireStreamFailed,
                            "RemoteDesktop stream does not cover the virtual desktop required for an all-displays screenshot",
                        ));
                    } else if let Err(error) = apply_model_capture(
                        capture_info,
                        snapshot_id,
                        &frame.path,
                        frame.pixel_size,
                        region_target,
                        environment,
                    ) {
                        clear_failed_image_capture(capture_info);
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
) -> bool {
    let has_unfilled_capture = capture
        .as_ref()
        .is_some_and(|capture_info| capture_info.screenshot_path.is_none());
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

fn clear_failed_image_capture(capture_info: &mut CaptureInfo) {
    capture_info.image_backend = None;
    capture_info.screenshot_path = None;
    capture_info.pixel_size = None;
    capture_info.original_screenshot_path = None;
    capture_info.original_pixel_size = None;
    capture_info.logical_to_pixel_scale = None;
    capture_info.model_image_format = None;
    capture_info.model_image_quality = None;
    capture_info.model_image_bytes = None;
    capture_info.model_image_encode_ms = None;
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

fn rect_contains_rect(outer: &RectF, inner: &RectF) -> bool {
    const EPSILON: f64 = 0.000_001;
    outer.space == inner.space
        && outer.width > 0.0
        && outer.height > 0.0
        && inner.width > 0.0
        && inner.height > 0.0
        && inner.x >= outer.x - EPSILON
        && inner.y >= outer.y - EPSILON
        && inner.right() <= outer.right() + EPSILON
        && inner.bottom() <= outer.bottom() + EPSILON
}

fn pipewire_source_covers_all_displays(
    capture_info: &CaptureInfo,
    environment: &EnvironmentInfo,
) -> bool {
    if capture_info.capture_scope != CaptureScope::AllDisplays {
        return true;
    }
    let Some(union) = virtual_desktop_rect(&environment.displays) else {
        return false;
    };
    capture_info
        .source_logical_rect
        .as_ref()
        .is_some_and(|source| {
            rect_contains_rect(source, &union) && rect_contains_rect(&union, source)
        })
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
    let (capture_path, raw_pixel_size) = match region_target {
        Some(target) => {
            let raw_pixel_size = raw_pixel_size.ok_or_else(|| {
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
            let cropped_path = screenshot::crop_capture(snapshot_id, raw_path, crop.pixel_rect)?;
            capture_info.logical_rect = Some(crop.logical_rect);
            capture_info.capture_scope = target.capture_scope.clone();
            capture_info.display = target.display.clone();
            (
                cropped_path,
                Some(PixelSize {
                    width: crop.pixel_rect.width,
                    height: crop.pixel_rect.height,
                }),
            )
        }
        None => {
            if capture_info.logical_rect.is_none()
                && let Some(logical_rect) =
                    infer_untargeted_capture_rect(&capture_info.capture_scope, environment)
            {
                if capture_info.source_logical_rect.is_none() {
                    capture_info.source_logical_rect = Some(logical_rect.clone());
                }
                capture_info.logical_rect = Some(logical_rect);
            }
            (raw_path.to_path_buf(), raw_pixel_size)
        }
    };
    let model_capture = screenshot::prepare_model_capture(snapshot_id, &capture_path)?;
    capture_info.coordinate_space = Some(CoordinateSpace::StreamPixels);
    capture_info.screenshot_path = Some(model_capture.path.display().to_string());
    capture_info.pixel_size = model_capture.pixel_size;
    capture_info.original_screenshot_path = model_capture
        .original_path
        .map(|path| path.display().to_string());
    capture_info.original_pixel_size = raw_pixel_size.or(model_capture.original_pixel_size);
    capture_info.model_image_format = Some(match model_capture.format {
        screenshot::ModelScreenshotFormat::Jpeg => ModelImageFormat::Jpeg,
        screenshot::ModelScreenshotFormat::Webp => ModelImageFormat::Webp,
    });
    capture_info.model_image_quality = Some(model_capture.quality);
    capture_info.model_image_bytes = model_capture.bytes;
    capture_info.model_image_encode_ms = Some(model_capture.encode_ms);
    update_model_capture_scale(capture_info);
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

fn infer_untargeted_capture_rect(
    capture_scope: &CaptureScope,
    environment: &EnvironmentInfo,
) -> Option<RectF> {
    if capture_scope != &CaptureScope::AllDisplays {
        return None;
    }
    virtual_desktop_rect(&environment.displays)
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
                BackendErrorCode::Internal,
                "targeted screenshot requires capture source geometry",
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

fn update_model_capture_scale(capture_info: &mut CaptureInfo) {
    capture_info.logical_to_pixel_scale = None;
    if let (Some(pixel_size), Some(logical_rect)) = (
        capture_info.pixel_size.as_ref(),
        capture_info.logical_rect.as_ref(),
    ) && logical_rect.width > 0.0
    {
        capture_info.logical_to_pixel_scale =
            Some(f64::from(pixel_size.width) / logical_rect.width);
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
        diagnostics.push(
            BackendErrorCode::PipeWireStreamFailed,
            if used_screenshot_fallback {
                "Live PipeWire frame capture failed before the snapshot image was produced"
            } else {
                "Live PipeWire frame capture failed and no fallback image was produced"
            },
            Some(error.message.clone()),
        );
        if used_screenshot_fallback {
            diagnostics.push(
                BackendErrorCode::CaptureBackendDowngraded,
                "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback",
                Some(
                    "primary_backend=portal_pipe_wire image_backend=portal_screenshot".to_string(),
                ),
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
                "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback",
                Some(
                    "primary_backend=portal_pipe_wire image_backend=portal_screenshot".to_string(),
                ),
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
        CaptureRegionTarget, compatible_dispatch_source, initial_capture,
        pipewire_source_covers_all_displays, pixel_crop_for_target, push_diagnostics,
        should_fallback_to_screenshot,
    };
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
    use sky_cua_platform::model::{
        CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace,
        EnvironmentInfo, InputBackendKind, PixelSize, PortalCapabilities, RectF,
        SemanticBackendKind, SessionKind,
    };

    fn wayland_pipewire_environment() -> EnvironmentInfo {
        EnvironmentInfo {
            session_kind: SessionKind::Wayland,
            compositor: Some("kde-kwin-wayland".to_string()),
            desktop_environment: Some("KDE".to_string()),
            capture_backend: CaptureBackendKind::PortalPipeWire,
            input_backend: InputBackendKind::PortalRemoteDesktop,
            semantic_backend: SemanticBackendKind::Atspi,
            portal_capabilities: PortalCapabilities {
                screencast_version: Some(5),
                remote_desktop_version: Some(2),
                screenshot_version: Some(2),
                available_source_types: None,
                available_cursor_modes: None,
                available_device_types: None,
            },
            xdg_session_type: Some("wayland".to_string()),
            display: None,
            wayland_display: Some("wayland-0".to_string()),
            displays: Vec::new(),
        }
    }

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
    fn pipewire_all_displays_requires_virtual_desktop_source() {
        let mut environment = wayland_pipewire_environment();
        environment.displays = vec![
            test_display("kwin:eDP-1", 0, true, 0.0, 0.0, 1920.0, 1080.0),
            test_display("kwin:HDMI-A-1", 1, false, 1920.0, 0.0, 1280.0, 720.0),
        ];
        let mut capture = capture_with_backend(
            CaptureBackendKind::PortalPipeWire,
            Some(CaptureBackendKind::PortalPipeWire),
            None,
        );
        capture.capture_scope = CaptureScope::AllDisplays;
        capture.source_logical_rect = Some(environment.displays[0].logical_rect.clone());

        assert!(!pipewire_source_covers_all_displays(&capture, &environment));

        capture.source_logical_rect = super::virtual_desktop_rect(&environment.displays);
        assert!(pipewire_source_covers_all_displays(&capture, &environment));

        environment.displays.clear();
        assert!(!pipewire_source_covers_all_displays(&capture, &environment));
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
            None
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
            None
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
            Some(&target)
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
            Some(&target)
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
            Some(&target)
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
            Some(&target)
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
            Some("primary_backend=portal_pipe_wire image_backend=portal_screenshot")
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
}
