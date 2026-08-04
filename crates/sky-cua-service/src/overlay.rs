use std::{collections::VecDeque, path::Path};

use sky_cua_overlay_host::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayArrivalCondition, OverlayArrivalOutcome,
    OverlayArrivalWaitRequest, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    OverlayMotionStatus,
};
use sky_cua_platform::model::{
    ActionOutcome, ActionRequest, AgentCursorBackendKind, AgentCursorCapabilities,
    AgentCursorPointerTrackingBackendKind, AgentCursorRendererBackendKind, AgentCursorState,
    AgentCursorSystemCursorBackendKind, AgentOverlayGestureEvent, AgentOverlayHostLifecycleState,
    AppStateSnapshot, DiagnosticEntry,
};

const AGENT_CURSOR_ENV: &str = "SKY_CUA_AGENT_CURSOR";
const OVERLAY_HIDE_FOR_CAPTURE_ENV: &str = "SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE";
const SCREENSHOT_CURSOR_ENV: &str = "SKY_CUA_SCREENSHOT_CURSOR";
const OVERLAY_IDLE_CLEANUP_MS: u64 = 15_000;

mod cursor_geometry;
mod gesture;
mod host;
mod synthetic_cursor;
#[cfg(test)]
pub(crate) mod test_support;

use cursor_geometry::{
    cursor_moving_action, pre_dispatch_state_from_action_request, state_from_action_request,
};
use gesture::gesture_from_action_request;
use host::OverlayHostConnection;
use synthetic_cursor::{
    compose_synthetic_cursor, compose_synthetic_cursor_with_size, remove_synthetic_cursor,
};

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
    prepared_action_sequence: Option<u64>,
    prepared_gesture_sequence: Option<u64>,
    last_motion: Option<OverlayMotionStatus>,
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
            prepared_action_sequence: None,
            prepared_gesture_sequence: None,
            last_motion: None,
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
            prepared_action_sequence: None,
            prepared_gesture_sequence: None,
            last_motion: None,
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
            prepared_action_sequence: None,
            prepared_gesture_sequence: None,
            last_motion: None,
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
            prepared_action_sequence: None,
            prepared_gesture_sequence: None,
            last_motion: None,
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
        self.prepared_action_sequence = None;
        self.prepared_gesture_sequence = None;
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
        let prepared_action_sequence = self.prepared_action_sequence.take();
        let prepared_gesture_sequence = self.prepared_gesture_sequence.take();
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
            // A successful non-pointer action (typing, scrolling, semantic
            // activation) still means the agent is acting on the desktop. Assert
            // the ambient "agent in control" glow without a cursor point: the edge
            // glow is gated on the overlay holding a visible state, decoupled from
            // cursor presence, so this lights it for keyboard/semantic activity.
            // The idle watchdog clears it after OVERLAY_IDLE_CLEANUP_MS.
            let status = self.mark_in_control(request);
            outcome.agent_cursor = status.state.clone();
            return status.diagnostics;
        };
        let (mut diagnostics, state) = if let Some(prepared_state) =
            self.prepared_state_for_success(&state, prepared_action_sequence)
        {
            (Vec::new(), Some(prepared_state))
        } else {
            let status = self.set_state(state);
            (status.diagnostics, status.state)
        };
        outcome.agent_cursor = state;
        if prepared_gesture_sequence.is_none()
            && let Some(gesture) = gesture_from_action_request(request, self.allocate_sequence())
        {
            diagnostics.extend(self.send_gesture_event(gesture));
        }
        diagnostics
    }

    /// Assert (or refresh) the ambient "agent in control" glow: a visible overlay
    /// state with no cursor point. The desktop edge glow is gated on the overlay
    /// holding a visible state (deliberately decoupled from per-surface cursor
    /// presence), so this lights it while the agent acts via keyboard, scroll, or
    /// semantic actions even though the pointer never moves. It is the desktop
    /// analogue of the phone companion's session-scoped `glowActive`; the idle
    /// watchdog releases it after OVERLAY_IDLE_CLEANUP_MS of inactivity.
    fn mark_in_control(&mut self, request: &ActionRequest) -> AgentCursorStatus {
        // Preserve any cursor point the overlay is already showing — the user's
        // tracked physical pointer (kept current by the host's
        // `follow_tracked_pointer`) or a prior action target — so the glyph stays
        // visible at the last known position while the agent acts, instead of
        // blanking to a glow-only state on every keystroke. The host hides the
        // real system cursor whenever the overlay holds a visible state, so a
        // persistent glyph is what keeps the user able to see where their pointer
        // is. Only the glow (a visible state) is asserted unconditionally.
        let (model_point, native_point) = self
            .state
            .as_ref()
            .map(|state| (state.model_point.clone(), state.native_point.clone()))
            .unwrap_or((None, None));
        self.set_state(AgentCursorState {
            visible: true,
            sequence: 0,
            model_point,
            native_point,
            snapshot_id: request.snapshot_id.clone(),
            source_action: Some(request.action.clone()),
            updated_at_ms: 0,
        })
    }

    /// Begin the visual part of a pointer action before backend input dispatch.
    /// The caller waits for [`Self::wait_for_action_visual_arrival`] when
    /// `wait_for_arrival` is true, so physical input cannot overtake the glyph.
    pub fn prepare_action_visual(&mut self, request: &ActionRequest) -> ActionVisualPreparation {
        if self.agent_cursor_mode == CursorMode::Never {
            return ActionVisualPreparation::default();
        }
        if !cursor_moving_action(&request.action) {
            return ActionVisualPreparation::default();
        }
        let Some(state) = pre_dispatch_state_from_action_request(request) else {
            return ActionVisualPreparation::default();
        };
        let status = self.set_state(state);
        self.prepared_action_sequence = status
            .host_delivered
            .then(|| status.state.as_ref().map(|state| state.sequence))
            .flatten();
        let mut diagnostics = status.diagnostics;

        if status.host_delivered
            && status.capabilities.visible_overlay
            && let Some(gesture) = gesture_from_action_request(request, self.allocate_sequence())
        {
            self.prepared_gesture_sequence = Some(gesture.sequence);
            diagnostics.extend(self.send_gesture_event(gesture));
        }

        ActionVisualPreparation {
            diagnostics,
            wait_for_arrival: self.action_visual_arrival_pending(),
        }
    }

    /// Wait once on the host's frame-paced arrival barrier. Socket I/O is
    /// asynchronous, and the caller supplies a budget inside its absolute
    /// dispatch deadline.
    pub async fn wait_for_action_visual_arrival(
        &mut self,
        host_timeout: std::time::Duration,
    ) -> ActionVisualArrival {
        let (sequence, condition) = if let Some(sequence) = self.prepared_gesture_sequence {
            (sequence, OverlayArrivalCondition::GestureFeedbackStarted)
        } else if let Some(sequence) = self.prepared_action_sequence {
            (sequence, OverlayArrivalCondition::MotionSettled)
        } else {
            return ActionVisualArrival {
                outcome: OverlayArrivalOutcome::Unavailable,
                diagnostics: vec![diagnostic(
                    "AgentCursorArrivalUnavailable",
                    "No prepared cursor sequence was available for arrival wait.",
                    None,
                )],
            };
        };
        let timeout_ms = host_timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        if timeout_ms == 0 {
            return ActionVisualArrival {
                outcome: OverlayArrivalOutcome::DeadlineElapsed,
                diagnostics: Vec::new(),
            };
        }
        let message = OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::WaitForArrival,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: Some(OverlayArrivalWaitRequest {
                sequence,
                condition,
                timeout_ms,
            }),
        };
        match self.host.send_arrival_wait(message).await {
            Ok(reply) => {
                let wait_reply = reply.arrival_wait;
                let mut diagnostics = self.apply_host_reply(reply);
                let outcome = wait_reply
                    .filter(|wait_reply| wait_reply.sequence == sequence)
                    .map_or(OverlayArrivalOutcome::Unavailable, |wait_reply| {
                        wait_reply.outcome
                    });
                if wait_reply.is_none_or(|wait_reply| wait_reply.sequence != sequence) {
                    diagnostics.push(diagnostic(
                        "AgentCursorArrivalUnavailable",
                        "Overlay host reply did not identify the prepared cursor sequence.",
                        Some(format!("sequence={sequence}")),
                    ));
                }
                ActionVisualArrival {
                    outcome,
                    diagnostics,
                }
            }
            Err(diagnostic) => ActionVisualArrival {
                outcome: OverlayArrivalOutcome::Unavailable,
                diagnostics: vec![diagnostic],
            },
        }
    }

    fn action_visual_arrival_pending(&self) -> bool {
        if !self
            .host_capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.visible_overlay)
        {
            return false;
        }
        let Some(motion) = self.last_motion else {
            return false;
        };
        if self.prepared_gesture_sequence.is_some() {
            motion.pending_gesture_feedback
        } else {
            !motion.settled
        }
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
                arrival_wait: None,
            };
            match self.host.send(message) {
                Ok(reply) => {
                    barrier_applied = reply.ok && reply.applied_sequence == Some(sequence);
                    diagnostics.extend(self.apply_host_reply(reply));
                }
                Err(diagnostic) => {
                    self.last_motion = None;
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
        self.set_local_visibility(true);
        self.send_host_message(OverlayHostMessageKind::Show, self.state.clone(), None, None)
            .diagnostics
    }

    pub fn apply_to_snapshot(&mut self, snapshot: &mut AppStateSnapshot) {
        self.apply_to_snapshot_with_cursor_size(snapshot, None);
    }

    pub fn apply_to_snapshot_with_cursor_size(
        &mut self,
        snapshot: &mut AppStateSnapshot,
        cursor_size_px: Option<u32>,
    ) {
        snapshot.diagnostics.extend(self.hide_idle_overlay());
        snapshot.agent_cursor = self.state();
        if !self.should_synthesize_cursor() || cursor_size_px == Some(0) {
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

        let result = match cursor_size_px {
            Some(size) => compose_synthetic_cursor_with_size(capture, model_point, Some(size)),
            None => compose_synthetic_cursor(capture, model_point),
        };
        match result {
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
                arrival_wait: None,
            };
            match self.host.send(message) {
                Ok(reply) => {
                    let host_delivered = reply.version == OVERLAY_HOST_PROTOCOL_VERSION && reply.ok;
                    diagnostics.extend(self.apply_host_reply(reply));
                    return AgentCursorStatus {
                        capabilities: self.combined_capabilities(),
                        state: self.state(),
                        diagnostics,
                        host_delivered,
                    };
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
            host_delivered: self.agent_cursor_mode == CursorMode::Never,
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
            arrival_wait: None,
        };
        match self.host.send(message) {
            Ok(reply) => self.apply_host_reply(reply),
            Err(diagnostic) => {
                self.last_motion = None;
                if diagnostic.code == "AgentCursorHostUnavailable" {
                    self.host_capabilities = None;
                    self.host_lifecycle_state = AgentOverlayHostLifecycleState::ProcessUnavailable;
                }
                vec![diagnostic]
            }
        }
    }

    fn apply_host_reply(&mut self, reply: OverlayHostReply) -> Vec<DiagnosticEntry> {
        self.last_motion = reply.motion;
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

    fn prepared_state_for_success(
        &self,
        requested: &AgentCursorState,
        prepared_action_sequence: Option<u64>,
    ) -> Option<AgentCursorState> {
        let current = self.state.as_ref()?;
        if Some(current.sequence) != prepared_action_sequence {
            return None;
        }
        if current.visible
            && current.model_point == requested.model_point
            && current.native_point == requested.native_point
            && current.snapshot_id == requested.snapshot_id
            && current.source_action == requested.source_action
        {
            return self.state.clone();
        }
        None
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
            host_delivered: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentCursorStatus {
    pub capabilities: AgentCursorCapabilities,
    pub state: Option<AgentCursorState>,
    pub diagnostics: Vec<DiagnosticEntry>,
    host_delivered: bool,
}

#[derive(Debug, Default)]
pub struct ActionVisualPreparation {
    pub diagnostics: Vec<DiagnosticEntry>,
    pub wait_for_arrival: bool,
}

#[derive(Debug)]
pub struct ActionVisualArrival {
    pub outcome: OverlayArrivalOutcome,
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
    #[cfg(unix)]
    use super::test_support::write_fake_overlay_host;
    use super::{
        OVERLAY_IDLE_CLEANUP_MS, OverlayController, gesture_from_action_request, now_ms,
        state_from_action_request,
    };
    use image::{ImageBuffer, Rgba};
    use sky_cua_overlay_host::{
        OVERLAY_HOST_PROTOCOL_VERSION, OverlayArrivalOutcome, OverlayHostReply,
    };
    use sky_cua_platform::model::{
        ActionName, ActionOutcome, ActionRequest, CaptureBackendKind, CaptureInfo, CaptureScope,
        CoordinateSpace, ElementNode, ModelImageFormat, PixelSize, RectF,
    };
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
            !controller
                .state()
                .expect("service state follows host-hidden capture state")
                .visible
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
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prepared_pointer_action_waits_for_host_arrival_and_does_not_replay_gesture() {
        if Command::new("python3").arg("--version").status().is_err() {
            return;
        }

        let dir = unique_temp_dir("host-action-arrival");
        let host_path = dir.join("fake-overlay-host.py");
        let socket_path = dir.join("agent-cursor.sock");
        write_fake_overlay_host(&host_path);
        let mut controller =
            OverlayController::new_for_tests_with_host(host_path, socket_path.clone());
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));

        let preparation = controller.prepare_action_visual(&request);

        assert!(preparation.diagnostics.is_empty());
        assert!(preparation.wait_for_arrival);
        assert_eq!(
            controller.recent_gesture_ids.front().map(String::as_str),
            Some("click-2")
        );

        let arrived = controller
            .wait_for_action_visual_arrival(std::time::Duration::from_millis(200))
            .await;
        assert_eq!(arrived.outcome, OverlayArrivalOutcome::Arrived);
        assert!(arrived.diagnostics.is_empty());
        assert_eq!(
            std::fs::read_to_string(format!("{}.requests", socket_path.display()))
                .expect("arrival request log")
                .lines()
                .collect::<Vec<_>>(),
            vec!["set_cursor", "animate_gesture", "wait_for_arrival"]
        );

        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };
        let diagnostics = controller.update_from_action(&request, &mut outcome);

        assert!(diagnostics.is_empty());
        assert_eq!(controller.recent_gesture_ids.len(), 1);
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
            motion: None,
            arrival_wait: None,
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
            motion: None,
            arrival_wait: None,
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
            motion: None,
            arrival_wait: None,
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
            motion: None,
            arrival_wait: None,
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
            motion: None,
            arrival_wait: None,
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
            motion: None,
            arrival_wait: None,
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
            motion: None,
            arrival_wait: None,
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
        assert_eq!(gesture.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(gesture.mapping_id.as_deref(), Some("mapping"));
        assert_eq!(
            gesture.duration_ms,
            sky_cua_platform::overlay_spec::shared::timing::MIN_GESTURE_DURATION_MS
        );
    }

    #[test]
    fn gesture_preserves_native_coordinate_space_and_mapping() {
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

        let gesture = gesture_from_action_request(&request, 8).expect("gesture");

        assert_eq!(gesture.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(gesture.mapping_id.as_deref(), Some("mapping"));
        assert_eq!(gesture.points[0].x, 120.0);
        assert_eq!(gesture.points[0].y, 70.0);
    }

    #[test]
    fn gesture_preserves_stream_logical_coordinate_space() {
        let mut request = action_request(ActionName::Click, serde_json::json!({}));
        request.resolved_element = Some(element_with_bounds(RectF {
            x: 10.0,
            y: 15.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::StreamLogical,
        }));
        request.resolved_capture = None;

        let gesture = gesture_from_action_request(&request, 9).expect("gesture");

        assert_eq!(gesture.coordinate_space, CoordinateSpace::StreamLogical);
        assert_eq!(gesture.mapping_id, None);
        assert_eq!(gesture.points[0].x, 20.0);
        assert_eq!(gesture.points[0].y, 20.0);
    }

    #[test]
    fn drag_gesture_rejects_incompatible_coordinate_mappings() {
        let mut request = action_request(ActionName::Drag, serde_json::json!({}));
        request.resolved_element = Some(element_with_bounds(RectF {
            x: 10.0,
            y: 15.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::StreamLogical,
        }));
        request.resolved_target_element = Some(element_with_bounds(RectF {
            x: 150.0,
            y: 70.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::DesktopLogical,
        }));

        assert!(gesture_from_action_request(&request, 10).is_none());
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

        let preparation = controller.prepare_action_visual(&request);

        assert!(preparation.diagnostics.is_empty());
        assert!(!preparation.wait_for_arrival);
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

        let preparation = controller.prepare_action_visual(&request);

        assert!(preparation.diagnostics.is_empty());
        assert!(!preparation.wait_for_arrival);
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
    fn update_from_action_lights_in_control_glow_for_non_pointer_action() {
        let mut controller = OverlayController::new_for_tests();
        let request = action_request(ActionName::TypeText, serde_json::json!({"text": "hi"}));
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        controller.update_from_action(&request, &mut outcome);

        let state = controller
            .state()
            .expect("non-pointer action should hold the in-control glow state");
        assert!(state.visible, "in-control glow state must be visible");
        assert!(
            state.model_point.is_none() && state.native_point.is_none(),
            "the in-control glow is decoupled from cursor presence: no point"
        );
        assert_eq!(state.source_action, Some(ActionName::TypeText));
        assert_eq!(
            outcome.agent_cursor,
            Some(state),
            "the glow state should be attached to the successful outcome"
        );
    }

    #[test]
    fn in_control_glow_preserves_existing_cursor_point_so_glyph_stays_visible() {
        let mut controller = OverlayController::new_for_tests();
        // Stand in for the host having placed the glyph (a prior action or the
        // user's tracked physical pointer).
        controller.set_state(synthetic_state(100, 200));
        let request = action_request(ActionName::TypeText, serde_json::json!({"text": "x"}));
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        controller.update_from_action(&request, &mut outcome);

        let state = controller.state().expect("in-control state");
        assert!(state.visible, "glow stays on");
        let point = state
            .model_point
            .expect("the glyph's prior point must be preserved, not blanked");
        assert!((point.x - 100.0).abs() < f64::EPSILON && (point.y - 200.0).abs() < f64::EPSILON);
        assert_eq!(state.source_action, Some(ActionName::TypeText));
    }

    #[test]
    fn update_from_action_does_not_light_glow_for_failed_non_pointer_action() {
        let mut controller = OverlayController::new_for_tests();
        let request = action_request(ActionName::TypeText, serde_json::json!({"text": "hi"}));
        let mut outcome = ActionOutcome {
            success: false,
            message: "nope".to_string(),
            code: "Error".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        controller.update_from_action(&request, &mut outcome);

        assert!(
            controller.state().is_none(),
            "a failed action must not assert the in-control glow"
        );
        assert!(outcome.agent_cursor.is_none());
    }

    #[test]
    fn update_from_action_reuses_prepared_cursor_state_for_success_effect() {
        let mut controller = OverlayController::new_for_tests();
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        controller.prepare_action_visual(&request);
        controller.update_from_action(&request, &mut outcome);

        let state = outcome.agent_cursor.expect("outcome should carry cursor");
        assert_eq!(state.sequence, 1);
        assert_eq!(controller.state().expect("controller state").sequence, 1);
        assert_eq!(
            controller.recent_gesture_ids.front().map(String::as_str),
            Some("click-2")
        );
    }

    #[test]
    fn update_from_action_for_drag_advances_to_target_without_reusing_origin() {
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
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        controller.prepare_action_visual(&request);
        // The prepared visual sits at the drag origin.
        assert_eq!(
            controller
                .state()
                .expect("prepared state")
                .native_point
                .as_ref()
                .expect("native")
                .x,
            40.0
        );

        controller.update_from_action(&request, &mut outcome);

        // The prepared origin state must not be reused for a drag: origin and
        // target differ, so `prepared_state_for_success` declines and the cursor
        // advances to the target with a fresh sequence. A future refactor that
        // aligned the prepare and update point builders would silently reuse the
        // origin and strand the cursor at the drag start; this pins the seam.
        let state = outcome.agent_cursor.expect("outcome should carry cursor");
        assert_eq!(state.native_point.as_ref().expect("native").x, 200.0);
        assert_eq!(state.native_point.as_ref().expect("native").y, 150.0);
        assert!(
            state.sequence > 1,
            "drag should allocate a new sequence past the prepared origin"
        );
        assert_eq!(
            controller
                .state()
                .expect("controller state")
                .native_point
                .as_ref()
                .expect("native")
                .x,
            200.0
        );
    }

    #[test]
    fn update_from_action_retries_cursor_state_after_failed_prepare() {
        let mut controller =
            OverlayController::new_for_tests_with_failing_host("AgentCursorHostRequestFailed");
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        let preparation = controller.prepare_action_visual(&request);
        assert!(
            preparation
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostRequestFailed")
        );

        let diagnostics = controller.update_from_action(&request, &mut outcome);

        let state = outcome.agent_cursor.expect("outcome should carry cursor");
        assert_eq!(state.sequence, 2);
        assert_eq!(controller.state().expect("controller state").sequence, 2);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|entry| entry.code == "AgentCursorHostRequestFailed")
                .count(),
            2
        );
        assert_eq!(
            controller.recent_gesture_ids.front().map(String::as_str),
            Some("click-3")
        );
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
            motion: None,
            arrival_wait: None,
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
    fn restore_after_capture_restores_local_visibility_when_host_request_fails() {
        let mut controller =
            OverlayController::new_for_tests_with_failing_host("AgentCursorHostRequestFailed");
        controller.apply_host_reply(OverlayHostReply {
            motion: None,
            arrival_wait: None,
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

        let diagnostics = controller.restore_after_capture(guard);

        assert!(
            diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostRequestFailed")
        );
        assert!(controller.state().expect("restored state").visible);
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
            appshot_id: None,
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
}
