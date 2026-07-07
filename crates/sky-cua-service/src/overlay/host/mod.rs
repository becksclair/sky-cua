mod lifecycle;

use std::path::Path;
use std::path::PathBuf;

#[cfg(not(unix))]
use lifecycle::TcpEndpoint;
#[cfg(unix)]
use lifecycle::UnixSocketEndpoint;
use lifecycle::{ManagedOverlayHost, diagnostic};
use sky_cua_overlay_host::{OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostReply};
use sky_cua_platform::model::DiagnosticEntry;
#[cfg(not(unix))]
use sky_cua_platform::overlay_host_tcp_addr;

use sky_cua_platform::config::OVERLAY_BACKEND_ENV;
const OVERLAY_HOST_PATH_ENV: &str = "SKY_CUA_OVERLAY_HOST_PATH";

/// Endpoint the production connection uses on this platform.
#[cfg(unix)]
type DefaultEndpoint = UnixSocketEndpoint;
#[cfg(not(unix))]
type DefaultEndpoint = TcpEndpoint;

#[derive(Debug)]
pub(super) enum OverlayHostConnection {
    Disabled {
        reason: String,
        report_diagnostic: bool,
    },
    #[cfg(test)]
    Failing {
        diagnostic: DiagnosticEntry,
    },
    Transport(ManagedOverlayHost<DefaultEndpoint>),
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
            Self::Transport(ManagedOverlayHost::new(
                overlay_host_path(),
                UnixSocketEndpoint::new(socket_path),
            ))
        }

        #[cfg(not(unix))]
        {
            let _ = service_socket_path;
            Self::Transport(ManagedOverlayHost::new(
                overlay_host_path(),
                TcpEndpoint::new(overlay_host_tcp_addr()),
            ))
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
    pub(super) fn unix_socket_transport_for_tests(
        host_path: PathBuf,
        socket_path: PathBuf,
    ) -> Self {
        Self::Transport(ManagedOverlayHost::new(
            host_path,
            UnixSocketEndpoint::new(socket_path),
        ))
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
                        motion: None,
                        version: OVERLAY_HOST_PROTOCOL_VERSION,
                        ok: true,
                        capabilities: None,
                        lifecycle_state: None,
                        applied_sequence: None,
                        state: None,
                        diagnostics: Vec::new(),
                    })
                }
            }
            #[cfg(test)]
            Self::Failing { diagnostic } => Err(diagnostic.clone()),
            Self::Transport(transport) => transport.send(message),
        }
    }

    pub(super) fn default_reason(&self) -> String {
        match self {
            Self::Disabled { reason, .. } => reason.clone(),
            #[cfg(test)]
            Self::Failing { diagnostic } => diagnostic.message.clone(),
            Self::Transport(transport) => transport.default_reason(),
        }
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

fn overlay_host_binary_name() -> &'static str {
    if cfg!(windows) {
        "sky-cua-overlay-host.exe"
    } else {
        "sky-cua-overlay-host"
    }
}

#[cfg(test)]
mod tests {
    use super::OverlayHostConnection;
    use super::lifecycle::{ManagedOverlayHost, TcpEndpoint};
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
    fn transport_host_unavailable_diagnostic_omits_local_binary_path() {
        let dir = unique_temp_dir("host-path-redaction");
        let host_path = dir.join("missing-overlay-host");
        let socket_path = dir.join("agent-cursor.sock");
        let mut host =
            OverlayHostConnection::unix_socket_transport_for_tests(host_path.clone(), socket_path);

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
    fn transport_host_request_failure_resets_child_for_respawn() {
        let dir = unique_temp_dir("host-bad-reply-reset");
        let host_path = dir.join("bad-overlay-host");
        let socket_path = dir.join("agent-cursor.sock");
        write_bad_reply_overlay_host(&host_path, unix_socket_bad_reply_server());
        let mut host =
            OverlayHostConnection::unix_socket_transport_for_tests(host_path, socket_path);

        let diagnostic = host
            .send(capabilities_message())
            .expect_err("bad host reply should be diagnostic");

        assert_eq!(diagnostic.code, "AgentCursorHostRequestFailed");
        let OverlayHostConnection::Transport(transport) = host else {
            panic!("expected transport host");
        };
        assert!(transport.child.is_none());
    }

    #[test]
    fn tcp_transport_unavailable_diagnostic_omits_local_binary_path() {
        let dir = unique_temp_dir("tcp-host-path-redaction");
        let host_path = dir.join("missing-overlay-host");
        let mut host = ManagedOverlayHost::new(
            host_path.clone(),
            TcpEndpoint::new("127.0.0.1:0".to_string()),
        );

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
    fn tcp_transport_request_failure_resets_child_for_respawn() {
        let dir = unique_temp_dir("tcp-host-bad-reply-reset");
        let host_path = dir.join("bad-overlay-host");
        let (port_reservation, port) = reserve_local_port();
        write_bad_reply_overlay_host(&host_path, tcp_bad_reply_server());
        let mut host =
            ManagedOverlayHost::new(host_path, TcpEndpoint::new(format!("127.0.0.1:{port}")));

        // Hold the reservation through setup and release it only now, immediately
        // before send() spawns the child that rebinds the port, so nothing can
        // claim it during test setup. The child's SO_REUSEADDR retry covers the
        // unavoidable handoff window; nextest serializes overlay tests so no
        // sibling test competes for the port.
        drop(port_reservation);
        let diagnostic = host
            .send(capabilities_message())
            .expect_err("bad host reply should be diagnostic");

        assert_eq!(diagnostic.code, "AgentCursorHostRequestFailed");
        assert!(host.child.is_none());
    }

    fn capabilities_message() -> OverlayHostMessage {
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Capabilities,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
        }
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-agent-cursor-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// Reserve an ephemeral local TCP port, returning the bound listener so the
    /// caller holds the reservation until the moment the child rebinds it.
    /// Dropping the listener frees the port; keep it alive through test setup and
    /// drop it immediately before spawning the child to keep the rebind window as
    /// small as possible.
    #[cfg(unix)]
    fn reserve_local_port() -> (std::net::TcpListener, u16) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
        let port = listener.local_addr().expect("read tcp addr").port();
        (listener, port)
    }

    #[cfg(unix)]
    fn unix_socket_bad_reply_server() -> &'static str {
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
"#
    }

    #[cfg(unix)]
    fn tcp_bad_reply_server() -> &'static str {
        r#"#!/usr/bin/env python3
import socket
import sys
import time

if len(sys.argv) != 4 or sys.argv[1:3] != ["serve", "--tcp"]:
    raise SystemExit(f"unexpected argv: {sys.argv!r}")

host, _, port = sys.argv[3].rpartition(":")
server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
# The harness reserves the port and releases it before this child runs, so under
# load a concurrent test can transiently hold the same OS-reused port. The hold is
# momentary, so retry the bind briefly (~1s, well inside the host start timeout)
# rather than exiting, which the service would misread as the host being
# unavailable.
for _ in range(20):
    try:
        server.bind((host, int(port)))
        break
    except OSError:
        time.sleep(0.05)
else:
    server.bind((host, int(port)))
server.listen(8)
while True:
    conn, _ = server.accept()
    with conn:
        data = conn.recv(4096)
        if data.strip():
            conn.sendall(b"not-json\n")
"#
    }

    #[cfg(unix)]
    fn write_bad_reply_overlay_host(path: &std::path::Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, script).expect("write bad overlay host");
        let mut permissions = std::fs::metadata(path)
            .expect("bad host metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).expect("chmod bad overlay host");
    }
}
