use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
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

use crate::launch_environment::LaunchEnvironment;

const SERVICE_READ_TIMEOUT: Duration = Duration::from_secs(60);
const SERVICE_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const STARTUP_HEALTH_READ_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_HEALTH_WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(150);
const STARTUP_HEALTH_ATTEMPTS: usize = 40;
#[derive(Debug, Clone)]
pub struct ServiceClient {
    endpoint: ServiceEndpoint,
    child: Arc<Mutex<Option<Child>>>,
    cached_stream: Arc<Mutex<Option<EitherStream>>>,
}

impl ServiceClient {
    pub fn connect_or_spawn() -> Result<Self> {
        let client = Self::new()?;
        let launch_environment = LaunchEnvironment::probe();
        // Startup probes must fail fast; a stale or half-ready daemon should not
        // consume the entire MCP server startup window before we even spawn a
        // fresh service instance.
        if client.startup_health(&launch_environment).is_ok() {
            return Ok(client);
        }

        client.spawn_service(&launch_environment)?;
        client.wait_for_startup_health(&launch_environment)?;
        Ok(client)
    }

    fn startup_health(&self, launch_environment: &LaunchEnvironment) -> Result<ServiceResponse> {
        let response = self.call_with_timeouts(
            &ServiceRequest::Health,
            STARTUP_HEALTH_READ_TIMEOUT,
            STARTUP_HEALTH_WRITE_TIMEOUT,
        )?;
        launch_environment.ensure_startup_health(&response)?;
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
                let launch_environment = LaunchEnvironment::probe();
                self.spawn_service(&launch_environment)?;
                self.wait_for_startup_health(&launch_environment)
                    .with_context(|| format!("after service call failed: {first_error}"))?;
                self.call_with_timeouts(request, SERVICE_READ_TIMEOUT, SERVICE_WRITE_TIMEOUT)
                    .with_context(|| format!("after service call failed: {first_error}"))
            }
        }
    }

    fn take_cached_stream(&self) -> Option<EitherStream> {
        self.cached_stream
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    fn store_cached_stream(&self, stream: EitherStream) {
        let _ = self
            .cached_stream
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(stream);
    }

    fn clear_cached_stream(&self) {
        let _ = self
            .cached_stream
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }

    fn call_with_timeouts(
        &self,
        request: &ServiceRequest,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<ServiceResponse> {
        // Attempt 1: try cached stream if available.
        let mut cached_response: Option<Result<ServiceResponse>> = None;
        if let Some(stream) = self.take_cached_stream() {
            match self.perform_call_on_stream(stream, request, read_timeout, write_timeout) {
                Ok((response, stream)) => {
                    self.store_cached_stream(stream);
                    return Ok(response);
                }
                Err(error) if is_stale_stream_error(&error) => {
                    self.clear_cached_stream();
                }
                Err(error) => {
                    self.clear_cached_stream();
                    cached_response = Some(Err(error));
                }
            }
        }

        // If we got a non-stale error from the cached stream, return it directly
        // rather than retrying with a fresh connect.
        if let Some(Err(error)) = cached_response {
            return Err(error);
        }

        // Attempt 2: fresh connection.
        let stream = self.endpoint.connect()?;
        let (response, stream) =
            self.perform_call_on_stream(stream, request, read_timeout, write_timeout)?;
        self.store_cached_stream(stream);
        Ok(response)
    }

    fn perform_call_on_stream(
        &self,
        mut stream: EitherStream,
        request: &ServiceRequest,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<(ServiceResponse, EitherStream)> {
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
            return Err(anyhow!("sky-cua-service connection closed before response"));
        }
        let response: ServiceResponse = serde_json::from_str(line.trim_end())?;
        let stream = reader.into_inner();
        Ok((response, stream))
    }

    fn new() -> Result<Self> {
        Ok(Self {
            endpoint: ServiceEndpoint::new()?,
            child: Arc::new(Mutex::new(None)),
            cached_stream: Arc::new(Mutex::new(None)),
        })
    }

    fn spawn_service(&self, launch_environment: &LaunchEnvironment) -> Result<()> {
        let mut child_guard = self
            .child
            .lock()
            .map_err(|_| anyhow!("sky-cua-service child state mutex was poisoned"))?;
        if let Some(child) = child_guard.as_mut()
            && child.try_wait()?.is_none()
        {
            return Ok(());
        }

        // Drop any cached stream from a previous service process before
        // spawning a new one so we don't write to a dead socket.
        self.clear_cached_stream();

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
        for (key, value) in launch_environment.repaired_desktop_vars() {
            command.env(key, value);
        }

        self.endpoint.configure_service_command(&mut command);
        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn service at {}", service_path.display()))?;
        *child_guard = Some(child);
        Ok(())
    }

    fn wait_for_startup_health(&self, launch_environment: &LaunchEnvironment) -> Result<()> {
        for _ in 0..STARTUP_HEALTH_ATTEMPTS {
            if self.startup_health(launch_environment).is_ok() {
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

#[derive(Debug)]
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

fn is_stale_stream_error(error: &anyhow::Error) -> bool {
    let error_string = error.to_string().to_lowercase();
    error_string.contains("broken pipe")
        || error_string.contains("connection refused")
        || error_string.contains("connection reset")
        || error_string.contains("connection closed before response")
        || error_string.contains("not connected")
        || error_string.contains("unexpected eof")
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
    fn closed_cached_connection_is_retryable() {
        let error = anyhow!("sky-cua-service connection closed before response");

        assert!(is_stale_stream_error(&error));
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
                        "PATH",
                        "WAYLAND_DISPLAY",
                        "XDG_CURRENT_DESKTOP",
                        "XDG_RUNTIME_DIR",
                        "XDG_SESSION_TYPE",
                    ]
                    if os.environ.get(key)
                },
                "browser_env": {
                    key: os.environ[key] for key in [
                        "SKY_CUA_BROWSER_USE_SOCKET_DIR",
                        "CODEX_BROWSER_USE_SOCKET_DIR",
                        "SKY_CUA_BROWSER",
                    ]
                    if os.environ.get(key)
                },
            }
        else:
            response = {"type": "error", "code": "UnexpectedRequest", "message": request.get("type", "<missing>")}
        conn.sendall(json.dumps(response).encode("utf-8") + b"\n")
"#;
}
