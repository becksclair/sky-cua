use anyhow::{Context, Result};
use serde::Deserialize;
use zbus::Proxy;

use super::common::{CompatBounds, normalize_window_id};
use super::gnome_introspect::gdbus_call_check;
use super::probe::BackendProbe;
use super::types::LinuxWindowInfo;

pub const GNOME_SHELL_EXTENSION_BACKEND: &str = "gnome-shell-extension";
pub const GNOME_SHELL_EXTENSION_SERVICE: &str = "com.openai.Codex.WindowControl";
pub const GNOME_SHELL_EXTENSION_OBJECT_PATH: &str = "/com/openai/Codex/WindowControl";

pub fn probe() -> BackendProbe {
    let check = gdbus_call_check(
        GNOME_SHELL_EXTENSION_SERVICE,
        GNOME_SHELL_EXTENSION_OBJECT_PATH,
        "com.openai.Codex.WindowControl.ListWindows",
        &[],
    );
    BackendProbe {
        id: GNOME_SHELL_EXTENSION_BACKEND,
        ok: check.ok,
        can_list_windows: check.ok,
        can_focus_apps: check.ok,
        can_focus_windows: check.ok,
        detail: check.detail,
    }
}

pub async fn list_windows() -> Result<Vec<LinuxWindowInfo>> {
    let json = call_extension_json("ListWindows").await?;
    let mut windows = parse_extension_windows(&json)
        .context("Codex GNOME Shell extension returned invalid JSON")?;
    windows.sort_by_key(|window| window.window_id.clone());
    Ok(windows)
}

pub async fn activate_window(
    window_id: &str,
) -> Result<(), sky_cua_platform::diagnostics::BackendError> {
    let parsed = window_id.parse::<u64>().map_err(|error| {
        super::common::backend_error(format!(
            "GNOME extension window_id {window_id} is not numeric: {error}"
        ))
    })?;
    let connection = zbus::Connection::session().await.map_err(|error| {
        super::common::backend_error(format!("failed to connect to session bus: {error}"))
    })?;
    let proxy = Proxy::new(
        &connection,
        GNOME_SHELL_EXTENSION_SERVICE,
        GNOME_SHELL_EXTENSION_OBJECT_PATH,
        GNOME_SHELL_EXTENSION_SERVICE,
    )
    .await
    .map_err(|error| {
        super::common::backend_error(format!(
            "failed to create Codex GNOME Shell extension proxy: {error}"
        ))
    })?;
    let (ok, message): (bool, String) =
        proxy
            .call("ActivateWindow", &(parsed))
            .await
            .map_err(|error| {
                super::common::backend_error(format!(
                    "Codex GNOME Shell extension ActivateWindow failed for {window_id}: {error}"
                ))
            })?;
    if ok {
        Ok(())
    } else {
        Err(super::common::backend_error(format!(
            "Codex GNOME Shell extension refused activation: {message}"
        )))
    }
}

async fn call_extension_json(method: &str) -> Result<String> {
    let connection = zbus::Connection::session()
        .await
        .context("failed to connect to session bus")?;
    let proxy = Proxy::new(
        &connection,
        GNOME_SHELL_EXTENSION_SERVICE,
        GNOME_SHELL_EXTENSION_OBJECT_PATH,
        GNOME_SHELL_EXTENSION_SERVICE,
    )
    .await
    .context("failed to create Codex GNOME Shell extension proxy")?;
    let json: String = proxy
        .call(method, &())
        .await
        .with_context(|| format!("Codex GNOME Shell extension {method} call failed"))?;
    Ok(json)
}

fn parse_extension_windows(json: &str) -> Result<Vec<LinuxWindowInfo>> {
    let windows: Vec<ExtensionWindowInfo> = serde_json::from_str(json)?;
    Ok(windows.into_iter().map(Into::into).collect())
}

#[derive(Debug, Deserialize)]
struct ExtensionWindowInfo {
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

impl From<ExtensionWindowInfo> for LinuxWindowInfo {
    fn from(window: ExtensionWindowInfo) -> Self {
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
            backend: GNOME_SHELL_EXTENSION_BACKEND.to_string(),
            terminal: None,
        }
    }
}
