//! Shared managed overlay-host process lifecycle.
//!
//! The service supervises one overlay host child process and talks to it over
//! an endpoint (Unix socket on Unix, localhost TCP elsewhere). Process
//! supervision, startup readiness polling, request/reset policy, and Drop
//! shutdown are endpoint-independent and live here; endpoint adapters only
//! own serve arguments, connection mechanics, and endpoint artifact cleanup.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use sky_cua_overlay_host::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
};
use sky_cua_platform::model::DiagnosticEntry;

const HOST_START_TIMEOUT: Duration = Duration::from_secs(2);
const HOST_CONNECT_INTERVAL: Duration = Duration::from_millis(25);
#[cfg(any(not(unix), test))]
const HOST_TCP_CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const HOST_READ_TIMEOUT: Duration = Duration::from_secs(2);
const HOST_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const HOST_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Endpoint-specific mechanics behind [`ManagedOverlayHost`]: how the host
/// binary is asked to serve, how the service connects, and what endpoint
/// artifacts need cleanup when the child stops.
pub(in crate::overlay) trait OverlayHostEndpoint: std::fmt::Debug {
    type Stream: Read + Write;

    /// Arguments passed to the host binary after `serve`.
    fn serve_args(&self) -> Vec<OsString>;

    /// Pre-spawn preparation, such as creating the socket directory and
    /// removing a stale socket file.
    fn prepare_spawn(&self) -> Result<(), String>;

    /// Connect a request stream with read/write timeouts already applied.
    fn connect(&self) -> Result<Self::Stream, String>;

    /// Cheap startup readiness probe.
    fn ready_probe(&self) -> Result<(), String>;

    /// Remove endpoint artifacts (socket file) after the child stops.
    fn cleanup(&self);

    /// Error text for a host that never became ready at this endpoint.
    fn not_ready_error(&self, last_error: Option<String>) -> String;
}

/// Owns the overlay host child process and the request/reset/shutdown policy
/// shared by all endpoints.
#[derive(Debug)]
pub(in crate::overlay) struct ManagedOverlayHost<E: OverlayHostEndpoint> {
    host_path: PathBuf,
    endpoint: E,
    pub(super) child: Option<Child>,
    last_error: Option<String>,
}

impl<E: OverlayHostEndpoint> ManagedOverlayHost<E> {
    pub(in crate::overlay) fn new(host_path: PathBuf, endpoint: E) -> Self {
        Self {
            host_path,
            endpoint,
            child: None,
            last_error: None,
        }
    }

    pub(in crate::overlay) fn send(
        &mut self,
        message: OverlayHostMessage,
    ) -> Result<OverlayHostReply, DiagnosticEntry> {
        if let Err(error) = self.ensure_running() {
            self.last_error = Some(error.clone());
            return Err(diagnostic(
                "AgentCursorHostUnavailable",
                "Overlay host process is unavailable.",
                Some(client_error_detail(&error)),
            ));
        }

        match self.send_once(&message) {
            Ok(reply) => {
                self.last_error = None;
                Ok(reply)
            }
            Err(error) => {
                self.last_error = Some(error.clone());
                self.reset_child();
                Err(diagnostic(
                    "AgentCursorHostRequestFailed",
                    "Overlay host request failed.",
                    Some(client_error_detail(&error)),
                ))
            }
        }
    }

    pub(in crate::overlay) fn default_reason(&self) -> String {
        self.last_error
            .clone()
            .map(|error| client_error_detail(&error))
            .unwrap_or_else(|| "native visible overlay host has not reported yet".to_string())
    }

    fn ensure_running(&mut self) -> Result<(), String> {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(status)) => {
                    self.child = None;
                    self.endpoint.cleanup();
                    return Err(format!("overlay host exited early with status {status}"));
                }
                Err(error) => return Err(format!("failed to inspect overlay host: {error}")),
            }
        }

        if !self.host_path.is_file() {
            return Err(format!(
                "overlay host binary not found: {}",
                self.host_path.display()
            ));
        }
        self.endpoint.prepare_spawn()?;
        let child = Command::new(&self.host_path)
            .arg("serve")
            .args(self.endpoint.serve_args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                format!(
                    "failed to spawn overlay host {}: {error}",
                    self.host_path.display()
                )
            })?;
        self.child = Some(child);
        tokio::task::block_in_place(|| self.wait_for_ready())
    }

    fn wait_for_ready(&mut self) -> Result<(), String> {
        let started = Instant::now();
        let mut last_error = None;
        while started.elapsed() < HOST_START_TIMEOUT {
            if let Some(child) = self.child.as_mut() {
                match child.try_wait() {
                    Ok(None) => {}
                    Ok(Some(status)) => {
                        self.child = None;
                        self.endpoint.cleanup();
                        return Err(format!("overlay host exited during startup with {status}"));
                    }
                    Err(error) => {
                        return Err(format!(
                            "failed to inspect overlay host during startup: {error}"
                        ));
                    }
                }
            }

            match self.endpoint.ready_probe() {
                Ok(()) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
            std::thread::sleep(HOST_CONNECT_INTERVAL);
        }

        Err(self.endpoint.not_ready_error(last_error))
    }

    fn send_once(&self, message: &OverlayHostMessage) -> Result<OverlayHostReply, String> {
        let stream = self.endpoint.connect()?;
        send_overlay_host_message(stream, message)
    }

    fn reset_child(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.endpoint.cleanup();
    }
}

impl<E: OverlayHostEndpoint> Drop for ManagedOverlayHost<E> {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = self.send_once(&OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
        });
        let deadline = Instant::now() + HOST_STOP_TIMEOUT;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                self.endpoint.cleanup();
                return;
            }
            std::thread::sleep(HOST_CONNECT_INTERVAL);
        }
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.endpoint.cleanup();
    }
}

#[cfg(unix)]
#[derive(Debug)]
pub(in crate::overlay) struct UnixSocketEndpoint {
    socket_path: PathBuf,
}

#[cfg(unix)]
impl UnixSocketEndpoint {
    pub(in crate::overlay) fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }
}

#[cfg(unix)]
impl OverlayHostEndpoint for UnixSocketEndpoint {
    type Stream = UnixStream;

    fn serve_args(&self) -> Vec<OsString> {
        vec![OsString::from("--socket"), self.socket_path.clone().into()]
    }

    fn prepare_spawn(&self) -> Result<(), String> {
        // The host binary repeats this directory/stale-socket cleanup before
        // binding; keeping it here too surfaces preparation failures as a
        // pre-spawn unavailable diagnostic instead of a startup exit.
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create overlay socket directory: {error}"))?;
        }
        let _ = std::fs::remove_file(&self.socket_path);
        Ok(())
    }

    fn connect(&self) -> Result<Self::Stream, String> {
        let stream = UnixStream::connect(&self.socket_path).map_err(|error| {
            format!(
                "failed to connect to overlay host socket {}: {error}",
                self.socket_path.display()
            )
        })?;
        stream
            .set_read_timeout(Some(HOST_READ_TIMEOUT))
            .map_err(|error| format!("failed to set overlay host read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(HOST_WRITE_TIMEOUT))
            .map_err(|error| format!("failed to set overlay host write timeout: {error}"))?;
        Ok(stream)
    }

    fn ready_probe(&self) -> Result<(), String> {
        UnixStream::connect(&self.socket_path)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }

    fn not_ready_error(&self, last_error: Option<String>) -> String {
        format!(
            "overlay host socket did not become ready at {}{}",
            self.socket_path.display(),
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )
    }
}

/// Localhost TCP endpoint for platforms without Unix sockets. Also compiled
/// into Unix test builds so the shared lifecycle and this adapter stay
/// covered by Linux tests even though production Unix builds select the Unix
/// socket endpoint.
#[cfg(any(not(unix), test))]
#[derive(Debug)]
pub(in crate::overlay) struct TcpEndpoint {
    addr: String,
    /// Parsed once at construction for the common literal `host:port` form;
    /// hostname overrides stay `None` and resolve on each connect attempt so
    /// DNS changes are honored at the cost of repeated resolution.
    literal_addr: Option<std::net::SocketAddr>,
}

#[cfg(any(not(unix), test))]
impl TcpEndpoint {
    pub(in crate::overlay) fn new(addr: String) -> Self {
        let literal_addr = addr.parse().ok();
        Self { addr, literal_addr }
    }

    fn connect_tcp(&self, timeout: Duration) -> Result<std::net::TcpStream, String> {
        if let Some(addr) = self.literal_addr {
            return std::net::TcpStream::connect_timeout(&addr, timeout).map_err(|error| {
                format!(
                    "failed to connect to overlay host TCP listener {}: {error}",
                    self.addr
                )
            });
        }
        let addrs = self.resolved_addrs()?;
        let mut last_error = None;
        for addr in addrs {
            match std::net::TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => return Ok(stream),
                Err(error) => last_error = Some(error),
            }
        }
        Err(format!(
            "failed to connect to overlay host TCP listener {}{}",
            self.addr,
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        ))
    }

    fn resolved_addrs(&self) -> Result<Vec<std::net::SocketAddr>, String> {
        use std::net::ToSocketAddrs;

        let addrs = self
            .addr
            .to_socket_addrs()
            .map_err(|error| {
                format!(
                    "failed to resolve overlay host TCP address {}: {error}",
                    self.addr
                )
            })?
            .collect::<Vec<_>>();
        if addrs.is_empty() {
            return Err(format!(
                "overlay host TCP address resolved to no socket addresses: {}",
                self.addr
            ));
        }
        Ok(addrs)
    }
}

#[cfg(any(not(unix), test))]
impl OverlayHostEndpoint for TcpEndpoint {
    type Stream = std::net::TcpStream;

    fn serve_args(&self) -> Vec<OsString> {
        vec![OsString::from("--tcp"), OsString::from(&self.addr)]
    }

    fn prepare_spawn(&self) -> Result<(), String> {
        Ok(())
    }

    fn connect(&self) -> Result<Self::Stream, String> {
        let stream = self.connect_tcp(HOST_TCP_CONNECT_TIMEOUT)?;
        stream
            .set_read_timeout(Some(HOST_READ_TIMEOUT))
            .map_err(|error| format!("failed to set overlay host read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(HOST_WRITE_TIMEOUT))
            .map_err(|error| format!("failed to set overlay host write timeout: {error}"))?;
        Ok(stream)
    }

    fn ready_probe(&self) -> Result<(), String> {
        self.connect_tcp(HOST_TCP_CONNECT_TIMEOUT).map(|_| ())
    }

    fn cleanup(&self) {}

    fn not_ready_error(&self, last_error: Option<String>) -> String {
        format!(
            "overlay host TCP listener did not become ready at {}{}",
            self.addr,
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )
    }
}

fn send_overlay_host_message(
    mut stream: impl Read + Write,
    message: &OverlayHostMessage,
) -> Result<OverlayHostReply, String> {
    let payload = serde_json::to_vec(message)
        .map_err(|error| format!("failed to serialize overlay host request: {error}"))?;
    stream
        .write_all(&payload)
        .and_then(|()| stream.write_all(b"\n"))
        .and_then(|()| stream.flush())
        .map_err(|error| format!("failed to write overlay host request: {error}"))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| format!("failed to read overlay host reply: {error}"))?;
    if line.trim().is_empty() {
        return Err("overlay host returned an empty reply".to_string());
    }
    serde_json::from_str(line.trim_end())
        .map_err(|error| format!("invalid overlay host reply JSON: {error}"))
}

/// Map internal error strings to client-facing detail without leaking local
/// filesystem paths or addresses.
pub(super) fn client_error_detail(error: &str) -> String {
    if error.starts_with("overlay host binary not found:") {
        "overlay host binary not found".to_string()
    } else if error.starts_with("failed to spawn overlay host ") {
        "failed to spawn overlay host".to_string()
    } else if error.starts_with("overlay host socket did not become ready at ") {
        "overlay host socket did not become ready".to_string()
    } else if error.starts_with("failed to connect to overlay host socket ") {
        "failed to connect to overlay host socket".to_string()
    } else if error.starts_with("overlay host TCP listener did not become ready at ") {
        "overlay host TCP listener did not become ready".to_string()
    } else if error.starts_with("failed to connect to overlay host TCP listener ") {
        "failed to connect to overlay host TCP listener".to_string()
    } else if error.starts_with("failed to resolve overlay host TCP address ") {
        "failed to resolve overlay host TCP address".to_string()
    } else if error.starts_with("overlay host TCP address resolved to no socket addresses") {
        "overlay host TCP address resolved to no socket addresses".to_string()
    } else {
        error.to_string()
    }
}

pub(super) fn diagnostic(code: &str, message: &str, details: Option<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::client_error_detail;

    /// Every endpoint/lifecycle error string that embeds a local path or
    /// address must map to a redacted client-facing detail. New error sites
    /// must extend `client_error_detail` and this list together.
    #[test]
    fn client_error_detail_redacts_paths_and_addresses() {
        let cases = [
            (
                "overlay host binary not found: /home/user/bin/host",
                "overlay host binary not found",
            ),
            (
                "failed to spawn overlay host /home/user/bin/host: oops",
                "failed to spawn overlay host",
            ),
            (
                "overlay host socket did not become ready at /run/user/1/x.sock: refused",
                "overlay host socket did not become ready",
            ),
            (
                "failed to connect to overlay host socket /run/user/1/x.sock: refused",
                "failed to connect to overlay host socket",
            ),
            (
                "overlay host TCP listener did not become ready at 127.0.0.1:1: refused",
                "overlay host TCP listener did not become ready",
            ),
            (
                "failed to connect to overlay host TCP listener 127.0.0.1:1: refused",
                "failed to connect to overlay host TCP listener",
            ),
            (
                "failed to resolve overlay host TCP address example.internal:1: nx",
                "failed to resolve overlay host TCP address",
            ),
            (
                "overlay host TCP address resolved to no socket addresses: example.internal:1",
                "overlay host TCP address resolved to no socket addresses",
            ),
        ];
        for (raw, expected) in cases {
            assert_eq!(client_error_detail(raw), expected, "raw: {raw}");
        }
    }
}
