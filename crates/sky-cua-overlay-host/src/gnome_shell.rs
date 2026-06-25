use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPointerTrackingBackendKind,
    AgentCursorRendererBackendKind, AgentCursorState, AgentCursorSystemCursorBackendKind,
    DiagnosticEntry,
};

use crate::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    diagnostic,
};

const SERVICE_NAME: &str = "com.openai.Codex.WindowControl";
const OBJECT_PATH: &str = "/com/openai/Codex/WindowControl";
const BACKEND_REASON: &str =
    "GNOME Shell extension overlay is active through com.openai.Codex.WindowControl";

#[derive(Debug, Default)]
pub struct GnomeShellOverlayBackend {
    state: Option<AgentCursorState>,
    visible: bool,
    system_cursor_hidden: bool,
    last_status: Option<GnomeAgentCursorStatus>,
    reason: Option<String>,
}

impl GnomeShellOverlayBackend {
    pub fn connect() -> Result<Self> {
        let status = agent_cursor_status()
            .context("GNOME Shell extension agent cursor DBus API unavailable")?;
        Ok(Self {
            visible: status.visible,
            system_cursor_hidden: status.system_cursor_hidden,
            last_status: Some(status),
            state: None,
            reason: None,
        })
    }

    pub fn handle_message(&mut self, message: OverlayHostMessage) -> OverlayHostReply {
        if message.version != OVERLAY_HOST_PROTOCOL_VERSION {
            return self.reply(
                false,
                vec![diagnostic(
                    "OverlayProtocolVersionMismatch",
                    "Overlay host protocol version mismatch.",
                    Some(format!(
                        "expected={} got={}",
                        OVERLAY_HOST_PROTOCOL_VERSION, message.version
                    )),
                )],
            );
        }

        match message.kind {
            OverlayHostMessageKind::Hello
            | OverlayHostMessageKind::Ping
            | OverlayHostMessageKind::Capabilities
            | OverlayHostMessageKind::AnimateGesture => {
                self.refresh_status();
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::Shutdown => {
                let diagnostics = self.show_or_diagnostic("shutdown");
                self.reply(diagnostics.is_empty(), diagnostics)
            }
            OverlayHostMessageKind::SetCursor => {
                self.state = message.state;
                self.set_state_reply()
            }
            OverlayHostMessageKind::Hide => {
                if let Some(state) = self.state.as_mut() {
                    state.visible = false;
                }
                let mut diagnostics = self.show_or_diagnostic(
                    message
                        .reason
                        .as_deref()
                        .unwrap_or("overlay hide requested"),
                );
                let ok = diagnostics.is_empty();
                if let Some(reason) = message.reason.filter(|value| !value.trim().is_empty()) {
                    diagnostics.push(diagnostic(
                        "OverlayCursorHidden",
                        "Overlay host hid the cursor.",
                        Some(reason),
                    ));
                }
                self.reply(ok, diagnostics)
            }
            OverlayHostMessageKind::Show => {
                self.state = message.state;
                if let Some(state) = self.state.as_mut() {
                    state.visible = true;
                }
                self.set_state_reply()
            }
        }
    }

    fn set_state_reply(&mut self) -> OverlayHostReply {
        let Some(state) = self.state.as_ref() else {
            let diagnostics = self.show_or_diagnostic("agent cursor state cleared");
            return self.reply(diagnostics.is_empty(), diagnostics);
        };
        match set_agent_cursor_state(state) {
            Ok(result) if result.ok => {
                self.apply_status(result.status);
                self.reason = Some(result.message);
                self.reply(true, Vec::new())
            }
            Ok(result) => {
                self.apply_status(result.status);
                self.reason = Some(result.message.clone());
                self.reply(
                    false,
                    vec![diagnostic(
                        "GnomeShellAgentCursorRejected",
                        "GNOME Shell extension rejected the agent cursor state.",
                        Some(result.message),
                    )],
                )
            }
            Err(error) => {
                self.reason = Some(error.to_string());
                self.reply(
                    false,
                    vec![diagnostic(
                        "GnomeShellAgentCursorFailed",
                        "GNOME Shell extension failed to update the agent cursor.",
                        Some(error.to_string()),
                    )],
                )
            }
        }
    }

    fn show_or_diagnostic(&mut self, reason: &str) -> Vec<DiagnosticEntry> {
        match hide_agent_cursor(reason) {
            Ok(result) if result.ok => {
                self.apply_status(result.status);
                self.reason = Some(result.message);
                Vec::new()
            }
            Ok(result) => {
                self.apply_status(result.status);
                self.reason = Some(result.message.clone());
                vec![diagnostic(
                    "GnomeShellAgentCursorRejected",
                    "GNOME Shell extension rejected the agent cursor hide request.",
                    Some(result.message),
                )]
            }
            Err(error) => {
                self.reason = Some(error.to_string());
                vec![diagnostic(
                    "GnomeShellAgentCursorFailed",
                    "GNOME Shell extension failed to hide the agent cursor.",
                    Some(error.to_string()),
                )]
            }
        }
    }

    fn refresh_status(&mut self) {
        if let Ok(status) = agent_cursor_status() {
            self.apply_status(status);
        }
    }

    fn apply_status(&mut self, status: GnomeAgentCursorStatus) {
        self.visible = status.visible;
        self.system_cursor_hidden = status.system_cursor_hidden;
        self.last_status = Some(status);
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

    fn capabilities(&self) -> AgentCursorCapabilities {
        AgentCursorCapabilities {
            backend: AgentCursorBackendKind::GnomeShellExtension,
            renderer_backend: AgentCursorRendererBackendKind::None,
            visible_overlay: self.visible,
            screenshot_synthetic_cursor: false,
            click_through: true,
            capture_exclusion: false,
            pointer_tracking_backend: AgentCursorPointerTrackingBackendKind::None,
            pointer_tracking_exact: false,
            system_cursor_hide_supported: self
                .last_status
                .as_ref()
                .is_some_and(|status| status.system_cursor_hide_supported),
            system_cursor_hidden: self.system_cursor_hidden,
            system_cursor_backend: AgentCursorSystemCursorBackendKind::GnomeShellExtension,
            needs_user_install: false,
            reason: Some(
                self.reason
                    .clone()
                    .unwrap_or_else(|| BACKEND_REASON.to_string()),
            ),
            ..Default::default()
        }
    }
}

impl Drop for GnomeShellOverlayBackend {
    fn drop(&mut self) {
        let _ = hide_agent_cursor("overlay host dropped");
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct GnomeAgentCursorStatus {
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    system_cursor_hide_supported: bool,
    #[serde(default)]
    system_cursor_hidden: bool,
}

#[derive(Debug, Clone)]
struct GnomeAgentCursorResult {
    ok: bool,
    message: String,
    status: GnomeAgentCursorStatus,
}

fn agent_cursor_status() -> Result<GnomeAgentCursorStatus> {
    let output = gdbus_call("AgentCursorStatus", &[])?;
    let json = quoted_gdbus_strings(&output)
        .into_iter()
        .next()
        .context("GNOME Shell extension AgentCursorStatus did not return status JSON")?;
    serde_json::from_str(&json).context("GNOME Shell extension returned invalid status JSON")
}

fn set_agent_cursor_state(state: &AgentCursorState) -> Result<GnomeAgentCursorResult> {
    let json = serde_json::to_string(state).context("serialize agent cursor state")?;
    let output = gdbus_call("SetAgentCursorState", &[json.as_str()])?;
    parse_result_tuple(&output)
}

fn hide_agent_cursor(reason: &str) -> Result<GnomeAgentCursorResult> {
    let output = gdbus_call("HideAgentCursor", &[reason])?;
    parse_result_tuple(&output)
}

fn gdbus_call(method: &str, args: &[&str]) -> Result<String> {
    let variant_args = args
        .iter()
        .map(|arg| gvariant_string_arg(arg))
        .collect::<Vec<_>>();
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            SERVICE_NAME,
            "--object-path",
            OBJECT_PATH,
            "--method",
        ])
        .arg(format!("{SERVICE_NAME}.{method}"))
        .args(&variant_args)
        .output()
        .context("failed to run gdbus")?;
    if !output.status.success() {
        bail!(
            "gdbus {} failed: {}",
            method,
            command_detail(&output.stdout, &output.stderr)
        );
    }
    String::from_utf8(output.stdout).context("gdbus output was not UTF-8")
}

fn gvariant_string_arg(value: &str) -> String {
    let mut rendered = String::with_capacity(value.len() + 2);
    rendered.push('\'');
    for ch in value.chars() {
        if ch == '\'' || ch == '\\' {
            rendered.push('\\');
        }
        rendered.push(ch);
    }
    rendered.push('\'');
    rendered
}

fn parse_result_tuple(output: &str) -> Result<GnomeAgentCursorResult> {
    let ok = output.trim_start().starts_with("(true,");
    let strings = quoted_gdbus_strings(output);
    let message = strings.first().cloned().unwrap_or_default();
    let status_json = strings
        .get(1)
        .context("GNOME Shell extension result did not include status JSON")?;
    let status = serde_json::from_str(status_json)
        .context("GNOME Shell extension returned invalid status JSON")?;
    Ok(GnomeAgentCursorResult {
        ok,
        message,
        status,
    })
}

fn quoted_gdbus_strings(output: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut chars = output.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\'' {
            continue;
        }
        let mut value = String::new();
        let mut escaped = false;
        for next in chars.by_ref() {
            if escaped {
                value.push(next);
                escaped = false;
            } else if next == '\\' {
                escaped = true;
            } else if next == '\'' {
                break;
            } else {
                value.push(next);
            }
        }
        strings.push(value);
    }
    strings
}

fn command_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    if stdout.is_empty() {
        "no output".to_string()
    } else {
        stdout
    }
}

#[cfg(test)]
mod tests {
    use super::{gvariant_string_arg, parse_result_tuple, quoted_gdbus_strings};

    #[test]
    fn parses_gdbus_result_tuple() {
        let parsed = parse_result_tuple("(true, 'updated', '{\"visible\":true,\"system_cursor_hide_supported\":true,\"system_cursor_hidden\":true}')")
            .expect("parse tuple");

        assert!(parsed.ok);
        assert_eq!(parsed.message, "updated");
        assert!(parsed.status.visible);
        assert!(parsed.status.system_cursor_hide_supported);
        assert!(parsed.status.system_cursor_hidden);
    }

    #[test]
    fn extracts_quoted_gdbus_strings_with_escapes() {
        assert_eq!(
            quoted_gdbus_strings("(true, 'it\\'s ok', '{\"visible\":false}')"),
            vec!["it's ok".to_string(), "{\"visible\":false}".to_string()]
        );
    }

    #[test]
    fn renders_gvariant_string_arguments_for_gdbus() {
        assert_eq!(
            gvariant_string_arg(r#"{"reason":"it's \ ok"}"#),
            r#"'{"reason":"it\'s \\ ok"}'"#
        );
    }
}
