use super::*;
use sky_cua_platform::model::{
    CaptureBackendKind, CaptureInfo, CaptureScope, CoordinateSpace, ElementTextReadback, PixelSize,
    RectF,
};

#[test]
fn maps_stream_pixel_action_coordinates_to_desktop_coordinates() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({ "x": 21.0, "y": 305.0 }),
        Some(capture_with_rect(420.0, 184.0, 1732.0, 1070.0, 1.0)),
        None,
    );

    assert_eq!(desktop_action_point(&request).unwrap(), (441.0, 489.0));
}

#[test]
fn maps_scaled_stream_pixel_action_coordinates_to_desktop_coordinates() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({ "x": 1280.0, "y": 720.0 }),
        Some(capture_with_rect(1920.0, 0.0, 1280.0, 720.0, 2.0)),
        None,
    );

    assert_eq!(desktop_action_point(&request).unwrap(), (2560.0, 360.0));
}

#[test]
fn maps_secondary_negative_origin_stream_pixels_to_desktop_coordinates() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({ "x": 640.0, "y": 360.0 }),
        Some(capture_with_rect(-1280.0, 0.0, 1280.0, 720.0, 1.0)),
        None,
    );

    assert_eq!(desktop_action_point(&request).unwrap(), (-640.0, 360.0));
}

#[test]
fn maps_element_center_from_stream_pixels_to_desktop_coordinates() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({}),
        Some(capture_with_rect(420.0, 184.0, 1732.0, 1070.0, 1.0)),
        Some(element(10.0, 20.0, 30.0, 40.0)),
    );

    assert_eq!(desktop_action_point(&request).unwrap(), (445.0, 224.0));
}

#[test]
fn retina_element_center_maps_to_desktop_points() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({}),
        Some(capture_with_rect(100.0, 50.0, 800.0, 600.0, 2.0)),
        Some(element(200.0, 100.0, 40.0, 20.0)),
    );

    assert_eq!(desktop_action_point(&request).unwrap(), (210.0, 105.0));
}

#[test]
fn desktop_logical_element_center_is_not_remapped_through_the_capture() {
    let mut target = element(500.0, 300.0, 40.0, 20.0);
    target.bounds.as_mut().unwrap().space = CoordinateSpace::DesktopLogical;
    let request = action_request(
        ActionName::Click,
        serde_json::json!({}),
        Some(capture_with_rect(100.0, 50.0, 800.0, 600.0, 2.0)),
        Some(target),
    );

    assert_eq!(desktop_action_point(&request).unwrap(), (520.0, 310.0));
}

#[test]
fn oversized_scrolled_view_uses_the_center_of_its_visible_part() {
    // A text view whose AX frame is the whole 3900pt document, inside a
    // 673x439pt window at (185, 83).
    let mut document = element(185.0, 115.0, 656.0, 3900.0);
    document.bounds.as_mut().unwrap().space = CoordinateSpace::DesktopLogical;
    let request = action_request(
        ActionName::Scroll,
        serde_json::json!({}),
        Some(capture_with_rect(185.0, 83.0, 673.0, 439.0, 2.0)),
        Some(document),
    );

    assert_eq!(desktop_action_point(&request).unwrap(), (513.0, 318.5));
}

#[test]
fn element_outside_the_visible_window_is_rejected() {
    let mut below = element(185.0, 900.0, 100.0, 20.0);
    below.bounds.as_mut().unwrap().space = CoordinateSpace::DesktopLogical;
    let request = action_request(
        ActionName::Click,
        serde_json::json!({}),
        Some(capture_with_rect(185.0, 83.0, 673.0, 439.0, 2.0)),
        Some(below),
    );

    let error = desktop_action_point(&request).unwrap_err();
    assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
}

#[test]
fn zero_area_element_inside_the_window_keeps_its_point() {
    let mut caret = element(300.0, 200.0, 0.0, 14.0);
    caret.bounds.as_mut().unwrap().space = CoordinateSpace::DesktopLogical;
    let request = action_request(
        ActionName::Click,
        serde_json::json!({}),
        Some(capture_with_rect(185.0, 83.0, 673.0, 439.0, 2.0)),
        Some(caret),
    );

    assert_eq!(desktop_action_point(&request).unwrap(), (300.0, 207.0));
}

#[test]
fn missing_coordinates_are_invalid_requests() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({ "x": 1.0 }),
        None,
        None,
    );

    let error = plan_action(&request).unwrap_err();
    assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
}

#[test]
fn maps_drag_from_coordinates_from_stream_pixels_to_desktop_coordinates() {
    let request = action_request(
        ActionName::Drag,
        serde_json::json!({ "from_x": 21.0, "from_y": 305.0 }),
        Some(capture_with_rect(420.0, 184.0, 1732.0, 1070.0, 1.0)),
        None,
    );

    assert_eq!(desktop_drag_from_point(&request).unwrap(), (441.0, 489.0));
}

#[test]
fn drag_plans_from_and_to_points() {
    let request = action_request(
        ActionName::Drag,
        serde_json::json!({ "from_x": 10.0, "from_y": 20.0, "to_x": 50.0, "to_y": 60.0 }),
        Some(capture_with_rect(100.0, 100.0, 400.0, 300.0, 2.0)),
        None,
    );

    assert_eq!(
        plan_action(&request).unwrap(),
        ActionPlan::Physical(PhysicalInput::Drag {
            from: (105.0, 110.0),
            to: (125.0, 130.0),
        })
    );
}

#[test]
fn drag_target_prefers_the_resolved_target_element() {
    let mut request = action_request(
        ActionName::Drag,
        serde_json::json!({ "x": 0.0, "y": 0.0, "to_x": 999.0, "to_y": 999.0 }),
        None,
        None,
    );
    request.resolved_target_element = Some(element(10.0, 10.0, 20.0, 20.0));

    assert_eq!(desktop_target_point(&request).unwrap(), (20.0, 20.0));
}

#[test]
fn drag_without_target_is_invalid() {
    let request = action_request(
        ActionName::Drag,
        serde_json::json!({ "x": 0.0, "y": 0.0 }),
        None,
        None,
    );

    let error = plan_action(&request).unwrap_err();
    assert!(error.message.contains("to_x"));
}

#[test]
fn scroll_delta_uses_direction_and_pages() {
    let request = action_request(
        ActionName::Scroll,
        serde_json::json!({ "direction": "up", "pages": 2 }),
        None,
        None,
    );
    assert_eq!(scroll_deltas(&request).unwrap(), (0, 6));

    let request = action_request(
        ActionName::Scroll,
        serde_json::json!({ "direction": "down" }),
        None,
        None,
    );
    assert_eq!(scroll_deltas(&request).unwrap(), (0, -3));

    let request = action_request(ActionName::Scroll, serde_json::json!({}), None, None);
    assert_eq!(scroll_deltas(&request).unwrap(), (0, -3));
}

#[test]
fn horizontal_scroll_directions_use_delta_x() {
    let left = action_request(
        ActionName::Scroll,
        serde_json::json!({ "direction": "left" }),
        None,
        None,
    );
    let right = action_request(
        ActionName::Scroll,
        serde_json::json!({ "direction": "right", "pages": 3 }),
        None,
        None,
    );

    assert_eq!(scroll_deltas(&left).unwrap(), (3, 0));
    assert_eq!(scroll_deltas(&right).unwrap(), (-9, 0));
}

#[test]
fn explicit_scroll_delta_takes_precedence() {
    let request = action_request(
        ActionName::Scroll,
        serde_json::json!({ "direction": "up", "pages": 2, "delta_y": -360.0 }),
        None,
        None,
    );

    assert_eq!(scroll_deltas(&request).unwrap(), (0, -9));
}

#[test]
fn small_explicit_scroll_delta_moves_at_least_one_line() {
    let request = action_request(
        ActionName::Scroll,
        serde_json::json!({ "delta_y": 10.0, "delta_x": -1.0 }),
        None,
        None,
    );

    assert_eq!(scroll_deltas(&request).unwrap(), (1, 1));
}

#[test]
fn unknown_scroll_direction_is_invalid() {
    let request = action_request(
        ActionName::Scroll,
        serde_json::json!({ "direction": "sideways" }),
        None,
        None,
    );

    assert!(scroll_deltas(&request).is_err());
}

#[test]
fn scroll_point_is_optional() {
    let request = action_request(
        ActionName::Scroll,
        serde_json::json!({ "direction": "down" }),
        None,
        None,
    );

    assert_eq!(
        plan_action(&request).unwrap(),
        ActionPlan::Physical(PhysicalInput::Scroll {
            at: None,
            delta_x: 0,
            delta_y: -3,
        })
    );
}

#[test]
fn click_reads_button_and_count() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({ "x": 5.0, "y": 6.0, "mouse_button": "right", "click_count": 2 }),
        None,
        None,
    );

    assert_eq!(
        plan_action(&request).unwrap(),
        ActionPlan::Physical(PhysicalInput::Click {
            at: (5.0, 6.0),
            button: MouseButton::Right,
            count: 2,
        })
    );
}

#[test]
fn click_defaults_to_one_left_click() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({ "x": 5.0, "y": 6.0, "click_count": 0 }),
        None,
        None,
    );

    assert_eq!(
        plan_action(&request).unwrap(),
        ActionPlan::Physical(PhysicalInput::Click {
            at: (5.0, 6.0),
            button: MouseButton::Left,
            count: 1,
        })
    );
}

#[test]
fn middle_button_is_unsupported() {
    let request = action_request(
        ActionName::Click,
        serde_json::json!({ "x": 5.0, "y": 6.0, "mouse_button": "middle" }),
        None,
        None,
    );

    let error = plan_action(&request).unwrap_err();
    assert_eq!(
        error.code,
        BackendErrorCode::ActionUnsupportedForEnvironment.as_str()
    );
}

#[test]
fn secondary_action_is_a_right_click() {
    let request = action_request(
        ActionName::PerformSecondaryAction,
        serde_json::json!({ "x": 1.0, "y": 2.0 }),
        None,
        None,
    );

    assert_eq!(
        plan_action(&request).unwrap(),
        ActionPlan::Physical(PhysicalInput::Click {
            at: (1.0, 2.0),
            button: MouseButton::Right,
            count: 1,
        })
    );
}

#[test]
fn parses_key_strings_and_arrays() {
    let request = action_request(
        ActionName::PressKey,
        serde_json::json!({ "key": "cmd + shift+z" }),
        None,
        None,
    );
    assert_eq!(
        plan_action(&request).unwrap(),
        ActionPlan::Physical(PhysicalInput::PressKeys(vec![
            "cmd".to_string(),
            "shift".to_string(),
            "z".to_string(),
        ]))
    );

    let request = action_request(
        ActionName::PressKey,
        serde_json::json!({ "keys": ["Ctrl", "A"] }),
        None,
        None,
    );
    assert_eq!(
        parse_keys(&request).unwrap(),
        vec!["Ctrl".to_string(), "A".to_string()]
    );
}

#[test]
fn press_key_requires_keys() {
    let missing = action_request(ActionName::PressKey, serde_json::json!({}), None, None);
    let empty = action_request(
        ActionName::PressKey,
        serde_json::json!({ "keys": [] }),
        None,
        None,
    );

    assert!(parse_keys(&missing).is_err());
    assert!(parse_keys(&empty).is_err());
}

#[test]
fn text_actions_require_their_argument() {
    let type_text = action_request(ActionName::TypeText, serde_json::json!({}), None, None);
    let set_value = action_request(
        ActionName::SetValue,
        serde_json::json!({ "value": "hello" }),
        None,
        None,
    );

    assert!(plan_action(&type_text).is_err());
    assert_eq!(
        plan_action(&set_value).unwrap(),
        ActionPlan::Physical(PhysicalInput::SetValue {
            at: None,
            value: "hello".to_string(),
        })
    );
}

#[test]
fn only_pointer_addressed_input_uses_a_snapshot_point() {
    let at = Some((1.0, 2.0));
    let pointed = [
        PhysicalInput::Move { at: (1.0, 2.0) },
        PhysicalInput::Click {
            at: (1.0, 2.0),
            button: MouseButton::Left,
            count: 1,
        },
        PhysicalInput::Drag {
            from: (1.0, 2.0),
            to: (3.0, 4.0),
        },
        PhysicalInput::Scroll {
            at,
            delta_x: 0,
            delta_y: 1,
        },
        PhysicalInput::SetValue {
            at,
            value: String::new(),
        },
    ];
    let keyboard = [
        PhysicalInput::TypeText("a".to_string()),
        PhysicalInput::PressKeys(vec!["Return".to_string()]),
        PhysicalInput::Scroll {
            at: None,
            delta_x: 0,
            delta_y: 1,
        },
        PhysicalInput::SetValue {
            at: None,
            value: String::new(),
        },
    ];
    assert!(pointed.iter().all(PhysicalInput::uses_snapshot_point));
    assert!(!keyboard.iter().any(PhysicalInput::uses_snapshot_point));
}

#[test]
fn text_too_long_to_type_in_one_request_is_refused_before_any_input() {
    // Every line break is its own key event, so this is 20,000 events.
    let long = "a\n".repeat(10_000);
    for (action, argument) in [
        (ActionName::TypeText, "text"),
        (ActionName::SetValue, "value"),
    ] {
        let error = plan_action(&action_request(
            action,
            serde_json::json!({ argument: long }),
            None,
            None,
        ))
        .unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("smaller parts"), "{}", error.message);
    }
    // 100,000 characters of prose is 5,000 events and fits.
    let prose = "a".repeat(100_000);
    assert!(
        plan_action(&action_request(
            ActionName::TypeText,
            serde_json::json!({ "text": prose }),
            None,
            None,
        ))
        .is_ok()
    );
}

#[test]
fn set_value_fallback_clicks_its_target_element_first() {
    let request = action_request(
        ActionName::SetValue,
        serde_json::json!({ "value": "hello" }),
        Some(capture_with_rect(100.0, 50.0, 800.0, 600.0, 2.0)),
        Some(text_field(200.0, 100.0, 40.0, 20.0)),
    );

    assert_eq!(
        plan_action(&request).unwrap(),
        ActionPlan::Physical(PhysicalInput::SetValue {
            at: Some((210.0, 105.0)),
            value: "hello".to_string(),
        })
    );
}

#[test]
fn set_value_fallback_is_refused_for_an_element_that_is_not_a_text_control() {
    let plan_for = |target: ElementNode| {
        plan_action(&action_request(
            ActionName::SetValue,
            serde_json::json!({ "value": "hello" }),
            Some(capture_with_rect(100.0, 50.0, 800.0, 600.0, 2.0)),
            Some(target),
        ))
    };
    let button = element(200.0, 100.0, 40.0, 20.0);

    let error = plan_for(button.clone()).unwrap_err();
    assert_eq!(error.code, "ActionUnsupportedForEnvironment");
    assert!(error.message.contains("button"), "{}", error.message);

    // A static label and a pop-up button share the text field's and combo
    // box's roles but carry no text readback.
    for role in ["text", "combo_box"] {
        let not_a_field = ElementNode {
            role: role.to_string(),
            ..button.clone()
        };
        let error = plan_for(not_a_field).unwrap_err();
        assert_eq!(error.code, "ActionUnsupportedForEnvironment", "{role}");
    }

    assert!(plan_for(text_field(200.0, 100.0, 40.0, 20.0)).is_ok());
    let read_only = ElementNode {
        state_flags: vec!["readonly".to_string()],
        ..text_field(200.0, 100.0, 40.0, 20.0)
    };
    let error = plan_for(read_only).unwrap_err();
    assert_eq!(error.code, "ActionUnsupportedForEnvironment");
    let editable = ElementNode {
        role: "custom".to_string(),
        supports_editable_text: true,
        ..button
    };
    assert!(plan_for(editable).is_ok());
}

#[test]
fn a_named_target_that_cannot_be_resolved_is_never_dropped() {
    // A text field, so `set_value` reaches the point resolution under test.
    let mut below = text_field(185.0, 900.0, 100.0, 20.0);
    below.bounds.as_mut().unwrap().space = CoordinateSpace::DesktopLogical;
    for action in [ActionName::Scroll, ActionName::SetValue] {
        let out_of_view = action_request(
            action.clone(),
            serde_json::json!({ "value": "hello" }),
            Some(capture_with_rect(185.0, 83.0, 673.0, 439.0, 2.0)),
            Some(below.clone()),
        );
        let half_a_point = action_request(
            action.clone(),
            serde_json::json!({ "value": "hello", "x": 1.0 }),
            None,
            None,
        );

        let no_bounds = action_request(
            action,
            serde_json::json!({ "value": "hello" }),
            Some(capture_with_rect(185.0, 83.0, 673.0, 439.0, 2.0)),
            Some(ElementNode {
                bounds: None,
                ..below.clone()
            }),
        );
        for request in [out_of_view, half_a_point, no_bounds] {
            let error = plan_action(&request).unwrap_err();
            assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        }
    }
}

#[test]
fn extreme_explicit_scroll_deltas_saturate() {
    let request = action_request(
        ActionName::Scroll,
        serde_json::json!({ "delta_x": -1.0e300, "delta_y": 1.0e300 }),
        None,
        None,
    );

    assert_eq!(scroll_deltas(&request).unwrap(), (i32::MAX, i32::MAX));
}

#[test]
fn semantic_primitives_have_no_physical_plan() {
    for action in [
        ActionName::FocusElement,
        ActionName::ActivateElement,
        ActionName::SelectElement,
        ActionName::ExpandElement,
        ActionName::CollapseElement,
        ActionName::ToggleElement,
        ActionName::PerformAction,
    ] {
        let request = action_request(action, serde_json::json!({}), None, None);
        assert_eq!(
            plan_action(&request).unwrap(),
            ActionPlan::RequiresAccessibility
        );
    }
}

fn action_request(
    action: ActionName,
    arguments: serde_json::Value,
    resolved_capture: Option<CaptureInfo>,
    resolved_element: Option<ElementNode>,
) -> ActionRequest {
    ActionRequest {
        action,
        appshot_id: None,
        snapshot_id: Some("snapshot".to_string()),
        element_index: None,
        arguments,
        resolved_element,
        resolved_target_element: None,
        resolved_capture,
        resolved_focused_app: None,
        environment: None,
    }
}

fn element(x: f64, y: f64, width: f64, height: f64) -> ElementNode {
    ElementNode {
        element_index: 0,
        parent_index: None,
        role: "button".to_string(),
        name: Some("Settings".to_string()),
        description: None,
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags: Vec::new(),
        semantic_actions: Vec::new(),
        bounds: Some(RectF {
            x,
            y,
            width,
            height,
            space: CoordinateSpace::StreamPixels,
        }),
        backend_ref: None,
    }
}

/// A text field Accessibility could not classify as settable or read-only:
/// it has text readback but does not report editable text.
fn text_field(x: f64, y: f64, width: f64, height: f64) -> ElementNode {
    ElementNode {
        role: "text".to_string(),
        text: Some(ElementTextReadback {
            character_count: 0,
            caret_offset: None,
            content: Some(String::new()),
            content_suppressed: false,
            truncated: false,
            selections: Vec::new(),
        }),
        ..element(x, y, width, height)
    }
}

fn capture_with_rect(x: f64, y: f64, width: f64, height: f64, scale: f64) -> CaptureInfo {
    CaptureInfo {
        backend: CaptureBackendKind::ScreenCaptureKit,
        image_backend: Some(CaptureBackendKind::ScreenCaptureKit),
        capture_scope: CaptureScope::Window,
        display: None,
        coordinate_space: Some(CoordinateSpace::StreamPixels),
        stream_id: None,
        source_type: None,
        mapping_id: None,
        logical_rect: Some(RectF {
            x,
            y,
            width,
            height,
            space: CoordinateSpace::DesktopLogical,
        }),
        source_logical_rect: None,
        pixel_size: Some(PixelSize {
            width: (width * scale) as u32,
            height: (height * scale) as u32,
        }),
        original_pixel_size: None,
        logical_to_pixel_scale: Some(scale),
        screenshot_path: None,
        original_screenshot_path: None,
        model_image_format: None,
        model_image_quality: None,
        model_image_bytes: None,
        model_image_encode_ms: None,
    }
}
