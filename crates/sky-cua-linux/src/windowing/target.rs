use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

use super::types::{LinuxWindowInfo, TerminalProcess, WindowTarget};

pub fn resolve_window_target<'a>(
    windows: &'a [LinuxWindowInfo],
    target: &WindowTarget,
) -> Result<&'a LinuxWindowInfo, BackendError> {
    if let Some(window_id) = normalized_target(target.window_id.as_deref()) {
        let matches = windows
            .iter()
            .filter(|window| window.window_id == window_id)
            .collect::<Vec<_>>();
        return unique_window_match(matches, &format!("window_id {window_id}"));
    }

    if target.has_terminal_target() {
        let matches = windows
            .iter()
            .filter(|window| window_matches_terminal_target(window, target))
            .filter(|window| target.pid.is_none_or(|pid| window.pid == Some(pid)))
            .filter(|window| optional_exact_match(&window.app_id, target.app_id.as_deref()))
            .filter(|window| optional_exact_match(&window.wm_class, target.wm_class.as_deref()))
            .filter(|window| optional_title_match(&window.title, target.title.as_deref()))
            .collect::<Vec<_>>();
        return unique_window_match(matches, "terminal target");
    }

    if let Some(pid) = target.pid {
        let matches = windows
            .iter()
            .filter(|window| window.pid == Some(pid))
            .collect::<Vec<_>>();
        return unique_window_match(matches, &format!("pid {pid}"));
    }

    if let Some(app_id) = normalized_target(target.app_id.as_deref()) {
        let matches = windows
            .iter()
            .filter(|window| {
                window
                    .app_id
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&app_id))
            })
            .collect::<Vec<_>>();
        return unique_window_match(matches, &format!("app_id {app_id}"));
    }

    if let Some(wm_class) = normalized_target(target.wm_class.as_deref()) {
        let matches = windows
            .iter()
            .filter(|window| {
                window
                    .wm_class
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&wm_class))
            })
            .collect::<Vec<_>>();
        return unique_window_match(matches, &format!("wm_class {wm_class}"));
    }

    if let Some(title) = normalized_target(target.title.as_deref()) {
        let title_lower = title.to_ascii_lowercase();
        let matches = windows
            .iter()
            .filter(|window| {
                window
                    .title
                    .as_deref()
                    .is_some_and(|value| value.to_ascii_lowercase().contains(&title_lower))
            })
            .collect::<Vec<_>>();
        return unique_window_match(matches, &format!("title containing {title}"));
    }

    Err(invalid(
        "Pass window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd to target a window.",
    ))
}

fn unique_window_match<'a>(
    matches: Vec<&'a LinuxWindowInfo>,
    description: &str,
) -> Result<&'a LinuxWindowInfo, BackendError> {
    match matches.as_slice() {
        [window] => Ok(*window),
        [] => Err(invalid(format!("No window matched {description}."))),
        windows => {
            let ids = windows
                .iter()
                .map(|window| window.window_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(invalid(format!(
                "{description} matched multiple windows ({ids}); add window_id, tty, title, or terminal_command to disambiguate."
            )))
        }
    }
}

fn window_matches_terminal_target(window: &LinuxWindowInfo, target: &WindowTarget) -> bool {
    let Some(terminal) = &window.terminal else {
        return false;
    };

    if let Some(tty) = normalized_target(target.tty.as_deref())
        && !tty_matches(&terminal.tty, &tty)
    {
        return false;
    }

    if let Some(pid) = target.terminal_pid {
        let active_pid = terminal.active_process.as_ref().map(|process| process.pid);
        if active_pid != Some(pid) && terminal.root_process.pid != pid {
            return false;
        }
    }

    if let Some(command) = normalized_target(target.terminal_command.as_deref()) {
        let command = command.to_ascii_lowercase();
        let active_matches = terminal
            .active_process
            .as_ref()
            .is_some_and(|process| terminal_process_matches_command(process, &command));
        if !active_matches && !terminal_process_matches_command(&terminal.root_process, &command) {
            return false;
        }
    }

    if let Some(cwd) = normalized_target(target.terminal_cwd.as_deref()) {
        let active_matches = terminal
            .active_process
            .as_ref()
            .is_some_and(|process| terminal_process_matches_cwd(process, &cwd));
        if !active_matches && !terminal_process_matches_cwd(&terminal.root_process, &cwd) {
            return false;
        }
    }

    true
}

fn terminal_process_matches_command(process: &TerminalProcess, command_lower: &str) -> bool {
    process
        .command_name
        .to_ascii_lowercase()
        .contains(command_lower)
        || process
            .command_line
            .to_ascii_lowercase()
            .contains(command_lower)
}

fn terminal_process_matches_cwd(process: &TerminalProcess, cwd: &str) -> bool {
    let requested = cwd.trim_end_matches('/');
    process.cwd.as_deref().is_some_and(|value| {
        let actual = value.trim_end_matches('/');
        actual == requested
            || (!requested.starts_with('/')
                && actual
                    .strip_suffix(requested)
                    .is_some_and(|prefix| prefix.ends_with('/')))
    })
}

fn tty_matches(actual: &str, requested: &str) -> bool {
    actual == requested
        || actual
            .strip_prefix("/dev/")
            .is_some_and(|value| value == requested)
        || actual
            .strip_prefix("/dev/pts/")
            .is_some_and(|value| value == requested)
}

fn optional_exact_match(actual: &Option<String>, requested: Option<&str>) -> bool {
    normalized_target(requested).is_none_or(|requested| {
        actual
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(&requested))
    })
}

fn optional_title_match(actual: &Option<String>, requested: Option<&str>) -> bool {
    normalized_target(requested).is_none_or(|requested| {
        let requested = requested.to_ascii_lowercase();
        actual
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains(&requested))
    })
}

fn normalized_target(value: Option<&str>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn invalid(message: impl Into<String>) -> BackendError {
    BackendError::new(BackendErrorCode::InvalidRequest, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windowing::types::{TerminalProcess, TerminalWindowContext};

    fn window(id: &str) -> LinuxWindowInfo {
        LinuxWindowInfo {
            window_id: id.to_string(),
            title: Some("Ghostty".to_string()),
            app_id: Some("com.mitchellh.ghostty.desktop".to_string()),
            wm_class: Some("com.mitchellh.ghostty".to_string()),
            pid: Some(100),
            bounds: None,
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: false,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "test".to_string(),
            terminal: Some(TerminalWindowContext {
                tty: "/dev/pts/7".to_string(),
                root_process: TerminalProcess {
                    pid: 200,
                    command_name: "zsh".to_string(),
                    command_line: "zsh".to_string(),
                    cwd: Some("/home/user/project".to_string()),
                },
                active_process: Some(TerminalProcess {
                    pid: 201,
                    command_name: "codex".to_string(),
                    command_line: "codex exec".to_string(),
                    cwd: Some("/home/user/project".to_string()),
                }),
                process_count: 2,
                confidence: "high".to_string(),
                match_reason: "test".to_string(),
            }),
        }
    }

    #[test]
    fn resolves_terminal_target_by_tty_command_and_cwd() {
        let windows = vec![window("abc")];
        let target = WindowTarget {
            tty: Some("pts/7".to_string()),
            terminal_command: Some("codex".to_string()),
            terminal_cwd: Some("project".to_string()),
            ..WindowTarget::default()
        };

        assert_eq!(
            resolve_window_target(&windows, &target).unwrap().window_id,
            "abc"
        );
    }

    #[test]
    fn reports_ambiguous_terminal_targets() {
        let windows = vec![window("a"), window("b")];
        let target = WindowTarget {
            terminal_command: Some("codex".to_string()),
            ..WindowTarget::default()
        };

        let error = resolve_window_target(&windows, &target).unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("matched multiple"));
    }

    #[test]
    fn reports_ambiguous_app_id_targets() {
        let windows = vec![window("a"), window("b")];
        let target = WindowTarget {
            app_id: Some("com.mitchellh.ghostty.desktop".to_string()),
            ..WindowTarget::default()
        };

        let error = resolve_window_target(&windows, &target).unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(
            error
                .message
                .contains("app_id com.mitchellh.ghostty.desktop")
        );
        assert!(error.message.contains("matched multiple"));
    }

    #[test]
    fn reports_ambiguous_wm_class_targets() {
        let windows = vec![window("a"), window("b")];
        let target = WindowTarget {
            wm_class: Some("com.mitchellh.ghostty".to_string()),
            ..WindowTarget::default()
        };

        let error = resolve_window_target(&windows, &target).unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("wm_class com.mitchellh.ghostty"));
        assert!(error.message.contains("matched multiple"));
    }

    #[test]
    fn reports_ambiguous_title_targets() {
        let windows = vec![window("a"), window("b")];
        let target = WindowTarget {
            title: Some("ghost".to_string()),
            ..WindowTarget::default()
        };

        let error = resolve_window_target(&windows, &target).unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert!(error.message.contains("title containing ghost"));
        assert!(error.message.contains("matched multiple"));
    }
}
