use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use sky_cua_platform::diagnostics::BackendError;
use sky_cua_platform::model::{
    AccessibilitySetupReport, DoctorReport, SetupCommandReport, WindowInfo,
    WindowTargetingSetupReport,
};

use crate::doctor::{self, format_command_output};
use crate::session_env;
use crate::windowing::gnome_extension;

pub const GNOME_EXTENSION_UUID: &str = "codex-window-control@openai.com";
const METADATA_JSON: &str = include_str!(
    "../../../resources/gnome-shell-extension/codex-window-control@openai.com/metadata.json"
);
const EXTENSION_JS: &str = include_str!(
    "../../../resources/gnome-shell-extension/codex-window-control@openai.com/extension.js"
);
const CURSOR_CHAT_PNG: &[u8] = include_bytes!("../../sky-cua-overlay-host/assets/cursor-chat.png");

pub async fn setup_accessibility_report<F, Fut>(
    doctor_fn: F,
) -> Result<AccessibilitySetupReport, BackendError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<DoctorReport, BackendError>>,
{
    let _ = session_env::hydrate_session_env();

    let before = doctor_fn().await?;
    let before_accessibility = before.accessibility.as_ref();
    let was_ready = before_accessibility.is_some_and(doctor::can_build_accessibility_tree);
    let accessibility_command = if was_ready {
        SetupCommandReport {
            ok: true,
            detail: "GNOME accessibility is already enabled".to_string(),
        }
    } else {
        run_gsettings_toolkit_accessibility()
    };
    let after = doctor_fn().await?;
    let after_accessibility = after.accessibility.as_ref();
    let is_ready = after_accessibility.is_some_and(doctor::can_build_accessibility_tree);
    let changed = !was_ready && is_ready;
    Ok(AccessibilitySetupReport {
        before: Box::new(before),
        accessibility_command,
        after: Box::new(after),
        changed,
        requires_restart: changed,
    })
}

pub async fn setup_window_targeting_report() -> WindowTargetingSetupReport {
    let _ = session_env::hydrate_session_env();

    let extension_dir = extension_dir();
    let mut wrote_files = false;
    let mut write_error = None;
    match write_extension_files(&extension_dir) {
        Ok(()) => wrote_files = true,
        Err(error) => write_error = Some(error),
    }
    let enable_command = if let Some(error) = &write_error {
        SetupCommandReport {
            ok: false,
            detail: format!("extension file write failed: {error}"),
        }
    } else {
        run_gnome_extensions_enable()
    };
    let (windows, windows_error) = match gnome_extension::list_windows().await {
        Ok(windows) => (windows.into_iter().map(WindowInfo::from).collect(), None),
        Err(error) => (Vec::new(), Some(format!("{error:#}"))),
    };
    let extension_available = windows_error.is_none();
    let requires_shell_reload = wrote_files && enable_command.ok && !extension_available;
    let message =
        setup_window_targeting_message(wrote_files, enable_command.ok, extension_available);
    let permissions_hint = windows_error
        .as_ref()
        .map(|_| crate::windowing::registry::WINDOW_PERMISSION_HINT.to_string());
    WindowTargetingSetupReport {
        extension_dir: extension_dir.display().to_string(),
        wrote_files,
        enable_command,
        windows,
        windows_error,
        requires_shell_reload,
        message,
        permissions_hint,
    }
}

fn setup_window_targeting_message(
    wrote_files: bool,
    enable_ok: bool,
    extension_available: bool,
) -> String {
    if extension_available {
        "GNOME Shell extension is installed and exact window targeting is available.".to_string()
    } else if !wrote_files {
        "Could not install the GNOME Shell extension files.".to_string()
    } else if !enable_ok {
        "GNOME Shell extension files were installed, but enabling the extension failed.".to_string()
    } else {
        "GNOME Shell extension files were installed and enable was requested, but GNOME Shell may need a reload or login restart before serving the DBus API.".to_string()
    }
}

fn run_gsettings_toolkit_accessibility() -> SetupCommandReport {
    let mut command = Command::new("gsettings");
    command.args([
        "set",
        "org.gnome.desktop.interface",
        "toolkit-accessibility",
        "true",
    ]);
    add_session_env(&mut command);
    match command.output() {
        Ok(output) if output.status.success() => SetupCommandReport {
            ok: true,
            detail: format_command_output(
                &output.stdout,
                &output.stderr,
                "toolkit-accessibility set to true",
            ),
        },
        Ok(output) => SetupCommandReport {
            ok: false,
            detail: format_command_output(&output.stdout, &output.stderr, "gsettings set failed"),
        },
        Err(error) => SetupCommandReport {
            ok: false,
            detail: format!("failed to run gsettings: {error}"),
        },
    }
}

fn write_extension_files(extension_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(extension_dir)
        .map_err(|error| format!("failed to create {}: {error}", extension_dir.display()))?;
    fs::write(extension_dir.join("metadata.json"), METADATA_JSON).map_err(|error| {
        format!(
            "failed to write {}: {error}",
            extension_dir.join("metadata.json").display()
        )
    })?;
    fs::write(extension_dir.join("extension.js"), EXTENSION_JS).map_err(|error| {
        format!(
            "failed to write {}: {error}",
            extension_dir.join("extension.js").display()
        )
    })?;
    fs::write(extension_dir.join("cursor-chat.png"), CURSOR_CHAT_PNG).map_err(|error| {
        format!(
            "failed to write {}: {error}",
            extension_dir.join("cursor-chat.png").display()
        )
    })?;
    Ok(())
}

fn run_gnome_extensions_enable() -> SetupCommandReport {
    let mut command = Command::new("gnome-extensions");
    command.args(["enable", GNOME_EXTENSION_UUID]);
    add_session_env(&mut command);
    let primary = match command.output() {
        Ok(output) if output.status.success() => SetupCommandReport {
            ok: true,
            detail: format_command_output(
                &output.stdout,
                &output.stderr,
                "gnome-extensions enable ok",
            ),
        },
        Ok(output) => SetupCommandReport {
            ok: false,
            detail: format_command_output(
                &output.stdout,
                &output.stderr,
                &format!("gnome-extensions exited with {}", output.status),
            ),
        },
        Err(error) => SetupCommandReport {
            ok: false,
            detail: format!("failed to run gnome-extensions: {error}"),
        },
    };
    if primary.ok {
        return primary;
    }
    let fallback = run_gsettings_enable_fallback();
    if fallback.ok {
        SetupCommandReport {
            ok: true,
            detail: format!(
                "gnome-extensions enable failed: {}; {}",
                primary.detail, fallback.detail
            ),
        }
    } else {
        SetupCommandReport {
            ok: false,
            detail: format!(
                "gnome-extensions enable failed: {}; gsettings fallback failed: {}",
                primary.detail, fallback.detail
            ),
        }
    }
}

fn run_gsettings_enable_fallback() -> SetupCommandReport {
    let mut get_command = Command::new("gsettings");
    get_command.args(["get", "org.gnome.shell", "enabled-extensions"]);
    add_session_env(&mut get_command);
    let current = match get_command.output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        Ok(output) => {
            return SetupCommandReport {
                ok: false,
                detail: format_command_output(
                    &output.stdout,
                    &output.stderr,
                    "gsettings get failed",
                ),
            };
        }
        Err(error) => {
            return SetupCommandReport {
                ok: false,
                detail: format!("failed to run gsettings get: {error}"),
            };
        }
    };
    let Some(updated) = enabled_extensions_literal(&current) else {
        return SetupCommandReport {
            ok: false,
            detail: format!("could not parse enabled-extensions value: {current}"),
        };
    };
    if updated == current {
        return SetupCommandReport {
            ok: true,
            detail: format!(
                "{GNOME_EXTENSION_UUID} already present in org.gnome.shell enabled-extensions"
            ),
        };
    }
    let mut set_command = Command::new("gsettings");
    set_command.args(["set", "org.gnome.shell", "enabled-extensions", &updated]);
    add_session_env(&mut set_command);
    match set_command.output() {
        Ok(output) if output.status.success() => SetupCommandReport {
            ok: true,
            detail: format!(
                "added {GNOME_EXTENSION_UUID} to org.gnome.shell enabled-extensions for the next GNOME Shell load"
            ),
        },
        Ok(output) => SetupCommandReport {
            ok: false,
            detail: format_command_output(&output.stdout, &output.stderr, "gsettings set failed"),
        },
        Err(error) => SetupCommandReport {
            ok: false,
            detail: format!("failed to run gsettings set: {error}"),
        },
    }
}

fn enabled_extensions_literal(current: &str) -> Option<String> {
    let trimmed = current.trim();
    let quoted = format!("'{GNOME_EXTENSION_UUID}'");
    if trimmed.contains(&quoted) {
        return Some(trimmed.to_string());
    }
    let list = if trimmed == "@as []" { "[]" } else { trimmed };
    if list == "[]" {
        return Some(format!("[{quoted}]"));
    }
    let prefix = list.strip_suffix(']')?;
    Some(format!("{prefix}, {quoted}]"))
}

fn extension_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local/share/gnome-shell/extensions")
        .join(GNOME_EXTENSION_UUID)
}

fn add_session_env(command: &mut Command) {
    if let Some(address) = session_env::dbus_session_address() {
        command.env("DBUS_SESSION_BUS_ADDRESS", address);
    }
    if let Some(runtime) = session_env::xdg_runtime_dir() {
        command.env("XDG_RUNTIME_DIR", runtime);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::{
        DoctorAccessibilityReport, DoctorCheck, DoctorInputReport, DoctorPortalReport,
        DoctorReadiness, DoctorReport, DoctorWindowingReport, EnvironmentInfo,
    };

    fn doctor_report_with_accessibility(accessibility: DoctorAccessibilityReport) -> DoctorReport {
        DoctorReport {
            environment: EnvironmentInfo {
                session_kind: sky_cua_platform::model::SessionKind::Wayland,
                compositor: Some("kde-kwin-wayland".to_string()),
                desktop_environment: Some("KDE".to_string()),
                capture_backend: sky_cua_platform::model::CaptureBackendKind::PortalPipeWire,
                input_backend: sky_cua_platform::model::InputBackendKind::PortalRemoteDesktop,
                semantic_backend: sky_cua_platform::model::SemanticBackendKind::Atspi,
                portal_capabilities: sky_cua_platform::model::PortalCapabilities {
                    screencast_version: Some(5),
                    remote_desktop_version: Some(2),
                    screenshot_version: Some(1),
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: Some("wayland".to_string()),
                display: None,
                wayland_display: Some("wayland-0".to_string()),
            },
            checks: vec![],
            readiness: DoctorReadiness {
                can_register_mcp_tools: true,
                can_build_accessibility_tree: true,
                can_capture_screen: true,
                can_send_input: true,
                can_list_windows: true,
                can_target_windows: true,
                can_inhibit_presence: false,
                can_unlock_session: false,
                recommended_next_step: "ready".to_string(),
                blockers: vec![],
            },
            platform: None,
            session_env: None,
            portal: Some(DoctorPortalReport {
                screencast_version: Some(5),
                remote_desktop_version: Some(2),
                screenshot_version: Some(1),
                input_capture_version: None,
                detail: String::new(),
            }),
            accessibility: Some(accessibility),
            windowing: Some(DoctorWindowingReport {
                probes: vec![],
                can_list_windows: true,
                can_focus_windows: true,
                detail: String::new(),
                note: String::new(),
            }),
            input: Some(DoctorInputReport {
                backend: sky_cua_platform::model::InputBackendKind::PortalRemoteDesktop,
                ydotool: DoctorCheck {
                    name: "ydotool".to_string(),
                    ok: false,
                    detail: String::new(),
                },
                ydotoold: DoctorCheck {
                    name: "ydotoold".to_string(),
                    ok: false,
                    detail: String::new(),
                },
                ydotool_socket: DoctorCheck {
                    name: "ydotool_socket".to_string(),
                    ok: false,
                    detail: String::new(),
                },
                xdotool: DoctorCheck {
                    name: "xdotool".to_string(),
                    ok: false,
                    detail: String::new(),
                },
                uinput: DoctorCheck {
                    name: "uinput".to_string(),
                    ok: false,
                    detail: String::new(),
                },
            }),
            browser_integration: None,
            session_presence: None,
        }
    }

    fn accessibility_ready() -> DoctorAccessibilityReport {
        DoctorAccessibilityReport {
            atspi_bus: DoctorCheck {
                name: "atspi_bus".to_string(),
                ok: true,
                detail: "ok".to_string(),
            },
            toolkit_accessibility: DoctorCheck {
                name: "toolkit_accessibility".to_string(),
                ok: true,
                detail: "true".to_string(),
            },
            at_spi_enabled: DoctorCheck {
                name: "at_spi_enabled".to_string(),
                ok: true,
                detail: "(<true>,)".to_string(),
            },
            screen_reader: DoctorCheck {
                name: "screen_reader".to_string(),
                ok: false,
                detail: "(<false>,)".to_string(),
            },
        }
    }

    fn accessibility_not_ready() -> DoctorAccessibilityReport {
        DoctorAccessibilityReport {
            atspi_bus: DoctorCheck {
                name: "atspi_bus".to_string(),
                ok: true,
                detail: "ok".to_string(),
            },
            toolkit_accessibility: DoctorCheck {
                name: "toolkit_accessibility".to_string(),
                ok: false,
                detail: "false".to_string(),
            },
            at_spi_enabled: DoctorCheck {
                name: "at_spi_enabled".to_string(),
                ok: false,
                detail: "(<false>,)".to_string(),
            },
            screen_reader: DoctorCheck {
                name: "screen_reader".to_string(),
                ok: false,
                detail: "(<false>,)".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn setup_accessibility_skips_gsettings_when_already_enabled() {
        let report = setup_accessibility_report(|| async {
            Ok(doctor_report_with_accessibility(accessibility_ready()))
        })
        .await
        .expect("setup should succeed");

        assert!(
            report.accessibility_command.ok,
            "command should report ok when skipped"
        );
        assert_eq!(
            report.accessibility_command.detail,
            "GNOME accessibility is already enabled"
        );
        assert!(!report.changed);
        assert!(!report.requires_restart);
    }

    #[tokio::test]
    async fn setup_accessibility_runs_gsettings_when_not_enabled() {
        let report = setup_accessibility_report(|| async {
            Ok(doctor_report_with_accessibility(accessibility_not_ready()))
        })
        .await
        .expect("setup should succeed");

        // In a non-GNOME test environment, gsettings set will likely fail.
        // The important thing is that we attempted it rather than skipping.
        assert!(
            report.accessibility_command.detail != "GNOME accessibility is already enabled",
            "should attempt gsettings when not already enabled"
        );
    }

    #[test]
    fn enabled_extensions_literal_adds_uuid_to_existing_list() {
        assert_eq!(
            enabled_extensions_literal("['ubuntu-dock@ubuntu.com']").unwrap(),
            "['ubuntu-dock@ubuntu.com', 'codex-window-control@openai.com']"
        );
    }

    #[test]
    fn enabled_extensions_literal_handles_empty_typed_array() {
        assert_eq!(
            enabled_extensions_literal("@as []").unwrap(),
            "['codex-window-control@openai.com']"
        );
    }

    #[test]
    fn setup_window_targeting_message_prefers_available_extension_over_enable_failure() {
        assert_eq!(
            setup_window_targeting_message(true, false, true),
            "GNOME Shell extension is installed and exact window targeting is available."
        );
        assert_eq!(
            setup_window_targeting_message(false, false, true),
            "GNOME Shell extension is installed and exact window targeting is available."
        );
    }

    #[test]
    fn setup_window_targeting_message_reports_reload_only_after_successful_enable() {
        assert_eq!(
            setup_window_targeting_message(true, true, false),
            "GNOME Shell extension files were installed and enable was requested, but GNOME Shell may need a reload or login restart before serving the DBus API."
        );
        assert_eq!(
            setup_window_targeting_message(true, false, false),
            "GNOME Shell extension files were installed, but enabling the extension failed."
        );
    }

    #[test]
    fn bundled_gnome_extension_includes_agent_cursor_asset() {
        assert!(EXTENSION_JS.contains("SetAgentCursorState"));
        assert!(EXTENSION_JS.contains("cursor-chat.png"));
        assert!(CURSOR_CHAT_PNG.len() > 100);
    }
}
