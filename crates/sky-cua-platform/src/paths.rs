use std::io;
use std::path::PathBuf;

pub const SERVICE_SOCKET_PATH_ENV: &str = "SKY_CUA_SERVICE_SOCKET_PATH";
pub const SERVICE_TCP_ADDR_ENV: &str = "SKY_CUA_SERVICE_TCP_ADDR";
pub const OVERLAY_HOST_TCP_ADDR_ENV: &str = "SKY_CUA_OVERLAY_HOST_TCP_ADDR";
const APP_STATE_DIR_NAME: &str = "sky-cua";
const DEFAULT_WINDOWS_SERVICE_ADDR: &str = "127.0.0.1:48931";

#[must_use]
pub fn service_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os(SERVICE_SOCKET_PATH_ENV) {
        return PathBuf::from(path);
    }

    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        return runtime_dir.join("sky-cua").join("service.sock");
    }

    if let Some(cache_dir) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from) {
        return cache_dir.join("sky-cua").join("service.sock");
    }

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        return home.join(".cache").join("sky-cua").join("service.sock");
    }

    std::env::temp_dir()
        .join(format!("sky-cua-uid-{}", current_uid()))
        .join("service.sock")
}

#[must_use]
pub fn service_tcp_addr() -> String {
    std::env::var(SERVICE_TCP_ADDR_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_WINDOWS_SERVICE_ADDR.to_string())
}

#[must_use]
pub fn overlay_host_tcp_addr() -> String {
    std::env::var(OVERLAY_HOST_TCP_ADDR_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(default_overlay_host_tcp_addr)
}

fn default_overlay_host_tcp_addr() -> String {
    let service_addr = service_tcp_addr();
    if let Some((host, port)) = service_addr.rsplit_once(':')
        && let Ok(port) = port.parse::<u16>()
        && let Some(overlay_port) = port.checked_add(1)
    {
        return format!("{host}:{overlay_port}");
    }
    "127.0.0.1:48932".to_string()
}

pub fn sky_cua_state_dir() -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        let state_root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("AppData").join("Local")))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "cannot determine a per-user state directory because LOCALAPPDATA, APPDATA, and USERPROFILE are unset",
                )
            })?;
        Ok(state_root.join(APP_STATE_DIR_NAME))
    }

    #[cfg(not(windows))]
    {
        let state_root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "cannot determine a per-user state directory because neither XDG_STATE_HOME nor HOME is set",
            )
        })?;
        Ok(state_root.join(APP_STATE_DIR_NAME))
    }
}

/// Resolve the durable Companion Direct state file without depending on the
/// daemon's current working directory. Absolute overrides remain explicit;
/// relative overrides are names within the per-user state directory.
pub fn phone_direct_state_path(configured: Option<&str>) -> io::Result<PathBuf> {
    let configured = configured.map(str::trim).filter(|value| !value.is_empty());
    Ok(match configured {
        Some(path) => {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                sky_cua_state_dir()?.join(path)
            }
        }
        None => sky_cua_state_dir()?.join("phone-direct-state.json"),
    })
}

pub fn portal_tokens_path() -> io::Result<PathBuf> {
    Ok(sky_cua_state_dir()?.join("portal-tokens.json"))
}

pub fn approvals_path() -> io::Result<PathBuf> {
    Ok(sky_cua_state_dir()?.join("approvals.json"))
}

#[must_use]
pub fn appshot_artifacts_dir() -> PathBuf {
    runtime_artifacts_root().join("appshots")
}

#[must_use]
pub fn capture_artifacts_dir() -> PathBuf {
    runtime_artifacts_root().join("captures")
}

fn runtime_artifacts_root() -> PathBuf {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        return runtime_dir.join("sky-cua");
    }
    std::env::temp_dir().join(format!("sky-cua-uid-{}", current_uid()))
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::{
        OVERLAY_HOST_TCP_ADDR_ENV, SERVICE_SOCKET_PATH_ENV, SERVICE_TCP_ADDR_ENV,
        overlay_host_tcp_addr, phone_direct_state_path, service_socket_path,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[test]
    fn service_socket_path_uses_stable_default_name() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_override = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        let old_runtime = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::remove_var(SERVICE_SOCKET_PATH_ENV);
            std::env::set_var("XDG_RUNTIME_DIR", "/tmp/sky-cua-runtime-test");
        }

        let first = service_socket_path();
        let second = service_socket_path();

        restore_env(SERVICE_SOCKET_PATH_ENV, old_override);
        restore_env("XDG_RUNTIME_DIR", old_runtime);

        assert_eq!(
            first,
            PathBuf::from("/tmp/sky-cua-runtime-test/sky-cua/service.sock")
        );
        assert_eq!(second, first);
    }

    #[test]
    fn service_socket_path_env_override_wins() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_override = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        unsafe {
            std::env::set_var(SERVICE_SOCKET_PATH_ENV, "/tmp/sky-cua-custom.sock");
        }

        let path = service_socket_path();

        restore_env(SERVICE_SOCKET_PATH_ENV, old_override);

        assert_eq!(path, PathBuf::from("/tmp/sky-cua-custom.sock"));
    }

    #[test]
    fn overlay_host_tcp_addr_defaults_to_service_port_plus_one() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_service_addr = std::env::var_os(SERVICE_TCP_ADDR_ENV);
        let old_overlay_addr = std::env::var_os(OVERLAY_HOST_TCP_ADDR_ENV);
        unsafe {
            std::env::set_var(SERVICE_TCP_ADDR_ENV, "127.0.0.1:50000");
            std::env::remove_var(OVERLAY_HOST_TCP_ADDR_ENV);
        }

        let addr = overlay_host_tcp_addr();

        restore_env(SERVICE_TCP_ADDR_ENV, old_service_addr);
        restore_env(OVERLAY_HOST_TCP_ADDR_ENV, old_overlay_addr);

        assert_eq!(addr, "127.0.0.1:50001");
    }

    #[test]
    fn overlay_host_tcp_addr_env_override_wins() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_overlay_addr = std::env::var_os(OVERLAY_HOST_TCP_ADDR_ENV);
        unsafe {
            std::env::set_var(OVERLAY_HOST_TCP_ADDR_ENV, "127.0.0.1:50123");
        }

        let addr = overlay_host_tcp_addr();

        restore_env(OVERLAY_HOST_TCP_ADDR_ENV, old_overlay_addr);

        assert_eq!(addr, "127.0.0.1:50123");
    }

    #[test]
    fn phone_direct_state_path_is_stable_and_cwd_independent() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_state = std::env::var_os("XDG_STATE_HOME");
        let old_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("XDG_STATE_HOME", "/tmp/sky-cua-state-test");
            std::env::remove_var("HOME");
        }

        let default_path = phone_direct_state_path(None).expect("state path");
        let relative_path =
            phone_direct_state_path(Some("nested/direct.json")).expect("state path");
        let absolute_path =
            phone_direct_state_path(Some("/var/lib/sky-cua/direct.json")).expect("state path");

        restore_env("XDG_STATE_HOME", old_state);
        restore_env("HOME", old_home);

        assert_eq!(
            default_path,
            PathBuf::from("/tmp/sky-cua-state-test/sky-cua/phone-direct-state.json")
        );
        assert_eq!(
            relative_path,
            PathBuf::from("/tmp/sky-cua-state-test/sky-cua/nested/direct.json")
        );
        assert_eq!(absolute_path, PathBuf::from("/var/lib/sky-cua/direct.json"));
    }
}
