use std::{fs, os::unix::fs::FileTypeExt, path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::common::{backend_error, output_detail, rect_from_i32};
use super::probe::BackendProbe;
use super::types::LinuxWindowInfo;
use crate::session_env::{env_var, xdg_runtime_dir};

pub const I3_BACKEND: &str = "i3";

pub fn probe() -> BackendProbe {
    match i3_msg_command().args(["-t", "get_tree"]).output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let ok = matches!(
                serde_json::from_str::<serde_json::Value>(&stdout),
                Ok(serde_json::Value::Object(_))
            );
            BackendProbe {
                id: I3_BACKEND,
                ok,
                can_list_windows: ok,
                can_focus_apps: ok,
                can_focus_windows: ok,
                detail: if ok {
                    "i3-msg get_tree returned a JSON tree".to_string()
                } else {
                    "i3-msg get_tree did not return a JSON object".to_string()
                },
            }
        }
        Ok(output) => BackendProbe {
            id: I3_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: output_detail(&output.stdout, &output.stderr, "i3-msg -t get_tree failed"),
        },
        Err(error) => BackendProbe {
            id: I3_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: error.to_string(),
        },
    }
}

pub fn list_windows() -> Result<Vec<LinuxWindowInfo>> {
    let output = i3_msg_command()
        .args(["-t", "get_tree"])
        .output()
        .context("failed to run i3-msg -t get_tree")?;
    if !output.status.success() {
        bail!(
            "i3-msg -t get_tree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut windows = parse_i3_tree(&String::from_utf8_lossy(&output.stdout))?;
    hydrate_i3_window_pids(&mut windows);
    Ok(windows)
}

pub(crate) fn parse_i3_tree(json: &str) -> Result<Vec<LinuxWindowInfo>> {
    let root: I3Node =
        serde_json::from_str(json).context("failed to parse i3-msg get_tree output")?;
    let mut windows = Vec::new();
    collect_i3_windows(&root, None, false, &mut windows);
    windows.sort_by_key(|window| window.window_id.clone());
    Ok(windows)
}

pub fn activate_window(window_id: &str) -> Result<(), sky_cua_platform::diagnostics::BackendError> {
    let selector = i3_focus_selector(window_id);
    let output = i3_msg_command()
        .arg(&selector)
        .output()
        .map_err(|error| backend_error(format!("failed to run i3-msg {selector}: {error}")))?;
    if !output.status.success() {
        return Err(backend_error(format!(
            "i3-msg {selector} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let replies: Vec<I3CommandReply> = serde_json::from_slice(&output.stdout)
        .map_err(|error| backend_error(format!("failed to parse i3-msg focus reply: {error}")))?;
    if replies.iter().all(|reply| reply.success) {
        Ok(())
    } else {
        let details = replies
            .into_iter()
            .filter_map(|reply| reply.error)
            .collect::<Vec<_>>()
            .join("; ");
        Err(backend_error(format!(
            "i3-msg {selector} did not focus the window: {}",
            if details.is_empty() {
                "unknown i3 failure"
            } else {
                details.as_str()
            }
        )))
    }
}

fn collect_i3_windows(
    node: &I3Node,
    workspace: Option<i32>,
    in_dockarea: bool,
    windows: &mut Vec<LinuxWindowInfo>,
) {
    let node_type = node.node_type.as_deref();
    let current_workspace = if node_type == Some("workspace") {
        node.num
    } else {
        workspace
    };
    let current_in_dockarea = in_dockarea || node_type == Some("dockarea");
    if let Some(window) = node.to_window_info(current_workspace, current_in_dockarea) {
        windows.push(window);
    }
    for child in &node.nodes {
        collect_i3_windows(child, current_workspace, current_in_dockarea, windows);
    }
    for child in &node.floating_nodes {
        collect_i3_windows(child, current_workspace, current_in_dockarea, windows);
    }
}

fn hydrate_i3_window_pids(windows: &mut [LinuxWindowInfo]) {
    for window in windows {
        if window.pid.is_none() {
            window.pid = i3_window_pid(&window.window_id);
        }
    }
}

fn i3_window_pid(window_id: &str) -> Option<u32> {
    let output = Command::new("xprop")
        .args(["-id", window_id, "_NET_WM_PID"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_xprop_pid(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_xprop_pid(output: &str) -> Option<u32> {
    output.split('=').nth(1)?.trim().parse::<u32>().ok()
}

fn i3_msg_command() -> Command {
    let mut command = Command::new("i3-msg");
    if let Some(socket_path) = i3_socket_path() {
        command.arg("-s").arg(socket_path);
    }
    command
}

fn i3_socket_path() -> Option<PathBuf> {
    if let Some(value) = env_var("I3SOCK") {
        return Some(PathBuf::from(value));
    }
    let socket_dir = xdg_runtime_dir()?.join("i3");
    let mut sockets = fs::read_dir(socket_dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let file_name = file_name.to_str()?;
            if !file_name.starts_with("ipc-socket.") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            if !metadata.file_type().is_socket() {
                return None;
            }
            let modified = metadata.modified().ok();
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    sockets.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    sockets.into_iter().map(|(_, path)| path).next()
}

fn clean_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null")
        .map(ToOwned::to_owned)
}
fn normalize_i3_window_id(window_id: &str) -> String {
    if window_id.starts_with("0x") {
        window_id.to_string()
    } else {
        format!("0x{window_id}")
    }
}

fn i3_focus_selector(window_id: &str) -> String {
    format!(r#"[id="{}"] focus"#, normalize_i3_window_id(window_id))
}

#[derive(Debug, Deserialize)]
struct I3CommandReply {
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct I3Node {
    #[serde(rename = "type")]
    node_type: Option<String>,
    name: Option<String>,
    window: Option<u64>,
    window_type: Option<String>,
    window_properties: Option<I3WindowProperties>,
    rect: Option<I3Rect>,
    geometry: Option<I3Rect>,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    nodes: Vec<I3Node>,
    #[serde(default)]
    floating_nodes: Vec<I3Node>,
    num: Option<i32>,
    scratchpad_state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct I3WindowProperties {
    class: Option<String>,
    instance: Option<String>,
    title: Option<String>,
}
#[derive(Debug, Deserialize)]
struct I3Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl I3Node {
    fn to_window_info(&self, workspace: Option<i32>, in_dockarea: bool) -> Option<LinuxWindowInfo> {
        if in_dockarea || self.window_type.as_deref() == Some("dock") {
            return None;
        }
        let window_id = self.window?;
        let properties = self.window_properties.as_ref();
        let title = clean_string(
            properties
                .and_then(|properties| properties.title.as_deref())
                .or(self.name.as_deref()),
        );
        let wm_class = clean_string(
            properties
                .and_then(|properties| properties.class.as_deref())
                .or_else(|| properties.and_then(|properties| properties.instance.as_deref())),
        );
        let app_id = clean_string(
            properties
                .and_then(|properties| properties.instance.as_deref())
                .or(wm_class.as_deref()),
        );
        let rect = self.rect.as_ref().or(self.geometry.as_ref());
        let bounds =
            rect.map(|rect| rect_from_i32(Some(rect.x), Some(rect.y), rect.width, rect.height));
        Some(LinuxWindowInfo {
            window_id: format!("0x{window_id:x}"),
            title,
            app_id,
            wm_class,
            pid: None,
            bounds,
            workspace,
            focused: self.focused,
            hidden: matches!(
                self.scratchpad_state.as_deref(),
                Some("fresh") | Some("changed")
            ),
            client_type: Some("x11".to_string()),
            backend: I3_BACKEND.to_string(),
            terminal: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_tree_json(scratchpad_state: Option<&str>) -> String {
        let state_field = scratchpad_state
            .map(|s| format!(r#", "scratchpad_state": "{s}""#))
            .unwrap_or_default();
        format!(
            r#"{{
                "type": "root",
                "nodes": [{{
                    "type": "output",
                    "nodes": [{{
                        "type": "workspace",
                        "num": 1,
                        "nodes": [],
                        "floating_nodes": [{{
                            "type": "floating_con",
                            "window": 1,
                            "window_properties": {{
                                "title": "test",
                                "class": "TestApp",
                                "instance": "testapp"
                            }},
                            "rect": {{"x": 0, "y": 0, "width": 100, "height": 100}},
                            "focused": false
                            {state_field}
                        }}]
                    }}]
                }}]
            }}"#
        )
    }

    #[test]
    fn fresh_scratchpad_window_is_hidden() {
        let windows = parse_i3_tree(&minimal_tree_json(Some("fresh"))).unwrap();
        assert_eq!(windows.len(), 1);
        assert!(windows[0].hidden, "fresh scratchpad state should be hidden");
    }

    #[test]
    fn changed_scratchpad_window_is_hidden() {
        let windows = parse_i3_tree(&minimal_tree_json(Some("changed"))).unwrap();
        assert_eq!(windows.len(), 1);
        assert!(
            windows[0].hidden,
            "changed scratchpad state should also be hidden"
        );
    }

    #[test]
    fn none_scratchpad_window_is_not_hidden() {
        let windows = parse_i3_tree(&minimal_tree_json(Some("none"))).unwrap();
        assert_eq!(windows.len(), 1);
        assert!(
            !windows[0].hidden,
            "none scratchpad state should not be hidden"
        );
    }

    #[test]
    fn missing_scratchpad_field_is_not_hidden() {
        let windows = parse_i3_tree(&minimal_tree_json(None)).unwrap();
        assert_eq!(windows.len(), 1);
        assert!(
            !windows[0].hidden,
            "absent scratchpad_state should not be hidden"
        );
    }

    #[test]
    fn i3_focus_selector_uses_real_quotes() {
        assert_eq!(i3_focus_selector("1"), r#"[id="0x1"] focus"#);
    }
}
