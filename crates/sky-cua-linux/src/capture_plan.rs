use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    CaptureBackendKind, CaptureInfo, CaptureScreenMode, CoordinateSpace, EnvironmentInfo,
    InputBackendKind, ModelImageFormat, PixelSize, SessionKind,
};

use crate::portal::remote_desktop::RemoteDesktopSessionManager;
use crate::portal::screenshot;
use crate::x11::capture as x11_capture;

#[derive(Debug)]
pub(crate) struct CapturePlanOutcome {
    pub(crate) capture: Option<CaptureInfo>,
    pub(crate) portal_session_error: Option<BackendError>,
    pub(crate) capture_error: Option<BackendError>,
}

pub(crate) async fn plan_capture(
    portal: &RemoteDesktopSessionManager,
    snapshot_id: &str,
    capture_screen: CaptureScreenMode,
    environment: &EnvironmentInfo,
    diagnostics: &mut DiagnosticBuilder,
) -> Result<CapturePlanOutcome, BackendError> {
    let should_capture_screen = capture_screen != CaptureScreenMode::Never;
    let mut capture = initial_capture(capture_screen, environment);
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
                    apply_model_capture(capture_info, snapshot_id, &frame.path, frame.pixel_size)?;
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
                    apply_model_capture(capture_info, snapshot_id, &frame.path, frame.pixel_size)?;
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
    ) {
        match screenshot::capture_still(snapshot_id).await {
            Ok(path) => {
                if let Some(capture_info) = capture.as_mut() {
                    capture_info.image_backend = Some(CaptureBackendKind::PortalScreenshot);
                    let original_pixel_size = screenshot::pixel_size_from_path(&path);
                    apply_model_capture(capture_info, snapshot_id, &path, original_pixel_size)?;
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

pub(crate) fn initial_capture(
    capture_screen: CaptureScreenMode,
    environment: &EnvironmentInfo,
) -> Option<CaptureInfo> {
    (capture_screen != CaptureScreenMode::Never
        && environment.capture_backend != CaptureBackendKind::None)
        .then_some(CaptureInfo {
            backend: environment.capture_backend.clone(),
            image_backend: None,
            coordinate_space: None,
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: None,
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
) -> bool {
    capture
        .as_ref()
        .is_some_and(|capture_info| capture_info.screenshot_path.is_none())
        && environment.portal_capabilities.screenshot_version.is_some()
        && !portal_approval_pending(portal_session_error)
        && !portal_approval_pending(capture_error)
        && matches!(environment.session_kind, SessionKind::Wayland)
}

fn apply_model_capture(
    capture_info: &mut CaptureInfo,
    snapshot_id: &str,
    raw_path: &std::path::Path,
    raw_pixel_size: Option<PixelSize>,
) -> Result<(), BackendError> {
    let model_capture = screenshot::prepare_model_capture(snapshot_id, raw_path)?;
    capture_info.coordinate_space = Some(CoordinateSpace::StreamPixels);
    capture_info.screenshot_path = Some(model_capture.path.display().to_string());
    capture_info.pixel_size = model_capture.pixel_size;
    capture_info.original_screenshot_path = model_capture
        .original_path
        .map(|path| path.display().to_string());
    capture_info.original_pixel_size = model_capture.original_pixel_size.or(raw_pixel_size);
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
    use super::{initial_capture, push_diagnostics, should_fallback_to_screenshot};
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
    use sky_cua_platform::model::{
        CaptureBackendKind, CaptureInfo, CaptureScreenMode, CoordinateSpace, EnvironmentInfo,
        InputBackendKind, PortalCapabilities, SemanticBackendKind, SessionKind,
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
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
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
            None
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
