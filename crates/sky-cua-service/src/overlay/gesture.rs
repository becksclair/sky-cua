//! Gesture-event construction for the overlay controller.
//!
//! Translates a pointer [`ActionRequest`] into the [`AgentOverlayGestureEvent`]
//! the host animates, reusing the native-point helpers from
//! [`super::cursor_geometry`]. Pure (no `self`), so the controller state machine
//! in the parent module stays focused on lifecycle and IPC.

use sky_cua_platform::model::{
    ActionName, ActionRequest, AgentCursorPoint, AgentOverlayGestureEvent, AgentOverlayGestureKind,
    Point2,
};

use super::cursor_geometry::{
    native_drag_start_point, native_drag_target_point, native_point_for_action,
};

pub(super) fn gesture_from_action_request(
    request: &ActionRequest,
    sequence: u64,
) -> Option<AgentOverlayGestureEvent> {
    use sky_cua_platform::overlay_spec::shared::effects::MAX_GESTURE_POINTS;
    use sky_cua_platform::overlay_spec::shared::timing::{
        MAX_GESTURE_DURATION_MS, MIN_GESTURE_DURATION_MS,
    };

    let (kind, source_points) = match request.action {
        ActionName::Click | ActionName::PerformSecondaryAction => {
            let point = native_point_for_action(request)?;
            (AgentOverlayGestureKind::Tap, vec![point])
        }
        ActionName::Drag => {
            let start = native_drag_start_point(request)?;
            let end = native_drag_target_point(request)?;
            (AgentOverlayGestureKind::Drag, vec![start, end])
        }
        _ => return None,
    };

    let first = source_points.first()?;
    let coordinate_space = first.coordinate_space.clone();
    let mapping_id = first.mapping_id.clone();
    if source_points
        .iter()
        .any(|point| point.coordinate_space != coordinate_space || point.mapping_id != mapping_id)
    {
        return None;
    }
    let points = source_points
        .into_iter()
        .map(point_to_point2)
        .collect::<Vec<_>>();

    if points.len() > MAX_GESTURE_POINTS as usize {
        return None;
    }
    if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }

    let requested_duration_ms = request
        .arguments
        .get("duration_ms")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(match kind {
            AgentOverlayGestureKind::Tap | AgentOverlayGestureKind::NoNo => {
                sky_cua_platform::overlay_spec::shared::timing::CLICK_FEEDBACK_MS
            }
            AgentOverlayGestureKind::Drag | AgentOverlayGestureKind::Swipe => {
                sky_cua_platform::overlay_spec::shared::timing::SWIPE_VISUAL_MIN_MS
            }
        });
    let duration_ms = requested_duration_ms.clamp(MIN_GESTURE_DURATION_MS, MAX_GESTURE_DURATION_MS);
    Some(AgentOverlayGestureEvent {
        event_id: format!("{}-{}", action_name_str(&request.action), sequence),
        sequence,
        kind,
        coordinate_space,
        mapping_id,
        points,
        duration_ms,
        source_action: Some(request.action.clone()),
    })
}

fn action_name_str(action: &ActionName) -> &'static str {
    match action {
        ActionName::FocusElement => "focus",
        ActionName::ActivateElement => "activate",
        ActionName::SelectElement => "select",
        ActionName::ExpandElement => "expand",
        ActionName::CollapseElement => "collapse",
        ActionName::ToggleElement => "toggle",
        ActionName::Click => "click",
        ActionName::PerformAction => "perform",
        ActionName::PerformSecondaryAction => "secondary",
        ActionName::Scroll => "scroll",
        ActionName::Drag => "drag",
        ActionName::TypeText => "type",
        ActionName::PressKey => "press",
        ActionName::SetValue => "set_value",
    }
}

fn point_to_point2(point: AgentCursorPoint) -> Point2 {
    Point2 {
        x: point.x,
        y: point.y,
    }
}
