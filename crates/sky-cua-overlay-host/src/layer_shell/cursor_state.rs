//! Pure `AgentCursorState` helpers: point extraction/preference (native over
//! model), and the compositor pointer-telemetry reconciliation that moves the
//! stored state target (never the drawn pose — the motion driver owns that).

use super::*;

pub(super) fn cursor_point(state: &AgentCursorState) -> Option<(f64, f64)> {
    state
        .native_point
        .as_ref()
        .or(state.model_point.as_ref())
        .and_then(point_to_overlay_coordinates)
}
fn point_to_overlay_coordinates(point: &AgentCursorPoint) -> Option<(f64, f64)> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }
    match point.coordinate_space {
        CoordinateSpace::DesktopLogical
        | CoordinateSpace::StreamLogical
        | CoordinateSpace::StreamPixels => Some((point.x, point.y)),
    }
}
pub(super) fn state_needs_system_pointer_update(
    state: &AgentCursorState,
    position: SystemPointerPosition,
) -> bool {
    let Some(point) = state.native_point.as_ref() else {
        return true;
    };
    if point.coordinate_space != CoordinateSpace::DesktopLogical {
        return true;
    }
    (point.x - position.x).abs() >= 0.5 || (point.y - position.y).abs() >= 0.5
}
pub(super) fn apply_system_pointer_position(
    state: &mut AgentCursorState,
    position: SystemPointerPosition,
) {
    state.native_point = Some(AgentCursorPoint {
        x: position.x,
        y: position.y,
        coordinate_space: CoordinateSpace::DesktopLogical,
        mapping_id: None,
    });
    state.sequence = state.sequence.saturating_add(1);
    state.updated_at_ms = current_epoch_ms();
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{AgentCursorPoint, AgentCursorState, CoordinateSpace};

    use super::{apply_system_pointer_position, cursor_point, state_needs_system_pointer_update};
    use crate::system_cursor::SystemPointerPosition;

    #[test]
    fn cursor_point_prefers_native_coordinates_for_visible_overlay() {
        let state = AgentCursorState {
            visible: true,
            sequence: 1,
            model_point: Some(AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: None,
            }),
            native_point: Some(AgentCursorPoint {
                x: 100.0,
                y: 200.0,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
            }),
            snapshot_id: None,
            source_action: None,
            updated_at_ms: 1,
        };

        assert_eq!(cursor_point(&state), Some((100.0, 200.0)));
    }
    #[test]
    fn system_pointer_update_moves_visible_state_to_desktop_coordinates() {
        let mut state = AgentCursorState {
            visible: true,
            sequence: 7,
            model_point: Some(AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: Some("stream".to_string()),
            }),
            native_point: None,
            snapshot_id: None,
            source_action: None,
            updated_at_ms: 0,
        };
        let position = SystemPointerPosition { x: 300.0, y: 400.0 };

        assert!(state_needs_system_pointer_update(&state, position));
        apply_system_pointer_position(&mut state, position);

        assert_eq!(state.sequence, 8);
        assert_eq!(
            state.native_point,
            Some(AgentCursorPoint {
                x: 300.0,
                y: 400.0,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
            })
        );
        assert_eq!(cursor_point(&state), Some((300.0, 400.0)));
        assert!(!state_needs_system_pointer_update(
            &state,
            SystemPointerPosition {
                x: 300.25,
                y: 400.25
            }
        ));
        assert!(state_needs_system_pointer_update(
            &state,
            SystemPointerPosition { x: 301.0, y: 400.0 }
        ));
    }
}
