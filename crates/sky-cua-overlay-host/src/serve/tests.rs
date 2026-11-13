//! Transport tests for the serve loops, pinned to the no-op backend.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sky_cua_overlay_host::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
};
use sky_cua_platform::model::{AgentCursorPoint, AgentCursorState, CoordinateSpace};

use super::{CLIENT_IO_TIMEOUT, ParentWatch, serve_tcp, serve_unix_socket, serve_unix_socket_with};

#[test]
fn parent_watch_reports_an_orphan_only_when_the_parent_process_changed() {
    let watch = ParentWatch::new();
    assert!(!watch.orphaned());
    // A host started by a process that has since exited sees a different
    // parent (the init process) than the one it recorded.
    let started_by_another_process = ParentWatch {
        parent: watch.parent.wrapping_add(1),
    };
    assert!(started_by_another_process.orphaned());
}

/// Pin the serve loops to the noop backend: these are transport tests and
/// must never attach to a live desktop backend (the auto-selected KWin
/// effect on a Plasma host with the sky-cua effect installed, for
/// example). Safe to set from parallel tests because the value is
/// identical everywhere.
fn pin_noop_overlay_backend() {
    unsafe { std::env::set_var("SKY_CUA_OVERLAY_BACKEND", "noop") };
}

#[test]
fn unix_socket_serve_keeps_overlay_visible_until_service_hides_it() {
    pin_noop_overlay_backend();
    let dir = unique_temp_dir("socket-no-idle-hide");
    let socket_path = dir.join("agent-cursor.sock");
    let server_path = socket_path.clone();
    let server = thread::spawn(move || serve_unix_socket_with(server_path));
    wait_for_socket(&socket_path);

    let set = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(set.state.as_ref().expect("state").visible);

    // The overlay host is no longer an independent lifecycle owner. It
    // keeps the surface visible until the service's idle/session cleanup
    // sends an explicit hide.
    thread::sleep(Duration::from_millis(200));

    let probe = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Capabilities,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(
        probe.state.as_ref().expect("state").visible,
        "overlay host should not auto-hide without a service hide request"
    );

    let shutdown = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(shutdown.ok);
    server
        .join()
        .expect("server thread join")
        .expect("server result");
}

#[test]
fn unix_socket_serve_round_trips_json_lines() {
    pin_noop_overlay_backend();
    let dir = unique_temp_dir("socket-serve");
    let socket_path = dir.join("agent-cursor.sock");
    let server_path = socket_path.clone();
    let server = thread::spawn(move || serve_unix_socket(server_path));
    wait_for_socket(&socket_path);

    let set = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(set.ok);
    assert!(set.state.as_ref().expect("state").visible);

    let hidden = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Hide,
            state: None,
            gesture: None,
            sequence: None,
            reason: Some("capture".to_string()),
            arrival_wait: None,
        },
    );
    assert!(!hidden.state.as_ref().expect("state").visible);
    assert!(
        hidden
            .diagnostics
            .iter()
            .any(|entry| entry.code == "OverlayCursorHidden")
    );

    let shutdown = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(shutdown.ok);
    server
        .join()
        .expect("join overlay host socket server")
        .expect("server should exit cleanly");
    assert!(!socket_path.exists());
}

#[test]
fn tcp_serve_round_trips_json_lines() {
    pin_noop_overlay_backend();
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
    let addr = listener.local_addr().expect("read tcp addr").to_string();
    drop(listener);
    let server_addr = addr.clone();
    let server = thread::spawn(move || serve_tcp(server_addr));
    wait_for_tcp(&addr);

    let set = send_tcp(
        &addr,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(set.ok);
    assert!(set.state.as_ref().expect("state").visible);

    let shutdown = send_tcp(
        &addr,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(shutdown.ok);
    server
        .join()
        .expect("join overlay host TCP server")
        .expect("server should exit cleanly");
}

/// The service's readiness probe connects and disconnects at once. Darwin
/// then rejects the timeout setup on the accepted stream with `EINVAL`,
/// which must cost that one connection, never the host.
#[test]
fn unix_socket_serve_survives_a_client_that_disconnects_before_setup() {
    pin_noop_overlay_backend();
    let dir = unique_temp_dir("socket-probe");
    let socket_path = dir.join("agent-cursor.sock");
    let server_path = socket_path.clone();
    let server = thread::spawn(move || serve_unix_socket(server_path));
    wait_for_socket(&socket_path);

    drop(UnixStream::connect(&socket_path).expect("connect readiness probe"));
    // Let the host accept the already-closed connection.
    thread::sleep(Duration::from_millis(80));

    let shutdown = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(shutdown.ok);
    server
        .join()
        .expect("join overlay host socket server")
        .expect("server should exit cleanly");
}

#[test]
fn unix_socket_serve_drops_idle_client_and_serves_next_request() {
    pin_noop_overlay_backend();
    let dir = unique_temp_dir("socket-idle-client");
    let socket_path = dir.join("agent-cursor.sock");
    let server_path = socket_path.clone();
    let server = thread::spawn(move || serve_unix_socket(server_path));
    wait_for_socket(&socket_path);

    let idle = UnixStream::connect(&socket_path).expect("connect idle socket client");
    thread::sleep(CLIENT_IO_TIMEOUT + Duration::from_millis(50));

    let set = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(set.ok);
    drop(idle);

    let shutdown = send(
        &socket_path,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(shutdown.ok);
    server
        .join()
        .expect("join overlay host socket server")
        .expect("server should exit cleanly");
}

#[test]
fn tcp_serve_drops_idle_client_and_serves_next_request() {
    pin_noop_overlay_backend();
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
    let addr = listener.local_addr().expect("read tcp addr").to_string();
    drop(listener);
    let server_addr = addr.clone();
    let server = thread::spawn(move || serve_tcp(server_addr));
    wait_for_tcp(&addr);

    let idle = TcpStream::connect(&addr).expect("connect idle client");
    thread::sleep(CLIENT_IO_TIMEOUT + Duration::from_millis(50));

    let set = send_tcp(
        &addr,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::SetCursor,
            state: Some(cursor_state()),
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(set.ok);
    drop(idle);

    let shutdown = send_tcp(
        &addr,
        OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            gesture: None,
            sequence: None,
            reason: None,
            arrival_wait: None,
        },
    );
    assert!(shutdown.ok);
    server
        .join()
        .expect("join overlay host TCP server")
        .expect("server should exit cleanly");
}

fn send(path: &Path, message: OverlayHostMessage) -> OverlayHostReply {
    let payload = serde_json::to_vec(&message).expect("serialize request");
    let mut last_err = None;
    for attempt in 0..8 {
        match UnixStream::connect(path) {
            Ok(mut stream) => {
                stream
                    .set_write_timeout(Some(CLIENT_IO_TIMEOUT + Duration::from_millis(200)))
                    .ok();
                stream
                    .set_read_timeout(Some(CLIENT_IO_TIMEOUT + Duration::from_millis(200)))
                    .ok();
                if stream.write_all(&payload).is_err()
                    || stream.write_all(b"\n").is_err()
                    || stream.flush().is_err()
                {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        last_err = Some("empty reply".to_string());
                    }
                    Ok(_) => {
                        if let Ok(reply) = serde_json::from_str::<OverlayHostReply>(line.trim_end())
                        {
                            return reply;
                        }
                        last_err = Some(format!("bad json: {line}"));
                    }
                    Err(e) => {
                        last_err = Some(format!("read reply: {e}"));
                    }
                }
            }
            Err(e) => {
                last_err = Some(format!("connect: {e}"));
            }
        }
        if attempt < 7 {
            thread::sleep(Duration::from_millis(20));
        }
    }
    panic!(
        "connect/read overlay host socket failed after retries: {}",
        last_err.unwrap_or_default()
    );
}

fn send_tcp(addr: &str, message: OverlayHostMessage) -> OverlayHostReply {
    let payload = serde_json::to_vec(&message).expect("serialize request");
    let mut last_err = None;
    for attempt in 0..8 {
        match TcpStream::connect(addr) {
            Ok(mut stream) => {
                stream
                    .set_write_timeout(Some(CLIENT_IO_TIMEOUT + Duration::from_millis(200)))
                    .ok();
                stream
                    .set_read_timeout(Some(CLIENT_IO_TIMEOUT + Duration::from_millis(200)))
                    .ok();
                if stream.write_all(&payload).is_err()
                    || stream.write_all(b"\n").is_err()
                    || stream.flush().is_err()
                {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        last_err = Some("empty reply".to_string());
                    }
                    Ok(_) => {
                        if let Ok(reply) = serde_json::from_str::<OverlayHostReply>(line.trim_end())
                        {
                            return reply;
                        }
                        last_err = Some(format!("bad json: {line}"));
                    }
                    Err(e) => {
                        last_err = Some(format!("read reply: {e}"));
                    }
                }
            }
            Err(e) => {
                last_err = Some(format!("connect: {e}"));
            }
        }
        if attempt < 7 {
            thread::sleep(Duration::from_millis(20));
        }
    }
    panic!(
        "connect/read overlay host TCP failed after retries: {}",
        last_err.unwrap_or_default()
    );
}

fn wait_for_socket(path: &Path) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if path.exists() {
            // Give the event loop a moment to enter accept(); the file can
            // appear before the listener is ready (previous ConnectionRefused
            // flake). No probe connect here — each probe would create an
            // empty connection the server must drain (would block the serial
            // accept loop and overload parallel nextest).
            thread::sleep(Duration::from_millis(20));
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("socket did not appear: {}", path.display());
}

fn wait_for_tcp(addr: &str) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        // Single probe at the end — the TcpListener::bind in the test
        // reserves the port before the server binds, so early probes create
        // empty connections that block the serial accept loop for
        // CLIENT_IO_TIMEOUT each (100ms × 200 probes = 20s). Just wait.
        thread::sleep(Duration::from_millis(20));
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        if started.elapsed() >= Duration::from_millis(100) {
            // After a short grace, the server should be listening; stop
            // probing and let `send` retry on ConnectionRefused instead.
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("TCP listener did not become ready: {addr}");
}

fn cursor_state() -> AgentCursorState {
    AgentCursorState {
        visible: true,
        sequence: 1,
        model_point: Some(AgentCursorPoint {
            x: 10.0,
            y: 20.0,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: None,
        }),
        native_point: None,
        snapshot_id: None,
        source_action: None,
        updated_at_ms: 42,
    }
}

fn unique_temp_dir(name: &str) -> PathBuf {
    // macOS's per-user `$TMPDIR` is about 50 bytes deep, which leaves too
    // little of the 104-byte `sun_path` for these socket fixtures.
    let root = if cfg!(target_os = "macos") {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = root.join(format!(
        "sky-cua-overlay-host-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time after unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}
