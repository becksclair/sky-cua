//! Machine-level sky-cua configuration.
//!
//! Per-machine settings live in one TOML file instead of being baked into
//! every MCP host registration's environment:
//!
//! - Linux/macOS: `$XDG_CONFIG_HOME/sky-cua/sky-cua.toml`
//!   (default `~/.config/sky-cua/sky-cua.toml`)
//! - Windows: `%APPDATA%\sky-cua\sky-cua.toml`
//!
//! Environment variables remain per-process overrides on top of the file
//! (e.g. `SKY_CUA_BROWSER` beats the file's `browser`). The file is read on
//! use, not cached: a daemon restart is never required just to observe a
//! changed value, and tests can point `SKY_CUA_CONFIG_PATH` at fixtures.

use std::path::PathBuf;

use serde::Deserialize;

/// Test/operator override for the machine config file location.
pub const MACHINE_CONFIG_PATH_ENV: &str = "SKY_CUA_CONFIG_PATH";

pub const BROWSER_SELECTION_ENV: &str = "SKY_CUA_BROWSER";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MachineConfig {
    /// Chrome-family browser selection: `brave`, `chrome`, `chromium`, or
    /// `all`. Unset means probe every Chrome-family browser.
    pub browser: Option<String>,
}

/// Resolve the machine config path for this platform, or None when no base
/// directory can be determined.
pub fn machine_config_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(MACHINE_CONFIG_PATH_ENV) {
        if explicit.is_empty() {
            return None;
        }
        return Some(PathBuf::from(explicit));
    }
    machine_config_base_dir().map(|base| base.join("sky-cua").join("sky-cua.toml"))
}

fn machine_config_base_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(PathBuf::from);
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(xdg));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

/// Load the machine config. A missing file is the empty config; an unreadable
/// or unparseable file is an error so callers can surface an honest
/// diagnostic instead of silently ignoring operator configuration.
pub fn load_machine_config() -> Result<MachineConfig, String> {
    let Some(path) = machine_config_path() else {
        return Ok(MachineConfig::default());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MachineConfig::default());
        }
        Err(error) => {
            return Err(format!(
                "failed to read machine config {}: {error}",
                path.display()
            ));
        }
    };
    parse_machine_config(&text)
        .map_err(|error| format!("invalid machine config {}: {error}", path.display()))
}

fn parse_machine_config(text: &str) -> Result<MachineConfig, toml::de::Error> {
    toml::from_str(text)
}

/// Effective browser selection: the `SKY_CUA_BROWSER` environment override
/// beats the machine config's `browser`; both unset means no selection.
/// Config-file errors are returned so the caller can attach a diagnostic.
pub fn resolved_browser_selection() -> Result<Option<String>, String> {
    if let Some(env_value) = browser_selection_env_value() {
        return Ok(Some(env_value));
    }
    Ok(load_machine_config()?
        .browser
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn browser_selection_env_value() -> Option<String> {
    std::env::var(BROWSER_SELECTION_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_empty_config() {
        assert_eq!(parse_machine_config(""), Ok(MachineConfig::default()));
    }

    #[test]
    fn parses_browser_selection() {
        let config = parse_machine_config("browser = \"brave\"\n").expect("valid config");
        assert_eq!(config.browser.as_deref(), Some("brave"));
    }

    #[test]
    fn unknown_keys_are_tolerated_for_forward_compatibility() {
        let config =
            parse_machine_config("browser = \"chrome\"\nfuture_knob = 3\n").expect("valid config");
        assert_eq!(config.browser.as_deref(), Some("chrome"));
    }

    #[test]
    fn invalid_toml_is_an_error_not_a_silent_default() {
        assert!(parse_machine_config("browser = ").is_err());
    }
}
