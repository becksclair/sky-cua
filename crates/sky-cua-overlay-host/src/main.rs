use std::io::{self, BufRead, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
#[cfg(unix)]
use calloop::{
    EventLoop, Interest, LoopSignal, Mode, PostAction,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};
use sky_cua_overlay_host::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostBackend, OverlayHostMessage, OverlayHostMessageKind,
    probe_environment_reply,
};
use sky_cua_platform::model::{AgentCursorPoint, AgentCursorState, CoordinateSpace};

#[cfg(not(test))]
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const CLIENT_IO_TIMEOUT: Duration = Duration::from_millis(100);

// Initial loop cadence before the first display-refresh-matched interval is
// computed; subsequent ticks use OverlayHostBackend::pointer_tick_interval().
#[cfg(unix)]
const SOCKET_LOOP_TICK_INTERVAL: Duration = Duration::from_millis(16);

fn main() -> Result<()> {
    // The overlay host is a long-lived child of the service daemon; drop any
    // descriptors leaked down the launcher chain before serving.
    sky_cua_platform::fd_hygiene::close_inherited_fds();
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "serve".to_string());
    match command.as_str() {
        "probe" => print_reply(&probe_environment_reply()),
        "set-cursor" => set_cursor_from_args(std::env::args().skip(2).collect()),
        "serve" => serve_from_args(std::env::args().skip(2).collect()),
        "playground" => sky_cua_overlay_host::run_playground(std::env::args().skip(2).collect()),
        other => anyhow::bail!("unsupported sky-cua-overlay-host mode: {other}"),
    }
}

fn set_cursor_from_args(args: Vec<String>) -> Result<()> {
    let [x, y] = args.as_slice() else {
        anyhow::bail!("usage: sky-cua-overlay-host set-cursor <x> <y>");
    };
    let x = x.parse::<f64>().context("invalid x coordinate")?;
    let y = y.parse::<f64>().context("invalid y coordinate")?;
    let mut backend = OverlayHostBackend::from_env();
    let reply = backend.handle_message(OverlayHostMessage {
        version: OVERLAY_HOST_PROTOCOL_VERSION,
        kind: OverlayHostMessageKind::SetCursor,
        state: Some(AgentCursorState {
            visible: true,
            sequence: 0,
            model_point: Some(AgentCursorPoint {
                x,
                y,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: None,
            }),
            native_point: None,
            snapshot_id: None,
            source_action: None,
            updated_at_ms: 0,
        }),
        gesture: None,
        sequence: None,
        reason: None,
        arrival_wait: None,
    });
    print_reply(&reply)
}

fn serve_from_args(args: Vec<String>) -> Result<()> {
    match OverlayHostServeMode::from_args(args)? {
        OverlayHostServeMode::JsonLines => serve_json_lines(),
        OverlayHostServeMode::UnixSocket(path) => serve_unix_socket(path),
        OverlayHostServeMode::Tcp(addr) => serve_tcp(addr),
    }
}

enum OverlayHostServeMode {
    JsonLines,
    UnixSocket(PathBuf),
    Tcp(String),
}

impl OverlayHostServeMode {
    fn from_args(args: Vec<String>) -> Result<Self> {
        match args.as_slice() {
            [] => Ok(Self::JsonLines),
            [flag, path] if flag == "--socket" => Ok(Self::UnixSocket(PathBuf::from(path))),
            [flag, addr] if flag == "--tcp" => Ok(Self::Tcp(addr.to_string())),
            _ => {
                anyhow::bail!("usage: sky-cua-overlay-host serve [--socket <path> | --tcp <addr>]")
            }
        }
    }
}

#[cfg(test)]
mod serve_mode_tests {
    use super::OverlayHostServeMode;

    #[test]
    fn serve_mode_parses_json_lines_and_socket_modes() {
        assert!(matches!(
            OverlayHostServeMode::from_args(Vec::new()).expect("json-lines mode"),
            OverlayHostServeMode::JsonLines
        ));
        match OverlayHostServeMode::from_args(vec![
            "--socket".to_string(),
            "/tmp/agent-cursor.sock".to_string(),
        ])
        .expect("socket mode")
        {
            OverlayHostServeMode::UnixSocket(path) => {
                assert_eq!(path, std::path::PathBuf::from("/tmp/agent-cursor.sock"));
            }
            OverlayHostServeMode::JsonLines | OverlayHostServeMode::Tcp(_) => {
                panic!("expected socket mode")
            }
        }
        match OverlayHostServeMode::from_args(vec![
            "--tcp".to_string(),
            "127.0.0.1:48932".to_string(),
        ])
        .expect("tcp mode")
        {
            OverlayHostServeMode::Tcp(addr) => assert_eq!(addr, "127.0.0.1:48932"),
            OverlayHostServeMode::JsonLines | OverlayHostServeMode::UnixSocket(_) => {
                panic!("expected tcp mode")
            }
        }
        assert!(OverlayHostServeMode::from_args(vec!["--pipe".to_string()]).is_err());
    }
}

fn serve_json_lines() -> Result<()> {
    let mut stdout = io::stdout().lock();
    let mut backend = OverlayHostBackend::from_env();

    // Stdin is drained on a reader thread so the serve loop can keep
    // frame-pacing the overlay between requests, like the socket loops do.
    // Without the idle ticks the vehicle-steered cursor would integrate one
    // dt slice per message and freeze mid-glide, and arrival-gated gesture
    // feedback would never fire on this transport.
    let (line_tx, line_rx) = std::sync::mpsc::channel::<io::Result<String>>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let disconnected = line_tx.send(line).is_err();
            if disconnected {
                return;
            }
        }
    });

    loop {
        let line = match line_rx.recv_timeout(backend.pointer_tick_interval()) {
            Ok(line) => line.context("failed to read overlay host request")?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                backend.tick();
                continue;
            }
            // Stdin reached EOF: the parent is gone, serve is done.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        if line.trim().is_empty() {
            continue;
        }
        let message: OverlayHostMessage =
            serde_json::from_str(&line).context("invalid overlay host request JSON")?;
        let shutdown = message.kind == OverlayHostMessageKind::Shutdown;
        let reply = backend.handle_message(message);
        serde_json::to_writer(&mut stdout, &reply).context("failed to write reply JSON")?;
        stdout
            .write_all(b"\n")
            .context("failed to write reply newline")?;
        stdout.flush().context("failed to flush reply")?;
        if shutdown {
            return Ok(());
        }
    }
}

fn serve_tcp(addr: String) -> Result<()> {
    serve_tcp_with(addr)
}

fn serve_tcp_with(addr: String) -> Result<()> {
    let listener = TcpListener::bind(&addr)
        .with_context(|| format!("failed to bind overlay host TCP listener {addr}"))?;
    listener
        .set_nonblocking(true)
        .context("failed to make overlay host TCP listener non-blocking")?;
    run_accept_loop(|| listener.accept().map(|(stream, _)| stream), "TCP")
}

#[cfg(unix)]
fn serve_unix_socket(path: PathBuf) -> Result<()> {
    serve_unix_socket_with(path)
}

#[cfg(unix)]
fn serve_unix_socket_with(path: PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create overlay host socket directory {}",
                parent.display()
            )
        })?;
    }
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| {
            format!(
                "failed to remove stale overlay host socket {}",
                path.display()
            )
        })?;
    }
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("failed to bind overlay host socket {}", path.display()))?;
    listener
        .set_nonblocking(true)
        .context("failed to make overlay host socket listener non-blocking")?;
    let result = run_unix_socket_event_loop(listener);
    let _ = std::fs::remove_file(&path);
    result
}

#[cfg(unix)]
fn run_unix_socket_event_loop(listener: UnixListener) -> Result<()> {
    let mut event_loop: EventLoop<UnixSocketLoopState> =
        EventLoop::try_new().context("failed to create overlay host socket event loop")?;
    let signal = event_loop.get_signal();
    let handle = event_loop.handle();
    let listener_source = Generic::new(
        listener
            .try_clone()
            .context("failed to clone overlay host socket listener for event loop")?,
        Interest::READ,
        Mode::Level,
    );
    handle
        .insert_source(listener_source, |_readiness, _listener, state| {
            state.accept_ready();
            Ok(PostAction::Continue)
        })
        .context("failed to register overlay host socket listener")?;
    handle
        .insert_source(
            Timer::from_duration(SOCKET_LOOP_TICK_INTERVAL),
            |_deadline, _timer, state| {
                state.backend.tick();
                TimeoutAction::ToDuration(state.backend.pointer_tick_interval())
            },
        )
        .map_err(|error| {
            anyhow::anyhow!("failed to register overlay host socket timer: {error}")
        })?;

    let mut state = UnixSocketLoopState {
        listener,
        backend: OverlayHostBackend::from_env(),
        signal,
        label: "socket",
    };
    event_loop
        .run(None::<Duration>, &mut state, |_state| {})
        .context("overlay host socket event loop failed")
}

#[cfg(unix)]
struct UnixSocketLoopState {
    listener: UnixListener,
    backend: OverlayHostBackend,
    signal: LoopSignal,
    label: &'static str,
}

#[cfg(unix)]
impl UnixSocketLoopState {
    fn accept_ready(&mut self) {
        loop {
            let mut stream = match self.listener.accept() {
                Ok((stream, _addr)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                Err(error) => {
                    eprintln!(
                        "failed to accept overlay host {} connection: {error}",
                        self.label
                    );
                    return;
                }
            };
            if let Err(error) = configure_client_stream(&stream, self.label) {
                eprintln!(
                    "overlay host {} connection setup failed: {error:#}",
                    self.label
                );
                continue;
            }
            match handle_socket_message(&mut self.backend, &mut stream) {
                Ok(shutdown) => {
                    if shutdown {
                        self.signal.stop();
                        return;
                    }
                }
                Err(error) => {
                    eprintln!("overlay host {} connection failed: {error:#}", self.label);
                }
            }
        }
    }
}

/// Shared serve loop for socket-style endpoints: poll a non-blocking listener
/// for clients, switch each accepted stream to blocking mode with per-client
/// I/O timeouts, handle one request per connection, advance frame-paced
/// overlay work while idle, and exit on a shutdown message.
///
/// Clients are handled serially: a connected client that never sends a
/// request can delay subsequent requests by at most `CLIENT_IO_TIMEOUT`.
/// That bound is acceptable for the supported single service-client model.
fn run_accept_loop<S: ClientStream>(
    mut accept: impl FnMut() -> io::Result<S>,
    label: &str,
) -> Result<()> {
    let mut backend = OverlayHostBackend::from_env();
    loop {
        let mut stream = match accept() {
            Ok(stream) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                backend.tick();
                std::thread::sleep(backend.pointer_tick_interval());
                continue;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to accept overlay host {label} connection"));
            }
        };
        configure_client_stream(&stream, label)?;
        match handle_socket_message(&mut backend, &mut stream) {
            Ok(shutdown) => {
                if shutdown {
                    return Ok(());
                }
            }
            Err(error) => {
                eprintln!("overlay host {label} connection failed: {error:#}");
            }
        }
    }
}

fn configure_client_stream(stream: &impl ClientStream, label: &str) -> Result<()> {
    stream
        .set_nonblocking(false)
        .with_context(|| format!("failed to make overlay host {label} stream blocking"))?;
    stream
        .set_read_timeout(Some(CLIENT_IO_TIMEOUT))
        .with_context(|| format!("failed to set overlay host {label} read timeout"))?;
    stream
        .set_write_timeout(Some(CLIENT_IO_TIMEOUT))
        .with_context(|| format!("failed to set overlay host {label} write timeout"))?;
    Ok(())
}

/// Stream configuration shared by the accepted client types.
trait ClientStream: io::Read + io::Write {
    fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()>;
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
}

impl ClientStream for std::net::TcpStream {
    fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        std::net::TcpStream::set_nonblocking(self, nonblocking)
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        std::net::TcpStream::set_read_timeout(self, timeout)
    }
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        std::net::TcpStream::set_write_timeout(self, timeout)
    }
}

#[cfg(unix)]
impl ClientStream for std::os::unix::net::UnixStream {
    fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        std::os::unix::net::UnixStream::set_nonblocking(self, nonblocking)
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        std::os::unix::net::UnixStream::set_read_timeout(self, timeout)
    }
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        std::os::unix::net::UnixStream::set_write_timeout(self, timeout)
    }
}

#[cfg(not(unix))]
fn serve_unix_socket(_path: PathBuf) -> Result<()> {
    anyhow::bail!("overlay host socket mode is not implemented on this platform yet")
}

fn handle_socket_message(
    backend: &mut OverlayHostBackend,
    stream: &mut (impl io::Read + io::Write),
) -> Result<bool> {
    let mut reader = io::BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("failed to read overlay host socket request")?;
    if line.trim().is_empty() {
        return Ok(false);
    }
    let message: OverlayHostMessage =
        serde_json::from_str(line.trim_end()).context("invalid overlay host request JSON")?;
    let shutdown = message.kind == OverlayHostMessageKind::Shutdown;
    let reply = backend.handle_message(message);
    let stream = reader.get_mut();
    serde_json::to_writer(&mut *stream, &reply).context("failed to write reply JSON")?;
    stream
        .write_all(b"\n")
        .context("failed to write reply newline")?;
    stream.flush().context("failed to flush reply")?;
    Ok(shutdown)
}

#[cfg(all(test, unix))]
mod tests {
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

    use super::{CLIENT_IO_TIMEOUT, serve_tcp, serve_unix_socket, serve_unix_socket_with};

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
        let mut stream = UnixStream::connect(path).expect("connect overlay host socket");
        let payload = serde_json::to_vec(&message).expect("serialize request");
        stream.write_all(&payload).expect("write payload");
        stream.write_all(b"\n").expect("write newline");
        stream.flush().expect("flush payload");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read reply");
        serde_json::from_str(line.trim_end()).expect("parse reply")
    }

    fn send_tcp(addr: &str, message: OverlayHostMessage) -> OverlayHostReply {
        let mut stream = TcpStream::connect(addr).expect("connect overlay host TCP listener");
        let payload = serde_json::to_vec(&message).expect("serialize request");
        stream.write_all(&payload).expect("write payload");
        stream.write_all(b"\n").expect("write newline");
        stream.flush().expect("flush payload");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read reply");
        serde_json::from_str(line.trim_end()).expect("parse reply")
    }

    fn wait_for_socket(path: &Path) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2) {
            if path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("socket did not appear: {}", path.display());
    }

    fn wait_for_tcp(addr: &str) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2) {
            if TcpStream::connect(addr).is_ok() {
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
        let dir = std::env::temp_dir().join(format!(
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
}

fn print_reply(reply: &sky_cua_overlay_host::OverlayHostReply) -> Result<()> {
    println!("{}", serde_json::to_string(reply)?);
    Ok(())
}
