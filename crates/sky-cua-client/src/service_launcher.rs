use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use sky_cua_platform::model::{ServiceRequest, ServiceResponse};
#[cfg(unix)]
use sky_cua_platform::{SERVICE_SOCKET_PATH_ENV, service_socket_path};
#[cfg(windows)]
use sky_cua_platform::{SERVICE_TCP_ADDR_ENV, service_tcp_addr};

const SERVICE_READ_TIMEOUT: Duration = Duration::from_secs(60);
const SERVICE_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const STARTUP_HEALTH_READ_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_HEALTH_WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(150);
const STARTUP_HEALTH_ATTEMPTS: usize = 40;
const DESKTOP_ENV_KEYS: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
];
#[derive(Debug, Clone)]
pub struct ServiceClient {
    endpoint: ServiceEndpoint,
    child: Arc<Mutex<Option<Child>>>,
}

/// Probe the active user session for missing desktop environment variables.
/// When a host spawns the MCP server without forwarding the full desktop
/// session (e.g., via systemd unit, remote SSH, or container entrypoint),
/// the service backends cannot initialize. This function attempts to
/// reconstruct the missing values from common well-known sources:
///
/// - `XDG_RUNTIME_DIR` from the running user's UID and /run/user
/// - `DBUS_SESSION_BUS_ADDRESS` from the active session bus socket
/// - `WAYLAND_DISPLAY` and `DISPLAY` from the active graphical session
/// - `XDG_CURRENT_DESKTOP` and `XDG_SESSION_TYPE` from systemd-logind
#[must_use]
fn probe_desktop_env_vars() -> Vec<(String, String)> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }

    let mut found = Vec::new();

    // Helper: only fill if the key is currently unset or empty.
    let needs = |key: &str| std::env::var_os(key).map(|v| v.is_empty()).unwrap_or(true);

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
        // Check for the most common Wayland socket names in order.
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

    if needs("XDG_CURRENT_DESKTOP") {
        // Try to read from systemd-logind session or fall back to a sensible default.
        if let Ok(output) = std::process::Command::new("loginctl")
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
    }

    if needs("XDG_SESSION_TYPE") {
        // Prefer wayland if WAYLAND_DISPLAY is present, otherwise x11 if DISPLAY is present.
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

    found
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

impl ServiceClient {
    pub fn connect_or_spawn() -> Result<Self> {
        let client = Self::new()?;
        // Startup probes must fail fast; a stale or half-ready daemon should not
        // consume the entire MCP server startup window before we even spawn a
        // fresh service instance.
        if client.startup_health().is_ok() {
            return Ok(client);
        }

        client.spawn_service()?;
        client.wait_for_startup_health()?;
        Ok(client)
    }

    fn startup_health(&self) -> Result<ServiceResponse> {
        let desktop_vars = probe_desktop_env_vars();
        let response = self.call_with_timeouts(
            &ServiceRequest::Health,
            STARTUP_HEALTH_READ_TIMEOUT,
            STARTUP_HEALTH_WRITE_TIMEOUT,
        )?;
        ensure_health_satisfies_desktop_env(&response, &desktop_vars)?;
        Ok(response)
    }

    pub fn clear_portal_tokens(&self) -> Result<ServiceResponse> {
        self.call(&ServiceRequest::ResetPortalTokens)
    }

    pub fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse> {
        match self.call_with_timeouts(request, SERVICE_READ_TIMEOUT, SERVICE_WRITE_TIMEOUT) {
            Ok(response) => Ok(response),
            Err(first_error) => {
                self.reap_exited_child()?;
                self.spawn_service()?;
                self.wait_for_startup_health()
                    .with_context(|| format!("after service call failed: {first_error}"))?;
                self.call_with_timeouts(request, SERVICE_READ_TIMEOUT, SERVICE_WRITE_TIMEOUT)
                    .with_context(|| format!("after service call failed: {first_error}"))
            }
        }
    }

    fn call_with_timeouts(
        &self,
        request: &ServiceRequest,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<ServiceResponse> {
        let mut stream = self.endpoint.connect()?;
        stream
            .set_read_timeout(Some(read_timeout))
            .context("failed to set a read timeout on the sky-cua-service socket")?;
        stream
            .set_write_timeout(Some(write_timeout))
            .context("failed to set a write timeout on the sky-cua-service socket")?;
        let payload = serde_json::to_vec(request)?;
        stream.write_all(&payload)?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            return Err(anyhow!("sky-cua-service returned an empty response"));
        }
        Ok(serde_json::from_str(line.trim_end())?)
    }

    fn new() -> Result<Self> {
        Ok(Self {
            endpoint: ServiceEndpoint::new()?,
            child: Arc::new(Mutex::new(None)),
        })
    }

    fn spawn_service(&self) -> Result<()> {
        let mut child_guard = self
            .child
            .lock()
            .map_err(|_| anyhow!("sky-cua-service child state mutex was poisoned"))?;
        if let Some(child) = child_guard.as_mut()
            && child.try_wait()?.is_none()
        {
            return Ok(());
        }

        let service_path = service_path();
        let mut command = Command::new(&service_path);
        command
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Forward reconstructed desktop session env vars so the service can
        // initialize its platform backends even when the MCP host did not pass
        // them through.
        let desktop_vars = probe_desktop_env_vars();
        for (key, value) in &desktop_vars {
            command.env(key, value);
        }

        self.endpoint.configure_service_command(&mut command);
        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn service at {}", service_path.display()))?;
        *child_guard = Some(child);
        Ok(())
    }

    fn wait_for_startup_health(&self) -> Result<()> {
        for _ in 0..STARTUP_HEALTH_ATTEMPTS {
            if self.startup_health().is_ok() {
                return Ok(());
            }
            self.reap_exited_child()?;
            thread::sleep(STARTUP_POLL_INTERVAL);
        }

        Err(anyhow!(
            "sky-cua-service did not become healthy on {}",
            self.endpoint
        ))
    }

    fn reap_exited_child(&self) -> Result<()> {
        let mut child_guard = self
            .child
            .lock()
            .map_err(|_| anyhow!("sky-cua-service child state mutex was poisoned"))?;
        if let Some(child) = child_guard.as_mut()
            && child.try_wait()?.is_some()
        {
            *child_guard = None;
        }
        Ok(())
    }
}

fn ensure_health_satisfies_desktop_env(
    response: &ServiceResponse,
    desktop_vars: &[(String, String)],
) -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }

    let mut required = DESKTOP_ENV_KEYS
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
        if DESKTOP_ENV_KEYS.contains(&key)
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

#[derive(Debug, Clone)]
enum ServiceEndpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    Tcp(String),
}

impl ServiceEndpoint {
    fn new() -> Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self::Unix(service_socket_path()))
        }
        #[cfg(windows)]
        {
            Ok(Self::Tcp(resolve_service_tcp_addr()?))
        }
    }

    fn connect(&self) -> Result<EitherStream> {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => UnixStream::connect(path)
                .with_context(|| {
                    format!(
                        "failed to connect to sky-cua-service socket {}",
                        path.display()
                    )
                })
                .map(EitherStream::Unix),
            #[cfg(windows)]
            Self::Tcp(addr) => TcpStream::connect(addr)
                .with_context(|| {
                    format!("failed to connect to sky-cua-service TCP endpoint {addr}")
                })
                .map(EitherStream::Tcp),
        }
    }

    fn configure_service_command(&self, command: &mut Command) {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => {
                command.env(SERVICE_SOCKET_PATH_ENV, path);
            }
            #[cfg(windows)]
            Self::Tcp(addr) => {
                command.env(SERVICE_TCP_ADDR_ENV, addr);
            }
        }
    }
}

impl std::fmt::Display for ServiceEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => write!(formatter, "{}", path.display()),
            #[cfg(windows)]
            Self::Tcp(addr) => write!(formatter, "{addr}"),
        }
    }
}

enum EitherStream {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Tcp(TcpStream),
}

impl std::io::Read for EitherStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buf),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.read(buf),
        }
    }
}

impl std::io::Write for EitherStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buf),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

impl EitherStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_read_timeout(timeout),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_write_timeout(timeout),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.set_write_timeout(timeout),
        }
    }
}

fn service_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SKY_CUA_SERVICE_PATH") {
        return PathBuf::from(path);
    }
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(sibling) = exe_path
            .parent()
            .map(|parent| parent.join(service_binary_name()))
        && sibling.is_file()
    {
        return sibling;
    }
    let repo_root = std::env::var_os("SKY_CUA_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    repo_root.join("bin").join(service_binary_name())
}

fn service_binary_name() -> &'static str {
    if cfg!(windows) {
        "sky-cua-service.exe"
    } else {
        "sky-cua-service"
    }
}

#[cfg(windows)]
fn resolve_service_tcp_addr() -> Result<String> {
    use std::net::TcpListener;

    if std::env::var_os(SERVICE_TCP_ADDR_ENV).is_some_and(|value| !value.is_empty()) {
        return Ok(service_tcp_addr());
    }

    let configured = service_tcp_addr();
    let bind_addr = configured
        .rsplit_once(':')
        .map(|(host, _)| format!("{host}:0"))
        .unwrap_or_else(|| "127.0.0.1:0".to_string());
    let listener = TcpListener::bind(&bind_addr)
        .with_context(|| format!("failed to reserve sky-cua-service TCP endpoint {bind_addr}"))?;
    let addr = listener
        .local_addr()
        .context("failed to read reserved sky-cua-service TCP endpoint")?
        .to_string();
    drop(listener);
    Ok(addr)
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use sky_cua_platform::SERVICE_SOCKET_PATH_ENV;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn unix_service_command_uses_client_socket_endpoint() {
        let socket_path = PathBuf::from("/tmp/sky-cua-test/service.sock");
        let endpoint = ServiceEndpoint::Unix(socket_path.clone());
        let mut command = Command::new("sky-cua-service");

        endpoint.configure_service_command(&mut command);

        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == SERVICE_SOCKET_PATH_ENV)
                .and_then(|(_, value)| value),
            Some(socket_path.as_os_str())
        );
    }

    #[test]
    fn startup_health_rejects_service_missing_repaired_desktop_env() {
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_string(),
            desktop_env: BTreeMap::from([("DISPLAY".to_string(), ":0".to_string())]),
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
        };

        let result = ensure_health_satisfies_desktop_env(&response, &[]);

        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        let error = result.expect_err("stale service env value should be rejected");
        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
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
    fn respawns_service_after_child_exits() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-client-respawn-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let service_script = temp_dir.join("fake-sky-cua-service");
        let socket_path = temp_dir.join("service.sock");
        fs::write(&service_script, FAKE_SERVICE).expect("write fake service script");
        fs::set_permissions(&service_script, fs::Permissions::from_mode(0o755))
            .expect("make fake service executable");

        let old_service_path = std::env::var_os("SKY_CUA_SERVICE_PATH");
        let old_socket_path = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        unsafe {
            std::env::set_var("SKY_CUA_SERVICE_PATH", &service_script);
            std::env::set_var(SERVICE_SOCKET_PATH_ENV, &socket_path);
        }

        let result = run_respawn_test();

        restore_env("SKY_CUA_SERVICE_PATH", old_service_path);
        restore_env(SERVICE_SOCKET_PATH_ENV, old_socket_path);
        let _ = fs::remove_dir_all(&temp_dir);

        result.expect("service client should respawn exited child");
    }

    fn run_respawn_test() -> Result<()> {
        let client = ServiceClient::connect_or_spawn()?;
        let first_child_id = child_id(&client)?;
        anyhow::ensure!(
            matches!(
                client.call(&ServiceRequest::Health)?,
                ServiceResponse::Health { ok: true, .. }
            ),
            "initial health call did not return ok"
        );

        terminate_child(&client)?;
        anyhow::ensure!(
            matches!(
                client.call(&ServiceRequest::Health)?,
                ServiceResponse::Health { ok: true, .. }
            ),
            "respawned health call did not return ok"
        );
        let second_child_id = child_id(&client)?;
        anyhow::ensure!(
            first_child_id != second_child_id,
            "service child id did not change after respawn"
        );
        terminate_child(&client)?;
        Ok(())
    }

    fn child_id(client: &ServiceClient) -> Result<u32> {
        let child_guard = client
            .child
            .lock()
            .map_err(|_| anyhow!("child state mutex was poisoned"))?;
        child_guard
            .as_ref()
            .map(Child::id)
            .ok_or_else(|| anyhow!("expected spawned service child"))
    }

    fn terminate_child(client: &ServiceClient) -> Result<()> {
        let mut child_guard = client
            .child
            .lock()
            .map_err(|_| anyhow!("child state mutex was poisoned"))?;
        if let Some(child) = child_guard.as_mut() {
            child.kill()?;
            let _ = child.wait()?;
        }
        Ok(())
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

    const FAKE_SERVICE: &str = r#"#!/usr/bin/env python3
import json
import os
import socket
import sys

if len(sys.argv) < 2 or sys.argv[1] != "daemon":
    raise SystemExit("expected daemon mode")

path = os.environ["SKY_CUA_SERVICE_SOCKET_PATH"]
try:
    os.unlink(path)
except FileNotFoundError:
    pass

server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(path)
server.listen(8)

while True:
    conn, _ = server.accept()
    with conn:
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(4096)
            if not chunk:
                break
            data += chunk
        if not data:
            continue
        request = json.loads(data.decode("utf-8"))
        if request.get("type") == "health":
            response = {
                "type": "health",
                "ok": True,
                "service_socket": path,
                "desktop_env": {
                    key: os.environ[key] for key in [
                        "DBUS_SESSION_BUS_ADDRESS",
                        "DESKTOP_SESSION",
                        "DISPLAY",
                        "WAYLAND_DISPLAY",
                        "XDG_CURRENT_DESKTOP",
                        "XDG_RUNTIME_DIR",
                        "XDG_SESSION_TYPE",
                    ]
                    if os.environ.get(key)
                },
            }
        else:
            response = {"type": "error", "code": "UnexpectedRequest", "message": request.get("type", "<missing>")}
        conn.sendall(json.dumps(response).encode("utf-8") + b"\n")
"#;
}
