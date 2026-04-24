use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{AppInfo, CoordinateSpace, RectF};

#[derive(Debug, Clone, PartialEq)]
pub struct X11WindowInfo {
    pub window_id: String,
    pub instance_name: Option<String>,
    pub class_name: Option<String>,
    pub app: AppInfo,
    pub bounds: Option<RectF>,
    pub child_regions: Vec<X11WindowRegion>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct X11WindowRegion {
    pub window_id: String,
    pub parent_window_id: Option<String>,
    pub depth: usize,
    pub name: Option<String>,
    pub bounds: RectF,
}

pub fn x11_server_running() -> bool {
    env::var_os("DISPLAY").is_some()
        && command_exists("xdpyinfo")
        && Command::new("xdpyinfo")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
}

pub fn xwayland_running() -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let pid = file_name.to_string_lossy();
        if !pid.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        if let Ok(name) = fs::read_to_string(entry.path().join("comm"))
            && name.trim() == "Xwayland"
        {
            return true;
        }
    }
    false
}

pub fn x11_window_query_available() -> bool {
    env::var_os("DISPLAY").is_some() && x11_server_running() && command_exists("xprop")
}

pub fn discover_windows() -> Result<Vec<X11WindowInfo>, BackendError> {
    if !x11_window_query_available() {
        return Ok(Vec::new());
    }

    let root_output = Command::new("xprop")
        .arg("-root")
        .arg("_NET_CLIENT_LIST")
        .arg("_NET_ACTIVE_WINDOW")
        .output()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to query X11 root window metadata with xprop: {error}"),
            )
        })?;

    if !root_output.status.success() {
        return Err(BackendError::new(
            BackendErrorCode::Internal,
            format!(
                "xprop failed while querying X11 root window metadata: {}",
                String::from_utf8_lossy(&root_output.stderr).trim()
            ),
        ));
    }

    let root_stdout = String::from_utf8_lossy(&root_output.stdout);
    let (mut window_ids, active_window_id) = parse_root_window_list(&root_stdout);
    if window_ids.is_empty() {
        window_ids = fallback_window_ids_from_tree()?;
    }
    let toolkit_guess = if xwayland_running() {
        "XWayland"
    } else {
        "X11"
    };

    let mut windows = Vec::new();
    for window_id in window_ids {
        let window_output = Command::new("xprop")
            .arg("-id")
            .arg(&window_id)
            .arg("WM_CLASS")
            .arg("_NET_WM_NAME")
            .arg("WM_NAME")
            .arg("_NET_WM_PID")
            .output()
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!("failed to query X11 window {window_id} metadata with xprop: {error}"),
                )
            })?;

        if !window_output.status.success() {
            continue;
        }

        let window_stdout = String::from_utf8_lossy(&window_output.stdout);
        if let Some(window) = parse_window_info(
            &window_id,
            &window_stdout,
            active_window_id.as_deref() == Some(window_id.as_str()),
            toolkit_guess,
        ) {
            windows.push(window);
        }
    }

    Ok(windows)
}

fn fallback_window_ids_from_tree() -> Result<Vec<String>, BackendError> {
    if !command_exists("xwininfo") {
        return Ok(Vec::new());
    }

    let output = Command::new("xwininfo")
        .arg("-root")
        .arg("-tree")
        .output()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to query X11 root tree with xwininfo: {error}"),
            )
        })?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    Ok(parse_window_ids_from_xwininfo_tree(
        &String::from_utf8_lossy(&output.stdout),
    ))
}

fn parse_root_window_list(output: &str) -> (Vec<String>, Option<String>) {
    let mut window_ids = Vec::new();
    let mut seen = HashSet::new();
    let mut active_window_id = None;

    for line in output.lines() {
        if line.contains("_NET_CLIENT_LIST") {
            for window_id in parse_window_ids_from_line(line) {
                if seen.insert(window_id.clone()) {
                    window_ids.push(window_id);
                }
            }
        } else if line.contains("_NET_ACTIVE_WINDOW") {
            active_window_id = parse_window_ids_from_line(line).into_iter().next();
        }
    }

    (window_ids, active_window_id)
}

fn parse_window_ids_from_line(line: &str) -> Vec<String> {
    let Some((_, values)) = line.split_once('#') else {
        return Vec::new();
    };
    values
        .split(',')
        .map(str::trim)
        .filter(|segment| segment.starts_with("0x"))
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_window_ids_from_xwininfo_tree(output: &str) -> Vec<String> {
    let mut window_ids = Vec::new();
    let mut seen = HashSet::new();

    for line in output.lines() {
        let trimmed = line.trim_start();
        let Some(candidate) = trimmed.split_whitespace().next() else {
            continue;
        };
        if !candidate.starts_with("0x") || line.contains("(the root window)") {
            continue;
        }
        if seen.insert(candidate.to_string()) {
            window_ids.push(candidate.to_string());
        }
    }

    window_ids
}

fn parse_window_info(
    window_id: &str,
    output: &str,
    is_focused_candidate: bool,
    toolkit_guess: &str,
) -> Option<X11WindowInfo> {
    let mut instance_name = None;
    let mut class_name = None;
    let mut window_title = None;
    let mut pid = None;

    for line in output.lines() {
        if line.starts_with("WM_CLASS(") {
            let strings = parse_quoted_strings(line);
            instance_name = strings.first().cloned();
            class_name = strings.get(1).cloned().or_else(|| instance_name.clone());
        } else if line.starts_with("_NET_WM_NAME(") || line.starts_with("WM_NAME(") {
            if let Some(value) = parse_quoted_strings(line).into_iter().next()
                && !value.trim().is_empty()
            {
                window_title = Some(value);
            }
        } else if line.starts_with("_NET_WM_PID(") {
            pid = parse_u32_after_equals(line);
        }
    }

    if instance_name.is_none() && class_name.is_none() && window_title.is_none() {
        return None;
    }

    let executable = pid.and_then(read_executable);
    let name = class_name
        .clone()
        .or_else(|| instance_name.clone())
        .or_else(|| executable.clone())
        .or_else(|| window_title.clone())
        .unwrap_or_else(|| format!("X11 Window {window_id}"));
    let desktop_file_id = executable
        .as_deref()
        .map(guess_desktop_file_id)
        .or_else(|| class_name.as_deref().map(guess_desktop_file_id))
        .or_else(|| instance_name.as_deref().map(guess_desktop_file_id));

    let bounds = query_window_bounds(window_id).ok().flatten();
    let child_regions = query_window_tree(window_id).ok().unwrap_or_default();

    Some(X11WindowInfo {
        window_id: window_id.to_string(),
        instance_name,
        class_name,
        app: AppInfo {
            app_id: format!("x11:{window_id}"),
            name,
            pid,
            executable,
            desktop_file_id,
            toolkit_guess: Some(toolkit_guess.to_string()),
            window_title,
            is_focused_candidate,
        },
        bounds,
        child_regions,
    })
}

fn parse_quoted_strings(line: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for character in line.chars() {
        if in_quotes {
            if character == '"' {
                strings.push(current.clone());
                current.clear();
                in_quotes = false;
            } else {
                current.push(character);
            }
        } else if character == '"' {
            in_quotes = true;
        }
    }

    strings
}

fn parse_u32_after_equals(line: &str) -> Option<u32> {
    let (_, value) = line.split_once('=')?;
    let digits = value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn query_window_bounds(window_id: &str) -> Result<Option<RectF>, BackendError> {
    if !command_exists("xwininfo") {
        return Ok(None);
    }

    let output = Command::new("xwininfo")
        .arg("-id")
        .arg(window_id)
        .output()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to query X11 window {window_id} geometry with xwininfo: {error}"),
            )
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let x = stdout
        .lines()
        .find(|line| line.contains("Absolute upper-left X:"))
        .and_then(parse_f64_after_colon);
    let y = stdout
        .lines()
        .find(|line| line.contains("Absolute upper-left Y:"))
        .and_then(parse_f64_after_colon);
    let width = stdout
        .lines()
        .find(|line| line.trim_start().starts_with("Width:"))
        .and_then(parse_f64_after_colon);
    let height = stdout
        .lines()
        .find(|line| line.trim_start().starts_with("Height:"))
        .and_then(parse_f64_after_colon);

    match (x, y, width, height) {
        (Some(x), Some(y), Some(width), Some(height)) if width > 0.0 && height > 0.0 => {
            Ok(Some(RectF {
                x,
                y,
                width,
                height,
                space: CoordinateSpace::DesktopLogical,
            }))
        }
        _ => Ok(None),
    }
}

fn query_window_tree(window_id: &str) -> Result<Vec<X11WindowRegion>, BackendError> {
    if !command_exists("xwininfo") {
        return Ok(Vec::new());
    }

    let output = Command::new("xwininfo")
        .arg("-id")
        .arg(window_id)
        .arg("-tree")
        .output()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to query X11 window {window_id} tree with xwininfo: {error}"),
            )
        })?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    Ok(parse_window_tree(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_window_tree(output: &str) -> Vec<X11WindowRegion> {
    let mut nodes = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("0x") {
            continue;
        }

        let Some((window_id, name, bounds)) = parse_window_tree_line(trimmed) else {
            continue;
        };
        let indent = line.len().saturating_sub(trimmed.len());

        while stack
            .last()
            .is_some_and(|(ancestor_indent, _)| *ancestor_indent >= indent)
        {
            stack.pop();
        }

        let parent_window_id = stack.last().map(|(_, window_id)| window_id.clone());
        let depth = stack.len() + 1;
        nodes.push(X11WindowRegion {
            window_id: window_id.clone(),
            parent_window_id,
            depth,
            name,
            bounds,
        });
        stack.push((indent, window_id));
    }

    nodes
}

fn parse_window_tree_line(line: &str) -> Option<(String, Option<String>, RectF)> {
    let window_id = line.split_whitespace().next()?.to_string();
    let mut reverse_tokens = line.split_whitespace().rev();
    let absolute_position = reverse_tokens.next()?;
    let size_and_relative = reverse_tokens.next()?;
    let bounds = parse_tree_bounds(size_and_relative, absolute_position)?;
    let name = parse_quoted_strings(line)
        .into_iter()
        .next()
        .filter(|name| !name.trim().is_empty());
    Some((window_id, name, bounds))
}

fn parse_tree_bounds(size_and_relative: &str, absolute_position: &str) -> Option<RectF> {
    let (width, rest) = size_and_relative.split_once('x')?;
    let width = width.parse::<f64>().ok()?;
    let (height, _) = rest.split_once('+')?;
    let height = height.parse::<f64>().ok()?;
    let (x, y) = parse_absolute_position(absolute_position)?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(RectF {
        x,
        y,
        width,
        height,
        space: CoordinateSpace::DesktopLogical,
    })
}

fn parse_absolute_position(value: &str) -> Option<(f64, f64)> {
    let value = value.strip_prefix('+')?;
    let (x, y) = value.split_once('+')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

fn parse_f64_after_colon(line: &str) -> Option<f64> {
    let (_, value) = line.split_once(':')?;
    value.trim().parse().ok()
}

fn read_executable(pid: u32) -> Option<String> {
    let path = PathBuf::from(format!("/proc/{pid}/exe"));
    fs::read_link(path).ok().and_then(|path| {
        path.file_name()
            .and_then(OsStr::to_str)
            .map(ToOwned::to_owned)
    })
}

fn guess_desktop_file_id(name: &str) -> String {
    format!("{}.desktop", normalize_name(name))
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn command_exists(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|directory| directory.join(name).exists())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_f64_after_colon, parse_quoted_strings, parse_root_window_list,
        parse_u32_after_equals, parse_window_ids_from_xwininfo_tree, parse_window_info,
        parse_window_tree,
    };

    #[test]
    fn parses_root_window_ids_and_active_window() {
        let output = "\
_NET_CLIENT_LIST(WINDOW): window id # 0x2400006, 0x3800030\n\
_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3800030\n";
        let (window_ids, active_window_id) = parse_root_window_list(output);
        assert_eq!(
            window_ids,
            vec!["0x2400006".to_string(), "0x3800030".to_string()]
        );
        assert_eq!(active_window_id.as_deref(), Some("0x3800030"));
    }

    #[test]
    fn parses_window_ids_from_xwininfo_tree_when_no_ewmh_client_list_exists() {
        let output = "\
xwininfo: Window id: 0x50d (the root window) (has no name)\n\
\n\
  Root window id: 0x50d (the root window) (has no name)\n\
  Parent window id: 0x0 (none)\n\
     2 children:\n\
     0x20000a \"sky-cua pure x11 xmessage probe\": (\"xmessage\" \"Xmessage\")  250x100+0+0  +0+0\n\
     0x20000b \"sky-cua pointer smoke\": (\"python3\" \"Python3\")  1280x900+0+0  +0+0\n";
        assert_eq!(
            parse_window_ids_from_xwininfo_tree(output),
            vec!["0x20000a".to_string(), "0x20000b".to_string()]
        );
    }

    #[test]
    fn parses_window_metadata_from_xprop_output() {
        let output = "\
WM_CLASS(STRING) = \"xmessage\", \"Xmessage\"\n\
_NET_WM_NAME:  not found.\n\
WM_NAME(STRING) = \"sky-cua x11 inspect title\"\n\
_NET_WM_PID:  not found.\n";
        let window = parse_window_info("0x3800030", output, true, "XWayland").unwrap();
        assert_eq!(window.app.app_id, "x11:0x3800030");
        assert_eq!(window.app.name, "Xmessage");
        assert_eq!(
            window.app.window_title.as_deref(),
            Some("sky-cua x11 inspect title")
        );
        assert_eq!(
            window.app.desktop_file_id.as_deref(),
            Some("xmessage.desktop")
        );
        assert_eq!(window.app.toolkit_guess.as_deref(), Some("XWayland"));
        assert!(window.app.is_focused_candidate);
        assert!(window.bounds.is_none());
        assert!(window.child_regions.is_empty());
    }

    #[test]
    fn parses_quoted_strings_from_xprop_line() {
        assert_eq!(
            parse_quoted_strings("WM_CLASS(STRING) = \"discord\", \"discord\""),
            vec!["discord".to_string(), "discord".to_string()]
        );
    }

    #[test]
    fn parses_u32_property_value() {
        assert_eq!(
            parse_u32_after_equals("_NET_WM_PID(CARDINAL) = 2807974"),
            Some(2_807_974)
        );
    }

    #[test]
    fn parses_f64_after_colon_value() {
        assert_eq!(parse_f64_after_colon("  Width:  174"), Some(174.0));
        assert_eq!(
            parse_f64_after_colon("  Absolute upper-left X:  832"),
            Some(832.0)
        );
    }

    #[test]
    fn parses_child_regions_from_xwininfo_tree() {
        let output = r#"xwininfo: Window id: 0x3a00030 "sky-cua inspect xmessage"

  Root window id: 0x405 (the root window) (has no name)
  Parent window id: 0x405 (the root window) (has no name)
     1 child:
     0x3a00031 (has no name): ()  62x52+0+0  +1248+704
        2 children:
        0x3a00035 (has no name): ()  52x18+4+4  +1252+708
           1 child:
           0x3a00036 (has no name): ()  14x18+-1+-1  +1252+708
        0x3a00032 (has no name): ()  20x17+4+29  +1252+733
"#;

        let nodes = parse_window_tree(output);
        assert_eq!(nodes.len(), 4);
        assert_eq!(nodes[0].window_id, "0x3a00031");
        assert_eq!(nodes[0].parent_window_id, None);
        assert_eq!(nodes[1].parent_window_id.as_deref(), Some("0x3a00031"));
        assert_eq!(nodes[1].depth, 2);
        assert_eq!(nodes[3].bounds.width, 20.0);
        assert_eq!(nodes[3].bounds.y, 733.0);
    }
}
