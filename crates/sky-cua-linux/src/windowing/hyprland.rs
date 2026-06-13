use std::process::Command;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tracing::warn;

use super::common::{backend_error, output_detail, rect_from_i32};
use super::probe::BackendProbe;
use super::types::LinuxWindowInfo;

pub const HYPRLAND_BACKEND: &str = "hyprland";

#[derive(Debug, Clone, Deserialize)]
struct HyprlandInstance {
    instance: String,
    #[serde(rename = "wl_socket")]
    wl_socket: Option<String>,
}

static TARGET_INSTANCE: Mutex<Option<(String, String)>> = Mutex::new(None);

/// Resolve the Hyprland instance whose `wl_socket` matches `WAYLAND_DISPLAY`.
/// Falls back to ambient `HYPRLAND_INSTANCE_SIGNATURE` when the `instances -j`
/// query is unavailable or returns no match.
///
/// The cache stores `(wayland_display, instance_signature)`. If `WAYLAND_DISPLAY`
/// changes, the cache is invalidated and re-resolved.
fn target_instance_signature() -> Option<String> {
    let wayland_display = crate::env_probe::non_empty_env("WAYLAND_DISPLAY")?;
    let ambient = crate::env_probe::non_empty_env("HYPRLAND_INSTANCE_SIGNATURE");

    // Fast path: cache hit for the same WAYLAND_DISPLAY, but validate that
    // the cached instance is still alive before returning it.
    if let Some(instance) = cached_target_instance(&wayland_display) {
        if hyprland_instance_alive(&instance) {
            return Some(instance);
        }
        // The cached instance has died (compositor restart). Clear the cache
        // and fall through to re-discovery.
        clear_target_instance_cache();
    }

    let discovered = match query_hyprland_instances() {
        Ok(instances) => {
            let mut candidates: Vec<_> = instances
                .into_iter()
                .filter(|i| i.wl_socket.as_deref() == Some(&wayland_display))
                .collect();

            if candidates.is_empty() {
                warn!(
                    "no Hyprland instance reports wl_socket={wayland_display}; \
                     falling back to HYPRLAND_INSTANCE_SIGNATURE"
                );
                None
            } else {
                // Prefer the candidate that matches the ambient HYPRLAND_INSTANCE_SIGNATURE.
                let chosen = ambient
                    .as_ref()
                    .and_then(|ambient_sig| {
                        candidates
                            .iter()
                            .find(|c| &c.instance == ambient_sig)
                            .cloned()
                    })
                    .unwrap_or_else(|| {
                        let first = candidates.remove(0);
                        if !candidates.is_empty() {
                            warn!(
                                "multiple Hyprland instances on {wayland_display}; \
                             arbitrarily choosing the first (instance={})",
                                first.instance
                            );
                        }
                        first
                    });
                Some(chosen.instance)
            }
        }
        Err(error) => {
            warn!(
                "hyprctl instances -j failed: {error}; \
                 falling back to HYPRLAND_INSTANCE_SIGNATURE"
            );
            None
        }
    };

    if let Some(instance) = discovered {
        store_target_instance(wayland_display, instance.clone());
        Some(instance)
    } else {
        ambient
    }
}

fn cached_target_instance(wayland_display: &str) -> Option<String> {
    let guard = TARGET_INSTANCE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .as_ref()
        .and_then(|(cached_display, cached_instance)| {
            (cached_display == wayland_display).then(|| cached_instance.clone())
        })
}

fn store_target_instance(wayland_display: String, instance: String) {
    let mut guard = TARGET_INSTANCE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some((wayland_display, instance));
}

fn clear_target_instance_cache() {
    let mut guard = TARGET_INSTANCE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = None;
}

/// Lightweight health check for a cached Hyprland instance signature.
/// Checks whether the Hyprland IPC socket exists, which is ~3 orders of
/// magnitude cheaper than spawning `hyprctl version`.
fn hyprland_instance_alive(instance: &str) -> bool {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .and_then(|p| p.into_string().ok())
        .unwrap_or_else(|| "/tmp".to_string());
    std::fs::metadata(format!("{runtime_dir}/hypr/{instance}/.socket.sock")).is_ok()
}

fn query_hyprland_instances() -> Result<Vec<HyprlandInstance>> {
    let output = hyprctl_command(None, &["instances", "-j"])
        .output()
        .context("failed to spawn hyprctl instances -j (old Hyprland?)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("exited with status {:?}: {stderr}", output.status.code());
    }

    serde_json::from_slice(&output.stdout).context("unexpected JSON from hyprctl instances -j")
}

fn hyprctl_command(instance: Option<&str>, base_args: &[&str]) -> Command {
    let mut cmd = Command::new("hyprctl");
    if let Some(instance) = instance {
        cmd.arg("-i").arg(instance);
    } else {
        // Discovery mode: clear any inherited HYPRLAND_INSTANCE_SIGNATURE so
        // hyprctl does not bias its socket search toward a stale signature.
        cmd.env_remove("HYPRLAND_INSTANCE_SIGNATURE");
    }
    cmd.args(base_args);
    cmd
}

pub fn probe() -> BackendProbe {
    // Use the lighter `monitors -j` query instead of `clients -j` for the
    // health check; the payload is smaller and faster to parse.
    match hyprctl_command(target_instance_signature().as_deref(), &["monitors", "-j"]).output() {
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
                    "hyprctl monitors -j returned a JSON array".to_string()
                } else {
                    "hyprctl monitors -j did not return a JSON array".to_string()
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
    let output = hyprctl_command(target_instance_signature().as_deref(), &["clients", "-j"])
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
        .filter(|client| client.mapped.unwrap_or(false))
        .map(LinuxWindowInfo::try_from)
        .collect::<Result<Vec<_>>>()?;
    windows.sort_by(|a, b| a.window_id.cmp(&b.window_id));
    Ok(windows)
}

pub fn activate_window(window_id: &str) -> Result<(), sky_cua_platform::diagnostics::BackendError> {
    let address = if window_id.starts_with("0x") {
        format!("address:{window_id}")
    } else {
        format!("address:0x{window_id}")
    };
    let output = hyprctl_command(
        target_instance_signature().as_deref(),
        &["dispatch", "focuswindow", address.as_str()],
    )
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
            display: None,
            display_intersections: Vec::new(),
            workspace: client.workspace.and_then(|workspace| workspace.id),
            focused: client.focus_history_id == Some(0),
            hidden: client.hidden.unwrap_or(false),
            client_type,
            backend: HYPRLAND_BACKEND.to_string(),
            terminal: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::{cached_target_instance, clear_target_instance_cache, store_target_instance};

    #[test]
    #[serial]
    fn target_instance_cache_updates_when_wayland_display_changes() {
        clear_target_instance_cache();

        store_target_instance("wayland-1".to_string(), "instance-1".to_string());
        assert_eq!(
            cached_target_instance("wayland-1"),
            Some("instance-1".to_string())
        );
        assert_eq!(cached_target_instance("wayland-2"), None);

        store_target_instance("wayland-2".to_string(), "instance-2".to_string());
        assert_eq!(cached_target_instance("wayland-1"), None);
        assert_eq!(
            cached_target_instance("wayland-2"),
            Some("instance-2".to_string())
        );

        clear_target_instance_cache();
    }
}
