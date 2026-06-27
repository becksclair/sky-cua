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
