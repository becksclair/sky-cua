use std::{collections::VecDeque, path::Path};

use sky_cua_overlay_host::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AgentCursorBackendKind, AgentCursorCapabilities,
    AgentCursorPoint, AgentCursorPointerTrackingBackendKind, AgentCursorRendererBackendKind,
    AgentCursorState, AgentCursorSystemCursorBackendKind, AgentOverlayGestureEvent,
    AgentOverlayGestureKind, AgentOverlayHostLifecycleState, AppStateSnapshot, CaptureBackendKind,
    CaptureInfo, CoordinateSpace, DiagnosticEntry, ElementNode, PixelSize, Point2, RectF,
};

const AGENT_CURSOR_ENV: &str = "SKY_CUA_AGENT_CURSOR";
const OVERLAY_HIDE_FOR_CAPTURE_ENV: &str = "SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE";
const SCREENSHOT_CURSOR_ENV: &str = "SKY_CUA_SCREENSHOT_CURSOR";
const OVERLAY_IDLE_CLEANUP_MS: u64 = 15_000;

mod host;
mod synthetic_cursor;

use host::OverlayHostConnection;
use synthetic_cursor::{compose_synthetic_cursor, remove_synthetic_cursor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug)]
pub struct OverlayController {
    state: Option<AgentCursorState>,
    next_sequence: u64,
    agent_cursor_mode: CursorMode,
    hide_for_capture_mode: CursorMode,
    screenshot_cursor_mode: CursorMode,
    host: OverlayHostConnection,
    host_capabilities: Option<AgentCursorCapabilities>,
    host_lifecycle_state: AgentOverlayHostLifecycleState,
    /// Bounded dedupe cache for one-shot gesture event IDs. The host also
    /// deduplicates, but the service must not replay an event if the host
    /// restarts mid-session.
    recent_gesture_ids: VecDeque<String>,
}

impl Default for OverlayController {
    fn default() -> Self {
        Self::new(Path::new(""))
    }
}

impl OverlayController {
    #[must_use]
    pub fn new(service_socket_path: &Path) -> Self {
        let agent_cursor_mode = mode_from_env(AGENT_CURSOR_ENV);
        Self {
            state: None,
            next_sequence: 1,
            agent_cursor_mode,
            hide_for_capture_mode: mode_from_env(OVERLAY_HIDE_FOR_CAPTURE_ENV),
            screenshot_cursor_mode: mode_from_env(SCREENSHOT_CURSOR_ENV),
            host: OverlayHostConnection::from_service_socket(service_socket_path),
            host_capabilities: None,
            host_lifecycle_state: AgentOverlayHostLifecycleState::ProcessUnavailable,
            recent_gesture_ids: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests() -> Self {
        Self {
            state: None,
            next_sequence: 1,
            agent_cursor_mode: CursorMode::Auto,
            hide_for_capture_mode: CursorMode::Auto,
            screenshot_cursor_mode: CursorMode::Auto,
            host: OverlayHostConnection::disabled_for_tests(),
            host_capabilities: None,
            host_lifecycle_state: AgentOverlayHostLifecycleState::ProcessUnavailable,
            recent_gesture_ids: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests_with_failing_host(code: &str) -> Self {
        Self {
            state: None,
            next_sequence: 1,
            agent_cursor_mode: CursorMode::Auto,
            hide_for_capture_mode: CursorMode::Auto,
            screenshot_cursor_mode: CursorMode::Auto,
            host: OverlayHostConnection::failing_for_tests(code),
            host_capabilities: None,
            host_lifecycle_state: AgentOverlayHostLifecycleState::ProcessUnavailable,
            recent_gesture_ids: VecDeque::new(),
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn new_for_tests_with_host(
        host_path: std::path::PathBuf,
        socket_path: std::path::PathBuf,
    ) -> Self {
        Self {
            state: None,
            next_sequence: 1,
            agent_cursor_mode: CursorMode::Auto,
            hide_for_capture_mode: CursorMode::Auto,
            screenshot_cursor_mode: CursorMode::Auto,
            host: OverlayHostConnection::unix_socket_transport_for_tests(host_path, socket_path),
            host_capabilities: None,
            host_lifecycle_state: AgentOverlayHostLifecycleState::ProcessUnavailable,
            recent_gesture_ids: VecDeque::new(),
        }
    }

    #[must_use]
    pub fn capabilities(&self) -> AgentCursorCapabilities {
        if self.agent_cursor_mode == CursorMode::Never {
            return AgentCursorCapabilities {
                backend: AgentCursorBackendKind::None,
                renderer_backend: AgentCursorRendererBackendKind::None,
                visible_overlay: false,
                screenshot_synthetic_cursor: false,
                click_through: false,
                capture_exclusion: false,
                pointer_tracking_backend: AgentCursorPointerTrackingBackendKind::None,
                pointer_tracking_exact: false,
                system_cursor_hide_supported: false,
                system_cursor_hidden: false,
                system_cursor_backend: AgentCursorSystemCursorBackendKind::None,
                needs_user_install: false,
                reason: Some(format!("{AGENT_CURSOR_ENV}=never")),
                ..Default::default()
            };
        }

        self.combined_capabilities()
    }

    #[must_use]
    pub fn state(&self) -> Option<AgentCursorState> {
        self.state.clone()
    }

    pub fn set_state(&mut self, state: AgentCursorState) -> AgentCursorStatus {
        if self.agent_cursor_mode == CursorMode::Never {
            self.state = None;
            return self.status_with_diagnostic(diagnostic(
                "AgentCursorDisabled",
                "Agent cursor state was ignored because agent cursor support is disabled.",
                Some(format!("{AGENT_CURSOR_ENV}=never")),
            ));
        }

        let state = self.normalize_state(state);
        self.state = Some(state);
        self.send_host_message(
            OverlayHostMessageKind::SetCursor,
            self.state.clone(),
            None,
            None,
        )
    }

    pub fn hide(&mut self, reason: Option<String>) -> AgentCursorStatus {
        let previous_state = self.state.clone();
        self.set_local_visibility(false);

        let mut status =
            self.send_host_message(OverlayHostMessageKind::Hide, None, None, reason.clone());
        let host_request_failed = status
            .diagnostics
            .iter()
            .any(|entry| entry.code == "AgentCursorHostRequestFailed");
        if host_request_failed {
            self.state = previous_state;
            status.state = self.state();
        }
        if !host_request_failed
            && let Some(reason) = reason.filter(|value| !value.trim().is_empty())
        {
            status.diagnostics.push(diagnostic(
                "AgentCursorHidden",
                "Agent cursor was hidden.",
                Some(reason),
            ));
        }
        status
    }

    pub fn show(&mut self) -> AgentCursorStatus {
        self.set_local_visibility(true);
        self.send_host_message(OverlayHostMessageKind::Show, self.state.clone(), None, None)
    }

    pub fn status(&mut self) -> AgentCursorStatus {
        self.send_host_message(OverlayHostMessageKind::Capabilities, None, None, None)
    }

    pub fn update_from_action(
        &mut self,
        request: &ActionRequest,
        outcome: &mut ActionOutcome,
    ) -> Vec<DiagnosticEntry> {
        if self.agent_cursor_mode == CursorMode::Never {
            return Vec::new();
        }

        if !outcome.success {
            // Failed dispatch cancels any pending visual feedback for this action.
            if cursor_moving_action(&request.action) {
                self.set_local_visibility(false);
                return self
                    .send_host_message(OverlayHostMessageKind::Hide, None, None, None)
                    .diagnostics;
            }
            return Vec::new();
        }

        if let Some(state) = outcome.agent_cursor.clone() {
            let status = self.set_state(state);
            outcome.agent_cursor = status.state;
            return status.diagnostics;
        }

        let Some(state) = state_from_action_request(request) else {
            if cursor_moving_action(&request.action) {
                self.state = None;
                return self
                    .send_host_message(OverlayHostMessageKind::Hide, None, None, None)
                    .diagnostics;
            }
            return Vec::new();
        };
        let status = self.set_state(state);
        outcome.agent_cursor = status.state;
        let mut diagnostics = status.diagnostics;
        if let Some(gesture) = gesture_from_action_request(request, self.allocate_sequence()) {
            diagnostics.extend(self.send_gesture_event(gesture));
        }
        diagnostics
    }

    /// Begin the visual part of a pointer action before backend input dispatch.
    /// This starts the cursor glide without delaying input dispatch.
    pub fn prepare_action_visual(&mut self, request: &ActionRequest) -> Vec<DiagnosticEntry> {
        if self.agent_cursor_mode == CursorMode::Never {
            return Vec::new();
        }
        if !cursor_moving_action(&request.action) {
            return Vec::new();
        }
        let Some(state) = pre_dispatch_state_from_action_request(request) else {
            return Vec::new();
        };
        self.set_state(state).diagnostics
    }

    pub fn prepare_for_capture(&mut self) -> OverlayCaptureGuard {
        if !self.should_hide_visible_overlay_for_capture() {
            return OverlayCaptureGuard::default();
        }

        self.set_local_visibility(false);
        let sequence = self.allocate_sequence();
        let mut diagnostics = Vec::new();
        let mut barrier_applied = false;
        if self.agent_cursor_mode != CursorMode::Never {
            let message = OverlayHostMessage {
                version: OVERLAY_HOST_PROTOCOL_VERSION,
                kind: OverlayHostMessageKind::Hide,
                state: None,
                gesture: None,
                sequence: Some(sequence),
                reason: None,
            };
            match self.host.send(message) {
                Ok(reply) => {
                    barrier_applied = reply.ok && reply.applied_sequence == Some(sequence);
                    diagnostics.extend(self.apply_host_reply(reply));
                }
                Err(diagnostic) => {
                    if diagnostic.code == "AgentCursorHostUnavailable" {
                        self.host_capabilities = None;
                        self.host_lifecycle_state =
                            AgentOverlayHostLifecycleState::ProcessUnavailable;
                    }
                    diagnostics.push(diagnostic);
                }
            }
        }
        if !barrier_applied {
            diagnostics.push(diagnostic(
                "AgentCursorCaptureBarrierPending",
                "Overlay host did not confirm the capture barrier; capture may include the cursor.",
                Some(format!("sequence={sequence}")),
            ));
        }
        OverlayCaptureGuard {
            restore_visible_overlay: true,
            diagnostics,
        }
    }

    pub fn restore_after_capture(&mut self, guard: OverlayCaptureGuard) -> Vec<DiagnosticEntry> {
        if !guard.restore_visible_overlay {
            return Vec::new();
        }
        self.send_host_message(OverlayHostMessageKind::Show, self.state.clone(), None, None)
            .diagnostics
    }

    pub fn apply_to_snapshot(&mut self, snapshot: &mut AppStateSnapshot) {
        snapshot.diagnostics.extend(self.hide_idle_overlay());
        snapshot.agent_cursor = self.state();
        if !self.should_synthesize_cursor() {
            remove_synthetic_cursor_from_snapshot(snapshot);
            return;
        }

        let Some(state) = self.state.as_ref().filter(|state| state.visible) else {
            remove_synthetic_cursor_from_snapshot(snapshot);
            return;
        };
        let Some(model_point) = state.model_point.as_ref() else {
            remove_synthetic_cursor_from_snapshot(snapshot);
            return;
        };
        let Some(capture) = snapshot.capture.as_ref() else {
            return;
        };

        match compose_synthetic_cursor(capture, model_point) {
            Ok(Some(updated_capture)) => snapshot.capture = Some(updated_capture),
            Ok(None) => {}
            Err(diagnostic) => snapshot.diagnostics.push(diagnostic),
        }
    }

    fn should_synthesize_cursor(&self) -> bool {
        self.agent_cursor_mode != CursorMode::Never
            && matches!(
                self.screenshot_cursor_mode,
                CursorMode::Auto | CursorMode::Always
            )
    }

    /// Hide the whole agent-cursor overlay when it has been idle past the timeout.
    ///
    /// Called lazily from snapshot handling and actively from the daemon's
    /// idle watchdog. The overlay host does not own this lifecycle: normal
    /// cleanup comes from the service when an agent session ends or goes idle,
    /// while the compositor shim only keeps a last-ditch cursor-unhide failsafe.
    pub(crate) fn hide_idle_overlay(&mut self) -> Vec<DiagnosticEntry> {
        let Some(state) = self.state.as_ref().filter(|state| state.visible) else {
            return Vec::new();
        };
        if now_ms().saturating_sub(state.updated_at_ms) < OVERLAY_IDLE_CLEANUP_MS {
            return Vec::new();
        }

        self.hide(Some("agent cursor overlay idle cleanup".to_string()))
            .diagnostics
    }

    fn should_hide_visible_overlay_for_capture(&self) -> bool {
        if self.agent_cursor_mode == CursorMode::Never
            || self.hide_for_capture_mode == CursorMode::Never
            || !self.state.as_ref().is_some_and(|state| state.visible)
        {
            return false;
        }

        self.hide_for_capture_mode == CursorMode::Always
            || self
                .host_capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.visible_overlay)
    }

    fn send_host_message(
        &mut self,
        kind: OverlayHostMessageKind,
        state: Option<AgentCursorState>,
        sequence: Option<u64>,
        reason: Option<String>,
    ) -> AgentCursorStatus {
        let mut diagnostics = Vec::new();
        if self.agent_cursor_mode != CursorMode::Never {
            let message = OverlayHostMessage {
                version: OVERLAY_HOST_PROTOCOL_VERSION,
                kind,
                state,
                gesture: None,
                sequence,
                reason,
            };
            match self.host.send(message) {
                Ok(reply) => {
                    diagnostics.extend(self.apply_host_reply(reply));
                }
                Err(diagnostic) => {
                    if diagnostic.code == "AgentCursorHostUnavailable" {
                        self.host_capabilities = None;
                        self.host_lifecycle_state =
                            AgentOverlayHostLifecycleState::ProcessUnavailable;
                    }
                    diagnostics.push(diagnostic);
                }
            }
        }

        AgentCursorStatus {
            capabilities: self.combined_capabilities(),
            state: self.state(),
            diagnostics,
        }
    }

    fn send_gesture_event(&mut self, gesture: AgentOverlayGestureEvent) -> Vec<DiagnosticEntry> {
        if self.agent_cursor_mode == CursorMode::Never {
            return Vec::new();
        }
        if self
            .recent_gesture_ids
            .iter()
            .any(|event_id| event_id == &gesture.event_id)
        {
            return vec![diagnostic(
                "AgentCursorGestureDuplicate",
                "Ignoring duplicate gesture event.",
                Some(gesture.event_id),
            )];
        }
        if self.recent_gesture_ids.len() >= 128 {
            let _ = self.recent_gesture_ids.pop_front();
        }
        self.recent_gesture_ids.push_back(gesture.event_id.clone());
        let message = OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::AnimateGesture,
            state: None,
            gesture: Some(gesture),
            sequence: None,
            reason: None,
        };
        match self.host.send(message) {
            Ok(reply) => self.apply_host_reply(reply),
            Err(diagnostic) => {
                if diagnostic.code == "AgentCursorHostUnavailable" {
                    self.host_capabilities = None;
                    self.host_lifecycle_state = AgentOverlayHostLifecycleState::ProcessUnavailable;
                }
                vec![diagnostic]
            }
        }
    }

    fn apply_host_reply(&mut self, reply: OverlayHostReply) -> Vec<DiagnosticEntry> {
        let mut diagnostics = reply.diagnostics;
        if reply.version != OVERLAY_HOST_PROTOCOL_VERSION {
            self.host_capabilities = None;
            self.host_lifecycle_state = AgentOverlayHostLifecycleState::ProcessUnavailable;
            diagnostics.push(diagnostic(
                "AgentCursorHostProtocolMismatch",
                "Overlay host replied with an incompatible protocol version.",
                Some(format!(
                    "expected={} got={}",
                    OVERLAY_HOST_PROTOCOL_VERSION, reply.version
                )),
            ));
            return diagnostics;
        }
        if let Some(lifecycle_state) = reply.lifecycle_state {
            self.host_lifecycle_state = lifecycle_state;
        }
        if let Some(capabilities) = reply.capabilities {
            self.host_capabilities = Some(capabilities);
        }
        if let Some(state) = reply.state {
            self.state = Some(state);
        }
        if !reply.ok && diagnostics.is_empty() {
            diagnostics.push(diagnostic(
                "AgentCursorHostRejected",
                "Overlay host rejected the cursor request.",
                None,
            ));
        }
        diagnostics
    }

    fn combined_capabilities(&self) -> AgentCursorCapabilities {
        let screenshot_synthetic_cursor = self.screenshot_cursor_mode != CursorMode::Never;
        let Some(host_capabilities) = self.host_capabilities.as_ref() else {
            return AgentCursorCapabilities {
                backend: if screenshot_synthetic_cursor {
                    AgentCursorBackendKind::ScreenshotSynthetic
                } else {
                    AgentCursorBackendKind::None
                },
                renderer_backend: AgentCursorRendererBackendKind::None,
                visible_overlay: false,
                screenshot_synthetic_cursor,
                click_through: false,
                capture_exclusion: false,
                pointer_tracking_backend: AgentCursorPointerTrackingBackendKind::None,
                pointer_tracking_exact: false,
                system_cursor_hide_supported: false,
                system_cursor_hidden: false,
                system_cursor_backend: AgentCursorSystemCursorBackendKind::None,
                needs_user_install: false,
                reason: Some(self.host.default_reason()),
                ..Default::default()
            };
        };

        let mut capabilities = host_capabilities.clone();
        capabilities.screenshot_synthetic_cursor = screenshot_synthetic_cursor;
        if !capabilities.visible_overlay && screenshot_synthetic_cursor {
            capabilities.backend = AgentCursorBackendKind::ScreenshotSynthetic;
        }
        capabilities
    }

    fn normalize_state(&mut self, mut state: AgentCursorState) -> AgentCursorState {
        state.sequence = self.allocate_sequence();
        state.updated_at_ms = now_ms();
        state
    }

    fn set_local_visibility(&mut self, visible: bool) {
        if let Some(mut state) = self.state.clone() {
            state.visible = visible;
            state.sequence = self.allocate_sequence();
            state.updated_at_ms = now_ms();
            self.state = Some(state);
        }
    }

    fn allocate_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }

    fn status_with_diagnostic(&self, diagnostic: DiagnosticEntry) -> AgentCursorStatus {
        AgentCursorStatus {
            capabilities: self.capabilities(),
            state: self.state(),
            diagnostics: vec![diagnostic],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentCursorStatus {
    pub capabilities: AgentCursorCapabilities,
    pub state: Option<AgentCursorState>,
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Default)]
pub struct OverlayCaptureGuard {
    restore_visible_overlay: bool,
    pub diagnostics: Vec<DiagnosticEntry>,
}

fn mode_from_env(name: &str) -> CursorMode {
    let value = std::env::var(name)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match value.as_str() {
        "always" => CursorMode::Always,
        "never" | "off" | "false" | "0" => CursorMode::Never,
        _ => CursorMode::Auto,
    }
}

fn remove_synthetic_cursor_from_snapshot(snapshot: &mut AppStateSnapshot) {
    if let Some(capture) = snapshot.capture.as_mut() {
        remove_synthetic_cursor(capture);
    }
}

fn state_from_action_request(request: &ActionRequest) -> Option<AgentCursorState> {
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

fn cursor_moving_action(action: &ActionName) -> bool {
    matches!(
        action,
        ActionName::Click | ActionName::PerformSecondaryAction | ActionName::Drag
    )
}

fn pre_dispatch_state_from_action_request(request: &ActionRequest) -> Option<AgentCursorState> {
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

fn gesture_from_action_request(
    request: &ActionRequest,
    sequence: u64,
) -> Option<AgentOverlayGestureEvent> {
    use sky_cua_platform::overlay_spec::shared::effects::MAX_GESTURE_POINTS;
    use sky_cua_platform::overlay_spec::shared::timing::{
        MAX_GESTURE_DURATION_MS, MIN_GESTURE_DURATION_MS,
    };

    let (kind, points) = match request.action {
        ActionName::Click | ActionName::PerformSecondaryAction => {
            let point = native_point_for_action(request)?;
            (AgentOverlayGestureKind::Tap, vec![point_to_point2(point)])
        }
        ActionName::Drag => {
            let start = native_drag_start_point(request)?;
            let end = native_drag_target_point(request)?;
            (
                AgentOverlayGestureKind::Drag,
                vec![point_to_point2(start), point_to_point2(end)],
            )
        }
        _ => return None,
    };

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
        coordinate_space: CoordinateSpace::DesktopLogical,
        mapping_id: None,
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

fn native_drag_start_point(request: &ActionRequest) -> Option<AgentCursorPoint> {
    explicit_native_point(request, "from_x", "from_y")
        .or_else(|| explicit_native_point(request, "x", "y"))
        .or_else(|| element_native_point(request.resolved_element.as_ref(), request))
}

fn native_drag_target_point(request: &ActionRequest) -> Option<AgentCursorPoint> {
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

fn native_point_for_action(request: &ActionRequest) -> Option<AgentCursorPoint> {
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

fn diagnostic(code: &str, message: &str, details: Option<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

fn now_ms() -> u64 {
    chrono::Utc::now()
        .timestamp_millis()
        .try_into()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        OVERLAY_IDLE_CLEANUP_MS, OverlayController, gesture_from_action_request, now_ms,
        state_from_action_request,
    };
    use image::{ImageBuffer, Rgba};
    use sky_cua_overlay_host::{OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostReply};
    use sky_cua_platform::model::{
        ActionName, ActionOutcome, ActionRequest, CaptureBackendKind, CaptureInfo, CaptureScope,
        CoordinateSpace, ElementNode, ModelImageFormat, PixelSize, RectF,
    };
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::process::Command;

    #[test]
    fn set_state_normalizes_sequence_and_timestamps() {
        let mut controller = OverlayController::new_for_tests();
        let status = controller.set_state(synthetic_state(99, 0));

        let state = status.state.expect("cursor state should be stored");
        assert_eq!(state.sequence, 1);
        assert!(state.updated_at_ms > 0);
        assert!(status.capabilities.screenshot_synthetic_cursor);
    }

    #[test]
    fn hide_and_show_toggle_current_state_without_losing_position() {
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(10, 20));

        let hidden = controller.hide(Some("capture".to_string()));
        let hidden_state = hidden.state.expect("hidden state should remain present");
        assert!(!hidden_state.visible);
        assert_eq!(hidden_state.model_point.as_ref().expect("point").x, 10.0);
        assert!(
            hidden
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHidden")
        );

        let shown = controller.show();
        assert!(shown.state.expect("shown state").visible);
    }

    #[cfg(unix)]
    #[test]
    fn host_process_round_trips_cursor_state_over_private_socket() {
        if Command::new("python3").arg("--version").status().is_err() {
            return;
        }

        let dir = unique_temp_dir("host-process");
        let host_path = dir.join("fake-overlay-host.py");
        let socket_path = dir.join("agent-cursor.sock");
        write_fake_overlay_host(&host_path);

        let mut controller =
            OverlayController::new_for_tests_with_host(host_path, socket_path.clone());

        let status = controller.status();
        assert!(status.diagnostics.is_empty());
        assert!(status.capabilities.visible_overlay);
        assert!(status.capabilities.screenshot_synthetic_cursor);

        let set = controller.set_state(synthetic_state(44, 55));
        assert!(set.diagnostics.is_empty());
        assert!(socket_path.exists());
        assert_eq!(set.state.as_ref().expect("state").sequence, 1);
        assert!(set.state.as_ref().expect("state").visible);

        let hidden = controller.hide(Some("capture".to_string()));
        assert!(!hidden.state.as_ref().expect("state").visible);
        assert!(
            hidden
                .diagnostics
                .iter()
                .any(|entry| entry.code == "OverlayCursorHidden")
        );

        let shown = controller.show();
        assert!(shown.state.as_ref().expect("state").visible);

        let guard = controller.prepare_for_capture();
        assert!(guard.restore_visible_overlay);
        assert!(guard.diagnostics.is_empty());
        assert!(
            controller
                .state()
                .expect("service state follows host-hidden capture state")
                .visible
                == false
        );
        let restore_diagnostics = controller.restore_after_capture(guard);
        assert!(restore_diagnostics.is_empty());
        assert!(
            controller
                .state()
                .expect("service state follows restored host state")
                .visible
        );

        drop(controller);
        assert!(!socket_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn host_process_failure_is_diagnostic_not_action_failure() {
        let dir = unique_temp_dir("host-missing");
        let host_path = dir.join("missing-overlay-host");
        let socket_path = dir.join("agent-cursor.sock");
        let mut controller = OverlayController::new_for_tests_with_host(host_path, socket_path);

        let status = controller.set_state(synthetic_state(1, 2));

        assert!(status.state.is_some());
        assert!(
            status
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostUnavailable")
        );
        assert!(status.capabilities.screenshot_synthetic_cursor);
    }

    #[cfg(unix)]
    #[test]
    fn host_process_failure_clears_stale_cached_capabilities() {
        let dir = unique_temp_dir("host-stale-capabilities");
        let host_path = dir.join("missing-overlay-host");
        let socket_path = dir.join("agent-cursor.sock");
        let mut controller = OverlayController::new_for_tests_with_host(host_path, socket_path);
        controller.apply_host_reply(OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok: true,
            capabilities: Some(visible_overlay_capabilities("healthy host")),
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            diagnostics: Vec::new(),
        });
        assert!(controller.capabilities().visible_overlay);

        let status = controller.set_state(synthetic_state(1, 2));

        assert!(
            status
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostUnavailable")
        );
        assert!(!status.capabilities.visible_overlay);
        assert!(status.capabilities.screenshot_synthetic_cursor);
    }

    #[test]
    fn transient_host_request_failure_keeps_visible_overlay_capabilities() {
        let mut controller =
            OverlayController::new_for_tests_with_failing_host("AgentCursorHostRequestFailed");
        controller.apply_host_reply(OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok: true,
            capabilities: Some(visible_overlay_capabilities("healthy host")),
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            diagnostics: Vec::new(),
        });
        assert!(controller.capabilities().visible_overlay);

        let status = controller.set_state(synthetic_state(1, 2));

        assert!(
            status
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostRequestFailed")
        );
        assert!(status.capabilities.visible_overlay);
    }

    #[test]
    fn failed_hide_keeps_local_state_visible_so_capture_hide_retries() {
        let mut controller =
            OverlayController::new_for_tests_with_failing_host("AgentCursorHostRequestFailed");
        controller.apply_host_reply(OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok: true,
            capabilities: Some(visible_overlay_capabilities("healthy host")),
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            diagnostics: Vec::new(),
        });
        controller.set_state(synthetic_state(1, 2));

        let status = controller.hide(Some("test hide".to_string()));

        assert!(
            status
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostRequestFailed")
        );
        assert!(
            !status
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHidden")
        );
        assert!(status.state.expect("state").visible);
        assert!(controller.state().expect("state").visible);
        let guard = controller.prepare_for_capture();
        assert!(guard.restore_visible_overlay);
        assert!(
            guard
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostRequestFailed")
        );
    }

    #[test]
    fn host_protocol_mismatch_is_reported_as_diagnostic() {
        let mut controller = OverlayController::new_for_tests();
        let diagnostics = controller.apply_host_reply(OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION + 1,
            ok: true,
            capabilities: None,
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            diagnostics: Vec::new(),
        });

        assert!(
            diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostProtocolMismatch")
        );
    }

    #[test]
    fn host_protocol_mismatch_does_not_update_cached_capabilities() {
        let mut controller = OverlayController::new_for_tests();
        controller.apply_host_reply(OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION + 1,
            ok: true,
            capabilities: Some(sky_cua_platform::model::AgentCursorCapabilities {
                backend: sky_cua_platform::model::AgentCursorBackendKind::WaylandLayerShell,
                renderer_backend: sky_cua_platform::model::AgentCursorRendererBackendKind::Wgpu,
                visible_overlay: true,
                screenshot_synthetic_cursor: false,
                click_through: true,
                capture_exclusion: true,
                pointer_tracking_backend:
                    sky_cua_platform::model::AgentCursorPointerTrackingBackendKind::KwinEffectSignal,
                pointer_tracking_exact: true,
                system_cursor_hide_supported: true,
                system_cursor_hidden: true,
                system_cursor_backend:
                    sky_cua_platform::model::AgentCursorSystemCursorBackendKind::HyprlandConfig,
                needs_user_install: false,
                reason: Some("mismatched host".to_string()),
                ..Default::default()
            }),
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            diagnostics: Vec::new(),
        });

        let capabilities = controller.capabilities();
        assert!(!capabilities.visible_overlay);
        assert!(capabilities.screenshot_synthetic_cursor);
        assert_eq!(
            capabilities.backend,
            sky_cua_platform::model::AgentCursorBackendKind::ScreenshotSynthetic
        );
    }

    #[test]
    fn host_protocol_mismatch_clears_stale_cached_capabilities() {
        let mut controller = OverlayController::new_for_tests();
        controller.apply_host_reply(OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok: true,
            capabilities: Some(visible_overlay_capabilities("healthy host")),
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            diagnostics: Vec::new(),
        });
        assert!(controller.capabilities().visible_overlay);

        controller.apply_host_reply(OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION + 1,
            ok: true,
            capabilities: None,
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            diagnostics: Vec::new(),
        });

        let capabilities = controller.capabilities();
        assert!(!capabilities.visible_overlay);
        assert_eq!(
            capabilities.backend,
            sky_cua_platform::model::AgentCursorBackendKind::ScreenshotSynthetic
        );
    }

    #[test]
    fn derives_cursor_state_from_explicit_click_coordinates() {
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        let state = state_from_action_request(&request).expect("cursor state");
        let point = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(point.x, 12.0);
        assert_eq!(point.y, 34.0);
        assert_eq!(point.coordinate_space, CoordinateSpace::StreamPixels);
        assert_eq!(native.x, 12.0);
        assert_eq!(native.y, 34.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(state.source_action, Some(ActionName::Click));
    }

    #[test]
    fn derives_native_cursor_from_bounded_capture_for_visible_overlay() {
        let mut request =
            action_request(ActionName::Click, serde_json::json!({"x": 40.0, "y": 50.0}));
        request.resolved_capture = Some(capture_with_rect(RectF {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        }));

        let state = state_from_action_request(&request).expect("cursor state");
        let model = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(model.x, 40.0);
        assert_eq!(model.y, 50.0);
        assert_eq!(native.x, 120.0);
        assert_eq!(native.y, 75.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(native.mapping_id.as_deref(), Some("mapping"));
    }

    #[test]
    fn derives_x11_native_cursor_from_original_capture_pixels() {
        let mut request = action_request(
            ActionName::Click,
            serde_json::json!({"x": 960.0, "y": 540.0}),
        );
        request.resolved_capture = Some(x11_capture_with_original_size());

        let state = state_from_action_request(&request).expect("cursor state");
        let model = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(model.x, 960.0);
        assert_eq!(model.y, 540.0);
        assert_eq!(native.x, 1280.0);
        assert_eq!(native.y, 720.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
    }

    #[test]
    fn snapshotless_explicit_click_sets_native_only_cursor() {
        let mut request =
            action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        request.snapshot_id = None;
        request.resolved_capture = None;

        let state = state_from_action_request(&request).expect("cursor state");

        assert!(state.model_point.is_none());
        let native = state.native_point.expect("native point");
        assert_eq!(native.x, 12.0);
        assert_eq!(native.y, 34.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(native.mapping_id, None);
    }

    #[test]
    fn derives_element_click_center_in_stream_pixels() {
        let mut request = action_request(ActionName::Click, serde_json::json!({}));
        request.resolved_element = Some(element_with_bounds(RectF {
            x: 110.0,
            y: 70.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::DesktopLogical,
        }));
        request.resolved_capture = Some(capture_with_rect(RectF {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        }));

        let state = state_from_action_request(&request).expect("cursor state");
        let point = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(point.x, 40.0);
        assert_eq!(point.y, 50.0);
        assert_eq!(native.x, 120.0);
        assert_eq!(native.y, 75.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
    }

    #[test]
    fn derives_element_native_cursor_through_stream_logical_capture_scale() {
        let mut request = action_request(ActionName::Click, serde_json::json!({}));
        request.resolved_element = Some(element_with_bounds(RectF {
            x: 10.0,
            y: 15.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::StreamLogical,
        }));
        request.resolved_capture = Some(capture_with_rect_and_scale(
            RectF {
                x: 100.0,
                y: 50.0,
                width: 200.0,
                height: 100.0,
                space: CoordinateSpace::DesktopLogical,
            },
            Some(2.0),
        ));

        let state = state_from_action_request(&request).expect("cursor state");
        let model = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(model.x, 40.0);
        assert_eq!(model.y, 40.0);
        assert_eq!(native.x, 120.0);
        assert_eq!(native.y, 70.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(native.mapping_id.as_deref(), Some("mapping"));
    }

    #[test]
    fn derives_drag_cursor_from_target_element() {
        let mut request = action_request(ActionName::Drag, serde_json::json!({}));
        request.resolved_target_element = Some(element_with_bounds(RectF {
            x: 150.0,
            y: 70.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::DesktopLogical,
        }));
        request.resolved_capture = Some(capture_with_rect(RectF {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        }));

        let point = state_from_action_request(&request)
            .expect("cursor state")
            .model_point
            .expect("model point");

        assert_eq!(point.x, 120.0);
        assert_eq!(point.y, 50.0);
    }

    #[test]
    fn drag_gesture_uses_distinct_start_and_target_points() {
        let request = action_request(
            ActionName::Drag,
            serde_json::json!({
                "from_x": 40.0,
                "from_y": 50.0,
                "to_x": 200.0,
                "to_y": 150.0,
                "duration_ms": 5,
            }),
        );

        let gesture = gesture_from_action_request(&request, 7).expect("gesture");

        assert_eq!(
            gesture.kind,
            sky_cua_platform::model::AgentOverlayGestureKind::Drag
        );
        assert_eq!(gesture.sequence, 7);
        assert_eq!(gesture.points.len(), 2);
        assert_eq!(gesture.points[0].x, 40.0);
        assert_eq!(gesture.points[0].y, 50.0);
        assert_eq!(gesture.points[1].x, 200.0);
        assert_eq!(gesture.points[1].y, 150.0);
        assert_eq!(
            gesture.duration_ms,
            sky_cua_platform::overlay_spec::shared::timing::MIN_GESTURE_DURATION_MS
        );
    }

    #[test]
    fn non_pointer_action_does_not_move_cursor() {
        let request = action_request(ActionName::TypeText, serde_json::json!({"text": "hello"}));
        assert!(state_from_action_request(&request).is_none());
    }

    #[test]
    fn prepare_action_visual_sets_state_before_dispatch_without_gesture() {
        let mut controller = OverlayController::new_for_tests();
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));

        let diagnostics = controller.prepare_action_visual(&request);

        assert!(diagnostics.is_empty());
        let state = controller.state().expect("pre-dispatch cursor state");
        assert_eq!(state.sequence, 1);
        assert_eq!(state.native_point.as_ref().expect("native").x, 12.0);
        assert!(controller.recent_gesture_ids.is_empty());
    }

    #[test]
    fn prepare_action_visual_for_drag_starts_at_drag_origin() {
        let mut controller = OverlayController::new_for_tests();
        let request = action_request(
            ActionName::Drag,
            serde_json::json!({
                "from_x": 40.0,
                "from_y": 50.0,
                "to_x": 200.0,
                "to_y": 150.0,
            }),
        );

        let diagnostics = controller.prepare_action_visual(&request);

        assert!(diagnostics.is_empty());
        let state = controller.state().expect("pre-dispatch drag state");
        assert_eq!(state.native_point.as_ref().expect("native").x, 40.0);
        assert_eq!(state.native_point.as_ref().expect("native").y, 50.0);
    }

    #[test]
    fn update_from_action_attaches_derived_cursor_to_successful_outcome() {
        let mut controller = OverlayController::new_for_tests();
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        controller.update_from_action(&request, &mut outcome);

        let state = outcome.agent_cursor.expect("outcome should carry cursor");
        assert_eq!(state.sequence, 1);
        assert_eq!(controller.state().expect("controller state").sequence, 1);
        assert_eq!(controller.recent_gesture_ids.len(), 1);
    }

    #[test]
    fn failed_pointer_action_hides_pending_visual_feedback() {
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(10, 20));
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        let mut outcome = ActionOutcome {
            success: false,
            message: "failed".to_string(),
            code: "Failed".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        let diagnostics = controller.update_from_action(&request, &mut outcome);

        assert!(diagnostics.is_empty());
        assert!(!controller.state().expect("state").visible);
        assert!(controller.recent_gesture_ids.is_empty());
    }

    #[test]
    fn update_from_unmapped_pointer_action_clears_stale_cursor() {
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(10, 20));
        let mut request = action_request(ActionName::Click, serde_json::json!({}));
        request.resolved_capture = None;
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        let diagnostics = controller.update_from_action(&request, &mut outcome);

        assert!(diagnostics.is_empty());
        assert!(outcome.agent_cursor.is_none());
        assert!(controller.state().is_none());
    }

    #[test]
    fn prepare_for_capture_requires_matching_applied_barrier() {
        let mut controller = OverlayController::new_for_tests();
        controller.apply_host_reply(OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok: true,
            capabilities: Some(visible_overlay_capabilities("healthy host")),
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            diagnostics: Vec::new(),
        });
        controller.set_state(synthetic_state(10, 20));

        let guard = controller.prepare_for_capture();

        assert!(guard.restore_visible_overlay);
        assert!(!controller.state().expect("hidden state").visible);
        assert!(
            guard
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorCaptureBarrierPending")
        );
    }

    #[test]
    fn apply_to_snapshot_synthesizes_visible_cursor_into_capture() {
        let dir = unique_temp_dir("snapshot-compose");
        let path = dir.join("capture.png");
        ImageBuffer::from_pixel(32, 32, Rgba([240u8, 240, 240, 255]))
            .save(&path)
            .expect("write source image");
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(16, 16));
        let mut snapshot = snapshot_with_capture(capture_with_path(&path, None));

        controller.apply_to_snapshot(&mut snapshot);

        assert!(snapshot.agent_cursor.is_some());
        assert!(snapshot.diagnostics.is_empty());
        assert!(
            snapshot
                .capture
                .expect("updated capture")
                .screenshot_path
                .expect("updated path")
                .ends_with("capture.agent-cursor.png")
        );
    }

    #[test]
    fn apply_to_snapshot_hides_idle_overlay_before_synthesizing() {
        let dir = unique_temp_dir("snapshot-idle-overlay");
        let path = dir.join("capture.png");
        ImageBuffer::from_pixel(32, 32, Rgba([240u8, 240, 240, 255]))
            .save(&path)
            .expect("write source image");
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(16, 16));
        if let Some(state) = controller.state.as_mut() {
            state.updated_at_ms = now_ms().saturating_sub(OVERLAY_IDLE_CLEANUP_MS + 1);
        }
        let mut snapshot = snapshot_with_capture(capture_with_path(&path, None));

        controller.apply_to_snapshot(&mut snapshot);

        assert!(!snapshot.agent_cursor.expect("cursor state").visible);
        assert_eq!(
            snapshot
                .capture
                .expect("capture")
                .screenshot_path
                .as_deref(),
            Some(path.to_str().expect("utf-8 path"))
        );
    }

    #[test]
    fn apply_to_snapshot_reports_synthetic_cursor_diagnostics() {
        let dir = unique_temp_dir("snapshot-compose-oob");
        let path = dir.join("capture.png");
        ImageBuffer::from_pixel(8, 8, Rgba([240u8, 240, 240, 255]))
            .save(&path)
            .expect("write source image");
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(100, 100));
        let mut snapshot = snapshot_with_capture(capture_with_path(&path, None));

        controller.apply_to_snapshot(&mut snapshot);

        assert!(snapshot.agent_cursor.is_some());
        assert!(
            snapshot
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorSyntheticOutOfBounds")
        );
    }

    #[test]
    fn apply_to_snapshot_removes_reused_synthetic_cursor_when_cursor_hidden() {
        let dir = unique_temp_dir("snapshot-hidden-reused-cursor");
        let raw_path = dir.join("capture.png");
        let synthetic_path = dir.join("capture.agent-cursor.png");
        ImageBuffer::from_pixel(16, 16, Rgba([240u8, 240, 240, 255]))
            .save(&raw_path)
            .expect("write raw image");
        ImageBuffer::from_pixel(16, 16, Rgba([0u8, 0, 0, 255]))
            .save(&synthetic_path)
            .expect("write synthetic image");
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(8, 8));
        controller.hide(Some("test".to_string()));
        let mut snapshot = snapshot_with_capture(capture_with_path(&synthetic_path, None));

        controller.apply_to_snapshot(&mut snapshot);

        assert_eq!(
            snapshot
                .capture
                .expect("capture")
                .screenshot_path
                .as_deref(),
            Some(raw_path.to_str().expect("utf-8 path"))
        );
    }

    fn synthetic_state(x: u64, y: u64) -> sky_cua_platform::model::AgentCursorState {
        sky_cua_platform::model::AgentCursorState {
            visible: true,
            sequence: 0,
            model_point: Some(synthetic_point(x as f64, y as f64)),
            native_point: None,
            snapshot_id: Some("snap".to_string()),
            source_action: Some(ActionName::Click),
            updated_at_ms: 0,
        }
    }

    fn synthetic_point(x: f64, y: f64) -> sky_cua_platform::model::AgentCursorPoint {
        sky_cua_platform::model::AgentCursorPoint {
            x,
            y,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: Some("stream".to_string()),
        }
    }

    fn action_request(action: ActionName, arguments: serde_json::Value) -> ActionRequest {
        ActionRequest {
            action,
            snapshot_id: Some("snap".to_string()),
            element_index: None,
            arguments,
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: Some(capture_with_rect(RectF {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
                space: CoordinateSpace::DesktopLogical,
            })),
            resolved_focused_app: None,
            environment: None,
        }
    }

    fn element_with_bounds(bounds: RectF) -> ElementNode {
        ElementNode {
            element_index: 0,
            parent_index: None,
            role: "button".to_string(),
            name: Some("OK".to_string()),
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: vec!["showing".to_string()],
            semantic_actions: vec!["activate".to_string()],
            bounds: Some(bounds),
            backend_ref: None,
        }
    }

    fn capture_with_rect(logical_rect: RectF) -> CaptureInfo {
        capture_with_rect_and_scale(logical_rect, None)
    }

    fn capture_with_path(path: &std::path::Path, format: Option<ModelImageFormat>) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: Some("mapping".to_string()),
            source_logical_rect: None,
            logical_rect: None,
            pixel_size: Some(PixelSize {
                width: 31,
                height: 31,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: Some(path.display().to_string()),
            original_screenshot_path: None,
            model_image_format: format,
            model_image_quality: Some(85),
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }

    fn capture_with_rect_and_scale(
        logical_rect: RectF,
        logical_to_pixel_scale: Option<f64>,
    ) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("stream".to_string()),
            source_type: Some(1),
            mapping_id: Some("mapping".to_string()),
            source_logical_rect: None,
            logical_rect: Some(logical_rect),
            pixel_size: Some(PixelSize {
                width: 400,
                height: 200,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale,
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }

    fn x11_capture_with_original_size() -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::X11,
            image_backend: Some(CaptureBackendKind::X11),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: Some("x11-root".to_string()),
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
        }
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-agent-cursor-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn visible_overlay_capabilities(
        reason: &str,
    ) -> sky_cua_platform::model::AgentCursorCapabilities {
        sky_cua_platform::model::AgentCursorCapabilities {
            backend: sky_cua_platform::model::AgentCursorBackendKind::WaylandLayerShell,
            renderer_backend: sky_cua_platform::model::AgentCursorRendererBackendKind::Wgpu,
            visible_overlay: true,
            screenshot_synthetic_cursor: false,
            click_through: true,
            capture_exclusion: true,
            pointer_tracking_backend:
                sky_cua_platform::model::AgentCursorPointerTrackingBackendKind::KwinEffectSignal,
            pointer_tracking_exact: true,
            system_cursor_hide_supported: true,
            system_cursor_hidden: true,
            system_cursor_backend:
                sky_cua_platform::model::AgentCursorSystemCursorBackendKind::HyprlandConfig,
            needs_user_install: false,
            reason: Some(reason.to_string()),
            ..Default::default()
        }
    }

    fn snapshot_with_capture(capture: CaptureInfo) -> sky_cua_platform::model::AppStateSnapshot {
        sky_cua_platform::model::AppStateSnapshot {
            snapshot_id: "snapshot".to_string(),
            created_at: chrono::Utc::now(),
            environment: environment(),
            capabilities: tool_capabilities(),
            focused_app: None,
            capture: Some(capture),
            elements: Vec::new(),
            diagnostics: Vec::new(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        }
    }

    fn environment() -> sky_cua_platform::model::EnvironmentInfo {
        sky_cua_platform::model::EnvironmentInfo {
            session_kind: sky_cua_platform::model::SessionKind::Wayland,
            compositor: None,
            desktop_environment: None,
            capture_backend: CaptureBackendKind::PortalPipeWire,
            input_backend: sky_cua_platform::model::InputBackendKind::PortalRemoteDesktop,
            semantic_backend: sky_cua_platform::model::SemanticBackendKind::Atspi,
            portal_capabilities: sky_cua_platform::model::PortalCapabilities {
                screencast_version: None,
                remote_desktop_version: None,
                screenshot_version: None,
                available_source_types: None,
                available_cursor_modes: None,
                available_device_types: None,
            },
            xdg_session_type: None,
            display: None,
            wayland_display: None,
            displays: Vec::new(),
        }
    }

    fn tool_capabilities() -> sky_cua_platform::model::ToolCapabilities {
        let available = sky_cua_platform::model::ToolAvailability {
            available: true,
            reason: None,
        };
        sky_cua_platform::model::ToolCapabilities {
            list_apps: available.clone(),
            get_app_state: available.clone(),
            focus_element: available.clone(),
            activate_element: available.clone(),
            select_element: available.clone(),
            expand_element: available.clone(),
            collapse_element: available.clone(),
            toggle_element: available.clone(),
            click: available.clone(),
            perform_action: available.clone(),
            perform_secondary_action: available.clone(),
            scroll: available.clone(),
            supported_scroll_directions: vec![sky_cua_platform::model::ScrollDirection::Up],
            drag: available.clone(),
            type_text: available.clone(),
            press_key: available.clone(),
            set_value: available,
        }
    }

    #[cfg(unix)]
    fn write_fake_overlay_host(path: &std::path::Path) {
        let script = format!(
            r#"#!/usr/bin/env python3
import json
import os
import socket
import sys

if len(sys.argv) != 4 or sys.argv[1:3] != ["serve", "--socket"]:
    raise SystemExit(f"unexpected argv: {{sys.argv!r}}")

socket_path = sys.argv[3]
try:
    os.unlink(socket_path)
except FileNotFoundError:
    pass

server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(socket_path)
server.listen(8)
state = None
capabilities = {{
    "backend": "wayland_layer_shell",
    "visible_overlay": True,
    "screenshot_synthetic_cursor": False,
    "click_through": True,
    "capture_exclusion": False,
    "needs_user_install": False,
    "reason": "fake host",
}}

while True:
    conn, _ = server.accept()
    with conn:
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(4096)
            if not chunk:
                break
            data += chunk
        if not data.strip():
            continue
        message = json.loads(data.decode("utf-8"))
        kind = message["kind"]
        diagnostics = []
        applied_sequence = None
        if kind == "set_cursor":
            state = message.get("state")
        elif kind == "hide":
            if state is not None:
                state["visible"] = False
            applied_sequence = message.get("sequence")
            if message.get("reason"):
                diagnostics.append({{
                    "code": "OverlayCursorHidden",
                    "message": "Overlay host hid the cursor.",
                    "details": message["reason"],
                }})
        elif kind == "show":
            if state is not None:
                state["visible"] = True
        reply = {{
            "version": {version},
            "ok": True,
            "capabilities": capabilities,
            "lifecycle_state": "backend_ready",
            "applied_sequence": applied_sequence,
            "state": state,
            "diagnostics": diagnostics,
        }}
        conn.sendall(json.dumps(reply).encode("utf-8") + b"\n")
        if kind == "shutdown":
            break

server.close()
try:
    os.unlink(socket_path)
except FileNotFoundError:
    pass
"#,
            version = OVERLAY_HOST_PROTOCOL_VERSION
        );
        std::fs::write(path, script).expect("write fake overlay host");
        let mut permissions = std::fs::metadata(path)
            .expect("fake host metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).expect("chmod fake overlay host");
    }
}
