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
}

impl LaunchEnvironment {
    pub(crate) fn probe() -> Self {
        Self {
            repaired_desktop_vars: probe_desktop_env_vars(),
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

    pub(crate) fn ensure_startup_health(&self, response: &ServiceResponse) -> Result<()> {
        ensure_health_satisfies_desktop_env(response, &self.repaired_desktop_vars)?;
        ensure_health_satisfies_browser_env(response)
    }

    #[cfg(test)]
    pub(crate) fn from_repaired_desktop_vars_for_tests(
        repaired_desktop_vars: Vec<(String, String)>,
    ) -> Self {
        Self {
            repaired_desktop_vars,
        }
    }
}

/// Probe the active user session for missing desktop environment variables.
/// When a host spawns the MCP server without forwarding the full desktop
/// session (e.g., via systemd unit, remote SSH, or container entrypoint),
/// the service backends cannot initialize. This function attempts to
/// reconstruct the missing values from common well-known sources.
#[must_use]
fn probe_desktop_env_vars() -> Vec<(String, String)> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }

    let mut found = Vec::new();
    let needs = |key: &str| std::env::var_os(key).is_none();

    if needs("XDG_RUNTIME_DIR") {
        let uid = current_uid();
        let candidate = PathBuf::from(format!("/run/user/{uid}"));
        if candidate.is_dir() {
            found.push((
                "XDG_RUNTIME_DIR".to_string(),
                candidate.to_string_lossy().to_string(),
            ));
        }
    }

    if needs("DBUS_SESSION_BUS_ADDRESS")
        && let Some(runtime_dir) = found
            .iter()
            .find(|(k, _)| k == "XDG_RUNTIME_DIR")
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
    {
        let socket = runtime_dir.join("bus");
        if socket.exists() {
            found.push((
                "DBUS_SESSION_BUS_ADDRESS".to_string(),
                format!("unix:path={}", socket.display()),
            ));
        }
    }

    if needs("WAYLAND_DISPLAY")
        && let Some(runtime_dir) = found
            .iter()
            .find(|(k, _)| k == "XDG_RUNTIME_DIR")
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
    {
        for name in &["wayland-0", "wayland-1", "wayland-2"] {
            if runtime_dir.join(name).exists() {
                found.push(("WAYLAND_DISPLAY".to_string(), (*name).to_string()));
                break;
            }
        }
    }

    if needs("DISPLAY")
        && let Some(display) = probe_x11_display()
    {
        found.push(("DISPLAY".to_string(), display));
    }

    if needs("XDG_CURRENT_DESKTOP")
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
            found.push(("XDG_CURRENT_DESKTOP".to_string(), stdout));
        }
    }

    if needs("XDG_SESSION_TYPE") {
        let wayland_present = found.iter().any(|(k, _)| k == "WAYLAND_DISPLAY")
            || std::env::var_os("WAYLAND_DISPLAY").is_some();
        let display_present =
            found.iter().any(|(k, _)| k == "DISPLAY") || std::env::var_os("DISPLAY").is_some();

        if wayland_present {
            found.push(("XDG_SESSION_TYPE".to_string(), "wayland".to_string()));
        } else if display_present {
            found.push(("XDG_SESSION_TYPE".to_string(), "x11".to_string()));
        }
    }

    if let Some(systemd_env) = systemd_user_environment() {
        for key in DESKTOP_LAUNCH_ENV_KEYS {
            if *key == "PATH" {
                continue;
            }
            if needs(key)
                && let Some(value) = systemd_env
                    .get(*key)
                    .filter(|value| !value.trim().is_empty())
                && !found.iter().any(|(found_key, _)| found_key == key)
            {
                found.push(((*key).to_string(), value.clone()));
            }
        }
    }

    if let Some(path) = normalized_path() {
        found.push(("PATH".to_string(), path));
    }

    found
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
) -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }

    let mut required = GRAPHICAL_SESSION_ENV_KEYS
        .iter()
        .copied()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (key, value))
        })
        .collect::<Vec<_>>();
    for (key, value) in desktop_vars {
        let key = key.as_str();
        if DESKTOP_LAUNCH_ENV_KEYS.contains(&key)
            && !value.is_empty()
            && !required
                .iter()
                .any(|(required_key, _)| *required_key == key)
        {
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

    let stale = BROWSER_ENV_HEALTH_KEYS
        .iter()
        .copied()
        .filter(|key| browser_env.get(*key) != desired.get(*key))
        .collect::<Vec<_>>();
    if stale.is_empty() {
        return Ok(());
    }

    Err(anyhow!(
        "existing sky-cua-service has stale browser environment keys: {}",
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
    fn startup_health_rejects_service_missing_repaired_desktop_env() {
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::from([("DISPLAY".to_string(), ":0".to_string())]),
            browser_env: BTreeMap::new(),
        };
        let desktop_vars = vec![
            ("DISPLAY".to_string(), ":0".to_string()),
            ("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string()),
        ];

        let error = ensure_health_satisfies_desktop_env(&response, &desktop_vars)
            .expect_err("stale service env should be rejected");

        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }

    #[test]
    fn startup_health_rejects_service_missing_repaired_path() {
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::new(),
            browser_env: BTreeMap::new(),
        };
        let desktop_vars = vec![("PATH".to_string(), "/tmp:/usr/bin".to_string())];

        let error = ensure_health_satisfies_desktop_env(&response, &desktop_vars)
            .expect_err("stale service PATH should be rejected");

        assert!(error.to_string().contains("PATH"));
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
        };

        let result = ensure_health_satisfies_desktop_env(&response, &[]);

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
        };

        let result = ensure_health_satisfies_desktop_env(&response, &[]);

        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        let error = result.expect_err("stale service env value should be rejected");
        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }

    #[test]
    fn startup_health_rejects_service_with_stale_browser_selection() {
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
        };

        let result = ensure_health_satisfies_browser_env(&response);

        restore_env("SKY_CUA_BROWSER", old_selection);
        let error = result.expect_err("stale service browser env should be rejected");
        assert!(error.to_string().contains("SKY_CUA_BROWSER"));
    }

    #[test]
    fn startup_health_rejects_browser_env_mismatch_without_enable_flag() {
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
        };

        let result = ensure_health_satisfies_browser_env(&response);

        restore_env("SKY_CUA_BROWSER", old_selection);
        let error = result.expect_err("browser env mismatch should be rejected");
        assert!(error.to_string().contains("SKY_CUA_BROWSER"));
    }

    #[test]
    fn startup_health_rejects_service_with_extra_browser_socket_dir() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_socket_dir = std::env::var_os("SKY_CUA_BROWSER_USE_SOCKET_DIR");
        unsafe {
            std::env::remove_var("SKY_CUA_BROWSER_USE_SOCKET_DIR");
        }
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::new(),
            browser_env: BTreeMap::from([(
                "SKY_CUA_BROWSER_USE_SOCKET_DIR".to_string(),
                "/tmp/old-browser-use".to_string(),
            )]),
        };

        let result = ensure_health_satisfies_browser_env(&response);

        restore_env("SKY_CUA_BROWSER_USE_SOCKET_DIR", old_socket_dir);
        let error = result.expect_err("stale service browser socket dir should be rejected");
        assert!(error.to_string().contains("SKY_CUA_BROWSER_USE_SOCKET_DIR"));
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
