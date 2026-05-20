use async_trait::async_trait;
use sky_cua_platform::diagnostics::BackendError;
use sky_cua_platform::model::{ActionRequest, DiagnosticEntry, FocusedApp};

use crate::app_policy::ResolvedSetValueFallbackPolicy;
use crate::portal::remote_desktop::MouseButton;
use crate::windowing::LinuxWindowInfo;
use crate::x11::windowing::X11WindowInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticAtspiAction {
    Activate,
    Select,
    Expand,
    Collapse,
    Toggle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticActionInvocation {
    pub action_index: i32,
    pub action_name: Option<String>,
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SemanticSetValueResult {
    Numeric { value: f64 },
    EditableText,
}

#[async_trait]
pub(crate) trait LinuxActionRuntime {
    async fn semantic_grab_focus(&self, backend_ref: &str) -> Result<bool, BackendError>;

    async fn semantic_perform(
        &self,
        backend_ref: &str,
        action: SemanticAtspiAction,
    ) -> Result<bool, BackendError>;

    async fn semantic_invoke_default(&self, backend_ref: &str) -> Result<bool, BackendError>;

    async fn semantic_invoke_secondary(&self, backend_ref: &str) -> Result<bool, BackendError>;

    async fn semantic_available_actions(
        &self,
        backend_ref: &str,
    ) -> Result<Vec<String>, BackendError>;

    async fn semantic_invoke_action_by_index(
        &self,
        backend_ref: &str,
        action_index: i32,
    ) -> Result<SemanticActionInvocation, BackendError>;

    async fn semantic_set_value(
        &self,
        backend_ref: &str,
        value: &str,
    ) -> Result<SemanticSetValueResult, BackendError>;

    async fn semantic_scroll_vertical_at(
        &self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
        app: Option<&FocusedApp>,
    ) -> Result<bool, BackendError>;

    fn resolve_set_value_fallback_policy(
        &self,
        app: Option<&FocusedApp>,
    ) -> Option<ResolvedSetValueFallbackPolicy>;

    async fn focus_window_target_for_keyboard(
        &self,
        request: &ActionRequest,
    ) -> Result<Option<LinuxWindowInfo>, BackendError>;

    fn matched_x11_window_for_request(&self, request: &ActionRequest) -> Option<X11WindowInfo>;

    fn xtest_is_available(&self) -> bool;

    fn activate_x11_window(&self, window: Option<&X11WindowInfo>);

    async fn portal_click_at(
        &self,
        x: f64,
        y: f64,
        button: MouseButton,
    ) -> Result<(), BackendError>;

    async fn portal_drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError>;

    async fn portal_scroll_vertical_at(
        &self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
    ) -> Result<(), BackendError>;

    async fn portal_scroll_vertical_smooth(&self, delta_y: f64) -> Result<(), BackendError>;

    async fn portal_scroll_vertical_discrete(&self, steps: i32) -> Result<(), BackendError>;

    async fn portal_send_text(&self, text: &str) -> Result<(), BackendError>;

    async fn portal_press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError>;

    async fn portal_press_key_sequence_portal_only(
        &self,
        keys: &[String],
    ) -> Result<(), BackendError>;

    async fn portal_take_lifecycle_diagnostics(&self) -> Vec<DiagnosticEntry>;

    async fn portal_reset_session(&self);

    fn xtest_pointer_move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError>;

    fn xtest_pointer_button(&self, button: MouseButton, pressed: bool) -> Result<(), BackendError>;

    fn xtest_click(&self, button: MouseButton) -> Result<(), BackendError>;

    fn xtest_scroll_vertical(
        &self,
        delta_y: Option<f64>,
        steps: Option<i32>,
    ) -> Result<(), BackendError>;

    fn xtest_send_text_to_target(
        &self,
        window_id: Option<&str>,
        text: &str,
    ) -> Result<(), BackendError>;

    fn xtest_press_key_sequence_to_target(
        &self,
        window_id: Option<&str>,
        keys: &[String],
    ) -> Result<(), BackendError>;

    fn virtual_click_at(&self, x: f64, y: f64, button: MouseButton) -> Result<(), BackendError>;

    fn virtual_pointer_mapping_diagnostic(
        &self,
        x: f64,
        y: f64,
    ) -> Result<Option<DiagnosticEntry>, BackendError>;

    fn virtual_drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError>;

    fn virtual_scroll_vertical(&self, steps: i32) -> Result<(), BackendError>;

    fn virtual_scroll_vertical_at(&self, x: f64, y: f64, steps: i32) -> Result<(), BackendError>;

    fn virtual_type_text(&self, text: &str) -> Result<(), BackendError>;

    fn virtual_press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError>;
}
