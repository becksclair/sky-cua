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
        // Exact (case-insensitive full-title) matches are safe to auto-resolve,
        // picking the focused window on a tie. Substring matches are not
        // identity proof, so keep the historical ambiguity error when a
        // substring matches more than one window.
        let exact_matches = windows
            .iter()
            .filter(|window| {
                window
                    .title
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&title))
            })
            .collect::<Vec<_>>();
        if !exact_matches.is_empty() {
            return best_window_match(exact_matches, &format!("title {title}"));
        }
        let title_lower = title.to_ascii_lowercase();
        let substring_matches = windows
            .iter()
            .filter(|window| {
                window
                    .title
                    .as_deref()
                    .is_some_and(|value| value.to_ascii_lowercase().contains(&title_lower))
            })
            .collect::<Vec<_>>();
        return unique_window_match(substring_matches, &format!("title containing {title}"));
    }

    Err(invalid(
        "Pass window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd to target a window.",
    ))
}

fn unique_window_match<'a>(
    matches: Vec<&'a LinuxWindowInfo>,
    description: &str,
) -> Result<&'a LinuxWindowInfo, BackendError> {
    resolve_window_match(matches, description, false)
}

/// Collapse COSMIC+X11 pairs that describe the same XWayland toplevel, keeping
/// the first (COSMIC, per backend order) entry so a selector that matches both
/// resolves to the single logical window with logical bounds.
fn dedupe_xwayland_alias_matches<'a>(
    matches: Vec<&'a LinuxWindowInfo>,
) -> Vec<&'a LinuxWindowInfo> {
    let mut unique: Vec<&'a LinuxWindowInfo> = Vec::new();
    for window in matches {
        if !unique
            .iter()
            .any(|existing| crate::app_match::xwayland_window_alias(existing, window))
        {
            unique.push(window);
        }
    }
    unique
}

fn best_window_match<'a>(
    matches: Vec<&'a LinuxWindowInfo>,
    description: &str,
) -> Result<&'a LinuxWindowInfo, BackendError> {
    resolve_window_match(matches, description, true)
}

/// Shared single-window resolver. `prefer_focused` makes a multi-match pick the
/// focused window and otherwise report richer per-window details; otherwise the
/// multi-match is ambiguous and lists bare ids. The singleton and no-match arms
/// are identical for both modes.
///
/// COSMIC+X11 XWayland aliases are collapsed first: an XWayland toplevel is
/// listed once by the COSMIC helper (WM_CLASS surfaced as `app_id`, no PID,
/// logical bounds) and once by the X11 EWMH backend (`<stem>.desktop` app_id,
/// PID, physical bounds). A title selector matching both must resolve to one
/// logical window rather than reporting a spurious ambiguity; explicit
/// `window_id`/`app_id` targets still match whichever backend carries them.
fn resolve_window_match<'a>(
    matches: Vec<&'a LinuxWindowInfo>,
    description: &str,
    prefer_focused: bool,
) -> Result<&'a LinuxWindowInfo, BackendError> {
    let matches = dedupe_xwayland_alias_matches(matches);
    match matches.as_slice() {
        [window] => Ok(*window),
        [] => Err(invalid(format!("No window matched {description}."))),
        windows if prefer_focused => {
            if let Some(focused) = windows.iter().find(|w| w.focused) {
                return Ok(*focused);
            }
            let details = windows
                .iter()
                .map(|w| {
                    let id = &w.window_id;
                    let title = w.title.as_deref().unwrap_or("(no title)");
                    let app = w.app_id.as_deref().unwrap_or("(unknown)");
                    format!("{id}: {app} — {title}")
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(invalid(format!(
                "{description} matched multiple windows [{details}]; add window_id, tty, title, or terminal_command to disambiguate."
            )))
        }
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
    if !window.terminal_target_sessions.is_empty() {
        return window.terminal_target_sessions.iter().any(|session| {
            terminal_session_matches_target(&session.tty, &session.processes, target)
        });
    }

    let Some(terminal) = &window.terminal else {
        return false;
    };
    let mut processes = vec![&terminal.root_process];
    if let Some(active) = &terminal.active_process
        && active.pid != terminal.root_process.pid
    {
        processes.push(active);
    }
    terminal_session_matches_target_refs(&terminal.tty, &processes, target)
}

fn terminal_session_matches_target(
    tty: &str,
    processes: &[TerminalProcess],
    target: &WindowTarget,
) -> bool {
    terminal_session_matches_target_refs(tty, &processes.iter().collect::<Vec<_>>(), target)
}

fn terminal_session_matches_target_refs(
    actual_tty: &str,
    processes: &[&TerminalProcess],
    target: &WindowTarget,
) -> bool {
    if let Some(requested_tty) = normalized_target(target.tty.as_deref())
        && !tty_matches(actual_tty, &requested_tty)
    {
        return false;
    }

    if let Some(pid) = target.terminal_pid
        && !processes.iter().any(|process| process.pid == pid)
    {
        return false;
    }

    if let Some(command) = normalized_target(target.terminal_command.as_deref()) {
        let command = command.to_ascii_lowercase();
        if !processes
            .iter()
            .any(|process| terminal_process_matches_command(process, &command))
        {
            return false;
        }
    }

    if let Some(cwd) = normalized_target(target.terminal_cwd.as_deref())
        && !processes
            .iter()
            .any(|process| terminal_process_matches_cwd(process, &cwd))
    {
        return false;
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
            terminal_target_sessions: Vec::new(),
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
    fn resolves_any_session_owned_by_a_single_terminal_window() {
        let mut terminal = window("abc");
        terminal.terminal = None;
        terminal.terminal_target_sessions = vec![
            crate::windowing::types::TerminalTargetSession {
                tty: "/dev/pts/7".to_string(),
                processes: vec![TerminalProcess {
                    pid: 201,
                    command_name: "simyo-renew".to_string(),
                    command_line: "python simyo-renew.py".to_string(),
                    cwd: Some("/home/user/simyo".to_string()),
                }],
            },
            crate::windowing::types::TerminalTargetSession {
                tty: "/dev/pts/8".to_string(),
                processes: vec![TerminalProcess {
                    pid: 301,
                    command_name: "codex".to_string(),
                    command_line: "codex".to_string(),
                    cwd: Some("/home/user/project".to_string()),
                }],
            },
        ];
        let windows = vec![terminal];

        for target in [
            WindowTarget {
                terminal_pid: Some(201),
                ..WindowTarget::default()
            },
            WindowTarget {
                terminal_command: Some("simyo-renew".to_string()),
                ..WindowTarget::default()
            },
            WindowTarget {
                tty: Some("pts/8".to_string()),
                terminal_cwd: Some("project".to_string()),
                ..WindowTarget::default()
            },
        ] {
            assert_eq!(
                resolve_window_target(&windows, &target).unwrap().window_id,
                "abc"
            );
        }
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

    #[test]
    fn exact_title_match_picks_focused_window() {
        let mut a = window("a");
        a.focused = false;
        let mut b = window("b");
        b.focused = true;
        let windows = vec![a, b];
        let target = WindowTarget {
            title: Some("Ghostty".to_string()),
            ..WindowTarget::default()
        };

        assert_eq!(
            resolve_window_target(&windows, &target).unwrap().window_id,
            "b"
        );
    }

    #[test]
    fn title_target_collapses_xwayland_alias_to_single_window() {
        // An XWayland window is listed twice: COSMIC surfaces its WM_CLASS as
        // app_id (no PID), and the X11 backend reports `<stem>.desktop` with a
        // PID. A substring title matching both must resolve to one logical
        // window (the COSMIC entry, first in backend order) instead of failing
        // as ambiguous.
        let mut cosmic = window("cosmic-1");
        cosmic.backend = crate::windowing::registry::COSMIC_WAYLAND_BACKEND.to_string();
        cosmic.app_id = Some("kwrite".to_string());
        cosmic.wm_class = None;
        cosmic.pid = None;
        cosmic.title = Some("proof.txt  \u{2014} KWrite".to_string());

        let mut x11 = window("0x800007");
        x11.backend = crate::windowing::registry::X11_BACKEND.to_string();
        x11.app_id = Some("kwrite.desktop".to_string());
        x11.wm_class = Some("kwrite".to_string());
        x11.pid = Some(4242);
        x11.title = Some("proof.txt ".to_string());

        let windows = vec![cosmic, x11];
        let target = WindowTarget {
            title: Some("proof.txt".to_string()),
            ..WindowTarget::default()
        };

        assert_eq!(
            resolve_window_target(&windows, &target).unwrap().window_id,
            "cosmic-1"
        );
    }
}
