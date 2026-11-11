//! Unit tests for the AX vocabulary mapping.
use super::*;

fn attrs(role: &str) -> RawAttrs {
    RawAttrs {
        role: Some(role.to_string()),
        enabled: Some(true),
        ..RawAttrs::default()
    }
}

#[test]
fn roles_use_the_shared_vocabulary() {
    assert_eq!(role_name("AXButton", None), "button");
    assert_eq!(role_name("AXButton", Some("AXCloseButton")), "button");
    assert_eq!(role_name("AXCheckBox", Some("AXSwitch")), "checkbox");
    assert_eq!(role_name("AXRadioButton", Some("AXTabButton")), "tab");
    assert_eq!(role_name("AXTabGroup", None), "tab_list");
    assert_eq!(role_name("AXTextField", Some("AXSecureTextField")), "text");
    assert_eq!(role_name("AXStaticText", None), "text");
    assert_eq!(role_name("AXPopUpButton", None), "combo_box");
    assert_eq!(role_name("AXRow", Some("AXOutlineRow")), "tree_item");
    assert_eq!(role_name("AXWebArea", None), "document");
    assert_eq!(role_name("AXToolbar", None), "tool_bar");
    assert_eq!(role_name("AXGrowArea", None), "grow_area");
    assert_eq!(role_name("AXRuler", None), "ruler");
    assert_eq!(role_name("", None), "custom");
}

#[test]
fn pressable_button_advertises_click_and_activate() {
    let mut button = attrs("AXButton");
    button.title = Some("Save".to_string());
    button.actions = vec!["AXPress".to_string(), "AXShowMenu".to_string()];
    button.focus_settable = Some(true);
    let node = draft(&button, None);
    assert_eq!(node.role, "button");
    assert_eq!(node.name.as_deref(), Some("Save"));
    assert_eq!(
        node.semantic_actions,
        vec!["click", "focus", "activate", "showmenu"]
    );
    assert_eq!(node.state_flags, vec!["ax", "enabled", "focusable"]);
    assert!(node.is_informative());
}

#[test]
fn web_defaults_for_expanded_and_selected_carry_no_state() {
    // Chromium reports AXExpanded = false and AXSelected = false on every
    // element.
    let mut group = attrs("AXGroup");
    group.expanded = Some(false);
    group.selected = Some(false);
    group.actions = vec!["AXPress".to_string(), "AXShowMenu".to_string()];
    let node = draft(&group, None);
    assert!(!node.state_flags.iter().any(|flag| flag == "collapsed"));
    assert_eq!(node.semantic_actions, vec!["click", "activate", "showmenu"]);
    // A true value is meaningful anywhere.
    group.expanded = Some(true);
    assert!(
        draft(&group, None)
            .semantic_actions
            .iter()
            .any(|a| a == "collapse")
    );
    // A collapsed pop-up button is collapsible.
    let mut popup = attrs("AXPopUpButton");
    popup.expanded = Some(false);
    assert!(
        draft(&popup, None)
            .semantic_actions
            .iter()
            .any(|a| a == "expand")
    );
    // Rows advertise select only when AXSelected is settable.
    let mut cell = attrs("AXCell");
    cell.selected = Some(false);
    assert!(wants_selected_settable(&cell));
    cell.selected_settable = Some(false);
    assert!(
        !draft(&cell, None)
            .semantic_actions
            .iter()
            .any(|a| a == "select")
    );
}

#[test]
fn checkbox_reports_toggle_state_and_action() {
    let mut checkbox = attrs("AXCheckBox");
    checkbox.title = Some("Wrap".to_string());
    checkbox.value = Some(RawValue::Number(1.0));
    checkbox.actions = vec!["AXPress".to_string()];
    let node = draft(&checkbox, None);
    assert!(node.state_flags.iter().any(|flag| flag == "toggle_on"));
    assert!(
        node.semantic_actions
            .iter()
            .any(|action| action == "toggle")
    );
    assert_eq!(node.value, None);
    assert_eq!(node.numeric_value, None);
}

#[test]
fn disclosure_state_drives_expand_or_collapse() {
    let mut row = attrs("AXRow");
    row.subrole = Some("AXOutlineRow".to_string());
    row.disclosing = Some(false);
    row.selected = Some(false);
    row.selected_settable = Some(true);
    let collapsed = draft(&row, None);
    assert!(collapsed.state_flags.iter().any(|flag| flag == "collapsed"));
    assert!(collapsed.semantic_actions.iter().any(|a| a == "expand"));
    assert!(collapsed.semantic_actions.iter().any(|a| a == "select"));
    row.disclosing = Some(true);
    let expanded = draft(&row, None);
    assert!(expanded.semantic_actions.iter().any(|a| a == "collapse"));

    let mut triangle = attrs("AXDisclosureTriangle");
    triangle.value = Some(RawValue::Number(0.0));
    assert_eq!(expanded_state(&triangle), Some(false));
}

#[test]
fn editable_text_field_reads_back_text_caret_and_selection() {
    let mut field = attrs("AXTextField");
    field.description = Some("Address".to_string());
    field.value = Some(RawValue::Text("hello world".to_string()));
    field.number_of_characters = Some(11);
    field.selected_text_range = Some((6, 5));
    field.value_settable = Some(true);
    field.focus_settable = Some(true);
    let node = draft(&field, None);
    assert_eq!(node.role, "text");
    assert_eq!(node.name.as_deref(), Some("Address"));
    assert_eq!(node.value.as_deref(), Some("hello world"));
    assert!(node.supports_editable_text);
    assert!(node.semantic_actions.iter().any(|a| a == "set_value"));
    let text = node.text.unwrap();
    assert_eq!(text.character_count, 11);
    assert_eq!(text.caret_offset, Some(11));
    assert_eq!(
        text.selections,
        vec![ElementTextSelection {
            start_offset: 6,
            end_offset: 11
        }]
    );
    assert!(!text.truncated);
}

#[test]
fn caret_without_selection_has_no_selection_entries() {
    let mut field = attrs("AXTextArea");
    field.value = Some(RawValue::Text("abc".to_string()));
    field.selected_text_range = Some((2, 0));
    let text = text_readback(&field).unwrap();
    assert_eq!(text.caret_offset, Some(2));
    assert!(text.selections.is_empty());
}

#[test]
fn long_text_is_capped_at_the_shared_limit() {
    let mut area = attrs("AXTextArea");
    let long = "x".repeat(MAX_TEXT_READBACK_CHARS + 10);
    area.value = Some(RawValue::Text(long.clone()));
    area.number_of_characters = Some(long.len() as i64);
    let text = text_readback(&area).unwrap();
    assert!(text.truncated);
    assert_eq!(text.content.unwrap().len(), MAX_TEXT_READBACK_CHARS);
    assert_eq!(text.character_count, long.len() as i32);
}

#[test]
fn secure_fields_suppress_content_and_never_advertise_set_value() {
    let mut secure = attrs("AXTextField");
    secure.subrole = Some("AXSecureTextField".to_string());
    secure.value = Some(RawValue::Text("hunter2".to_string()));
    secure.number_of_characters = Some(7);
    secure.value_settable = Some(true);
    let node = draft(&secure, None);
    assert_eq!(node.value, None);
    assert!(node.state_flags.iter().any(|flag| flag == "password"));
    assert!(!node.semantic_actions.iter().any(|a| a == "set_value"));
    let text = node.text.unwrap();
    assert!(text.content_suppressed);
    assert_eq!(text.content, None);
    assert_eq!(text.character_count, 7);
}

#[test]
fn readonly_text_is_flagged() {
    let mut field = attrs("AXTextField");
    field.value = Some(RawValue::Text("fixed".to_string()));
    field.value_settable = Some(false);
    let node = draft(&field, None);
    assert!(node.state_flags.iter().any(|flag| flag == "readonly"));
    assert!(!node.supports_editable_text);
}

#[test]
fn scroll_bar_reads_back_position_on_the_unit_interval() {
    let mut bar = attrs("AXScrollBar");
    bar.value = Some(RawValue::Number(0.4));
    let node = draft(&bar, None);
    assert_eq!(node.role, "scroll_bar");
    let numeric = node.numeric_value.unwrap();
    assert_eq!(
        (numeric.current, numeric.minimum, numeric.maximum),
        (0.4, 0.0, 1.0)
    );
}

#[test]
fn slider_reads_back_numeric_range() {
    let mut slider = attrs("AXSlider");
    slider.value = Some(RawValue::Number(0.25));
    slider.min_value = Some(0.0);
    slider.max_value = Some(1.0);
    slider.value_settable = Some(true);
    slider.actions = vec!["AXIncrement".to_string(), "AXDecrement".to_string()];
    let node = draft(&slider, None);
    assert_eq!(node.role, "slider");
    let numeric = node.numeric_value.unwrap();
    assert_eq!(
        (numeric.current, numeric.minimum, numeric.maximum),
        (0.25, 0.0, 1.0)
    );
    assert_eq!(node.value.as_deref(), Some("0.25"));
    assert!(!node.supports_editable_text);
    assert_eq!(
        node.semantic_actions,
        vec!["set_value", "increment", "decrement"]
    );
}

#[test]
fn static_text_value_becomes_the_name() {
    let mut label = attrs("AXStaticText");
    label.value = Some(RawValue::Text("Recents".to_string()));
    let node = draft(&label, None);
    assert_eq!(node.name.as_deref(), Some("Recents"));
    assert_eq!(node.value, None);
    assert!(node.is_informative());
}

#[test]
fn unnamed_actionless_group_is_pruned() {
    let mut group = attrs("AXGroup");
    group.position = Some((0.0, 0.0));
    group.size = Some((10.0, 10.0));
    assert!(!draft(&group, None).is_informative());
}

#[test]
fn offscreen_is_geometric_and_empty_frames_have_no_bounds() {
    let mut inside = attrs("AXButton");
    inside.position = Some((110.0, 110.0));
    inside.size = Some((20.0, 20.0));
    let window = Some((100.0, 100.0, 400.0, 300.0));
    assert!(
        !draft(&inside, window)
            .state_flags
            .iter()
            .any(|f| f == "offscreen")
    );
    let mut outside = inside.clone();
    outside.position = Some((900.0, 110.0));
    assert!(
        draft(&outside, window)
            .state_flags
            .iter()
            .any(|f| f == "offscreen")
    );
    let mut empty = inside;
    empty.size = Some((0.0, 20.0));
    assert_eq!(draft(&empty, window).frame, None);
}

#[test]
fn title_and_distinct_description_are_both_kept() {
    let mut button = attrs("AXButton");
    button.title = Some("OK".to_string());
    button.description = Some("Confirm the dialog".to_string());
    button.identifier = Some("okButton".to_string());
    let node = draft(&button, None);
    assert_eq!(
        node.description.as_deref(),
        Some("Confirm the dialog ax_role=AXButton identifier=okButton")
    );
}

#[test]
fn into_node_reports_bounds_in_desktop_points() {
    let mut button = attrs("AXButton");
    button.position = Some((150.0, 70.0));
    button.size = Some((40.0, 20.0));
    let node = draft(&button, None).into_node(3, Some(0), "ax:x".to_string());
    assert_eq!(node.element_index, 3);
    assert_eq!(node.parent_index, Some(0));
    assert_eq!(
        node.bounds,
        Some(coords::desktop_rect(150.0, 70.0, 40.0, 20.0))
    );
    assert_eq!(node.backend_ref.as_deref(), Some("ax:x"));
}
