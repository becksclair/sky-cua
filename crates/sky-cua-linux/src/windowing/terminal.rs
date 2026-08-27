use std::collections::{BTreeMap, HashMap, HashSet};
use std::{fs, path::PathBuf};

use super::types::{
    LinuxWindowInfo, TerminalProcess, TerminalTargetSession, TerminalWindowContext,
};

#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u32,
    ppid: u32,
    start_ticks: u64,
    command_name: String,
    command_line: String,
    cwd: Option<String>,
    tty_paths: Vec<String>,
}

pub fn enrich_terminal_windows(windows: &mut [LinuxWindowInfo]) {
    let processes = read_process_table();
    if !processes.is_empty() {
        enrich_terminal_windows_with_processes(windows, &processes);
    }
}

fn enrich_terminal_windows_with_processes(
    windows: &mut [LinuxWindowInfo],
    processes: &[ProcessInfo],
) {
    let mut windows_by_terminal_pid: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, window) in windows.iter().enumerate() {
        if looks_like_terminal_window(window)
            && let Some(pid) = window.pid
        {
            windows_by_terminal_pid.entry(pid).or_default().push(index);
        }
    }

    if windows_by_terminal_pid.is_empty() {
        return;
    }

    let by_pid = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    let terminal_pids = windows_by_terminal_pid
        .keys()
        .copied()
        .collect::<HashSet<_>>();
    let session_index = terminal_session_index(processes, &by_pid, &terminal_pids);

    for (terminal_pid, mut window_indexes) in windows_by_terminal_pid {
        let mut sessions = terminal_sessions_for_pid(terminal_pid, &session_index);
        if sessions.is_empty() {
            continue;
        }

        window_indexes.sort_by_key(|index| windows[*index].window_id.clone());
        sessions.sort_by_key(|session| session.root_start_ticks);

        if window_indexes.len() == 1 {
            windows[window_indexes[0]].terminal_target_sessions =
                sessions.iter().map(terminal_target_session).collect();
        } else if window_indexes.len() == sessions.len() {
            for (window_index, session) in window_indexes.iter().copied().zip(&sessions) {
                windows[window_index].terminal_target_sessions =
                    vec![terminal_target_session(session)];
            }
        }

        let confidence = if window_indexes.len() == 1 && sessions.len() == 1 {
            Some((
                "high",
                "Only one terminal window and one PTY session share the terminal app PID.",
            ))
        } else if window_indexes.len() == sessions.len() {
            Some((
                "heuristic",
                "Matched terminal windows to PTY sessions by shared terminal app PID and creation order.",
            ))
        } else {
            None
        };

        let Some((confidence, reason)) = confidence else {
            continue;
        };

        for (window_index, session) in window_indexes.into_iter().zip(sessions) {
            windows[window_index].terminal = Some(TerminalWindowContext {
                tty: session.tty,
                root_process: process_summary(&session.root_process),
                active_process: session.active_process.as_ref().map(process_summary),
                process_count: session.process_count,
                confidence: confidence.to_string(),
                match_reason: reason.to_string(),
            });
        }
    }
}

#[derive(Debug, Clone)]
struct TerminalSession {
    tty: String,
    root_process: ProcessInfo,
    active_process: Option<ProcessInfo>,
    processes: Vec<ProcessInfo>,
    process_count: usize,
    root_start_ticks: u64,
}

type IndexedTerminalProcess<'a> = (&'a ProcessInfo, usize);
type TerminalSessionIndex<'a> = HashMap<u32, BTreeMap<String, Vec<IndexedTerminalProcess<'a>>>>;

fn terminal_session_index<'a>(
    processes: &'a [ProcessInfo],
    by_pid: &HashMap<u32, &'a ProcessInfo>,
    terminal_pids: &HashSet<u32>,
) -> TerminalSessionIndex<'a> {
    let mut index: TerminalSessionIndex<'a> = HashMap::new();
    for process in processes {
        let mut current = process.pid;
        let mut visited = HashSet::new();
        let mut depth = 0usize;
        while visited.insert(current) {
            let Some(current_process) = by_pid.get(&current) else {
                break;
            };
            let parent = current_process.ppid;
            if terminal_pids.contains(&parent) {
                for tty in &process.tty_paths {
                    index
                        .entry(parent)
                        .or_default()
                        .entry(tty.clone())
                        .or_default()
                        .push((process, depth));
                }
            }
            if parent == 0 || parent == current {
                break;
            }
            current = parent;
            depth += 1;
        }
    }
    index
}

fn terminal_sessions_for_pid(
    terminal_pid: u32,
    session_index: &TerminalSessionIndex<'_>,
) -> Vec<TerminalSession> {
    session_index
        .get(&terminal_pid)
        .into_iter()
        .flat_map(|grouped| grouped.iter())
        .filter_map(|(tty, indexed_processes)| {
            let mut processes = indexed_processes.clone();
            processes.sort_by_key(|(process, depth)| (*depth, process.start_ticks, process.pid));
            let root_process = processes.first()?.0.clone();
            let active_process = active_terminal_process(&processes);
            let session_processes = processes
                .iter()
                .map(|(process, _)| (*process).clone())
                .collect::<Vec<_>>();
            Some(TerminalSession {
                tty: tty.clone(),
                root_start_ticks: root_process.start_ticks,
                process_count: session_processes.len(),
                root_process,
                active_process,
                processes: session_processes,
            })
        })
        .collect()
}

fn active_terminal_process(processes: &[IndexedTerminalProcess<'_>]) -> Option<ProcessInfo> {
    let same_tty_parents = processes
        .iter()
        .map(|(process, _)| process.ppid)
        .collect::<HashSet<_>>();
    processes
        .iter()
        .map(|(process, _)| *process)
        .filter(|process| !same_tty_parents.contains(&process.pid))
        .max_by_key(|process| (process.start_ticks, process.pid))
        .cloned()
        .or_else(|| {
            processes
                .iter()
                .map(|(process, _)| *process)
                .max_by_key(|process| (process.start_ticks, process.pid))
                .cloned()
        })
}

fn process_summary(process: &ProcessInfo) -> TerminalProcess {
    TerminalProcess {
        pid: process.pid,
        command_name: process.command_name.clone(),
        command_line: process.command_line.clone(),
        cwd: process.cwd.clone(),
    }
}

fn terminal_target_session(session: &TerminalSession) -> TerminalTargetSession {
    TerminalTargetSession {
        tty: session.tty.clone(),
        processes: session.processes.iter().map(process_summary).collect(),
    }
}

fn looks_like_terminal_window(window: &LinuxWindowInfo) -> bool {
    let haystack = [
        window.app_id.as_deref(),
        window.wm_class.as_deref(),
        window.title.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();

    [
        "ghostty",
        "gnome-terminal",
        "org.gnome.terminal",
        "ptyxis",
        "org.gnome.ptyxis",
        "kgx",
        "konsole",
        "kitty",
        "alacritty",
        "wezterm",
        "xterm",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn read_process_table() -> Vec<ProcessInfo> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            read_process_info(pid)
        })
        .collect()
}

fn read_process_info(pid: u32) -> Option<ProcessInfo> {
    let (ppid, start_ticks) = parse_stat(pid)?;
    let command_name = fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| pid.to_string());
    let command_line = read_command_line(pid).unwrap_or_else(|| command_name.clone());
    let cwd = fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(path_to_string);
    let tty_paths = read_tty_paths(pid);

    Some(ProcessInfo {
        pid,
        ppid,
        start_ticks,
        command_name,
        command_line,
        cwd,
        tty_paths,
    })
}

fn read_command_line(pid: u32) -> Option<String> {
    let bytes = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = bytes
        .split(|&b| b == 0)
        .filter(|slice| !slice.is_empty())
        .filter_map(|slice| String::from_utf8(slice.to_vec()).ok())
        .collect();
    if args.is_empty() {
        return None;
    }
    Some(args.join(" "))
}

fn parse_stat(pid: u32) -> Option<(u32, u64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat_contents(&stat)
}

fn parse_stat_contents(stat: &str) -> Option<(u32, u64)> {
    let close_paren = stat.rfind(')')?;
    let fields = stat
        .get(close_paren + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let ppid = fields.get(1)?.parse().ok()?;
    let start_ticks = fields.get(19)?.parse().ok()?;
    Some((ppid, start_ticks))
}

fn read_tty_paths(pid: u32) -> Vec<String> {
    let Ok(entries) = fs::read_dir(format!("/proc/{pid}/fd")) else {
        return Vec::new();
    };
    let mut paths = entries
        .flatten()
        .filter_map(|entry| fs::read_link(entry.path()).ok())
        .filter_map(|path| {
            let value = path_to_string(path);
            value.starts_with("/dev/pts/").then_some(value)
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{CoordinateSpace, RectF};

    use super::*;

    fn terminal_window(window_id: &str, pid: u32) -> LinuxWindowInfo {
        LinuxWindowInfo {
            window_id: window_id.to_string(),
            title: Some("Ghostty".to_string()),
            app_id: Some("com.mitchellh.ghostty.desktop".to_string()),
            wm_class: Some("com.mitchellh.ghostty".to_string()),
            pid: Some(pid),
            bounds: Some(RectF {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            display: None,
            display_intersections: Vec::new(),
            workspace: Some(0),
            focused: false,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "test".to_string(),
            terminal: None,
            terminal_target_sessions: Vec::new(),
        }
    }

    fn process(
        pid: u32,
        ppid: u32,
        start_ticks: u64,
        command_name: &str,
        tty: Option<&str>,
    ) -> ProcessInfo {
        ProcessInfo {
            pid,
            ppid,
            start_ticks,
            command_name: command_name.to_string(),
            command_line: command_name.to_string(),
            cwd: Some("/home/user".to_string()),
            tty_paths: tty.into_iter().map(ToOwned::to_owned).collect(),
        }
    }

    fn process_with_cmdline(
        pid: u32,
        ppid: u32,
        start_ticks: u64,
        command_name: &str,
        command_line: &str,
        tty: Option<&str>,
    ) -> ProcessInfo {
        ProcessInfo {
            pid,
            ppid,
            start_ticks,
            command_name: command_name.to_string(),
            command_line: command_line.to_string(),
            cwd: Some("/home/user".to_string()),
            tty_paths: tty.into_iter().map(ToOwned::to_owned).collect(),
        }
    }

    #[test]
    fn assigns_terminal_sessions_by_window_and_pty_creation_order() {
        let mut windows = vec![terminal_window("11", 100), terminal_window("12", 100)];
        let processes = vec![
            process(100, 1, 1, "ghostty", None),
            process(200, 100, 10, "sh", Some("/dev/pts/0")),
            process(201, 200, 11, "zsh", Some("/dev/pts/0")),
            process(202, 201, 12, "claude", Some("/dev/pts/0")),
            process(300, 100, 20, "sh", Some("/dev/pts/1")),
            process(301, 300, 21, "zsh", Some("/dev/pts/1")),
            process(302, 301, 22, "codex", Some("/dev/pts/1")),
        ];

        enrich_terminal_windows_with_processes(&mut windows, &processes);

        let first = windows[0].terminal.as_ref().unwrap();
        assert_eq!(first.tty, "/dev/pts/0");
        assert_eq!(
            first.active_process.as_ref().unwrap().command_name,
            "claude"
        );
        assert_eq!(first.confidence, "heuristic");

        let second = windows[1].terminal.as_ref().unwrap();
        assert_eq!(second.tty, "/dev/pts/1");
        assert_eq!(
            second.active_process.as_ref().unwrap().command_name,
            "codex"
        );
    }

    #[test]
    fn indexes_multiple_terminal_process_trees_in_one_ancestry_pass() {
        let processes = vec![
            process(100, 1, 1, "ghostty", None),
            process(200, 100, 10, "sh", Some("/dev/pts/0")),
            process(201, 200, 11, "codex", Some("/dev/pts/0")),
            process(300, 1, 20, "konsole", None),
            process(400, 300, 30, "zsh", Some("/dev/pts/1")),
            process(401, 400, 31, "claude", Some("/dev/pts/1")),
        ];
        let by_pid = processes
            .iter()
            .map(|process| (process.pid, process))
            .collect::<HashMap<_, _>>();
        let terminal_pids = HashSet::from([100, 300]);

        let index = terminal_session_index(&processes, &by_pid, &terminal_pids);

        let first = terminal_sessions_for_pid(100, &index);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].root_process.pid, 200);
        assert_eq!(first[0].active_process.as_ref().unwrap().pid, 201);

        let second = terminal_sessions_for_pid(300, &index);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].root_process.pid, 400);
        assert_eq!(second[0].active_process.as_ref().unwrap().pid, 401);
    }

    #[test]
    fn terminal_session_index_preserves_nested_terminal_ancestor_semantics() {
        let processes = vec![
            process(100, 1, 1, "ghostty", None),
            process(200, 100, 10, "nested-terminal", None),
            process(300, 200, 20, "zsh", Some("/dev/pts/0")),
        ];
        let by_pid = processes
            .iter()
            .map(|process| (process.pid, process))
            .collect::<HashMap<_, _>>();
        let terminal_pids = HashSet::from([100, 200]);

        let index = terminal_session_index(&processes, &by_pid, &terminal_pids);

        assert_eq!(
            terminal_sessions_for_pid(100, &index)[0].root_process.pid,
            300
        );
        assert_eq!(
            terminal_sessions_for_pid(200, &index)[0].root_process.pid,
            300
        );
    }

    #[test]
    fn leaves_terminal_context_empty_when_window_session_counts_do_not_match() {
        let mut windows = vec![terminal_window("11", 100), terminal_window("12", 100)];
        let processes = vec![
            process(100, 1, 1, "ghostty", None),
            process(200, 100, 10, "sh", Some("/dev/pts/0")),
            process(201, 200, 11, "zsh", Some("/dev/pts/0")),
        ];

        enrich_terminal_windows_with_processes(&mut windows, &processes);

        assert!(windows.iter().all(|window| window.terminal.is_none()));
        assert!(
            windows
                .iter()
                .all(|window| window.terminal_target_sessions.is_empty())
        );
    }

    #[test]
    fn indexes_all_sessions_for_a_single_owning_terminal_window() {
        let mut windows = vec![terminal_window("11", 100)];
        let processes = vec![
            process(100, 1, 1, "ghostty", None),
            process(200, 100, 10, "zsh", Some("/dev/pts/0")),
            process(201, 200, 11, "simyo-renew", Some("/dev/pts/0")),
            process(300, 100, 20, "zsh", Some("/dev/pts/1")),
            process(301, 300, 21, "codex", Some("/dev/pts/1")),
        ];

        enrich_terminal_windows_with_processes(&mut windows, &processes);

        assert!(windows[0].terminal.is_none());
        assert_eq!(windows[0].terminal_target_sessions.len(), 2);
        assert!(windows[0].terminal_target_sessions.iter().any(|session| {
            session
                .processes
                .iter()
                .any(|process| process.command_name == "simyo-renew")
        }));
        assert!(windows[0].terminal_target_sessions.iter().any(|session| {
            session
                .processes
                .iter()
                .any(|process| process.command_name == "codex")
        }));
    }

    #[test]
    fn parses_proc_stat_with_parenthesized_command() {
        let stat =
            "123 (cmd with spaces) S 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 12345 26";

        assert_eq!(parse_stat_contents(stat), Some((7, 12345)));
    }

    #[test]
    fn process_summary_preserves_full_command_line() {
        let process = process_with_cmdline(
            42,
            1,
            10,
            "codex",
            "codex --dangerously-bypass-approvals-and-sandbox",
            Some("/dev/pts/0"),
        );

        let summary = process_summary(&process);

        assert_eq!(summary.command_name, "codex");
        assert_eq!(
            summary.command_line,
            "codex --dangerously-bypass-approvals-and-sandbox"
        );
    }

    #[test]
    fn process_summary_falls_back_to_command_name_when_command_line_empty() {
        let process = process(42, 1, 10, "codex", Some("/dev/pts/0"));

        let summary = process_summary(&process);

        assert_eq!(summary.command_name, "codex");
        assert_eq!(summary.command_line, "codex");
    }

    #[test]
    fn command_line_passes_through_terminal_enrichment() {
        let mut windows = vec![terminal_window("11", 100)];
        let processes = vec![
            process(100, 1, 1, "ghostty", None),
            process_with_cmdline(200, 100, 10, "sh", "sh -c 'echo hello'", Some("/dev/pts/0")),
            process_with_cmdline(
                201,
                200,
                11,
                "node",
                "node /home/user/app.js --port 3000",
                Some("/dev/pts/0"),
            ),
        ];

        enrich_terminal_windows_with_processes(&mut windows, &processes);

        let terminal = windows[0].terminal.as_ref().unwrap();
        assert_eq!(terminal.root_process.command_line, "sh -c 'echo hello'");
        assert_eq!(
            terminal.active_process.as_ref().unwrap().command_line,
            "node /home/user/app.js --port 3000"
        );
    }
}
