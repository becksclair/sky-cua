//! The `serve` command: the JSON-lines request loop over stdin/stdout, a
//! Unix socket, or a local TCP port. One request and one reply per
//! connection; between requests the loop ticks the backend so the overlay
//! keeps animating.
//!
//! The socket loop is platform-specific where the platform has an event loop
//! to integrate with (calloop beside the Wayland queue on Linux, the main run
//! loop on macOS); everything else uses the shared polling accept loop.

use std::io::{self, BufRead, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use sky_cua_overlay_host::{OverlayHostBackend, OverlayHostMessage, OverlayHostMessageKind};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(test, unix))]
mod tests;

#[cfg(target_os = "linux")]
use linux::run_unix_socket_event_loop;
#[cfg(target_os = "macos")]
use macos::run_unix_socket_event_loop;

#[cfg(not(test))]
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const CLIENT_IO_TIMEOUT: Duration = Duration::from_millis(500);

// Upper bound on the TCP accept loop's poll sleep; see `run_accept_loop`.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(16);

pub(crate) fn serve_from_args(args: Vec<String>) -> Result<()> {
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

/// Detects that the process which started this host has exited.
///
/// The service owns the overlay's lifecycle: it hides the overlay when a
/// session ends or goes idle, and shuts the host down when it stops. A service
/// that is killed can do neither, and would leave an orphaned host drawing a
/// visible overlay with nothing left to hide it. The kernel re-parents the
/// children of a process that exits, so a changed parent process id means the
/// owner is gone.
///
/// Attaching a debugger also re-parents the process on macOS, so a host that
/// is attached to exits; launch it under the debugger instead.
#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
struct ParentWatch {
    parent: u32,
}

/// The parent process id read at process start, before backend startup, which
/// can take long enough for the owner to be killed meanwhile.
#[cfg(unix)]
static STARTUP_PARENT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

/// Records the parent process. Called first thing in `main`.
pub(crate) fn record_startup_parent() {
    #[cfg(unix)]
    STARTUP_PARENT.get_or_init(std::os::unix::process::parent_id);
}

#[cfg(unix)]
impl ParentWatch {
    fn new() -> Self {
        Self {
            parent: *STARTUP_PARENT.get_or_init(std::os::unix::process::parent_id),
        }
    }

    fn orphaned(self) -> bool {
        std::os::unix::process::parent_id() != self.parent
    }
}

/// Hides the overlay and restores the system cursor, as a `shutdown` request
/// would, before a serve loop exits without one.
#[cfg(unix)]
fn shut_down_backend(backend: &mut OverlayHostBackend) {
    let _ = backend.handle_message(OverlayHostMessage {
        version: sky_cua_overlay_host::OVERLAY_HOST_PROTOCOL_VERSION,
        kind: OverlayHostMessageKind::Shutdown,
        state: None,
        gesture: None,
        sequence: None,
        reason: None,
        arrival_wait: None,
    });
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
                // This loop waits on the request channel, not on the main run
                // loop, which AppKit needs turned to commit window changes.
                #[cfg(target_os = "macos")]
                backend.idle(Duration::from_millis(1));
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

/// On other Unix platforms there is no event source to integrate with, so the
/// socket endpoint uses the shared polling accept loop. The backend there is
/// the no-op overlay, which reports its own unsupported capabilities.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn run_unix_socket_event_loop(listener: UnixListener) -> Result<()> {
    run_accept_loop(|| listener.accept().map(|(stream, _)| stream), "socket")
}

/// Shared serve loop for socket-style endpoints: poll a non-blocking listener
/// for clients, switch each accepted stream to blocking mode with per-client
/// I/O timeouts, handle one request per connection, advance frame-paced
/// overlay work while idle, and exit on a shutdown message or when the
/// process that started the host is gone.
///
/// Clients are handled serially: a connected client that never sends a
/// request can delay subsequent requests by at most `CLIENT_IO_TIMEOUT`.
/// That bound is acceptable for the supported single service-client model.
fn run_accept_loop<S: ClientStream>(
    mut accept: impl FnMut() -> io::Result<S>,
    label: &str,
) -> Result<()> {
    let mut backend = OverlayHostBackend::from_env();
    #[cfg(unix)]
    let parent = ParentWatch::new();
    loop {
        let mut stream = match accept() {
            Ok(stream) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                #[cfg(unix)]
                if parent.orphaned() {
                    shut_down_backend(&mut backend);
                    return Ok(());
                }
                backend.tick();
                // This loop polls accept, so its wait bounds per-request
                // latency; the idle period applies only to the evented loops.
                let pause = backend.pointer_tick_interval().min(ACCEPT_POLL_INTERVAL);
                backend.idle(pause);
                continue;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to accept overlay host {label} connection"));
            }
        };
        if serve_client(&mut backend, &mut stream, label) {
            return Ok(());
        }
    }
}

/// Serves one accepted client: configures the stream, handles its one
/// request, and returns whether that request shut the host down. A failed
/// connection is logged and costs only itself.
fn serve_client(
    backend: &mut OverlayHostBackend,
    stream: &mut impl ClientStream,
    label: &str,
) -> bool {
    // A client that disconnected before setup costs only its own connection:
    // Darwin rejects the timeout options with `EINVAL` once the peer has
    // closed, which is what a connect-and-drop readiness probe leaves behind.
    if let Err(error) = configure_client_stream(stream, label) {
        eprintln!("overlay host {label} connection setup failed: {error:#}");
        return false;
    }
    match handle_socket_message(backend, stream) {
        Ok(shutdown) => shutdown,
        Err(error) => {
            eprintln!("overlay host {label} connection failed: {error:#}");
            false
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
