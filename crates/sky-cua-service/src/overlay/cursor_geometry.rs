//! Pure cursor point/state derivation helpers for the overlay controller.
//!
//! These functions translate an [`ActionRequest`] (and its resolved capture and
//! element metadata) into the model/native cursor points and pre-dispatch
//! cursor state the controller drives. They are deliberately free of `self`,
//! diagnostics, and timestamps so they can be reasoned about and tested in
//! isolation. The controller state machine lives in the parent module.

use sky_cua_platform::model::{
    ActionName, ActionRequest, AgentCursorPoint, AgentCursorState, CaptureBackendKind, CaptureInfo,
    CoordinateSpace, ElementNode, PixelSize, RectF,
};

pub(super) fn state_from_action_request(request: &ActionRequest) -> Option<AgentCursorState> {
    let model_point = model_point_for_action(request);
    let native_point = native_point_for_action(request);
    if model_point.is_none() && native_point.is_none() {
        return None;
    }
    Some(AgentCursorState {
        visible: true,
        sequence: 0,
        model_point,
        native_point,
        snapshot_id: request.snapshot_id.clone(),
        source_action: Some(request.action.clone()),
        updated_at_ms: 0,
    })
}

pub(super) fn cursor_moving_action(action: &ActionName) -> bool {
    matches!(
        action,
        ActionName::Click | ActionName::PerformSecondaryAction | ActionName::Drag
    )
}

pub(super) fn pre_dispatch_state_from_action_request(
    request: &ActionRequest,
) -> Option<AgentCursorState> {
    if request.action != ActionName::Drag {
        return state_from_action_request(request);
    }
    let model_point = model_drag_start_point(request);
    let native_point = native_drag_start_point(request);
    if model_point.is_none() && native_point.is_none() {
        return None;
    }
    Some(AgentCursorState {
        visible: true,
        sequence: 0,
        model_point,
        native_point,
        snapshot_id: request.snapshot_id.clone(),
        source_action: Some(request.action.clone()),
        updated_at_ms: 0,
    })
}

pub(super) fn native_drag_start_point(request: &ActionRequest) -> Option<AgentCursorPoint> {
    explicit_native_point(request, "from_x", "from_y")
        .or_else(|| explicit_native_point(request, "x", "y"))
        .or_else(|| element_native_point(request.resolved_element.as_ref(), request))
}

pub(super) fn native_drag_target_point(request: &ActionRequest) -> Option<AgentCursorPoint> {
    explicit_native_point(request, "to_x", "to_y")
        .or_else(|| element_native_point(request.resolved_target_element.as_ref(), request))
}

fn model_drag_start_point(request: &ActionRequest) -> Option<AgentCursorPoint> {
    explicit_model_point(request, "from_x", "from_y")
        .or_else(|| explicit_model_point(request, "x", "y"))
        .or_else(|| element_model_point(request.resolved_element.as_ref(), request))
}

fn model_point_for_action(request: &ActionRequest) -> Option<AgentCursorPoint> {
    match request.action {
        ActionName::Click | ActionName::PerformSecondaryAction => {
            explicit_model_point(request, "x", "y")
                .or_else(|| element_model_point(request.resolved_element.as_ref(), request))
        }
        ActionName::Drag => explicit_model_point(request, "to_x", "to_y")
            .or_else(|| element_model_point(request.resolved_target_element.as_ref(), request)),
        _ => None,
    }
}

fn explicit_model_point(
    request: &ActionRequest,
    x_field: &str,
    y_field: &str,
) -> Option<AgentCursorPoint> {
    let x = request.arguments.get(x_field)?.as_f64()?;
    let y = request.arguments.get(y_field)?.as_f64()?;
    let capture = request.resolved_capture.as_ref()?;
    Some(AgentCursorPoint {
        x,
        y,
        coordinate_space: CoordinateSpace::StreamPixels,
        mapping_id: capture.mapping_id.clone(),
    })
}

pub(super) fn native_point_for_action(request: &ActionRequest) -> Option<AgentCursorPoint> {
    match request.action {
        ActionName::Click | ActionName::PerformSecondaryAction => {
            explicit_native_point(request, "x", "y")
                .or_else(|| element_native_point(request.resolved_element.as_ref(), request))
        }
        ActionName::Drag => explicit_native_point(request, "to_x", "to_y")
            .or_else(|| element_native_point(request.resolved_target_element.as_ref(), request)),
        _ => None,
    }
}

fn explicit_native_point(
    request: &ActionRequest,
    x_field: &str,
    y_field: &str,
) -> Option<AgentCursorPoint> {
    let x = request.arguments.get(x_field)?.as_f64()?;
    let y = request.arguments.get(y_field)?.as_f64()?;
    request
        .resolved_capture
        .as_ref()
        .and_then(|capture| stream_pixels_to_native_point((x, y), capture))
        .or_else(|| {
            request.snapshot_id.is_none().then_some(AgentCursorPoint {
                x,
                y,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
            })
        })
}

fn element_model_point(
    element: Option<&ElementNode>,
    request: &ActionRequest,
) -> Option<AgentCursorPoint> {
    let bounds = element?.bounds.as_ref()?;
    let capture = request.resolved_capture.as_ref()?;
    let (x, y) = rect_center(bounds);
    let (x, y) = point_to_stream_pixels((x, y), bounds.space.clone(), capture)?;
    Some(AgentCursorPoint {
        x,
        y,
        coordinate_space: CoordinateSpace::StreamPixels,
        mapping_id: capture.mapping_id.clone(),
    })
}

fn element_native_point(
    element: Option<&ElementNode>,
    request: &ActionRequest,
) -> Option<AgentCursorPoint> {
    let bounds = element?.bounds.as_ref()?;
    let capture = request.resolved_capture.as_ref();
    let (x, y) = rect_center(bounds);
    if let Some(capture) = capture
        && let Some(stream_pixels) = point_to_stream_pixels((x, y), bounds.space.clone(), capture)
        && let Some(native_point) = stream_pixels_to_native_point(stream_pixels, capture)
    {
        return Some(native_point);
    }
    match bounds.space {
        CoordinateSpace::DesktopLogical | CoordinateSpace::StreamLogical => {
            Some(AgentCursorPoint {
                x,
                y,
                coordinate_space: bounds.space.clone(),
                mapping_id: capture.and_then(|capture| capture.mapping_id.clone()),
            })
        }
        CoordinateSpace::StreamPixels => {
            stream_pixels_to_native_point((x, y), capture?).or_else(|| {
                Some(AgentCursorPoint {
                    x,
                    y,
                    coordinate_space: CoordinateSpace::StreamPixels,
                    mapping_id: capture.and_then(|capture| capture.mapping_id.clone()),
                })
            })
        }
    }
}

fn rect_center(bounds: &RectF) -> (f64, f64) {
    (
        bounds.x + (bounds.width / 2.0),
        bounds.y + (bounds.height / 2.0),
    )
}

fn point_to_stream_pixels(
    point: (f64, f64),
    space: CoordinateSpace,
    capture: &CaptureInfo,
) -> Option<(f64, f64)> {
    match space {
        CoordinateSpace::StreamPixels => Some(point),
        CoordinateSpace::DesktopLogical | CoordinateSpace::StreamLogical => {
            let pixel_size = capture.pixel_size.as_ref()?;
            point_to_pixels_through_rect(point, &space, capture.logical_rect.as_ref(), pixel_size)
                .or_else(|| {
                    (space == CoordinateSpace::StreamLogical)
                        .then_some(capture.logical_to_pixel_scale)
                        .flatten()
                        .map(|scale| (point.0 * scale, point.1 * scale))
                })
        }
    }
}

fn stream_pixels_to_native_point(
    point: (f64, f64),
    capture: &CaptureInfo,
) -> Option<AgentCursorPoint> {
    let pixel_size = capture.pixel_size.as_ref()?;
    if pixel_size.width == 0 || pixel_size.height == 0 {
        return None;
    }
    if let Some(logical_rect) = capture
        .logical_rect
        .as_ref()
        .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
    {
        let x = (point.0 / f64::from(pixel_size.width)) * logical_rect.width;
        let y = (point.1 / f64::from(pixel_size.height)) * logical_rect.height;
        if capture.backend == CaptureBackendKind::PortalPipeWire {
            if logical_rect.space == CoordinateSpace::DesktopLogical {
                return Some(AgentCursorPoint {
                    x: logical_rect.x + x,
                    y: logical_rect.y + y,
                    coordinate_space: CoordinateSpace::DesktopLogical,
                    mapping_id: capture.mapping_id.clone(),
                });
            }
            return Some(AgentCursorPoint {
                x,
                y,
                coordinate_space: CoordinateSpace::StreamLogical,
                mapping_id: capture.mapping_id.clone(),
            });
        }
        return Some(AgentCursorPoint {
            x: logical_rect.x + x,
            y: logical_rect.y + y,
            coordinate_space: logical_rect.space.clone(),
            mapping_id: capture.mapping_id.clone(),
        });
    }
    if let Some(scale) = capture
        .logical_to_pixel_scale
        .filter(|scale| scale.is_finite() && *scale > 0.0)
    {
        return Some(AgentCursorPoint {
            x: point.0 / scale,
            y: point.1 / scale,
            coordinate_space: CoordinateSpace::StreamLogical,
            mapping_id: capture.mapping_id.clone(),
        });
    }
    if capture.backend == CaptureBackendKind::X11
        && let Some(original_pixel_size) = capture.original_pixel_size.as_ref()
        && original_pixel_size.width > 0
        && original_pixel_size.height > 0
    {
        return Some(AgentCursorPoint {
            x: (point.0 / f64::from(pixel_size.width)) * f64::from(original_pixel_size.width),
            y: (point.1 / f64::from(pixel_size.height)) * f64::from(original_pixel_size.height),
            coordinate_space: CoordinateSpace::DesktopLogical,
            mapping_id: capture.mapping_id.clone(),
        });
    }
    Some(AgentCursorPoint {
        x: point.0,
        y: point.1,
        coordinate_space: CoordinateSpace::StreamPixels,
        mapping_id: capture.mapping_id.clone(),
    })
}

fn point_to_pixels_through_rect(
    point: (f64, f64),
    point_space: &CoordinateSpace,
    logical_rect: Option<&RectF>,
    pixel_size: &PixelSize,
) -> Option<(f64, f64)> {
    let logical_rect = logical_rect?;
    if &logical_rect.space != point_space || logical_rect.width <= 0.0 || logical_rect.height <= 0.0
    {
        return None;
    }
    let rel_x = (point.0 - logical_rect.x) / logical_rect.width;
    let rel_y = (point.1 - logical_rect.y) / logical_rect.height;
    Some((
        rel_x * f64::from(pixel_size.width),
        rel_y * f64::from(pixel_size.height),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sky_cua_platform::model::CaptureScope;

    use super::*;

    /// A `CaptureInfo` with every field at a neutral default. Individual tests
    /// override only the fields relevant to the case under test via struct
    /// update syntax.
    fn base_capture() -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalScreenshot,
            image_backend: None,
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: None,
            stream_id: None,
            source_type: None,
            mapping_id: Some("map-1".to_string()),
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
        }
    }

    /// A no-op `ActionRequest` with every field at a neutral default.
    /// Individual tests override only the fields relevant to the case.
    fn base_request(action: ActionName) -> ActionRequest {
        ActionRequest {
            action,
            appshot_id: None,
            snapshot_id: Some("snap-1".to_string()),
            element_index: None,
            arguments: json!({}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: None,
        }
    }

    fn pixel_size(width: u32, height: u32) -> PixelSize {
        PixelSize { width, height }
    }

    fn rect(space: CoordinateSpace, x: f64, y: f64, width: f64, height: f64) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
            space,
        }
    }

    fn element(bounds: Option<RectF>) -> ElementNode {
        ElementNode {
            element_index: 0,
            parent_index: None,
            role: "button".to_string(),
            name: Some("Go".to_string()),
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: Vec::new(),
            semantic_actions: Vec::new(),
            bounds,
            backend_ref: None,
        }
    }

    // -----------------------------------------------------------------
    // point_to_stream_pixels / stream_pixels_to_native_point round trips
    // -----------------------------------------------------------------

    #[test]
    fn stream_pixels_identity_at_scale_one() {
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(1000, 2000)),
            logical_rect: Some(rect(
                CoordinateSpace::DesktopLogical,
                0.0,
                0.0,
                1000.0,
                2000.0,
            )),
            ..base_capture()
        };
        let native = stream_pixels_to_native_point((500.0, 1000.0), &capture).expect("native");
        assert!((native.x - 500.0).abs() < 1e-9);
        assert!((native.y - 1000.0).abs() < 1e-9);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
    }

    #[test]
    fn stream_pixels_to_native_2x_logical_to_pixel_scale() {
        // `pixel_size` gates every resolution path (an early `?` return), so it
        // must be present even though this case exercises the `scale`
        // fallback rather than the `logical_rect` path.
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(400, 800)),
            logical_to_pixel_scale: Some(2.0),
            ..base_capture()
        };
        let native = stream_pixels_to_native_point((200.0, 400.0), &capture).expect("native");
        assert!((native.x - 100.0).abs() < 1e-9);
        assert!((native.y - 200.0).abs() < 1e-9);
        assert_eq!(native.coordinate_space, CoordinateSpace::StreamLogical);
    }

    #[test]
    fn stream_pixels_to_native_asymmetric_width_height_scaling() {
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(2000, 1000)),
            logical_rect: Some(rect(
                CoordinateSpace::StreamLogical,
                0.0,
                0.0,
                1000.0,
                250.0,
            )),
            ..base_capture()
        };
        // width scale is 0.5, height scale is 0.25 -- asymmetric on purpose.
        let native = stream_pixels_to_native_point((1000.0, 400.0), &capture).expect("native");
        assert!((native.x - 500.0).abs() < 1e-9);
        assert!((native.y - 100.0).abs() < 1e-9);
    }

    #[test]
    fn stream_pixels_round_trip_within_one_pixel() {
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(1920, 1080)),
            logical_rect: Some(rect(
                CoordinateSpace::DesktopLogical,
                10.0,
                20.0,
                960.0,
                540.0,
            )),
            ..base_capture()
        };
        let original_native = (300.5, 150.25);
        let stream_pixels = point_to_pixels_through_rect(
            original_native,
            &CoordinateSpace::DesktopLogical,
            capture.logical_rect.as_ref(),
            capture.pixel_size.as_ref().unwrap(),
        )
        .expect("to stream pixels");
        let back = stream_pixels_to_native_point(stream_pixels, &capture).expect("back to native");
        assert!((back.x - original_native.0).abs() < 1.0);
        assert!((back.y - original_native.1).abs() < 1.0);
    }

    #[test]
    fn point_to_stream_pixels_is_identity_for_stream_pixels_space() {
        let capture = base_capture();
        let point = point_to_stream_pixels((42.0, 84.0), CoordinateSpace::StreamPixels, &capture)
            .expect("identity");
        assert_eq!(point, (42.0, 84.0));
    }

    #[test]
    fn point_to_stream_pixels_scales_stream_logical_via_scale_factor() {
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(200, 100)),
            logical_to_pixel_scale: Some(2.0),
            ..base_capture()
        };
        // No logical_rect, so the scale-factor fallback path is taken.
        let point = point_to_stream_pixels((10.0, 20.0), CoordinateSpace::StreamLogical, &capture)
            .expect("scaled");
        assert_eq!(point, (20.0, 40.0));
    }

    // -----------------------------------------------------------------
    // Degenerate inputs
    // -----------------------------------------------------------------

    #[test]
    fn stream_pixels_to_native_none_on_zero_pixel_size() {
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(0, 0)),
            ..base_capture()
        };
        assert_eq!(stream_pixels_to_native_point((10.0, 10.0), &capture), None);
    }

    #[test]
    fn stream_pixels_to_native_none_when_pixel_size_absent() {
        // `pixel_size` is an early `?` gate: with it absent the function
        // returns `None` immediately, before any of the logical_rect/scale/X11
        // resolution paths run.
        let capture = base_capture();
        assert_eq!(stream_pixels_to_native_point((5.0, 6.0), &capture), None);
    }

    #[test]
    fn stream_pixels_to_native_falls_back_to_stream_pixels_identity() {
        // pixel_size present (so the early gate passes) but no logical_rect,
        // no logical_to_pixel_scale, and backend is not X11: every named
        // resolution path is unavailable, so the function falls through to its
        // final identity-in-StreamPixels fallback rather than returning None.
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(1000, 1000)),
            ..base_capture()
        };
        let native = stream_pixels_to_native_point((5.0, 6.0), &capture).expect("fallback point");
        assert_eq!(native.coordinate_space, CoordinateSpace::StreamPixels);
        assert_eq!((native.x, native.y), (5.0, 6.0));
    }

    #[test]
    fn point_to_pixels_through_rect_none_on_zero_size_rect() {
        let zero_rect = rect(CoordinateSpace::DesktopLogical, 0.0, 0.0, 0.0, 0.0);
        let size = pixel_size(100, 100);
        assert_eq!(
            point_to_pixels_through_rect(
                (1.0, 1.0),
                &CoordinateSpace::DesktopLogical,
                Some(&zero_rect),
                &size
            ),
            None
        );
    }

    #[test]
    fn point_to_pixels_through_rect_none_on_mismatched_space() {
        let r = rect(CoordinateSpace::StreamLogical, 0.0, 0.0, 100.0, 100.0);
        let size = pixel_size(100, 100);
        assert_eq!(
            point_to_pixels_through_rect(
                (1.0, 1.0),
                &CoordinateSpace::DesktopLogical,
                Some(&r),
                &size
            ),
            None
        );
    }

    // -----------------------------------------------------------------
    // native_point_for_action / native_drag_start_point
    // -----------------------------------------------------------------

    #[test]
    fn native_point_for_action_uses_explicit_coordinate() {
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(1000, 1000)),
            logical_rect: Some(rect(
                CoordinateSpace::DesktopLogical,
                0.0,
                0.0,
                1000.0,
                1000.0,
            )),
            ..base_capture()
        };
        let request = ActionRequest {
            arguments: json!({"x": 250.0, "y": 500.0}),
            resolved_capture: Some(capture),
            ..base_request(ActionName::Click)
        };
        let point = native_point_for_action(&request).expect("native point");
        assert!((point.x - 250.0).abs() < 1e-9);
        assert!((point.y - 500.0).abs() < 1e-9);
    }

    #[test]
    fn native_point_for_action_resolves_through_element_bounds() {
        let capture = CaptureInfo {
            pixel_size: Some(pixel_size(1000, 1000)),
            logical_rect: Some(rect(
                CoordinateSpace::DesktopLogical,
                0.0,
                0.0,
                1000.0,
                1000.0,
            )),
            ..base_capture()
        };
        let bounds = rect(CoordinateSpace::DesktopLogical, 100.0, 100.0, 50.0, 50.0);
        let request = ActionRequest {
            resolved_element: Some(element(Some(bounds))),
            resolved_capture: Some(capture),
            ..base_request(ActionName::Click)
        };
        // Element center is (125, 125).
        let point = native_point_for_action(&request).expect("native point");
        assert!((point.x - 125.0).abs() < 1e-9);
        assert!((point.y - 125.0).abs() < 1e-9);
    }

    #[test]
    fn native_point_for_action_none_when_no_point_available() {
        let request = base_request(ActionName::Click);
        assert_eq!(native_point_for_action(&request), None);
    }

    #[test]
    fn native_drag_start_point_prefers_from_coordinate() {
        // The `snapshot_id.is_none()` legacy fallback in `explicit_native_point`
        // is the only path that resolves an explicit coordinate without a
        // `resolved_capture`; a snapshot-bearing request instead requires a
        // capture to map through, so this case models the non-snapshot flow.
        let request = ActionRequest {
            snapshot_id: None,
            arguments: json!({"from_x": 12.0, "from_y": 34.0, "x": 1.0, "y": 2.0}),
            ..base_request(ActionName::Drag)
        };
        let point = native_drag_start_point(&request).expect("start point");
        assert_eq!(point.coordinate_space, CoordinateSpace::DesktopLogical);
        assert!((point.x - 12.0).abs() < 1e-9);
        assert!((point.y - 34.0).abs() < 1e-9);
    }

    #[test]
    fn native_drag_start_point_none_when_nothing_resolves() {
        let request = base_request(ActionName::Drag);
        assert_eq!(native_drag_start_point(&request), None);
    }

    // -----------------------------------------------------------------
    // PortalPipeWire vs non-PipeWire branch of point_to_stream_pixels /
    // stream_pixels_to_native_point
    // -----------------------------------------------------------------

    #[test]
    fn stream_pixels_to_native_pipewire_desktop_logical_offsets_by_rect_origin() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            pixel_size: Some(pixel_size(1000, 1000)),
            logical_rect: Some(rect(
                CoordinateSpace::DesktopLogical,
                50.0,
                60.0,
                1000.0,
                1000.0,
            )),
            ..base_capture()
        };
        let native = stream_pixels_to_native_point((100.0, 200.0), &capture).expect("native");
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert!((native.x - 150.0).abs() < 1e-9);
        assert!((native.y - 260.0).abs() < 1e-9);
    }

    #[test]
    fn stream_pixels_to_native_pipewire_stream_logical_has_no_offset() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            pixel_size: Some(pixel_size(1000, 1000)),
            logical_rect: Some(rect(
                CoordinateSpace::StreamLogical,
                999.0,
                999.0,
                1000.0,
                1000.0,
            )),
            ..base_capture()
        };
        let native = stream_pixels_to_native_point((100.0, 200.0), &capture).expect("native");
        assert_eq!(native.coordinate_space, CoordinateSpace::StreamLogical);
        // The StreamLogical PipeWire branch does not add the rect origin.
        assert!((native.x - 100.0).abs() < 1e-9);
        assert!((native.y - 200.0).abs() < 1e-9);
    }

    #[test]
    fn stream_pixels_to_native_non_pipewire_always_offsets_by_rect_origin() {
        let capture = CaptureInfo {
            backend: CaptureBackendKind::PortalScreenshot,
            pixel_size: Some(pixel_size(1000, 1000)),
            logical_rect: Some(rect(
                CoordinateSpace::StreamLogical,
                5.0,
                7.0,
                1000.0,
                1000.0,
            )),
            ..base_capture()
        };
        let native = stream_pixels_to_native_point((100.0, 200.0), &capture).expect("native");
        assert_eq!(native.coordinate_space, CoordinateSpace::StreamLogical);
        assert!((native.x - 105.0).abs() < 1e-9);
        assert!((native.y - 207.0).abs() < 1e-9);
    }
}
