use std::env;
use std::process::Command;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X11MouseButton {
    Left,
    Middle,
    Right,
}

pub fn xtest_is_available() -> bool {
    env::var("DISPLAY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .is_some()
        && command_exists("xdotool")
}

pub fn pointer_move_absolute(x: f64, y: f64) -> Result<(), BackendError> {
    run_xdotool([
        "mousemove",
        "--sync",
        &round_coordinate(x),
        &round_coordinate(y),
    ])
}

pub fn pointer_button(button: X11MouseButton, pressed: bool) -> Result<(), BackendError> {
    let action = if pressed { "mousedown" } else { "mouseup" };
    run_xdotool([action, xdotool_button(button)])
}

pub fn click(button: X11MouseButton) -> Result<(), BackendError> {
    run_xdotool(["click", xdotool_button(button)])
}

pub fn key_state(key: &str, pressed: bool) -> Result<(), BackendError> {
    let action = if pressed { "keydown" } else { "keyup" };
    run_xdotool([action, &normalize_key_name(key)])
}

pub fn window_activate(window_id: &str) -> Result<(), BackendError> {
    run_xdotool(["windowactivate", "--sync", window_id])
}

pub fn scroll_vertical(delta_y: Option<f64>, steps: Option<i32>) -> Result<(), BackendError> {
    let signed_steps = if let Some(delta_y) = delta_y {
        if delta_y == 0.0 {
            0
        } else {
            let magnitude = (delta_y.abs() / 120.0).ceil().max(1.0) as i32;
            if delta_y.is_sign_positive() {
                magnitude
            } else {
                -magnitude
            }
        }
    } else {
        steps.unwrap_or(-1)
    };

    if signed_steps == 0 {
        return Ok(());
    }

    let button = if signed_steps > 0 { "4" } else { "5" };
    run_xdotool(["click", "--repeat", &signed_steps.abs().to_string(), button])
}

pub fn scroll_horizontal(delta_x: Option<f64>, steps: Option<i32>) -> Result<(), BackendError> {
    let signed_steps = if let Some(delta_x) = delta_x {
        if delta_x == 0.0 {
            0
        } else {
            let magnitude = (delta_x.abs() / 120.0).ceil().max(1.0) as i32;
            if delta_x.is_sign_positive() {
                magnitude
            } else {
                -magnitude
            }
        }
    } else {
        steps.unwrap_or(-1)
    };

    if signed_steps == 0 {
        return Ok(());
    }

    let button = horizontal_scroll_button(signed_steps);
    run_xdotool(["click", "--repeat", &signed_steps.abs().to_string(), button])
}

/// X11 core button convention: 6 scrolls left and 7 scrolls right.
pub(crate) fn horizontal_scroll_button(signed_steps: i32) -> &'static str {
    if signed_steps > 0 { "7" } else { "6" }
}

pub fn send_text(text: &str) -> Result<(), BackendError> {
    send_text_to_target(None, text)
}

pub fn send_text_to_window(window_id: &str, text: &str) -> Result<(), BackendError> {
    send_text_to_target(Some(window_id), text)
}

pub fn send_text_to_target(window_id: Option<&str>, text: &str) -> Result<(), BackendError> {
    send_text_inner(window_id, text)
}

pub fn press_key_sequence(keys: &[String]) -> Result<(), BackendError> {
    press_key_sequence_to_target(None, keys)
}

pub fn press_key_sequence_to_window(window_id: &str, keys: &[String]) -> Result<(), BackendError> {
    press_key_sequence_to_target(Some(window_id), keys)
}

pub fn press_key_sequence_to_target(
    window_id: Option<&str>,
    keys: &[String],
) -> Result<(), BackendError> {
    if keys.is_empty() {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            "press_key requires at least one key",
        ));
    }
    let chord = keys
        .iter()
        .map(|key| normalize_key_name(key))
        .collect::<Vec<_>>()
        .join("+");
    match window_id {
        Some(window_id) => run_xdotool(["key", "--window", window_id, "--clearmodifiers", &chord]),
        None => run_xdotool(["key", "--clearmodifiers", &chord]),
    }
}

fn send_text_inner(window_id: Option<&str>, text: &str) -> Result<(), BackendError> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut current = String::new();
    for ch in normalized.chars() {
        if ch == '\n' {
            if !current.is_empty() {
                type_text_chunk(window_id, &current)?;
                current.clear();
            }
            press_return(window_id)?;
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        type_text_chunk(window_id, &current)?;
    }

    Ok(())
}

fn type_text_chunk(window_id: Option<&str>, text: &str) -> Result<(), BackendError> {
    match window_id {
        Some(window_id) => run_xdotool([
            "type",
            "--window",
            window_id,
            "--delay",
            "1",
            "--clearmodifiers",
            "--",
            text,
        ]),
        None => run_xdotool(["type", "--delay", "1", "--clearmodifiers", "--", text]),
    }
}

fn press_return(window_id: Option<&str>) -> Result<(), BackendError> {
    match window_id {
        Some(window_id) => {
            run_xdotool(["key", "--window", window_id, "--clearmodifiers", "Return"])
        }
        None => run_xdotool(["key", "--clearmodifiers", "Return"]),
    }
}

fn run_xdotool(arguments: impl IntoIterator<Item = impl AsRef<str>>) -> Result<(), BackendError> {
    let args = arguments
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .collect::<Vec<_>>();
    let output = Command::new("xdotool")
        .args(&args)
        .output()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to launch xdotool for X11 input injection: {error}"),
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        if stderr.is_empty() {
            format!(
                "xdotool exited with status {} while injecting X11 input",
                output.status
            )
        } else {
            format!("xdotool failed while injecting X11 input: {stderr}")
        },
    ))
}

fn xdotool_button(button: X11MouseButton) -> &'static str {
    match button {
        X11MouseButton::Left => "1",
        X11MouseButton::Middle => "2",
        X11MouseButton::Right => "3",
    }
}

fn round_coordinate(value: f64) -> String {
    ((value.round().clamp(0.0, f64::from(i32::MAX))) as i32).to_string()
}

fn normalize_key_name(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "ctrl".to_string(),
        "enter" => "Return".to_string(),
        "esc" => "Escape".to_string(),
        "alt" => "Alt".to_string(),
        "shift" => "Shift".to_string(),
        "super" | "meta" | "cmd" | "command" => "Super_L".to_string(),
        other if other.len() == 1 => other.to_string(),
        _ => key.to_string(),
    }
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
        X11MouseButton, horizontal_scroll_button, normalize_key_name, round_coordinate,
        xdotool_button,
    };

    #[test]
    fn maps_mouse_buttons() {
        assert_eq!(xdotool_button(X11MouseButton::Left), "1");
        assert_eq!(xdotool_button(X11MouseButton::Right), "3");
    }

    #[test]
    fn maps_horizontal_scroll_sign_to_x11_buttons() {
        assert_eq!(horizontal_scroll_button(-1), "6");
        assert_eq!(horizontal_scroll_button(1), "7");
    }

    #[test]
    fn normalizes_common_key_names() {
        assert_eq!(normalize_key_name("Ctrl"), "ctrl");
        assert_eq!(normalize_key_name("Enter"), "Return");
        assert_eq!(normalize_key_name("a"), "a");
    }

    #[test]
    fn rounds_coordinates_for_xdotool() {
        assert_eq!(round_coordinate(100.4), "100");
        assert_eq!(round_coordinate(100.5), "101");
    }
}
