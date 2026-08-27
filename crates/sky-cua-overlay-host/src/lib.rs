use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPointerTrackingBackendKind,
    AgentCursorRendererBackendKind, AgentCursorState, AgentCursorSystemCursorBackendKind,
    AgentOverlayEffectsCapabilities, AgentOverlayGestureEvent, AgentOverlayGestureKind,
    AgentOverlayHostLifecycleState, DiagnosticEntry,
};

pub mod cursor_motion;
#[cfg(target_os = "linux")]
mod layer_shell;
pub mod motion;
#[cfg(target_os = "linux")]
mod playground;
#[cfg(target_os = "linux")]
mod pointer_tracking;
#[cfg(target_os = "linux")]
mod renderer;
mod system_cursor;

pub const OVERLAY_HOST_PROTOCOL_VERSION: u32 = 3;

/// Run the interactive desktop pointer playground (Wayland layer-shell only).
#[cfg(target_os = "linux")]
pub fn run_playground(args: Vec<String>) -> anyhow::Result<()> {
    playground::run_from_args(args)
}

/// Run the interactive desktop pointer playground (unsupported off Linux).
#[cfg(not(target_os = "linux"))]
pub fn run_playground(_args: Vec<String>) -> anyhow::Result<()> {
    anyhow::bail!("sky-cua-overlay-host playground requires a Linux/Wayland session")
}
use sky_cua_platform::config::OVERLAY_BACKEND_ENV;

pub mod cursor_asset {
    pub const AGENT_CURSOR_PNG: &[u8] = include_bytes!("../assets/cursor-chat.png");
    pub const AGENT_CURSOR_SOURCE_WIDTH: u32 = 46;
    pub const AGENT_CURSOR_SOURCE_HEIGHT: u32 = 48;
    pub const AGENT_CURSOR_WIDTH: u32 = 23;
    pub const AGENT_CURSOR_HEIGHT: u32 = 24;

    // The Chrome extension renders the 2x PNG at 23x24 CSS pixels.
    pub const AGENT_CURSOR_HOTSPOT_X: i32 = 10;
    pub const AGENT_CURSOR_HOTSPOT_Y: i32 = 11;

    // The desktop overlay (layer-shell, X11, KWin effect, playground) draws the
    // cursor near the browser/synthetic size: the 46x48 source scaled down to a
    // compact on-screen footprint, with a proportional hotspot. The
    // screenshot-synthetic and phone cursor planes keep the base 23x24 size.
    pub const AGENT_CURSOR_DESKTOP_WIDTH: u32 = 30;
    pub const AGENT_CURSOR_DESKTOP_HEIGHT: u32 = 31;
    pub const AGENT_CURSOR_DESKTOP_HOTSPOT_X: i32 = 13;
    pub const AGENT_CURSOR_DESKTOP_HOTSPOT_Y: i32 = 14;

    // Logical-pixel margin of animated smoke space around the desktop cursor
    // glyph. The cursor texture is rendered at the glyph size plus this margin
    // on every side so the WGPU shader has room to billow border-style smoke off
    // the glyph silhouette (see `renderer::render_vector_cursor` /
    // `cursor_smoke`). The glyph itself stays `AGENT_CURSOR_DESKTOP_*`; only the
    // sampled footprint and its hotspot grow.
    pub const AGENT_CURSOR_SMOKE_MARGIN: u32 = 30;
    pub const AGENT_CURSOR_FOOTPRINT_WIDTH: u32 =
        AGENT_CURSOR_DESKTOP_WIDTH + 2 * AGENT_CURSOR_SMOKE_MARGIN;
    pub const AGENT_CURSOR_FOOTPRINT_HEIGHT: u32 =
        AGENT_CURSOR_DESKTOP_HEIGHT + 2 * AGENT_CURSOR_SMOKE_MARGIN;
    pub const AGENT_CURSOR_FOOTPRINT_HOTSPOT_X: i32 =
        AGENT_CURSOR_DESKTOP_HOTSPOT_X + AGENT_CURSOR_SMOKE_MARGIN as i32;
    pub const AGENT_CURSOR_FOOTPRINT_HOTSPOT_Y: i32 =
        AGENT_CURSOR_DESKTOP_HOTSPOT_Y + AGENT_CURSOR_SMOKE_MARGIN as i32;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayHostMessage {
    pub version: u32,
    pub kind: OverlayHostMessageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AgentCursorState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<AgentOverlayGestureEvent>,
    /// Sequence number for stateful requests such as hide-for-capture barriers.
    /// The host replies with `applied_sequence` once the request has taken
    /// effect on all active surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_wait: Option<OverlayArrivalWaitRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayHostMessageKind {
    Hello,
    Capabilities,
    SetCursor,
    Hide,
    Show,
    Ping,
    Shutdown,
    AnimateGesture,
    WaitForArrival,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayHostReply {
    pub version: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<AgentCursorCapabilities>,
    /// Current host lifecycle state. Clients must not infer state from prose
    /// when this field is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<AgentOverlayHostLifecycleState>,
    /// Sequence number applied by the host for requests that require a barrier
    /// (for example, hide-for-capture). Present only when the host has
    /// confirmed the request has taken effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AgentCursorState>,
    /// Live drawn-cursor motion pose (layer-shell backend only). `state`
    /// carries the target; this carries where the vehicle-steered glyph
    /// actually is, so clients and smokes can assert glide behavior from
    /// structured fields instead of prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<OverlayMotionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_wait: Option<OverlayArrivalWaitReply>,
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverlayArrivalWaitRequest {
    pub sequence: u64,
    pub condition: OverlayArrivalCondition,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayArrivalCondition {
    GestureFeedbackStarted,
    MotionSettled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverlayArrivalWaitReply {
    pub sequence: u64,
    pub outcome: OverlayArrivalOutcome,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayArrivalOutcome {
    Arrived,
    DeadlineElapsed,
    Superseded,
    Unavailable,
}

/// Structured echo of the motion driver's latest frame. Coordinates are in
/// the mover's coordinate space (desktop-logical for normal desktop use).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct OverlayMotionStatus {
    pub x: f64,
    pub y: f64,
    pub heading_deg: f64,
    pub speed: f64,
    /// True when the mover is parked on its most recent target.
    pub settled: bool,
    /// True while a gesture waits for the cursor to sail to its start point
    /// (arrival-gated feedback has not fired yet).
    pub pending_gesture_feedback: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NoopOverlayBackend {
    state: Option<AgentCursorState>,
    reason: Option<String>,
    gesture_tracker: GestureEventTracker,
}

impl NoopOverlayBackend {
    #[must_use]
    pub fn with_reason(reason: impl Into<String>) -> Self {
        Self {
            state: None,
            reason: Some(reason.into()),
            gesture_tracker: GestureEventTracker::default(),
        }
    }

    #[must_use]
    pub fn default_capabilities() -> AgentCursorCapabilities {
        AgentCursorCapabilities {
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
            reason: Some("no visible overlay backend selected".to_string()),
            effects: Some(AgentOverlayEffectsCapabilities::default()),
            coverage: Some(sky_cua_platform::model::AgentOverlayCoverageKind::None),
            supported_coordinate_spaces: Vec::new(),
            max_gesture_points: Some(
                sky_cua_platform::overlay_spec::shared::effects::MAX_GESTURE_POINTS,
            ),
            protocol_version: Some(OVERLAY_HOST_PROTOCOL_VERSION),
            effect_schema_version: Some(sky_cua_platform::overlay_spec::SCHEMA_VERSION),
            active_output_count: Some(0),
            rendered_output_count: Some(0),
            adapter_name: None,
        }
    }

    #[must_use]
    pub fn capabilities(&self) -> AgentCursorCapabilities {
        let mut capabilities = Self::default_capabilities();
        if let Some(reason) = self.reason.clone() {
            capabilities.reason = Some(reason);
        }
        capabilities
    }

    pub fn handle_message(&mut self, message: OverlayHostMessage) -> OverlayHostReply {
        if message.version != OVERLAY_HOST_PROTOCOL_VERSION {
            return error_reply(
                "OverlayProtocolVersionMismatch",
                "Overlay host protocol version mismatch.",
                Some(format!(
                    "expected={} got={}",
                    OVERLAY_HOST_PROTOCOL_VERSION, message.version
                )),
            );
        }

        match message.kind {
            OverlayHostMessageKind::Hello
            | OverlayHostMessageKind::Ping
            | OverlayHostMessageKind::Shutdown
            | OverlayHostMessageKind::Capabilities => self.reply(true, Vec::new()),
            OverlayHostMessageKind::WaitForArrival => {
                let Some(wait) = message.arrival_wait else {
                    return self.reply(
                        false,
                        vec![diagnostic(
                            "OverlayArrivalWaitMissing",
                            "WaitForArrival message did not include an arrival wait request.",
                            None,
                        )],
                    );
                };
                self.arrival_reply(wait.sequence, OverlayArrivalOutcome::Unavailable)
            }
            OverlayHostMessageKind::SetCursor => {
                self.state = message.state;
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::AnimateGesture => {
                let (ok, _gesture, mut diagnostics) =
                    validate_gesture_message(message.gesture, &mut self.gesture_tracker);
                if ok {
                    diagnostics.push(diagnostic(
                        "OverlayGestureNotSupported",
                        "Noop backend does not render gestures.",
                        None,
                    ));
                }
                self.reply(ok, diagnostics)
            }
            OverlayHostMessageKind::Hide => {
                if let Some(state) = self.state.as_mut() {
                    state.visible = false;
                }
                let applied_sequence = message.sequence;
                self.reply_with_lifecycle(
                    true,
                    AgentOverlayHostLifecycleState::BackendUnsupported,
                    applied_sequence,
                    message.reason.map_or_else(Vec::new, |reason| {
                        vec![diagnostic(
                            "OverlayCursorHidden",
                            "Overlay host hid the cursor.",
                            Some(reason),
                        )]
                    }),
                )
            }
            OverlayHostMessageKind::Show => {
                self.state = message.state;
                if let Some(state) = self.state.as_mut() {
                    state.visible = true;
                }
                self.reply(true, Vec::new())
            }
        }
    }

    fn reply(&self, ok: bool, diagnostics: Vec<DiagnosticEntry>) -> OverlayHostReply {
        self.reply_with_lifecycle(
            ok,
            AgentOverlayHostLifecycleState::BackendUnsupported,
            None,
            diagnostics,
        )
    }

    fn reply_with_lifecycle(
        &self,
        ok: bool,
        lifecycle_state: AgentOverlayHostLifecycleState,
        applied_sequence: Option<u64>,
        diagnostics: Vec<DiagnosticEntry>,
    ) -> OverlayHostReply {
        OverlayHostReply {
            motion: None,
            arrival_wait: None,
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok,
            capabilities: Some(self.capabilities()),
            lifecycle_state: Some(lifecycle_state),
            applied_sequence,
            state: self.state.clone(),
            diagnostics,
        }
    }

    fn arrival_reply(&self, sequence: u64, outcome: OverlayArrivalOutcome) -> OverlayHostReply {
        let mut reply = self.reply(true, Vec::new());
        reply.arrival_wait = Some(OverlayArrivalWaitReply { sequence, outcome });
        reply
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct GestureEventTracker {
    recent_event_ids: VecDeque<String>,
    highest_sequence: u64,
}

impl GestureEventTracker {
    const MAX_RECENT_EVENTS: usize = 128;

    fn record_event_id(&mut self, event_id: String) {
        if self.recent_event_ids.len() >= Self::MAX_RECENT_EVENTS {
            let _ = self.recent_event_ids.pop_front();
        }
        self.recent_event_ids.push_back(event_id);
    }

    fn has_event_id(&self, event_id: &str) -> bool {
        self.recent_event_ids
            .iter()
            .any(|recent| recent == event_id)
    }
}

pub(crate) fn validate_gesture_message(
    gesture: Option<AgentOverlayGestureEvent>,
    tracker: &mut GestureEventTracker,
) -> (bool, Option<AgentOverlayGestureEvent>, Vec<DiagnosticEntry>) {
    let Some(mut gesture) = gesture else {
        return (
            false,
            None,
            vec![diagnostic(
                "OverlayGestureMissing",
                "AnimateGesture message did not include a gesture payload.",
                None,
            )],
        );
    };

    if gesture.event_id.trim().is_empty() {
        return (
            false,
            None,
            vec![diagnostic(
                "OverlayGestureInvalid",
                "Gesture event_id must be non-empty.",
                None,
            )],
        );
    }

    if tracker.has_event_id(&gesture.event_id) {
        return (
            true,
            None,
            vec![diagnostic(
                "OverlayGestureDuplicate",
                "Duplicate gesture event ignored.",
                Some(gesture.event_id),
            )],
        );
    }

    if gesture.sequence <= tracker.highest_sequence {
        return (
            false,
            None,
            vec![diagnostic(
                "OverlayGestureStaleSequence",
                "Gesture sequence is stale.",
                Some(format!(
                    "event_id={} sequence={} highest_seen={}",
                    gesture.event_id, gesture.sequence, tracker.highest_sequence
                )),
            )],
        );
    }

    let required_points = match gesture.kind {
        AgentOverlayGestureKind::Tap | AgentOverlayGestureKind::NoNo => 1,
        AgentOverlayGestureKind::Drag | AgentOverlayGestureKind::Swipe => 2,
    };
    let point_count = gesture.points.len();
    let valid_point_count = match gesture.kind {
        AgentOverlayGestureKind::Tap | AgentOverlayGestureKind::NoNo => point_count == 1,
        AgentOverlayGestureKind::Drag | AgentOverlayGestureKind::Swipe => point_count >= 2,
    };
    if !valid_point_count {
        return (
            false,
            None,
            vec![diagnostic(
                "OverlayGestureInvalidPointCount",
                "Gesture point count does not match the gesture kind.",
                Some(format!(
                    "event_id={} kind={:?} points={} required={}",
                    gesture.event_id, gesture.kind, point_count, required_points
                )),
            )],
        );
    }

    let max_points = sky_cua_platform::overlay_spec::shared::effects::MAX_GESTURE_POINTS as usize;
    if point_count > max_points {
        return (
            false,
            None,
            vec![diagnostic(
                "OverlayGestureTooManyPoints",
                "Gesture contains too many points.",
                Some(format!(
                    "event_id={} points={} max={}",
                    gesture.event_id, point_count, max_points
                )),
            )],
        );
    }

    if gesture
        .points
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return (
            false,
            None,
            vec![diagnostic(
                "OverlayGestureInvalidPoint",
                "Gesture points must contain finite coordinates.",
                Some(gesture.event_id),
            )],
        );
    }

    let original_duration_ms = gesture.duration_ms;
    gesture.duration_ms = gesture.duration_ms.clamp(
        sky_cua_platform::overlay_spec::shared::timing::MIN_GESTURE_DURATION_MS,
        sky_cua_platform::overlay_spec::shared::timing::MAX_GESTURE_DURATION_MS,
    );

    tracker.highest_sequence = gesture.sequence;
    tracker.record_event_id(gesture.event_id.clone());

    let mut diagnostics = Vec::new();
    if original_duration_ms != gesture.duration_ms {
        diagnostics.push(diagnostic(
            "OverlayGestureDurationClamped",
            "Gesture duration was clamped to the shared overlay spec bounds.",
            Some(format!(
                "event_id={} requested_ms={} clamped_ms={}",
                gesture.event_id, original_duration_ms, gesture.duration_ms
            )),
        ));
    }
    (true, Some(gesture), diagnostics)
}

#[derive(Debug)]
pub enum OverlayHostBackend {
    Noop(NoopOverlayBackend),
    #[cfg(target_os = "linux")]
    LayerShell(Box<layer_shell::LayerShellOverlayBackend>),
}

impl OverlayHostBackend {
    #[must_use]
    pub fn from_env() -> Self {
        let mode = std::env::var(OVERLAY_BACKEND_ENV)
            .unwrap_or_else(|_| "auto".to_string())
            .trim()
            .to_ascii_lowercase();
        Self::from_mode(&mode)
    }

    #[must_use]
    fn from_mode(mode: &str) -> Self {
        if matches!(mode, "none" | "never" | "off" | "false" | "0") {
            return Self::Noop(NoopOverlayBackend::with_reason(format!(
                "{OVERLAY_BACKEND_ENV}={mode}"
            )));
        }
        if matches!(mode, "noop" | "no-op") {
            return Self::Noop(NoopOverlayBackend::with_reason(format!(
                "{OVERLAY_BACKEND_ENV}={mode}"
            )));
        }
        if matches!(
            mode,
            "kwin" | "kwin-effect" | "kwin_effect" | "kde-kwin-effect" | "kde_kwin_effect"
        ) {
            return Self::Noop(NoopOverlayBackend::with_reason(format!(
                "{OVERLAY_BACKEND_ENV}={mode} is no longer a selectable visible overlay backend; use auto or wayland_layer_shell for KDE visuals"
            )));
        }

        #[cfg(target_os = "linux")]
        {
            if matches!(
                mode,
                "gnome"
                    | "gnome-shell"
                    | "gnome_shell"
                    | "gnome-shell-extension"
                    | "gnome_shell_extension"
            ) {
                return Self::Noop(NoopOverlayBackend::with_reason(
                    "GNOME Shell visual rendering was retired; no WGPU GNOME overlay host is available",
                ));
            }
            if matches!(
                mode,
                "layer-shell" | "layer_shell" | "wayland-layer-shell" | "wayland_layer_shell"
            ) {
                return match layer_shell::LayerShellOverlayBackend::connect() {
                    Ok(backend) => Self::LayerShell(Box::new(backend)),
                    Err(error) => Self::Noop(NoopOverlayBackend::with_reason(format!(
                        "wayland layer-shell unavailable: {error}"
                    ))),
                };
            }
            if matches!(
                mode,
                "x11" | "x11-shaped" | "x11_shaped" | "x11-shaped-window" | "x11_shaped_window"
            ) {
                return Self::Noop(NoopOverlayBackend::with_reason(
                    "X11 visible overlay requires a WGPU X11 host, tracked as a follow-on plan",
                ));
            }
            if matches!(mode, "auto" | "") {
                let mut reasons = Vec::new();
                if linux_env_value("WAYLAND_DISPLAY").is_some() {
                    match layer_shell::LayerShellOverlayBackend::connect() {
                        Ok(backend) => return Self::LayerShell(Box::new(backend)),
                        Err(error) => {
                            reasons.push(format!("wayland layer-shell unavailable: {error}"));
                        }
                    }
                }
                if reasons.is_empty() {
                    reasons.push("auto visible overlay requires a WGPU-capable Wayland layer-shell session; GNOME actor and X11 rectangle renderers are retired".to_string());
                }
                return Self::Noop(NoopOverlayBackend::with_reason(reasons.join("; ")));
            }
        }

        Self::Noop(NoopOverlayBackend::with_reason(format!(
            "unsupported {OVERLAY_BACKEND_ENV}={mode}"
        )))
    }

    pub fn handle_message(&mut self, message: OverlayHostMessage) -> OverlayHostReply {
        match self {
            Self::Noop(backend) => backend.handle_message(message),
            #[cfg(target_os = "linux")]
            Self::LayerShell(backend) => backend.handle_message(message),
        }
    }

    pub fn tick(&mut self) {
        match self {
            Self::Noop(_backend) => {}
            #[cfg(target_os = "linux")]
            Self::LayerShell(backend) => backend.tick(),
        }
    }

    /// Cadence the host event loop should tick at, matched to the fastest
    /// connected display so the agent-cursor follow renders at the panel's
    /// refresh rate instead of a fixed 60 Hz. Falls back to 60 Hz when no
    /// display refresh rate is known.
    #[must_use]
    pub fn pointer_tick_interval(&self) -> std::time::Duration {
        match self {
            Self::Noop(_backend) => std::time::Duration::from_millis(16),
            #[cfg(target_os = "linux")]
            Self::LayerShell(backend) => backend.pointer_tick_interval(),
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[must_use]
pub fn probe_reply() -> OverlayHostReply {
    noop_probe_reply(NoopOverlayBackend::default_capabilities())
}

#[must_use]
pub fn probe_environment_reply() -> OverlayHostReply {
    OverlayHostBackend::from_env().handle_message(OverlayHostMessage {
        version: OVERLAY_HOST_PROTOCOL_VERSION,
        kind: OverlayHostMessageKind::Capabilities,
        state: None,
        gesture: None,
        sequence: None,
        reason: None,
        arrival_wait: None,
    })
}

#[must_use]
fn noop_probe_reply(capabilities: AgentCursorCapabilities) -> OverlayHostReply {
    OverlayHostReply {
        motion: None,
        arrival_wait: None,
        version: OVERLAY_HOST_PROTOCOL_VERSION,
        ok: true,
        capabilities: Some(capabilities),
        lifecycle_state: Some(AgentOverlayHostLifecycleState::BackendUnsupported),
        applied_sequence: None,
        state: None,
        diagnostics: Vec::new(),
    }
}

fn error_reply(code: &str, message: &str, details: Option<String>) -> OverlayHostReply {
    OverlayHostReply {
        motion: None,
        arrival_wait: None,
        version: OVERLAY_HOST_PROTOCOL_VERSION,
        ok: false,
        capabilities: Some(NoopOverlayBackend::default_capabilities()),
        lifecycle_state: None,
        applied_sequence: None,
        state: None,
        diagnostics: vec![diagnostic(code, message, details)],
    }
}

pub(crate) fn diagnostic(code: &str, message: &str, details: Option<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use image::GenericImageView;
    use sky_cua_platform::model::{
        ActionName, AgentCursorBackendKind, AgentCursorPoint, AgentCursorState,
        AgentOverlayGestureEvent, AgentOverlayGestureKind, CoordinateSpace, Point2,
    };

    use super::{
        NoopOverlayBackend, OVERLAY_HOST_PROTOCOL_VERSION, OverlayArrivalCondition,
        OverlayArrivalOutcome, OverlayArrivalWaitRequest, OverlayHostMessage,
        OverlayHostMessageKind, cursor_asset, probe_reply,
    };

    #[test]
    fn protocol_messages_use_snake_case_kind_values() {
        let message = OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        };

        let rendered = serde_json::to_value(message).expect("serialize message");

        assert_eq!(rendered["version"], OVERLAY_HOST_PROTOCOL_VERSION);
        assert_eq!(rendered["kind"], "set_cursor");
        assert_eq!(
            rendered["state"]["model_point"]["coordinate_space"],
            "stream_pixels"
        );
    }

    #[test]
    fn arrival_wait_protocol_round_trips_structured_outcome() {
        let mut backend = NoopOverlayBackend::default();
        let reply = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::WaitForArrival,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: Some(OverlayArrivalWaitRequest {
                sequence: 42,
                condition: OverlayArrivalCondition::GestureFeedbackStarted,
                timeout_ms: 100,
            }),
        });

        assert!(reply.ok);
        let wait = reply.arrival_wait.expect("arrival wait reply");
        assert_eq!(wait.sequence, 42);
        assert_eq!(wait.outcome, OverlayArrivalOutcome::Unavailable);
        let rendered = serde_json::to_value(&reply).expect("serialize arrival wait reply");
        assert_eq!(rendered["arrival_wait"]["outcome"], "unavailable");
    }

    #[test]
    fn reply_motion_echo_round_trips_and_stays_optional() {
        let with_motion = super::OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok: true,
            capabilities: None,
            lifecycle_state: None,
            applied_sequence: None,
            state: None,
            motion: Some(super::OverlayMotionStatus {
                x: 120.5,
                y: 480.25,
                heading_deg: -135.0,
                speed: 812.0,
                settled: false,
                pending_gesture_feedback: true,
            }),
            arrival_wait: None,
            diagnostics: Vec::new(),
        };
        let rendered = serde_json::to_value(&with_motion).expect("serialize reply");
        assert_eq!(rendered["motion"]["x"], 120.5);
        assert_eq!(rendered["motion"]["settled"], false);
        assert_eq!(rendered["motion"]["pending_gesture_feedback"], true);
        let parsed: super::OverlayHostReply =
            serde_json::from_value(rendered).expect("round trip reply");
        assert_eq!(parsed, with_motion);

        // A reply without the field (an older host) must still parse, and a
        // motion-less reply must not serialize the key at all.
        let legacy: super::OverlayHostReply =
            serde_json::from_str(r#"{"version":2,"ok":true}"#).expect("parse legacy reply");
        assert_eq!(legacy.motion, None);
        let without = super::OverlayHostReply {
            motion: None,
            ..with_motion
        };
        let rendered = serde_json::to_value(&without).expect("serialize motion-less reply");
        assert!(rendered.get("motion").is_none());
    }

    #[test]
    fn noop_backend_records_latest_state_and_toggles_visibility() {
        let mut backend = NoopOverlayBackend::default();
        let set = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        });
        assert!(set.ok);
        assert!(set.state.as_ref().expect("state").visible);

        let hidden = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Hide,
            state: None,
            gesture: None,
            sequence: None,
            reason: Some("capture".to_string()),
            arrival_wait: None,
        });
        assert!(!hidden.state.as_ref().expect("state").visible);
        assert!(
            hidden
                .diagnostics
                .iter()
                .any(|entry| entry.code == "OverlayCursorHidden")
        );

        let shown = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Show,
            state: hidden.state.clone(),
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        });
        assert!(shown.state.expect("state").visible);

        let cleared = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Show,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        });
        assert!(cleared.state.is_none());
    }

    #[test]
    fn protocol_version_mismatch_fails_loudly() {
        let mut backend = NoopOverlayBackend::default();

        let reply = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION + 1,
            kind: OverlayHostMessageKind::Capabilities,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        });

        assert!(!reply.ok);
        assert_eq!(
            reply.diagnostics.first().map(|entry| entry.code.as_str()),
            Some("OverlayProtocolVersionMismatch")
        );
    }

    #[test]
    fn old_protocol_version_mismatch_fails_loudly() {
        let mut backend = NoopOverlayBackend::default();

        let reply = backend.handle_message(OverlayHostMessage {
            version: 1,
            kind: OverlayHostMessageKind::Capabilities,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        });

        assert!(!reply.ok);
        assert_eq!(
            reply.diagnostics.first().map(|entry| entry.code.as_str()),
            Some("OverlayProtocolVersionMismatch")
        );
    }

    #[test]
    fn animate_gesture_message_round_trips() {
        let message = OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::AnimateGesture,
            state: None,
            gesture: Some(AgentOverlayGestureEvent {
                event_id: "evt-1".to_string(),
                sequence: 1,
                kind: AgentOverlayGestureKind::Tap,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
                points: vec![Point2 { x: 100.0, y: 200.0 }],
                duration_ms: 250,
                source_action: Some(ActionName::Click),
            }),
            sequence: None,
            reason: None,
            arrival_wait: None,
        };
        let rendered = serde_json::to_value(&message).expect("serialize animate gesture");
        assert_eq!(rendered["kind"], "animate_gesture");
        assert_eq!(rendered["gesture"]["kind"], "tap");
        let round_tripped: OverlayHostMessage =
            serde_json::from_value(rendered).expect("deserialize animate gesture");
        assert_eq!(round_tripped, message);
    }

    #[test]
    fn old_message_without_gesture_field_deserializes() {
        let old = serde_json::json!({
            "version": OVERLAY_HOST_PROTOCOL_VERSION,
            "kind": "set_cursor",
            "state": null
        });
        let message: OverlayHostMessage =
            serde_json::from_value(old).expect("deserialize old message without gesture");
        assert_eq!(message.kind, OverlayHostMessageKind::SetCursor);
        assert!(message.gesture.is_none());
    }

    #[test]
    fn noop_backend_acknowledges_animate_gesture_with_diagnostic() {
        let mut backend = NoopOverlayBackend::default();
        let reply = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::AnimateGesture,
            state: None,
            gesture: Some(tap_gesture("evt-1", 1)),
            sequence: None,
            reason: None,
            arrival_wait: None,
        });
        assert!(reply.ok);
        assert!(
            reply
                .diagnostics
                .iter()
                .any(|entry| entry.code == "OverlayGestureNotSupported")
        );
    }

    #[test]
    fn noop_backend_deduplicates_gesture_event_ids() {
        let mut backend = NoopOverlayBackend::default();
        let first = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::AnimateGesture,
            state: None,
            gesture: Some(tap_gesture("evt-dup", 1)),
            sequence: None,
            reason: None,
            arrival_wait: None,
        });
        assert!(first.ok);

        let duplicate = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::AnimateGesture,
            state: None,
            gesture: Some(tap_gesture("evt-dup", 2)),
            sequence: None,
            reason: None,
            arrival_wait: None,
        });

        assert!(duplicate.ok);
        assert!(
            duplicate
                .diagnostics
                .iter()
                .any(|entry| entry.code == "OverlayGestureDuplicate")
        );
    }

    #[test]
    fn noop_backend_rejects_stale_gesture_sequences() {
        let mut backend = NoopOverlayBackend::default();
        let first = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::AnimateGesture,
            state: None,
            gesture: Some(tap_gesture("evt-new", 4)),
            sequence: None,
            reason: None,
            arrival_wait: None,
        });
        assert!(first.ok);

        let stale = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::AnimateGesture,
            state: None,
            gesture: Some(tap_gesture("evt-old", 3)),
            sequence: None,
            reason: None,
            arrival_wait: None,
        });

        assert!(!stale.ok);
        assert!(
            stale
                .diagnostics
                .iter()
                .any(|entry| entry.code == "OverlayGestureStaleSequence")
        );
    }

    #[test]
    fn noop_backend_rejects_invalid_gesture_shapes() {
        let mut gesture = tap_gesture("evt-bad", 1);
        gesture.points.push(Point2 { x: 2.0, y: 3.0 });
        let mut backend = NoopOverlayBackend::default();

        let reply = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::AnimateGesture,
            state: None,
            gesture: Some(gesture),
            sequence: None,
            reason: None,
            arrival_wait: None,
        });

        assert!(!reply.ok);
        assert!(
            reply
                .diagnostics
                .iter()
                .any(|entry| entry.code == "OverlayGestureInvalidPointCount")
        );
    }

    #[test]
    fn noop_backend_echoes_hide_capture_barrier_sequence() {
        let mut backend = NoopOverlayBackend::default();

        let reply = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Hide,
            state: None,
            gesture: None,
            sequence: Some(42),
            reason: Some("capture".to_string()),
            arrival_wait: None,
        });

        assert!(reply.ok);
        assert_eq!(reply.applied_sequence, Some(42));
    }

    #[test]
    fn probe_reports_no_visible_overlay_backend() {
        let reply = probe_reply();
        let capabilities = reply.capabilities.expect("capabilities");

        assert!(reply.ok);
        assert!(!capabilities.visible_overlay);
        assert!(!capabilities.screenshot_synthetic_cursor);
    }

    #[test]
    fn bundled_cursor_asset_has_expected_chrome_extension_source_dimensions() {
        let image = image::load_from_memory(cursor_asset::AGENT_CURSOR_PNG)
            .expect("decode bundled cursor asset");

        assert_eq!(
            image.dimensions(),
            (
                cursor_asset::AGENT_CURSOR_SOURCE_WIDTH,
                cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT
            )
        );
    }

    #[test]
    fn rendered_cursor_size_matches_browser_css_size() {
        assert_eq!(cursor_asset::AGENT_CURSOR_WIDTH, 23);
        assert_eq!(cursor_asset::AGENT_CURSOR_HEIGHT, 24);
        assert_eq!(cursor_asset::AGENT_CURSOR_HOTSPOT_X, 10);
        assert_eq!(cursor_asset::AGENT_CURSOR_HOTSPOT_Y, 11);
    }

    #[test]
    fn kwin_effect_is_not_a_selectable_visible_backend() {
        let reply = match super::OverlayHostBackend::from_mode("kwin_effect") {
            super::OverlayHostBackend::Noop(mut backend) => {
                backend.handle_message(OverlayHostMessage {
                    version: OVERLAY_HOST_PROTOCOL_VERSION,
                    kind: OverlayHostMessageKind::Capabilities,
                    state: None,
                    gesture: None,
                    sequence: None,
                    reason: None,
                    arrival_wait: None,
                })
            }
            #[cfg(target_os = "linux")]
            other => panic!("expected noop backend, got {other:?}"),
        };

        let capabilities = reply.capabilities.expect("capabilities");
        assert_eq!(capabilities.backend, AgentCursorBackendKind::None);
        assert!(
            capabilities
                .reason
                .expect("reason")
                .contains("no longer a selectable visible overlay backend")
        );
    }

    #[test]
    fn legacy_gnome_and_x11_modes_report_noop_capabilities() {
        for (mode, reason) in [
            (
                "gnome_shell_extension",
                "GNOME Shell visual rendering was retired",
            ),
            ("x11", "X11 visible overlay requires a WGPU X11 host"),
        ] {
            let reply = match super::OverlayHostBackend::from_mode(mode) {
                super::OverlayHostBackend::Noop(mut backend) => {
                    backend.handle_message(OverlayHostMessage {
                        version: OVERLAY_HOST_PROTOCOL_VERSION,
                        kind: OverlayHostMessageKind::Capabilities,
                        state: None,
                        gesture: None,
                        sequence: None,
                        reason: None,
                        arrival_wait: None,
                    })
                }
                #[cfg(target_os = "linux")]
                other => panic!("expected noop backend, got {other:?}"),
            };

            let capabilities = reply.capabilities.expect("capabilities");
            assert_eq!(capabilities.backend, AgentCursorBackendKind::None);
            assert_eq!(
                capabilities.renderer_backend,
                sky_cua_platform::model::AgentCursorRendererBackendKind::None
            );
            assert!(!capabilities.visible_overlay);
            assert_eq!(capabilities.active_output_count, Some(0));
            assert_eq!(capabilities.rendered_output_count, Some(0));
            assert!(capabilities.reason.expect("reason").contains(reason));
        }
    }

    fn cursor_state() -> AgentCursorState {
        AgentCursorState {
            visible: true,
            sequence: 1,
            model_point: Some(AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: Some("stream".to_string()),
            }),
            native_point: None,
            snapshot_id: Some("snap".to_string()),
            source_action: Some(ActionName::Click),
            updated_at_ms: 42,
        }
    }

    fn tap_gesture(event_id: &str, sequence: u64) -> AgentOverlayGestureEvent {
        AgentOverlayGestureEvent {
            event_id: event_id.to_string(),
            sequence,
            kind: AgentOverlayGestureKind::Tap,
            coordinate_space: CoordinateSpace::DesktopLogical,
            mapping_id: None,
            points: vec![Point2 { x: 100.0, y: 200.0 }],
            duration_ms: 250,
            source_action: Some(ActionName::Click),
        }
    }
}
