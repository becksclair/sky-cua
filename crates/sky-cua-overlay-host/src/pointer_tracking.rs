use std::{
    env,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sky_cua_platform::model::AgentCursorPointerTrackingBackendKind;
use zbus::blocking::Proxy;

use crate::system_cursor::SystemPointerPosition;

const KWIN_SERVICE: &str = "org.kde.KWin";
const KWIN_AGENT_CURSOR_PATH: &str = "/com/skycua/AgentCursor";
const KWIN_AGENT_CURSOR_INTERFACE: &str = "com.skycua.AgentCursor";
const KWIN_SIGNAL_PROBE_TIMEOUT: Duration = Duration::from_millis(750);
const HELPER_STREAM_PROBE_TIMEOUT: Duration = Duration::from_millis(750);
const HELPER_PROTOCOL_VERSION: u32 = 1;
const INPUT_HELPER_SOCKET_ENV: &str = "SKY_CUA_INPUT_HELPER_SOCKET";
const DEFAULT_INPUT_HELPER_SOCKET: &str = "/run/sky-cua/input-helper.sock";
const POINTER_TRACKING_DEBUG_ENV: &str = "SKY_CUA_POINTER_TRACKING_DEBUG";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub(crate) struct PointerTrackingBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_milli: u32,
}

#[derive(Debug)]
pub(crate) struct PointerTracker {
    backend: AgentCursorPointerTrackingBackendKind,
    exact: bool,
    receiver: Option<Receiver<SystemPointerPosition>>,
    reason: Option<String>,
}

impl PointerTracker {
    #[must_use]
    pub(crate) fn for_wayland_session(bounds: Option<PointerTrackingBounds>) -> Self {
        if is_kde_session()
            && let Some(tracker) = Self::kwin_signal()
        {
            return tracker;
        }
        if let Some(bounds) = bounds
            && let Some(tracker) = Self::privileged_input_helper(bounds)
        {
            return tracker;
        }
        Self::none("no evented compositor pointer tracker available")
    }

    #[cfg(test)]
    pub(crate) fn test_with_events(events: Vec<SystemPointerPosition>) -> Self {
        let (sender, receiver) = mpsc::channel();
        for event in events {
            sender.send(event).expect("send test pointer event");
        }
        Self {
            backend: AgentCursorPointerTrackingBackendKind::KwinEffectSignal,
            exact: true,
            receiver: Some(receiver),
            reason: Some("test pointer tracker".to_string()),
        }
    }

    #[must_use]
    pub(crate) fn backend(&self) -> AgentCursorPointerTrackingBackendKind {
        self.backend
    }

    #[must_use]
    pub(crate) fn exact(&self) -> bool {
        self.exact
    }

    #[must_use]
    pub(crate) fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub(crate) fn latest_position(&mut self) -> Option<SystemPointerPosition> {
        let receiver = self.receiver.as_ref()?;
        let mut latest = None;
        loop {
            match receiver.try_recv() {
                Ok(position) => latest = Some(position),
                Err(TryRecvError::Empty) => return latest,
                Err(TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.backend = AgentCursorPointerTrackingBackendKind::None;
                    self.exact = false;
                    self.reason = Some("evented pointer tracker disconnected".to_string());
                    return latest;
                }
            }
        }
    }

    pub(crate) fn none(reason: impl Into<String>) -> Self {
        Self {
            backend: AgentCursorPointerTrackingBackendKind::None,
            exact: false,
            receiver: None,
            reason: Some(reason.into()),
        }
    }

    fn kwin_signal() -> Option<Self> {
        let (event_sender, event_receiver) = mpsc::channel();
        let (probe_sender, probe_receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("sky-cua-kwin-pointer-signal".to_string())
            .spawn(move || {
                run_kwin_signal_listener(event_sender, probe_sender);
            })
            .ok()?;

        match probe_receiver.recv_timeout(KWIN_SIGNAL_PROBE_TIMEOUT) {
            Ok(Ok(reason)) => Some(Self {
                backend: AgentCursorPointerTrackingBackendKind::KwinEffectSignal,
                exact: true,
                receiver: Some(event_receiver),
                reason: Some(reason),
            }),
            Ok(Err(_error)) => None,
            Err(_) => None,
        }
    }

    fn privileged_input_helper(bounds: PointerTrackingBounds) -> Option<Self> {
        let socket_path = input_helper_socket_path();
        if !socket_path.exists() {
            return None;
        }
        let (event_sender, event_receiver) = mpsc::channel();
        let (probe_sender, probe_receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("sky-cua-input-helper-pointer-stream".to_string())
            .spawn(move || {
                let result = run_helper_pointer_listener(socket_path, bounds, event_sender);
                let _ = probe_sender.send(result);
            })
            .ok()?;

        match probe_receiver.recv_timeout(HELPER_STREAM_PROBE_TIMEOUT) {
            Ok(Ok(reason)) => Some(Self {
                backend: AgentCursorPointerTrackingBackendKind::PrivilegedInputHelper,
                exact: false,
                receiver: Some(event_receiver),
                reason: Some(reason),
            }),
            Ok(Err(_error)) => None,
            Err(_) => None,
        }
    }
}

#[derive(Debug, Serialize)]
struct HelperObservePointerRequest {
    version: u32,
    op: &'static str,
    bounds: PointerTrackingBounds,
}

#[derive(Debug, Deserialize)]
struct HelperResponse {
    #[serde(default)]
    ok: bool,
}

#[derive(Debug, Deserialize)]
struct HelperPointerEvent {
    event: String,
    x: f64,
    y: f64,
    #[serde(default)]
    coordinate_space: String,
}

fn run_kwin_signal_listener(
    sender: mpsc::Sender<SystemPointerPosition>,
    ready_sender: SyncSender<Result<String, String>>,
) {
    let Ok(connection) = zbus::blocking::Connection::session() else {
        let _ = ready_sender.send(Err("failed to connect to session bus".to_string()));
        return;
    };
    let Ok(proxy) = Proxy::new(
        &connection,
        KWIN_SERVICE,
        KWIN_AGENT_CURSOR_PATH,
        KWIN_AGENT_CURSOR_INTERFACE,
    ) else {
        let _ = ready_sender.send(Err("failed to create KWin cursor shim proxy".to_string()));
        return;
    };
    let Ok(build_id): Result<String, _> = proxy.call("BuildId", &()) else {
        let _ = ready_sender.send(Err("KWin cursor shim BuildId probe failed".to_string()));
        return;
    };
    let Ok(mut monitor) = spawn_gdbus_pointer_monitor() else {
        let _ = ready_sender.send(Err(
            "KWin cursor shim PointerMoved signal unavailable".to_string()
        ));
        return;
    };
    let Some(stdout) = monitor.stdout.take() else {
        let _ = ready_sender.send(Err(
            "KWin cursor shim PointerMoved monitor stdout unavailable".to_string(),
        ));
        let _ = monitor.kill();
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut monitor_ready = false;
    for line in lines.by_ref() {
        let Ok(line) = line else {
            break;
        };
        pointer_tracking_debug(format_args!("kwin signal monitor setup: {line}"));
        if line.starts_with("Monitoring signals on object ") {
            monitor_ready = true;
            break;
        }
    }
    if !monitor_ready {
        let _ = ready_sender.send(Err(
            "KWin cursor shim PointerMoved monitor did not become ready".to_string(),
        ));
        let _ = monitor.kill();
        return;
    }

    let reason = if build_id.trim().is_empty() {
        "KWin cursor shim PointerMoved signal tracker active".to_string()
    } else {
        format!(
            "KWin cursor shim PointerMoved signal tracker active; build_id={}",
            build_id.trim()
        )
    };
    let _ = ready_sender.send(Ok(reason));

    for line in lines {
        let Ok(line) = line else {
            continue;
        };
        pointer_tracking_debug(format_args!("kwin signal monitor line: {line}"));
        let Some((x, y)) = parse_gdbus_pointer_moved_line(&line) else {
            continue;
        };
        pointer_tracking_debug(format_args!("kwin signal monitor parsed: {x},{y}"));
        if sender.send(SystemPointerPosition { x, y }).is_err() {
            break;
        }
    }
    let _ = monitor.kill();
}

fn pointer_tracking_debug(args: std::fmt::Arguments<'_>) {
    if env::var_os(POINTER_TRACKING_DEBUG_ENV).is_some() {
        eprintln!("sky-cua pointer tracking: {args}");
    }
}

fn spawn_gdbus_pointer_monitor() -> std::io::Result<Child> {
    Command::new("gdbus")
        .arg("monitor")
        .arg("--session")
        .arg("--dest")
        .arg(KWIN_SERVICE)
        .arg("--object-path")
        .arg(KWIN_AGENT_CURSOR_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

fn parse_gdbus_pointer_moved_line(line: &str) -> Option<(f64, f64)> {
    let (_, args) = line.split_once(".PointerMoved (")?;
    let args = args.strip_suffix(')')?;
    let mut parts = args.split(',').map(str::trim);
    let x: f64 = parts.next()?.parse().ok()?;
    let y: f64 = parts.next()?.parse().ok()?;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some((x, y))
}

fn run_helper_pointer_listener(
    socket_path: PathBuf,
    bounds: PointerTrackingBounds,
    sender: mpsc::Sender<SystemPointerPosition>,
) -> Result<String, String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect to input helper: {error}"))?;
    let request = HelperObservePointerRequest {
        version: HELPER_PROTOCOL_VERSION,
        op: "observe_pointer",
        bounds,
    };
    let mut request_line = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    request_line.push('\n');
    stream
        .write_all(request_line.as_bytes())
        .map_err(|error| format!("failed to write observe_pointer request: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("failed to flush observe_pointer request: {error}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| format!("failed to read observe_pointer response: {error}"))?;
    let response: HelperResponse =
        serde_json::from_str(line.trim_end()).map_err(|error| error.to_string())?;
    if !response.ok {
        return Err("input helper rejected observe_pointer request".to_string());
    }
    let reason = format!(
        "privileged input helper observe_pointer stream active at {}",
        socket_path.display()
    );

    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read observe_pointer event: {error}"))?;
        if bytes == 0 {
            break;
        }
        let Ok(event) = serde_json::from_str::<HelperPointerEvent>(line.trim_end()) else {
            continue;
        };
        if event.event != "pointer_moved"
            || event.coordinate_space != "desktop_logical"
            || !event.x.is_finite()
            || !event.y.is_finite()
        {
            continue;
        }
        if sender
            .send(SystemPointerPosition {
                x: event.x,
                y: event.y,
            })
            .is_err()
        {
            break;
        }
    }
    Ok(reason)
}

fn input_helper_socket_path() -> PathBuf {
    env::var_os(INPUT_HELPER_SOCKET_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_INPUT_HELPER_SOCKET))
}

fn is_kde_session() -> bool {
    [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ]
    .into_iter()
    .filter_map(|name| env::var(name).ok())
    .flat_map(|value| {
        value
            .split([':', ';'])
            .map(|part| part.trim().to_ascii_lowercase())
            .collect::<Vec<_>>()
    })
    .any(|part| matches!(part.as_str(), "kde" | "plasma"))
}

#[cfg(test)]
mod tests {
    use super::{PointerTracker, SystemPointerPosition, parse_gdbus_pointer_moved_line};

    #[test]
    fn pointer_tracker_coalesces_to_latest_event() {
        let mut tracker = PointerTracker::test_with_events(vec![
            SystemPointerPosition { x: 1.0, y: 2.0 },
            SystemPointerPosition { x: 3.0, y: 4.0 },
            SystemPointerPosition { x: 5.0, y: 6.0 },
        ]);

        assert_eq!(
            tracker.latest_position(),
            Some(SystemPointerPosition { x: 5.0, y: 6.0 })
        );
        assert_eq!(tracker.latest_position(), None);
    }

    #[test]
    fn parses_gdbus_pointer_moved_signal_line() {
        let line = "/com/skycua/AgentCursor: com.skycua.AgentCursor.PointerMoved (1885.0, 2010.0, uint64 11)";

        assert_eq!(parse_gdbus_pointer_moved_line(line), Some((1885.0, 2010.0)));
        assert_eq!(parse_gdbus_pointer_moved_line("unrelated"), None);
    }
}
