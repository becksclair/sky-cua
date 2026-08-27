use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use cosmic_protocols::{
    toplevel_info::v1::client::{zcosmic_toplevel_handle_v1, zcosmic_toplevel_info_v1},
    toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
};
use serde::{Deserialize, Serialize};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry, wl_seat},
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

const HELP: &str = "sky-cua-cosmic-helper\n\nUsage:\n  sky-cua-cosmic-helper probe\n  sky-cua-cosmic-helper list-windows\n  sky-cua-cosmic-helper focused-window\n  sky-cua-cosmic-helper activate-window --window-id <id>\n  sky-cua-cosmic-helper cursor-bridge [--socket <path>] [--state-file <path>]";
const BACKEND: &str = "cosmic-wayland";
const ACTIVATION_STATE_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowInfo {
    window_id: u64,
    title: Option<String>,
    app_id: Option<String>,
    wm_class: Option<String>,
    pid: Option<u32>,
    bounds: Option<WindowBounds>,
    workspace: Option<i32>,
    focused: bool,
    hidden: bool,
    client_type: Option<String>,
    backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowBounds {
    x: Option<i32>,
    y: Option<i32>,
    width: u32,
    height: u32,
}

/// Toplevel geometry reported by the compositor, relative to a single output.
#[derive(Debug, Clone, Copy)]
struct WindowGeometry {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// A bound `wl_output` and its position within the global compositor space.
///
/// `zcosmic_toplevel_handle_v1::geometry` is relative to the provided output,
/// so the output's global position must be added to recover desktop-logical
/// bounds. `xdg_output.logical_position` is the canonical source; the legacy
/// `wl_output.geometry` x/y is kept as a fallback for compositors that do not
/// advertise the xdg-output manager. `wl_output.geometry` x/y is in physical
/// pixels, so the fallback divides by the output scale to recover logical
/// offsets and stay consistent with the logical toplevel geometry.
#[derive(Debug, Clone)]
struct OutputState {
    proxy: wl_output::WlOutput,
    xdg: Option<zxdg_output_v1::ZxdgOutputV1>,
    wl_pending_position: Option<(i32, i32)>,
    wl_position: Option<(i32, i32)>,
    scale: Option<i32>,
    xdg_pending_position: Option<(i32, i32)>,
    xdg_position: Option<(i32, i32)>,
}

impl OutputState {
    fn new(proxy: wl_output::WlOutput) -> Self {
        Self {
            proxy,
            xdg: None,
            wl_pending_position: None,
            wl_position: None,
            scale: None,
            xdg_pending_position: None,
            xdg_position: None,
        }
    }

    fn position(&self) -> Option<(i32, i32)> {
        self.xdg_position.or(self.wl_position)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProbeOutput {
    ok: bool,
    can_list_windows: bool,
    can_activate_windows: bool,
    detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivationOutput {
    ok: bool,
    detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivationState {
    window_id: u64,
    timestamp_ms: u64,
}

#[derive(Debug, Deserialize)]
struct CursorBridgeRequest {
    command: String,
}

#[derive(Debug, Serialize)]
struct CursorBridgeResponse {
    ok: bool,
    supported: bool,
    hidden: bool,
    detail: String,
}

#[derive(Debug, Clone, Default)]
struct ToplevelRecord {
    foreign: Option<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1>,
    cosmic: Option<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1>,
    identifier: Option<String>,
    title: Option<String>,
    app_id: Option<String>,
    focused: bool,
    hidden: bool,
    geometry: HashMap<wl_output::WlOutput, WindowGeometry>,
}

impl ToplevelRecord {
    fn to_window(&self, outputs: &[OutputState]) -> Option<WindowInfo> {
        let identifier = self.identifier.as_deref()?;
        Some(WindowInfo {
            window_id: stable_window_id(identifier),
            title: self.title.clone().filter(|value| !value.trim().is_empty()),
            app_id: self.app_id.clone().filter(|value| !value.trim().is_empty()),
            wm_class: None,
            pid: None,
            bounds: self.window_bounds(outputs),
            workspace: None,
            focused: self.focused,
            hidden: self.hidden,
            client_type: Some("wayland".to_string()),
            backend: BACKEND.to_string(),
        })
    }

    /// Global desktop-logical bounds, computed as the union of the window's
    /// per-output geometry translated by each output's position in the global
    /// compositor space. A window that spans multiple outputs reports geometry
    /// relative to each entered output; taking a single output's rect clips the
    /// window to that output, so union the translated rects.
    fn window_bounds(&self, outputs: &[OutputState]) -> Option<WindowBounds> {
        let mut union: Option<(i32, i32, i32, i32)> = None;
        for (output, geometry) in &self.geometry {
            let (offset_x, offset_y) = output_position(outputs, output)?;
            let left = offset_x + geometry.x;
            let top = offset_y + geometry.y;
            let right = left + geometry.width;
            let bottom = top + geometry.height;
            union = Some(match union {
                None => (left, top, right, bottom),
                Some((min_left, min_top, max_right, max_bottom)) => (
                    min_left.min(left),
                    min_top.min(top),
                    max_right.max(right),
                    max_bottom.max(bottom),
                ),
            });
        }
        let (left, top, right, bottom) = union?;
        Some(WindowBounds {
            x: Some(left),
            y: Some(top),
            width: (right - left).max(0) as u32,
            height: (bottom - top).max(0) as u32,
        })
    }
}

fn output_position(outputs: &[OutputState], output: &wl_output::WlOutput) -> Option<(i32, i32)> {
    outputs
        .iter()
        .find(|state| state.proxy == *output)
        .and_then(OutputState::position)
}

#[derive(Default)]
struct AppData {
    toplevel_info: Option<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1>,
    toplevel_manager: Option<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1>,
    xdg_output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    outputs: Vec<OutputState>,
    seats: Vec<wl_seat::WlSeat>,
    capabilities:
        Vec<WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>>,
    records: Vec<ToplevelRecord>,
    by_foreign_id: HashMap<u32, usize>,
    by_cosmic_id: HashMap<u32, usize>,
}

fn main() -> Result<()> {
    match Command::parse(std::env::args().skip(1).collect())? {
        Command::Probe => print_json(&probe()?),
        Command::ListWindows => print_json(&collect_windows()?),
        Command::FocusedWindow => print_json(&focused_window()?),
        Command::ActivateWindow { window_id } => print_json(&activate_window(window_id)?),
        Command::CursorBridge {
            socket_path,
            state_file,
            ready_file,
        } => run_cursor_bridge(socket_path, state_file, ready_file),
    }
}

#[derive(Debug)]
enum Command {
    Probe,
    ListWindows,
    FocusedWindow,
    ActivateWindow {
        window_id: u64,
    },
    CursorBridge {
        socket_path: PathBuf,
        state_file: PathBuf,
        ready_file: PathBuf,
    },
}

impl Command {
    fn parse(args: Vec<String>) -> Result<Self> {
        match args.as_slice() {
            [command] if command == "probe" => Ok(Self::Probe),
            [command] if command == "list-windows" => Ok(Self::ListWindows),
            [command] if command == "focused-window" => Ok(Self::FocusedWindow),
            [command, flag, value] if command == "activate-window" && flag == "--window-id" => {
                Ok(Self::ActivateWindow {
                    window_id: value
                        .parse::<u64>()
                        .with_context(|| format!("invalid window id {value}"))?,
                })
            }
            [command] if command == "cursor-bridge" => Ok(Self::CursorBridge {
                socket_path: default_cursor_bridge_socket()?,
                state_file: default_cursor_bridge_state_file()?,
                ready_file: default_cursor_bridge_ready_file()?,
            }),
            [command, socket_flag, socket]
                if command == "cursor-bridge" && socket_flag == "--socket" =>
            {
                Ok(Self::CursorBridge {
                    socket_path: PathBuf::from(socket),
                    state_file: default_cursor_bridge_state_file()?,
                    ready_file: default_cursor_bridge_ready_file()?,
                })
            }
            [command, state_flag, state_file]
                if command == "cursor-bridge" && state_flag == "--state-file" =>
            {
                Ok(Self::CursorBridge {
                    socket_path: default_cursor_bridge_socket()?,
                    state_file: PathBuf::from(state_file),
                    ready_file: default_cursor_bridge_ready_file()?,
                })
            }
            [command, socket_flag, socket, state_flag, state_file]
                if command == "cursor-bridge"
                    && socket_flag == "--socket"
                    && state_flag == "--state-file" =>
            {
                Ok(Self::CursorBridge {
                    socket_path: PathBuf::from(socket),
                    state_file: PathBuf::from(state_file),
                    ready_file: default_cursor_bridge_ready_file()?,
                })
            }
            [command] if command == "--help" || command == "-h" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            [] => {
                println!("{HELP}");
                std::process::exit(0);
            }
            _ => bail!(
                "unknown arguments. Expected one of: probe, list-windows, focused-window, activate-window --window-id <id>, cursor-bridge [--socket <path>] [--state-file <path>]"
            ),
        }
    }
}

fn default_cursor_bridge_socket() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SKY_CUA_COSMIC_CURSOR_BRIDGE") {
        return Ok(PathBuf::from(path));
    }
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR is required for the COSMIC cursor bridge"))?;
    Ok(PathBuf::from(runtime_dir).join("sky-cua-cosmic-cursor.sock"))
}

fn default_cursor_bridge_state_file() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SKY_CUA_COSMIC_CURSOR_STATE") {
        return Ok(PathBuf::from(path));
    }
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR is required for the COSMIC cursor bridge"))?;
    Ok(PathBuf::from(runtime_dir).join("sky-cua-cosmic-cursor-hidden"))
}

fn default_cursor_bridge_ready_file() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SKY_CUA_COSMIC_CURSOR_READY") {
        return Ok(PathBuf::from(path));
    }
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR is required for the COSMIC cursor bridge"))?;
    Ok(PathBuf::from(runtime_dir).join("sky-cua-cosmic-cursor-ready"))
}

fn run_cursor_bridge(socket_path: PathBuf, state_file: PathBuf, ready_file: PathBuf) -> Result<()> {
    let _ = write_cursor_bridge_state(&state_file, false);
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create socket directory {}", parent.display()))?;
    }
    if socket_path.exists() {
        fs::remove_file(&socket_path)
            .with_context(|| format!("failed to remove stale socket {}", socket_path.display()))?;
    }
    let listener = UnixListener::bind(&socket_path).with_context(|| {
        format!(
            "failed to bind COSMIC cursor bridge {}",
            socket_path.display()
        )
    })?;
    eprintln!(
        "COSMIC cursor bridge listening on {} with state {} and ready sentinel {}",
        socket_path.display(),
        state_file.display(),
        ready_file.display()
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_cursor_bridge_client(stream, &state_file, &ready_file) {
                    eprintln!("COSMIC cursor bridge request failed: {error:#}");
                }
            }
            Err(error) => eprintln!("COSMIC cursor bridge accept failed: {error}"),
        }
    }
    Ok(())
}

fn handle_cursor_bridge_client(
    mut stream: UnixStream,
    state_file: &Path,
    ready_file: &Path,
) -> Result<()> {
    let mut line = String::new();
    BufReader::new(stream.try_clone()?)
        .read_line(&mut line)
        .context("failed to read COSMIC cursor bridge request")?;
    let request: CursorBridgeRequest =
        serde_json::from_str(line.trim()).context("invalid COSMIC cursor bridge request")?;
    let response = handle_cursor_bridge_request(&request.command, state_file, ready_file);
    serde_json::to_writer(&mut stream, &response)
        .context("failed to write COSMIC cursor bridge response")?;
    stream
        .write_all(b"\n")
        .context("failed to terminate COSMIC cursor bridge response")?;
    Ok(())
}

fn handle_cursor_bridge_request(
    command: &str,
    state_file: &Path,
    ready_file: &Path,
) -> CursorBridgeResponse {
    let supported = ready_file.exists();
    match command {
        "status" => cursor_bridge_response(
            supported,
            state_file,
            ready_file,
            if supported {
                "COSMIC cursor bridge is running and compositor integration is active"
            } else {
                "COSMIC cursor bridge is running but compositor integration is not active"
            },
        ),
        "hide" if !supported => cursor_bridge_response(
            false,
            state_file,
            ready_file,
            "COSMIC cursor compositor integration is not active",
        ),
        "hide" => match write_cursor_bridge_state(state_file, true) {
            Ok(()) => {
                cursor_bridge_response(true, state_file, ready_file, "COSMIC cursor hide requested")
            }
            Err(error) => CursorBridgeResponse {
                ok: false,
                supported,
                hidden: state_file.exists(),
                detail: format!("failed to request COSMIC cursor hide: {error:#}"),
            },
        },
        "show" => match write_cursor_bridge_state(state_file, false) {
            Ok(()) => cursor_bridge_response(
                true,
                state_file,
                ready_file,
                if supported {
                    "COSMIC cursor show requested"
                } else {
                    "COSMIC cursor hidden state cleared; compositor integration is not active"
                },
            ),
            Err(error) => CursorBridgeResponse {
                ok: false,
                supported,
                hidden: state_file.exists(),
                detail: format!("failed to request COSMIC cursor show: {error:#}"),
            },
        },
        other => CursorBridgeResponse {
            ok: false,
            supported,
            hidden: state_file.exists(),
            detail: format!("unknown COSMIC cursor bridge command: {other}"),
        },
    }
}

fn cursor_bridge_response(
    ok: bool,
    state_file: &Path,
    ready_file: &Path,
    detail: &str,
) -> CursorBridgeResponse {
    CursorBridgeResponse {
        ok,
        supported: ready_file.exists(),
        hidden: state_file.exists(),
        detail: detail.to_string(),
    }
}

fn write_cursor_bridge_state(state_file: &Path, hidden: bool) -> Result<()> {
    if hidden {
        if let Some(parent) = state_file.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create COSMIC cursor state directory {}",
                    parent.display()
                )
            })?;
        }
        fs::write(state_file, b"hidden\n")
            .with_context(|| format!("failed to write {}", state_file.display()))?;
    } else if state_file.exists() {
        fs::remove_file(state_file)
            .with_context(|| format!("failed to remove {}", state_file.display()))?;
    }
    Ok(())
}

fn probe() -> Result<ProbeOutput> {
    let mut snapshot = Snapshot::collect()?;
    snapshot.wait_for_cosmic_info()?;
    let windows = snapshot.windows();
    let can_activate = snapshot.can_activate_windows();
    Ok(ProbeOutput {
        ok: !windows.is_empty(),
        can_list_windows: !windows.is_empty(),
        can_activate_windows: can_activate,
        detail: if !windows.is_empty() {
            if can_activate {
                format!(
                    "COSMIC foreign toplevel listing is available and activation is supported for {} window(s).",
                    windows.len()
                )
            } else {
                format!(
                    "COSMIC foreign toplevel listing is available for {} window(s), but activation support is incomplete.",
                    windows.len()
                )
            }
        } else {
            "COSMIC foreign toplevel listing is unavailable in this session.".to_string()
        },
    })
}

fn collect_windows() -> Result<Vec<WindowInfo>> {
    let mut snapshot = Snapshot::collect()?;
    snapshot.wait_for_cosmic_info()?;
    Ok(snapshot.windows())
}

fn focused_window() -> Result<Option<WindowInfo>> {
    let mut snapshot = Snapshot::collect()?;
    snapshot.wait_for_cosmic_info()?;
    if let Some(window) = snapshot.windows().into_iter().find(|window| window.focused) {
        clear_activation_state();
        return Ok(Some(window));
    }

    let Some(state) = read_activation_state() else {
        return Ok(None);
    };

    if state_is_stale(&state) {
        clear_activation_state();
        return Ok(None);
    }

    let mut window = snapshot
        .windows()
        .into_iter()
        .find(|window| window.window_id == state.window_id);
    if let Some(window) = window.as_mut() {
        window.focused = true;
    }
    Ok(window)
}

fn activate_window(window_id: u64) -> Result<ActivationOutput> {
    let mut snapshot = Snapshot::collect()?;
    snapshot.activate(window_id)?;
    write_activation_state(window_id)?;
    Ok(ActivationOutput {
        ok: true,
        detail: format!("Requested COSMIC activation for window_id {window_id}."),
    })
}

struct Snapshot {
    event_queue: wayland_client::EventQueue<AppData>,
    app_data: AppData,
}

impl Snapshot {
    fn collect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let (globals, event_queue) =
            registry_queue_init(&conn).context("failed to initialize Wayland registry queue")?;
        let mut snapshot = Self {
            event_queue,
            app_data: AppData::default(),
        };
        let qh = snapshot.event_queue.handle();
        snapshot.app_data.toplevel_info = globals
            .bind::<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, _, _>(&qh, 1..=3, ())
            .ok();
        snapshot.app_data.toplevel_manager = globals
            .bind::<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, _, _>(&qh, 1..=4, ())
            .ok();
        snapshot.app_data.xdg_output_manager = globals
            .bind::<zxdg_output_manager_v1::ZxdgOutputManagerV1, _, _>(&qh, 1..=3, ())
            .ok();
        globals.contents().with_list(|entries| {
            for global in entries {
                match global.interface.as_str() {
                    "wl_seat" => {
                        snapshot.app_data.seats.push(
                            globals.registry().bind::<wl_seat::WlSeat, _, _>(
                                global.name,
                                global.version.min(9),
                                &qh,
                                (),
                            ),
                        );
                    }
                    "wl_output" => {
                        let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
                            global.name,
                            global.version.min(4),
                            &qh,
                            (),
                        );
                        snapshot.app_data.outputs.push(OutputState::new(output));
                    }
                    _ => {}
                }
            }
        });
        if let Some(manager) = snapshot.app_data.xdg_output_manager.as_ref() {
            for output in &mut snapshot.app_data.outputs {
                output.xdg = Some(manager.get_xdg_output(&output.proxy, &qh, ()));
            }
        }
        if snapshot.app_data.toplevel_info.is_some() {
            let _ = globals
                .bind::<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, _, _>(
                    &qh,
                    1..=1,
                    (),
                )
                .ok();
        }
        snapshot.prime()?;
        Ok(snapshot)
    }

    fn prime(&mut self) -> Result<()> {
        for _ in 0..4 {
            self.event_queue
                .roundtrip(&mut self.app_data)
                .context("Wayland roundtrip failed")?;
        }
        Ok(())
    }

    /// cosmic-comp pushes toplevel-info events (state, geometry, output enter)
    /// from its throttled refresh routine (at most once per ~150ms) rather than
    /// replaying them on handle creation, so a freshly connected client sees
    /// nothing until the next refresh cycle. Wait just long enough for one
    /// refresh cycle so geometry and focus state are populated. Do not require
    /// every record to carry geometry: windows that never intersect an output
    /// (minimized, on another workspace) get no geometry event, and demanding
    /// it would spin until the deadline on every call — eating the parent's
    /// focus-verification poll deadline. Bounded by a deadline so the helper
    /// still completes (with whatever state arrived) on a silent compositor.
    fn wait_for_cosmic_info(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut list_identified_at: Option<Instant> = None;
        loop {
            let all_identified = self
                .app_data
                .records
                .iter()
                .all(|record| record.identifier.is_some());
            let all_geometried = self
                .app_data
                .records
                .iter()
                .all(|record| !record.geometry.is_empty());
            // `window_bounds` also needs each output's global position. If a
            // window's geometry lands before its output's logical position is
            // committed, bounds silently become `None` and PID-less
            // corroboration degrades to a bare title match. Gate the early
            // return on output positions the same way we gate on geometry.
            let all_positioned = self
                .app_data
                .outputs
                .iter()
                .all(|output| output.position().is_some());
            let cycle_elapsed = list_identified_at
                .is_some_and(|at| Instant::now() >= at + Duration::from_millis(200));
            if all_identified && all_positioned && (all_geometried || cycle_elapsed) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Ok(());
            }
            if all_identified {
                list_identified_at.get_or_insert_with(Instant::now);
            }
            thread::sleep(Duration::from_millis(50));
            self.event_queue
                .roundtrip(&mut self.app_data)
                .context("Wayland roundtrip failed")?;
        }
    }

    fn windows(&self) -> Vec<WindowInfo> {
        self.app_data
            .records
            .iter()
            .filter_map(|record| record.to_window(&self.app_data.outputs))
            .collect()
    }

    fn can_activate_windows(&self) -> bool {
        !self.app_data.seats.is_empty()
            && self.app_data.toplevel_manager.is_some()
            && self.app_data.records.iter().any(|record| record.cosmic.is_some())
            && self.app_data.capabilities.iter().any(|capability| {
                matches!(
                    capability,
                    WEnum::Value(
                        zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1::Activate
                    )
                )
            })
    }

    fn activate(&mut self, window_id: u64) -> Result<()> {
        if !self.can_activate_windows() {
            bail!("COSMIC activation capability is unavailable");
        }
        let seat = self
            .app_data
            .seats
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("no wl_seat available for activation"))?;
        let record = self
            .app_data
            .records
            .iter()
            .find(|record| {
                record
                    .identifier
                    .as_deref()
                    .is_some_and(|id| stable_window_id(id) == window_id)
            })
            .ok_or_else(|| anyhow!("no COSMIC toplevel matched window_id {window_id}"))?;
        let cosmic = record
            .cosmic
            .as_ref()
            .ok_or_else(|| anyhow!("matched window has no COSMIC activation handle"))?;
        let manager = self
            .app_data
            .toplevel_manager
            .as_ref()
            .ok_or_else(|| anyhow!("COSMIC toplevel management protocol not advertised"))?;
        manager.activate(cosmic, &seat);
        self.event_queue
            .roundtrip(&mut self.app_data)
            .context("Wayland roundtrip after activation failed")?;
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppData {
    fn event(
        app_data: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "ext_foreign_toplevel_list_v1" => {
                    registry.bind::<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, _, _>(
                        name,
                        version.min(1),
                        qh,
                        (),
                    );
                }
                "zcosmic_toplevel_info_v1" => {
                    app_data.toplevel_info = Some(
                        registry.bind::<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, _, _>(
                            name,
                            version.min(3),
                            qh,
                            (),
                        ),
                    );
                }
                "zcosmic_toplevel_manager_v1" => {
                    app_data.toplevel_manager = Some(
                        registry
                            .bind::<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, _, _>(
                                name,
                                version.min(4),
                                qh,
                                (),
                            ),
                    );
                }
                "wl_seat" => {
                    app_data.seats.push(registry.bind::<wl_seat::WlSeat, _, _>(
                        name,
                        version.min(9),
                        qh,
                        (),
                    ));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        _list: &ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                let foreign_id = toplevel.id().protocol_id();
                let mut record = ToplevelRecord {
                    foreign: Some(toplevel.clone()),
                    ..Default::default()
                };
                if let Some(info) = app_data.toplevel_info.as_ref() {
                    let cosmic = info.get_cosmic_toplevel(&toplevel, qh, ());
                    app_data
                        .by_cosmic_id
                        .insert(cosmic.id().protocol_id(), app_data.records.len());
                    record.cosmic = Some(cosmic);
                }
                app_data
                    .by_foreign_id
                    .insert(foreign_id, app_data.records.len());
                app_data.records.push(record);
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {}
            _ => unreachable!(),
        }
    }

    event_created_child!(
        AppData,
        ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        [
            ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()),
        ]
    );
}

impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        handle: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(index) = app_data
            .by_foreign_id
            .get(&handle.id().protocol_id())
            .copied()
        else {
            return;
        };
        let record = &mut app_data.records[index];
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                record.identifier = Some(identifier);
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                record.title = Some(title);
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                record.app_id = Some(app_id);
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {}
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                app_data.records[index].foreign = None;
                app_data.records[index].cosmic = None;
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, ()> for AppData {
    fn event(
        _app_data: &mut Self,
        _info: &zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        _event: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(
        AppData,
        zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        [
            zcosmic_toplevel_info_v1::EVT_TOPLEVEL_OPCODE => (zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()),
        ]
    );
}

impl Dispatch<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        handle: &zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = app_data
            .by_cosmic_id
            .get(&handle.id().protocol_id())
            .copied()
        else {
            return;
        };
        let record = &mut app_data.records[index];
        match event {
            zcosmic_toplevel_handle_v1::Event::State { state } => {
                record.focused = false;
                record.hidden = false;
                for value in state.chunks_exact(4) {
                    if let Ok(bytes) = <[u8; 4]>::try_from(value)
                        && let Ok(parsed) =
                            zcosmic_toplevel_handle_v1::State::try_from(u32::from_ne_bytes(bytes))
                    {
                        if parsed == zcosmic_toplevel_handle_v1::State::Activated {
                            record.focused = true;
                        }
                        if parsed == zcosmic_toplevel_handle_v1::State::Minimized {
                            record.hidden = true;
                        }
                    }
                }
            }
            zcosmic_toplevel_handle_v1::Event::Geometry {
                output,
                x,
                y,
                width,
                height,
            } => {
                record.geometry.insert(
                    output,
                    WindowGeometry {
                        x,
                        y,
                        width,
                        height,
                    },
                );
            }
            zcosmic_toplevel_handle_v1::Event::OutputLeave { output } => {
                // Geometry is keyed per output; dropping the stale rect keeps
                // the `window_bounds` union from translating geometry for an
                // output the window no longer intersects.
                record.geometry.remove(&output);
            }
            zcosmic_toplevel_handle_v1::Event::OutputEnter { .. }
            | zcosmic_toplevel_handle_v1::Event::WorkspaceEnter { .. }
            | zcosmic_toplevel_handle_v1::Event::WorkspaceLeave { .. }
            | zcosmic_toplevel_handle_v1::Event::ExtWorkspaceEnter { .. }
            | zcosmic_toplevel_handle_v1::Event::ExtWorkspaceLeave { .. }
            | zcosmic_toplevel_handle_v1::Event::Title { .. }
            | zcosmic_toplevel_handle_v1::Event::AppId { .. }
            | zcosmic_toplevel_handle_v1::Event::Done
            | zcosmic_toplevel_handle_v1::Event::Closed => {}
            _ => unreachable!(),
        }
    }
}

impl Dispatch<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        _manager: &zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1,
        event: zcosmic_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zcosmic_toplevel_manager_v1::Event::Capabilities { capabilities } => {
                app_data.capabilities = capabilities
                    .chunks_exact(4)
                    .map(|chunk| {
                        WEnum::from(u32::from_ne_bytes(
                            chunk
                                .try_into()
                                .expect("chunks_exact(4) guarantees 4-byte chunks"),
                        ))
                    })
                    .collect();
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for AppData {
    fn event(
        _app_data: &mut Self,
        _seat: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for AppData {
    fn event(
        app_data: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = app_data
            .outputs
            .iter_mut()
            .find(|state| &state.proxy == output)
        else {
            return;
        };
        match event {
            // Legacy fallback source for the output position; committed on the
            // wl_output `done` that follows the full property batch.
            wl_output::Event::Geometry { x, y, .. } => {
                state.wl_pending_position = Some((x, y));
            }
            wl_output::Event::Scale { factor } => {
                state.scale = Some(factor);
            }
            wl_output::Event::Done => {
                // wl_output.geometry is in physical pixels; divide by the
                // output scale to recover the logical offset the toplevel
                // geometry is expressed in.
                state.wl_position = state.wl_pending_position.map(|(x, y)| {
                    let factor = state.scale.filter(|factor| *factor > 1);
                    match factor {
                        Some(factor) => (x / factor, y / factor),
                        None => (x, y),
                    }
                });
                // For zxdg_output objects bound at version 3+ the compositor
                // sends wl_output.done (not the deprecated zxdg_output.done)
                // after the xdg_output properties, so commit the canonical
                // logical position here as well.
                state.xdg_position = state.xdg_pending_position;
            }
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_manager_v1::ZxdgOutputManagerV1, ()> for AppData {
    fn event(
        _app_data: &mut Self,
        _manager: &zxdg_output_manager_v1::ZxdgOutputManagerV1,
        _event: zxdg_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        xdg_output: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = app_data
            .outputs
            .iter_mut()
            .find(|state| state.xdg.as_ref() == Some(xdg_output))
        else {
            return;
        };
        match event {
            // Canonical position of the output in the global compositor
            // space, in logical coordinates.
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                state.xdg_pending_position = Some((x, y));
            }
            zxdg_output_v1::Event::Done => {
                state.xdg_position = state.xdg_pending_position;
            }
            _ => {}
        }
    }
}

fn stable_window_id(identifier: &str) -> u64 {
    fnv1a_64(identifier.as_bytes())
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("failed to serialize JSON output")?
    );
    Ok(())
}

fn activation_state_path() -> PathBuf {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("sky-cua-cosmic-helper-last-activation.json")
    } else {
        std::env::temp_dir().join("sky-cua-cosmic-helper-last-activation.json")
    }
}

fn write_activation_state(window_id: u64) -> Result<()> {
    let state = ActivationState {
        window_id,
        timestamp_ms: now_timestamp_ms()?,
    };
    let path = activation_state_path();
    let json = serde_json::to_vec(&state).context("failed to serialize activation state")?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write activation state to {}", path.display()))
}

fn read_activation_state() -> Option<ActivationState> {
    let path = activation_state_path();
    let contents = std::fs::read(&path).ok()?;
    serde_json::from_slice(&contents).ok()
}

fn clear_activation_state() {
    let _ = std::fs::remove_file(activation_state_path());
}

fn state_is_stale(state: &ActivationState) -> bool {
    let Ok(now_ms) = now_timestamp_ms() else {
        return false;
    };
    now_ms.saturating_sub(state.timestamp_ms) > ACTIVATION_STATE_TTL.as_millis() as u64
}

fn now_timestamp_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_activate_window_args_requires_numeric_id() {
        let error = Command::parse(vec![
            "activate-window".to_string(),
            "--window-id".to_string(),
            "nope".to_string(),
        ])
        .unwrap_err()
        .to_string();

        assert!(error.contains("invalid window id"));
    }

    #[test]
    fn stable_window_id_is_stable() {
        assert_eq!(stable_window_id("window-1"), stable_window_id("window-1"));
    }

    #[test]
    fn activation_state_expires_after_ttl() {
        let state = ActivationState {
            window_id: 7,
            timestamp_ms: now_timestamp_ms().unwrap()
                - (ACTIVATION_STATE_TTL.as_millis() as u64 + 1),
        };

        assert!(state_is_stale(&state));
    }

    #[test]
    fn activation_state_is_fresh_within_ttl() {
        let state = ActivationState {
            window_id: 7,
            timestamp_ms: now_timestamp_ms().unwrap(),
        };

        assert!(!state_is_stale(&state));
    }

    #[test]
    fn cursor_bridge_hide_and_show_toggle_state_file() {
        let state_file =
            std::env::temp_dir().join(format!("sky-cua-cosmic-helper-test-{}", std::process::id()));
        let ready_file = std::env::temp_dir().join(format!(
            "sky-cua-cosmic-helper-ready-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&state_file);
        fs::write(&ready_file, b"ready\n").unwrap();

        let hide = handle_cursor_bridge_request("hide", &state_file, &ready_file);
        assert!(hide.ok);
        assert!(hide.supported);
        assert!(hide.hidden);
        assert!(state_file.exists());

        let show = handle_cursor_bridge_request("show", &state_file, &ready_file);
        assert!(show.ok);
        assert!(show.supported);
        assert!(!show.hidden);
        assert!(!state_file.exists());
        let _ = fs::remove_file(&ready_file);
    }

    #[test]
    fn cursor_bridge_rejects_unknown_command() {
        let state_file = std::env::temp_dir().join(format!(
            "sky-cua-cosmic-helper-test-unknown-{}",
            std::process::id()
        ));
        let ready_file = std::env::temp_dir().join(format!(
            "sky-cua-cosmic-helper-ready-test-unknown-{}",
            std::process::id()
        ));
        fs::write(&ready_file, b"ready\n").unwrap();
        let response = handle_cursor_bridge_request("dance", &state_file, &ready_file);

        assert!(!response.ok);
        assert!(response.supported);
        assert!(
            response
                .detail
                .contains("unknown COSMIC cursor bridge command")
        );
        let _ = fs::remove_file(&ready_file);
    }

    #[test]
    fn cursor_bridge_reports_unsupported_without_compositor_ready_sentinel() {
        let state_file = std::env::temp_dir().join(format!(
            "sky-cua-cosmic-helper-test-unsupported-{}",
            std::process::id()
        ));
        let ready_file = std::env::temp_dir().join(format!(
            "sky-cua-cosmic-helper-ready-test-unsupported-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&ready_file);

        let response = handle_cursor_bridge_request("hide", &state_file, &ready_file);
        assert!(!response.ok);
        assert!(!response.supported);
        assert!(!response.hidden);
        assert!(!state_file.exists());
    }

    #[test]
    fn cursor_bridge_show_clears_hidden_state_without_ready_sentinel() {
        let state_file = std::env::temp_dir().join(format!(
            "sky-cua-cosmic-helper-test-show-unsupported-{}",
            std::process::id()
        ));
        let ready_file = std::env::temp_dir().join(format!(
            "sky-cua-cosmic-helper-ready-test-show-unsupported-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&ready_file);
        fs::write(&state_file, b"hidden\n").unwrap();

        let response = handle_cursor_bridge_request("show", &state_file, &ready_file);
        assert!(response.ok);
        assert!(!response.supported);
        assert!(!response.hidden);
        assert!(!state_file.exists());
    }
}
