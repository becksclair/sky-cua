mod key_sequence;
pub(crate) mod runtime;
pub(crate) mod targeting;

use std::process::Stdio;
use std::time::Duration;

use runtime::{LinuxActionRuntime, SemanticAtspiAction, SemanticSetValueResult};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, DiagnosticEntry, InputBackendKind, SessionKind,
};
use sky_cua_platform::{SetValueFallbackMode, SetValueRouting};
use targeting::{
    action_point_for_backend, drag_from_point, drag_to_point, effective_keyboard_input_backend,
    effective_keyboard_input_backend_for_target, effective_pointer_input_backend_for_target,
    explicit_point, input_backend_for, point_for_element_for_backend,
    virtual_scroll_steps_from_delta,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use zbus::Proxy;

use crate::app_policy::ResolvedSetValueFallbackPolicy;
use crate::atspi::normalize_action;
use crate::portal::remote_desktop::MouseButton;
use crate::windowing::common::command_exists;
use crate::x11::windowing::X11WindowInfo;
use key_sequence::parse_key_sequence;

pub(crate) const SET_VALUE_PHYSICAL_FALLBACK_MESSAGE: &str =
    "Set the value through a heuristics-backed physical typing fallback.";

#[derive(Debug, Clone, Copy, PartialEq)]
struct LinuxVirtualDispatchPoint {
    x: f64,
    y: f64,
    requested_x: f64,
    requested_y: f64,
    coordinate_scale: Option<f64>,
}

impl LinuxVirtualDispatchPoint {
    fn diagnostics(self) -> Vec<DiagnosticEntry> {
        let Some(scale) = self.coordinate_scale else {
            return Vec::new();
        };
        vec![DiagnosticEntry {
            code: "LinuxVirtualInputCoordinateScale".to_string(),
            message: "Adjusted Linux virtual input coordinates for a scaled Wayland display."
                .to_string(),
            details: Some(format!(
                "requested=({:.1},{:.1}) emitted=({:.1},{:.1}) coordinate_scale={scale:.2}",
                self.requested_x, self.requested_y, self.x, self.y
            )),
        }]
    }
}

pub(crate) struct LinuxActionExecutor<'a, R> {
    runtime: &'a R,
}

impl<'a, R> LinuxActionExecutor<'a, R>
where
    R: LinuxActionRuntime + Sync,
{
    pub(crate) fn new(runtime: &'a R) -> Self {
        Self { runtime }
    }

    pub(crate) async fn execute(
        &self,
        request: ActionRequest,
    ) -> Result<ActionOutcome, BackendError> {
        match request.action {
            ActionName::FocusElement => self.focus_element(request).await,
            ActionName::ActivateElement => self.activate_element(request).await,
            ActionName::SelectElement => self.select_element(request).await,
            ActionName::ExpandElement => self.expand_element(request).await,
            ActionName::CollapseElement => self.collapse_element(request).await,
            ActionName::ToggleElement => self.toggle_element(request).await,
            ActionName::Click => self.click(request).await,
            ActionName::PerformAction => self.perform_action(request).await,
            ActionName::PerformSecondaryAction => self.secondary_click(request).await,
            ActionName::Scroll => self.scroll(request).await,
            ActionName::Drag => self.drag(request).await,
            ActionName::TypeText => self.type_text(request).await,
            ActionName::PressKey => self.press_key(request).await,
            ActionName::SetValue => self.set_value(request).await,
        }
    }

    async fn focus_element(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let backend_ref = semantic_backend_ref(&request, "focus_element")?;
        if self.runtime.semantic_grab_focus(backend_ref).await? {
            return Ok(success("Focused the element semantically through AT-SPI."));
        }
        Err(BackendError::new(
            BackendErrorCode::ActionRequiresPhysicalInput,
            format!("AT-SPI focus was unavailable for element {backend_ref}"),
        ))
    }

    async fn activate_element(
        &self,
        request: ActionRequest,
    ) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "activate_element",
            "Activated the element semantically through AT-SPI.",
            SemanticAtspiAction::Activate,
        )
        .await
    }

    async fn select_element(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "select_element",
            "Selected the element semantically through AT-SPI.",
            SemanticAtspiAction::Select,
        )
        .await
    }

    async fn expand_element(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "expand_element",
            "Expanded the element semantically through AT-SPI.",
            SemanticAtspiAction::Expand,
        )
        .await
    }

    async fn collapse_element(
        &self,
        request: ActionRequest,
    ) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "collapse_element",
            "Collapsed the element semantically through AT-SPI.",
            SemanticAtspiAction::Collapse,
        )
        .await
    }

    async fn toggle_element(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        self.semantic_atspi_action(
            &request,
            "toggle_element",
            "Toggled the element semantically through AT-SPI.",
            SemanticAtspiAction::Toggle,
        )
        .await
    }

    async fn semantic_atspi_action(
        &self,
        request: &ActionRequest,
        tool_name: &str,
        success_message: &str,
        action: SemanticAtspiAction,
    ) -> Result<ActionOutcome, BackendError> {
        let backend_ref = semantic_backend_ref(request, tool_name)?;
        let performed = self.runtime.semantic_perform(backend_ref, action).await?;
        if performed {
            return Ok(success(success_message));
        }
        Err(BackendError::new(
            BackendErrorCode::ActionRequiresPhysicalInput,
            format!("AT-SPI {tool_name} was unavailable for element {backend_ref}"),
        ))
    }

    async fn click(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
            && self
                .runtime
                .semantic_invoke_default(backend_ref)
                .await
                .unwrap_or(false)
        {
            return Ok(success("Invoked the element semantically through AT-SPI."));
        }
        self.execute_pointer_click(
            &request,
            MouseButton::Left,
            "Clicked the target through the RemoteDesktop portal.",
            "Clicked the target through the X11 input fallback.",
            "Clicked the target through the Linux virtual input fallback.",
            "click fallback",
        )
        .await
    }

    async fn perform_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let backend_ref = semantic_backend_ref(&request, "perform_action")?;
        let action_index = self
            .resolve_requested_action_index(backend_ref, &request)
            .await?;
        let invocation = self
            .runtime
            .semantic_invoke_action_by_index(backend_ref, action_index)
            .await?;
        if invocation.ok {
            Ok(success(format!(
                "Invoked AT-SPI action {} ({}).",
                invocation.action_index,
                invocation
                    .action_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("unnamed")
            )))
        } else {
            Err(BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!(
                    "AT-SPI action {} ({}) returned false for element {backend_ref}",
                    invocation.action_index,
                    invocation
                        .action_name
                        .as_deref()
                        .filter(|name| !name.is_empty())
                        .unwrap_or("unnamed")
                ),
            ))
        }
    }

    async fn secondary_click(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
            && self
                .runtime
                .semantic_invoke_secondary(backend_ref)
                .await
                .unwrap_or(false)
        {
            return Ok(success(
                "Performed the secondary action semantically through AT-SPI.",
            ));
        }
        self.execute_pointer_click(
            &request,
            MouseButton::Right,
            "Performed the secondary click through the RemoteDesktop portal.",
            "Performed the secondary click through the X11 input fallback.",
            "Performed the secondary click through the Linux virtual input fallback.",
            "secondary click fallback",
        )
        .await
    }

    async fn execute_pointer_click(
        &self,
        request: &ActionRequest,
        button: MouseButton,
        portal_message: &'static str,
        xtest_message: &'static str,
        virtual_message: &'static str,
        action_name: &'static str,
    ) -> Result<ActionOutcome, BackendError> {
        let input_backend = effective_pointer_input_backend_for_target(request);
        let (x, y) = action_point_for_backend(request, input_backend.clone())?;
        match input_backend {
            InputBackendKind::PortalRemoteDesktop => {
                self.runtime.portal_click_at(x, y, button).await?;
                Ok(success_with_diagnostics(
                    portal_message,
                    self.runtime.portal_take_lifecycle_diagnostics().await,
                ))
            }
            InputBackendKind::XTest => {
                let x11_window = self.runtime.matched_x11_window_for_request(request);
                self.runtime.activate_x11_window(x11_window.as_ref());
                self.runtime.xtest_pointer_move_absolute(x, y)?;
                self.runtime.xtest_click(button)?;
                Ok(success(xtest_message))
            }
            InputBackendKind::LinuxVirtualInput => {
                let dispatch_point = linux_virtual_dispatch_point(request, (x, y));
                let mut diagnostics = dispatch_point.diagnostics();
                if dispatch_point.coordinate_scale.is_none()
                    && let Some(diagnostic) = self
                        .runtime
                        .virtual_pointer_mapping_diagnostic(dispatch_point.x, dispatch_point.y)?
                {
                    diagnostics.push(diagnostic);
                }
                self.runtime
                    .virtual_click_at(dispatch_point.x, dispatch_point.y, button)?;
                Ok(success_with_diagnostics(virtual_message, diagnostics))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error(action_name))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("no physical input backend is available for {action_name}"),
            )),
        }
    }

    async fn scroll(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let input_backend = input_backend_for(&request);
        let target_point = match action_point_for_backend(&request, input_backend.clone()) {
            Ok(point) => Some(point),
            Err(error) if scroll_target_requested(&request) => return Err(error),
            Err(_) => None,
        };
        if let Some((x, y)) = target_point {
            match input_backend {
                InputBackendKind::PortalRemoteDesktop => {}
                InputBackendKind::XTest => self.runtime.xtest_pointer_move_absolute(x, y)?,
                InputBackendKind::LinuxVirtualInput => {}
                InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {}
                InputBackendKind::None => {}
            }
        }

        let delta_y = scroll_delta_y(&request.arguments)?;
        let steps = request
            .arguments
            .get("steps")
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .or_else(|| virtual_scroll_steps_from_delta(delta_y))
            .unwrap_or(-1);

        match input_backend {
            InputBackendKind::PortalRemoteDesktop => {
                if let Some((x, y)) = target_point {
                    self.runtime
                        .portal_scroll_vertical_at(x, y, delta_y, steps)
                        .await?;
                } else if let Some(delta_y) = delta_y {
                    self.runtime.portal_scroll_vertical_smooth(delta_y).await?;
                } else {
                    self.runtime.portal_scroll_vertical_discrete(steps).await?;
                }
                Ok(success_with_diagnostics(
                    "Scrolled through the RemoteDesktop portal.",
                    self.runtime.portal_take_lifecycle_diagnostics().await,
                ))
            }
            InputBackendKind::XTest => {
                self.runtime.xtest_scroll_vertical(delta_y, Some(steps))?;
                Ok(success("Scrolled through the X11 input fallback."))
            }
            InputBackendKind::LinuxVirtualInput => {
                let mut diagnostics = Vec::new();
                if let Some((x, y)) = target_point {
                    let dispatch_point = linux_virtual_dispatch_point(&request, (x, y));
                    if self
                        .runtime
                        .semantic_scroll_vertical_at(
                            x,
                            y,
                            delta_y,
                            steps,
                            request.resolved_focused_app.as_ref(),
                        )
                        .await
                        .unwrap_or(false)
                    {
                        return Ok(success(
                            "Scrolled through the AT-SPI value fallback for Linux virtual input.",
                        ));
                    }
                    diagnostics.extend(dispatch_point.diagnostics());
                    self.runtime.virtual_scroll_vertical_at(
                        dispatch_point.x,
                        dispatch_point.y,
                        steps,
                    )?;
                } else {
                    self.runtime.virtual_scroll_vertical(steps)?;
                }
                Ok(success_with_diagnostics(
                    "Scrolled through the Linux virtual input fallback.",
                    diagnostics,
                ))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("scroll"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for scroll",
            )),
        }
    }

    async fn drag(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let input_backend = effective_pointer_input_backend_for_target(&request);
        let from = drag_from_point(&request, input_backend.clone())?;
        let to = if let Some(element) = request.resolved_target_element.as_ref() {
            point_for_element_for_backend(
                element,
                request.resolved_capture.as_ref(),
                input_backend.clone(),
                request.snapshot_id.is_some(),
            )?
        } else {
            drag_to_point(&request, input_backend.clone())?.ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "drag requires either to_element_index or explicit to_x/to_y coordinates",
                )
            })?
        };

        match input_backend {
            InputBackendKind::PortalRemoteDesktop => {
                self.runtime.portal_drag(from, to).await?;
                Ok(success_with_diagnostics(
                    "Dragged through the RemoteDesktop portal.",
                    self.runtime.portal_take_lifecycle_diagnostics().await,
                ))
            }
            InputBackendKind::XTest => {
                self.runtime.xtest_pointer_move_absolute(from.0, from.1)?;
                self.runtime.xtest_pointer_button(MouseButton::Left, true)?;
                tokio::time::sleep(Duration::from_millis(40)).await;
                self.runtime.xtest_pointer_move_absolute(to.0, to.1)?;
                tokio::time::sleep(Duration::from_millis(40)).await;
                self.runtime
                    .xtest_pointer_button(MouseButton::Left, false)?;
                Ok(success("Dragged through the X11 input fallback."))
            }
            InputBackendKind::LinuxVirtualInput => {
                let dispatch_from = linux_virtual_dispatch_point(&request, from);
                let dispatch_to = linux_virtual_dispatch_point(&request, to);
                let mut diagnostics = dispatch_from.diagnostics();
                diagnostics.extend(dispatch_to.diagnostics());
                self.runtime.virtual_drag(
                    (dispatch_from.x, dispatch_from.y),
                    (dispatch_to.x, dispatch_to.y),
                )?;
                Ok(success_with_diagnostics(
                    "Dragged through the Linux virtual input fallback.",
                    diagnostics,
                ))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("drag"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for drag",
            )),
        }
    }

    async fn type_text(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let text = request
            .arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "type_text requires a text argument",
                )
            })?;
        let (input_backend, x11_window, x11_window_id) =
            self.resolve_keyboard_backend(&request).await?;
        match input_backend {
            InputBackendKind::PortalRemoteDesktop => {
                if should_prefer_kde_clipboard_text_backend(&request) {
                    match run_kde_clipboard_paste_text(self.runtime, &text).await {
                        Ok(message) => {
                            return Ok(success_with_diagnostics(
                                message,
                                self.runtime.portal_take_lifecycle_diagnostics().await,
                            ));
                        }
                        Err(error) => {
                            if error.clear_portal_session {
                                self.runtime.portal_reset_session().await;
                            }
                            if !error.can_fallback_to_portal_keysym {
                                return Err(BackendError::new(
                                    BackendErrorCode::ActionUnsupportedForEnvironment,
                                    error.message,
                                ));
                            }
                        }
                    }
                }
                self.runtime.portal_send_text(&text).await?;
                Ok(success_with_diagnostics(
                    "Typed text through the RemoteDesktop portal.",
                    self.runtime.portal_take_lifecycle_diagnostics().await,
                ))
            }
            InputBackendKind::XTest => {
                self.runtime.activate_x11_window(x11_window.as_ref());
                self.runtime
                    .xtest_send_text_to_target(x11_window_id.as_deref(), &text)?;
                Ok(success("Typed text through the X11 input fallback."))
            }
            InputBackendKind::LinuxVirtualInput => {
                self.runtime.virtual_type_text(&text)?;
                Ok(success(
                    "Typed text through the Linux virtual input fallback.",
                ))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("type_text"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for type_text",
            )),
        }
    }

    async fn press_key(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let keys = parse_key_sequence(&request.arguments).ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                "press_key requires a key string or keys array",
            )
        })?;
        let (input_backend, x11_window, x11_window_id) =
            self.resolve_keyboard_backend(&request).await?;
        match input_backend {
            InputBackendKind::PortalRemoteDesktop => {
                self.runtime.portal_press_key_sequence(&keys).await?;
                Ok(success_with_diagnostics(
                    "Pressed the key sequence through the RemoteDesktop portal.",
                    self.runtime.portal_take_lifecycle_diagnostics().await,
                ))
            }
            InputBackendKind::XTest => {
                self.runtime.activate_x11_window(x11_window.as_ref());
                self.runtime
                    .xtest_press_key_sequence_to_target(x11_window_id.as_deref(), &keys)?;
                Ok(success(
                    "Pressed the key sequence through the X11 input fallback.",
                ))
            }
            InputBackendKind::LinuxVirtualInput => {
                self.runtime.virtual_press_key_sequence(&keys)?;
                Ok(success(
                    "Pressed the key sequence through the Linux virtual input fallback.",
                ))
            }
            InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                Err(windows_input_backend_error("press_key"))
            }
            InputBackendKind::None => Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "no physical input backend is available for press_key",
            )),
        }
    }

    async fn resolve_keyboard_backend(
        &self,
        request: &ActionRequest,
    ) -> Result<(InputBackendKind, Option<X11WindowInfo>, Option<String>), BackendError> {
        if let Some(element) = request.resolved_element.as_ref()
            && let Some(backend_ref) = element.backend_ref.as_deref()
        {
            let _ = self.runtime.semantic_grab_focus(backend_ref).await;
        }
        let target_window = self
            .runtime
            .focus_window_target_for_keyboard(request)
            .await?;
        let x11_window = if target_window.is_none() {
            self.runtime.matched_x11_window_for_request(request)
        } else {
            None
        };
        let x11_window_id = target_window
            .as_ref()
            .filter(|window| window.backend == "x11")
            .map(|window| window.window_id.clone())
            .or_else(|| x11_window.as_ref().map(|window| window.window_id.clone()));
        let input_backend = effective_keyboard_input_backend_for_target(
            request,
            x11_window.is_some(),
            x11_window_id.as_deref(),
            self.runtime.xtest_is_available(),
        );
        Ok((input_backend, x11_window, x11_window_id))
    }

    async fn set_value(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let backend_ref = semantic_backend_ref(&request, "set_value")?;
        let value = request
            .arguments
            .get("value")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    "set_value requires a string value argument",
                )
            })?;

        let policy = self
            .runtime
            .resolve_set_value_fallback_policy(request.resolved_focused_app.as_ref());
        if let Some(policy) = policy
            .as_ref()
            .filter(|policy| policy.routing == SetValueRouting::PreferPhysicalFallback)
        {
            return self
                .set_value_with_fallback_policy(&request, value, policy)
                .await;
        }

        let _ = self.runtime.semantic_grab_focus(backend_ref).await;
        match self.runtime.semantic_set_value(backend_ref, value).await {
            Ok(SemanticSetValueResult::EditableText) => {
                return Ok(success("Set editable text semantically through AT-SPI."));
            }
            Ok(SemanticSetValueResult::Numeric { value }) => {
                return Ok(success(format!(
                    "Set numeric value to {value} semantically through AT-SPI."
                )));
            }
            Err(error) if error.code == BackendErrorCode::ActionRequiresPhysicalInput.as_str() => {
                if let Some(policy) = policy.as_ref() {
                    return self
                        .set_value_with_fallback_policy(&request, value, policy)
                        .await;
                }
            }
            Err(error) => return Err(error),
        }

        Err(BackendError::new(
            BackendErrorCode::ActionRequiresPhysicalInput,
            "semantic set_value failed and no physical fallback is enabled for set_value",
        ))
    }

    async fn resolve_requested_action_index(
        &self,
        backend_ref: &str,
        request: &ActionRequest,
    ) -> Result<i32, BackendError> {
        if let Some(index) = request
            .arguments
            .get("action_index")
            .and_then(serde_json::Value::as_i64)
            .or_else(|| {
                request
                    .arguments
                    .get("action_index")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| value.trim().parse::<i64>().ok())
            })
            .or_else(|| {
                request
                    .arguments
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| value.trim().parse::<i64>().ok())
            })
        {
            return i32::try_from(index).map_err(|error| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("action_index {index} is not a valid AT-SPI action index: {error}"),
                )
            });
        }

        let Some(action_name) = request
            .arguments
            .get("action_name")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                request
                    .arguments
                    .get("action")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(0);
        };

        let actions = self.runtime.semantic_available_actions(backend_ref).await?;
        actions
            .iter()
            .position(|candidate| action_name_matches(candidate, action_name))
            .and_then(|index| i32::try_from(index).ok())
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "element {backend_ref} exposes actions [{}], but none matched requested action_name {action_name:?}",
                        actions.join(", ")
                    ),
                )
            })
    }

    async fn set_value_with_fallback_policy(
        &self,
        request: &ActionRequest,
        value: &str,
        policy: &ResolvedSetValueFallbackPolicy,
    ) -> Result<ActionOutcome, BackendError> {
        match policy.mode {
            SetValueFallbackMode::FocusClickSelectAllType => {
                let x11_window = self.runtime.matched_x11_window_for_request(request);
                let physical_backend = effective_keyboard_input_backend(
                    request,
                    x11_window.is_some(),
                    self.runtime.xtest_is_available(),
                );
                let (x, y) = action_point_for_backend(request, physical_backend.clone())?;
                let select_all = vec!["Ctrl".to_string(), "a".to_string()];
                let mut diagnostics = vec![DiagnosticEntry {
                    code: "HeuristicSetValueFallbackUsed".to_string(),
                    message: match policy.routing {
                        SetValueRouting::PreferSemantic =>
                            "Used a heuristics-backed physical fallback for set_value after semantic editing was unavailable"
                                .to_string(),
                        SetValueRouting::PreferPhysicalFallback =>
                            "Used a heuristics-backed physical set_value path because this app prefers keyboard-driven replacement"
                                .to_string(),
                    },
                    details: Some(format!(
                        "policy_key={} mode=focus_click_select_all_type routing={}",
                        policy.key,
                        match policy.routing {
                            SetValueRouting::PreferSemantic => "prefer_semantic",
                            SetValueRouting::PreferPhysicalFallback => "prefer_physical_fallback",
                        }
                    )),
                }];

                match physical_backend {
                    InputBackendKind::PortalRemoteDesktop => {
                        self.runtime
                            .portal_click_at(x, y, MouseButton::Left)
                            .await?;
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        self.runtime.portal_press_key_sequence(&select_all).await?;
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        if should_prefer_kde_clipboard_text_backend(request) {
                            match run_kde_clipboard_paste_text(self.runtime, value).await {
                                Ok(_) => {}
                                Err(error) => {
                                    if error.clear_portal_session {
                                        self.runtime.portal_reset_session().await;
                                    }
                                    if !error.can_fallback_to_portal_keysym {
                                        return Err(BackendError::new(
                                            BackendErrorCode::ActionUnsupportedForEnvironment,
                                            error.message,
                                        ));
                                    }
                                    self.runtime.portal_send_text(value).await?;
                                }
                            }
                        } else {
                            self.runtime.portal_send_text(value).await?;
                        }
                        diagnostics.extend(self.runtime.portal_take_lifecycle_diagnostics().await);
                        Ok(success_with_diagnostics(
                            SET_VALUE_PHYSICAL_FALLBACK_MESSAGE,
                            diagnostics,
                        ))
                    }
                    InputBackendKind::XTest => {
                        self.runtime.activate_x11_window(x11_window.as_ref());
                        self.runtime.xtest_pointer_move_absolute(x, y)?;
                        self.runtime.xtest_click(MouseButton::Left)?;
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        self.runtime.xtest_press_key_sequence_to_target(
                            x11_window.as_ref().map(|window| window.window_id.as_str()),
                            &select_all,
                        )?;
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        self.runtime.xtest_send_text_to_target(
                            x11_window.as_ref().map(|window| window.window_id.as_str()),
                            value,
                        )?;
                        Ok(success_with_diagnostics(
                            SET_VALUE_PHYSICAL_FALLBACK_MESSAGE,
                            diagnostics,
                        ))
                    }
                    InputBackendKind::LinuxVirtualInput => {
                        let dispatch_point = linux_virtual_dispatch_point(request, (x, y));
                        diagnostics.extend(dispatch_point.diagnostics());
                        self.runtime.virtual_click_at(
                            dispatch_point.x,
                            dispatch_point.y,
                            MouseButton::Left,
                        )?;
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        self.runtime.virtual_press_key_sequence(&select_all)?;
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        self.runtime.virtual_type_text(value)?;
                        Ok(success_with_diagnostics(
                            SET_VALUE_PHYSICAL_FALLBACK_MESSAGE,
                            diagnostics,
                        ))
                    }
                    InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
                        Err(windows_input_backend_error("set_value fallback"))
                    }
                    InputBackendKind::None => Err(BackendError::new(
                        BackendErrorCode::ActionUnsupportedForEnvironment,
                        "heuristics allowed a physical set_value fallback, but no physical input backend is available",
                    )),
                }
            }
        }
    }
}

fn scroll_target_requested(request: &ActionRequest) -> bool {
    explicit_point(&request.arguments).is_some()
        || request.element_index.is_some()
        || request.resolved_element.is_some()
}

fn semantic_backend_ref<'a>(
    request: &'a ActionRequest,
    tool_name: &str,
) -> Result<&'a str, BackendError> {
    if let Some(backend_ref) = direct_backend_ref(&request.arguments) {
        return Ok(backend_ref);
    }
    let element = request.resolved_element.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!(
                "{tool_name} requires element_index, element_identifier, or a semantic selector so the service can resolve a semantic target"
            ),
        )
    })?;
    element.backend_ref.as_deref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("{tool_name} target did not include a backend_ref"),
        )
    })
}

fn direct_backend_ref(arguments: &serde_json::Value) -> Option<&str> {
    arguments
        .get("element_identifier")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn action_name_matches(candidate: &str, requested: &str) -> bool {
    let candidate = normalize_action(candidate);
    let requested = normalize_action(requested);
    if candidate == requested {
        return true;
    }
    canonical_action_aliases(&requested)
        .iter()
        .any(|alias| candidate == *alias)
        || canonical_action_aliases(&candidate)
            .iter()
            .any(|alias| requested == *alias)
}

fn canonical_action_aliases(action: &str) -> &'static [&'static str] {
    match action {
        "activate" => &["press", "click", "open", "jump", "invoke"],
        "select" => &["choose"],
        "expand" => &["open"],
        "collapse" => &["close"],
        "toggle" => &["check", "uncheck"],
        _ => &[],
    }
}

fn windows_input_backend_error(action: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!("Windows input backends are unavailable in the Linux backend for {action}"),
    )
}

fn linux_virtual_dispatch_point(
    request: &ActionRequest,
    point: (f64, f64),
) -> LinuxVirtualDispatchPoint {
    let Some(scale) = linux_virtual_coordinate_scale(request, point) else {
        return LinuxVirtualDispatchPoint {
            x: point.0,
            y: point.1,
            requested_x: point.0,
            requested_y: point.1,
            coordinate_scale: None,
        };
    };
    LinuxVirtualDispatchPoint {
        x: point.0 / scale,
        y: point.1 / scale,
        requested_x: point.0,
        requested_y: point.1,
        coordinate_scale: Some(scale),
    }
}

fn linux_virtual_coordinate_scale(request: &ActionRequest, _point: (f64, f64)) -> Option<f64> {
    let environment = request.environment.as_ref()?;
    if environment.input_backend != InputBackendKind::LinuxVirtualInput {
        return None;
    }
    if environment.session_kind != SessionKind::Wayland
        && environment.xdg_session_type.as_deref() != Some("wayland")
    {
        return None;
    }
    if environment
        .desktop_environment
        .as_deref()
        .is_some_and(|desktop| desktop.to_ascii_lowercase().contains("cosmic"))
    {
        return Some(2.0);
    }
    None
}

fn success(message: impl Into<String>) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: message.into(),
        code: "Ok".to_string(),
        diagnostics: Vec::new(),
        agent_cursor: None,
    }
}

pub(crate) fn success_with_diagnostics(
    message: impl Into<String>,
    diagnostics: Vec<DiagnosticEntry>,
) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: message.into(),
        code: "Ok".to_string(),
        diagnostics,
        agent_cursor: None,
    }
}

fn scroll_delta_y(arguments: &serde_json::Value) -> Result<Option<f64>, BackendError> {
    if let Some(delta_y) = arguments.get("delta_y").and_then(serde_json::Value::as_f64) {
        return Ok(Some(delta_y));
    }

    let pages = arguments
        .get("pages")
        .and_then(serde_json::Value::as_f64)
        .filter(|pages| *pages > 0.0)
        .unwrap_or(1.0);
    let delta_y = match arguments
        .get("direction")
        .and_then(serde_json::Value::as_str)
    {
        Some("up") => 120.0 * pages,
        Some("down") | None => -120.0 * pages,
        Some(direction) => {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!("unsupported vertical scroll direction: {direction}"),
            ));
        }
    };
    Ok(Some(delta_y))
}

const WL_COPY_STARTUP_GRACE_MS: u64 = 50;
const WL_COPY_PASTE_ONCE_TIMEOUT_MS: u64 = 2_000;
const PLAIN_TEXT_CLIPBOARD_MIME_TYPES: &[&str] = &["text/plain", "utf8_string", "string", "text"];

#[derive(Debug)]
struct KdeClipboardPasteError {
    message: String,
    can_fallback_to_portal_keysym: bool,
    clear_portal_session: bool,
}

impl KdeClipboardPasteError {
    fn before_text_input(message: String) -> Self {
        Self {
            message,
            can_fallback_to_portal_keysym: true,
            clear_portal_session: false,
        }
    }

    fn after_portal_input(message: String) -> Self {
        Self {
            message,
            can_fallback_to_portal_keysym: false,
            clear_portal_session: true,
        }
    }
}

fn should_prefer_kde_clipboard_text_backend(request: &ActionRequest) -> bool {
    let Some(environment) = request.environment.as_ref() else {
        return false;
    };
    if environment.input_backend != InputBackendKind::PortalRemoteDesktop {
        return false;
    }
    environment
        .desktop_environment
        .as_deref()
        .is_some_and(|desktop| desktop.to_ascii_lowercase().contains("kde"))
}

async fn run_kde_clipboard_paste_text<R>(
    runtime: &R,
    text: &str,
) -> Result<String, KdeClipboardPasteError>
where
    R: LinuxActionRuntime + Sync,
{
    ensure_clipboard_is_plain_text_only()
        .await
        .map_err(KdeClipboardPasteError::before_text_input)?;
    let previous = kde_clipboard_contents()
        .await
        .map_err(KdeClipboardPasteError::before_text_input)?;
    let mut paste_once = wl_copy_sensitive_paste_once(text)
        .await
        .map_err(KdeClipboardPasteError::before_text_input)?;
    paste_once.wait_until_serving().await;

    let paste_chord = ["Ctrl".to_string(), "v".to_string()];
    let paste_result = runtime
        .portal_press_key_sequence_portal_only(&paste_chord)
        .await
        .map_err(|error| error.message);

    match paste_result.and(paste_once.stop_after_paste().await.map(|_| ())) {
        Ok(()) => match kde_set_clipboard_contents(&previous).await {
            Ok(()) => Ok("Typed text through the KDE clipboard portal fallback.".to_string()),
            Err(restore_error) => Ok(format!(
                "Typed text through the KDE clipboard portal fallback. Warning: previous KDE clipboard contents could not be restored: {restore_error}"
            )),
        },
        Err(error) => match kde_set_clipboard_contents(&previous).await {
            Ok(()) => Err(KdeClipboardPasteError::after_portal_input(error)),
            Err(restore_error) => Err(KdeClipboardPasteError::after_portal_input(format!(
                "{error}; previous KDE clipboard contents could not be restored: {restore_error}"
            ))),
        },
    }
}

async fn ensure_clipboard_is_plain_text_only() -> Result<(), String> {
    let mime_types = wl_paste_mime_types().await?;
    if clipboard_mime_types_are_plain_text_only(&mime_types) {
        return Ok(());
    }
    Err(format!(
        "KDE clipboard paste fallback refused to overwrite non-text clipboard contents: {}",
        mime_types.join(", ")
    ))
}

fn clipboard_mime_types_are_plain_text_only(mime_types: &[String]) -> bool {
    mime_types.iter().all(|mime_type| {
        let normalized = mime_type.trim().to_ascii_lowercase();
        normalized.is_empty()
            || PLAIN_TEXT_CLIPBOARD_MIME_TYPES
                .iter()
                .any(|plain| normalized == *plain || normalized.starts_with(&format!("{plain};")))
    })
}

async fn kde_clipboard_contents() -> Result<String, String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| format!("failed to connect to session bus: {error}"))?;
    let proxy = kde_klipper_proxy(&connection).await?;
    proxy
        .call("getClipboardContents", &())
        .await
        .map_err(|error| format!("failed to read KDE clipboard contents: {error}"))
}

async fn kde_set_clipboard_contents(text: &str) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| format!("failed to connect to session bus: {error}"))?;
    let proxy = kde_klipper_proxy(&connection).await?;
    proxy
        .call("setClipboardContents", &(text))
        .await
        .map_err(|error| format!("failed to set KDE clipboard contents: {error}"))
}

async fn wl_paste_mime_types() -> Result<Vec<String>, String> {
    if !command_exists("wl-paste") {
        return Err(
            "wl-paste is required to safely inspect current clipboard MIME types".to_string(),
        );
    }
    let output = Command::new("wl-paste")
        .args(["--list-types"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| format!("failed to run wl-paste --list-types: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if wl_paste_reports_empty_clipboard(&stderr) || wl_paste_reports_empty_clipboard(&stdout) {
            return Ok(Vec::new());
        }
        return Err(if stderr.is_empty() {
            format!("wl-paste --list-types exited with {}", output.status)
        } else {
            format!(
                "wl-paste --list-types exited with {}: {stderr}",
                output.status
            )
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn wl_paste_reports_empty_clipboard(output: &str) -> bool {
    output.trim().eq_ignore_ascii_case("Nothing is copied")
}

struct WlCopyPasteOnce {
    child: tokio::process::Child,
}

impl WlCopyPasteOnce {
    async fn wait_until_serving(&mut self) {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    async fn stop_after_paste(&mut self) -> Result<(), String> {
        match tokio::time::timeout(
            Duration::from_millis(WL_COPY_PASTE_ONCE_TIMEOUT_MS),
            self.child.wait(),
        )
        .await
        {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(Ok(status)) => Err(format!(
                "wl-copy exited before serving a paste request with {status}"
            )),
            Ok(Err(error)) => Err(format!("failed to wait for wl-copy paste request: {error}")),
            Err(_) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                Err("timed out waiting for wl-copy paste request".to_string())
            }
        }
    }
}

async fn wl_copy_sensitive_paste_once(text: &str) -> Result<WlCopyPasteOnce, String> {
    if !command_exists("wl-copy") {
        return Err("wl-copy is required for KDE clipboard paste fallback".to_string());
    }
    let mut child = Command::new("wl-copy")
        .args([
            "--foreground",
            "--paste-once",
            "--sensitive",
            "--type",
            "text/plain;charset=utf-8",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to run wl-copy: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open wl-copy stdin".to_string())?;
    stdin
        .write_all(text.as_bytes())
        .await
        .map_err(|error| format!("failed to write text to wl-copy stdin: {error}"))?;
    drop(stdin);

    tokio::time::sleep(Duration::from_millis(WL_COPY_STARTUP_GRACE_MS)).await;
    if let Some(status) = child
        .try_wait()
        .map_err(|error| format!("failed to inspect wl-copy status: {error}"))?
    {
        return Err(format!(
            "wl-copy exited before the paste request with {status}"
        ));
    }

    Ok(WlCopyPasteOnce { child })
}

async fn kde_klipper_proxy(connection: &zbus::Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(
        connection,
        "org.kde.klipper",
        "/klipper",
        "org.kde.klipper.klipper",
    )
    .await
    .map_err(|error| format!("failed to create KDE Klipper proxy: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        KdeClipboardPasteError, LinuxActionExecutor, SET_VALUE_PHYSICAL_FALLBACK_MESSAGE,
        action_name_matches, clipboard_mime_types_are_plain_text_only,
        should_prefer_kde_clipboard_text_backend, wl_paste_reports_empty_clipboard,
    };
    use crate::actions::runtime::{
        LinuxActionRuntime, SemanticActionInvocation, SemanticAtspiAction, SemanticSetValueResult,
    };
    use crate::app_policy::ResolvedSetValueFallbackPolicy;
    use crate::portal::remote_desktop::MouseButton;
    use crate::windowing::LinuxWindowInfo;
    use crate::x11::windowing::X11WindowInfo;
    use async_trait::async_trait;
    use serde_json::json;
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
    use sky_cua_platform::model::test_support::wayland_pipewire_environment;
    use sky_cua_platform::model::{
        ActionName, ActionRequest, CaptureBackendKind, CaptureInfo, CaptureScope, CoordinateSpace,
        DiagnosticEntry, DisplayInfo, ElementNode, EnvironmentInfo, FocusedApp, InputBackendKind,
        PixelSize, RectF,
    };
    use sky_cua_platform::{SetValueFallbackMode, SetValueRouting};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRuntime {
        semantic_default: bool,
        xtest_available: bool,
        policy: Option<ResolvedSetValueFallbackPolicy>,
        events: Mutex<Vec<String>>,
        diagnostics: Mutex<Vec<DiagnosticEntry>>,
    }

    impl FakeRuntime {
        fn take_events(&self) -> Vec<String> {
            std::mem::take(&mut *self.events.lock().expect("events lock"))
        }

        fn push_event(&self, event: impl Into<String>) {
            self.events.lock().expect("events lock").push(event.into());
        }
    }

    #[async_trait]
    impl LinuxActionRuntime for FakeRuntime {
        async fn semantic_grab_focus(&self, _backend_ref: &str) -> Result<bool, BackendError> {
            Ok(true)
        }

        async fn semantic_perform(
            &self,
            _backend_ref: &str,
            _action: SemanticAtspiAction,
        ) -> Result<bool, BackendError> {
            Ok(true)
        }

        async fn semantic_invoke_default(&self, _backend_ref: &str) -> Result<bool, BackendError> {
            Ok(self.semantic_default)
        }

        async fn semantic_invoke_secondary(
            &self,
            _backend_ref: &str,
        ) -> Result<bool, BackendError> {
            Ok(false)
        }

        async fn semantic_available_actions(
            &self,
            _backend_ref: &str,
        ) -> Result<Vec<String>, BackendError> {
            Ok(vec!["press".to_string()])
        }

        async fn semantic_invoke_action_by_index(
            &self,
            _backend_ref: &str,
            action_index: i32,
        ) -> Result<SemanticActionInvocation, BackendError> {
            Ok(SemanticActionInvocation {
                action_index,
                action_name: Some("press".to_string()),
                ok: true,
            })
        }

        async fn semantic_set_value(
            &self,
            _backend_ref: &str,
            _value: &str,
        ) -> Result<SemanticSetValueResult, BackendError> {
            Ok(SemanticSetValueResult::EditableText)
        }

        async fn semantic_scroll_vertical_at(
            &self,
            x: f64,
            y: f64,
            delta_y: Option<f64>,
            steps: i32,
            _app: Option<&FocusedApp>,
        ) -> Result<bool, BackendError> {
            self.push_event(format!("semantic_scroll_at:{x},{y}:{delta_y:?}:{steps}"));
            Ok(false)
        }

        fn resolve_set_value_fallback_policy(
            &self,
            _app: Option<&FocusedApp>,
        ) -> Option<ResolvedSetValueFallbackPolicy> {
            self.policy.clone()
        }

        async fn focus_window_target_for_keyboard(
            &self,
            _request: &ActionRequest,
        ) -> Result<Option<LinuxWindowInfo>, BackendError> {
            Ok(None)
        }

        fn matched_x11_window_for_request(
            &self,
            _request: &ActionRequest,
        ) -> Option<X11WindowInfo> {
            None
        }

        fn xtest_is_available(&self) -> bool {
            self.xtest_available
        }

        fn activate_x11_window(&self, _window: Option<&X11WindowInfo>) {}

        async fn portal_click_at(
            &self,
            x: f64,
            y: f64,
            button: MouseButton,
        ) -> Result<(), BackendError> {
            self.push_event(format!("portal_click_at:{x},{y}:{button:?}"));
            Ok(())
        }

        async fn portal_drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
            self.push_event(format!(
                "portal_drag:{},{}:{},{}",
                from.0, from.1, to.0, to.1
            ));
            Ok(())
        }

        async fn portal_scroll_vertical_at(
            &self,
            x: f64,
            y: f64,
            delta_y: Option<f64>,
            steps: i32,
        ) -> Result<(), BackendError> {
            self.push_event(format!("portal_scroll_at:{x},{y}:{delta_y:?}:{steps}"));
            Ok(())
        }

        async fn portal_scroll_vertical_smooth(&self, delta_y: f64) -> Result<(), BackendError> {
            self.push_event(format!("portal_scroll_smooth:{delta_y}"));
            Ok(())
        }

        async fn portal_scroll_vertical_discrete(&self, steps: i32) -> Result<(), BackendError> {
            self.push_event(format!("portal_scroll_discrete:{steps}"));
            Ok(())
        }

        async fn portal_send_text(&self, text: &str) -> Result<(), BackendError> {
            self.push_event(format!("portal_text:{text}"));
            Ok(())
        }

        async fn portal_press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
            self.push_event(format!("portal_key:{}", keys.join("+")));
            Ok(())
        }

        async fn portal_press_key_sequence_portal_only(
            &self,
            keys: &[String],
        ) -> Result<(), BackendError> {
            self.push_event(format!("portal_key_portal_only:{}", keys.join("+")));
            Ok(())
        }

        async fn portal_take_lifecycle_diagnostics(&self) -> Vec<DiagnosticEntry> {
            std::mem::take(&mut *self.diagnostics.lock().expect("diagnostics lock"))
        }

        async fn portal_reset_session(&self) {}

        fn xtest_pointer_move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
            self.push_event(format!("xtest_move:{x},{y}"));
            Ok(())
        }

        fn xtest_pointer_button(
            &self,
            button: MouseButton,
            pressed: bool,
        ) -> Result<(), BackendError> {
            self.push_event(format!("xtest_button:{button:?}:{pressed}"));
            Ok(())
        }

        fn xtest_click(&self, button: MouseButton) -> Result<(), BackendError> {
            self.push_event(format!("xtest_click:{button:?}"));
            Ok(())
        }

        fn xtest_scroll_vertical(
            &self,
            delta_y: Option<f64>,
            steps: Option<i32>,
        ) -> Result<(), BackendError> {
            self.push_event(format!("xtest_scroll:{delta_y:?}:{steps:?}"));
            Ok(())
        }

        fn xtest_send_text_to_target(
            &self,
            _window_id: Option<&str>,
            text: &str,
        ) -> Result<(), BackendError> {
            self.push_event(format!("xtest_text:{text}"));
            Ok(())
        }

        fn xtest_press_key_sequence_to_target(
            &self,
            _window_id: Option<&str>,
            keys: &[String],
        ) -> Result<(), BackendError> {
            self.push_event(format!("xtest_key:{}", keys.join("+")));
            Ok(())
        }

        fn virtual_click_at(
            &self,
            x: f64,
            y: f64,
            button: MouseButton,
        ) -> Result<(), BackendError> {
            self.push_event(format!("virtual_click:{x},{y}:{button:?}"));
            Ok(())
        }

        fn virtual_pointer_mapping_diagnostic(
            &self,
            x: f64,
            y: f64,
        ) -> Result<Option<DiagnosticEntry>, BackendError> {
            Ok(Some(DiagnosticEntry {
                code: "LinuxVirtualInputPointerMapping".to_string(),
                message: "Linux virtual input pointer coordinate mapping.".to_string(),
                details: Some(format!("requested=({x:.1},{y:.1})")),
            }))
        }

        fn virtual_drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
            self.push_event(format!("virtual_drag:{from:?}:{to:?}"));
            Ok(())
        }

        fn virtual_scroll_vertical(&self, steps: i32) -> Result<(), BackendError> {
            self.push_event(format!("virtual_scroll:{steps}"));
            Ok(())
        }

        fn virtual_scroll_vertical_at(
            &self,
            x: f64,
            y: f64,
            steps: i32,
        ) -> Result<(), BackendError> {
            self.push_event(format!("virtual_scroll_at:{x},{y}:{steps}"));
            Ok(())
        }

        fn virtual_type_text(&self, text: &str) -> Result<(), BackendError> {
            self.push_event(format!("virtual_text:{text}"));
            Ok(())
        }

        fn virtual_press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
            self.push_event(format!("virtual_key:{}", keys.join("+")));
            Ok(())
        }
    }

    fn action_request(action: ActionName, arguments: serde_json::Value) -> ActionRequest {
        ActionRequest {
            action,
            snapshot_id: None,
            element_index: None,
            arguments,
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        }
    }

    fn element_with_backend_ref() -> ElementNode {
        ElementNode {
            element_index: 1,
            parent_index: None,
            role: "button".to_string(),
            name: Some("Submit".to_string()),
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: Vec::new(),
            semantic_actions: Vec::new(),
            bounds: Some(RectF {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            backend_ref: Some("atspi://button".to_string()),
        }
    }

    #[tokio::test]
    async fn executor_click_prefers_semantic_default_action() {
        let runtime = FakeRuntime {
            semantic_default: true,
            ..FakeRuntime::default()
        };
        let mut request = action_request(ActionName::Click, json!({"x": 100.0, "y": 120.0}));
        request.resolved_element = Some(element_with_backend_ref());

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("click should succeed");

        assert_eq!(
            outcome.message,
            "Invoked the element semantically through AT-SPI."
        );
        assert!(runtime.take_events().is_empty());
    }

    #[tokio::test]
    async fn executor_click_routes_explicit_coordinates_to_portal() {
        let runtime = FakeRuntime::default();
        let request = action_request(ActionName::Click, json!({"x": 100.0, "y": 120.0}));

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("click should succeed");

        assert_eq!(
            outcome.message,
            "Clicked the target through the RemoteDesktop portal."
        );
        assert_eq!(runtime.take_events(), vec!["portal_click_at:100,120:Left"]);
    }

    #[tokio::test]
    async fn executor_linux_virtual_click_maps_screenshot_coordinates() {
        let runtime = FakeRuntime::default();
        let mut request = action_request(ActionName::Click, json!({"x": 640.0, "y": 360.0}));
        request.snapshot_id = Some("snapshot-1".to_string());
        request.environment = Some(EnvironmentInfo {
            input_backend: InputBackendKind::LinuxVirtualInput,
            ..wayland_pipewire_environment()
        });
        request.resolved_capture = Some(CaptureInfo {
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
                x: 100.0,
                y: 50.0,
                width: 1280.0,
                height: 720.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale: Some(1.0),
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        });

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("click should succeed");

        assert_eq!(
            outcome.message,
            "Clicked the target through the Linux virtual input fallback."
        );
        assert_eq!(runtime.take_events(), vec!["virtual_click:740,410:Left"]);
    }

    #[tokio::test]
    async fn executor_linux_virtual_click_scales_ydotool_coordinates_on_scaled_wayland_display() {
        let runtime = FakeRuntime::default();
        let mut request = action_request(ActionName::Click, json!({"x": 179.0, "y": 306.0}));
        let mut environment = wayland_pipewire_environment();
        environment.desktop_environment = Some("COSMIC".to_string());
        environment.input_backend = InputBackendKind::LinuxVirtualInput;
        environment.displays = vec![DisplayInfo {
            display_id: "cosmic:Virtual-1".to_string(),
            name: Some("Virtual-1".to_string()),
            index: 0,
            primary: true,
            logical_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 960.0,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: Some(PixelSize {
                width: 1600,
                height: 1200,
            }),
            scale_factor: Some(1.25),
            backend: "cosmic".to_string(),
        }];
        request.environment = Some(environment);

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("click should succeed");

        assert_eq!(
            outcome.message,
            "Clicked the target through the Linux virtual input fallback."
        );
        assert_eq!(runtime.take_events(), vec!["virtual_click:89.5,153:Left"]);
        assert!(outcome.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "LinuxVirtualInputCoordinateScale"
                && diagnostic
                    .details
                    .as_deref()
                    .is_some_and(|details| details.contains("coordinate_scale=2.00"))
        }));
    }

    #[tokio::test]
    async fn executor_linux_virtual_click_scales_ydotool_coordinates_on_cosmic_wayland() {
        let runtime = FakeRuntime::default();
        let mut request = action_request(ActionName::Click, json!({"x": 179.0, "y": 240.0}));
        let mut environment = wayland_pipewire_environment();
        environment.desktop_environment = Some("COSMIC".to_string());
        environment.input_backend = InputBackendKind::LinuxVirtualInput;
        environment.displays = vec![DisplayInfo {
            display_id: "cosmic:Virtual-1".to_string(),
            name: Some("Virtual-1".to_string()),
            index: 0,
            primary: true,
            logical_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 800.0,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: Some(PixelSize {
                width: 1280,
                height: 800,
            }),
            scale_factor: Some(1.0),
            backend: "cosmic".to_string(),
        }];
        request.environment = Some(environment);

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("click should succeed");

        assert_eq!(runtime.take_events(), vec!["virtual_click:89.5,120:Left"]);
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "LinuxVirtualInputCoordinateScale")
        );
    }

    #[tokio::test]
    async fn executor_drag_requires_destination() {
        let runtime = FakeRuntime::default();
        let request = action_request(ActionName::Drag, json!({"from_x": 10.0, "from_y": 20.0}));

        let error = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect_err("drag destination should be required");

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert_eq!(
            error.message,
            "drag requires either to_element_index or explicit to_x/to_y coordinates"
        );
    }

    #[tokio::test]
    async fn executor_press_key_requires_key_payload() {
        let runtime = FakeRuntime::default();
        let request = action_request(ActionName::PressKey, json!({}));

        let error = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect_err("key payload should be required");

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert_eq!(
            error.message,
            "press_key requires a key string or keys array"
        );
    }

    #[tokio::test]
    async fn executor_press_key_normalizes_shortcut_letters() {
        let runtime = FakeRuntime::default();
        let request = action_request(ActionName::PressKey, json!({"key": "Ctrl+L"}));

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("press_key should succeed");

        assert_eq!(
            outcome.message,
            "Pressed the key sequence through the RemoteDesktop portal."
        );
        assert_eq!(runtime.take_events(), vec!["portal_key:Ctrl+l"]);
    }

    #[tokio::test]
    async fn executor_secondary_click_uses_portal_physical_fallback() {
        let runtime = FakeRuntime::default();
        let request = action_request(
            ActionName::PerformSecondaryAction,
            json!({"x": 25.0, "y": 35.0}),
        );

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("secondary click should succeed");

        assert_eq!(
            outcome.message,
            "Performed the secondary click through the RemoteDesktop portal."
        );
        assert_eq!(runtime.take_events(), vec!["portal_click_at:25,35:Right"]);
    }

    #[tokio::test]
    async fn executor_scroll_preserves_portal_smooth_delta() {
        let runtime = FakeRuntime::default();
        let request = action_request(ActionName::Scroll, json!({"delta_y": -180.0}));

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("scroll should succeed");

        assert_eq!(
            outcome.message,
            "Scrolled through the RemoteDesktop portal."
        );
        assert_eq!(runtime.take_events(), vec!["portal_scroll_smooth:-180"]);
    }

    #[tokio::test]
    async fn executor_scroll_maps_direction_and_pages_to_portal_delta() {
        let runtime = FakeRuntime::default();
        let request = action_request(ActionName::Scroll, json!({"direction": "up", "pages": 2}));

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("scroll should succeed");

        assert_eq!(
            outcome.message,
            "Scrolled through the RemoteDesktop portal."
        );
        assert_eq!(runtime.take_events(), vec!["portal_scroll_smooth:240"]);
    }

    #[tokio::test]
    async fn executor_scroll_rejects_invalid_snapshot_coordinates_instead_of_global_scroll() {
        let runtime = FakeRuntime::default();
        let mut request = action_request(
            ActionName::Scroll,
            json!({"x": 1316.0, "y": 785.0, "delta_y": -120.0}),
        );
        request.snapshot_id = Some("snapshot-1".to_string());
        request.resolved_capture = Some(CaptureInfo {
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
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        });

        let error = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect_err("scroll should reject invalid targeted coordinates");

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("outside the captured image bounds"));
        assert!(runtime.take_events().is_empty());
    }

    #[tokio::test]
    async fn executor_type_text_uses_portal_keysym_path_outside_kde() {
        let runtime = FakeRuntime::default();
        let mut request = action_request(ActionName::TypeText, json!({"text": "hello"}));
        request.environment = Some(EnvironmentInfo {
            desktop_environment: Some("GNOME".to_string()),
            ..wayland_pipewire_environment()
        });

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("type_text should succeed");

        assert_eq!(
            outcome.message,
            "Typed text through the RemoteDesktop portal."
        );
        assert_eq!(runtime.take_events(), vec!["portal_text:hello"]);
    }

    #[tokio::test]
    async fn executor_set_value_can_prefer_physical_fallback_policy() {
        let runtime = FakeRuntime {
            policy: Some(ResolvedSetValueFallbackPolicy {
                key: "kwrite desktop".to_string(),
                mode: SetValueFallbackMode::FocusClickSelectAllType,
                routing: SetValueRouting::PreferPhysicalFallback,
            }),
            ..FakeRuntime::default()
        };
        let mut request = action_request(ActionName::SetValue, json!({"value": "replacement"}));
        request.resolved_element = Some(element_with_backend_ref());

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("set_value fallback should succeed");

        assert_eq!(outcome.message, SET_VALUE_PHYSICAL_FALLBACK_MESSAGE);
        assert_eq!(
            runtime.take_events(),
            vec![
                "portal_click_at:25,40:Left",
                "portal_key:Ctrl+a",
                "portal_text:replacement"
            ]
        );
        assert_eq!(outcome.diagnostics[0].code, "HeuristicSetValueFallbackUsed");
    }

    #[tokio::test]
    async fn executor_set_value_scales_cosmic_linux_virtual_focus_click() {
        let runtime = FakeRuntime {
            policy: Some(ResolvedSetValueFallbackPolicy {
                key: "cosmic text field".to_string(),
                mode: SetValueFallbackMode::FocusClickSelectAllType,
                routing: SetValueRouting::PreferPhysicalFallback,
            }),
            ..FakeRuntime::default()
        };
        let mut request = action_request(ActionName::SetValue, json!({"value": "replacement"}));
        request.resolved_element = Some(element_with_backend_ref());
        request.environment = Some(EnvironmentInfo {
            desktop_environment: Some("COSMIC".to_string()),
            input_backend: InputBackendKind::LinuxVirtualInput,
            ..wayland_pipewire_environment()
        });

        let outcome = LinuxActionExecutor::new(&runtime)
            .execute(request)
            .await
            .expect("set_value fallback should succeed");

        assert_eq!(outcome.message, SET_VALUE_PHYSICAL_FALLBACK_MESSAGE);
        assert_eq!(
            runtime.take_events(),
            vec![
                "virtual_click:12.5,20:Left",
                "virtual_key:Ctrl+a",
                "virtual_text:replacement"
            ]
        );
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "LinuxVirtualInputCoordinateScale")
        );
    }

    #[test]
    fn kde_clipboard_text_backend_is_kde_portal_only() {
        let request = ActionRequest {
            action: ActionName::TypeText,
            snapshot_id: None,
            element_index: None,
            arguments: json!({"text": "hello"}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: Some(wayland_pipewire_environment()),
        };
        assert!(should_prefer_kde_clipboard_text_backend(&request));

        let non_kde = ActionRequest {
            environment: Some(EnvironmentInfo {
                desktop_environment: Some("GNOME".to_string()),
                ..wayland_pipewire_environment()
            }),
            ..request.clone()
        };
        assert!(!should_prefer_kde_clipboard_text_backend(&non_kde));

        let non_portal = ActionRequest {
            environment: Some(EnvironmentInfo {
                input_backend: InputBackendKind::XTest,
                ..wayland_pipewire_environment()
            }),
            ..request
        };
        assert!(!should_prefer_kde_clipboard_text_backend(&non_portal));
    }

    #[test]
    fn kde_clipboard_error_contract_only_falls_back_before_text_input() {
        let before = KdeClipboardPasteError::before_text_input("missing qdbus".to_string());
        assert!(before.can_fallback_to_portal_keysym);
        assert!(!before.clear_portal_session);
        assert_eq!(before.message, "missing qdbus");

        let after = KdeClipboardPasteError::after_portal_input("paste failed".to_string());
        assert!(!after.can_fallback_to_portal_keysym);
        assert!(after.clear_portal_session);
        assert_eq!(after.message, "paste failed");
    }

    #[test]
    fn kde_clipboard_plain_text_guard_rejects_rich_clipboard_types() {
        assert!(clipboard_mime_types_are_plain_text_only(&[]));
        assert!(clipboard_mime_types_are_plain_text_only(&[
            "text/plain;charset=utf-8".to_string(),
            "UTF8_STRING".to_string(),
        ]));
        assert!(!clipboard_mime_types_are_plain_text_only(&[
            "text/plain".to_string(),
            "text/html".to_string(),
        ]));
        assert!(!clipboard_mime_types_are_plain_text_only(&[
            "image/png".to_string()
        ]));
    }

    #[test]
    fn wl_paste_empty_clipboard_message_is_safe_for_kde_fallback() {
        assert!(wl_paste_reports_empty_clipboard("Nothing is copied"));
        assert!(wl_paste_reports_empty_clipboard(" nothing is copied \n"));
        assert!(!wl_paste_reports_empty_clipboard("compositor unavailable"));
    }

    #[test]
    fn action_name_matching_accepts_advertised_canonical_aliases() {
        assert!(action_name_matches("Press", "activate"));
        assert!(action_name_matches("choose", "select"));
        assert!(action_name_matches("close", "collapse"));
        assert!(!action_name_matches("scroll", "activate"));
    }
}
