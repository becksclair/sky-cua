use std::collections::HashMap;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    CoordinateSpace, DisplayInfo, DisplayIntersection, DisplayTarget, DoctorDisplayProbeReport,
    DoctorDisplayTopologyReport, EnvironmentInfo, PixelSize, RectF,
};
use zbus::Proxy;
use zbus::zvariant::OwnedValue;

use crate::windowing::types::LinuxWindowInfo;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const GNOME_DISPLAY_CONFIG_BUS_NAME: &str = "org.gnome.Mutter.DisplayConfig";
const GNOME_DISPLAY_CONFIG_OBJECT_PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const GNOME_DISPLAY_CONFIG_INTERFACE: &str = "org.gnome.Mutter.DisplayConfig";

type Properties = HashMap<String, OwnedValue>;
type MonitorSpec = (String, String, String, String);
type MonitorMode = (String, i32, i32, f64, f64, Vec<f64>, Properties);
type Monitor = (MonitorSpec, Vec<MonitorMode>, Properties);
type LogicalMonitor = (i32, i32, f64, u32, bool, Vec<MonitorSpec>, Properties);
type DisplayConfigState = (u32, Vec<Monitor>, Vec<LogicalMonitor>, Properties);

#[derive(Debug)]
struct CommandProbeOutput {
    output: Option<Output>,
    timed_out: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DisplayDiscoveryOutcome {
    pub(crate) displays: Vec<DisplayInfo>,
    pub(crate) report: DoctorDisplayTopologyReport,
}

pub(crate) async fn discover_display_topology(
    environment: &EnvironmentInfo,
) -> DisplayDiscoveryOutcome {
    let (displays, probes) = discover_displays_with_probes(environment).await;
    let report = display_topology_report(displays.as_slice(), probes);
    DisplayDiscoveryOutcome { displays, report }
}

pub(crate) fn display_topology_report_from_environment(
    environment: &EnvironmentInfo,
) -> DoctorDisplayTopologyReport {
    display_topology_report_from_displays(&environment.displays)
}

fn display_topology_report_from_displays(displays: &[DisplayInfo]) -> DoctorDisplayTopologyReport {
    display_topology_report(displays, Vec::new())
}

fn display_topology_report(
    displays: &[DisplayInfo],
    probes: Vec<DoctorDisplayProbeReport>,
) -> DoctorDisplayTopologyReport {
    let selected_provider = selected_provider_from_displays(displays);
    DoctorDisplayTopologyReport {
        display_count: displays.len(),
        selected_provider: selected_provider.clone(),
        probes,
        detail: match selected_provider {
            Some(provider) => format!("{} display(s) available via {provider}", displays.len()),
            None => format!("{} display(s) available", displays.len()),
        },
    }
}

fn selected_provider_from_displays(displays: &[DisplayInfo]) -> Option<String> {
    let provider = displays.first()?.backend.as_str();
    if provider.is_empty() {
        return None;
    }
    Some(
        match provider {
            "x11" => "xrandr",
            value => value,
        }
        .to_string(),
    )
}

async fn discover_displays_with_probes(
    environment: &EnvironmentInfo,
) -> (Vec<DisplayInfo>, Vec<DoctorDisplayProbeReport>) {
    let (mut displays, mut probes) = if environment_matches(environment, &["gnome"]) {
        let (displays, probe) = displays_from_gnome_display_config().await;
        (displays, vec![probe])
    } else if let Some((displays, probe)) = displays_from_environment_provider(environment).await {
        (displays, vec![probe])
    } else {
        (Vec::new(), Vec::new())
    };

    if displays.is_empty() {
        let (fallback_displays, probe) = displays_from_xrandr_blocking().await;
        probes.push(probe);
        displays = fallback_displays;
    }
    (normalize_displays(displays), probes)
}

async fn displays_from_environment_provider(
    environment: &EnvironmentInfo,
) -> Option<(Vec<DisplayInfo>, DoctorDisplayProbeReport)> {
    let environment = environment.clone();
    tokio::task::spawn_blocking(move || {
        if environment_matches(&environment, &["kde", "plasma", "kwin"]) {
            Some(displays_from_kscreen_doctor())
        } else if environment_matches(&environment, &["hyprland"]) {
            Some(displays_from_hyprland())
        } else if environment_matches(&environment, &["cosmic"]) {
            Some(displays_from_cosmic_randr())
        } else {
            None
        }
    })
    .await
    .unwrap_or_default()
}

async fn displays_from_xrandr_blocking() -> (Vec<DisplayInfo>, DoctorDisplayProbeReport) {
    tokio::task::spawn_blocking(displays_from_xrandr)
        .await
        .unwrap_or_else(|_| command_probe_result("xrandr", None, true, Vec::new()))
}

pub(crate) fn assign_window_displays(windows: &mut [LinuxWindowInfo], displays: &[DisplayInfo]) {
    for window in windows {
        let Some(bounds) = window.bounds.as_ref() else {
            window.display = None;
            window.display_intersections.clear();
            continue;
        };
        let mut intersections = displays
            .iter()
            .filter_map(|display| DisplayIntersection::from_bounds(display, bounds))
            .collect::<Vec<_>>();
        intersections.sort_by(|left, right| {
            right
                .intersection_area
                .total_cmp(&left.intersection_area)
                .then_with(|| left.display.index.cmp(&right.display.index))
        });
        window.display = intersections
            .first()
            .map(|intersection| intersection.display.clone());
        window.display_intersections = intersections;
    }
}

pub(crate) fn resolve_display_target(
    displays: &[DisplayInfo],
    target: &DisplayTarget,
) -> Result<DisplayInfo, BackendError> {
    let mut matched_display = None;
    for display in displays
        .iter()
        .filter(|display| display_matches_target(display, target))
    {
        if matched_display.is_some() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!("display target is ambiguous: {target:?}"),
            ));
        }
        matched_display = Some(display);
    }
    matched_display.cloned().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("no display matched requested screenshot target: {target:?}"),
        )
    })
}

pub(crate) fn primary_display(displays: &[DisplayInfo]) -> Option<DisplayInfo> {
    sky_cua_platform::model::primary_flagged_display(displays)
        .or_else(|| displays.first())
        .cloned()
}

fn display_matches_target(display: &DisplayInfo, target: &DisplayTarget) -> bool {
    let mut matched = false;
    if let Some(value) = target.display_id.as_ref() {
        if !display.display_id.eq_ignore_ascii_case(value.trim()) {
            return false;
        }
        matched = true;
    }
    if let Some(value) = target.display_name.as_ref() {
        if !{
            let value = value.trim();
            display
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(value))
                || display.display_id.eq_ignore_ascii_case(value)
        } {
            return false;
        }
        matched = true;
    }
    if let Some(index) = target.display_index {
        if display.index != index {
            return false;
        }
        matched = true;
    }
    matched
}

fn normalize_displays(mut displays: Vec<DisplayInfo>) -> Vec<DisplayInfo> {
    displays.retain(|display| {
        display.logical_rect.width > 0.0
            && display.logical_rect.height > 0.0
            && display.logical_rect.space == CoordinateSpace::DesktopLogical
    });
    displays.sort_by(|left, right| {
        left.index
            .cmp(&right.index)
            .then_with(|| left.display_id.cmp(&right.display_id))
    });
    if !displays.iter().any(|display| display.primary)
        && let Some(first) = displays.first_mut()
    {
        first.primary = true;
    }
    for (index, display) in displays.iter_mut().enumerate() {
        display.index = u32::try_from(index).unwrap_or(u32::MAX);
    }
    displays
}

fn environment_matches(environment: &EnvironmentInfo, needles: &[&str]) -> bool {
    let matches_value = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::to_ascii_lowercase)
            .is_some_and(|value| needles.iter().any(|needle| value.contains(needle)))
    };
    matches_value(&environment.desktop_environment) || matches_value(&environment.compositor)
}

fn displays_from_kscreen_doctor() -> (Vec<DisplayInfo>, DoctorDisplayProbeReport) {
    // Prefer the structured `-j` JSON: its schema is stable across KDE versions
    // and immune to the ANSI-colored `-o` status rows (`HDR: disabled`, `Wide
    // Color Gamut: disabled`) that the legacy text scanner misread as the
    // output's own disabled state, dropping every enabled monitor. Fall back to
    // `-o` only when `-j` yields nothing (e.g. a kscreen build without JSON).
    let json = command_output_with_timeout(
        Command::new("kscreen-doctor")
            .arg("-j")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        COMMAND_TIMEOUT,
    );
    let json_displays = json
        .output
        .as_ref()
        .filter(|output| output.status.success())
        .and_then(|output| parse_kscreen_doctor_json(&String::from_utf8_lossy(&output.stdout)).ok())
        .unwrap_or_default();
    if !json_displays.is_empty() {
        return command_probe_result("kscreen-doctor", json.output, json.timed_out, json_displays);
    }

    let text = command_output_with_timeout(
        Command::new("kscreen-doctor")
            .arg("-o")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        COMMAND_TIMEOUT,
    );
    let text_displays = text
        .output
        .as_ref()
        .filter(|output| output.status.success())
        .map(|output| parse_kscreen_doctor(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    command_probe_result("kscreen-doctor", text.output, text.timed_out, text_displays)
}

async fn displays_from_gnome_display_config() -> (Vec<DisplayInfo>, DoctorDisplayProbeReport) {
    let displays = match gnome_display_config_state().await {
        Some((monitors, logical_monitors)) => {
            displays_from_gnome_state(&monitors, &logical_monitors)
        }
        None => Vec::new(),
    };
    let probe = DoctorDisplayProbeReport {
        provider: "gnome-display-config".to_string(),
        attempted: true,
        ok: !displays.is_empty(),
        timed_out: false,
        exit_status: None,
        stdout_bytes: 0,
        stderr_snippet: None,
        display_count: displays.len(),
        detail: if displays.is_empty() {
            "GNOME DisplayConfig returned no displays".to_string()
        } else {
            format!("GNOME DisplayConfig returned {} display(s)", displays.len())
        },
    };
    (displays, probe)
}

async fn gnome_display_config_state() -> Option<(Vec<Monitor>, Vec<LogicalMonitor>)> {
    let connection = zbus::Connection::session().await.ok()?;
    let proxy = Proxy::new(
        &connection,
        GNOME_DISPLAY_CONFIG_BUS_NAME,
        GNOME_DISPLAY_CONFIG_OBJECT_PATH,
        GNOME_DISPLAY_CONFIG_INTERFACE,
    )
    .await
    .ok()?;
    let (_serial, monitors, logical_monitors, _properties): DisplayConfigState =
        proxy.call("GetCurrentState", &()).await.ok()?;
    Some((monitors, logical_monitors))
}

fn displays_from_hyprland() -> (Vec<DisplayInfo>, DoctorDisplayProbeReport) {
    let output = command_output_with_timeout(
        Command::new("hyprctl")
            .args(["monitors", "-j"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        COMMAND_TIMEOUT,
    );
    let displays = output
        .output
        .as_ref()
        .filter(|output| output.status.success())
        .and_then(|output| parse_hyprland_monitors(&String::from_utf8_lossy(&output.stdout)).ok())
        .unwrap_or_default();
    command_probe_result("hyprland", output.output, output.timed_out, displays)
}

fn displays_from_cosmic_randr() -> (Vec<DisplayInfo>, DoctorDisplayProbeReport) {
    let output = command_output_with_timeout(
        Command::new("cosmic-randr")
            .arg("list")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        COMMAND_TIMEOUT,
    );
    let displays = output
        .output
        .as_ref()
        .filter(|output| output.status.success())
        .map(|output| parse_cosmic_randr(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    command_probe_result("cosmic-randr", output.output, output.timed_out, displays)
}

fn displays_from_xrandr() -> (Vec<DisplayInfo>, DoctorDisplayProbeReport) {
    let output = command_output_with_timeout(
        Command::new("xrandr")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        COMMAND_TIMEOUT,
    );
    let displays = output
        .output
        .as_ref()
        .filter(|output| output.status.success())
        .map(|output| parse_xrandr(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    command_probe_result("xrandr", output.output, output.timed_out, displays)
}

fn command_probe_result(
    provider: &str,
    output: Option<Output>,
    timed_out: bool,
    displays: Vec<DisplayInfo>,
) -> (Vec<DisplayInfo>, DoctorDisplayProbeReport) {
    let exit_status = output.as_ref().and_then(|output| output.status.code());
    let stdout_bytes = output.as_ref().map_or(0, |output| output.stdout.len());
    let stderr_snippet = output.as_ref().and_then(|output| {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        (!stderr.is_empty()).then_some(stderr.chars().take(240).collect())
    });
    let display_count = displays.len();
    let ok = output
        .as_ref()
        .is_some_and(|output| output.status.success() && display_count > 0);
    let detail = if timed_out {
        format!("{provider} timed out after {}s", COMMAND_TIMEOUT.as_secs())
    } else if output.is_none() {
        format!("{provider} could not be started")
    } else if ok {
        format!("{provider} returned {display_count} display(s)")
    } else if let Some(status) = exit_status {
        format!("{provider} exited with status {status} and returned {display_count} display(s)")
    } else {
        format!("{provider} returned {display_count} display(s)")
    };
    (
        displays,
        DoctorDisplayProbeReport {
            provider: provider.to_string(),
            attempted: true,
            ok,
            timed_out,
            exit_status,
            stdout_bytes,
            stderr_snippet,
            display_count,
            detail,
        },
    )
}

fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> CommandProbeOutput {
    let Ok(mut child) = command.spawn() else {
        return CommandProbeOutput {
            output: None,
            timed_out: false,
        };
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return CommandProbeOutput {
                    output: child.wait_with_output().ok(),
                    timed_out: false,
                };
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return CommandProbeOutput {
                    output: None,
                    timed_out: true,
                };
            }
        }
    }
}

fn display(
    backend: &str,
    name: String,
    index: u32,
    primary: bool,
    logical_rect: RectF,
    pixel_size: Option<PixelSize>,
    scale_factor: Option<f64>,
) -> DisplayInfo {
    DisplayInfo {
        display_id: format!("{backend}:{name}"),
        name: Some(name),
        index,
        primary,
        logical_rect,
        pixel_size,
        scale_factor,
        backend: backend.to_string(),
    }
}

fn parse_kscreen_doctor(output: &str) -> Vec<DisplayInfo> {
    #[derive(Default)]
    struct Block {
        name: Option<String>,
        enabled: bool,
        connected: bool,
        primary: bool,
        rect: Option<RectF>,
        scale: Option<f64>,
        mode_size: Option<(u32, u32)>,
    }

    fn update_state_from_line(line: &str, block: &mut Block) {
        // Output-level state (enabled/disabled/connected) appears on the
        // `Output:` line or on standalone word lines. Capability rows such as
        // `HDR: disabled` and `Wide Color Gamut: disabled` are `Key: value`
        // rows whose value must never be read as the output's own state.
        if line.contains(':') && !line.trim_start().starts_with("Output:") {
            return;
        }
        for word in line.split_whitespace().map(|word| {
            word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        }) {
            match word.as_str() {
                "enabled" => block.enabled = true,
                "disabled" => block.enabled = false,
                "connected" => block.connected = true,
                "disconnected" => block.connected = false,
                "primary" => block.primary = true,
                _ => {}
            }
        }
    }

    fn flush(block: &mut Block, out: &mut Vec<DisplayInfo>) {
        let Some(name) = block.name.take() else {
            *block = Block::default();
            return;
        };
        if let Some(rect) = block.rect.take() {
            if !block.enabled || !block.connected {
                *block = Block::default();
                return;
            }
            let pixel_size = block
                .mode_size
                .map(|(width, height)| PixelSize { width, height })
                .or_else(|| pixel_size_for_logical_rect(&rect, block.scale));
            out.push(display(
                "kwin",
                name,
                u32::try_from(out.len()).unwrap_or(u32::MAX),
                block.primary,
                rect,
                pixel_size,
                block.scale,
            ));
        }
        *block = Block::default();
    }

    let mut displays = Vec::new();
    let mut block = Block::default();
    for raw_line in output.lines() {
        let line = strip_ansi(raw_line);
        let line = line.trim();
        if line.starts_with("Output:") {
            flush(&mut block, &mut displays);
            let parts = line.split_whitespace().collect::<Vec<_>>();
            block.name = parts
                .get(2)
                .map(|value| value.trim_matches(',').to_string());
            update_state_from_line(line, &mut block);
            block.rect = parse_geometry_after_keyword(line, "Geometry:");
            block.scale = parse_scale_after_keyword(line, "Scale:");
            continue;
        }
        update_state_from_line(line, &mut block);
        if block.rect.is_none() {
            block.rect = parse_geometry_after_keyword(line, "Geometry:");
        }
        if block.scale.is_none() {
            block.scale = parse_scale_after_keyword(line, "Scale:");
        }
        if block.mode_size.is_none() {
            block.mode_size = parse_current_mode_size(line);
        }
    }
    flush(&mut block, &mut displays);
    displays
}

#[derive(Deserialize)]
struct KscreenDoctorJson {
    #[serde(default)]
    outputs: Vec<KscreenDoctorOutput>,
}

#[derive(Deserialize)]
struct KscreenDoctorOutput {
    name: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    priority: i64,
    #[serde(default)]
    scale: Option<f64>,
    #[serde(default)]
    rotation: Option<i64>,
    #[serde(default)]
    pos: Option<KscreenDoctorPoint>,
    #[serde(default)]
    size: Option<KscreenDoctorSize>,
}

#[derive(Deserialize)]
struct KscreenDoctorPoint {
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
}

#[derive(Deserialize)]
struct KscreenDoctorSize {
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

/// Parse `kscreen-doctor -j` output into the desktop-logical display set.
///
/// Preferred over the legacy `-o` text parse: the JSON schema is stable across
/// KDE versions and immune to the status rows (`HDR: disabled`, `Wide Color
/// Gamut: disabled`) the text scanner misread as the output's own disabled
/// state. Logical geometry is the current mode's pixel size divided by the
/// fractional scale, with width/height swapped for quarter rotations; the
/// `pos` is the desktop-logical origin and `priority == 1` marks the primary
/// output. Disabled, disconnected, or geometry-less outputs are excluded.
fn parse_kscreen_doctor_json(json: &str) -> Result<Vec<DisplayInfo>, serde_json::Error> {
    let parsed: KscreenDoctorJson = serde_json::from_str(json)?;
    let mut displays = Vec::new();
    for output in parsed.outputs {
        if !output.enabled || !output.connected {
            continue;
        }
        let (Some(pos), Some(size)) = (output.pos, output.size) else {
            continue;
        };
        if size.width == 0 || size.height == 0 {
            continue;
        }
        let scale = output.scale.filter(|scale| *scale > 0.0).unwrap_or(1.0);
        // KScreen rotation enum: 1=None, 2=Left(90), 4=Inverted(180), 8=Right(270).
        // Quarter turns swap the logical footprint and the framebuffer extent.
        let quarter_turn = matches!(output.rotation, Some(2) | Some(8));
        let (logical_width, logical_height) = if quarter_turn {
            (
                f64::from(size.height) / scale,
                f64::from(size.width) / scale,
            )
        } else {
            (
                f64::from(size.width) / scale,
                f64::from(size.height) / scale,
            )
        };
        let (pixel_width, pixel_height) = if quarter_turn {
            (size.height, size.width)
        } else {
            (size.width, size.height)
        };
        displays.push(display(
            "kwin",
            output.name,
            u32::try_from(displays.len()).unwrap_or(u32::MAX),
            output.priority == 1,
            RectF {
                x: f64::from(pos.x),
                y: f64::from(pos.y),
                width: logical_width,
                height: logical_height,
                space: CoordinateSpace::DesktopLogical,
            },
            Some(PixelSize {
                width: pixel_width,
                height: pixel_height,
            }),
            Some(scale),
        ));
    }
    Ok(displays)
}

fn parse_geometry_after_keyword(line: &str, keyword: &str) -> Option<RectF> {
    let value = line.split_once(keyword)?.1.trim();
    let mut parts = value.split_whitespace();
    let (x, y) = parse_position(parts.next()?.trim_end_matches(','))?;
    let (width, height) = parse_size(parts.next()?)?;
    Some(RectF {
        x: f64::from(x),
        y: f64::from(y),
        width: f64::from(width),
        height: f64::from(height),
        space: CoordinateSpace::DesktopLogical,
    })
}

fn parse_scale_after_keyword(line: &str, keyword: &str) -> Option<f64> {
    let value = line.split_once(keyword)?.1.split_whitespace().next()?;
    value
        .trim_end_matches('%')
        .parse::<f64>()
        .ok()
        .map(|scale| {
            if value.ends_with('%') {
                scale / 100.0
            } else {
                scale
            }
        })
}

fn parse_current_mode_size(line: &str) -> Option<(u32, u32)> {
    line.split_whitespace().find_map(|token| {
        if !token.contains('*') {
            return None;
        }
        let mode = token.split('@').next()?;
        let mode = mode.rsplit_once(':').map_or(mode, |(_, value)| value);
        parse_size(mode)
    })
}

fn displays_from_gnome_state(
    monitors: &[Monitor],
    logical_monitors: &[LogicalMonitor],
) -> Vec<DisplayInfo> {
    let modes_by_connector = monitors
        .iter()
        .filter_map(|monitor| {
            let connector = monitor.0.0.clone();
            let mode = current_or_first_mode(&monitor.1)?;
            Some((connector, mode))
        })
        .collect::<HashMap<_, _>>();

    logical_monitors
        .iter()
        .enumerate()
        .filter_map(|(index, logical)| {
            let (x, y, scale, transform, primary, specs, _properties) = logical;
            let connector = specs.first()?.0.clone();
            let mode = modes_by_connector.get(&connector)?;
            let mode_width = u32::try_from(mode.1).ok()?;
            let mode_height = u32::try_from(mode.2).ok()?;
            let (pixel_width, pixel_height) = if gnome_transform_swaps_axes(*transform) {
                (mode_height, mode_width)
            } else {
                (mode_width, mode_height)
            };
            let scale = if *scale > 0.0 { *scale } else { 1.0 };
            Some(display(
                "gnome",
                connector,
                u32::try_from(index).unwrap_or(u32::MAX),
                *primary,
                RectF {
                    x: f64::from(*x),
                    y: f64::from(*y),
                    width: f64::from(pixel_width) / scale,
                    height: f64::from(pixel_height) / scale,
                    space: CoordinateSpace::DesktopLogical,
                },
                Some(PixelSize {
                    width: pixel_width,
                    height: pixel_height,
                }),
                Some(scale),
            ))
        })
        .collect()
}

fn gnome_transform_swaps_axes(transform: u32) -> bool {
    matches!(transform, 1 | 3 | 5 | 7)
}

fn current_or_first_mode(modes: &[MonitorMode]) -> Option<&MonitorMode> {
    modes
        .iter()
        .find(|mode| property_bool(&mode.6, "is-current"))
        .or_else(|| modes.first())
}

fn property_bool(properties: &Properties, name: &str) -> bool {
    properties
        .get(name)
        .and_then(|value| bool::try_from(value.clone()).ok())
        .unwrap_or(false)
}

#[derive(Debug, Deserialize)]
struct HyprlandMonitor {
    name: String,
    id: Option<i64>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: Option<f64>,
    focused: Option<bool>,
}

fn parse_hyprland_monitors(json: &str) -> Result<Vec<DisplayInfo>, serde_json::Error> {
    let monitors: Vec<HyprlandMonitor> = serde_json::from_str(json)?;
    Ok(monitors
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let scale = monitor.scale.filter(|scale| *scale > 0.0).unwrap_or(1.0);
            display(
                "hyprland",
                monitor.name,
                monitor
                    .id
                    .and_then(|id| u32::try_from(id).ok())
                    .unwrap_or_else(|| u32::try_from(index).unwrap_or(u32::MAX)),
                monitor.focused.unwrap_or(index == 0),
                RectF {
                    x: f64::from(monitor.x),
                    y: f64::from(monitor.y),
                    width: f64::from(monitor.width) / scale,
                    height: f64::from(monitor.height) / scale,
                    space: CoordinateSpace::DesktopLogical,
                },
                Some(PixelSize {
                    width: monitor.width,
                    height: monitor.height,
                }),
                Some(scale),
            )
        })
        .collect())
}

fn parse_cosmic_randr(output: &str) -> Vec<DisplayInfo> {
    #[derive(Default)]
    struct Block {
        name: Option<String>,
        position: Option<(i32, i32)>,
        scale: Option<f64>,
        size: Option<(u32, u32)>,
    }

    fn flush(block: &mut Block, out: &mut Vec<DisplayInfo>) {
        let Some(name) = block.name.take() else {
            *block = Block::default();
            return;
        };
        if let (Some((x, y)), Some((width, height))) = (block.position, block.size) {
            let scale = block.scale.filter(|scale| *scale > 0.0).unwrap_or(1.0);
            out.push(display(
                "cosmic",
                name,
                u32::try_from(out.len()).unwrap_or(u32::MAX),
                out.is_empty(),
                RectF {
                    x: f64::from(x),
                    y: f64::from(y),
                    width: f64::from(width) / scale,
                    height: f64::from(height) / scale,
                    space: CoordinateSpace::DesktopLogical,
                },
                Some(PixelSize { width, height }),
                Some(scale),
            ));
        }
        *block = Block::default();
    }

    let mut displays = Vec::new();
    let mut block = Block::default();
    for raw_line in output.lines() {
        let line = strip_ansi(raw_line);
        let trimmed = line.trim();
        if trimmed.ends_with("(enabled)")
            && !trimmed.starts_with("Position:")
            && !trimmed.starts_with("Scale:")
        {
            flush(&mut block, &mut displays);
            block.name = trimmed
                .split_whitespace()
                .next()
                .map(|value| value.to_string());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("Position:") {
            block.position = parse_position(value.trim());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("Scale:") {
            block.scale = parse_scale_value(value.trim());
            continue;
        }
        if trimmed.contains("(current)")
            && let Some(size) = parse_first_mode_size(trimmed)
        {
            block.size = Some(size);
        }
    }
    flush(&mut block, &mut displays);
    displays
}

fn parse_xrandr(output: &str) -> Vec<DisplayInfo> {
    let mut displays = Vec::new();
    for line in output.lines() {
        if !line.contains(" connected") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next().map(ToOwned::to_owned) else {
            continue;
        };
        let primary = line.contains(" primary ");
        let geometry = parts.find_map(parse_xrandr_geometry);
        if let Some(rect) = geometry {
            let pixel_size = pixel_size_for_logical_rect(&rect, Some(1.0));
            displays.push(display(
                "x11",
                name,
                u32::try_from(displays.len()).unwrap_or(u32::MAX),
                primary,
                rect,
                pixel_size,
                Some(1.0),
            ));
        }
    }
    displays
}

fn parse_xrandr_geometry(value: &str) -> Option<RectF> {
    let (width_text, rest) = value.split_once('x')?;
    let offset_start = rest.find(['+', '-'])?;
    let height_text = &rest[..offset_start];
    let offsets = &rest[offset_start..];
    let second_offset_start = offsets
        .char_indices()
        .skip(1)
        .find_map(|(index, ch)| matches!(ch, '+' | '-').then_some(index))?;
    Some(RectF {
        x: offsets[..second_offset_start].parse::<f64>().ok()?,
        y: offsets[second_offset_start..].parse::<f64>().ok()?,
        width: f64::from(width_text.parse::<u32>().ok()?),
        height: f64::from(height_text.parse::<u32>().ok()?),
        space: CoordinateSpace::DesktopLogical,
    })
}

fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

fn pixel_size_for_logical_rect(rect: &RectF, scale_factor: Option<f64>) -> Option<PixelSize> {
    let scale = scale_factor.filter(|scale| *scale > 0.0).unwrap_or(1.0);
    Some(PixelSize {
        width: (rect.width * scale).round().max(1.0) as u32,
        height: (rect.height * scale).round().max(1.0) as u32,
    })
}

fn parse_position(value: &str) -> Option<(i32, i32)> {
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn parse_scale_value(value: &str) -> Option<f64> {
    let value = value.trim();
    let scale = value.trim_end_matches('%').parse::<f64>().ok()?;
    Some(if value.ends_with('%') {
        scale / 100.0
    } else {
        scale
    })
}

fn parse_first_mode_size(value: &str) -> Option<(u32, u32)> {
    value.split_whitespace().find_map(parse_size)
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for code in chars.by_ref() {
                if code.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    use super::*;

    #[test]
    fn parses_xrandr_multiple_displays() {
        let output = "Virtual-1 connected primary 1280x800+0+0 normal\nVirtual-2 connected 1024x768+1280+0 normal\n";
        let displays = normalize_displays(parse_xrandr(output));

        assert_eq!(displays.len(), 2);
        assert_eq!(displays[0].display_id, "x11:Virtual-1");
        assert!(displays[0].primary);
        assert_eq!(displays[1].logical_rect.x, 1280.0);
    }

    #[test]
    fn parses_xrandr_displays_with_negative_offsets() {
        let output = "Virtual-1 connected primary 1280x800+0+0 normal\nVirtual-2 connected 1024x768-1024+0 normal\nVirtual-3 connected 800x600+0-600 normal\n";
        let displays = normalize_displays(parse_xrandr(output));

        assert_eq!(displays.len(), 3);
        assert_eq!(displays[1].display_id, "x11:Virtual-2");
        assert_eq!(displays[1].logical_rect.x, -1024.0);
        assert_eq!(displays[1].logical_rect.y, 0.0);
        assert_eq!(displays[2].display_id, "x11:Virtual-3");
        assert_eq!(displays[2].logical_rect.x, 0.0);
        assert_eq!(displays[2].logical_rect.y, -600.0);
    }

    #[test]
    fn display_topology_report_marks_xrandr_provider_without_changing_display_ids() {
        let output = "Virtual-1 connected primary 1280x800+0+0 normal\n";
        let displays = normalize_displays(parse_xrandr(output));
        let report = display_topology_report_from_displays(&displays);

        assert_eq!(displays[0].display_id, "x11:Virtual-1");
        assert_eq!(report.selected_provider.as_deref(), Some("xrandr"));
        assert!(report.detail.contains("xrandr"));
    }

    #[test]
    fn display_topology_report_omits_provider_for_empty_topology() {
        let report = display_topology_report_from_displays(&[]);

        assert_eq!(report.display_count, 0);
        assert_eq!(report.selected_provider, None);
    }

    #[test]
    fn display_topology_report_preserves_empty_probe_evidence() {
        let report = display_topology_report(
            &[],
            vec![DoctorDisplayProbeReport {
                provider: "xrandr".to_string(),
                attempted: true,
                ok: false,
                timed_out: true,
                exit_status: None,
                stdout_bytes: 0,
                stderr_snippet: None,
                display_count: 0,
                detail: "xrandr timed out after 3s".to_string(),
            }],
        );

        assert_eq!(report.display_count, 0);
        assert_eq!(report.selected_provider, None);
        assert_eq!(report.probes.len(), 1);
        assert!(report.probes[0].timed_out);
    }

    #[test]
    fn display_topology_report_preserves_failed_primary_and_xrandr_fallback() {
        let displays = normalize_displays(parse_xrandr(
            "Virtual-1 connected primary 1280x800+0+0 normal\n",
        ));
        let report = display_topology_report(
            &displays,
            vec![
                DoctorDisplayProbeReport {
                    provider: "kscreen-doctor".to_string(),
                    attempted: true,
                    ok: false,
                    timed_out: false,
                    exit_status: Some(1),
                    stdout_bytes: 0,
                    stderr_snippet: Some("not available".to_string()),
                    display_count: 0,
                    detail: "kscreen-doctor exited with status 1".to_string(),
                },
                DoctorDisplayProbeReport {
                    provider: "xrandr".to_string(),
                    attempted: true,
                    ok: true,
                    timed_out: false,
                    exit_status: Some(0),
                    stdout_bytes: 52,
                    stderr_snippet: None,
                    display_count: 1,
                    detail: "xrandr returned 1 display(s)".to_string(),
                },
            ],
        );

        assert_eq!(report.display_count, 1);
        assert_eq!(report.selected_provider.as_deref(), Some("xrandr"));
        assert_eq!(report.probes.len(), 2);
        assert_eq!(report.probes[0].provider, "kscreen-doctor");
        assert_eq!(report.probes[1].provider, "xrandr");
    }

    #[test]
    fn command_probe_result_preserves_stderr_snippet() {
        let (_displays, report) = command_probe_result(
            "xrandr",
            Some(Output {
                status: std::process::ExitStatus::from_raw(256),
                stdout: Vec::new(),
                stderr: b"cannot open display\n".to_vec(),
            }),
            false,
            Vec::new(),
        );

        assert_eq!(
            report.stderr_snippet.as_deref(),
            Some("cannot open display")
        );
        assert_eq!(report.exit_status, Some(1));
    }

    #[test]
    fn parses_cosmic_randr_multiple_displays() {
        let output = "\u{1b}[1mVirtual-1\u{1b}[0m \u{1b}[1;32m(enabled)\u{1b}[0m\n  Position: 0,0\n  Scale: 100%\n  Modes:\n    1280x800 @ 60.000 Hz (current)\n\u{1b}[1mVirtual-2\u{1b}[0m \u{1b}[1;32m(enabled)\u{1b}[0m\n  Position: 1280,0\n  Scale: 125%\n  Modes:\n    1600x1200 @ 60.000 Hz (current)\n";
        let displays = normalize_displays(parse_cosmic_randr(output));

        assert_eq!(displays.len(), 2);
        assert_eq!(displays[0].display_id, "cosmic:Virtual-1");
        assert_eq!(displays[1].scale_factor, Some(1.25));
        assert_eq!(displays[1].logical_rect.x, 1280.0);
        assert_eq!(displays[1].logical_rect.width, 1280.0);
        assert_eq!(
            displays[1].pixel_size,
            Some(PixelSize {
                width: 1600,
                height: 1200,
            })
        );
    }

    #[test]
    fn parses_hyprland_monitors() {
        let displays = normalize_displays(
            parse_hyprland_monitors(
                r#"[{"id":0,"name":"DP-1","x":0,"y":0,"width":1920,"height":1080,"scale":1.0,"focused":false},{"id":1,"name":"HDMI-A-1","x":1920,"y":0,"width":2560,"height":1440,"scale":2.0,"focused":true}]"#,
            )
            .unwrap(),
        );

        assert_eq!(displays.len(), 2);
        assert!(displays[1].primary);
        assert_eq!(displays[1].logical_rect.width, 1280.0);
        assert_eq!(
            displays[1].pixel_size,
            Some(PixelSize {
                width: 2560,
                height: 1440,
            })
        );
    }

    #[test]
    fn parses_gnome_rotated_logical_monitor_axes() {
        let spec: MonitorSpec = (
            "DP-1".to_string(),
            "Dell".to_string(),
            "Portrait".to_string(),
            "SERIAL".to_string(),
        );
        let monitors = vec![(
            spec.clone(),
            vec![(
                "mode-1".to_string(),
                1920,
                1080,
                60.0,
                1.0,
                Vec::new(),
                Properties::new(),
            )],
            Properties::new(),
        )];
        let logical_monitors = vec![(0, 0, 1.0, 1, true, vec![spec], Properties::new())];

        let displays = displays_from_gnome_state(&monitors, &logical_monitors);

        assert_eq!(displays.len(), 1);
        assert_eq!(displays[0].logical_rect.width, 1080.0);
        assert_eq!(displays[0].logical_rect.height, 1920.0);
        assert_eq!(
            displays[0].pixel_size,
            Some(PixelSize {
                width: 1080,
                height: 1920,
            })
        );
    }

    #[test]
    fn parses_kscreen_doctor_blocks() {
        let output = "Output: 1 eDP-1 enabled connected primary\n  Modes: 1:1920x1080@60.00*\n  Geometry: 0,0 1920x1080\n  Scale: 1\nOutput: 2 DP-1 disabled connected\n  Modes: 2:2560x1440@60.00*\n  Geometry: -1280,0 1280x720\n  Scale: 2\nOutput: 3 HDMI-A-1 enabled connected\n  Modes: 3:\u{1b}[01;32m1600x900@60.00*\u{1b}[0m\n  Geometry: 1920,0 1280x720\n  Scale: 1.25\n";
        let displays = normalize_displays(parse_kscreen_doctor(output));

        assert_eq!(displays.len(), 2);
        assert_eq!(displays[0].display_id, "kwin:eDP-1");
        assert!(displays[0].primary);
        assert!(
            displays
                .iter()
                .all(|display| display.display_id != "kwin:DP-1")
        );
        assert_eq!(displays[1].logical_rect.x, 1920.0);
        assert_eq!(
            displays[1].pixel_size,
            Some(PixelSize {
                width: 1600,
                height: 900,
            })
        );
    }

    #[test]
    fn parses_kscreen_doctor_json_mixed_scale() {
        // Real KDE Plasma 6 shape: eDP-1 disabled (excluded); DP-3 enabled
        // scale 1.0 primary (priority 1); HDMI-A-3 enabled scale 1.5 -> logical
        // 1920x1080 / 1.5 = 1280x720 at desktop-logical origin (336,1440).
        let json = r#"{
            "outputs": [
                {"name":"eDP-1","enabled":false,"connected":true,"priority":-1,
                 "pos":{"x":0,"y":373},"scale":1.5,"rotation":1,
                 "size":{"width":2560,"height":1600}},
                {"name":"DP-3","enabled":true,"connected":true,"priority":1,
                 "pos":{"x":0,"y":0},"scale":1.0,"rotation":1,
                 "size":{"width":2560,"height":1440}},
                {"name":"HDMI-A-3","enabled":true,"connected":true,"priority":2,
                 "pos":{"x":336,"y":1440},"scale":1.5,"rotation":1,
                 "size":{"width":1920,"height":1080}}
            ]
        }"#;
        let displays = normalize_displays(parse_kscreen_doctor_json(json).expect("valid json"));

        assert_eq!(displays.len(), 2, "disabled eDP-1 must be excluded");
        let dp = displays
            .iter()
            .find(|display| display.display_id == "kwin:DP-3")
            .expect("DP-3 present");
        assert!(dp.primary, "priority 1 is primary");
        assert_eq!(dp.scale_factor, Some(1.0));
        assert_eq!(dp.logical_rect.x, 0.0);
        assert_eq!(dp.logical_rect.y, 0.0);
        assert_eq!(dp.logical_rect.width, 2560.0);
        assert_eq!(dp.logical_rect.height, 1440.0);
        assert_eq!(
            dp.pixel_size,
            Some(PixelSize {
                width: 2560,
                height: 1440,
            })
        );
        let hdmi = displays
            .iter()
            .find(|display| display.display_id == "kwin:HDMI-A-3")
            .expect("HDMI-A-3 present");
        assert!(!hdmi.primary);
        assert_eq!(hdmi.scale_factor, Some(1.5));
        assert_eq!(hdmi.logical_rect.x, 336.0);
        assert_eq!(hdmi.logical_rect.y, 1440.0);
        assert_eq!(hdmi.logical_rect.width, 1280.0);
        assert_eq!(hdmi.logical_rect.height, 720.0);
        assert_eq!(
            hdmi.pixel_size,
            Some(PixelSize {
                width: 1920,
                height: 1080,
            })
        );
    }

    #[test]
    fn kscreen_doctor_json_swaps_logical_extent_for_quarter_rotation() {
        // rotation 2 (Left/90) swaps the logical footprint and framebuffer extent.
        let json = r#"{"outputs":[{"name":"DP-1","enabled":true,"connected":true,
            "priority":1,"pos":{"x":0,"y":0},"scale":1.0,"rotation":2,
            "size":{"width":1920,"height":1080}}]}"#;
        let displays = normalize_displays(parse_kscreen_doctor_json(json).expect("valid json"));
        assert_eq!(displays.len(), 1);
        assert_eq!(displays[0].logical_rect.width, 1080.0);
        assert_eq!(displays[0].logical_rect.height, 1920.0);
        assert_eq!(
            displays[0].pixel_size,
            Some(PixelSize {
                width: 1080,
                height: 1920,
            })
        );
    }

    #[test]
    fn kscreen_doctor_text_ignores_hdr_disabled_capability_rows() {
        // Regression: newer kscreen-doctor -o emits `HDR: disabled` / `Wide Color
        // Gamut: disabled` rows; the scanner must not read those as the output's
        // own disabled state and drop the monitor.
        let output = "Output: 2 DP-3 2a6c3921\n\tenabled\n\tconnected\n\tGeometry: 0,0 2560x1440\n\tScale: 1\n\tHDR: disabled\n\tWide Color Gamut: disabled\n";
        let displays = normalize_displays(parse_kscreen_doctor(output));
        assert_eq!(displays.len(), 1, "enabled DP-3 must survive HDR rows");
        assert_eq!(displays[0].display_id, "kwin:DP-3");
        assert_eq!(displays[0].logical_rect.width, 2560.0);
    }

    #[test]
    fn assigns_window_to_largest_display_intersection() {
        let displays = normalize_displays(vec![
            display(
                "test",
                "left".to_string(),
                0,
                true,
                RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                    space: CoordinateSpace::DesktopLogical,
                },
                Some(PixelSize {
                    width: 100,
                    height: 100,
                }),
                Some(1.0),
            ),
            display(
                "test",
                "right".to_string(),
                1,
                false,
                RectF {
                    x: 100.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                    space: CoordinateSpace::DesktopLogical,
                },
                Some(PixelSize {
                    width: 100,
                    height: 100,
                }),
                Some(1.0),
            ),
        ]);
        let mut windows = vec![LinuxWindowInfo {
            window_id: "w".to_string(),
            title: None,
            app_id: None,
            wm_class: None,
            pid: None,
            bounds: Some(RectF {
                x: 80.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: false,
            hidden: false,
            client_type: None,
            backend: "test".to_string(),
            terminal: None,
            terminal_target_sessions: Vec::new(),
        }];

        assign_window_displays(&mut windows, &displays);

        assert_eq!(
            windows[0]
                .display
                .as_ref()
                .map(|display| display.display_id.as_str()),
            Some("test:right")
        );
        assert_eq!(windows[0].display_intersections.len(), 2);
    }

    #[test]
    fn display_target_fields_must_match_same_display() {
        let displays = normalize_displays(vec![
            display(
                "test",
                "left".to_string(),
                0,
                true,
                RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                    space: CoordinateSpace::DesktopLogical,
                },
                Some(PixelSize {
                    width: 100,
                    height: 100,
                }),
                Some(1.0),
            ),
            display(
                "test",
                "right".to_string(),
                1,
                false,
                RectF {
                    x: 100.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                    space: CoordinateSpace::DesktopLogical,
                },
                Some(PixelSize {
                    width: 100,
                    height: 100,
                }),
                Some(1.0),
            ),
        ]);

        let resolved = resolve_display_target(
            &displays,
            &DisplayTarget {
                display_id: Some("test:right".to_string()),
                display_name: Some("right".to_string()),
                display_index: Some(1),
            },
        )
        .unwrap();
        assert_eq!(resolved.display_id, "test:right");

        let error = resolve_display_target(
            &displays,
            &DisplayTarget {
                display_id: Some("test:missing".to_string()),
                display_name: None,
                display_index: Some(0),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
            space: CoordinateSpace::DesktopLogical,
        }
    }

    #[test]
    fn normalize_displays_rejects_non_positive_and_wrong_space() {
        let good = display(
            "test",
            "good".to_string(),
            0,
            false,
            rect(0.0, 0.0, 1920.0, 1080.0),
            Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            Some(1.0),
        );
        let zero_width = display(
            "test",
            "zero-w".to_string(),
            1,
            false,
            rect(0.0, 0.0, 0.0, 1080.0),
            None,
            None,
        );
        let stream_space = display(
            "test",
            "stream".to_string(),
            2,
            false,
            RectF {
                space: CoordinateSpace::StreamPixels,
                ..rect(0.0, 0.0, 1920.0, 1080.0)
            },
            None,
            None,
        );
        let mut out_of_order = vec![
            display(
                "test",
                "second".to_string(),
                5,
                false,
                rect(1920.0, 0.0, 1920.0, 1080.0),
                None,
                None,
            ),
            good.clone(),
        ];
        out_of_order.push(zero_width);
        out_of_order.push(stream_space);

        let normalized = normalize_displays(out_of_order);
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].display_id, "test:good");
        assert_eq!(normalized[0].index, 0);
        assert!(normalized[0].primary);
        assert_eq!(normalized[1].display_id, "test:second");
        assert_eq!(normalized[1].index, 1);
    }

    #[test]
    fn primary_display_prefers_flag_then_first() {
        let d1 = display(
            "test",
            "d1".to_string(),
            0,
            false,
            rect(0.0, 0.0, 100.0, 100.0),
            None,
            None,
        );
        let d2 = display(
            "test",
            "d2".to_string(),
            1,
            true,
            rect(100.0, 0.0, 100.0, 100.0),
            None,
            None,
        );
        assert_eq!(
            primary_display(&[d1.clone(), d2.clone()]).map(|d| d.display_id),
            Some("test:d2".to_string())
        );
        assert_eq!(
            primary_display(std::slice::from_ref(&d1)).map(|d| d.display_id),
            Some("test:d1".to_string())
        );
        assert_eq!(primary_display(&[]), None);
    }

    #[test]
    fn display_matches_target_by_id_name_and_index() {
        let d = display(
            "test",
            "Monitor A".to_string(),
            2,
            false,
            rect(0.0, 0.0, 1920.0, 1080.0),
            None,
            None,
        );
        assert!(display_matches_target(
            &d,
            &DisplayTarget {
                display_id: Some("test:monitor a".to_string()),
                display_name: None,
                display_index: None,
            }
        ));
        assert!(!display_matches_target(
            &d,
            &DisplayTarget {
                display_id: Some("test:DP-2".to_string()),
                display_name: None,
                display_index: None,
            }
        ));
        assert!(display_matches_target(
            &d,
            &DisplayTarget {
                display_id: None,
                display_name: Some("monitor a".to_string()),
                display_index: None,
            }
        ));
        assert!(display_matches_target(
            &d,
            &DisplayTarget {
                display_id: None,
                display_name: None,
                display_index: Some(2),
            }
        ));
        assert!(!display_matches_target(
            &d,
            &DisplayTarget {
                display_id: None,
                display_name: None,
                display_index: Some(3),
            }
        ));
        assert!(!display_matches_target(
            &d,
            &DisplayTarget {
                display_id: None,
                display_name: None,
                display_index: None,
            }
        ));
    }

    #[test]
    fn resolve_display_target_rejects_missing_and_ambiguous() {
        let d1 = display(
            "test",
            "d1".to_string(),
            0,
            false,
            rect(0.0, 0.0, 100.0, 100.0),
            None,
            None,
        );
        let d2 = display(
            "test",
            "d2".to_string(),
            1,
            false,
            rect(100.0, 0.0, 100.0, 100.0),
            None,
            None,
        );
        let target = DisplayTarget {
            display_id: Some("test:d1".to_string()),
            display_name: None,
            display_index: None,
        };
        assert_eq!(
            resolve_display_target(&[d1.clone(), d2.clone()], &target)
                .unwrap()
                .display_id,
            "test:d1"
        );

        let missing = DisplayTarget {
            display_id: Some("none".to_string()),
            display_name: None,
            display_index: None,
        };
        assert_eq!(
            resolve_display_target(&[d1.clone(), d2.clone()], &missing)
                .unwrap_err()
                .code,
            BackendErrorCode::InvalidRequest.as_str()
        );

        let ambiguous = DisplayTarget {
            display_id: None,
            display_name: None,
            display_index: None,
        };
        assert!(resolve_display_target(&[d1, d2], &ambiguous).is_err());
    }

    #[test]
    fn assign_window_displays_picks_largest_intersection() {
        let left = display(
            "test",
            "left".to_string(),
            0,
            false,
            rect(0.0, 0.0, 100.0, 100.0),
            None,
            None,
        );
        let right = display(
            "test",
            "right".to_string(),
            1,
            false,
            rect(100.0, 0.0, 100.0, 100.0),
            None,
            None,
        );
        let window = LinuxWindowInfo {
            window_id: "w".to_string(),
            title: None,
            app_id: None,
            wm_class: None,
            pid: None,
            bounds: Some(rect(50.0, 0.0, 100.0, 100.0)),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: false,
            hidden: false,
            client_type: None,
            backend: "test".to_string(),
            terminal: None,
            terminal_target_sessions: Vec::new(),
        };
        let windows = &mut [window];
        assign_window_displays(windows, &[left, right]);
        assert_eq!(
            windows[0].display.as_ref().map(|d| d.display_id.as_str()),
            Some("test:left")
        );
        assert_eq!(windows[0].display_intersections.len(), 2);
    }
}
