use std::io;
use std::path::PathBuf;

pub const SERVICE_SOCKET_PATH_ENV: &str = "SKY_CUA_SERVICE_SOCKET_PATH";
pub const SERVICE_TCP_ADDR_ENV: &str = "SKY_CUA_SERVICE_TCP_ADDR";
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

pub fn portal_tokens_path() -> io::Result<PathBuf> {
    Ok(sky_cua_state_dir()?.join("portal-tokens.json"))
}

pub fn approvals_path() -> io::Result<PathBuf> {
    Ok(sky_cua_state_dir()?.join("approvals.json"))
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
