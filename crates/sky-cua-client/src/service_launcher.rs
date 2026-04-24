use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use sky_cua_platform::{
    model::{ServiceRequest, ServiceResponse},
    service_socket_path,
};

const SERVICE_READ_TIMEOUT: Duration = Duration::from_secs(60);
const SERVICE_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const STARTUP_HEALTH_READ_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_HEALTH_WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(150);
const STARTUP_HEALTH_ATTEMPTS: usize = 40;
#[derive(Debug, Clone)]
pub struct ServiceClient {
    socket_path: PathBuf,
}

impl ServiceClient {
    pub fn connect_or_spawn() -> Result<Self> {
        let client = Self {
            socket_path: service_socket_path(),
        };
        // Startup probes must fail fast; a stale or half-ready daemon should not
        // consume the entire MCP server startup window before we even spawn a
        // fresh service instance.
        if client.startup_health().is_ok() {
            return Ok(client);
        }

        let service_path = service_path();
        Command::new(&service_path)
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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
            client.socket_path.display()
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
        let mut stream = UnixStream::connect(&self.socket_path).with_context(|| {
            format!(
                "failed to connect to sky-cua-service socket {}",
                self.socket_path.display()
            )
        })?;
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
}

fn service_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SKY_CUA_SERVICE_PATH") {
        return PathBuf::from(path);
    }
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(sibling) = exe_path
            .parent()
            .map(|parent| parent.join("sky-cua-service"))
        && sibling.is_file()
    {
        return sibling;
    }
    let repo_root = std::env::var_os("SKY_CUA_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    repo_root.join("bin").join("sky-cua-service")
}
