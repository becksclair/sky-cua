use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    process::Command,
};

use sky_cua_platform::{
    CURRENT_ENV_HEALTH_KEYS,
    model::{DoctorSessionEnvRepair, DoctorSessionEnvReport},
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
    hydrate_desktop_env_from_process_tree(&mut report);
    hydrate_desktop_env_from_systemd(&mut report);

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

/// Backward-compatible wrapper for older callers that only need mutation.
pub fn hydrate_session_bus_env() {
    let _ = hydrate_session_env();
}

fn hydrate_desktop_env_from_process_tree(report: &mut DoctorSessionEnvReport) {
    for process_env in desktop_process_environments() {
        for key in CURRENT_ENV_HEALTH_KEYS {
            if env_var(key).is_some() {
                continue;
            }
            if let Some(value) = process_env
                .get(*key)
                .filter(|value| !value.trim().is_empty())
            {
                unsafe { env::set_var(key, value) };
                push_repair(report, *key, "process-tree", Some(value.clone()));
            }
        }

        if CURRENT_ENV_HEALTH_KEYS
            .iter()
            .all(|key| env_var(key).is_some())
        {
            break;
        }
    }
}

fn hydrate_desktop_env_from_systemd(report: &mut DoctorSessionEnvReport) {
    let Some(systemd_env) = systemd_user_environment(report) else {
        return;
    };
    hydrate_desktop_env_from_map(report, "systemd-user", &systemd_env);
}

fn hydrate_desktop_env_from_map(
    report: &mut DoctorSessionEnvReport,
    source: &str,
    values: &HashMap<String, String>,
) {
    for key in CURRENT_ENV_HEALTH_KEYS {
        if env_var(key).is_some() {
            continue;
        }
        if let Some(value) = values.get(*key).filter(|value| !value.trim().is_empty()) {
            unsafe { env::set_var(key, value) };
            push_repair(report, *key, source, Some(value.clone()));
        }
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
    use super::*;
    use serial_test::serial;

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

        hydrate_desktop_env_from_map(&mut report, "systemd-user", &values);

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
}
