use serde::{Deserialize, Serialize};
use sky_cua_platform::model::{RectF, TerminalProcessInfo, TerminalWindowInfo, WindowInfo};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinuxWindowInfo {
    pub window_id: String,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub wm_class: Option<String>,
    pub pid: Option<u32>,
    pub bounds: Option<RectF>,
    pub workspace: Option<i32>,
    pub focused: bool,
    pub hidden: bool,
    pub client_type: Option<String>,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalWindowContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wm_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalWindowContext {
    pub tty: String,
    pub root_process: TerminalProcess,
    pub active_process: Option<TerminalProcess>,
    pub process_count: usize,
    pub confidence: String,
    pub match_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalProcess {
    pub pid: u32,
    pub command_name: String,
    pub command_line: String,
    pub cwd: Option<String>,
}

impl WindowTarget {
    pub fn has_target(&self) -> bool {
        self.window_id.as_deref().is_some_and(non_empty)
            || self.pid.is_some_and(non_zero)
            || self.has_terminal_target()
            || self.app_id.as_deref().is_some_and(non_empty)
            || self.wm_class.as_deref().is_some_and(non_empty)
            || self.title.as_deref().is_some_and(non_empty)
    }

    pub fn has_terminal_target(&self) -> bool {
        self.terminal_pid.is_some_and(non_zero)
            || self.tty.as_deref().is_some_and(non_empty)
            || self.terminal_command.as_deref().is_some_and(non_empty)
            || self.terminal_cwd.as_deref().is_some_and(non_empty)
    }
}

impl From<TerminalProcess> for TerminalProcessInfo {
    fn from(process: TerminalProcess) -> Self {
        Self {
            pid: process.pid,
            command_name: process.command_name,
            command_line: process.command_line,
            cwd: process.cwd,
        }
    }
}

impl From<TerminalWindowContext> for TerminalWindowInfo {
    fn from(terminal: TerminalWindowContext) -> Self {
        Self {
            tty: terminal.tty,
            root_process: terminal.root_process.into(),
            active_process: terminal.active_process.map(Into::into),
            process_count: terminal.process_count,
            confidence: terminal.confidence,
            match_reason: terminal.match_reason,
        }
    }
}

impl From<LinuxWindowInfo> for WindowInfo {
    fn from(window: LinuxWindowInfo) -> Self {
        Self {
            window_id: window.window_id,
            title: window.title,
            app_id: window.app_id,
            wm_class: window.wm_class,
            pid: window.pid,
            bounds: window.bounds,
            workspace: window.workspace,
            focused: window.focused,
            hidden: window.hidden,
            client_type: window.client_type,
            backend: window.backend,
            terminal: window.terminal.map(Into::into),
        }
    }
}

impl From<sky_cua_platform::model::WindowTarget> for WindowTarget {
    fn from(mut target: sky_cua_platform::model::WindowTarget) -> Self {
        target.normalize_empty_fields();
        Self {
            window_id: target.window_id,
            pid: target.pid,
            tty: target.tty,
            terminal_pid: target.terminal_pid,
            terminal_command: target.terminal_command,
            terminal_cwd: target.terminal_cwd,
            app_id: target.app_id,
            wm_class: target.wm_class,
            title: target.title,
        }
    }
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn non_zero(value: u32) -> bool {
    value != 0
}

#[cfg(test)]
mod tests {
    use super::WindowTarget;

    #[test]
    fn platform_window_target_conversion_normalizes_empty_defaults() {
        let target = sky_cua_platform::model::WindowTarget {
            window_id: Some(" ".to_string()),
            pid: Some(0),
            tty: Some("".to_string()),
            terminal_pid: Some(0),
            terminal_command: Some("\t".to_string()),
            terminal_cwd: Some("".to_string()),
            app_id: Some(" chromium.desktop ".to_string()),
            wm_class: Some("".to_string()),
            title: Some("".to_string()),
        };

        let target = WindowTarget::from(target);

        assert_eq!(target.app_id.as_deref(), Some("chromium.desktop"));
        assert_eq!(target.pid, None);
        assert_eq!(target.terminal_pid, None);
        assert!(!target.has_terminal_target());
    }

    #[test]
    fn zero_process_ids_do_not_count_as_local_targets() {
        let target = WindowTarget {
            pid: Some(0),
            terminal_pid: Some(0),
            ..WindowTarget::default()
        };

        assert!(!target.has_target());
        assert!(!target.has_terminal_target());
    }
}
