#[cfg(unix)]
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use sky_cua_platform::{
    CLIENT_CLEARED_SESSION_ENV_KEYS_ENV,
    model::{DoctorSessionEnvRepair, ServiceRequest, ServiceResponse},
};
use sky_cua_platform::{CLIENT_SESSION_ENV_REPAIRS_ENV, GRAPHICAL_SESSION_ENV_KEYS};
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
const STARTUP_HEALTH_ATTEMPTS: usize = 160;
#[cfg(unix)]
const STALE_SERVICE_TERMINATION_TIMEOUT: Duration = Duration::from_secs(3);
#[derive(Debug, Clone)]
pub struct ServiceClient {
    endpoint: ServiceEndpoint,
    child: Arc<Mutex<Option<Child>>>,
    cached_stream: Arc<Mutex<Option<EitherStream>>>,
}

impl ServiceClient {
    pub fn connect_or_spawn() -> Result<Self> {
        let launch_environment = LaunchEnvironment::probe();
        let client = Self::new(&launch_environment)?;
        // Startup probes must fail fast; a stale or half-ready daemon should not
        // consume the entire MCP server startup window before we even spawn a
        // fresh service instance.
        match client.startup_health(&launch_environment) {
            Ok(_) => return Ok(client),
            Err(error) => {
                if is_stale_startup_health_error(&error) {
                    client.displace_stale_service(&error)?;
                }
            }
        }

        client.spawn_service(&launch_environment)?;
        client.wait_for_startup_health(&launch_environment)?;
        Ok(client)
    }

    fn startup_health(&self, launch_environment: &LaunchEnvironment) -> Result<ServiceResponse> {
        let (response, owner_pid) = self.call_with_timeouts_with_peer(
            &ServiceRequest::Health,
            STARTUP_HEALTH_READ_TIMEOUT,
            STARTUP_HEALTH_WRITE_TIMEOUT,
        )?;
        if let Err(error) = launch_environment.ensure_startup_health(&response) {
            return Err(anyhow!(StaleStartupService {
                detail: error.to_string(),
                owner_pid,
            }));
        }
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
        self.call_with_timeouts_with_peer(request, read_timeout, write_timeout)
            .map(|(response, _)| response)
    }

    fn call_with_timeouts_with_peer(
        &self,
        request: &ServiceRequest,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<(ServiceResponse, Option<u32>)> {
        // Attempt 1: try cached stream if available.
        let mut cached_response: Option<Result<ServiceResponse>> = None;
        if let Some(stream) = self.take_cached_stream() {
            match self.perform_call_on_stream(stream, request, read_timeout, write_timeout) {
                Ok((response, stream, owner_pid)) => {
                    self.store_cached_stream(stream);
                    return Ok((response, owner_pid));
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
        let (response, stream, owner_pid) =
            self.perform_call_on_stream(stream, request, read_timeout, write_timeout)?;
        self.store_cached_stream(stream);
        Ok((response, owner_pid))
    }

    fn perform_call_on_stream(
        &self,
        mut stream: EitherStream,
        request: &ServiceRequest,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<(ServiceResponse, EitherStream, Option<u32>)> {
        let owner_pid = stream.peer_pid();
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
        Ok((response, stream, owner_pid))
    }

    fn new(launch_environment: &LaunchEnvironment) -> Result<Self> {
        Ok(Self {
            endpoint: ServiceEndpoint::new(launch_environment)?,
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

        configure_launch_environment_env(&mut command, launch_environment);

        self.endpoint.configure_service_command(&mut command);
        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn service at {}", service_path.display()))?;
        *child_guard = Some(child);
        Ok(())
    }

    fn wait_for_startup_health(&self, launch_environment: &LaunchEnvironment) -> Result<()> {
        let mut last_error: Option<anyhow::Error> = None;
        for _ in 0..STARTUP_HEALTH_ATTEMPTS {
            match self.startup_health(launch_environment) {
                Ok(_) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
            self.reap_exited_child()?;
            thread::sleep(STARTUP_POLL_INTERVAL);
        }

        // Surface the concrete per-poll failure; "did not become healthy"
        // alone hides whether the daemon was unreachable, slow, or rejected
        // by an environment staleness check.
        let detail = last_error
            .map(|error| format!(": last health error: {error:#}"))
            .unwrap_or_default();
        Err(anyhow!(
            "sky-cua-service did not become healthy on {}{detail}",
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

    #[cfg(unix)]
    fn displace_stale_service(&self, reason: &anyhow::Error) -> Result<()> {
        let fallback_owner_pid = reason
            .downcast_ref::<StaleStartupService>()
            .and_then(|error| error.owner_pid);
        match self.endpoint.terminate_stale_owners(fallback_owner_pid) {
            Ok(killed) if !killed.is_empty() => {
                self.clear_cached_stream();
                self.endpoint.wait_for_singleton_release()?;
                Ok(())
            }
            Ok(_) => Err(anyhow!(
                "existing sky-cua-service is stale ({reason}) but its singleton owner could not be identified"
            )),
            Err(error) => Err(error).context("failed to terminate stale sky-cua-service"),
        }
    }

    #[cfg(windows)]
    fn displace_stale_service(&self, reason: &anyhow::Error) -> Result<()> {
        Err(anyhow!(
            "existing sky-cua-service is stale ({reason}) and automatic daemon replacement is not implemented on Windows"
        ))
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
    fn new(launch_environment: &LaunchEnvironment) -> Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self::Unix(service_socket_path_for_launch_environment(
                launch_environment,
            )))
        }
        #[cfg(windows)]
        {
            let _ = launch_environment;
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

    #[cfg(unix)]
    fn terminate_stale_owners(&self, fallback_owner_pid: Option<u32>) -> Result<Vec<u32>> {
        let Self::Unix(path) = self;
        let candidates = owner_pids_for_termination(path, fallback_owner_pid)?;
        let mut killed = Vec::new();
        for pid in candidates {
            // SAFETY: `kill` with SIGTERM has no Rust-side memory safety
            // preconditions. Candidate PIDs are either the connected Unix
            // socket peer or a lock-file owner that still looks like our
            // service binary.
            let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            if result == 0 {
                killed.push(pid);
            } else {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    killed.push(pid);
                } else {
                    return Err(error.into());
                }
            }
        }
        Ok(killed)
    }

    #[cfg(unix)]
    fn wait_for_singleton_release(&self) -> Result<()> {
        let Self::Unix(path) = self;
        let deadline = Instant::now() + STALE_SERVICE_TERMINATION_TIMEOUT;
        while Instant::now() < deadline {
            if singleton_lock_is_available(path)? {
                return Ok(());
            }
            thread::sleep(STARTUP_POLL_INTERVAL);
        }
        Err(anyhow!(
            "timed out waiting for stale sky-cua-service singleton lock to release"
        ))
    }
}

#[cfg(unix)]
fn service_socket_path_for_launch_environment(launch_environment: &LaunchEnvironment) -> PathBuf {
    if std::env::var_os(SERVICE_SOCKET_PATH_ENV).is_some() {
        return service_socket_path();
    }

    if std::env::var_os("XDG_RUNTIME_DIR").is_none_or(|value| value.is_empty())
        && let Some(runtime_dir) = launch_environment.repaired_desktop_var("XDG_RUNTIME_DIR")
    {
        return PathBuf::from(runtime_dir)
            .join("sky-cua")
            .join("service.sock");
    }

    service_socket_path()
}

#[cfg(unix)]
fn service_lock_path(socket_path: &std::path::Path) -> PathBuf {
    let mut lock_name = socket_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("service.sock"));
    lock_name.push(".lock");
    socket_path.with_file_name(lock_name)
}

#[cfg(unix)]
fn read_singleton_owner_pid(socket_path: &std::path::Path) -> Result<Option<u32>> {
    let lock_path = service_lock_path(socket_path);
    let Ok(raw) = std::fs::read_to_string(&lock_path) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let pid = trimmed.parse::<u32>().with_context(|| {
        format!(
            "invalid sky-cua-service singleton owner pid in {}",
            lock_path.display()
        )
    })?;
    Ok((pid > 1).then_some(pid))
}

#[cfg(unix)]
fn owner_pids_for_termination(
    socket_path: &std::path::Path,
    peer_pid: Option<u32>,
) -> Result<Vec<u32>> {
    let mut candidates = BTreeSet::new();
    if let Some(pid) = peer_pid {
        candidates.insert(pid);
    }
    if let Some(pid) = read_singleton_owner_pid(socket_path)?
        && pid_looks_like_sky_cua_service(pid)
    {
        candidates.insert(pid);
    }
    Ok(candidates.into_iter().collect())
}

#[cfg(unix)]
fn pid_looks_like_sky_cua_service(pid: u32) -> bool {
    let proc_root = PathBuf::from(format!("/proc/{pid}"));
    let exe_name = std::fs::read_link(proc_root.join("exe"))
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_os_string()));
    let cmdline = std::fs::read(proc_root.join("cmdline")).ok();
    process_identity_looks_like_sky_cua_service(exe_name.as_deref(), cmdline.as_deref())
}

#[cfg(unix)]
fn process_identity_looks_like_sky_cua_service(
    exe_name: Option<&std::ffi::OsStr>,
    cmdline: Option<&[u8]>,
) -> bool {
    exe_name.is_some_and(|name| name == "sky-cua-service")
        || cmdline.is_some_and(|bytes| {
            bytes
                .split(|byte| *byte == 0)
                .filter_map(|part| std::str::from_utf8(part).ok())
                .any(|part| {
                    std::path::Path::new(part)
                        .file_name()
                        .is_some_and(|name| name == "sky-cua-service")
                })
        })
}

#[cfg(unix)]
fn singleton_lock_is_available(socket_path: &std::path::Path) -> Result<bool> {
    use std::os::unix::io::AsRawFd;

    let lock_path = service_lock_path(socket_path);
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    let result = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        let _ = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_UN) };
        Ok(true)
    } else {
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => Ok(false),
            Some(libc::EINTR) => Ok(false),
            _ => Err(error.into()),
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

fn configure_launch_environment_env(command: &mut Command, launch_environment: &LaunchEnvironment) {
    // Forward reconstructed desktop session env vars so the service can
    // initialize its platform backends even when the MCP host did not pass
    // them through.
    if launch_environment.detached_graphical_env() {
        for key in GRAPHICAL_SESSION_ENV_KEYS {
            command.env_remove(key);
        }
        if let Ok(serialized) = serde_json::to_string(GRAPHICAL_SESSION_ENV_KEYS) {
            command.env(CLIENT_CLEARED_SESSION_ENV_KEYS_ENV, serialized);
        }
    }
    for (key, value) in launch_environment.repaired_desktop_vars() {
        command.env(key, value);
    }
    if !launch_environment.repaired_desktop_vars().is_empty() {
        let repairs = launch_environment
            .repaired_desktop_vars()
            .iter()
            .map(|(key, value)| DoctorSessionEnvRepair {
                key: key.clone(),
                source: "client-launch".to_string(),
                value: Some(value.clone()),
            })
            .collect::<Vec<_>>();
        if let Ok(serialized) = serde_json::to_string(&repairs) {
            command.env(CLIENT_SESSION_ENV_REPAIRS_ENV, serialized);
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

    fn peer_pid(&self) -> Option<u32> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Unix(stream) => unix_stream_peer_pid(stream),
            #[cfg(all(unix, not(target_os = "linux")))]
            Self::Unix(_stream) => None,
            #[cfg(windows)]
            Self::Tcp(_stream) => None,
        }
    }
}

#[cfg(target_os = "linux")]
fn unix_stream_peer_pid(stream: &UnixStream) -> Option<u32> {
    use std::mem::MaybeUninit;
    use std::os::unix::io::AsRawFd;

    let mut credentials = MaybeUninit::<libc::ucred>::uninit();
    let mut credentials_len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut credentials_len,
        )
    };
    if result != 0 || credentials_len < std::mem::size_of::<libc::ucred>() as libc::socklen_t {
        return None;
    }
    let credentials = unsafe { credentials.assume_init() };
    (credentials.pid > 1).then_some(credentials.pid as u32)
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

#[derive(Debug)]
struct StaleStartupService {
    detail: String,
    owner_pid: Option<u32>,
}

impl std::fmt::Display for StaleStartupService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "existing sky-cua-service is stale: {}",
            self.detail
        )
    }
}

impl std::error::Error for StaleStartupService {}

fn is_stale_startup_health_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<StaleStartupService>().is_some()
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
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use sky_cua_platform::{
        CLIENT_CLEARED_SESSION_ENV_KEYS_ENV, CLIENT_SESSION_ENV_REPAIRS_ENV,
        SERVICE_SOCKET_PATH_ENV,
    };

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
    fn detached_launch_env_removes_stale_graphical_keys_before_repairs() {
        let launch_environment =
            LaunchEnvironment::from_repaired_desktop_vars_and_detached_for_tests(
                vec![
                    ("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string()),
                    ("DISPLAY".to_string(), ":0".to_string()),
                ],
                true,
            );
        let mut command = Command::new("sky-cua-service");

        configure_launch_environment_env(&mut command, &launch_environment);

        assert_eq!(command_env_value(&command, "WAYLAND_DISPLAY"), Some(None));
        assert_eq!(
            command_env_value(&command, "XDG_RUNTIME_DIR"),
            Some(Some(OsStr::new("/run/user/1000")))
        );
        assert_eq!(
            command_env_value(&command, "DISPLAY"),
            Some(Some(OsStr::new(":0")))
        );

        let raw_repairs = command_env_value(&command, CLIENT_SESSION_ENV_REPAIRS_ENV)
            .and_then(|value| value)
            .and_then(OsStr::to_str)
            .expect("client launch repairs should be serialized");
        let repairs = serde_json::from_str::<Vec<DoctorSessionEnvRepair>>(raw_repairs)
            .expect("client launch repairs should be valid JSON");
        assert_eq!(repairs.len(), 2);
        assert!(
            repairs
                .iter()
                .all(|repair| repair.source == "client-launch")
        );
        assert!(repairs.iter().any(|repair| {
            repair.key == "XDG_RUNTIME_DIR" && repair.value.as_deref() == Some("/run/user/1000")
        }));

        let raw_cleared = command_env_value(&command, CLIENT_CLEARED_SESSION_ENV_KEYS_ENV)
            .and_then(|value| value)
            .and_then(OsStr::to_str)
            .expect("client cleared keys should be serialized");
        let cleared =
            serde_json::from_str::<Vec<String>>(raw_cleared).expect("cleared keys should be JSON");
        assert!(cleared.iter().any(|key| key == "DISPLAY"));
        assert!(cleared.iter().any(|key| key == "WAYLAND_DISPLAY"));
    }

    #[test]
    fn unix_service_endpoint_uses_repaired_runtime_dir_before_cache_fallback() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_socket_path = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        let old_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::remove_var(SERVICE_SOCKET_PATH_ENV);
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        let launch_environment = LaunchEnvironment::from_repaired_desktop_vars_for_tests(vec![(
            "XDG_RUNTIME_DIR".to_string(),
            "/run/user/1000".to_string(),
        )]);

        let endpoint = ServiceEndpoint::new(&launch_environment).expect("endpoint should resolve");

        restore_env(SERVICE_SOCKET_PATH_ENV, old_socket_path);
        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        match endpoint {
            ServiceEndpoint::Unix(path) => {
                assert_eq!(path, PathBuf::from("/run/user/1000/sky-cua/service.sock"));
            }
        }
    }

    #[test]
    fn unix_service_endpoint_treats_empty_runtime_dir_as_missing() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_socket_path = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        let old_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::remove_var(SERVICE_SOCKET_PATH_ENV);
            std::env::set_var("XDG_RUNTIME_DIR", "");
        }
        let launch_environment = LaunchEnvironment::from_repaired_desktop_vars_for_tests(vec![(
            "XDG_RUNTIME_DIR".to_string(),
            "/run/user/1000".to_string(),
        )]);

        let endpoint = ServiceEndpoint::new(&launch_environment).expect("endpoint should resolve");

        restore_env(SERVICE_SOCKET_PATH_ENV, old_socket_path);
        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        match endpoint {
            ServiceEndpoint::Unix(path) => {
                assert_eq!(path, PathBuf::from("/run/user/1000/sky-cua/service.sock"));
            }
        }
    }

    #[test]
    fn unix_service_endpoint_preserves_explicit_socket_override() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_socket_path = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        let old_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::set_var(SERVICE_SOCKET_PATH_ENV, "/tmp/sky-cua-explicit.sock");
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        let launch_environment = LaunchEnvironment::from_repaired_desktop_vars_for_tests(vec![(
            "XDG_RUNTIME_DIR".to_string(),
            "/run/user/1000".to_string(),
        )]);

        let endpoint = ServiceEndpoint::new(&launch_environment).expect("endpoint should resolve");

        restore_env(SERVICE_SOCKET_PATH_ENV, old_socket_path);
        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        match endpoint {
            ServiceEndpoint::Unix(path) => {
                assert_eq!(path, PathBuf::from("/tmp/sky-cua-explicit.sock"));
            }
        }
    }

    #[test]
    fn closed_cached_connection_is_retryable() {
        let error = anyhow!("sky-cua-service connection closed before response");

        assert!(is_stale_stream_error(&error));
    }

    #[test]
    fn startup_health_stale_error_is_typed() {
        let error = anyhow!(StaleStartupService {
            detail: "DISPLAY".to_string(),
            owner_pid: Some(4242),
        });

        assert!(is_stale_startup_health_error(&error));
        assert_eq!(
            error
                .downcast_ref::<StaleStartupService>()
                .and_then(|error| error.owner_pid),
            Some(4242)
        );
    }

    #[test]
    fn unix_service_lock_path_sits_next_to_socket() {
        assert_eq!(
            service_lock_path(std::path::Path::new("/tmp/sky-cua/service.sock")),
            PathBuf::from("/tmp/sky-cua/service.sock.lock")
        );
    }

    #[test]
    fn unix_service_lock_owner_pid_parses_valid_lock_file() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-client-lock-pid-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let socket_path = temp_dir.join("service.sock");
        fs::write(service_lock_path(&socket_path), "4242\n").expect("write lock pid");

        let pid = read_singleton_owner_pid(&socket_path).expect("pid should parse");

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(pid, Some(4242));
    }

    #[test]
    fn unix_service_termination_ignores_unverified_lock_pid_when_peer_is_known() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-client-peer-pid-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let socket_path = temp_dir.join("service.sock");
        fs::write(service_lock_path(&socket_path), "4242\n").expect("write lock pid");

        let pids =
            owner_pids_for_termination(&socket_path, Some(7777)).expect("pid should resolve");

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(pids, vec![7777]);
    }

    #[test]
    fn unix_service_process_identity_matches_service_binary_name() {
        assert!(process_identity_looks_like_sky_cua_service(
            Some(OsStr::new("sky-cua-service")),
            None,
        ));
        assert!(process_identity_looks_like_sky_cua_service(
            None,
            Some(b"/home/bex/.local/share/sky-cua/bin/sky-cua-service\0daemon\0"),
        ));
        assert!(!process_identity_looks_like_sky_cua_service(
            Some(OsStr::new("unrelated")),
            Some(b"/usr/bin/unrelated\0"),
        ));
    }

    #[test]
    fn startup_health_budget_allows_slow_desktop_service_startup() {
        let budget = STARTUP_POLL_INTERVAL * STARTUP_HEALTH_ATTEMPTS as u32;

        assert!(budget >= Duration::from_secs(20));
        assert!(budget < Duration::from_secs(30));
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

    fn command_env_value<'a>(
        command: &'a Command,
        key: &str,
    ) -> Option<Option<&'a std::ffi::OsStr>> {
        command
            .get_envs()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, value)| value)
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
