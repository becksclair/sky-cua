use anyhow::{Context, Result};
use serde::Deserialize;

use crate::cosmic_helper;

use super::common::{CompatBounds, normalize_window_id};
use super::probe::BackendProbe;
use super::types::LinuxWindowInfo;

pub const COSMIC_WAYLAND_BACKEND: &str = "cosmic-wayland";

pub fn probe() -> BackendProbe {
    match cosmic_helper::probe() {
        Ok(probe) => BackendProbe {
            id: COSMIC_WAYLAND_BACKEND,
            ok: probe.ok,
            can_list_windows: probe.can_list_windows,
            can_focus_apps: probe.can_activate_windows,
            can_focus_windows: probe.can_activate_windows,
            detail: probe.detail,
        },
        Err(error) => BackendProbe {
            id: COSMIC_WAYLAND_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: error.to_string(),
        },
    }
}

pub fn list_windows() -> Result<Vec<LinuxWindowInfo>> {
    let json = cosmic_helper::list_windows_json()?;
    let mut windows =
        parse_helper_windows(&json).context("COSMIC helper returned invalid list-windows JSON")?;
    windows.sort_by_key(|window| window.window_id.clone());
    Ok(windows)
}

pub fn focused_window() -> Result<Option<LinuxWindowInfo>> {
    let json = cosmic_helper::focused_window_json()?;
    let window: Option<HelperWindowInfo> = serde_json::from_str(&json)
        .context("COSMIC helper returned invalid focused-window JSON")?;
    Ok(window.map(Into::into))
}

pub fn activate_window(window_id: &str) -> Result<(), sky_cua_platform::diagnostics::BackendError> {
    let activation = cosmic_helper::activate_window(window_id)
        .map_err(|error| super::common::backend_error(error.to_string()))?;
    if activation.ok {
        Ok(())
    } else {
        Err(super::common::backend_error(format!(
            "COSMIC helper refused activation: {}",
            activation.detail
        )))
    }
}

fn parse_helper_windows(json: &str) -> Result<Vec<LinuxWindowInfo>> {
    let windows: Vec<HelperWindowInfo> = serde_json::from_str(json)?;
    Ok(windows.into_iter().map(Into::into).collect())
}

#[derive(Debug, Deserialize)]
struct HelperWindowInfo {
    window_id: serde_json::Value,
    title: Option<String>,
    app_id: Option<String>,
    wm_class: Option<String>,
    pid: Option<u32>,
    bounds: Option<CompatBounds>,
    workspace: Option<i32>,
    focused: bool,
    hidden: bool,
    client_type: Option<String>,
}

impl From<HelperWindowInfo> for LinuxWindowInfo {
    fn from(window: HelperWindowInfo) -> Self {
        LinuxWindowInfo {
            window_id: normalize_window_id(&window.window_id).unwrap_or_default(),
            title: window.title,
            app_id: window.app_id,
            wm_class: window.wm_class,
            pid: window.pid,
            bounds: window.bounds.map(Into::into),
            display: None,
            display_intersections: Vec::new(),
            workspace: window.workspace,
            focused: window.focused,
            hidden: window.hidden,
            client_type: window.client_type,
            backend: COSMIC_WAYLAND_BACKEND.to_string(),
            terminal: None,
        }
    }
}
