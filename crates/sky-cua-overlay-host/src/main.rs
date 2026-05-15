use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use anyhow::{Context, Result};
use sky_cua_overlay_host::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostBackend, OverlayHostMessage, OverlayHostMessageKind,
    probe_environment_reply,
};
use sky_cua_platform::model::{AgentCursorPoint, AgentCursorState, CoordinateSpace};

fn main() -> Result<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "serve".to_string());
    match command.as_str() {
        "probe" => print_reply(&probe_environment_reply()),
        "set-cursor" => set_cursor_from_args(std::env::args().skip(2).collect()),
        "serve" => serve_from_args(std::env::args().skip(2).collect()),
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
        reason: None,
    });
    print_reply(&reply)
}

fn serve_from_args(args: Vec<String>) -> Result<()> {
    match args.as_slice() {
        [] => serve_json_lines(),
        [flag, path] if flag == "--socket" => serve_unix_socket(PathBuf::from(path)),
        _ => anyhow::bail!("usage: sky-cua-overlay-host serve [--socket <path>]"),
    }
}

fn serve_json_lines() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut backend = OverlayHostBackend::from_env();

    for line in stdin.lock().lines() {
        let line = line.context("failed to read overlay host request")?;
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
            break;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn serve_unix_socket(path: PathBuf) -> Result<()> {
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
    let mut backend = OverlayHostBackend::from_env();
    for stream in listener.incoming() {
        let mut stream = stream.context("failed to accept overlay host socket connection")?;
        let shutdown = handle_socket_message(&mut backend, &mut stream)?;
        if shutdown {
            break;
        }
    }
    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[cfg(not(unix))]
fn serve_unix_socket(_path: PathBuf) -> Result<()> {
    anyhow::bail!("overlay host socket mode is not implemented on this platform yet")
}

fn handle_socket_message(
    backend: &mut OverlayHostBackend,
    stream: &mut impl ReadWrite,
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

trait ReadWrite: io::Read + io::Write {}

impl<T: io::Read + io::Write> ReadWrite for T {}

#[cfg(all(test, unix))]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use sky_cua_overlay_host::{
        OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    };
    use sky_cua_platform::model::{AgentCursorPoint, AgentCursorState, CoordinateSpace};

    use super::serve_unix_socket;

    #[test]
    fn unix_socket_serve_round_trips_json_lines() {
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
                reason: None,
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
                reason: Some("capture".to_string()),
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
                reason: None,
            },
        );
        assert!(shutdown.ok);
        server
            .join()
            .expect("join overlay host socket server")
            .expect("server should exit cleanly");
        assert!(!socket_path.exists());
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
