use serde::{Deserialize, Serialize};
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorState,
    AgentCursorSystemCursorBackendKind, DiagnosticEntry,
};

#[cfg(target_os = "linux")]
mod gnome_shell;
#[cfg(target_os = "linux")]
mod kwin_effect;
#[cfg(target_os = "linux")]
mod layer_shell;
mod system_cursor;
#[cfg(target_os = "linux")]
mod x11;

pub const OVERLAY_HOST_PROTOCOL_VERSION: u32 = 1;
const OVERLAY_BACKEND_ENV: &str = "SKY_CUA_OVERLAY_BACKEND";

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
    // cursor at 2x the browser/synthetic size for on-screen legibility: the full
    // 46x48 source rendered 1:1, with a doubled hotspot. The screenshot-synthetic
    // and phone cursor planes keep the base 23x24 size above.
    pub const AGENT_CURSOR_DESKTOP_WIDTH: u32 = 46;
    pub const AGENT_CURSOR_DESKTOP_HEIGHT: u32 = 48;
    pub const AGENT_CURSOR_DESKTOP_HOTSPOT_X: i32 = 20;
    pub const AGENT_CURSOR_DESKTOP_HOTSPOT_Y: i32 = 22;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayHostMessage {
    pub version: u32,
    pub kind: OverlayHostMessageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AgentCursorState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayHostReply {
    pub version: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<AgentCursorCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AgentCursorState>,
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct NoopOverlayBackend {
    state: Option<AgentCursorState>,
    reason: Option<String>,
}

impl NoopOverlayBackend {
    #[must_use]
    pub fn with_reason(reason: impl Into<String>) -> Self {
        Self {
            state: None,
            reason: Some(reason.into()),
        }
    }

    #[must_use]
    pub fn default_capabilities() -> AgentCursorCapabilities {
        AgentCursorCapabilities {
            backend: AgentCursorBackendKind::None,
            visible_overlay: false,
            screenshot_synthetic_cursor: false,
            click_through: false,
            capture_exclusion: false,
            system_cursor_hide_supported: false,
            system_cursor_hidden: false,
            system_cursor_backend: AgentCursorSystemCursorBackendKind::None,
            needs_user_install: false,
            reason: Some("no visible overlay backend selected".to_string()),
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
            OverlayHostMessageKind::SetCursor => {
                self.state = message.state;
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::Hide => {
                if let Some(state) = self.state.as_mut() {
                    state.visible = false;
                }
                self.reply(
                    true,
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
        OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok,
            capabilities: Some(self.capabilities()),
            state: self.state.clone(),
            diagnostics,
        }
    }
}

#[derive(Debug)]
pub enum OverlayHostBackend {
    Noop(NoopOverlayBackend),
    #[cfg(target_os = "linux")]
    GnomeShell(gnome_shell::GnomeShellOverlayBackend),
    #[cfg(target_os = "linux")]
    KwinEffect(kwin_effect::KwinEffectOverlayBackend),
    #[cfg(target_os = "linux")]
    LayerShell(Box<layer_shell::LayerShellOverlayBackend>),
    #[cfg(target_os = "linux")]
    X11(Box<x11::X11OverlayBackend>),
}

impl OverlayHostBackend {
    #[must_use]
    pub fn from_env() -> Self {
        let mode = std::env::var(OVERLAY_BACKEND_ENV)
            .unwrap_or_else(|_| "auto".to_string())
            .trim()
            .to_ascii_lowercase();

        if matches!(mode.as_str(), "none" | "never" | "off" | "false" | "0") {
            return Self::Noop(NoopOverlayBackend::with_reason(format!(
                "{OVERLAY_BACKEND_ENV}={mode}"
            )));
        }
        if matches!(mode.as_str(), "noop" | "no-op") {
            return Self::Noop(NoopOverlayBackend::with_reason(format!(
                "{OVERLAY_BACKEND_ENV}={mode}"
            )));
        }

        #[cfg(target_os = "linux")]
        {
            if matches!(
                mode.as_str(),
                "gnome"
                    | "gnome-shell"
                    | "gnome_shell"
                    | "gnome-shell-extension"
                    | "gnome_shell_extension"
            ) {
                return match gnome_shell::GnomeShellOverlayBackend::connect() {
                    Ok(backend) => Self::GnomeShell(backend),
                    Err(error) => Self::Noop(NoopOverlayBackend::with_reason(format!(
                        "GNOME Shell extension overlay unavailable: {error}"
                    ))),
                };
            }
            if matches!(
                mode.as_str(),
                "kwin" | "kwin-effect" | "kwin_effect" | "kde-kwin-effect" | "kde_kwin_effect"
            ) {
                return match kwin_effect::KwinEffectOverlayBackend::connect() {
                    Ok(backend) => Self::KwinEffect(backend),
                    Err(error) => Self::Noop(NoopOverlayBackend::with_reason(format!(
                        "KWin effect overlay unavailable: {error}"
                    ))),
                };
            }
            if matches!(
                mode.as_str(),
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
                mode.as_str(),
                "x11" | "x11-shaped" | "x11_shaped" | "x11-shaped-window" | "x11_shaped_window"
            ) {
                return match x11::X11OverlayBackend::connect() {
                    Ok(backend) => Self::X11(Box::new(backend)),
                    Err(error) => Self::Noop(NoopOverlayBackend::with_reason(format!(
                        "X11 shaped-window overlay unavailable: {error}"
                    ))),
                };
            }
            if matches!(mode.as_str(), "auto" | "") {
                let mut reasons = Vec::new();
                match kwin_effect::KwinEffectOverlayBackend::connect() {
                    Ok(backend) => return Self::KwinEffect(backend),
                    Err(error) => {
                        reasons.push(format!("KWin effect overlay unavailable: {error}"));
                    }
                }
                if is_gnome_session() {
                    match gnome_shell::GnomeShellOverlayBackend::connect() {
                        Ok(backend) => return Self::GnomeShell(backend),
                        Err(error) => {
                            reasons.push(format!(
                                "GNOME Shell extension overlay unavailable: {error}"
                            ));
                        }
                    }
                }
                if linux_env_value("WAYLAND_DISPLAY").is_some() {
                    match layer_shell::LayerShellOverlayBackend::connect() {
                        Ok(backend) => return Self::LayerShell(Box::new(backend)),
                        Err(error) => {
                            reasons.push(format!("wayland layer-shell unavailable: {error}"));
                        }
                    }
                }
                if should_try_x11_auto(
                    linux_env_value("XDG_SESSION_TYPE").as_deref(),
                    linux_env_value("WAYLAND_DISPLAY").as_deref(),
                    linux_env_value("DISPLAY").as_deref(),
                ) {
                    match x11::X11OverlayBackend::connect() {
                        Ok(backend) => return Self::X11(Box::new(backend)),
                        Err(error) => {
                            reasons.push(format!("X11 shaped-window overlay unavailable: {error}"));
                        }
                    }
                }
                if reasons.is_empty() {
                    reasons.push(
                        "no auto-selected visible overlay backend for this session".to_string(),
                    );
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
            Self::GnomeShell(backend) => backend.handle_message(message),
            #[cfg(target_os = "linux")]
            Self::KwinEffect(backend) => backend.handle_message(message),
            #[cfg(target_os = "linux")]
            Self::LayerShell(backend) => backend.handle_message(message),
            #[cfg(target_os = "linux")]
            Self::X11(backend) => backend.handle_message(message),
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(target_os = "linux")]
fn is_gnome_session() -> bool {
    [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ]
    .into_iter()
    .filter_map(linux_env_value)
    .flat_map(|value| {
        value
            .split([':', ';'])
            .map(|part| part.trim().to_ascii_lowercase())
            .collect::<Vec<_>>()
    })
    .any(|part| part == "gnome")
}

#[cfg(target_os = "linux")]
fn should_try_x11_auto(
    session_type: Option<&str>,
    wayland_display: Option<&str>,
    display: Option<&str>,
) -> bool {
    let Some(display) = display else {
        return false;
    };
    if display.trim().is_empty() {
        return false;
    }
    if session_type.is_some_and(|value| value.trim().eq_ignore_ascii_case("x11")) {
        return true;
    }
    wayland_display.is_none_or(|value| value.trim().is_empty())
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
        reason: None,
    })
}

#[must_use]
fn noop_probe_reply(capabilities: AgentCursorCapabilities) -> OverlayHostReply {
    OverlayHostReply {
        version: OVERLAY_HOST_PROTOCOL_VERSION,
        ok: true,
        capabilities: Some(capabilities),
        state: None,
        diagnostics: Vec::new(),
    }
}

fn error_reply(code: &str, message: &str, details: Option<String>) -> OverlayHostReply {
    OverlayHostReply {
        version: OVERLAY_HOST_PROTOCOL_VERSION,
        ok: false,
        capabilities: Some(NoopOverlayBackend::default_capabilities()),
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
    use super::{
        NoopOverlayBackend, OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage,
        OverlayHostMessageKind, cursor_asset, probe_reply,
    };
    use image::GenericImageView;
    use sky_cua_platform::model::{
        ActionName, AgentCursorPoint, AgentCursorState, CoordinateSpace,
    };

    #[test]
    fn protocol_messages_use_snake_case_kind_values() {
        let message = OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            reason: None,
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
    fn noop_backend_records_latest_state_and_toggles_visibility() {
        let mut backend = NoopOverlayBackend::default();
        let set = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            reason: None,
        });
        assert!(set.ok);
        assert!(set.state.as_ref().expect("state").visible);

        let hidden = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Hide,
            state: None,
            reason: Some("capture".to_string()),
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
            reason: None,
        });
        assert!(shown.state.expect("state").visible);

        let cleared = backend.handle_message(OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Show,
            state: None,
            reason: None,
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
            reason: None,
        });

        assert!(!reply.ok);
        assert_eq!(
            reply.diagnostics.first().map(|entry| entry.code.as_str()),
            Some("OverlayProtocolVersionMismatch")
        );
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

    #[cfg(target_os = "linux")]
    #[test]
    fn auto_backend_uses_x11_only_for_x11_sessions() {
        assert!(super::should_try_x11_auto(
            Some("x11"),
            Some("wayland-0"),
            Some(":0")
        ));
        assert!(super::should_try_x11_auto(None, None, Some(":0")));
        assert!(!super::should_try_x11_auto(
            Some("wayland"),
            Some("wayland-0"),
            Some(":0")
        ));
        assert!(!super::should_try_x11_auto(Some("x11"), None, None));
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
}
