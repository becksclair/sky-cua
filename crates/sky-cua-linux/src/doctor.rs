use std::{env, fs, path::Path, path::PathBuf, process::Command};

use sky_cua_platform::model::{
    BrowserIntegrationReport, DoctorAccessibilityReport, DoctorCheck, DoctorInputReport,
    DoctorPlatformReport, DoctorPortalReport, DoctorReadiness, DoctorReport,
    DoctorSessionEnvReport, DoctorWindowingReport, EnvironmentInfo, InputBackendKind,
    WindowBackendProbe,
};

use crate::{session_env, windowing};

pub fn build_doctor_report(
    environment: EnvironmentInfo,
    session_env_report: DoctorSessionEnvReport,
) -> DoctorReport {
    let window_probes = windowing::probe_backends(&environment);
    let can_list_windows = window_probes.iter().any(|probe| probe.can_list_windows);
    let can_target_windows = window_probes.iter().any(|probe| probe.can_focus_windows);
    let accessibility = accessibility_report();
    let input = input_report(environment.input_backend.clone());
    let portal = portal_report(&environment);
    let browser_integration = browser_report();

    let mut checks = vec![
        DoctorCheck {
            name: "semantic_backend".to_string(),
            ok: environment.semantic_backend != sky_cua_platform::model::SemanticBackendKind::None,
            detail: format!("{:?}", environment.semantic_backend),
        },
        DoctorCheck {
            name: "capture_backend".to_string(),
            ok: environment.capture_backend != sky_cua_platform::model::CaptureBackendKind::None,
            detail: format!("{:?}", environment.capture_backend),
        },
        DoctorCheck {
            name: "input_backend".to_string(),
            ok: environment.input_backend != InputBackendKind::None,
            detail: format!("{:?}", environment.input_backend),
        },
        DoctorCheck {
            name: "windowing_backend".to_string(),
            ok: can_list_windows,
            detail: window_probes
                .iter()
                .map(|probe| format!("{}={}", probe.id, probe.ok))
                .collect::<Vec<_>>()
                .join(", "),
        },
    ];
    checks.push(accessibility.toolkit_accessibility.clone());
    checks.push(accessibility.at_spi_enabled.clone());
    checks.push(bus_name_check("org.freedesktop.portal.Desktop"));
    checks.push(portal_interface_check(
        "org.freedesktop.portal.RemoteDesktop",
    ));
    checks.push(portal_interface_check("org.freedesktop.portal.ScreenCast"));
    checks.push(gnome_shell_version_check());
    checks.push(session_env_check());

    let can_build_accessibility_tree = can_build_accessibility_tree(&accessibility);
    let can_capture_screen =
        environment.capture_backend != sky_cua_platform::model::CaptureBackendKind::None;
    let can_send_input = environment.input_backend != InputBackendKind::None;
    let blockers = readiness_blockers(
        can_build_accessibility_tree,
        can_capture_screen,
        can_send_input,
        can_list_windows,
        can_target_windows,
    );
    let recommended_next_step = if blockers.is_empty() {
        "Computer Use desktop integrations are ready.".to_string()
    } else if !can_build_accessibility_tree {
        "Run setup_accessibility, then restart target applications.".to_string()
    } else if !can_target_windows {
        "Run setup_window_targeting on GNOME or install the session-specific window backend tools."
            .to_string()
    } else {
        format!("{}.", blockers.join(". "))
    };

    let note = if can_list_windows {
        if window_probes.iter().any(|p| p.id == "cosmic" && p.ok) {
            "A COSMIC Wayland window backend is available for list_windows, focused_window, and targeted input verification.".to_string()
        } else if window_probes.iter().any(|p| p.id == "kwin" && p.ok) {
            "A KWin/Plasma window backend is available for list_windows, focused_window, and targeted input verification.".to_string()
        } else if window_probes.iter().any(|p| p.id == "hyprland" && p.ok) {
            "A Hyprland window backend is available for list_windows, focused_window, and targeted input verification.".to_string()
        } else {
            "A GNOME window listing backend is available for list_windows, focused_window, and targeted input verification.".to_string()
        }
    } else {
        "Window listing is unavailable or denied. Computer Use can still use screenshots, AT-SPI, and global ydotool input, but targeted window input cannot be verified. On GNOME, run setup_window_targeting to install the optional GNOME Shell extension backend. On COSMIC, ensure the bundled COSMIC helper is present and can connect to the session. On KDE/Plasma, ensure KWin exposes org.kde.KWin scripting on the session bus. On Hyprland, ensure hyprctl is available in the session.".to_string()
    };

    DoctorReport {
        environment: environment.clone(),
        checks,
        readiness: DoctorReadiness {
            can_register_mcp_tools: true,
            can_build_accessibility_tree,
            can_capture_screen,
            can_send_input,
            can_list_windows,
            can_target_windows,
            recommended_next_step,
            blockers,
        },
        platform: Some(DoctorPlatformReport {
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
            session_kind: environment.session_kind.clone(),
            xdg_session_type: environment.xdg_session_type.clone(),
            desktop_environment: environment.desktop_environment.clone(),
            compositor: environment.compositor.clone(),
            display: environment.display.clone(),
            wayland_display: environment.wayland_display.clone(),
        }),
        session_env: Some(session_env_report),
        portal: Some(portal),
        accessibility: Some(accessibility),
        windowing: Some(DoctorWindowingReport {
            probes: window_probes
                .into_iter()
                .map(|probe| WindowBackendProbe {
                    id: probe.id.to_string(),
                    ok: probe.ok,
                    can_list_windows: probe.can_list_windows,
                    can_focus_apps: probe.can_focus_apps,
                    can_focus_windows: probe.can_focus_windows,
                    detail: probe.detail,
                })
                .collect(),
            can_list_windows,
            can_focus_windows: can_target_windows,
            detail: windowing_detail(can_list_windows, can_target_windows),
            note,
        }),
        input: Some(input),
        browser_integration: Some(browser_integration),
    }
}

fn session_env_check() -> DoctorCheck {
    let ok = session_env::required_session_env_present();
    DoctorCheck {
        name: "session_env".to_string(),
        ok,
        detail: if ok {
            "display, runtime dir, and session bus are available after repair".to_string()
        } else {
            "desktop session env is incomplete after repair".to_string()
        },
    }
}

fn readiness_blockers(
    can_build_accessibility_tree: bool,
    can_capture_screen: bool,
    can_send_input: bool,
    can_list_windows: bool,
    can_target_windows: bool,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if !can_build_accessibility_tree {
        blockers.push("AT-SPI semantic accessibility is unavailable".to_string());
    }
    if !can_capture_screen {
        blockers.push("No screenshot/capture backend is available".to_string());
    }
    if !can_send_input {
        blockers.push("No physical input backend is available".to_string());
    }
    if !can_list_windows {
        blockers.push("No native window-listing backend is available".to_string());
    }
    if !can_target_windows {
        blockers.push("No exact window-targeting backend is available".to_string());
    }
    blockers
}

fn windowing_detail(can_list_windows: bool, can_target_windows: bool) -> String {
    if can_list_windows && can_target_windows {
        "At least one window backend supports listing and exact targeting".to_string()
    } else if can_list_windows {
        "At least one window backend supports listing, but no exact window-targeting backend is available"
            .to_string()
    } else {
        windowing::registry::WINDOW_PERMISSION_HINT.to_string()
    }
}

fn portal_report(environment: &EnvironmentInfo) -> DoctorPortalReport {
    let caps = &environment.portal_capabilities;
    DoctorPortalReport {
        screencast_version: caps.screencast_version,
        remote_desktop_version: caps.remote_desktop_version,
        screenshot_version: caps.screenshot_version,
        input_capture_version: None,
        detail: format!(
            "screencast={:?}, remote_desktop={:?}, screenshot={:?}",
            caps.screencast_version, caps.remote_desktop_version, caps.screenshot_version
        ),
    }
}

fn accessibility_report() -> DoctorAccessibilityReport {
    DoctorAccessibilityReport {
        atspi_bus: atspi_bus_check(),
        toolkit_accessibility: toolkit_accessibility_check(),
        at_spi_enabled: at_spi_enabled_check(),
        screen_reader: screen_reader_check(),
    }
}

fn atspi_bus_check() -> DoctorCheck {
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.a11y.Bus",
            "--object-path",
            "/org/a11y/bus",
            "--method",
            "org.a11y.Bus.GetAddress",
        ])
        .output();
    command_check("atspi_bus", output, "AT-SPI bus address is available")
}

pub fn toolkit_accessibility_check() -> DoctorCheck {
    let output = Command::new("gsettings")
        .args([
            "get",
            "org.gnome.desktop.interface",
            "toolkit-accessibility",
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            DoctorCheck {
                name: "toolkit_accessibility".to_string(),
                ok: value == "true",
                detail: value,
            }
        }
        other => command_check(
            "toolkit_accessibility",
            other,
            "gsettings toolkit-accessibility unavailable",
        ),
    }
}

fn at_spi_enabled_check() -> DoctorCheck {
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.a11y.Bus",
            "--object-path",
            "/org/a11y/bus",
            "--method",
            "org.freedesktop.DBus.Properties.Get",
            "org.a11y.Status",
            "IsEnabled",
        ])
        .output();
    command_check("at_spi_enabled", output, "AT-SPI is enabled")
}

fn screen_reader_check() -> DoctorCheck {
    let output = Command::new("gsettings")
        .args([
            "get",
            "org.gnome.desktop.a11y.applications",
            "screen-reader-enabled",
        ])
        .output();
    command_check("screen_reader", output, "screen reader setting is readable")
}

fn input_report(backend: InputBackendKind) -> DoctorInputReport {
    DoctorInputReport {
        backend,
        ydotool: binary_check("ydotool"),
        ydotoold: process_check("ydotoold"),
        ydotool_socket: ydotool_socket_check(),
        xdotool: binary_check("xdotool"),
        uinput: path_check("uinput", "/dev/uinput"),
    }
}

fn ydotool_socket_check() -> DoctorCheck {
    let mut checked = Vec::new();
    for candidate in ydotool_socket_candidates() {
        match socket_connect_result(&candidate) {
            Ok(()) => {
                return DoctorCheck {
                    name: "ydotool_socket".to_string(),
                    ok: true,
                    detail: format!("connectable: {}", candidate.display()),
                };
            }
            Err(detail) => checked.push(detail),
        }
    }
    DoctorCheck {
        name: "ydotool_socket".to_string(),
        ok: false,
        detail: format!("no connectable ydotool socket ({})", checked.join("; ")),
    }
}

fn browser_report() -> BrowserIntegrationReport {
    BrowserIntegrationReport {
        chrome: binary_check("google-chrome"),
        chromium: binary_check("chromium"),
        brave: binary_check("brave-browser"),
        native_host_manifest: native_host_manifest_check(),
    }
}

fn native_host_manifest_check() -> DoctorCheck {
    let home = env::var("HOME").unwrap_or_else(|_| String::new());
    let candidates = [
        ".config/google-chrome/NativeMessagingHosts/com.openai.codexextension.json",
        ".config/chromium/NativeMessagingHosts/com.openai.codexextension.json",
        ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts/com.openai.codexextension.json",
    ];
    let hits = candidates
        .iter()
        .map(|path| PathBuf::from(&home).join(path))
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    DoctorCheck {
        name: "native_host_manifest".to_string(),
        ok: !hits.is_empty(),
        detail: if hits.is_empty() {
            "No Codex-compatible Chrome native host manifest found".to_string()
        } else {
            hits.iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        },
    }
}

fn binary_check(binary: &str) -> DoctorCheck {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {binary}"))
        .output();
    command_check(binary, output, &format!("{binary} is available"))
}

fn process_check(name: &str) -> DoctorCheck {
    let ok = fs::read_dir("/proc")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            fs::read_to_string(entry.path().join("comm")).is_ok_and(|comm| comm.trim() == name)
        });
    DoctorCheck {
        name: name.to_string(),
        ok,
        detail: if ok {
            format!("{name} is running")
        } else {
            format!("{name} is not running")
        },
    }
}

fn path_check(name: &str, path: &str) -> DoctorCheck {
    let ok = PathBuf::from(path).exists();
    DoctorCheck {
        name: name.to_string(),
        ok,
        detail: if ok {
            format!("{path} exists")
        } else {
            format!("{path} is missing")
        },
    }
}

fn command_check(
    name: &str,
    output: std::io::Result<std::process::Output>,
    ok_detail: &str,
) -> DoctorCheck {
    match output {
        Ok(output) if output.status.success() => DoctorCheck {
            name: name.to_string(),
            ok: true,
            detail: {
                let detail = format_command_output(&output.stdout, &output.stderr, "");
                if detail.is_empty() {
                    ok_detail.to_string()
                } else {
                    detail
                }
            },
        },
        Ok(output) => DoctorCheck {
            name: name.to_string(),
            ok: false,
            detail: {
                let detail = format_command_output(&output.stdout, &output.stderr, "");
                if detail.is_empty() {
                    format!("command exited with {}", output.status)
                } else {
                    detail
                }
            },
        },
        Err(error) => DoctorCheck {
            name: name.to_string(),
            ok: false,
            detail: error.to_string(),
        },
    }
}

pub(crate) fn format_command_output(stdout: &[u8], stderr: &[u8], fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    fallback.to_string()
}

pub(crate) fn can_build_accessibility_tree(accessibility: &DoctorAccessibilityReport) -> bool {
    accessibility.atspi_bus.ok
        && (check_detail_contains_true(&accessibility.at_spi_enabled)
            || check_detail_contains_true(&accessibility.toolkit_accessibility))
}

fn check_detail_contains_true(check: &DoctorCheck) -> bool {
    check.ok && check.detail.to_ascii_lowercase().contains("true")
}

fn ydotool_socket_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(value) = env::var("YDOTOOL_SOCKET")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        candidates.push(PathBuf::from(value));
    }
    if let Some(runtime_socket) =
        session_env::xdg_runtime_dir().map(|runtime| runtime.join(".ydotool_socket"))
    {
        candidates.push(runtime_socket);
    }
    candidates.push(PathBuf::from("/tmp/.ydotool_socket"));
    candidates
}

fn socket_connect_result(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("missing: {}", path.display()));
    }
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Ok(());
    }
    if let Ok(sock) = std::os::unix::net::UnixDatagram::unbound()
        && sock.connect(path).is_ok()
    {
        return Ok(());
    }
    Err(format!("{}: not connectable", path.display()))
}

fn gnome_shell_version_check() -> DoctorCheck {
    run_command("gnome_shell_version", "gnome-shell", &["--version"], false)
}

fn bus_name_check(name: &str) -> DoctorCheck {
    run_command(name, "busctl", &["--user", "status", name], true)
}

fn portal_interface_check(interface: &str) -> DoctorCheck {
    run_command(
        interface,
        "busctl",
        &[
            "--user",
            "introspect",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            interface,
        ],
        true,
    )
}

fn run_command(name: &str, command: &str, args: &[&str], with_session_bus: bool) -> DoctorCheck {
    let mut cmd = Command::new(command);
    cmd.args(args);
    if with_session_bus {
        if let Some(address) = session_env::dbus_session_address() {
            cmd.env("DBUS_SESSION_BUS_ADDRESS", address);
        }
        if let Some(runtime) = session_env::xdg_runtime_dir() {
            cmd.env("XDG_RUNTIME_DIR", runtime);
        }
    }
    match cmd.output() {
        Ok(output) if output.status.success() => {
            let detail = String::from_utf8_lossy(&output.stdout).trim().to_string();
            DoctorCheck {
                name: name.to_string(),
                ok: true,
                detail: if detail.is_empty() {
                    "ok".into()
                } else {
                    detail
                },
            }
        }
        Ok(output) => {
            let detail = format_command_output(&output.stdout, &output.stderr, "");
            DoctorCheck {
                name: name.to_string(),
                ok: false,
                detail: if detail.is_empty() {
                    format!("exit status {}", output.status)
                } else {
                    detail
                },
            }
        }
        Err(error) => DoctorCheck {
            name: name.to_string(),
            ok: false,
            detail: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accessibility_report(
        at_spi_bus: DoctorCheck,
        toolkit_accessibility: DoctorCheck,
        at_spi_enabled: DoctorCheck,
    ) -> DoctorAccessibilityReport {
        DoctorAccessibilityReport {
            atspi_bus: at_spi_bus,
            toolkit_accessibility,
            at_spi_enabled,
            screen_reader: DoctorCheck {
                name: "screen_reader".to_string(),
                ok: false,
                detail: "(<false>,)".to_string(),
            },
        }
    }

    #[test]
    fn readiness_blocks_when_window_listing_exists_without_targeting() {
        let blockers = readiness_blockers(true, true, true, true, false);

        assert_eq!(
            blockers,
            vec!["No exact window-targeting backend is available".to_string()]
        );
    }

    #[test]
    fn windowing_detail_distinguishes_listing_from_exact_targeting() {
        assert_eq!(
            windowing_detail(true, false),
            "At least one window backend supports listing, but no exact window-targeting backend is available"
        );
        assert_eq!(
            windowing_detail(true, true),
            "At least one window backend supports listing and exact targeting"
        );
    }

    #[test]
    fn accessibility_tree_requires_reachable_at_spi_bus() {
        let report = accessibility_report(
            DoctorCheck {
                name: "atspi_bus".to_string(),
                ok: false,
                detail: "permission denied".to_string(),
            },
            DoctorCheck {
                name: "toolkit_accessibility".to_string(),
                ok: true,
                detail: "true".to_string(),
            },
            DoctorCheck {
                name: "at_spi_enabled".to_string(),
                ok: false,
                detail: "(<false>,)".to_string(),
            },
        );

        assert!(!can_build_accessibility_tree(&report));
    }

    #[test]
    fn accessibility_tree_is_ready_when_bus_and_toolkit_are_ready() {
        let report = accessibility_report(
            DoctorCheck {
                name: "atspi_bus".to_string(),
                ok: true,
                detail: "('unix:path=/run/user/1000/at-spi/bus',)".to_string(),
            },
            DoctorCheck {
                name: "toolkit_accessibility".to_string(),
                ok: true,
                detail: "true".to_string(),
            },
            DoctorCheck {
                name: "at_spi_enabled".to_string(),
                ok: false,
                detail: "(<false>,)".to_string(),
            },
        );

        assert!(can_build_accessibility_tree(&report));
    }

    #[test]
    fn accessibility_tree_is_ready_when_bus_and_at_spi_enabled_are_true() {
        let report = accessibility_report(
            DoctorCheck {
                name: "atspi_bus".to_string(),
                ok: true,
                detail: "('unix:path=/run/user/1000/at-spi/bus',)".to_string(),
            },
            DoctorCheck {
                name: "toolkit_accessibility".to_string(),
                ok: false,
                detail: "false".to_string(),
            },
            DoctorCheck {
                name: "at_spi_enabled".to_string(),
                ok: true,
                detail: "(<true>,)".to_string(),
            },
        );

        assert!(can_build_accessibility_tree(&report));
    }

    #[test]
    fn ydotool_socket_connect_succeeds_against_bound_unix_socket() {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-diagnostics-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp diagnostics dir");
        let socket = dir.join("ydotool.sock");
        let listener =
            std::os::unix::net::UnixListener::bind(&socket).expect("bind temp diagnostics socket");

        let result = socket_connect_result(&socket);

        assert!(result.is_ok(), "{result:?}");
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ydotool_socket_connect_fails_when_socket_missing() {
        let missing = PathBuf::from("/tmp/sky-cua-test-missing-ydotool-socket.sock");
        let result = socket_connect_result(&missing);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }

    #[test]
    fn check_detail_contains_true_matches_lowercase_true() {
        let check = DoctorCheck {
            name: "test".to_string(),
            ok: true,
            detail: "(<true>,)".to_string(),
        };
        assert!(check_detail_contains_true(&check));
    }

    #[test]
    fn check_detail_contains_true_rejects_false_detail() {
        let check = DoctorCheck {
            name: "test".to_string(),
            ok: true,
            detail: "(<false>,)".to_string(),
        };
        assert!(!check_detail_contains_true(&check));
    }
}
