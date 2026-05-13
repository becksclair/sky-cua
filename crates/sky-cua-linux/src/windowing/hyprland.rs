use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::common::{backend_error, output_detail, rect_from_i32};
use super::probe::BackendProbe;
use super::types::LinuxWindowInfo;

pub const HYPRLAND_BACKEND: &str = "hyprland";

pub fn probe() -> BackendProbe {
    match Command::new("hyprctl").args(["clients", "-j"]).output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let ok = matches!(
                serde_json::from_str::<serde_json::Value>(&stdout),
                Ok(serde_json::Value::Array(_))
            );
            BackendProbe {
                id: HYPRLAND_BACKEND,
                ok,
                can_list_windows: ok,
                can_focus_apps: ok,
                can_focus_windows: ok,
                detail: if ok {
                    "hyprctl clients -j returned a JSON array".to_string()
                } else {
                    "hyprctl clients -j did not return a JSON array".to_string()
                },
            }
        }
        Ok(output) => BackendProbe {
            id: HYPRLAND_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: output_detail(&output.stdout, &output.stderr, "hyprctl clients -j failed"),
        },
        Err(error) => BackendProbe {
            id: HYPRLAND_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: error.to_string(),
        },
    }
}

pub fn list_windows() -> Result<Vec<LinuxWindowInfo>> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .context("failed to run hyprctl clients -j")?;
    if !output.status.success() {
        bail!(
            "hyprctl clients -j failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_hyprland_clients(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_hyprland_clients(json: &str) -> Result<Vec<LinuxWindowInfo>> {
    let clients: Vec<HyprlandClient> =
        serde_json::from_str(json).context("failed to parse hyprctl clients -j output")?;
    let mut windows = clients
        .into_iter()
        .filter(|client| client.mapped.unwrap_or(true))
        .map(LinuxWindowInfo::try_from)
        .collect::<Result<Vec<_>>>()?;
    windows.sort_by_key(|window| window.window_id.clone());
    Ok(windows)
}

pub fn activate_window(window_id: &str) -> Result<(), sky_cua_platform::diagnostics::BackendError> {
    let address = if window_id.starts_with("0x") {
        format!("address:{window_id}")
    } else {
        format!("address:0x{window_id}")
    };
    let output = Command::new("hyprctl")
        .args(["dispatch", "focuswindow", &address])
        .output()
        .map_err(|error| {
            backend_error(format!(
                "failed to run hyprctl dispatch focuswindow {address}: {error}"
            ))
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(backend_error(format!(
            "hyprctl dispatch focuswindow {address} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

#[derive(Debug, Deserialize)]
struct HyprlandClient {
    address: String,
    mapped: Option<bool>,
    hidden: Option<bool>,
    at: Option<[i32; 2]>,
    size: Option<[u32; 2]>,
    workspace: Option<HyprlandWorkspace>,
    #[serde(rename = "class")]
    class_name: Option<String>,
    title: Option<String>,
    pid: Option<i64>,
    xwayland: Option<bool>,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct HyprlandWorkspace {
    id: Option<i32>,
}

impl TryFrom<HyprlandClient> for LinuxWindowInfo {
    type Error = anyhow::Error;

    fn try_from(client: HyprlandClient) -> Result<Self> {
        let bounds = client.size.map(|[width, height]| {
            rect_from_i32(
                client.at.map(|[x, _]| x),
                client.at.map(|[_, y]| y),
                width,
                height,
            )
        });
        let client_type = client.xwayland.map(|xwayland| {
            if xwayland {
                "x11".to_string()
            } else {
                "wayland".to_string()
            }
        });
        Ok(LinuxWindowInfo {
            window_id: client.address,
            title: client.title,
            app_id: client.class_name.clone(),
            wm_class: client.class_name,
            pid: client.pid.and_then(|pid| u32::try_from(pid).ok()),
            bounds,
            workspace: client.workspace.and_then(|workspace| workspace.id),
            focused: client.focus_history_id == Some(0),
            hidden: client.hidden.unwrap_or(false),
            client_type,
            backend: HYPRLAND_BACKEND.to_string(),
            terminal: None,
        })
    }
}
