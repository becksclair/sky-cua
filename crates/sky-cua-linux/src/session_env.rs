use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
    process::Command,
};

use sky_cua_platform::{
    CLIENT_CLEARED_SESSION_ENV_KEYS_ENV, CLIENT_SESSION_ENV_REPAIRS_ENV, DESKTOP_LAUNCH_ENV_KEYS,
    GRAPHICAL_SESSION_ENV_KEYS,
    model::{DoctorSessionEnvRepair, DoctorSessionEnvReport},
    x11_display_number,
};
const DEFAULT_PATH_DIRS: &[&str] = &[
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

/// Hydrate session bus and desktop environment variables by walking the
/// process tree, the systemd user manager, and well-known runtime paths. This
/// is critical when sky-cua-service is launched without inheriting the user's
/// graphical session environment.
pub fn hydrate_session_env() -> DoctorSessionEnvReport {
    let mut report = DoctorSessionEnvReport::default();
    normalize_path(&mut report);
    merge_client_launch_repairs(&mut report);
    let blocked_keys = client_cleared_graphical_keys();
    hydrate_desktop_env_from_process_tree(&mut report, &blocked_keys);
    hydrate_desktop_env_from_systemd(&mut report, &blocked_keys);
    hydrate_desktop_env_from_active_sessions(&mut report, &blocked_keys);

    if env_var("XDG_RUNTIME_DIR").is_none()
        && let Some(runtime) = xdg_runtime_dir()
        && runtime.exists()
    {
        let value = runtime.display().to_string();
        unsafe { env::set_var("XDG_RUNTIME_DIR", runtime) };
        push_repair(&mut report, "XDG_RUNTIME_DIR", "runtime-dir", Some(value));
    }

    if env_var("DBUS_SESSION_BUS_ADDRESS").is_none()
        && let Some(runtime) = xdg_runtime_dir()
    {
        let bus = runtime.join("bus");
        if bus.exists() {
            let value = format!("unix:path={}", bus.display());
            unsafe {
                env::set_var("DBUS_SESSION_BUS_ADDRESS", &value);
            }
            push_repair(
                &mut report,
                "DBUS_SESSION_BUS_ADDRESS",
                "runtime-bus",
                Some(value),
            );
        }
    }

    report
}

fn merge_client_launch_repairs(report: &mut DoctorSessionEnvReport) {
    let Some(raw_repairs) = env_var(CLIENT_SESSION_ENV_REPAIRS_ENV) else {
        return;
    };
    match serde_json::from_str::<Vec<DoctorSessionEnvRepair>>(&raw_repairs) {
        Ok(repairs) => {
            for repair in repairs {
                if repair.key == "PATH" || !DESKTOP_LAUNCH_ENV_KEYS.contains(&repair.key.as_str()) {
                    continue;
                }
                report.repaired.push(DoctorSessionEnvRepair {
                    key: repair.key,
                    source: "client-launch".to_string(),
                    value: repair.value,
                });
            }
        }
        Err(error) => report.notes.push(format!(
            "client launch session-env repair report was invalid: {error}"
        )),
    }
}

fn client_cleared_graphical_keys() -> HashSet<String> {
    let Some(raw_keys) = env_var(CLIENT_CLEARED_SESSION_ENV_KEYS_ENV) else {
        return HashSet::new();
    };
    serde_json::from_str::<Vec<String>>(&raw_keys)
        .unwrap_or_default()
        .into_iter()
        .filter(|key| GRAPHICAL_SESSION_ENV_KEYS.contains(&key.as_str()))
        .collect()
}

/// Backward-compatible wrapper for older callers that only need mutation.
pub fn hydrate_session_bus_env() {
    let _ = hydrate_session_env();
}

fn hydrate_desktop_env_from_process_tree(
    report: &mut DoctorSessionEnvReport,
    blocked_keys: &HashSet<String>,
) {
    for process_env in desktop_process_environments() {
        hydrate_desktop_env_from_map(report, "process-tree", &process_env, blocked_keys);

        if GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .all(|key| env_var(key).is_some())
        {
            break;
        }
    }
}

fn hydrate_desktop_env_from_systemd(
    report: &mut DoctorSessionEnvReport,
    blocked_keys: &HashSet<String>,
) {
    let Some(systemd_env) = systemd_user_environment(report) else {
        return;
    };
    hydrate_desktop_env_from_map(report, "systemd-user", &systemd_env, blocked_keys);
}

fn hydrate_desktop_env_from_active_sessions(
    report: &mut DoctorSessionEnvReport,
    blocked_keys: &HashSet<String>,
) {
    for session_env in active_session_environments() {
        hydrate_desktop_env_from_map(report, "active-session", &session_env, blocked_keys);
        if GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .all(|key| env_var(key).is_some())
        {
            break;
        }
    }
}

fn hydrate_desktop_env_from_map(
    report: &mut DoctorSessionEnvReport,
    source: &str,
    values: &HashMap<String, String>,
    blocked_keys: &HashSet<String>,
) {
    for key in GRAPHICAL_SESSION_ENV_KEYS {
        let Some(candidate) = values.get(*key).filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        let current = env_var(key);
        let is_blocked = blocked_keys.contains(*key);
        // Non-active sources only fill missing slots and respect the client's
        // cleared list (e.g. isolated X11 sandbox that intentionally removed
        // WAYLAND_DISPLAY). They never correct a stale value.
        if source != "active-session" {
            if current.is_some() || is_blocked {
                continue;
            }
            unsafe { env::set_var(key, candidate) };
            push_repair(report, *key, source, Some(candidate.clone()));
            continue;
        }
        // active-session is authoritative and may correct stale values (e.g.
        // DISPLAY :0 -> :1 after Xwayland restart) even when the daemon was
        // spawned with a blocked/cleared list containing :0 from client-launch.
        if let Some(cur) = current {
            if cur == *candidate {
                continue;
            }
            // Isolated X11 sandbox intentionally hosts DISPLAY=:N with
            // WAYLAND_DISPLAY removed and XDG_SESSION_TYPE=x11. Host's
            // wayland session must not flip it to :1 even if its X socket
            // is not yet live (xpra just spawned). Detect via the sandbox
            // markers and skip all active-session overwrites for the sandbox.
            if is_isolated_sandbox(blocked_keys) {
                continue;
            }
            // Only overwrite a present value when the current value is no
            // longer live (its socket/path vanished). This prevents an
            // isolated xpra daemon (DISPLAY=:131, X131 live) from being
            // flipped to the host's :1, while still healing a shared daemon
            // whose old X0 socket has been removed and only X1 remains.
            if is_env_value_live(key, &cur) {
                continue;
            }
            unsafe { env::set_var(key, candidate) };
            push_repair(report, *key, source, Some(candidate.clone()));
        } else {
            // Missing - active session fills even when the client marked the
            // key as cleared (detached launch). The live session is the truth,
            // except for the isolated X11 sandbox which intentionally removed
            // WAYLAND_DISPLAY to force Qt/GTK onto X11. That sandbox has
            // XDG_SESSION_TYPE=x11 and DISPLAY=:N (high N) and must stay
            // without WAYLAND_DISPLAY even though the host's wayland-1 exists.
            if *key == "WAYLAND_DISPLAY" && env_var("XDG_SESSION_TYPE").as_deref() == Some("x11") {
                continue;
            }
            unsafe { env::set_var(key, candidate) };
            push_repair(report, *key, source, Some(candidate.clone()));
        }
    }
}

fn is_isolated_sandbox(blocked_keys: &HashSet<String>) -> bool {
    // Isolated xpra sandbox: build_spawn_env sets XDG_SESSION_TYPE=x11,
    // WAYLAND_DISPLAY removed (blocked), DISPLAY=:N, QT_QPA_PLATFORM=xcb and
    // GDK_BACKEND=x11. Host X11 also has x11 type but lacks xcb/x11 forcing,
    // so it stays eligible for active-session correction.
    env_var("XDG_SESSION_TYPE").as_deref() == Some("x11")
        && env_var("WAYLAND_DISPLAY").is_none()
        && blocked_keys.contains("WAYLAND_DISPLAY")
        && env_var("DISPLAY").is_some()
        && env_var("QT_QPA_PLATFORM").as_deref() == Some("xcb")
        && env_var("GDK_BACKEND").as_deref() == Some("x11")
}

fn is_env_value_live(key: &str, value: &str) -> bool {
    match key {
        "DISPLAY" => {
            // :N or unix/:N - live iff /tmp/.X11-unix/XN exists
            if let Some(num) = x11_display_number(value) {
                return PathBuf::from(format!("/tmp/.X11-unix/X{num}")).exists();
            }
            // Fallback: treat non-local or unparseable as live to avoid
            // aggressive overwrites (e.g. localhost:10.0 forwarding).
            true
        }
        "WAYLAND_DISPLAY" => {
            if let Some(runtime) = env_var("XDG_RUNTIME_DIR") {
                return Path::new(&runtime).join(value).exists();
            }
            if let Some(runtime) = xdg_runtime_dir() {
                return runtime.join(value).exists();
            }
            false
        }
        "XDG_RUNTIME_DIR" => Path::new(value).is_dir(),
        "DBUS_SESSION_BUS_ADDRESS" => {
            if let Some(path) = value.strip_prefix("unix:path=") {
                // Address may include ",guid=..." suffix after the socket path.
                let socket_path = path
                    .split(',')
                    .next()
                    .unwrap_or(path)
                    .split(';')
                    .next()
                    .unwrap_or(path);
                return Path::new(socket_path).exists();
            }
            // abstract or other transports - consider live
            true
        }
        // Desktop/type are not socket-bound; treat any non-empty as live so
        // we only overwrite when missing, not when stale. The client health
        // check will still trigger a daemon restart if they diverge, which is
        // the correct self-heal for those keys.
        _ => true,
    }
}

fn systemd_user_environment(
    report: &mut DoctorSessionEnvReport,
) -> Option<HashMap<String, String>> {
    let output = Command::new(systemctl_path())
        .args(["--user", "show-environment"])
        .output();
    match output {
        Ok(output) if output.status.success() => Some(parse_systemd_environment(&output.stdout)),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if !stderr.is_empty() {
                report
                    .notes
                    .push(format!("systemd-user show-environment failed: {stderr}"));
            }
            None
        }
        Err(error) => {
            report.notes.push(format!(
                "systemd-user show-environment unavailable: {error}"
            ));
            None
        }
    }
}

fn systemctl_path() -> &'static str {
    if Path::new("/usr/bin/systemctl").is_file() {
        "/usr/bin/systemctl"
    } else {
        "systemctl"
    }
}

fn parse_systemd_environment(bytes: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(parse_environment_line)
        .collect()
}

fn parse_environment_line(line: &str) -> Option<(String, String)> {
    let split = line.find('=')?;
    let (key, value) = line.split_at(split);
    if key.is_empty() || key.chars().any(char::is_whitespace) {
        return None;
    }
    Some((key.to_string(), value[1..].to_string()))
}

fn active_session_environments() -> Vec<HashMap<String, String>> {
    let mut environments = Vec::new();
    // Prefer the canonical logind view: active graphical user sessions.
    for pid in loginctl_session_leaders() {
        if let Some(env) = read_process_environ(pid) {
            if env.contains_key("WAYLAND_DISPLAY") || env.contains_key("DISPLAY") {
                environments.push(env);
            }
        } else {
            // Leader may be privileged (e.g. greetd worker); try its children.
            for child_pid in child_pids(pid) {
                if let Some(env) = read_process_environ(child_pid)
                    && (env.contains_key("WAYLAND_DISPLAY") || env.contains_key("DISPLAY"))
                {
                    environments.push(env);
                }
            }
        }
    }
    // Fallback: scan all user processes for any with a graphical display.
    // This covers greetd's 2-session mode where the daemon's parent chain
    // (systemd --user) has no display, but a sibling desktop session does.
    if environments.is_empty() {
        for pid in all_user_pids_with_display() {
            if let Some(env) = read_process_environ(pid) {
                environments.push(env);
                // One good sample is enough; avoid collecting stale duplicates.
                if environments.len() >= 2 {
                    break;
                }
            }
        }
    }
    // Last resort: well-known compositor processes (may have minimal env but
    // still carry XDG_CURRENT_DESKTOP).
    if environments.is_empty() {
        for pid in compositor_pids() {
            if let Some(env) = read_process_environ(pid) {
                environments.push(env);
            }
        }
    }
    environments
}

fn loginctl_session_leaders() -> Vec<u32> {
    let output = Command::new("loginctl")
        .args(["list-sessions", "--no-legend"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut leaders = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // loginctl format: SESSION UID USER SEAT LEADER CLASS TTY ...
        if parts.len() < 6 {
            continue;
        }
        let session_id = parts[0];
        let class = parts[5];
        // Only user sessions (not manager) that could hold a desktop.
        if class != "user" {
            continue;
        }
        // Fetch per-session details to confirm graphical.
        let detail = Command::new("loginctl")
            .args(["show-session", session_id, "-p", "Type", "-p", "Display"])
            .output();
        if let Ok(detail) = detail
            && detail.status.success()
        {
            let text = String::from_utf8_lossy(&detail.stdout);
            // Type=wayland or Type=x11 (manager is Type=unspecified)
            let is_graphical = text.contains("Type=wayland") || text.contains("Type=x11");
            // Some compositors report Display=:1 even on Wayland (Xwayland).
            // We accept any session that loginctl marks as graphical.
            if !is_graphical {
                continue;
            }
        }
        if let Ok(pid) = parts[4].parse::<u32>() {
            leaders.push(pid);
        }
    }
    leaders
}

fn compositor_pids() -> Vec<u32> {
    const COMPOSITORS: &[&str] = &[
        "cosmic-comp",
        "kwin_wayland",
        "gnome-shell",
        "sway",
        "hyprland",
        "river",
        "niri",
        "weston",
    ];
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return pids;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid_str = name.to_string_lossy();
        if pid_str.parse::<u32>().is_err() {
            continue;
        }
        let comm_path = format!("/proc/{pid_str}/comm");
        let Ok(comm) = std::fs::read_to_string(&comm_path) else {
            continue;
        };
        let comm = comm.trim();
        if COMPOSITORS.contains(&comm)
            && let Ok(pid) = pid_str.parse::<u32>()
        {
            pids.push(pid);
        }
    }
    pids
}

fn child_pids(parent: u32) -> Vec<u32> {
    let mut children = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return children;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid_str = name.to_string_lossy();
        if pid_str.parse::<u32>().is_err() {
            continue;
        }
        let status_path = format!("/proc/{pid_str}/status");
        let Ok(status) = std::fs::read_to_string(&status_path) else {
            continue;
        };
        if parse_parent_pid(&status) == Some(parent)
            && let Ok(pid) = pid_str.parse::<u32>()
        {
            children.push(pid);
        }
    }
    children
}

fn all_user_pids_with_display() -> Vec<u32> {
    let uid = user_id().and_then(|s| s.parse::<u32>().ok());
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return pids;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid_str = name.to_string_lossy();
        let Ok(pid) = pid_str.parse::<u32>() else {
            continue;
        };
        // Only consider processes owned by current user.
        if let Some(expected_uid) = uid {
            let status_path = format!("/proc/{pid}/status");
            let Ok(status) = std::fs::read_to_string(&status_path) else {
                continue;
            };
            let has_uid = status.lines().any(|line| {
                line.starts_with("Uid:")
                    && line
                        .split_whitespace()
                        .any(|v| v == expected_uid.to_string())
            });
            if !has_uid {
                continue;
            }
        }
        // Quick check: does environ contain DISPLAY or WAYLAND_DISPLAY?
        let environ_path = format!("/proc/{pid}/environ");
        let Ok(bytes) = std::fs::read(&environ_path) else {
            continue;
        };
        if bytes.windows(8).any(|w| w == b"DISPLAY=")
            || bytes.windows(16).any(|w| w == b"WAYLAND_DISPLAY=")
        {
            pids.push(pid);
        }
    }
    pids
}

fn normalize_path(report: &mut DoctorSessionEnvReport) {
    let original = env::var_os("PATH");
    let mut normalized = Vec::<PathBuf>::new();
    if let Some(path) = original.as_ref() {
        for entry in env::split_paths(path) {
            if entry.as_os_str().is_empty() || normalized.contains(&entry) {
                continue;
            }
            normalized.push(entry);
        }
    }
    for entry in DEFAULT_PATH_DIRS {
        let path = PathBuf::from(entry);
        if !normalized.contains(&path) {
            normalized.push(path);
        }
    }

    let Ok(joined) = env::join_paths(&normalized) else {
        report
            .notes
            .push("PATH normalization skipped because joined PATH was invalid".to_string());
        return;
    };
    let changed = original.as_ref() != Some(&joined);
    if changed {
        unsafe { env::set_var("PATH", &joined) };
        report.path_changed = true;
        report.final_path = Some(joined.to_string_lossy().to_string());
    } else if let Some(path) = original {
        report.final_path = Some(path.to_string_lossy().to_string());
    }
}

fn push_repair(
    report: &mut DoctorSessionEnvReport,
    key: impl Into<String>,
    source: impl Into<String>,
    value: Option<String>,
) {
    report.repaired.push(DoctorSessionEnvRepair {
        key: key.into(),
        source: source.into(),
        value,
    });
}

pub fn session_env_diagnostic(
    report: &DoctorSessionEnvReport,
) -> Option<sky_cua_platform::model::DiagnosticEntry> {
    if !report.changed() {
        return None;
    }
    let keys = report
        .repaired
        .iter()
        .map(|repair| format!("{}:{}", repair.key, repair.source))
        .collect::<Vec<_>>()
        .join(", ");
    let details = if report.path_changed && !keys.is_empty() {
        Some(format!("repaired={keys}; PATH normalized"))
    } else if report.path_changed {
        Some("PATH normalized".to_string())
    } else if !keys.is_empty() {
        Some(format!("repaired={keys}"))
    } else {
        None
    };
    Some(sky_cua_platform::model::DiagnosticEntry {
        code: "SessionEnvRepaired".to_string(),
        message:
            "Repaired missing Linux desktop session environment for detached Computer Use launch."
                .to_string(),
        details,
    })
}

pub fn required_session_env_present() -> bool {
    let has_display = env_var("DISPLAY").is_some() || env_var("WAYLAND_DISPLAY").is_some();
    let has_runtime = env_var("XDG_RUNTIME_DIR").is_some();
    let has_bus = dbus_session_address().is_some();
    has_display && has_runtime && has_bus
}

fn desktop_process_environments() -> Vec<HashMap<String, String>> {
    let mut environments = Vec::new();
    let mut pid = parent_pid("self");

    for _ in 0..8 {
        let Some(current_pid) = pid else {
            break;
        };
        if current_pid <= 1 {
            break;
        }

        if let Some(process_env) = read_process_environ(current_pid) {
            environments.push(process_env);
        }
        pid = parent_pid(&current_pid.to_string());
    }

    environments
}

fn parent_pid(pid: &str) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    parse_parent_pid(&status)
}

fn parse_parent_pid(status: &str) -> Option<u32> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix("PPid:")?.trim();
        value.parse::<u32>().ok()
    })
}

fn read_process_environ(pid: u32) -> Option<HashMap<String, String>> {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(parse_environ(&bytes))
}

fn parse_environ(bytes: &[u8]) -> HashMap<String, String> {
    bytes
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            if entry.is_empty() {
                return None;
            }
            let split = entry.iter().position(|byte| *byte == b'=')?;
            let (key, value) = entry.split_at(split);
            let value = &value[1..];
            let key = std::str::from_utf8(key).ok()?.to_string();
            let value = std::str::from_utf8(value).ok()?.to_string();
            Some((key, value))
        })
        .collect()
}

pub fn env_var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

pub fn xdg_runtime_dir() -> Option<PathBuf> {
    if let Some(value) = env_var("XDG_RUNTIME_DIR") {
        return Some(PathBuf::from(value));
    }
    user_id().map(|uid| PathBuf::from(format!("/run/user/{uid}")))
}

pub fn dbus_session_address() -> Option<String> {
    if let Some(value) = env_var("DBUS_SESSION_BUS_ADDRESS") {
        return Some(value);
    }
    xdg_runtime_dir()
        .map(|runtime| format!("unix:path={}", runtime.join("bus").display()))
        .filter(|address| {
            address
                .strip_prefix("unix:path=")
                .is_some_and(|p| Path::new(p).exists())
        })
}

fn user_id() -> Option<String> {
    let output = Command::new("id").arg("-u").output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::*;

    struct EnvRestore {
        values: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvRestore {
        fn capture(keys: &[&'static str]) -> Self {
            Self {
                values: keys
                    .iter()
                    .map(|key| (*key, std::env::var_os(key)))
                    .collect(),
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (key, value) in self.values.drain(..) {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }

    #[test]
    fn parses_parent_pid_from_proc_status() {
        let status = "Name:\ttest\nPid:\t42\nPPid:\t7\n";
        assert_eq!(parse_parent_pid(status), Some(7));
    }

    #[test]
    fn parse_parent_pid_returns_none_when_ppidi_missing() {
        let status = "Name:\ttest\nPid:\t42\n";
        assert_eq!(parse_parent_pid(status), None);
    }

    #[test]
    fn parses_nul_separated_process_environment() {
        let environment = parse_environ(
            b"DISPLAY=:0\0WAYLAND_DISPLAY=wayland-0\0EMPTY=\0NO_EQUALS\0XDG_SESSION_TYPE=wayland\0",
        );

        assert_eq!(environment.get("DISPLAY").map(String::as_str), Some(":0"));
        assert_eq!(
            environment.get("WAYLAND_DISPLAY").map(String::as_str),
            Some("wayland-0")
        );
        assert_eq!(environment.get("EMPTY").map(String::as_str), Some(""));
        assert!(!environment.contains_key("NO_EQUALS"));
    }

    #[test]
    fn parse_environ_handles_empty_input() {
        let environment = parse_environ(b"");
        assert!(environment.is_empty());
    }

    #[test]
    fn parse_environ_skips_invalid_utf8() {
        let environment = parse_environ(b"VALID=yes\0INVALID=\xff\0");
        assert_eq!(environment.get("VALID").map(String::as_str), Some("yes"));
        assert_eq!(environment.len(), 1);
    }

    #[test]
    #[serial]
    fn client_launch_repairs_are_reported_without_mutating_again() {
        let _restore = EnvRestore::capture(&[CLIENT_SESSION_ENV_REPAIRS_ENV]);
        let repairs = vec![
            DoctorSessionEnvRepair {
                key: "XDG_SESSION_TYPE".to_string(),
                source: "ignored".to_string(),
                value: Some("wayland".to_string()),
            },
            DoctorSessionEnvRepair {
                key: "XAUTHORITY".to_string(),
                source: "ignored".to_string(),
                value: Some("/run/user/1000/xpra/Xauthority".to_string()),
            },
            DoctorSessionEnvRepair {
                key: "PATH".to_string(),
                source: "ignored".to_string(),
                value: Some("/custom/bin:/usr/bin".to_string()),
            },
            DoctorSessionEnvRepair {
                key: "UNRELATED".to_string(),
                source: "ignored".to_string(),
                value: Some("nope".to_string()),
            },
        ];
        unsafe {
            std::env::set_var(
                CLIENT_SESSION_ENV_REPAIRS_ENV,
                serde_json::to_string(&repairs).expect("repairs should serialize"),
            );
        }
        let mut report = DoctorSessionEnvReport::default();

        merge_client_launch_repairs(&mut report);

        assert_eq!(report.repaired.len(), 2);
        assert_eq!(report.repaired[0].key, "XDG_SESSION_TYPE");
        assert_eq!(report.repaired[0].source, "client-launch");
        assert_eq!(report.repaired[0].value.as_deref(), Some("wayland"));
        assert_eq!(report.repaired[1].key, "XAUTHORITY");
        assert_eq!(report.repaired[1].source, "client-launch");
        assert_eq!(
            report.repaired[1].value.as_deref(),
            Some("/run/user/1000/xpra/Xauthority")
        );
    }

    #[test]
    #[serial]
    fn client_cleared_graphical_keys_filters_to_session_keys() {
        let _restore = EnvRestore::capture(&[CLIENT_CLEARED_SESSION_ENV_KEYS_ENV]);
        unsafe {
            std::env::set_var(
                CLIENT_CLEARED_SESSION_ENV_KEYS_ENV,
                serde_json::to_string(&vec!["DISPLAY", "UNRELATED", "WAYLAND_DISPLAY"])
                    .expect("keys should serialize"),
            );
        }

        let cleared = client_cleared_graphical_keys();

        assert!(cleared.contains("DISPLAY"));
        assert!(cleared.contains("WAYLAND_DISPLAY"));
        assert!(!cleared.contains("UNRELATED"));
    }

    #[test]
    fn parses_systemd_show_environment_output() {
        let environment = parse_systemd_environment(
            b"DISPLAY=:1\nWAYLAND_DISPLAY=wayland-1\nBAD LINE=value\nEMPTY=\nNO_EQUALS\n",
        );

        assert_eq!(environment.get("DISPLAY").map(String::as_str), Some(":1"));
        assert_eq!(
            environment.get("WAYLAND_DISPLAY").map(String::as_str),
            Some("wayland-1")
        );
        assert_eq!(environment.get("EMPTY").map(String::as_str), Some(""));
        assert!(!environment.contains_key("BAD LINE"));
        assert!(!environment.contains_key("NO_EQUALS"));
    }

    #[test]
    #[serial]
    fn systemd_map_fills_missing_values_without_overwriting_existing_env() {
        let _restore = EnvRestore::capture(&["DISPLAY", "WAYLAND_DISPLAY"]);
        unsafe {
            std::env::set_var("DISPLAY", ":existing");
            std::env::remove_var("WAYLAND_DISPLAY");
        }
        let mut report = DoctorSessionEnvReport::default();
        let values = HashMap::from([
            ("DISPLAY".to_string(), ":systemd".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "wayland-7".to_string()),
        ]);

        hydrate_desktop_env_from_map(&mut report, "systemd-user", &values, &HashSet::new());

        assert_eq!(std::env::var("DISPLAY").ok().as_deref(), Some(":existing"));
        assert_eq!(
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            Some("wayland-7")
        );
        assert_eq!(report.repaired.len(), 1);
        assert_eq!(report.repaired[0].key, "WAYLAND_DISPLAY");
        assert_eq!(report.repaired[0].source, "systemd-user");
    }

    #[test]
    #[serial]
    fn map_hydration_skips_client_cleared_keys() {
        let _restore = EnvRestore::capture(&["DISPLAY", "WAYLAND_DISPLAY"]);
        unsafe {
            std::env::remove_var("DISPLAY");
            std::env::remove_var("WAYLAND_DISPLAY");
        }
        let mut report = DoctorSessionEnvReport::default();
        let values = HashMap::from([
            ("DISPLAY".to_string(), "localhost:10.0".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string()),
        ]);
        let blocked_keys = HashSet::from(["DISPLAY".to_string()]);

        hydrate_desktop_env_from_map(&mut report, "process-tree", &values, &blocked_keys);

        assert_eq!(std::env::var("DISPLAY").ok(), None);
        assert_eq!(
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            Some("wayland-0")
        );
        assert_eq!(report.repaired.len(), 1);
        assert_eq!(report.repaired[0].key, "WAYLAND_DISPLAY");
    }

    #[test]
    #[serial]
    fn normalize_path_removes_duplicates_and_appends_default_dirs() {
        let _restore = EnvRestore::capture(&["PATH"]);
        unsafe {
            std::env::set_var("PATH", "/tmp:/usr/bin:/tmp");
        }
        let mut report = DoctorSessionEnvReport::default();

        normalize_path(&mut report);

        let path = std::env::var("PATH").expect("PATH should be set");
        let parts = std::env::split_paths(&path).collect::<Vec<_>>();
        assert_eq!(parts[0], PathBuf::from("/tmp"));
        assert_eq!(
            parts
                .iter()
                .filter(|entry| *entry == &PathBuf::from("/tmp"))
                .count(),
            1
        );
        assert!(parts.contains(&PathBuf::from("/usr/local/bin")));
        assert!(parts.contains(&PathBuf::from("/bin")));
        assert!(report.path_changed);
        assert_eq!(report.final_path.as_deref(), Some(path.as_str()));
    }

    #[test]
    fn session_env_diagnostic_is_absent_when_nothing_changed() {
        assert!(session_env_diagnostic(&DoctorSessionEnvReport::default()).is_none());
    }

    #[test]
    #[serial]
    fn hydrate_isolated_sandbox_keeps_wayland_absent_and_dbus_private() {
        let _restore = EnvRestore::capture(&[
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_SESSION_TYPE",
            "QT_QPA_PLATFORM",
            "GDK_BACKEND",
            "DBUS_SESSION_BUS_ADDRESS",
            "XDG_RUNTIME_DIR",
        ]);
        // Isolated xpra sandbox: DISPLAY=:90, x11, toolkit forced to xcb, WAYLAND_DISPLAY intentionally removed
        unsafe {
            std::env::set_var("DISPLAY", ":90");
            std::env::set_var("XDG_SESSION_TYPE", "x11");
            std::env::set_var("QT_QPA_PLATFORM", "xcb");
            std::env::set_var("GDK_BACKEND", "x11");
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/tmp/dbus-isolated-test,guid=abc",
            );
            std::env::set_var("XDG_RUNTIME_DIR", "/tmp");
        }
        // Create the private bus socket file so is_env_value_live considers it live
        let _ = std::fs::File::create("/tmp/dbus-isolated-test");
        let mut report = DoctorSessionEnvReport::default();
        let values = HashMap::from([
            ("DISPLAY".to_string(), ":1".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "wayland-1".to_string()),
            (
                "DBUS_SESSION_BUS_ADDRESS".to_string(),
                "unix:path=/run/user/1000/bus".to_string(),
            ),
            ("XDG_SESSION_TYPE".to_string(), "wayland".to_string()),
        ]);
        let blocked = HashSet::from(["WAYLAND_DISPLAY".to_string()]);

        hydrate_desktop_env_from_map(&mut report, "active-session", &values, &blocked);

        assert_eq!(std::env::var("DISPLAY").ok().as_deref(), Some(":90"));
        assert_eq!(std::env::var("WAYLAND_DISPLAY").ok(), None);
        assert_eq!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
            Some("unix:path=/tmp/dbus-isolated-test,guid=abc")
        );
        // Only XDG_SESSION_TYPE would be considered for overwrite but it's live (x11), so no repair
        assert!(report.repaired.is_empty());
        let _ = std::fs::remove_file("/tmp/dbus-isolated-test");
    }

    #[test]
    #[serial]
    fn hydrate_shared_detached_corrects_display_when_x0_dead() {
        let _restore = EnvRestore::capture(&["DISPLAY", "WAYLAND_DISPLAY", "XDG_SESSION_TYPE"]);
        // Use high numbers not owned by host (X0/X1 are host sockets owned by root/bex)
        let x99 = PathBuf::from("/tmp/.X11-unix/X99");
        let x100 = PathBuf::from("/tmp/.X11-unix/X100");
        let had_x99 = x99.exists();
        let had_x100 = x100.exists();
        if had_x99 {
            let _ = std::fs::remove_file(&x99);
        }
        // X99 dead, X100 live
        let _ = std::fs::File::create(&x100);
        unsafe {
            std::env::set_var("DISPLAY", ":99");
            std::env::set_var("XDG_SESSION_TYPE", "wayland");
            std::env::remove_var("WAYLAND_DISPLAY");
        }
        let mut report = DoctorSessionEnvReport::default();
        let values = HashMap::from([
            ("DISPLAY".to_string(), ":100".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "wayland-1".to_string()),
        ]);
        let blocked = HashSet::from(["DISPLAY".to_string(), "WAYLAND_DISPLAY".to_string()]);

        hydrate_desktop_env_from_map(&mut report, "active-session", &values, &blocked);

        // X99 dead → should correct to :100 even though blocked and present
        assert_eq!(std::env::var("DISPLAY").ok().as_deref(), Some(":100"));
        // WAYLAND_DISPLAY was missing but blocked; active-session fills it for shared (x11 guard not triggered because wayland)
        assert_eq!(
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            Some("wayland-1")
        );
        assert!(report.repaired.iter().any(|r| r.key == "DISPLAY"));
        // Cleanup
        let _ = std::fs::remove_file(&x100);
        if had_x99 {
            let _ = std::fs::File::create(&x99);
        }
        if !had_x100 {
            let _ = std::fs::remove_file(&x100);
        }
    }

    #[test]
    #[serial]
    fn hydrate_shared_detached_keeps_display_when_x0_live() {
        let _restore = EnvRestore::capture(&["DISPLAY"]);
        let x99 = PathBuf::from("/tmp/.X11-unix/X99");
        let had_x99 = x99.exists();
        let _ = std::fs::File::create(&x99);
        unsafe {
            std::env::set_var("DISPLAY", ":99");
        }
        let mut report = DoctorSessionEnvReport::default();
        let values = HashMap::from([("DISPLAY".to_string(), ":100".to_string())]);
        let blocked = HashSet::from(["DISPLAY".to_string()]);

        hydrate_desktop_env_from_map(&mut report, "active-session", &values, &blocked);

        // X99 live → must not flip to :100 even though candidate differs
        assert_eq!(std::env::var("DISPLAY").ok().as_deref(), Some(":99"));
        assert!(report.repaired.is_empty());
        if !had_x99 {
            let _ = std::fs::remove_file(&x99);
        }
    }
}
