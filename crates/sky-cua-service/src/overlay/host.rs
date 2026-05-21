#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use sky_cua_overlay_host::OverlayHostMessageKind;
use sky_cua_overlay_host::{OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostReply};
use sky_cua_platform::model::DiagnosticEntry;

const OVERLAY_BACKEND_ENV: &str = "SKY_CUA_OVERLAY_BACKEND";
#[cfg(unix)]
const OVERLAY_HOST_PATH_ENV: &str = "SKY_CUA_OVERLAY_HOST_PATH";
#[cfg(unix)]
const HOST_START_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(unix)]
const HOST_CONNECT_INTERVAL: Duration = Duration::from_millis(25);
#[cfg(unix)]
const HOST_READ_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(unix)]
const HOST_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(unix)]
const HOST_STOP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(super) enum OverlayHostConnection {
    Disabled {
        reason: String,
        report_diagnostic: bool,
    },
    #[cfg(test)]
    Failing { diagnostic: DiagnosticEntry },
    #[cfg(unix)]
    Process(ProcessOverlayHostClient),
}

impl OverlayHostConnection {
    pub(super) fn from_service_socket(service_socket_path: &Path) -> Self {
        if overlay_backend_disabled() {
            return Self::Disabled {
                reason: format!("{OVERLAY_BACKEND_ENV}=none"),
                report_diagnostic: false,
            };
        }

        #[cfg(unix)]
        {
            let Some(socket_path) = overlay_socket_path(service_socket_path) else {
                return Self::Disabled {
                    reason: "service socket path has no parent directory".to_string(),
                    report_diagnostic: true,
                };
            };
            Self::Process(ProcessOverlayHostClient::new(
                overlay_host_path(),
                socket_path,
            ))
        }

        #[cfg(not(unix))]
        {
            let _ = service_socket_path;
            Self::Disabled {
                reason: "overlay host process IPC is not implemented on this platform yet"
                    .to_string(),
                report_diagnostic: true,
            }
        }
    }

    #[cfg(test)]
    pub(super) fn disabled_for_tests() -> Self {
        Self::Disabled {
            reason: "test overlay host disabled".to_string(),
            report_diagnostic: false,
        }
    }

    #[cfg(test)]
    pub(super) fn failing_for_tests(code: &str) -> Self {
        Self::Failing {
            diagnostic: diagnostic(code, "test overlay host failure", None),
        }
    }

    #[cfg(all(test, unix))]
    pub(super) fn process_for_tests(host_path: PathBuf, socket_path: PathBuf) -> Self {
        Self::Process(ProcessOverlayHostClient::new(host_path, socket_path))
    }

    pub(super) fn send(
        &mut self,
        message: OverlayHostMessage,
    ) -> Result<OverlayHostReply, DiagnosticEntry> {
        match self {
            Self::Disabled {
                reason,
                report_diagnostic,
            } => {
                if *report_diagnostic {
                    Err(diagnostic(
                        "AgentCursorHostUnavailable",
                        "Overlay host is not available.",
                        Some(reason.clone()),
                    ))
                } else {
                    Ok(OverlayHostReply {
                        version: OVERLAY_HOST_PROTOCOL_VERSION,
                        ok: true,
                        capabilities: None,
                        state: None,
                        diagnostics: Vec::new(),
                    })
                }
            }
            #[cfg(test)]
            Self::Failing { diagnostic } => Err(diagnostic.clone()),
            #[cfg(unix)]
            Self::Process(client) => client.send(message),
        }
    }

    pub(super) fn default_reason(&self) -> String {
        match self {
            Self::Disabled { reason, .. } => reason.clone(),
            #[cfg(test)]
            Self::Failing { diagnostic } => diagnostic.message.clone(),
            #[cfg(unix)]
            Self::Process(client) => client
                .last_error
                .clone()
                .map(|error| client_error_detail(&error))
                .unwrap_or_else(|| "native visible overlay host has not reported yet".to_string()),
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
pub(super) struct ProcessOverlayHostClient {
    host_path: PathBuf,
    socket_path: PathBuf,
    child: Option<Child>,
    last_error: Option<String>,
}

#[cfg(unix)]
impl ProcessOverlayHostClient {
    fn new(host_path: PathBuf, socket_path: PathBuf) -> Self {
        Self {
            host_path,
            socket_path,
            child: None,
            last_error: None,
        }
    }

    fn send(&mut self, message: OverlayHostMessage) -> Result<OverlayHostReply, DiagnosticEntry> {
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

    fn ensure_running(&mut self) -> Result<(), String> {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(status)) => {
                    self.child = None;
                    let _ = fs::remove_file(&self.socket_path);
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
        if let Some(parent) = self.socket_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create overlay socket directory: {error}"))?;
        }
        let _ = fs::remove_file(&self.socket_path);
        let child = Command::new(&self.host_path)
            .arg("serve")
            .arg("--socket")
            .arg(&self.socket_path)
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
        self.wait_for_socket()
    }

    fn wait_for_socket(&mut self) -> Result<(), String> {
        let started = Instant::now();
        let mut last_error = None;
        while started.elapsed() < HOST_START_TIMEOUT {
            if let Some(child) = self.child.as_mut() {
                match child.try_wait() {
                    Ok(None) => {}
                    Ok(Some(status)) => {
                        self.child = None;
                        return Err(format!("overlay host exited during startup with {status}"));
                    }
                    Err(error) => {
                        return Err(format!(
                            "failed to inspect overlay host during startup: {error}"
                        ));
                    }
                }
            }

            match UnixStream::connect(&self.socket_path) {
                Ok(_) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
            std::thread::sleep(HOST_CONNECT_INTERVAL);
        }

        Err(format!(
            "overlay host socket did not become ready at {}{}",
            self.socket_path.display(),
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        ))
    }

    fn send_once(&self, message: &OverlayHostMessage) -> Result<OverlayHostReply, String> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|error| {
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

    fn reset_child(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

#[cfg(unix)]
impl Drop for ProcessOverlayHostClient {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = self.send_once(&OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            reason: None,
        });
        let deadline = Instant::now() + HOST_STOP_TIMEOUT;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                let _ = fs::remove_file(&self.socket_path);
                return;
            }
            std::thread::sleep(HOST_CONNECT_INTERVAL);
        }
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

fn overlay_backend_disabled() -> bool {
    matches!(
        std::env::var(OVERLAY_BACKEND_ENV)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "none" | "never" | "off" | "false" | "0"
    )
}

#[cfg(unix)]
fn overlay_socket_path(service_socket_path: &Path) -> Option<PathBuf> {
    service_socket_path.parent().map(|parent| {
        if parent.as_os_str().is_empty() {
            PathBuf::from("agent-cursor.sock")
        } else {
            parent.join("agent-cursor.sock")
        }
    })
}

#[cfg(unix)]
fn overlay_host_path() -> PathBuf {
    if let Some(path) = std::env::var_os(OVERLAY_HOST_PATH_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(sibling) = exe_path
            .parent()
            .map(|parent| parent.join(overlay_host_binary_name()))
        && sibling.is_file()
    {
        return sibling;
    }
    let repo_root = std::env::var_os("SKY_CUA_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    repo_root.join("bin").join(overlay_host_binary_name())
}

#[cfg(unix)]
fn overlay_host_binary_name() -> &'static str {
    "sky-cua-overlay-host"
}

fn client_error_detail(error: &str) -> String {
    if error.starts_with("overlay host binary not found:") {
        "overlay host binary not found".to_string()
    } else if error.starts_with("failed to spawn overlay host ") {
        "failed to spawn overlay host".to_string()
    } else if error.starts_with("overlay host socket did not become ready at ") {
        "overlay host socket did not become ready".to_string()
    } else if error.starts_with("failed to connect to overlay host socket ") {
        "failed to connect to overlay host socket".to_string()
    } else {
        error.to_string()
    }
}

fn diagnostic(code: &str, message: &str, details: Option<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::OverlayHostConnection;
    use sky_cua_overlay_host::{
        OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind,
    };

    #[test]
    fn disabled_host_without_reporting_acknowledges_request() {
        let mut host = OverlayHostConnection::disabled_for_tests();

        let reply = host
            .send(capabilities_message())
            .expect("disabled test host ack");

        assert_eq!(reply.version, OVERLAY_HOST_PROTOCOL_VERSION);
        assert!(reply.ok);
        assert!(reply.diagnostics.is_empty());
    }

    #[test]
    fn disabled_host_with_reporting_returns_unavailable_diagnostic() {
        let mut host = OverlayHostConnection::Disabled {
            reason: "missing host".to_string(),
            report_diagnostic: true,
        };

        let diagnostic = host
            .send(capabilities_message())
            .expect_err("reported disabled host should be diagnostic");

        assert_eq!(diagnostic.code, "AgentCursorHostUnavailable");
        assert_eq!(diagnostic.details.as_deref(), Some("missing host"));
    }

    #[cfg(unix)]
    #[test]
    fn process_host_unavailable_diagnostic_omits_local_binary_path() {
        let dir = unique_temp_dir("host-path-redaction");
        let host_path = dir.join("missing-overlay-host");
        let socket_path = dir.join("agent-cursor.sock");
        let mut host = OverlayHostConnection::process_for_tests(host_path.clone(), socket_path);

        let diagnostic = host
            .send(capabilities_message())
            .expect_err("missing host should be diagnostic");

        assert_eq!(diagnostic.code, "AgentCursorHostUnavailable");
        assert_eq!(
            diagnostic.details.as_deref(),
            Some("overlay host binary not found")
        );
        assert_eq!(host.default_reason(), "overlay host binary not found");
        assert!(
            !diagnostic
                .details
                .as_deref()
                .unwrap_or_default()
                .contains(&host_path.display().to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_host_request_failure_resets_child_for_respawn() {
        let dir = unique_temp_dir("host-bad-reply-reset");
        let host_path = dir.join("bad-overlay-host");
        let socket_path = dir.join("agent-cursor.sock");
        write_bad_reply_overlay_host(&host_path);
        let mut host = OverlayHostConnection::process_for_tests(host_path, socket_path);

        let diagnostic = host
            .send(capabilities_message())
            .expect_err("bad host reply should be diagnostic");

        assert_eq!(diagnostic.code, "AgentCursorHostRequestFailed");
        let OverlayHostConnection::Process(client) = host else {
            panic!("expected process host");
        };
        assert!(client.child.is_none());
    }

    fn capabilities_message() -> OverlayHostMessage {
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Capabilities,
            state: None,
            reason: None,
        }
    }

    #[cfg(unix)]
    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-agent-cursor-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[cfg(unix)]
    fn write_bad_reply_overlay_host(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(
            path,
            r#"#!/usr/bin/env python3
import os
import socket
import sys

if len(sys.argv) != 4 or sys.argv[1:3] != ["serve", "--socket"]:
    raise SystemExit(f"unexpected argv: {sys.argv!r}")

socket_path = sys.argv[3]
try:
    os.unlink(socket_path)
except FileNotFoundError:
    pass

server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(socket_path)
server.listen(8)
while True:
    conn, _ = server.accept()
    with conn:
        data = conn.recv(4096)
        if data.strip():
            conn.sendall(b"not-json\n")
"#,
        )
        .expect("write bad overlay host");
        let mut permissions = std::fs::metadata(path)
            .expect("bad host metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).expect("chmod bad overlay host");
    }
}
