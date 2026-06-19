//! scrcpy binary resolution and version probing.
//!
//! Resolution precedence is the `SKY_CUA_SCRCPY` env override *over* config
//! (`PhoneConfig.scrcpy_path`) — both already folded into
//! [`ResolvedPhoneSelection::scrcpy_path`] by the config resolver, where env
//! wins — then `PATH`. A configured-but-missing path is an honest
//! [`ScrcpyResolution::Missing`] so the operator learns their override is wrong
//! rather than silently falling back to a different binary.

use sky_cua_platform::config::ResolvedPhoneSelection;

use crate::phone::command::CommandRunner;

/// Outcome of resolving the scrcpy binary: either a concrete path or a
/// structured reason it is unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) enum ScrcpyResolution {
    /// scrcpy was found at this path (configured/env path or a `PATH` hit).
    Found { path: String },
    /// scrcpy could not be resolved; `reason` is a short, structured message
    /// suitable for [`sky_cua_platform::model::PhoneScrcpyCapabilities::reason`].
    Missing { reason: String },
}

impl ScrcpyResolution {
    /// The resolved path, if scrcpy was found.
    pub(in crate::phone) fn path(&self) -> Option<&str> {
        match self {
            ScrcpyResolution::Found { path } => Some(path.as_str()),
            ScrcpyResolution::Missing { .. } => None,
        }
    }
}

/// Resolve scrcpy from the resolved selection, falling back to a `PATH` lookup.
///
/// Precedence is the `SKY_CUA_SCRCPY` env override over config, then `PATH`. The
/// env-over-config layering is already done in
/// [`ResolvedPhoneSelection::scrcpy_path`] by the config resolver (env wins), so
/// this only adds the `PATH` fallback and validates that a configured path
/// actually exists. A configured-but-missing path is an honest `Missing` so the
/// operator learns their override is wrong instead of silently falling back to a
/// different binary.
pub(in crate::phone) fn resolve_scrcpy(selection: &ResolvedPhoneSelection) -> ScrcpyResolution {
    if let Some(configured) = selection.scrcpy_path.as_deref() {
        return if is_existing_file(configured) {
            ScrcpyResolution::Found {
                path: configured.to_string(),
            }
        } else {
            ScrcpyResolution::Missing {
                reason: format!("configured scrcpy path does not exist: {configured}"),
            }
        };
    }
    match find_on_path("scrcpy") {
        Some(path) => ScrcpyResolution::Found { path },
        None => ScrcpyResolution::Missing {
            reason: "scrcpy not found on PATH and no scrcpy_path configured".to_string(),
        },
    }
}

/// Whether `candidate` names an existing regular file. Split out so geometry and
/// builder tests stay filesystem-free.
fn is_existing_file(candidate: &str) -> bool {
    std::path::Path::new(candidate).is_file()
}

/// First `PATH` entry containing an executable named `binary`. Mirrors the
/// `command_path` idiom used elsewhere in the workspace.
fn find_on_path(binary: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|entry| entry.join(binary))
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

/// Probe `scrcpy --version` through the command seam, returning the trimmed
/// version line when it succeeds. Used to populate
/// [`sky_cua_platform::model::PhoneScrcpyCapabilities::version`]. Never panics on
/// failure; returns `None`.
pub(in crate::phone) async fn probe_version(
    runner: &dyn CommandRunner,
    scrcpy_path: &str,
) -> Option<String> {
    let output = runner.run(scrcpy_path, &["--version"]).await.ok()?;
    if output.status != Some(0) {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_scrcpy_version(&text)
}

/// Extract a `scrcpy X.Y[.Z]` version from `--version` output. scrcpy prints
/// `scrcpy 4.0 <https://...>` on the first line.
fn parse_scrcpy_version(text: &str) -> Option<String> {
    let first = text.lines().next()?.trim();
    let rest = first.strip_prefix("scrcpy").unwrap_or(first).trim();
    let token = rest.split_whitespace().next()?;
    let version: String = token
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (!version.is_empty()).then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::config::PhoneConfig;

    #[test]
    fn resolve_scrcpy_reports_missing_when_configured_path_absent() {
        let mut selection =
            sky_cua_platform::config::resolve_phone_selection(&PhoneConfig::default());
        selection.scrcpy_path = Some("/nonexistent/scrcpy-binary".to_string());
        match resolve_scrcpy(&selection) {
            ScrcpyResolution::Missing { reason } => {
                assert!(reason.contains("/nonexistent/scrcpy-binary"));
            }
            ScrcpyResolution::Found { .. } => {
                panic!("configured-but-missing path must not resolve")
            }
        }
    }

    #[test]
    fn parse_scrcpy_version_extracts_semver() {
        assert_eq!(
            parse_scrcpy_version("scrcpy 4.0 <https://github.com/Genymobile/scrcpy>"),
            Some("4.0".to_string())
        );
        assert_eq!(
            parse_scrcpy_version("scrcpy 2.7\nINFO: foo"),
            Some("2.7".to_string())
        );
        assert_eq!(parse_scrcpy_version(""), None);
    }
}
