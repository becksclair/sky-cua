use std::{collections::HashMap, process::Command};

use anyhow::{Context, Result};
use zbus::{Proxy, zvariant::OwnedValue};

use super::common::{output_detail, rect_from_i32};
use super::probe::BackendProbe;
use super::types::LinuxWindowInfo;

pub const GNOME_SHELL_INTROSPECT_BACKEND: &str = "gnome-shell-introspect";

pub fn probe() -> BackendProbe {
    let list = gdbus_call_check(
        "org.gnome.Shell",
        "/org/gnome/Shell/Introspect",
        "org.gnome.Shell.Introspect.GetWindows",
        &[],
    );
    let focus_apps = gdbus_introspect_contains(
        "org.gnome.Shell",
        "/org/gnome/Shell",
        "org.gnome.Shell",
        "FocusApp",
    );
    BackendProbe {
        id: GNOME_SHELL_INTROSPECT_BACKEND,
        ok: list.ok,
        can_list_windows: list.ok,
        can_focus_apps: focus_apps.ok,
        can_focus_windows: false,
        detail: list.detail,
    }
}

pub async fn list_windows() -> Result<Vec<LinuxWindowInfo>> {
    let connection = zbus::Connection::session()
        .await
        .context("failed to connect to session bus")?;
    let proxy = Proxy::new(
        &connection,
        "org.gnome.Shell",
        "/org/gnome/Shell/Introspect",
        "org.gnome.Shell.Introspect",
    )
    .await
    .context("failed to create GNOME Shell introspection proxy")?;
    let windows: HashMap<u64, HashMap<String, OwnedValue>> = proxy
        .call("GetWindows", &())
        .await
        .context("GNOME Shell GetWindows call failed")?;
    let mut windows = windows
        .into_iter()
        .map(|(window_id, properties)| window_from_properties(window_id, &properties))
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| window.window_id.clone());
    Ok(windows)
}

pub async fn focus_app(app_id: &str) -> Result<()> {
    let connection = zbus::Connection::session()
        .await
        .context("failed to connect to session bus")?;
    let proxy = Proxy::new(
        &connection,
        "org.gnome.Shell",
        "/org/gnome/Shell",
        "org.gnome.Shell",
    )
    .await
    .context("failed to create GNOME Shell proxy")?;
    let _: () = proxy
        .call("FocusApp", &(app_id))
        .await
        .with_context(|| format!("GNOME Shell FocusApp failed for app_id {app_id}"))?;
    Ok(())
}

fn window_from_properties(
    window_id: u64,
    properties: &HashMap<String, OwnedValue>,
) -> LinuxWindowInfo {
    let bounds = get_u32(properties, "width")
        .zip(get_u32(properties, "height"))
        .map(|(width, height)| {
            rect_from_i32(
                get_i32(properties, "x"),
                get_i32(properties, "y"),
                width,
                height,
            )
        });
    LinuxWindowInfo {
        window_id: window_id.to_string(),
        title: get_string(properties, "title"),
        app_id: get_string(properties, "app-id"),
        wm_class: get_string(properties, "wm-class"),
        pid: get_u32(properties, "pid"),
        bounds,
        display: None,
        display_intersections: Vec::new(),
        workspace: get_i32(properties, "workspace"),
        focused: get_bool(properties, "has-focus").unwrap_or(false),
        hidden: get_bool(properties, "is-hidden").unwrap_or(false),
        client_type: get_u32(properties, "client-type").map(client_type_name),
        backend: GNOME_SHELL_INTROSPECT_BACKEND.to_string(),
        terminal: None,
        terminal_target_sessions: Vec::new(),
    }
}

fn get_string(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(ToOwned::to_owned)
}
fn get_bool(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    properties
        .get(key)
        .and_then(|value| bool::try_from(value).ok())
}
fn get_u32(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    properties
        .get(key)
        .and_then(|value| u32::try_from(value).ok())
}
fn get_i32(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    properties.get(key).and_then(|value| {
        i32::try_from(value).ok().or_else(|| {
            u32::try_from(value)
                .ok()
                .and_then(|value| value.try_into().ok())
        })
    })
}
fn client_type_name(value: u32) -> String {
    match value {
        0 => "wayland",
        1 => "x11",
        _ => "unknown",
    }
    .to_string()
}

pub(crate) struct ProbeCheck {
    pub ok: bool,
    pub detail: String,
}

pub(crate) fn gdbus_call_check(
    destination: &str,
    object_path: &str,
    method: &str,
    args: &[&str],
) -> ProbeCheck {
    let mut command = Command::new("gdbus");
    command.args([
        "call",
        "--session",
        "--dest",
        destination,
        "--object-path",
        object_path,
        "--method",
        method,
    ]);
    command.args(args);
    run_probe_command(command)
}

pub(crate) fn gdbus_introspect_contains(
    destination: &str,
    object_path: &str,
    interface: &str,
    member: &str,
) -> ProbeCheck {
    let check = run_probe_command({
        let mut command = Command::new("gdbus");
        command.args([
            "introspect",
            "--session",
            "--dest",
            destination,
            "--object-path",
            object_path,
        ]);
        command
    });
    if !check.ok {
        return check;
    }
    let needle = format!("{interface}.{member}");
    let ok = check.detail.contains(&needle) || check.detail.contains(member);
    ProbeCheck {
        ok,
        detail: if ok {
            format!("{interface}.{member} is present")
        } else {
            format!("{interface}.{member} not found")
        },
    }
}

fn run_probe_command(mut command: Command) -> ProbeCheck {
    match command.output() {
        Ok(output) if output.status.success() => ProbeCheck {
            ok: true,
            detail: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        },
        Ok(output) => ProbeCheck {
            ok: false,
            detail: output_detail(&output.stdout, &output.stderr, "probe command failed"),
        },
        Err(error) => ProbeCheck {
            ok: false,
            detail: error.to_string(),
        },
    }
}
