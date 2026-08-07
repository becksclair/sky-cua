use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use sky_cua_platform::{
    BROWSER_ENV_HEALTH_KEYS, DESKTOP_LAUNCH_ENV_KEYS, GRAPHICAL_SESSION_ENV_KEYS,
    model::ServiceResponse,
};

const DEFAULT_PATH_DIRS: &[&str] = &[
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

#[derive(Debug, Clone)]
pub(crate) struct LaunchEnvironment {
    repaired_desktop_vars: Vec<(String, String)>,
    detached_graphical_env: bool,
}

impl LaunchEnvironment {
    pub(crate) fn probe() -> Self {
        let detached_graphical_env = remote_or_detached_launch();
        Self {
            repaired_desktop_vars: probe_desktop_env_vars(detached_graphical_env),
            detached_graphical_env,
        }
    }

    pub(crate) fn repaired_desktop_vars(&self) -> &[(String, String)] {
        &self.repaired_desktop_vars
    }

    pub(crate) fn repaired_desktop_var(&self, key: &str) -> Option<&str> {
        self.repaired_desktop_vars
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn detached_graphical_env(&self) -> bool {
        self.detached_graphical_env
    }

    pub(crate) fn ensure_startup_health(
        &self,
        response: &ServiceResponse,
        require_desktop_match: bool,
    ) -> Result<()> {
        if require_desktop_match {
            ensure_health_satisfies_desktop_env(
                response,
                &self.repaired_desktop_vars,
                !self.detached_graphical_env,
            )?;
        }
        ensure_health_satisfies_browser_env(response)
    }

    /// Build a launch environment whose health-equality expectations are the
    /// isolated daemon's *sandbox* graphical session, not the client's host
    /// session. In isolated mode the daemon legitimately runs under
    /// `DISPLAY=:N`, `XDG_SESSION_TYPE=x11`, the sandbox D-Bus address, and with
    /// `WAYLAND_DISPLAY` removed; the default `probe()` environment would demand
    /// the daemon match the *client's* (host Wayland) values and reject the
    /// sandboxed daemon forever. `spawn_env` is the handle's daemon spawn env;
    /// `removed_env` are the keys cleared on the daemon (e.g. `WAYLAND_DISPLAY`),
    /// which must therefore be expected absent. `detached_graphical_env` is set
    /// so the host-process graphical vars are not consulted for the comparison.
    #[cfg(unix)]
    pub(crate) fn for_isolated_daemon(
        spawn_env: &[(String, String)],
        removed_env: &[&'static str],
    ) -> Self {
        let repaired_desktop_vars = spawn_env
            .iter()
            .filter(|(key, _)| DESKTOP_LAUNCH_ENV_KEYS.contains(&key.as_str()))
            .filter(|(key, _)| !removed_env.contains(&key.as_str()))
            .cloned()
            .collect();
        Self {
            repaired_desktop_vars,
            // Skip seeding the required set from the client's live process env;
            // the sandbox repaired vars above carry the authoritative
            // expectations for the isolated daemon.
            detached_graphical_env: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_repaired_desktop_vars_for_tests(
        repaired_desktop_vars: Vec<(String, String)>,
    ) -> Self {
        Self {
            repaired_desktop_vars,
            detached_graphical_env: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_repaired_desktop_vars_and_detached_for_tests(
        repaired_desktop_vars: Vec<(String, String)>,
        detached_graphical_env: bool,
    ) -> Self {
        Self {
            repaired_desktop_vars,
            detached_graphical_env,
        }
    }
}

/// Probe the active user session for missing desktop environment variables.
/// When a host spawns the MCP server without forwarding the full desktop
/// session (e.g., via systemd unit, remote SSH, or container entrypoint),
/// the service backends cannot initialize. This function attempts to
/// reconstruct the missing values from common well-known sources.
#[must_use]
fn probe_desktop_env_vars(remote_or_detached: bool) -> Vec<(String, String)> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }

    let mut found = Vec::new();
    let should_repair = |key: &str| {
        env_missing_or_empty(key)
            || (remote_or_detached && GRAPHICAL_SESSION_ENV_KEYS.contains(&key))
    };

    if let Some(session_env) =
        graphical_session_environment().or_else(graphical_process_environment)
    {
        let replacing_display = remote_or_detached
            && session_env
                .get("DISPLAY")
                .is_some_and(|value| !value.trim().is_empty());
        for key in DESKTOP_LAUNCH_ENV_KEYS {
            if *key == "PATH" {
                continue;
            }
            if (should_repair(key) || (*key == "XAUTHORITY" && replacing_display))
                && let Some(value) = session_env
                    .get(*key)
                    .filter(|value| !value.trim().is_empty())
            {
                push_repaired_var(&mut found, *key, value.clone());
            }
        }
        if has_repaired_var(&found, "DISPLAY") {
            push_repaired_var(&mut found, "NO_AT_BRIDGE", "0");
            push_repaired_var(&mut found, "ACCESSIBILITY_ENABLED", "1");
        }
    }

    if should_repair("XDG_RUNTIME_DIR") && !has_repaired_var(&found, "XDG_RUNTIME_DIR") {
        let uid = current_uid();
        let candidate = PathBuf::from(format!("/run/user/{uid}"));
        if candidate.is_dir() {
            push_repaired_var(&mut found, "XDG_RUNTIME_DIR", candidate.to_string_lossy());
        }
    }

    if should_repair("DBUS_SESSION_BUS_ADDRESS")
        && !has_repaired_var(&found, "DBUS_SESSION_BUS_ADDRESS")
        && let Some(runtime_dir) = found
            .iter()
            .find(|(k, _)| k == "XDG_RUNTIME_DIR")
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
    {
        let socket = runtime_dir.join("bus");
        if socket.exists() {
            push_repaired_var(
                &mut found,
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", socket.display()),
            );
        }
    }

    if should_repair("WAYLAND_DISPLAY")
        && !has_repaired_var(&found, "WAYLAND_DISPLAY")
        && let Some(runtime_dir) = found
            .iter()
            .find(|(k, _)| k == "XDG_RUNTIME_DIR")
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
    {
        for name in &["wayland-0", "wayland-1", "wayland-2"] {
            if runtime_dir.join(name).exists() {
                push_repaired_var(&mut found, "WAYLAND_DISPLAY", (*name).to_string());
                break;
            }
        }
    }

    if should_repair("DISPLAY")
        && !has_repaired_var(&found, "DISPLAY")
        && let Some(display) = probe_x11_display()
    {
        push_repaired_var(&mut found, "DISPLAY", display);
    }

    if should_repair("XDG_CURRENT_DESKTOP")
        && !has_repaired_var(&found, "XDG_CURRENT_DESKTOP")
        && let Ok(output) = std::process::Command::new("loginctl")
            .args([
                "show-session",
                "self",
                "--property=DesktopEnvironment",
                "--value",
            ])
            .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() && stdout != "n/a" {
            push_repaired_var(&mut found, "XDG_CURRENT_DESKTOP", stdout);
        }
    }

    if should_repair("XDG_SESSION_TYPE") && !has_repaired_var(&found, "XDG_SESSION_TYPE") {
        let wayland_present = repaired_or_env_present(&found, "WAYLAND_DISPLAY");
        let display_present = repaired_or_env_present(&found, "DISPLAY");

        if wayland_present {
            push_repaired_var(&mut found, "XDG_SESSION_TYPE", "wayland");
        } else if display_present {
            push_repaired_var(&mut found, "XDG_SESSION_TYPE", "x11");
        }
    }

    if let Some(systemd_env) = systemd_user_environment() {
        for key in DESKTOP_LAUNCH_ENV_KEYS {
            if *key == "PATH" {
                continue;
            }
            if should_repair(key)
                && systemd_fallback_key_allowed(key, remote_or_detached)
                && let Some(value) = systemd_env
                    .get(*key)
                    .filter(|value| !value.trim().is_empty())
                && !found.iter().any(|(found_key, _)| found_key == key)
            {
                push_repaired_var(&mut found, *key, value.clone());
            }
        }
    }

    if let Some(path) = normalized_path() {
        found.push(("PATH".to_string(), path));
    }

    found
}

fn systemd_fallback_key_allowed(key: &str, remote_or_detached: bool) -> bool {
    !remote_or_detached || SELECTED_SESSION_SYSTEMD_FILL_KEYS.contains(&key)
}

fn push_repaired_var(
    found: &mut Vec<(String, String)>,
    key: impl Into<String>,
    value: impl Into<String>,
) {
    let key = key.into();
    let value = value.into();
    if let Some((_, existing)) = found
        .iter_mut()
        .find(|(existing_key, _)| existing_key == &key)
    {
        *existing = value;
    } else {
        found.push((key, value));
    }
}

fn has_repaired_var(found: &[(String, String)], key: &str) -> bool {
    found.iter().any(|(found_key, _)| found_key == key)
}

fn repaired_or_env_present(found: &[(String, String)], key: &str) -> bool {
    has_repaired_var(found, key) || !env_missing_or_empty(key)
}

fn remote_or_detached_launch() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var_os("SSH_TTY").is_some()
        || std::env::var_os("TMUX").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .is_ok_and(|value| matches!(value.as_str(), "tty" | "unspecified"))
        || std::env::var("DISPLAY").is_ok_and(|value| forwarded_x11_display(&value))
        || env_missing_or_empty("XDG_RUNTIME_DIR")
        || (env_missing_or_empty("DISPLAY") && env_missing_or_empty("WAYLAND_DISPLAY"))
}

fn env_missing_or_empty(key: &str) -> bool {
    std::env::var_os(key).is_none_or(|value| value.is_empty())
}

fn forwarded_x11_display(value: &str) -> bool {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() || trimmed.starts_with(':') || trimmed.starts_with("unix:") {
        return false;
    }
    let host = if let Some(rest) = trimmed.strip_prefix('[') {
        let Some((host, _)) = rest.split_once("]:") else {
            return false;
        };
        host
    } else {
        let Some((host, _)) = trimmed.split_once(':') else {
            return false;
        };
        host
    };
    host == "localhost" || host == "localhost/unix" || host == "::1" || host.starts_with("127.")
}

fn graphical_session_environment() -> Option<HashMap<String, String>> {
    let session = selected_graphical_session()?;
    let mut environment = HashMap::new();
    if let Some(process_env) = session.leader.and_then(read_process_environ) {
        environment.extend(process_env);
    }
    if let Some(systemd_env) = systemd_user_environment() {
        fill_missing_selected_session_systemd_environment(&mut environment, systemd_env);
    }
    apply_logind_session_metadata(&mut environment, &session);
    if environment.is_empty() {
        None
    } else {
        Some(environment)
    }
}

fn graphical_process_environment() -> Option<HashMap<String, String>> {
    graphical_process_environment_from(Path::new("/proc"), Path::new("/tmp/.X11-unix"))
}

fn graphical_process_environment_from(
    proc_root: &Path,
    x11_socket_root: &Path,
) -> Option<HashMap<String, String>> {
    let mut candidates = std::fs::read_dir(proc_root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .filter_map(|pid| read_process_environ_from(proc_root, pid).map(|env| (pid, env)))
        .filter(|(_, env)| graphical_process_environment_is_live(env, x11_socket_root))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(pid, env)| (graphical_process_environment_rank(env), *pid));
    candidates.pop().map(|(_, env)| env)
}

fn graphical_process_environment_is_live(
    environment: &HashMap<String, String>,
    x11_socket_root: &Path,
) -> bool {
    let runtime_ok = environment
        .get("XDG_RUNTIME_DIR")
        .is_some_and(|value| Path::new(value).is_dir());
    let bus_ok = environment
        .get("DBUS_SESSION_BUS_ADDRESS")
        .is_some_and(|value| value.starts_with("unix:"));
    let x11_ok = environment
        .get("DISPLAY")
        .and_then(|value| local_x11_display_number(value))
        .is_some_and(|display| x11_socket_root.join(format!("X{display}")).exists());
    let wayland_ok = environment
        .get("WAYLAND_DISPLAY")
        .zip(environment.get("XDG_RUNTIME_DIR"))
        .is_some_and(|(display, runtime)| Path::new(runtime).join(display).exists());
    runtime_ok && bus_ok && (x11_ok || wayland_ok)
}

fn graphical_process_environment_rank(environment: &HashMap<String, String>) -> usize {
    DESKTOP_LAUNCH_ENV_KEYS
        .iter()
        .filter(|key| {
            environment
                .get(**key)
                .is_some_and(|value| !value.trim().is_empty())
        })
        .count()
}

fn local_x11_display_number(display: &str) -> Option<&str> {
    let value = display.trim();
    let value = value
        .strip_prefix("unix/:")
        .or_else(|| value.strip_prefix("unix:"))
        .or_else(|| value.strip_prefix(':'))?;
    let number = value.split('.').next()?;
    (!number.is_empty() && number.chars().all(|character| character.is_ascii_digit()))
        .then_some(number)
}

fn apply_logind_session_metadata(
    environment: &mut HashMap<String, String>,
    session: &LogindSession,
) {
    if let Some(session_type) = session
        .session_type
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        environment.insert("XDG_SESSION_TYPE".to_string(), session_type.clone());
    }
    if let Some(desktop) = session.desktop.as_ref().filter(|value| !value.is_empty()) {
        // logind's Desktop field names the desktop environment (for example
        // "KDE"), which is valid XDG_CURRENT_DESKTOP data but is not the
        // display manager's DESKTOP_SESSION value (for Plasma that is commonly
        // "plasma"). Inventing DESKTOP_SESSION from Desktop makes detached MCP
        // clients reject an otherwise healthy shared daemon.
        insert_if_missing_or_empty(environment, "XDG_CURRENT_DESKTOP", desktop.clone());
    }
    if let Some(display) = session.display.as_ref().filter(|value| !value.is_empty()) {
        insert_if_missing_or_empty(environment, "DISPLAY", display.clone());
    }
}

fn insert_if_missing_or_empty(environment: &mut HashMap<String, String>, key: &str, value: String) {
    if environment
        .get(key)
        .is_none_or(|existing| existing.is_empty())
    {
        environment.insert(key.to_string(), value);
    }
}

const SELECTED_SESSION_SYSTEMD_FILL_KEYS: &[&str] =
    &["DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR"];

fn fill_missing_selected_session_systemd_environment(
    environment: &mut HashMap<String, String>,
    fallback: HashMap<String, String>,
) {
    for (key, value) in fallback {
        if !SELECTED_SESSION_SYSTEMD_FILL_KEYS.contains(&key.as_str()) {
            continue;
        }
        environment.entry(key).or_insert(value);
    }
}

#[derive(Debug, Clone, Default)]
struct LogindSession {
    id: String,
    uid: Option<u32>,
    remote: bool,
    active: bool,
    state: Option<String>,
    class: Option<String>,
    session_type: Option<String>,
    desktop: Option<String>,
    display: Option<String>,
    leader: Option<u32>,
}

fn selected_graphical_session() -> Option<LogindSession> {
    let output = std::process::Command::new("loginctl")
        .args(["list-sessions", "--no-legend"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let session_ids = parse_loginctl_session_ids(&String::from_utf8_lossy(&output.stdout));
    let current_uid = current_uid();
    let mut candidates = session_ids
        .into_iter()
        .filter_map(show_logind_session)
        .filter(|session| session.uid == Some(current_uid))
        .filter(is_graphical_logind_session)
        .collect::<Vec<_>>();
    candidates.sort_by_key(logind_session_rank);
    candidates.pop()
}

fn parse_loginctl_session_ids(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

fn show_logind_session(id: String) -> Option<LogindSession> {
    let output = std::process::Command::new("loginctl")
        .args([
            "show-session",
            &id,
            "-p",
            "Id",
            "-p",
            "User",
            "-p",
            "Type",
            "-p",
            "Class",
            "-p",
            "State",
            "-p",
            "Active",
            "-p",
            "Remote",
            "-p",
            "Desktop",
            "-p",
            "Display",
            "-p",
            "Leader",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_logind_session_properties(
        &id,
        &String::from_utf8_lossy(&output.stdout),
    ))
}

fn parse_logind_session_properties(id: &str, output: &str) -> LogindSession {
    let values = parse_key_value_lines(output);
    LogindSession {
        id: values
            .get("Id")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| id.to_string()),
        uid: values
            .get("User")
            .and_then(|value| value.parse::<u32>().ok()),
        remote: values.get("Remote").is_some_and(|value| value == "yes"),
        active: values.get("Active").is_some_and(|value| value == "yes"),
        state: values
            .get("State")
            .filter(|value| !value.is_empty())
            .cloned(),
        class: values
            .get("Class")
            .filter(|value| !value.is_empty())
            .cloned(),
        session_type: values
            .get("Type")
            .filter(|value| !value.is_empty())
            .cloned(),
        desktop: values
            .get("Desktop")
            .filter(|value| !value.is_empty())
            .cloned(),
        display: values
            .get("Display")
            .filter(|value| !value.is_empty())
            .cloned(),
        leader: values
            .get("Leader")
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|leader| *leader > 1),
    }
}

fn parse_key_value_lines(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            let split = line.find('=')?;
            let (key, value) = line.split_at(split);
            if key.is_empty() || key.chars().any(char::is_whitespace) {
                return None;
            }
            Some((key.to_string(), value[1..].to_string()))
        })
        .collect()
}

fn is_graphical_logind_session(session: &LogindSession) -> bool {
    if session.remote || session.class.as_deref() == Some("manager") {
        return false;
    }
    matches!(
        session.session_type.as_deref(),
        Some("wayland" | "x11" | "mir")
    ) || session
        .desktop
        .as_ref()
        .is_some_and(|value| !value.is_empty())
        || session
            .display
            .as_ref()
            .is_some_and(|value| !value.is_empty())
}

fn logind_session_rank(session: &LogindSession) -> (u8, u8, u8, u8, String) {
    let active = u8::from(session.active);
    let running = u8::from(matches!(
        session.state.as_deref(),
        Some("active" | "online")
    ));
    let typed = u8::from(matches!(
        session.session_type.as_deref(),
        Some("wayland" | "x11" | "mir")
    ));
    let has_desktop = u8::from(session.desktop.is_some());
    (active, running, typed, has_desktop, session.id.clone())
}

fn read_process_environ(pid: u32) -> Option<HashMap<String, String>> {
    read_process_environ_from(Path::new("/proc"), pid)
}

fn read_process_environ_from(proc_root: &Path, pid: u32) -> Option<HashMap<String, String>> {
    let bytes = std::fs::read(proc_root.join(pid.to_string()).join("environ")).ok()?;
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
            let key = std::str::from_utf8(key).ok()?.to_string();
            let value = std::str::from_utf8(&value[1..]).ok()?.to_string();
            Some((key, value))
        })
        .collect()
}

fn systemd_user_environment() -> Option<HashMap<String, String>> {
    let output = std::process::Command::new(systemctl_path())
        .args(["--user", "show-environment"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_systemd_user_environment(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_systemd_user_environment(output: &str) -> HashMap<String, String> {
    parse_key_value_lines(output)
}

fn systemctl_path() -> &'static str {
    if Path::new("/usr/bin/systemctl").is_file() {
        "/usr/bin/systemctl"
    } else {
        "systemctl"
    }
}

fn normalized_path() -> Option<String> {
    let original = std::env::var_os("PATH");
    let mut paths = Vec::<PathBuf>::new();
    if let Some(path) = original.as_ref() {
        for entry in std::env::split_paths(path) {
            if entry.as_os_str().is_empty() || paths.contains(&entry) {
                continue;
            }
            paths.push(entry);
        }
    }
    for entry in DEFAULT_PATH_DIRS {
        let path = PathBuf::from(entry);
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    let joined = std::env::join_paths(paths).ok()?;
    (original.as_ref() != Some(&joined)).then(|| joined.to_string_lossy().to_string())
}

fn probe_x11_display() -> Option<String> {
    x11_display_from_socket_root(Path::new("/tmp/.X11-unix"))
}

fn x11_display_from_socket_root(socket_root: &Path) -> Option<String> {
    for display in 0..=2 {
        if socket_root.join(format!("X{display}")).exists() {
            return Some(format!(":{display}"));
        }
    }
    None
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: `geteuid` is a simple libc query with no preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

fn ensure_health_satisfies_desktop_env(
    response: &ServiceResponse,
    desktop_vars: &[(String, String)],
    trust_client_graphical_env: bool,
) -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }

    let mut required = if trust_client_graphical_env {
        GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .copied()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .filter(|value| !value.is_empty())
                    .map(|value| (key, value))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    // Health equality includes every value this client explicitly repaired
    // except PATH. Hosts legitimately have divergent PATHs, so requiring the
    // daemon's PATH to equal this client's would make a daemon spawned by one
    // host permanently unhealthy for all others.
    for (key, value) in desktop_vars {
        let key = key.as_str();
        if key != "PATH" && DESKTOP_LAUNCH_ENV_KEYS.contains(&key) && !value.is_empty() {
            required.retain(|(required_key, _)| *required_key != key);
            required.push((key, value.clone()));
        }
    }
    if required.is_empty() {
        return Ok(());
    }

    let ServiceResponse::Health { desktop_env, .. } = response else {
        return Ok(());
    };
    let missing = required
        .into_iter()
        .filter(|(key, value)| desktop_env.get(*key) != Some(value))
        .map(|(key, _)| key)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    Err(anyhow!(
        "existing sky-cua-service is missing repaired desktop environment keys: {}",
        missing.join(", ")
    ))
}

fn ensure_health_satisfies_browser_env(response: &ServiceResponse) -> Result<()> {
    if !cfg!(unix) {
        return Ok(());
    }

    let desired = browser_env_values_present();
    let ServiceResponse::Health { browser_env, .. } = response else {
        return Ok(());
    };

    // Only an explicit conflict is stale: both sides pinning different
    // values. An unset side is compatible — unset selection means "probe
    // every Chrome-family browser" (a superset), and demanding exact
    // equality would let the first spawning host's environment permanently
    // starve every other host now that the daemon is a singleton.
    let stale = BROWSER_ENV_HEALTH_KEYS
        .iter()
        .copied()
        .filter(|key| {
            matches!(
                (browser_env.get(*key), desired.get(*key)),
                (Some(actual), Some(wanted)) if actual != wanted
            )
        })
        .collect::<Vec<_>>();
    if stale.is_empty() {
        return Ok(());
    }

    Err(anyhow!(
        "existing sky-cua-service has conflicting browser environment keys: {}",
        stale.join(", ")
    ))
}

fn browser_env_values_present() -> BTreeMap<String, String> {
    BROWSER_ENV_HEALTH_KEYS
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| ((*key).to_string(), value))
        })
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn discovers_detached_xpra_graphical_process_environment() {
        let tmp_path = std::env::temp_dir().join(format!(
            "sky-cua-xpra-env-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let proc_root = tmp_path.join("proc");
        let socket_root = tmp_path.join("x11");
        let runtime = tmp_path.join("xpra-runtime");
        fs::create_dir_all(proc_root.join("42")).expect("fake proc pid should be created");
        fs::create_dir_all(&socket_root).expect("fake X11 socket root should be created");
        fs::create_dir_all(&runtime).expect("fake Xpra runtime should be created");
        fs::write(socket_root.join("X100"), b"").expect("fake X11 socket should be created");
        fs::write(runtime.join("Xauthority"), b"cookie")
            .expect("fake Xauthority should be created");
        fs::write(
            proc_root.join("42/environ"),
            format!(
                "DISPLAY=:100\0XAUTHORITY={}\0XDG_RUNTIME_DIR={}\0DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/dbus-xpra\0XDG_SESSION_TYPE=x11\0",
                runtime.join("Xauthority").display(),
                runtime.display(),
            ),
        )
        .expect("fake process environment should be created");

        let environment = graphical_process_environment_from(&proc_root, &socket_root)
            .expect("Xpra environment should be selected");

        assert_eq!(environment.get("DISPLAY").map(String::as_str), Some(":100"));
        assert_eq!(
            environment.get("XDG_RUNTIME_DIR").map(String::as_str),
            Some(runtime.to_string_lossy().as_ref())
        );
        assert_eq!(local_x11_display_number("unix:100.0"), Some("100"));
        assert_eq!(local_x11_display_number("unix/:100.0"), Some("100"));
        assert_eq!(local_x11_display_number("localhost:10.0"), None);
        fs::remove_dir_all(tmp_path).expect("fake process tree should be removed");
    }

    #[test]
    fn for_isolated_daemon_scopes_health_to_sandbox_graphical_identity() {
        // Pure (no process env): mirror IsolatedDesktopHandle::spawn_env() plus a
        // hypothetical WAYLAND_DISPLAY to prove the removed-env filter excludes it.
        let spawn_env = vec![
            ("DISPLAY".to_string(), ":131".to_string()),
            ("XDG_SESSION_TYPE".to_string(), "x11".to_string()),
            ("QT_QPA_PLATFORM".to_string(), "xcb".to_string()),
            ("GDK_BACKEND".to_string(), "x11".to_string()),
            (
                "DBUS_SESSION_BUS_ADDRESS".to_string(),
                "unix:path=/tmp/dbus-sandbox".to_string(),
            ),
            (
                "XAUTHORITY".to_string(),
                "/run/user/1000/xpra/Xauthority".to_string(),
            ),
            (
                "SKY_CUA_SERVICE_SOCKET_PATH".to_string(),
                "/run/user/1000/sky-cua/service-isolated-131.sock".to_string(),
            ),
            ("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string()),
        ];
        let removed_env: &[&'static str] = &["WAYLAND_DISPLAY"];

        let env = LaunchEnvironment::for_isolated_daemon(&spawn_env, removed_env);

        // Detached, so the client's host graphical vars are NOT folded into the
        // required set — otherwise the host's Wayland identity would reject the
        // sandboxed X11 daemon forever.
        assert!(env.detached_graphical_env());

        // The sandbox's graphical identity is exactly what the daemon must echo.
        assert_eq!(env.repaired_desktop_var("DISPLAY"), Some(":131"));
        assert_eq!(env.repaired_desktop_var("XDG_SESSION_TYPE"), Some("x11"));
        assert_eq!(
            env.repaired_desktop_var("DBUS_SESSION_BUS_ADDRESS"),
            Some("unix:path=/tmp/dbus-sandbox")
        );
        assert_eq!(
            env.repaired_desktop_var("XAUTHORITY"),
            Some("/run/user/1000/xpra/Xauthority")
        );

        // WAYLAND_DISPLAY is removed on the daemon, so it must NOT become a
        // health expectation even though it is a graphical key — this exclusion
        // is what closes the toolkit Wayland-escape (the daemon is expected to
        // run without Wayland).
        assert_eq!(env.repaired_desktop_var("WAYLAND_DISPLAY"), None);

        // Toolkit and socket vars outside DESKTOP_LAUNCH_ENV_KEYS are launch
        // material, not health identity, so they are excluded from the required
        // set.
        assert_eq!(env.repaired_desktop_var("QT_QPA_PLATFORM"), None);
        assert_eq!(env.repaired_desktop_var("GDK_BACKEND"), None);
        assert_eq!(
            env.repaired_desktop_var("SKY_CUA_SERVICE_SOCKET_PATH"),
            None
        );
    }

    #[test]
    fn startup_health_ignores_repaired_path_differences() {
        // PATH is launch-spawn repair material; hosts have divergent PATHs,
        // so a daemon spawned by one host must stay healthy for the others.
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let mut desktop_env = GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| ((*key).to_string(), value))
            })
            .collect::<BTreeMap<_, _>>();
        desktop_env.insert(
            "PATH".to_string(),
            "/daemon/spawner/path:/usr/bin".to_string(),
        );
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env,
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };
        let desktop_vars = vec![(
            "PATH".to_string(),
            "/client/specific/path:/usr/bin".to_string(),
        )];

        assert!(ensure_health_satisfies_desktop_env(&response, &desktop_vars, true).is_ok());
    }

    #[test]
    fn startup_health_rejects_service_missing_repaired_desktop_env() {
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::from([("DISPLAY".to_string(), ":0".to_string())]),
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };
        let desktop_vars = vec![
            ("DISPLAY".to_string(), ":0".to_string()),
            ("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string()),
        ];

        let error = ensure_health_satisfies_desktop_env(&response, &desktop_vars, true)
            .expect_err("stale service env should be rejected");

        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }

    #[test]
    fn startup_health_rejects_service_missing_spawn_only_repairs() {
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::from([("DISPLAY".to_string(), ":100".to_string())]),
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };
        let desktop_vars = vec![
            ("DISPLAY".to_string(), ":100".to_string()),
            (
                "XAUTHORITY".to_string(),
                "/run/user/1000/xpra/Xauthority".to_string(),
            ),
            ("NO_AT_BRIDGE".to_string(), "0".to_string()),
            ("ACCESSIBILITY_ENABLED".to_string(), "1".to_string()),
        ];

        let error = ensure_health_satisfies_desktop_env(&response, &desktop_vars, false)
            .expect_err("daemon missing spawn-only repairs should be rejected");

        let detail = error.to_string();
        assert!(detail.contains("XAUTHORITY"));
        assert!(detail.contains("NO_AT_BRIDGE"));
        assert!(detail.contains("ACCESSIBILITY_ENABLED"));
    }

    #[test]
    fn shared_browser_daemon_health_can_ignore_client_specific_desktop_identity() {
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::from([("DISPLAY".to_string(), ":0".to_string())]),
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };
        let launch = LaunchEnvironment::from_repaired_desktop_vars_for_tests(vec![(
            "DISPLAY".to_string(),
            ":99".to_string(),
        )]);

        assert!(launch.ensure_startup_health(&response, false).is_ok());
        assert!(launch.ensure_startup_health(&response, true).is_err());
    }

    #[test]
    fn startup_health_prefers_repaired_graphical_session_over_remote_display() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_values = GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        unsafe {
            for key in GRAPHICAL_SESSION_ENV_KEYS {
                std::env::remove_var(key);
            }
            std::env::set_var("DISPLAY", "localhost:10.0");
        }
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::from([("DISPLAY".to_string(), ":0".to_string())]),
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };
        let desktop_vars = vec![("DISPLAY".to_string(), ":0".to_string())];

        let result = ensure_health_satisfies_desktop_env(&response, &desktop_vars, false);

        for (key, value) in old_values {
            restore_env(key, value);
        }
        assert!(
            result.is_ok(),
            "selected graphical session should override the remote shell DISPLAY"
        );
    }

    #[test]
    fn detached_startup_health_does_not_require_unrepaired_inherited_display() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_values = GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        unsafe {
            for key in GRAPHICAL_SESSION_ENV_KEYS {
                std::env::remove_var(key);
            }
            std::env::set_var("DISPLAY", "localhost:10.0");
        }
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::from([(
                "XDG_RUNTIME_DIR".to_string(),
                "/run/user/1000".to_string(),
            )]),
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };
        let desktop_vars = vec![("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string())];

        let result = ensure_health_satisfies_desktop_env(&response, &desktop_vars, false);

        for (key, value) in old_values {
            restore_env(key, value);
        }
        assert!(
            result.is_ok(),
            "detached health should ignore stale inherited display when it was not repaired"
        );
    }

    #[test]
    fn empty_graphical_env_values_are_treated_as_detached() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_values = GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        unsafe {
            for key in GRAPHICAL_SESSION_ENV_KEYS {
                std::env::set_var(key, "");
            }
        }

        let detached = remote_or_detached_launch();

        for (key, value) in old_values {
            restore_env(key, value);
        }
        assert!(detached, "empty forwarded env vars should be repaired");
    }

    #[test]
    fn missing_session_bus_alone_does_not_make_graphical_identity_untrusted() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_values = GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        let old_tmux = std::env::var_os("TMUX");
        let old_ssh_connection = std::env::var_os("SSH_CONNECTION");
        let old_ssh_tty = std::env::var_os("SSH_TTY");
        unsafe {
            std::env::remove_var("TMUX");
            std::env::remove_var("SSH_CONNECTION");
            std::env::remove_var("SSH_TTY");
            std::env::remove_var("DBUS_SESSION_BUS_ADDRESS");
            std::env::set_var("DISPLAY", ":55");
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
            std::env::set_var("XDG_SESSION_TYPE", "x11");
            std::env::remove_var("WAYLAND_DISPLAY");
        }

        let detached = remote_or_detached_launch();

        for (key, value) in old_values {
            restore_env(key, value);
        }
        restore_env("TMUX", old_tmux);
        restore_env("SSH_CONNECTION", old_ssh_connection);
        restore_env("SSH_TTY", old_ssh_tty);
        assert!(
            !detached,
            "a missing session bus should repair only the bus, not override a valid display"
        );
    }

    #[test]
    fn forwarded_x11_display_is_treated_as_detached_without_ssh_hints() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_values = GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        let old_tmux = std::env::var_os("TMUX");
        let old_ssh_connection = std::env::var_os("SSH_CONNECTION");
        let old_ssh_tty = std::env::var_os("SSH_TTY");
        unsafe {
            std::env::remove_var("TMUX");
            std::env::remove_var("SSH_CONNECTION");
            std::env::remove_var("SSH_TTY");
            std::env::set_var("DISPLAY", "localhost:10.0");
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
            std::env::remove_var("XDG_SESSION_TYPE");
            std::env::remove_var("WAYLAND_DISPLAY");
        }

        let detached = remote_or_detached_launch();

        for (key, value) in old_values {
            restore_env(key, value);
        }
        restore_env("TMUX", old_tmux);
        restore_env("SSH_CONNECTION", old_ssh_connection);
        restore_env("SSH_TTY", old_ssh_tty);
        assert!(
            detached,
            "loopback X11 forwarding should be repaired even when SSH hints are stripped"
        );
    }

    #[test]
    fn tmux_launch_is_treated_as_detached_even_with_full_graphical_env() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_values = GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        let old_tmux = std::env::var_os("TMUX");
        let old_ssh_connection = std::env::var_os("SSH_CONNECTION");
        let old_ssh_tty = std::env::var_os("SSH_TTY");
        unsafe {
            std::env::remove_var("SSH_CONNECTION");
            std::env::remove_var("SSH_TTY");
            std::env::set_var("TMUX", "/tmp/tmux-1000/default,123,0");
            std::env::set_var("DISPLAY", ":55");
            std::env::set_var("WAYLAND_DISPLAY", "wayland-55");
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
            std::env::set_var("XDG_SESSION_TYPE", "wayland");
        }

        let detached = remote_or_detached_launch();

        for (key, value) in old_values {
            restore_env(key, value);
        }
        restore_env("TMUX", old_tmux);
        restore_env("SSH_CONNECTION", old_ssh_connection);
        restore_env("SSH_TTY", old_ssh_tty);
        assert!(
            detached,
            "tmux panes can retain stale graphical identity from a previous desktop session"
        );
    }

    #[test]
    fn forwarded_x11_display_detector_keeps_local_unix_displays_trusted() {
        assert!(forwarded_x11_display("localhost:10.0"));
        assert!(forwarded_x11_display("127.0.0.1:10.0"));
        assert!(forwarded_x11_display("[::1]:10.0"));
        assert!(!forwarded_x11_display(":0"));
        assert!(!forwarded_x11_display("unix/:0"));
        assert!(!forwarded_x11_display("unix:0"));
    }

    #[test]
    fn repaired_or_env_present_treats_empty_env_as_missing() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_display = std::env::var_os("DISPLAY");
        unsafe {
            std::env::set_var("DISPLAY", "");
        }

        let present_without_repair = repaired_or_env_present(&[], "DISPLAY");
        let present_with_repair =
            repaired_or_env_present(&[("DISPLAY".to_string(), ":0".to_string())], "DISPLAY");

        restore_env("DISPLAY", old_display);
        assert!(
            !present_without_repair,
            "empty display env should not infer a session type"
        );
        assert!(present_with_repair);
    }

    #[test]
    fn normalized_path_preserves_order_and_appends_defaults() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_path = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", "/tmp:/usr/bin:/tmp");
        }

        let path = normalized_path().expect("PATH should need normalization");

        restore_env("PATH", old_path);
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
    }

    #[test]
    fn startup_health_rejects_service_missing_client_desktop_env() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        }
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::new(),
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };

        let result = ensure_health_satisfies_desktop_env(&response, &[], true);

        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        let error = result.expect_err("stale service env should be rejected");
        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }

    #[test]
    fn startup_health_rejects_service_with_stale_desktop_env_value() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        }
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::from([(
                "XDG_RUNTIME_DIR".to_string(),
                "/run/user/9999".to_string(),
            )]),
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };

        let result = ensure_health_satisfies_desktop_env(&response, &[], true);

        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        let error = result.expect_err("stale service env value should be rejected");
        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }

    #[test]
    fn startup_health_rejects_conflicting_browser_selection() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_selection = std::env::var_os("SKY_CUA_BROWSER");
        unsafe {
            std::env::set_var("SKY_CUA_BROWSER", "brave");
        }
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::new(),
            browser_env: BTreeMap::from([("SKY_CUA_BROWSER".to_string(), "chrome".to_string())]),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };

        let result = ensure_health_satisfies_browser_env(&response);

        restore_env("SKY_CUA_BROWSER", old_selection);
        let error = result.expect_err("conflicting browser pins should be rejected");
        assert!(error.to_string().contains("SKY_CUA_BROWSER"));
    }

    #[test]
    fn startup_health_accepts_unpinned_service_for_pinned_client() {
        // A daemon without a browser selection probes every Chrome-family
        // browser (a superset), so a client pinning one browser must not
        // reject it: with the daemon singleton, exact-equality would let the
        // first spawning host permanently starve every other host.
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_selection = std::env::var_os("SKY_CUA_BROWSER");
        unsafe {
            std::env::set_var("SKY_CUA_BROWSER", "brave");
        }
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::new(),
            browser_env: BTreeMap::new(),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };

        let result = ensure_health_satisfies_browser_env(&response);

        restore_env("SKY_CUA_BROWSER", old_selection);
        assert!(result.is_ok(), "unpinned daemon must serve pinned clients");
    }

    #[test]
    fn startup_health_accepts_pinned_service_for_unpinned_client() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_selection = std::env::var_os("SKY_CUA_BROWSER");
        let old_socket_dir = std::env::var_os("SKY_CUA_BROWSER_USE_SOCKET_DIR");
        unsafe {
            std::env::remove_var("SKY_CUA_BROWSER");
            std::env::remove_var("SKY_CUA_BROWSER_USE_SOCKET_DIR");
        }
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::new(),
            browser_env: BTreeMap::from([
                ("SKY_CUA_BROWSER".to_string(), "brave".to_string()),
                (
                    "SKY_CUA_BROWSER_USE_SOCKET_DIR".to_string(),
                    "/tmp/old-browser-use".to_string(),
                ),
            ]),
            protocol_version: 1,
            service_version: "0.1.0".to_string(),
            capabilities: Vec::new(),
        };

        let result = ensure_health_satisfies_browser_env(&response);

        restore_env("SKY_CUA_BROWSER", old_selection);
        restore_env("SKY_CUA_BROWSER_USE_SOCKET_DIR", old_socket_dir);
        assert!(result.is_ok(), "pinned daemon must serve unpinned clients");
    }

    #[test]
    fn x11_display_probe_requires_an_actual_socket() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-client-x11-probe-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create test temp dir");

        assert_eq!(x11_display_from_socket_root(&temp_dir), None);

        fs::write(temp_dir.join("X1"), b"").expect("write x11 socket sentinel");

        assert_eq!(
            x11_display_from_socket_root(&temp_dir),
            Some(":1".to_string())
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn parses_loginctl_session_ids_from_table_output() {
        let ids = parse_loginctl_session_ids(
            "1 1000 bex - 1254 manager - no -\n5 1000 bex seat0 26330 user tty3 no -\n",
        );

        assert_eq!(ids, vec!["1".to_string(), "5".to_string()]);
    }

    #[test]
    fn graphical_logind_session_filter_rejects_remote_and_manager_sessions() {
        let manager = parse_logind_session_properties(
            "1",
            "Id=1\nUser=1000\nRemote=no\nClass=manager\nType=unspecified\nActive=yes\nState=active\n",
        );
        let ssh = parse_logind_session_properties(
            "7",
            "Id=7\nUser=1000\nRemote=yes\nClass=user\nType=tty\nActive=yes\nState=active\n",
        );
        let graphical = parse_logind_session_properties(
            "5",
            "Id=5\nUser=1000\nRemote=no\nClass=user\nType=wayland\nDesktop=KDE\nLeader=26330\nActive=yes\nState=active\n",
        );

        assert!(!is_graphical_logind_session(&manager));
        assert!(!is_graphical_logind_session(&ssh));
        assert!(is_graphical_logind_session(&graphical));
        assert_eq!(graphical.session_type.as_deref(), Some("wayland"));
        assert_eq!(graphical.desktop.as_deref(), Some("KDE"));
        assert_eq!(graphical.leader, Some(26330));
    }

    #[test]
    fn logind_desktop_metadata_does_not_invent_desktop_session() {
        let session = parse_logind_session_properties(
            "5",
            "Id=5\nUser=1000\nRemote=no\nClass=user\nType=wayland\nDesktop=KDE\nDisplay=:0\nActive=yes\nState=active\n",
        );
        let mut environment = HashMap::new();

        apply_logind_session_metadata(&mut environment, &session);

        assert_eq!(
            environment.get("XDG_CURRENT_DESKTOP").map(String::as_str),
            Some("KDE")
        );
        assert_eq!(
            environment.get("XDG_SESSION_TYPE").map(String::as_str),
            Some("wayland")
        );
        assert_eq!(environment.get("DISPLAY").map(String::as_str), Some(":0"));
        assert!(!environment.contains_key("DESKTOP_SESSION"));

        environment.insert("DESKTOP_SESSION".to_string(), "plasma".to_string());
        apply_logind_session_metadata(&mut environment, &session);
        assert_eq!(
            environment.get("DESKTOP_SESSION").map(String::as_str),
            Some("plasma")
        );
    }

    #[test]
    fn graphical_logind_session_rank_prefers_active_typed_sessions() {
        let stale = parse_logind_session_properties(
            "4",
            "Id=4\nUser=1000\nRemote=no\nClass=user\nType=x11\nDesktop=KDE\nActive=no\nState=closing\n",
        );
        let active = parse_logind_session_properties(
            "5",
            "Id=5\nUser=1000\nRemote=no\nClass=user\nType=wayland\nDesktop=KDE\nActive=yes\nState=active\n",
        );

        assert!(logind_session_rank(&active) > logind_session_rank(&stale));
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
    fn selected_session_systemd_environment_fills_only_support_keys() {
        let mut environment = HashMap::from([
            ("DISPLAY".to_string(), ":selected".to_string()),
            ("XDG_SESSION_TYPE".to_string(), "wayland".to_string()),
        ]);
        let systemd_env = HashMap::from([
            ("DISPLAY".to_string(), "localhost:10.0".to_string()),
            (
                "DBUS_SESSION_BUS_ADDRESS".to_string(),
                "unix:path=/run/user/1000/bus".to_string(),
            ),
            ("WAYLAND_DISPLAY".to_string(), "wayland-stale".to_string()),
        ]);

        fill_missing_selected_session_systemd_environment(&mut environment, systemd_env);

        assert_eq!(
            environment.get("DISPLAY").map(String::as_str),
            Some(":selected")
        );
        assert_eq!(environment.get("WAYLAND_DISPLAY"), None);
        assert_eq!(
            environment
                .get("DBUS_SESSION_BUS_ADDRESS")
                .map(String::as_str),
            Some("unix:path=/run/user/1000/bus")
        );
    }

    #[test]
    fn selected_session_systemd_environment_does_not_supply_stale_display_identity() {
        let mut environment = HashMap::new();
        let systemd_env = HashMap::from([
            ("DISPLAY".to_string(), "localhost:10.0".to_string()),
            ("WAYLAND_DISPLAY".to_string(), "wayland-stale".to_string()),
            ("XDG_CURRENT_DESKTOP".to_string(), "SSH".to_string()),
            (
                "DBUS_SESSION_BUS_ADDRESS".to_string(),
                "unix:path=/run/user/1000/bus".to_string(),
            ),
        ]);

        fill_missing_selected_session_systemd_environment(&mut environment, systemd_env);

        assert!(!environment.contains_key("DISPLAY"));
        assert!(!environment.contains_key("WAYLAND_DISPLAY"));
        assert!(!environment.contains_key("XDG_CURRENT_DESKTOP"));
        assert_eq!(
            environment
                .get("DBUS_SESSION_BUS_ADDRESS")
                .map(String::as_str),
            Some("unix:path=/run/user/1000/bus")
        );
    }

    #[test]
    fn selected_logind_metadata_replaces_empty_leader_values() {
        let mut environment = HashMap::from([
            ("XDG_CURRENT_DESKTOP".to_string(), String::new()),
            ("DESKTOP_SESSION".to_string(), String::new()),
            ("DISPLAY".to_string(), String::new()),
            ("WAYLAND_DISPLAY".to_string(), "wayland-live".to_string()),
        ]);

        insert_if_missing_or_empty(&mut environment, "XDG_CURRENT_DESKTOP", "KDE".to_string());
        insert_if_missing_or_empty(&mut environment, "DESKTOP_SESSION", "KDE".to_string());
        insert_if_missing_or_empty(&mut environment, "DISPLAY", ":0".to_string());
        insert_if_missing_or_empty(
            &mut environment,
            "WAYLAND_DISPLAY",
            "wayland-logind".to_string(),
        );

        assert_eq!(
            environment.get("XDG_CURRENT_DESKTOP").map(String::as_str),
            Some("KDE")
        );
        assert_eq!(
            environment.get("DESKTOP_SESSION").map(String::as_str),
            Some("KDE")
        );
        assert_eq!(environment.get("DISPLAY").map(String::as_str), Some(":0"));
        assert_eq!(
            environment.get("WAYLAND_DISPLAY").map(String::as_str),
            Some("wayland-live")
        );
    }

    #[test]
    fn logind_display_metadata_never_becomes_wayland_display() {
        let mut environment =
            HashMap::from([("XDG_SESSION_TYPE".to_string(), "wayland".to_string())]);

        insert_if_missing_or_empty(&mut environment, "DISPLAY", ":0".to_string());

        assert_eq!(environment.get("DISPLAY").map(String::as_str), Some(":0"));
        assert_eq!(environment.get("WAYLAND_DISPLAY"), None);
    }

    #[test]
    fn detached_systemd_fallback_rejects_display_identity_keys() {
        assert!(!systemd_fallback_key_allowed("DISPLAY", true));
        assert!(!systemd_fallback_key_allowed("WAYLAND_DISPLAY", true));
        assert!(!systemd_fallback_key_allowed("XDG_SESSION_TYPE", true));
        assert!(!systemd_fallback_key_allowed("XDG_CURRENT_DESKTOP", true));
        assert!(!systemd_fallback_key_allowed("DESKTOP_SESSION", true));
        assert!(systemd_fallback_key_allowed(
            "DBUS_SESSION_BUS_ADDRESS",
            true
        ));
        assert!(systemd_fallback_key_allowed("XDG_RUNTIME_DIR", true));
        assert!(systemd_fallback_key_allowed("DISPLAY", false));
    }

    fn restore_env(key: &str, old_value: Option<std::ffi::OsString>) {
        unsafe {
            if let Some(value) = old_value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}
