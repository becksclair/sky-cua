use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    ActionRequest, CaptureInfo, CoordinateSpace, ElementNode, InputBackendKind,
};

use crate::coords::{center_of, desktop_to_stream, logical_to_pixel, rect_contains_rect};

fn missing_element_bounds_error(element: &ElementNode, target_kind: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidRequest,
        format!(
            "element {} did not include bounds, so a {target_kind} target cannot be derived",
            element.element_index
        ),
    )
}

pub(crate) fn input_backend_for(request: &ActionRequest) -> InputBackendKind {
    request
        .environment
        .as_ref()
        .map(|environment| environment.input_backend.clone())
        .unwrap_or(InputBackendKind::None)
}

pub(crate) fn effective_pointer_input_backend_for_target(
    request: &ActionRequest,
) -> InputBackendKind {
    input_backend_for(request)
}

pub(crate) fn virtual_scroll_steps_from_delta(delta_y: Option<f64>) -> Option<i32> {
    let delta_y = delta_y?;
    if delta_y == 0.0 {
        return None;
    }
    let magnitude = (delta_y.abs() / 120.0).ceil().max(1.0) as i32;
    Some(if delta_y.is_sign_positive() {
        -magnitude
    } else {
        magnitude
    })
}

pub(crate) fn element_is_x11_fallback(element: &ElementNode) -> bool {
    element.role.starts_with("x11_")
        || element
            .state_flags
            .iter()
            .any(|flag| flag == "x11_fallback")
}

pub(crate) fn effective_keyboard_input_backend(
    request: &ActionRequest,
    x11_window_present: bool,
    xtest_available: bool,
) -> InputBackendKind {
    let backend = input_backend_for(request);
    if backend == InputBackendKind::PortalRemoteDesktop && x11_window_present && xtest_available {
        return InputBackendKind::XTest;
    }
    backend
}

pub(crate) fn effective_keyboard_input_backend_for_target(
    request: &ActionRequest,
    x11_window_present: bool,
    target_window_id: Option<&str>,
    xtest_available: bool,
) -> InputBackendKind {
    if target_window_id.is_some() {
        if xtest_available {
            InputBackendKind::XTest
        } else {
            InputBackendKind::None
        }
    } else {
        effective_keyboard_input_backend(request, x11_window_present, xtest_available)
    }
}

pub(crate) fn action_point_for_backend(
    request: &ActionRequest,
    backend: InputBackendKind,
) -> Result<(f64, f64), BackendError> {
    if let Some(point) = explicit_point(&request.arguments) {
        return point_from_action_pixels(point, request, backend);
    }
    let element = request.resolved_element.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "this action requires either explicit x/y coordinates or a resolved element target",
        )
    })?;

    point_for_element_for_backend(
        element,
        request.resolved_capture.as_ref(),
        backend,
        request.snapshot_id.is_some(),
    )
}

pub(crate) fn point_for_element_for_backend(
    element: &ElementNode,
    capture: Option<&CaptureInfo>,
    backend: InputBackendKind,
    snapshot_based: bool,
) -> Result<(f64, f64), BackendError> {
    match backend {
        InputBackendKind::PortalRemoteDesktop => {
            if element_is_x11_fallback(element) {
                point_for_x11_element_through_portal(element, capture, snapshot_based)
            } else {
                validate_portal_dispatch_source(capture, snapshot_based)?;
                point_for_element(element, capture)
            }
        }
        InputBackendKind::XTest => point_for_x11_element(element, capture),
        InputBackendKind::LinuxVirtualInput => point_for_linux_virtual_element(element, capture),
        InputBackendKind::SendInput
        | InputBackendKind::WindowsMessages
        | InputBackendKind::None => point_for_element(element, None),
    }
}

pub(crate) fn explicit_point(arguments: &serde_json::Value) -> Option<(f64, f64)> {
    point_from_fields(arguments, "x", "y")
}

pub(crate) fn drag_from_point(
    request: &ActionRequest,
    backend: InputBackendKind,
) -> Result<(f64, f64), BackendError> {
    if let Some(point) = point_from_fields(&request.arguments, "from_x", "from_y")
        .or_else(|| explicit_point(&request.arguments))
    {
        return point_from_action_pixels(point, request, backend);
    }

    let element = request.resolved_element.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "drag requires either element_index or explicit from_x/from_y coordinates",
        )
    })?;
    point_for_element_for_backend(
        element,
        request.resolved_capture.as_ref(),
        backend,
        request.snapshot_id.is_some(),
    )
}

pub(crate) fn drag_to_point(
    request: &ActionRequest,
    backend: InputBackendKind,
) -> Result<Option<(f64, f64)>, BackendError> {
    point_from_fields(&request.arguments, "to_x", "to_y")
        .map(|point| point_from_action_pixels(point, request, backend))
        .transpose()
}

fn point_for_element(
    element: &ElementNode,
    capture: Option<&CaptureInfo>,
) -> Result<(f64, f64), BackendError> {
    let bounds = element
        .bounds
        .as_ref()
        .ok_or_else(|| missing_element_bounds_error(element, "physical action"))?;
    let center = center_of(bounds);
    if let Some(capture) = capture
        && let Some(stream_point) = portal_stream_point_from_desktop(center, capture)
    {
        return Ok(stream_point);
    }
    Ok(center)
}

fn point_for_x11_element(
    element: &ElementNode,
    capture: Option<&CaptureInfo>,
) -> Result<(f64, f64), BackendError> {
    let bounds = element
        .bounds
        .as_ref()
        .ok_or_else(|| missing_element_bounds_error(element, "physical action"))?;
    let center = center_of(bounds);
    if bounds.space == CoordinateSpace::DesktopLogical
        && let Some(capture) = capture
        && let (Some(logical_rect), Some(original_pixel_size)) = (
            capture.logical_rect.as_ref(),
            capture.original_pixel_size.as_ref(),
        )
        && let Some(pixel_point) = logical_to_pixel(center, logical_rect, original_pixel_size)
    {
        return Ok(pixel_point);
    }
    Ok(center)
}

pub(crate) fn point_for_x11_element_through_portal(
    element: &ElementNode,
    capture: Option<&CaptureInfo>,
    snapshot_based: bool,
) -> Result<(f64, f64), BackendError> {
    let bounds = element
        .bounds
        .as_ref()
        .ok_or_else(|| missing_element_bounds_error(element, "physical action"))?;
    validate_portal_dispatch_source(capture, snapshot_based)?;
    let center = center_of(bounds);
    if let Some(capture) = capture
        && let (Some(logical_rect), Some(original_pixel_size)) = (
            capture.logical_rect.as_ref(),
            capture.original_pixel_size.as_ref(),
        )
        && logical_rect.width > 0.0
        && logical_rect.height > 0.0
        && original_pixel_size.width > 0
        && original_pixel_size.height > 0
    {
        let rel_x = center.0 / f64::from(original_pixel_size.width);
        let rel_y = center.1 / f64::from(original_pixel_size.height);
        return Ok((rel_x * logical_rect.width, rel_y * logical_rect.height));
    }
    if snapshot_based && bounds.space == CoordinateSpace::DesktopLogical {
        return Err(missing_portal_dispatch_source_error());
    }
    Ok(center)
}

fn point_for_linux_virtual_element(
    element: &ElementNode,
    capture: Option<&CaptureInfo>,
) -> Result<(f64, f64), BackendError> {
    let bounds = element
        .bounds
        .as_ref()
        .ok_or_else(|| missing_element_bounds_error(element, "Linux virtual input"))?;
    let center = center_of(bounds);
    match bounds.space {
        CoordinateSpace::DesktopLogical => Ok(center),
        CoordinateSpace::StreamLogical => {
            let logical_rect = capture
                .and_then(|capture| capture.logical_rect.as_ref())
                .ok_or_else(missing_linux_virtual_logical_rect_error)?;
            Ok((logical_rect.x + center.0, logical_rect.y + center.1))
        }
        CoordinateSpace::StreamPixels => {
            linux_virtual_point_from_screenshot_pixels(center, capture, true)
        }
    }
}

fn point_from_fields(
    arguments: &serde_json::Value,
    x_field: &str,
    y_field: &str,
) -> Option<(f64, f64)> {
    let x = arguments.get(x_field).and_then(serde_json::Value::as_f64)?;
    let y = arguments.get(y_field).and_then(serde_json::Value::as_f64)?;
    Some((x, y))
}

fn point_from_action_pixels(
    point: (f64, f64),
    request: &ActionRequest,
    backend: InputBackendKind,
) -> Result<(f64, f64), BackendError> {
    if request.snapshot_id.is_some() {
        validate_snapshot_pixel_point(point, request.resolved_capture.as_ref())?;
    }
    if backend == InputBackendKind::LinuxVirtualInput {
        return linux_virtual_point_from_screenshot_pixels(
            point,
            request.resolved_capture.as_ref(),
            request.snapshot_id.is_some(),
        );
    }
    if backend == InputBackendKind::PortalRemoteDesktop {
        validate_portal_dispatch_source(
            request.resolved_capture.as_ref(),
            request.snapshot_id.is_some(),
        )?;
    }
    Ok(point_from_screenshot_pixels(
        point,
        request.resolved_capture.as_ref(),
        backend,
    ))
}

pub(crate) fn point_from_screenshot_pixels(
    point: (f64, f64),
    capture: Option<&CaptureInfo>,
    backend: InputBackendKind,
) -> (f64, f64) {
    let Some(capture) = capture else {
        return point;
    };
    let Some(pixel_size) = capture.pixel_size.as_ref() else {
        return point;
    };
    if pixel_size.width == 0 || pixel_size.height == 0 {
        return point;
    }

    match backend {
        InputBackendKind::PortalRemoteDesktop => {
            if let Some(desktop_point) = desktop_point_from_screenshot_pixels(point, capture) {
                return portal_stream_point_from_desktop(desktop_point, capture)
                    .unwrap_or(desktop_point);
            }
            point
        }
        InputBackendKind::XTest => {
            if let Some(desktop_point) = desktop_point_from_screenshot_pixels(point, capture) {
                return desktop_point;
            }
            let rel_x = point.0 / f64::from(pixel_size.width);
            let rel_y = point.1 / f64::from(pixel_size.height);
            if let Some(original_pixel_size) = capture.original_pixel_size.as_ref() {
                return (
                    rel_x * f64::from(original_pixel_size.width),
                    rel_y * f64::from(original_pixel_size.height),
                );
            }
            point
        }
        InputBackendKind::LinuxVirtualInput => {
            if let Some(desktop_point) = desktop_point_from_screenshot_pixels(point, capture) {
                return desktop_point;
            }
            point
        }
        InputBackendKind::SendInput | InputBackendKind::WindowsMessages => point,
        InputBackendKind::None => point,
    }
}

fn validate_snapshot_pixel_point(
    point: (f64, f64),
    capture: Option<&CaptureInfo>,
) -> Result<(), BackendError> {
    let capture = capture.ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "snapshot-based explicit coordinates require capture metadata",
        )
    })?;
    let pixel_size = capture.pixel_size.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "snapshot-based explicit coordinates require capture pixel_size metadata",
        )
    })?;
    if pixel_size.width == 0 || pixel_size.height == 0 {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "snapshot-based explicit coordinates cannot target a zero-sized capture",
        ));
    }
    let width = f64::from(pixel_size.width);
    let height = f64::from(pixel_size.height);
    if point.0 < 0.0 || point.1 < 0.0 || point.0 >= width || point.1 >= height {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!(
                "snapshot coordinates x={},y={} are outside the captured image bounds {}x{}; use pixel coordinates from this snapshot's screenshot_path image",
                point.0, point.1, pixel_size.width, pixel_size.height
            ),
        ));
    }
    Ok(())
}

fn desktop_point_from_screenshot_pixels(
    point: (f64, f64),
    capture: &CaptureInfo,
) -> Option<(f64, f64)> {
    let pixel_size = capture.pixel_size.as_ref()?;
    if pixel_size.width == 0 || pixel_size.height == 0 {
        return None;
    }
    let logical_rect = capture.logical_rect.as_ref()?;
    if logical_rect.width <= 0.0 || logical_rect.height <= 0.0 {
        return None;
    }
    let rel_x = point.0 / f64::from(pixel_size.width);
    let rel_y = point.1 / f64::from(pixel_size.height);
    match logical_rect.space {
        CoordinateSpace::DesktopLogical | CoordinateSpace::StreamLogical => Some((
            logical_rect.x + (rel_x * logical_rect.width),
            logical_rect.y + (rel_y * logical_rect.height),
        )),
        CoordinateSpace::StreamPixels => None,
    }
}

fn portal_stream_point_from_desktop(
    point: (f64, f64),
    capture: &CaptureInfo,
) -> Option<(f64, f64)> {
    if let Some(source_logical_rect) = capture.source_logical_rect.as_ref()
        && let Some(stream_point) = desktop_to_stream(point, source_logical_rect)
    {
        return Some(stream_point);
    }
    let logical_rect = capture.logical_rect.as_ref()?;
    match logical_rect.space {
        CoordinateSpace::DesktopLogical => desktop_to_stream(point, logical_rect),
        CoordinateSpace::StreamLogical => Some(point),
        CoordinateSpace::StreamPixels => None,
    }
}

fn validate_portal_dispatch_source(
    capture: Option<&CaptureInfo>,
    snapshot_based: bool,
) -> Result<(), BackendError> {
    if !snapshot_based {
        return Ok(());
    }
    let Some(capture) = capture else {
        return Ok(());
    };
    let Some(logical_rect) = capture.logical_rect.as_ref() else {
        return Err(missing_portal_dispatch_source_error());
    };
    match logical_rect.space {
        CoordinateSpace::StreamLogical => Ok(()),
        CoordinateSpace::DesktopLogical => {
            let Some(source_logical_rect) = capture.source_logical_rect.as_ref() else {
                return Err(missing_portal_dispatch_source_error());
            };
            if rect_contains_rect(source_logical_rect, logical_rect) {
                Ok(())
            } else {
                Err(missing_portal_dispatch_source_error())
            }
        }
        CoordinateSpace::StreamPixels => Err(missing_portal_dispatch_source_error()),
    }
}

fn missing_portal_dispatch_source_error() -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidRequest,
        "Portal RemoteDesktop snapshot actions require capture source geometry that covers the screenshot; this snapshot image was produced outside the active RemoteDesktop stream",
    )
}

fn linux_virtual_point_from_screenshot_pixels(
    point: (f64, f64),
    capture: Option<&CaptureInfo>,
    snapshot_based: bool,
) -> Result<(f64, f64), BackendError> {
    let Some(capture) = capture else {
        if snapshot_based {
            return Err(missing_linux_virtual_capture_error());
        }
        return Ok(point);
    };
    let pixel_size = capture.pixel_size.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "Linux virtual input requires capture pixel_size to map screenshot pixels to desktop logical coordinates",
        )
    })?;
    if pixel_size.width == 0 || pixel_size.height == 0 {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "Linux virtual input cannot map screenshot pixels from a zero-sized capture",
        ));
    }
    let logical_rect = capture
        .logical_rect
        .as_ref()
        .ok_or_else(missing_linux_virtual_logical_rect_error)?;
    if logical_rect.space != CoordinateSpace::DesktopLogical
        || logical_rect.width <= 0.0
        || logical_rect.height <= 0.0
    {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "Linux virtual input requires a positive desktop-logical capture logical_rect",
        ));
    }

    let rel_x = point.0 / f64::from(pixel_size.width);
    let rel_y = point.1 / f64::from(pixel_size.height);
    Ok((
        logical_rect.x + (rel_x * logical_rect.width),
        logical_rect.y + (rel_y * logical_rect.height),
    ))
}

fn missing_linux_virtual_capture_error() -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidRequest,
        "Linux virtual input requires capture metadata for snapshot-based screenshot coordinates",
    )
}

fn missing_linux_virtual_logical_rect_error() -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidRequest,
        "Linux virtual input requires capture logical_rect to map screenshot coordinates to desktop logical coordinates",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        action_point_for_backend, drag_from_point, drag_to_point,
        effective_pointer_input_backend_for_target, explicit_point,
        point_for_x11_element_through_portal, point_from_screenshot_pixels,
        virtual_scroll_steps_from_delta,
    };
    use serde_json::json;
    use sky_cua_platform::diagnostics::BackendErrorCode;
    use sky_cua_platform::model::test_support::wayland_pipewire_environment;
    use sky_cua_platform::model::{
        ActionName, ActionRequest, CaptureBackendKind, CaptureInfo, CaptureScope, CoordinateSpace,
        ElementNode, EnvironmentInfo, InputBackendKind, ModelImageFormat, PixelSize, RectF,
    };

    #[test]
    fn maps_scroll_delta_to_portal_discrete_steps() {
        assert_eq!(virtual_scroll_steps_from_delta(Some(-180.0)), Some(2));
        assert_eq!(virtual_scroll_steps_from_delta(Some(120.0)), Some(-1));
        assert_eq!(virtual_scroll_steps_from_delta(Some(0.0)), None);
        assert_eq!(virtual_scroll_steps_from_delta(None), None);
    }

    #[test]
    fn xwayland_fallback_elements_stay_on_portal_pointer_backend() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: Some(1),
            arguments: json!({}),
            resolved_element: Some(ElementNode {
                element_index: 1,
                parent_index: Some(0),
                role: "x11_action_region".to_string(),
                name: None,
                description: None,
                value: None,
                text: None,
                numeric_value: None,
                supports_editable_text: false,
                state_flags: vec!["x11_fallback".to_string(), "physical_target".to_string()],
                semantic_actions: Vec::new(),
                bounds: Some(RectF {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                backend_ref: None,
            }),
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        assert_eq!(
            effective_pointer_input_backend_for_target(&request),
            InputBackendKind::PortalRemoteDesktop
        );
    }

    #[test]
    fn parses_explicit_drag_destination_coordinates() {
        let request = ActionRequest {
            action: ActionName::Drag,
            snapshot_id: None,
            element_index: None,
            arguments: json!({"to_x": 320.0, "to_y": 240.0}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: None,
        };
        assert_eq!(
            drag_to_point(&request, InputBackendKind::PortalRemoteDesktop).unwrap(),
            Some((320.0, 240.0))
        );
        let request_without_to = ActionRequest {
            arguments: json!({"x": 1.0, "y": 2.0}),
            ..request
        };
        assert_eq!(
            drag_to_point(&request_without_to, InputBackendKind::PortalRemoteDesktop).unwrap(),
            None
        );
    }

    #[test]
    fn parses_explicit_action_coordinates() {
        assert_eq!(
            explicit_point(&json!({"x": 640.0, "y": 360.0})),
            Some((640.0, 360.0))
        );
    }

    #[test]
    fn maps_screenshot_pixels_to_portal_stream_coordinates() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 2560.0,
                height: 1440.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            original_pixel_size: Some(PixelSize {
                width: 2560,
                height: 1440,
            }),
            logical_to_pixel_scale: Some(0.75),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        assert_eq!(
            point_from_screenshot_pixels(
                (960.0, 540.0),
                Some(&capture),
                InputBackendKind::PortalRemoteDesktop
            ),
            (1280.0, 720.0)
        );
    }

    #[test]
    fn portal_snapshot_pixels_without_logical_rect_are_not_remapped_to_raw_pixels() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("64".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: None,
            pixel_size: Some(PixelSize {
                width: 914,
                height: 900,
            }),
            original_pixel_size: Some(PixelSize {
                width: 2560,
                height: 2520,
            }),
            logical_to_pixel_scale: None,
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        let point = point_from_screenshot_pixels(
            (470.0, 280.0),
            Some(&capture),
            InputBackendKind::PortalRemoteDesktop,
        );

        assert_eq!(point, (470.0, 280.0));
    }

    #[test]
    fn snapshot_explicit_coordinates_reject_points_outside_model_image_bounds() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: None,
            arguments: json!({"x": 1316.0, "y": 785.0}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: Some(CaptureInfo {
                backend: CaptureBackendKind::PortalPipeWire,
                image_backend: Some(CaptureBackendKind::PortalPipeWire),
                capture_scope: CaptureScope::Unknown,
                display: None,
                coordinate_space: Some(CoordinateSpace::StreamPixels),
                stream_id: Some("64".to_string()),
                source_type: Some(1),
                mapping_id: None,
                source_logical_rect: None,
                logical_rect: None,
                pixel_size: Some(PixelSize {
                    width: 914,
                    height: 900,
                }),
                original_pixel_size: Some(PixelSize {
                    width: 2560,
                    height: 2520,
                }),
                logical_to_pixel_scale: None,
                screenshot_path: Some("/tmp/capture.jpg".to_string()),
                original_screenshot_path: Some("/tmp/capture.png".to_string()),
                model_image_format: Some(ModelImageFormat::Jpeg),
                model_image_quality: Some(85),
                model_image_bytes: Some(1234),
                model_image_encode_ms: Some(7),
            }),
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        let error =
            action_point_for_backend(&request, InputBackendKind::PortalRemoteDesktop).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("outside the captured image bounds"));
    }

    #[test]
    fn snapshot_explicit_coordinates_reject_image_edge_coordinates() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: None,
            arguments: json!({"x": 914.0, "y": 899.0}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: Some(CaptureInfo {
                backend: CaptureBackendKind::PortalPipeWire,
                image_backend: Some(CaptureBackendKind::PortalPipeWire),
                capture_scope: CaptureScope::Unknown,
                display: None,
                coordinate_space: Some(CoordinateSpace::StreamPixels),
                stream_id: Some("64".to_string()),
                source_type: Some(1),
                mapping_id: None,
                source_logical_rect: None,
                logical_rect: None,
                pixel_size: Some(PixelSize {
                    width: 914,
                    height: 900,
                }),
                original_pixel_size: Some(PixelSize {
                    width: 2560,
                    height: 2520,
                }),
                logical_to_pixel_scale: None,
                screenshot_path: Some("/tmp/capture.jpg".to_string()),
                original_screenshot_path: Some("/tmp/capture.png".to_string()),
                model_image_format: Some(ModelImageFormat::Jpeg),
                model_image_quality: Some(85),
                model_image_bytes: Some(1234),
                model_image_encode_ms: Some(7),
            }),
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        let error =
            action_point_for_backend(&request, InputBackendKind::PortalRemoteDesktop).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("outside the captured image bounds"));
    }

    #[test]
    fn maps_cropped_screenshot_pixels_to_portal_stream_coordinates() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 300.0,
                y: 200.0,
                width: 800.0,
                height: 600.0,
                space: CoordinateSpace::StreamLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 400,
                height: 300,
            }),
            original_pixel_size: Some(PixelSize {
                width: 800,
                height: 600,
            }),
            logical_to_pixel_scale: Some(0.5),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        assert_eq!(
            point_from_screenshot_pixels(
                (200.0, 150.0),
                Some(&capture),
                InputBackendKind::PortalRemoteDesktop
            ),
            (700.0, 500.0)
        );
    }

    #[test]
    fn maps_screenshot_pixels_to_original_x11_pixels() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::X11,
            image_backend: Some(CaptureBackendKind::X11),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: None,
            pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            original_pixel_size: Some(PixelSize {
                width: 2560,
                height: 1440,
            }),
            logical_to_pixel_scale: None,
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        assert_eq!(
            point_from_screenshot_pixels((960.0, 540.0), Some(&capture), InputBackendKind::XTest),
            (1280.0, 720.0)
        );
    }

    #[test]
    fn maps_cropped_screenshot_pixels_to_x11_root_pixels() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::X11,
            image_backend: Some(CaptureBackendKind::X11),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 640.0,
                y: 360.0,
                width: 800.0,
                height: 600.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 400,
                height: 300,
            }),
            original_pixel_size: Some(PixelSize {
                width: 800,
                height: 600,
            }),
            logical_to_pixel_scale: Some(0.5),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        assert_eq!(
            point_from_screenshot_pixels((200.0, 150.0), Some(&capture), InputBackendKind::XTest),
            (1040.0, 660.0)
        );
    }

    #[test]
    fn maps_screenshot_pixels_to_linux_virtual_desktop_logical_coordinates() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalScreenshot,
            image_backend: Some(CaptureBackendKind::PortalScreenshot),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 1920.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 2560,
                height: 1440,
            }),
            original_pixel_size: Some(PixelSize {
                width: 2560,
                height: 1440,
            }),
            logical_to_pixel_scale: Some(2.0),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        assert_eq!(
            point_from_screenshot_pixels(
                (1280.0, 720.0),
                Some(&capture),
                InputBackendKind::LinuxVirtualInput
            ),
            (2560.0, 360.0)
        );
    }

    #[test]
    fn portal_snapshot_coordinates_fail_without_dispatch_source_for_screenshot_fallback() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: None,
            arguments: json!({"x": 1280.0, "y": 720.0}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: Some(CaptureInfo {
                backend: CaptureBackendKind::PortalPipeWire,
                image_backend: Some(CaptureBackendKind::PortalScreenshot),
                capture_scope: CaptureScope::Display,
                display: None,
                coordinate_space: Some(CoordinateSpace::StreamPixels),
                stream_id: Some("116".to_string()),
                source_type: Some(1),
                mapping_id: None,
                source_logical_rect: None,
                logical_rect: Some(RectF {
                    x: 1920.0,
                    y: 0.0,
                    width: 1280.0,
                    height: 720.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                pixel_size: Some(PixelSize {
                    width: 2560,
                    height: 1440,
                }),
                original_pixel_size: Some(PixelSize {
                    width: 2560,
                    height: 1440,
                }),
                logical_to_pixel_scale: Some(2.0),
                screenshot_path: Some("/tmp/capture.jpg".to_string()),
                original_screenshot_path: Some("/tmp/capture.png".to_string()),
                model_image_format: Some(ModelImageFormat::Jpeg),
                model_image_quality: Some(85),
                model_image_bytes: Some(1234),
                model_image_encode_ms: Some(7),
            }),
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        let error = drag_from_point(&request, InputBackendKind::PortalRemoteDesktop).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("source geometry"));
    }

    #[test]
    fn portal_snapshot_coordinates_fail_without_dispatch_source_for_pipewire_capture() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: None,
            arguments: json!({"x": 470.0, "y": 280.0}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: Some(CaptureInfo {
                backend: CaptureBackendKind::PortalPipeWire,
                image_backend: Some(CaptureBackendKind::PortalPipeWire),
                capture_scope: CaptureScope::Unknown,
                display: None,
                coordinate_space: Some(CoordinateSpace::StreamPixels),
                stream_id: Some("64".to_string()),
                source_type: Some(1),
                mapping_id: None,
                source_logical_rect: None,
                logical_rect: None,
                pixel_size: Some(PixelSize {
                    width: 914,
                    height: 900,
                }),
                original_pixel_size: Some(PixelSize {
                    width: 2560,
                    height: 2520,
                }),
                logical_to_pixel_scale: None,
                screenshot_path: Some("/tmp/capture.jpg".to_string()),
                original_screenshot_path: Some("/tmp/capture.png".to_string()),
                model_image_format: Some(ModelImageFormat::Jpeg),
                model_image_quality: Some(85),
                model_image_bytes: Some(1234),
                model_image_encode_ms: Some(7),
            }),
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        let error =
            action_point_for_backend(&request, InputBackendKind::PortalRemoteDesktop).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("source geometry"));
    }

    #[test]
    fn portal_snapshot_semantic_element_maps_when_dispatch_source_covers_capture() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: Some(1),
            arguments: json!({}),
            resolved_element: Some(ElementNode {
                element_index: 1,
                parent_index: Some(0),
                role: "button".to_string(),
                name: Some("OK".to_string()),
                description: None,
                value: None,
                text: None,
                numeric_value: None,
                supports_editable_text: false,
                state_flags: Vec::new(),
                semantic_actions: Vec::new(),
                bounds: Some(RectF {
                    x: 110.0,
                    y: 70.0,
                    width: 40.0,
                    height: 20.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                backend_ref: None,
            }),
            resolved_target_element: None,
            resolved_capture: Some(CaptureInfo {
                backend: CaptureBackendKind::PortalPipeWire,
                image_backend: Some(CaptureBackendKind::PortalPipeWire),
                capture_scope: CaptureScope::Unknown,
                display: None,
                coordinate_space: Some(CoordinateSpace::StreamPixels),
                stream_id: Some("116".to_string()),
                source_type: Some(1),
                mapping_id: None,
                source_logical_rect: Some(RectF {
                    x: 100.0,
                    y: 50.0,
                    width: 800.0,
                    height: 600.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                logical_rect: Some(RectF {
                    x: 100.0,
                    y: 50.0,
                    width: 800.0,
                    height: 600.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                pixel_size: Some(PixelSize {
                    width: 800,
                    height: 600,
                }),
                original_pixel_size: Some(PixelSize {
                    width: 800,
                    height: 600,
                }),
                logical_to_pixel_scale: Some(1.0),
                screenshot_path: Some("/tmp/capture.jpg".to_string()),
                original_screenshot_path: Some("/tmp/capture.png".to_string()),
                model_image_format: Some(ModelImageFormat::Jpeg),
                model_image_quality: Some(85),
                model_image_bytes: Some(1234),
                model_image_encode_ms: Some(7),
            }),
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        let point =
            action_point_for_backend(&request, InputBackendKind::PortalRemoteDesktop).unwrap();

        assert_eq!(point, (30.0, 30.0));
    }

    #[test]
    fn portal_snapshot_element_fails_without_dispatch_source_for_pipewire_capture() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: Some(1),
            arguments: json!({}),
            resolved_element: Some(ElementNode {
                element_index: 1,
                parent_index: Some(0),
                role: "button".to_string(),
                name: Some("Decode".to_string()),
                description: None,
                value: None,
                text: None,
                numeric_value: None,
                supports_editable_text: false,
                state_flags: Vec::new(),
                semantic_actions: Vec::new(),
                bounds: Some(RectF {
                    x: 100.0,
                    y: 50.0,
                    width: 80.0,
                    height: 24.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                backend_ref: None,
            }),
            resolved_target_element: None,
            resolved_capture: Some(CaptureInfo {
                backend: CaptureBackendKind::PortalPipeWire,
                image_backend: Some(CaptureBackendKind::PortalPipeWire),
                capture_scope: CaptureScope::Unknown,
                display: None,
                coordinate_space: Some(CoordinateSpace::StreamPixels),
                stream_id: Some("116".to_string()),
                source_type: Some(1),
                mapping_id: None,
                source_logical_rect: None,
                logical_rect: None,
                pixel_size: Some(PixelSize {
                    width: 1920,
                    height: 1080,
                }),
                original_pixel_size: Some(PixelSize {
                    width: 1920,
                    height: 1080,
                }),
                logical_to_pixel_scale: None,
                screenshot_path: Some("/tmp/capture.jpg".to_string()),
                original_screenshot_path: Some("/tmp/capture.png".to_string()),
                model_image_format: Some(ModelImageFormat::Jpeg),
                model_image_quality: Some(85),
                model_image_bytes: Some(1234),
                model_image_encode_ms: Some(7),
            }),
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        let error =
            action_point_for_backend(&request, InputBackendKind::PortalRemoteDesktop).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("source geometry"));
    }

    #[test]
    fn maps_cropped_screenshot_pixels_to_linux_virtual_desktop_logical_coordinates() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 640.0,
                y: 360.0,
                width: 800.0,
                height: 600.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 400,
                height: 300,
            }),
            original_pixel_size: Some(PixelSize {
                width: 800,
                height: 600,
            }),
            logical_to_pixel_scale: Some(0.5),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };

        assert_eq!(
            point_from_screenshot_pixels(
                (200.0, 150.0),
                Some(&capture),
                InputBackendKind::LinuxVirtualInput
            ),
            (1040.0, 660.0)
        );
    }

    #[test]
    fn linux_virtual_snapshot_coordinates_fail_without_logical_rect() {
        let request = ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: None,
            arguments: json!({"x": 640.0, "y": 360.0}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: Some(CaptureInfo {
                backend: CaptureBackendKind::PortalScreenshot,
                image_backend: Some(CaptureBackendKind::PortalScreenshot),
                capture_scope: CaptureScope::Unknown,
                display: None,
                coordinate_space: Some(CoordinateSpace::StreamPixels),
                stream_id: None,
                source_type: None,
                mapping_id: None,
                source_logical_rect: None,
                logical_rect: None,
                pixel_size: Some(PixelSize {
                    width: 1280,
                    height: 720,
                }),
                original_pixel_size: None,
                logical_to_pixel_scale: None,
                screenshot_path: Some("/tmp/capture.jpg".to_string()),
                original_screenshot_path: None,
                model_image_format: Some(ModelImageFormat::Jpeg),
                model_image_quality: Some(85),
                model_image_bytes: Some(1234),
                model_image_encode_ms: Some(7),
            }),
            resolved_focused_app: None,
            environment: Some(EnvironmentInfo {
                input_backend: InputBackendKind::LinuxVirtualInput,
                ..wayland_pipewire_environment()
            }),
        };

        let error = drag_from_point(&request, InputBackendKind::LinuxVirtualInput).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("capture logical_rect"));
    }

    #[test]
    fn maps_xwayland_x11_pixels_to_portal_logical_coordinates() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("166".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 1536.0,
                height: 864.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 1536.0,
                height: 864.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1440,
                height: 810,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            logical_to_pixel_scale: Some(0.9375),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };
        let element = ElementNode {
            element_index: 4,
            parent_index: Some(1),
            role: "x11_action_region".to_string(),
            name: None,
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: vec!["x11_fallback".to_string(), "physical_target".to_string()],
            semantic_actions: Vec::new(),
            bounds: Some(RectF {
                x: 896.0,
                y: 552.0,
                width: 32.0,
                height: 24.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            backend_ref: None,
        };

        let point = point_for_x11_element_through_portal(&element, Some(&capture), true).unwrap();

        assert!((point.0 - 729.6).abs() < 0.000_001);
        assert!((point.1 - 451.2).abs() < 0.000_001);
    }

    #[test]
    fn xwayland_portal_snapshot_element_fails_without_dispatch_source() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("166".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 1536.0,
                height: 864.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1440,
                height: 810,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            logical_to_pixel_scale: Some(0.9375),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };
        let element = ElementNode {
            element_index: 4,
            parent_index: Some(1),
            role: "x11_action_region".to_string(),
            name: None,
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: vec!["x11_fallback".to_string(), "physical_target".to_string()],
            semantic_actions: Vec::new(),
            bounds: Some(RectF {
                x: 896.0,
                y: 552.0,
                width: 32.0,
                height: 24.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            backend_ref: None,
        };

        let error =
            point_for_x11_element_through_portal(&element, Some(&capture), true).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("source geometry"));
    }

    #[test]
    fn maps_xwayland_drag_element_start_through_same_portal_scaling() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("166".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 1536.0,
                height: 864.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            logical_rect: Some(RectF {
                x: 100.0,
                y: 50.0,
                width: 1536.0,
                height: 864.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1440,
                height: 810,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            logical_to_pixel_scale: Some(0.9375),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        };
        let request = ActionRequest {
            action: ActionName::Drag,
            snapshot_id: Some("snapshot-1".to_string()),
            element_index: Some(4),
            arguments: json!({"to_x": 640.0, "to_y": 480.0}),
            resolved_element: Some(ElementNode {
                element_index: 4,
                parent_index: Some(1),
                role: "x11_action_region".to_string(),
                name: None,
                description: None,
                value: None,
                text: None,
                numeric_value: None,
                supports_editable_text: false,
                state_flags: vec!["x11_fallback".to_string(), "physical_target".to_string()],
                semantic_actions: Vec::new(),
                bounds: Some(RectF {
                    x: 896.0,
                    y: 552.0,
                    width: 32.0,
                    height: 24.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                backend_ref: None,
            }),
            resolved_target_element: None,
            resolved_capture: Some(capture),
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };

        let point = drag_from_point(&request, InputBackendKind::PortalRemoteDesktop).unwrap();

        assert!((point.0 - 729.6).abs() < 0.000_001);
        assert!((point.1 - 451.2).abs() < 0.000_001);
    }
}
