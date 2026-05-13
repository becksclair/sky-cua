use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{AppInfo, CoordinateSpace, EnvironmentInfo, RectF, SessionKind};

#[derive(Debug, Clone, PartialEq)]
pub struct KWinWindowInfo {
    pub window_id: String,
    pub resource_name: Option<String>,
    pub resource_class: Option<String>,
    pub app: AppInfo,
    pub bounds: Option<RectF>,
    pub workspace: Option<i32>,
}

pub fn kwin_window_query_available(environment: &EnvironmentInfo) -> bool {
    environment.session_kind == SessionKind::Wayland
        && environment
            .compositor
            .as_deref()
            .is_some_and(|value| value.contains("kde-kwin-wayland"))
        && command_exists("gdbus")
}

pub fn kwin_exact_activation_available(environment: &EnvironmentInfo) -> bool {
    kwin_window_query_available(environment) && qdbus_command().is_some()
}

pub fn discover_windows(
    environment: &EnvironmentInfo,
) -> Result<Vec<KWinWindowInfo>, BackendError> {
    if !kwin_window_query_available(environment) {
        return Ok(Vec::new());
    }

    let active_window = query_active_window(environment)?;
    let mut window_ids = query_window_runner_ids("")?;
    for query in candidate_window_queries() {
        window_ids.extend(query_window_runner_ids(&query)?);
    }
    let mut unique_ids = HashSet::new();
    let mut windows = Vec::new();
    let mut seen = HashSet::new();
    for window_id in window_ids {
        if !unique_ids.insert(window_id.clone()) {
            continue;
        }
        let Some(mut window) = query_window_by_uuid(&window_id)? else {
            continue;
        };
        if active_window
            .as_ref()
            .is_some_and(|active| active.window_id == window.window_id)
        {
            window.app.is_focused_candidate = true;
        }
        seen.insert(window.window_id.clone());
        windows.push(window);
    }

    if let Some(active_window) = active_window
        && seen.insert(active_window.window_id.clone())
    {
        windows.push(active_window);
    }

    windows.sort_by_key(|window| (!window.app.is_focused_candidate, window.app.name.clone()));
    Ok(windows)
}

pub fn query_active_window(
    environment: &EnvironmentInfo,
) -> Result<Option<KWinWindowInfo>, BackendError> {
    let _ = environment;
    // `org.kde.KWin.queryWindowInfo` has proven unreliable under Codex-launched
    // plugin/service environments: sometimes it returns `UserCancel`, and
    // sometimes it simply hangs long enough to wedge the entire backend call.
    // Background-window discovery via WindowsRunner + getWindowInfo is the
    // higher-value seam for native Wayland apps like TIDAL, so for now we
    // intentionally skip the active-window hint instead of risking a deadlock.
    Ok(None)
}

pub fn activate_window(window_id: &str) -> Result<(), BackendError> {
    let uuid = kwin_uuid_from_window_id(window_id)?;
    let Some(qdbus) = qdbus_command() else {
        return Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "KWin exact activation requires qdbus6 or qdbus on PATH",
        ));
    };

    let script_path = write_activation_script(&uuid)?;
    let script_path_string = script_path.display().to_string();
    let load = run_qdbus(
        &qdbus,
        &[
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting.loadScript",
            &script_path_string,
            "sky-cua-activate-window",
        ],
    );
    let script_id = match load.and_then(|output| {
        parse_script_id(&output).ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "KWin did not return a script id while loading activation script: {output}"
                ),
            )
        })
    }) {
        Ok(script_id) => script_id,
        Err(error) => {
            let _ = fs::remove_file(&script_path);
            return Err(error);
        }
    };

    let script_object = format!("/Scripting/Script{script_id}");
    let run_result = run_qdbus(
        &qdbus,
        &["org.kde.KWin", &script_object, "org.kde.kwin.Script.run"],
    );
    let _ = run_qdbus(
        &qdbus,
        &[
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting.unloadScript",
            &script_path_string,
        ],
    );
    let _ = fs::remove_file(&script_path);
    run_result.map(|_| ())
}

fn kwin_uuid_from_window_id(window_id: &str) -> Result<String, BackendError> {
    let value = window_id
        .trim()
        .strip_prefix("kwin:")
        .unwrap_or_else(|| window_id.trim())
        .trim();
    if is_uuid_token(value) {
        return Ok(value.trim_matches(['{', '}']).to_ascii_lowercase());
    }
    Err(BackendError::new(
        BackendErrorCode::InvalidRequest,
        format!("KWin window id {window_id:?} is not a UUID-backed registry window id"),
    ))
}

fn write_activation_script(uuid: &str) -> Result<PathBuf, BackendError> {
    let mut path = env::temp_dir();
    path.push(format!(
        "sky-cua-kwin-activate-{}-{}.js",
        std::process::id(),
        uuid
    ));
    let mut file = fs::File::create(&path).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to create KWin activation script: {error}"),
        )
    })?;
    file.write_all(activation_script(uuid).as_bytes())
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to write KWin activation script: {error}"),
            )
        })?;
    Ok(path)
}

fn activation_script(uuid: &str) -> String {
    format!(
        r#"const target = "{uuid}";
function normalize(value) {{
    return String(value || "").replace(/[{{}}]/g, "").toLowerCase();
}}
function candidates() {{
    if (typeof workspace.windowList === "function") {{
        return workspace.windowList();
    }}
    if (workspace.stackingOrder) {{
        return workspace.stackingOrder;
    }}
    return [];
}}
let matched = null;
const windows = candidates();
for (let i = 0; i < windows.length; i++) {{
    const window = windows[i];
    if (normalize(window.internalId) === target || normalize(window.uuid) === target) {{
        matched = window;
        break;
    }}
}}
if (matched === null) {{
    print("sky-cua: no KWin window matched " + target);
}} else {{
    if (typeof workspace.activateWindow === "function") {{
        workspace.activateWindow(matched);
    }} else {{
        workspace.activeWindow = matched;
    }}
    if (typeof workspace.raiseWindow === "function") {{
        workspace.raiseWindow(matched);
    }}
    print("sky-cua: activated " + target);
}}
"#
    )
}

fn qdbus_command() -> Option<String> {
    ["qdbus6", "qdbus"]
        .into_iter()
        .find(|binary| command_exists(binary))
        .map(str::to_string)
}

fn run_qdbus(binary: &str, args: &[&str]) -> Result<String, BackendError> {
    let output = if command_exists("timeout") {
        let mut timeout_args = vec!["2s", binary];
        timeout_args.extend_from_slice(args);
        Command::new("timeout").args(timeout_args).output()
    } else {
        Command::new(binary).args(args).output()
    }
    .map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to run KWin qdbus command: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!(
                "KWin qdbus command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn parse_script_id(output: &str) -> Option<String> {
    let id = output
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    (!id.is_empty()).then_some(id)
}

fn query_window_runner_ids(query: &str) -> Result<Vec<String>, BackendError> {
    let output = Command::new("gdbus")
        .arg("call")
        .arg("--session")
        .arg("--dest")
        .arg("org.kde.KWin")
        .arg("--object-path")
        .arg("/WindowsRunner")
        .arg("--method")
        .arg("org.kde.krunner1.Match")
        .arg(query)
        .output()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to query KWin window matches with gdbus: {error}"),
            )
        })?;

    if !output.status.success() {
        return Err(BackendError::new(
            BackendErrorCode::Internal,
            format!(
                "gdbus failed while querying KWin window matches: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }

    Ok(extract_window_ids(&String::from_utf8_lossy(&output.stdout)))
}

fn candidate_window_queries() -> Vec<String> {
    let mut candidates = HashSet::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    for entry in entries.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if file_name.parse::<u32>().is_err() {
            continue;
        }
        let cmdline_path = entry.path().join("cmdline");
        let Ok(bytes) = fs::read(cmdline_path) else {
            continue;
        };
        let arguments = bytes
            .split(|byte| *byte == 0)
            .filter_map(|segment| {
                if segment.is_empty() {
                    return None;
                }
                std::str::from_utf8(segment).ok().map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>();
        let Some(executable) = arguments
            .first()
            .and_then(|argument| std::path::Path::new(argument).file_name())
            .and_then(|file_name| file_name.to_str())
        else {
            continue;
        };
        if !plausible_window_process(executable, &arguments) {
            continue;
        }
        candidates.insert(executable.to_string());
    }

    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort();
    candidates
}

fn plausible_window_process(executable: &str, arguments: &[String]) -> bool {
    if executable.len() < 3 {
        return false;
    }
    if arguments
        .iter()
        .skip(1)
        .any(|argument| argument.starts_with("--type="))
    {
        return false;
    }
    let normalized = normalize_match_key(executable);
    if [
        "bash",
        "cargo",
        "codex",
        "dbus daemon",
        "gdbus",
        "ghostty bash sh",
        "kded6",
        "krunner",
        "kwin wayland",
        "pipewire",
        "plasmashell",
        "python",
        "python3",
        "qdbus",
        "rg",
        "rustc",
        "sed",
        "sh",
        "sky cua client",
        "sky cua service",
        "ssh",
        "sshd",
        "systemd",
        "timeout",
        "wireplumber",
        "xdg desktop portal",
        "xdg desktop portal kde",
        "zsh",
    ]
    .into_iter()
    .any(|needle| normalized == needle)
    {
        return false;
    }

    desktop_file_exists(executable)
}

fn desktop_file_exists(stem: &str) -> bool {
    let desktop_file_name = format!("{stem}.desktop");
    [
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".local/share/applications"))
            .unwrap_or_default(),
    ]
    .into_iter()
    .filter(|path| !path.as_os_str().is_empty())
    .any(|path| path.join(&desktop_file_name).is_file())
}

fn query_window_by_uuid(window_uuid: &str) -> Result<Option<KWinWindowInfo>, BackendError> {
    let output = Command::new("gdbus")
        .arg("call")
        .arg("--session")
        .arg("--dest")
        .arg("org.kde.KWin")
        .arg("--object-path")
        .arg("/KWin")
        .arg("--method")
        .arg("org.kde.KWin.getWindowInfo")
        .arg(window_uuid)
        .output()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to query the KWin window {window_uuid}: {error}"),
            )
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(parse_window_info(
        &String::from_utf8_lossy(&output.stdout),
        false,
    ))
}

fn parse_window_info(output: &str, focused: bool) -> Option<KWinWindowInfo> {
    let mut values = parse_qdbus_map(output);
    if !values.contains_key("uuid")
        && !values.contains_key("caption")
        && !values.contains_key("desktopFile")
    {
        values = parse_gdbus_map(output);
    }
    if values.is_empty() {
        return None;
    }
    if parse_bool(values.get("minimized")).unwrap_or(false) {
        return None;
    }

    let uuid = values.get("uuid").cloned();
    let desktop_file_stem = values
        .get("desktopFile")
        .cloned()
        .filter(|value| !value.trim().is_empty());
    let resource_class = values
        .get("resourceClass")
        .cloned()
        .filter(|value| !value.trim().is_empty());
    let resource_name = values
        .get("resourceName")
        .cloned()
        .filter(|value| !value.trim().is_empty());
    let caption = values
        .get("caption")
        .cloned()
        .filter(|value| !value.trim().is_empty());

    if uuid.is_none()
        && desktop_file_stem.is_none()
        && resource_class.is_none()
        && resource_name.is_none()
        && caption.is_none()
    {
        return None;
    }

    let desktop_file_id = desktop_file_stem.as_ref().map(|stem| {
        if stem.ends_with(".desktop") {
            stem.clone()
        } else {
            format!("{stem}.desktop")
        }
    });
    let name = desktop_file_stem
        .clone()
        .or_else(|| resource_class.clone())
        .or_else(|| resource_name.clone())
        .or_else(|| caption.clone())
        .unwrap_or_else(|| "Wayland Window".to_string());
    let window_id = uuid
        .clone()
        .map(|value| format!("kwin:{value}"))
        .unwrap_or_else(|| format!("kwin:{}", normalize_match_key(&name)));
    let bounds = parse_bounds(&values);
    let workspace = parse_workspace(&values);

    Some(KWinWindowInfo {
        window_id: window_id.clone(),
        resource_name,
        resource_class,
        app: AppInfo {
            app_id: window_id,
            name,
            pid: None,
            executable: desktop_file_stem.clone(),
            desktop_file_id,
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("Wayland".to_string()),
            window_title: caption,
            is_focused_candidate: focused,
        },
        bounds,
        workspace,
    })
}

fn parse_qdbus_map(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn parse_gdbus_map(output: &str) -> HashMap<String, String> {
    [
        "caption",
        "desktopFile",
        "height",
        "minimized",
        "resourceClass",
        "resourceName",
        "desktop",
        "desktops",
        "workspace",
        "uuid",
        "width",
        "x",
        "y",
    ]
    .into_iter()
    .filter_map(|key| extract_gdbus_value(output, key).map(|value| (key.to_string(), value)))
    .collect()
}

fn extract_gdbus_value(output: &str, key: &str) -> Option<String> {
    let pattern = format!("'{key}': <");
    let start = output.find(&pattern)? + pattern.len();
    let remainder = &output[start..];
    let mut quoted = false;
    for (index, character) in remainder.char_indices() {
        match character {
            '\'' => quoted = !quoted,
            '>' if !quoted => {
                return Some(remainder[..index].trim().trim_matches('\'').to_string());
            }
            _ => {}
        }
    }
    None
}

fn parse_bool(value: Option<&String>) -> Option<bool> {
    match value?.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_f64(value: Option<&String>) -> Option<f64> {
    value?.trim().parse::<f64>().ok()
}

fn parse_workspace_value(value: Option<&String>) -> Option<i32> {
    let value = value?.trim();
    if let Ok(parsed) = value.parse::<i32>() {
        return Some(parsed);
    }
    value
        .trim_matches(['[', ']'])
        .split(',')
        .map(|candidate| candidate.trim().trim_matches(['\'', '"']))
        .find_map(|candidate| candidate.parse::<i32>().ok())
}

fn parse_workspace(values: &HashMap<String, String>) -> Option<i32> {
    parse_workspace_value(values.get("workspace"))
        .or_else(|| parse_workspace_value(values.get("desktop")))
        .or_else(|| parse_workspace_value(values.get("desktops")))
}

fn parse_bounds(values: &HashMap<String, String>) -> Option<RectF> {
    Some(RectF {
        x: parse_f64(values.get("x"))?,
        y: parse_f64(values.get("y"))?,
        width: parse_f64(values.get("width"))?,
        height: parse_f64(values.get("height"))?,
        space: CoordinateSpace::DesktopLogical,
    })
}

fn extract_window_ids(output: &str) -> Vec<String> {
    let mut window_ids = Vec::new();
    let mut seen = HashSet::new();
    let bytes = output.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        let Some(relative_end) = output[index..].find('}') else {
            break;
        };
        let end = index + relative_end + 1;
        let candidate = &output[index..end];
        if is_uuid_token(candidate) && seen.insert(candidate.to_string()) {
            window_ids.push(candidate.to_string());
        }
        index = end;
    }
    window_ids
}

fn is_uuid_token(candidate: &str) -> bool {
    let Some(inner) = candidate
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    else {
        return false;
    };
    let segments = inner.split('-').collect::<Vec<_>>();
    if segments.len() != 5 {
        return false;
    }
    let expected_lengths = [8usize, 4, 4, 4, 12];
    segments
        .into_iter()
        .zip(expected_lengths)
        .all(|(segment, expected_len)| {
            segment.len() == expected_len
                && segment
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
        })
}

fn command_exists(binary: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|path| path.join(binary).is_file()))
}

fn normalize_match_key(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{extract_window_ids, parse_window_info};

    #[test]
    fn parses_kwin_window_info() {
        let parsed = parse_window_info(
            "caption: TIDAL Hi-Fi\n\
             desktopFile: tidal-hifi\n\
             height: 999\n\
             minimized: false\n\
             resourceClass: tidal-hifi\n\
             resourceName: tidal-hifi\n\
             workspace: 2\n\
             uuid: {71afaa3f-26ba-47a3-a07b-abc3ce9a4296}\n\
             width: 1383\n\
             x: 61\n\
             y: 276\n",
            false,
        )
        .expect("expected a parsed window");

        assert_eq!(
            parsed.window_id,
            "kwin:{71afaa3f-26ba-47a3-a07b-abc3ce9a4296}"
        );
        assert_eq!(
            parsed.app.desktop_file_id.as_deref(),
            Some("tidal-hifi.desktop")
        );
        assert_eq!(parsed.app.window_title.as_deref(), Some("TIDAL Hi-Fi"));
        assert_eq!(parsed.app.name, "tidal-hifi");
        assert_eq!(parsed.workspace, Some(2));
        assert!(!parsed.app.is_focused_candidate);
        let bounds = parsed.bounds.expect("expected bounds");
        assert_eq!(bounds.width, 1383.0);
        assert_eq!(bounds.height, 999.0);
    }

    #[test]
    fn ignores_minimized_window() {
        let parsed = parse_window_info(
            "caption: Hidden\n\
             minimized: true\n\
             width: 100\n\
             height: 100\n\
             x: 0\n\
             y: 0\n",
            false,
        );
        assert!(parsed.is_none());
    }

    #[test]
    fn extracts_uuid_tokens_from_windows_runner_output() {
        let output = "([('0_{71afaa3f-26ba-47a3-a07b-abc3ce9a4296}', 'TIDAL Hi-Fi', '', 100, 0.8, {'subtext': <'Activate running window on Desktop 1'>}), ('0_{41f4ba34-69c7-4a96-88f2-3464922409c3}', 'htop', '', 30, 0.7, {'subtext': <'Activate running window on Desktop 1'>})],)";
        let ids = extract_window_ids(output);
        assert_eq!(
            ids,
            vec![
                "{71afaa3f-26ba-47a3-a07b-abc3ce9a4296}".to_string(),
                "{41f4ba34-69c7-4a96-88f2-3464922409c3}".to_string()
            ]
        );
    }

    #[test]
    fn parses_gdbus_window_info() {
        let parsed = parse_window_info(
            "({'caption': <'TIDAL Hi-Fi'>, 'desktopFile': <'tidal-hifi'>, 'height': <999.0>, 'minimized': <false>, 'resourceClass': <'tidal-hifi'>, 'resourceName': <'tidal-hifi'>, 'desktops': <['2']>, 'uuid': <'{71afaa3f-26ba-47a3-a07b-abc3ce9a4296}'>, 'width': <1383.0>, 'x': <61.0>, 'y': <276.0>},)",
            false,
        )
        .expect("expected a parsed gdbus window");

        assert_eq!(
            parsed.window_id,
            "kwin:{71afaa3f-26ba-47a3-a07b-abc3ce9a4296}"
        );
        assert_eq!(
            parsed.app.desktop_file_id.as_deref(),
            Some("tidal-hifi.desktop")
        );
        assert_eq!(parsed.app.window_title.as_deref(), Some("TIDAL Hi-Fi"));
        assert_eq!(parsed.workspace, Some(2));
        let bounds = parsed.bounds.expect("expected bounds");
        assert_eq!(bounds.width, 1383.0);
        assert_eq!(bounds.height, 999.0);
    }

    #[test]
    fn ignores_non_numeric_kwin_workspace_values() {
        let parsed = parse_window_info(
            "caption: TIDAL Hi-Fi\n\
             desktopFile: tidal-hifi\n\
             minimized: false\n\
             resourceClass: tidal-hifi\n\
             resourceName: tidal-hifi\n\
             desktops: [workspace-two]\n\
             uuid: {71afaa3f-26ba-47a3-a07b-abc3ce9a4296}\n\
             width: 1383\n\
             height: 999\n\
             x: 61\n\
             y: 276\n",
            false,
        )
        .expect("expected a parsed window");

        assert_eq!(parsed.workspace, None);
    }
}
