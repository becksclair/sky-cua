use super::*;

#[async_trait::async_trait]
impl LinuxActionRuntime for LinuxDesktopBackend {
    async fn semantic_grab_focus(&self, backend_ref: &str) -> Result<bool, BackendError> {
        let connection = self.accessibility_connection().await?;
        atspi_actions::grab_focus(&connection, backend_ref).await
    }

    async fn semantic_perform(
        &self,
        backend_ref: &str,
        action: SemanticAtspiAction,
    ) -> Result<bool, BackendError> {
        let connection = self.accessibility_connection().await?;
        match action {
            SemanticAtspiAction::Activate => {
                atspi_actions::activate(&connection, backend_ref).await
            }
            SemanticAtspiAction::Select => atspi_actions::select(&connection, backend_ref).await,
            SemanticAtspiAction::Expand => atspi_actions::expand(&connection, backend_ref).await,
            SemanticAtspiAction::Collapse => {
                atspi_actions::collapse(&connection, backend_ref).await
            }
            SemanticAtspiAction::Toggle => atspi_actions::toggle(&connection, backend_ref).await,
        }
    }

    async fn semantic_invoke_default(&self, backend_ref: &str) -> Result<bool, BackendError> {
        let connection = self.accessibility_connection().await?;
        atspi_actions::invoke_default_action(&connection, backend_ref).await
    }

    async fn semantic_invoke_secondary(&self, backend_ref: &str) -> Result<bool, BackendError> {
        let connection = self.accessibility_connection().await?;
        atspi_actions::invoke_secondary_action(&connection, backend_ref).await
    }

    async fn semantic_available_actions(
        &self,
        backend_ref: &str,
    ) -> Result<Vec<String>, BackendError> {
        let connection = self.accessibility_connection().await?;
        atspi_actions::available_actions(&connection, backend_ref).await
    }

    async fn semantic_invoke_action_by_index(
        &self,
        backend_ref: &str,
        action_index: i32,
    ) -> Result<SemanticActionInvocation, BackendError> {
        let connection = self.accessibility_connection().await?;
        let result =
            atspi_actions::invoke_action_by_index(&connection, backend_ref, action_index).await?;
        Ok(SemanticActionInvocation {
            action_index: result.action_index,
            action_name: result.action_name,
            ok: result.ok,
        })
    }

    async fn semantic_set_value(
        &self,
        backend_ref: &str,
        value: &str,
    ) -> Result<SemanticSetValueResult, BackendError> {
        let connection = self.accessibility_connection().await?;
        match atspi_actions::set_value(&connection, backend_ref, value).await? {
            atspi_actions::SetValueResult::EditableText => Ok(SemanticSetValueResult::EditableText),
            atspi_actions::SetValueResult::Numeric { value } => {
                Ok(SemanticSetValueResult::Numeric { value })
            }
        }
    }

    async fn semantic_scroll_vertical_at(
        &self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
        app: Option<&FocusedApp>,
    ) -> Result<bool, BackendError> {
        let (connection, apps) = self.discover_accessible_apps().await?;
        let preferred_selector = app.map(selector_from_focused_app);
        let mut selected: Option<(f64, String, f64)> = None;

        for candidate_app in apps.iter().filter(|candidate_app| {
            preferred_selector.as_ref().is_none_or(|selector| {
                selector_match_score(&candidate_app.info, selector).is_some()
            })
        }) {
            let (elements, _) = self
                .at_spi_call_with_timeout(snapshot_for_app(&connection, candidate_app))
                .await?;
            let Some((area, scrollbar)) = vertical_scrollbar_for_point(&elements, x, y) else {
                continue;
            };
            let (Some(backend_ref), Some(target_value)) = (
                scrollbar.backend_ref.as_ref(),
                scroll_target_value(scrollbar, delta_y, steps),
            ) else {
                continue;
            };
            if selected
                .as_ref()
                .is_none_or(|(selected_area, _, _)| area < *selected_area)
            {
                selected = Some((area, backend_ref.clone(), target_value));
            }
        }

        let Some((_, backend_ref, target_value)) = selected else {
            return Ok(false);
        };

        atspi_actions::set_value(&connection, &backend_ref, &target_value.to_string()).await?;
        Ok(true)
    }

    fn resolve_set_value_fallback_policy(
        &self,
        app: Option<&FocusedApp>,
    ) -> Option<ResolvedSetValueFallbackPolicy> {
        self.app_policies.resolve_set_value_fallback(app)
    }

    async fn focus_window_target_for_keyboard(
        &self,
        request: &ActionRequest,
    ) -> Result<Option<linux_windowing::LinuxWindowInfo>, BackendError> {
        self.focus_window_target_for_keyboard(request).await
    }

    fn matched_x11_window_for_request(&self, request: &ActionRequest) -> Option<X11WindowInfo> {
        matched_x11_window_for_request(request)
    }

    fn xtest_is_available(&self) -> bool {
        input_xtest::xtest_is_available()
    }

    fn activate_x11_window(&self, window: Option<&X11WindowInfo>) {
        activate_x11_window(window);
    }

    async fn portal_click_at(
        &self,
        x: f64,
        y: f64,
        button: MouseButton,
    ) -> Result<(), BackendError> {
        self.portal.click_at(x, y, button).await
    }

    async fn portal_drag(
        &self,
        waypoints: &[(f64, f64)],
        step_delay: Duration,
    ) -> Result<(), BackendError> {
        self.portal.drag(waypoints, step_delay).await
    }

    async fn portal_scroll_vertical_at(
        &self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
    ) -> Result<(), BackendError> {
        self.portal.scroll_vertical_at(x, y, delta_y, steps).await
    }

    async fn portal_scroll_vertical_smooth(&self, delta_y: f64) -> Result<(), BackendError> {
        self.portal.scroll_vertical_smooth(delta_y).await
    }

    async fn portal_scroll_vertical_discrete(&self, steps: i32) -> Result<(), BackendError> {
        self.portal.scroll_vertical_discrete(steps).await
    }

    async fn portal_send_text(&self, text: &str) -> Result<(), BackendError> {
        self.portal.send_text(text).await
    }

    async fn portal_press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        self.portal.press_key_sequence(keys).await
    }

    async fn portal_press_key_sequence_portal_only(
        &self,
        keys: &[String],
    ) -> Result<(), BackendError> {
        self.portal.press_key_sequence_portal_only(keys).await
    }

    async fn portal_take_lifecycle_diagnostics(&self) -> Vec<DiagnosticEntry> {
        let mut events = self.portal.take_lifecycle_events().await;
        portal_lifecycle_diagnostics(&mut events)
    }

    async fn portal_reset_session(&self) {
        self.portal.reset_session().await;
    }

    fn xtest_pointer_move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
        input_xtest::pointer_move_absolute(x, y)
    }

    fn xtest_pointer_button(&self, button: MouseButton, pressed: bool) -> Result<(), BackendError> {
        input_xtest::pointer_button(x11_mouse_button(button), pressed)
    }

    fn xtest_click(&self, button: MouseButton) -> Result<(), BackendError> {
        input_xtest::click(x11_mouse_button(button))
    }

    fn xtest_scroll_vertical(
        &self,
        delta_y: Option<f64>,
        steps: Option<i32>,
    ) -> Result<(), BackendError> {
        input_xtest::scroll_vertical(delta_y, steps)
    }

    fn xtest_send_text_to_target(
        &self,
        window_id: Option<&str>,
        text: &str,
    ) -> Result<(), BackendError> {
        input_xtest::send_text_to_target(window_id, text)
    }

    fn xtest_press_key_sequence_to_target(
        &self,
        window_id: Option<&str>,
        keys: &[String],
    ) -> Result<(), BackendError> {
        input_xtest::press_key_sequence_to_target(window_id, keys)
    }

    fn virtual_pointer_prefers_absolute(&self) -> bool {
        self.cached_virtual_input()
            .map(|virtual_input| virtual_input.pointer_via_helper())
            .unwrap_or(false)
    }

    fn virtual_click_at(&self, x: f64, y: f64, button: MouseButton) -> Result<(), BackendError> {
        self.cached_virtual_input()?.click_at(x, y, button)
    }

    fn virtual_pointer_mapping_diagnostic(
        &self,
        x: f64,
        y: f64,
    ) -> Result<Option<DiagnosticEntry>, BackendError> {
        let virtual_input = self.cached_virtual_input()?;
        Ok(Some(DiagnosticEntry {
            code: "LinuxVirtualInputPointerMapping".to_string(),
            message: "Linux virtual input pointer coordinate mapping.".to_string(),
            details: Some(virtual_input.pointer_mapping_details(x, y)),
        }))
    }

    fn virtual_drag(
        &self,
        waypoints: &[(f64, f64)],
        step_delay: Duration,
    ) -> Result<(), BackendError> {
        self.cached_virtual_input()?.drag(waypoints, step_delay)
    }

    fn virtual_scroll_vertical(&self, steps: i32) -> Result<(), BackendError> {
        self.cached_virtual_input()?.scroll_vertical(steps)
    }

    fn virtual_scroll_vertical_at(&self, x: f64, y: f64, steps: i32) -> Result<(), BackendError> {
        self.cached_virtual_input()?.scroll_vertical_at(x, y, steps)
    }

    fn virtual_type_text(&self, text: &str) -> Result<(), BackendError> {
        self.cached_virtual_input()?.type_text(text)
    }

    fn virtual_press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        self.cached_virtual_input()?.press_key_sequence(keys)
    }
}

fn x11_mouse_button(button: MouseButton) -> X11MouseButton {
    match button {
        MouseButton::Left => X11MouseButton::Left,
        MouseButton::Middle => X11MouseButton::Middle,
        MouseButton::Right => X11MouseButton::Right,
    }
}

fn activate_x11_window(window: Option<&X11WindowInfo>) {
    if let Some(window) = window
        && let Err(error) = input_xtest::window_activate(&window.window_id)
    {
        warn!(
            "X11 window activation failed before input fallback; continuing with physical input fallback: {}",
            error.message
        );
    }
}

fn matched_x11_window_for_request(request: &ActionRequest) -> Option<X11WindowInfo> {
    let app = request.resolved_focused_app.as_ref()?;
    if !windowing::x11_window_query_available() {
        return None;
    }

    let windows = windowing::discover_windows().ok()?;
    let app = AppInfo {
        app_id: app.app_id.clone(),
        name: app.name.clone(),
        pid: app.pid,
        executable: None,
        desktop_file_id: app.desktop_file_id.clone(),
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: app.toolkit_guess.clone(),
        window_title: app.window_title.clone(),
        is_focused_candidate: true,
    };
    best_x11_window_match(&windows, &app).cloned()
}

fn portal_lifecycle_diagnostics(
    events: &mut Vec<PortalLifecycleEvent>,
) -> Vec<sky_cua_platform::model::DiagnosticEntry> {
    events
        .drain(..)
        .map(|event| sky_cua_platform::model::DiagnosticEntry {
            code: event.code.to_string(),
            message: event.message,
            details: event.details,
        })
        .collect()
}

fn selector_from_focused_app(app: &FocusedApp) -> AppSelector {
    AppSelector {
        app_id: Some(app.app_id.clone()),
        desktop_file_id: app.desktop_file_id.clone(),
        window_title: app.window_title.clone(),
        name: Some(app.name.clone()),
    }
}

pub(super) fn vertical_scrollbar_for_point(
    elements: &[ElementNode],
    x: f64,
    y: f64,
) -> Option<(f64, &ElementNode)> {
    elements
        .iter()
        .filter(|node| is_vertical_value_scrollbar(node))
        .filter_map(|node| {
            scroll_ancestor_area_containing_point(elements, node, x, y).map(|area| (area, node))
        })
        .min_by(|(left, _), (right, _)| left.total_cmp(right))
}

fn is_vertical_value_scrollbar(node: &ElementNode) -> bool {
    node.role == "scroll bar"
        && node
            .numeric_value
            .as_ref()
            .is_some_and(|value| value.maximum > value.minimum)
        && node.state_flags.iter().any(|state| state == "vertical")
        && node
            .semantic_actions
            .iter()
            .any(|action| action == "set_value")
}

fn scroll_ancestor_area_containing_point(
    elements: &[ElementNode],
    node: &ElementNode,
    x: f64,
    y: f64,
) -> Option<f64> {
    let mut parent_index = node.parent_index;
    while let Some(index) = parent_index {
        let parent = elements.get(index)?;
        if let Some(bounds) = parent.bounds.as_ref()
            && bounds_contains(bounds, x, y)
        {
            return Some(bounds.width * bounds.height);
        }
        parent_index = parent.parent_index;
    }
    node.bounds
        .as_ref()
        .filter(|bounds| bounds_contains(bounds, x, y))
        .map(|bounds| bounds.width * bounds.height)
}

fn bounds_contains(bounds: &RectF, x: f64, y: f64) -> bool {
    bounds.width > 0.0
        && bounds.height > 0.0
        && x >= bounds.x
        && x <= bounds.x + bounds.width
        && y >= bounds.y
        && y <= bounds.y + bounds.height
}

pub(super) fn scroll_target_value(
    node: &ElementNode,
    delta_y: Option<f64>,
    steps: i32,
) -> Option<f64> {
    let value = node.numeric_value.as_ref()?;
    let steps =
        crate::actions::targeting::virtual_scroll_steps_from_delta(delta_y).unwrap_or(steps);
    if steps == 0 {
        return None;
    }
    let increment = if value.minimum_increment > 0.0 {
        value.minimum_increment
    } else {
        ((value.maximum - value.minimum) / 10.0).max(1.0)
    };
    Some((value.current + f64::from(steps) * increment).clamp(value.minimum, value.maximum))
}
