use std::ffi::OsStr;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorState,
    AgentCursorSystemCursorBackendKind,
};

use crate::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    diagnostic, error_reply,
};

const KWIN_SERVICE: &str = "org.kde.KWin";
const KWIN_EFFECTS_PATH: &str = "/Effects";
const KWIN_EFFECTS_INTERFACE: &str = "org.kde.kwin.Effects";
const KWIN_AGENT_CURSOR_PATH: &str = "/com/skycua/AgentCursor";
const KWIN_AGENT_CURSOR_INTERFACE: &str = "com.skycua.AgentCursor";
const KWIN_EFFECT_ID: &str = "sky-cua-agent-cursor";

#[derive(Debug, Clone)]
pub struct KwinEffectOverlayBackend {
    qdbus: String,
    state: Option<AgentCursorState>,
}

impl KwinEffectOverlayBackend {
    pub fn connect() -> Result<Self> {
        let qdbus = find_qdbus().ok_or_else(|| anyhow!("qdbus6 or qdbus is not available"))?;
        let backend = Self { qdbus, state: None };
        if !backend.effect_loaded()? {
            bail!("KWin effect {KWIN_EFFECT_ID} is not loaded");
        }
        Ok(backend)
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

        let hide_reason = if message.kind == OverlayHostMessageKind::Hide {
            message
                .reason
                .clone()
                .filter(|value| !value.trim().is_empty())
        } else {
            None
        };
        let result = match message.kind {
            OverlayHostMessageKind::Hello
            | OverlayHostMessageKind::Ping
            | OverlayHostMessageKind::Shutdown
            | OverlayHostMessageKind::Capabilities => Ok(()),
            OverlayHostMessageKind::SetCursor => self.set_cursor(message.state),
            OverlayHostMessageKind::Hide => self.hide(message.reason),
            OverlayHostMessageKind::Show => self.show(message.state),
        };

        match result {
            Ok(()) => {
                // Diagnostic parity with the other visible backends.
                let diagnostics = hide_reason.map_or_else(Vec::new, |reason| {
                    vec![diagnostic(
                        "OverlayCursorHidden",
                        "Overlay host hid the cursor.",
                        Some(reason),
                    )]
                });
                self.reply(true, diagnostics)
            }
            Err(error) => OverlayHostReply {
                version: OVERLAY_HOST_PROTOCOL_VERSION,
                ok: false,
                capabilities: Some(self.capabilities()),
                state: self.state.clone(),
                diagnostics: vec![diagnostic(
                    "KwinEffectOverlayCommandFailed",
                    "KWin effect overlay command failed.",
                    Some(error.to_string()),
                )],
            },
        }
    }

    fn set_cursor(&mut self, state: Option<AgentCursorState>) -> Result<()> {
        let Some(state) = state else {
            self.call_agent_cursor_method("Hide", std::iter::empty::<&str>())?;
            self.state = None;
            return Ok(());
        };
        let state_json = serde_json::to_string(&state).context("serialize agent cursor state")?;
        let output =
            self.call_agent_cursor_method("SetCursorState", std::iter::once(state_json.as_str()))?;
        if !qdbus_bool(&output) {
            bail!("KWin effect rejected SetCursorState");
        }
        self.state = Some(state);
        Ok(())
    }

    fn hide(&mut self, _reason: Option<String>) -> Result<()> {
        self.call_agent_cursor_method("Hide", std::iter::empty::<&str>())?;
        if let Some(state) = self.state.as_mut() {
            state.visible = false;
        }
        Ok(())
    }

    fn show(&mut self, state: Option<AgentCursorState>) -> Result<()> {
        if let Some(mut state) = state {
            state.visible = true;
            return self.set_cursor(Some(state));
        }
        self.call_agent_cursor_method("Show", std::iter::empty::<&str>())?;
        if let Some(state) = self.state.as_mut() {
            state.visible = true;
        }
        Ok(())
    }

    fn effect_loaded(&self) -> Result<bool> {
        let output = self.call_effects_method("isEffectLoaded", [KWIN_EFFECT_ID])?;
        Ok(qdbus_bool(&output))
    }

    fn call_effects_method<I, S>(&self, method: &str, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.call_qdbus(
            KWIN_SERVICE,
            KWIN_EFFECTS_PATH,
            format!("{KWIN_EFFECTS_INTERFACE}.{method}"),
            args,
        )
    }

    fn call_agent_cursor_method<I, S>(&self, method: &str, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.call_qdbus(
            KWIN_SERVICE,
            KWIN_AGENT_CURSOR_PATH,
            format!("{KWIN_AGENT_CURSOR_INTERFACE}.{method}"),
            args,
        )
    }

    fn call_qdbus<I, S>(
        &self,
        service: &str,
        object_path: &str,
        method: String,
        args: I,
    ) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = Command::new(&self.qdbus)
            .arg(service)
            .arg(object_path)
            .arg(method)
            .args(args)
            .output()
            .with_context(|| format!("failed to run {}", self.qdbus))?;
        if !output.status.success() {
            bail!(
                "{} exited with status {}: {}",
                self.qdbus,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn capabilities(&self) -> AgentCursorCapabilities {
        AgentCursorCapabilities {
            backend: AgentCursorBackendKind::KwinEffect,
            visible_overlay: true,
            screenshot_synthetic_cursor: false,
            click_through: true,
            capture_exclusion: false,
            system_cursor_hide_supported: true,
            system_cursor_hidden: self.state.as_ref().is_some_and(|state| state.visible),
            system_cursor_backend: AgentCursorSystemCursorBackendKind::KwinEffect,
            needs_user_install: true,
            reason: Some(
                "KWin effect overlay is active through com.skycua.AgentCursor DBus bridge"
                    .to_string(),
            ),
        }
    }

    fn reply(
        &self,
        ok: bool,
        diagnostics: Vec<sky_cua_platform::model::DiagnosticEntry>,
    ) -> OverlayHostReply {
        OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok,
            capabilities: Some(self.capabilities()),
            state: self.state.clone(),
            diagnostics,
        }
    }
}

fn qdbus_bool(stdout: &str) -> bool {
    stdout.lines().next().is_some_and(|line| {
        matches!(
            line.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes"
        )
    })
}

fn find_qdbus() -> Option<String> {
    ["qdbus6", "qdbus"]
        .into_iter()
        .find(|candidate| command_exists(candidate))
        .map(str::to_string)
}

fn command_exists(command: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let path = dir.join(command);
            path.is_file()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::qdbus_bool;

    #[test]
    fn parses_qdbus_bool_output() {
        assert!(qdbus_bool("true\n"));
        assert!(qdbus_bool("1\n"));
        assert!(qdbus_bool("YES\n"));
        assert!(!qdbus_bool("false\n"));
        assert!(!qdbus_bool(""));
    }
}
