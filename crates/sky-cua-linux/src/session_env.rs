use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    process::Command,
};

const DESKTOP_ENV_KEYS: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
];

/// Hydrate session bus and desktop environment variables by walking the
/// process tree and reading parent process environ files. This is critical
/// when sky-cua-service is launched by Codex Desktop without inheriting the
/// user's graphical session environment.
pub fn hydrate_session_bus_env() {
    hydrate_desktop_env_from_process_tree();

    if env_var("XDG_RUNTIME_DIR").is_none()
        && let Some(runtime) = xdg_runtime_dir()
        && runtime.exists()
    {
        unsafe { env::set_var("XDG_RUNTIME_DIR", runtime) };
    }

    if env_var("DBUS_SESSION_BUS_ADDRESS").is_none()
        && let Some(runtime) = xdg_runtime_dir()
    {
        let bus = runtime.join("bus");
        if bus.exists() {
            unsafe {
                env::set_var(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}", bus.display()),
                );
            }
        }
    }
}

fn hydrate_desktop_env_from_process_tree() {
    for process_env in desktop_process_environments() {
        for key in DESKTOP_ENV_KEYS {
            if env_var(key).is_some() {
                continue;
            }
            if let Some(value) = process_env
                .get(*key)
                .filter(|value| !value.trim().is_empty())
            {
                unsafe { env::set_var(key, value) };
            }
        }

        if DESKTOP_ENV_KEYS.iter().all(|key| env_var(key).is_some()) {
            break;
        }
    }
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
    use super::*;

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
}
