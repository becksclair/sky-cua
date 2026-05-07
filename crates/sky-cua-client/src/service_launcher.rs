use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use sky_cua_platform::model::{ServiceRequest, ServiceResponse};
#[cfg(unix)]
use sky_cua_platform::service_socket_path;
#[cfg(windows)]
use sky_cua_platform::{SERVICE_TCP_ADDR_ENV, service_tcp_addr};

const SERVICE_READ_TIMEOUT: Duration = Duration::from_secs(60);
const SERVICE_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const STARTUP_HEALTH_READ_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_HEALTH_WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(150);
const STARTUP_HEALTH_ATTEMPTS: usize = 40;
#[derive(Debug, Clone)]
pub struct ServiceClient {
    endpoint: ServiceEndpoint,
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

        let service_path = service_path();
        let mut command = Command::new(&service_path);
        command
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        client.endpoint.configure_service_command(&mut command);
        command
            .spawn()
            .with_context(|| format!("failed to spawn service at {}", service_path.display()))?;

        for _ in 0..STARTUP_HEALTH_ATTEMPTS {
            if client.startup_health().is_ok() {
                return Ok(client);
            }
            thread::sleep(STARTUP_POLL_INTERVAL);
        }

        Err(anyhow!(
            "sky-cua-service did not become healthy on {}",
            client.endpoint
        ))
    }

    fn startup_health(&self) -> Result<ServiceResponse> {
        self.call_with_timeouts(
            &ServiceRequest::Health,
            STARTUP_HEALTH_READ_TIMEOUT,
            STARTUP_HEALTH_WRITE_TIMEOUT,
        )
    }

    pub fn clear_portal_tokens(&self) -> Result<ServiceResponse> {
        self.call(&ServiceRequest::ResetPortalTokens)
    }

    pub fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse> {
        self.call_with_timeouts(request, SERVICE_READ_TIMEOUT, SERVICE_WRITE_TIMEOUT)
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
        })
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
            Self::Unix(_) => {}
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
