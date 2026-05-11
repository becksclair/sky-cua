use std::env;
use std::fs;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    CaptureBackendKind, EnvironmentInfo, InputBackendKind, PortalCapabilities, SemanticBackendKind,
    SessionKind,
};

use crate::portal::{remote_desktop, screencast, screenshot};
use crate::x11;
use tracing::debug;

pub async fn probe_environment() -> Result<EnvironmentInfo, BackendError> {
    debug!("probing Linux desktop environment");
    let xdg_session_type = non_empty_env("XDG_SESSION_TYPE");
    let display_var = non_empty_env("DISPLAY");
    let wayland_display = non_empty_env("WAYLAND_DISPLAY");

    let compositor = detect_compositor();
    let desktop_environment = detect_desktop_environment();
    if display_var.is_none() && wayland_display.is_none() {
        return Ok(EnvironmentInfo {
            session_kind: SessionKind::Unsupported,
            compositor,
            desktop_environment,
            capture_backend: CaptureBackendKind::None,
            input_backend: InputBackendKind::None,
            semantic_backend: SemanticBackendKind::None,
            portal_capabilities: empty_portal_capabilities(),
            xdg_session_type,
            display: display_var,
            wayland_display,
        });
    }

    debug!("probing portal capabilities");
    let portal_capabilities = probe_portals().await;
    debug!(
        session_type = ?xdg_session_type,
        x11_display = ?display_var,
        wayland_display = ?wayland_display,
        compositor = ?compositor,
        desktop_environment = ?desktop_environment,
        screencast = ?portal_capabilities.screencast_version,
        remote_desktop = ?portal_capabilities.remote_desktop_version,
        screenshot = ?portal_capabilities.screenshot_version,
        "finished Linux environment probe before semantic backend resolution"
    );

    let session_kind = infer_session_kind(
        xdg_session_type.as_deref(),
        display_var.is_some(),
        wayland_display.is_some(),
        compositor.as_deref(),
        x11::windowing::x11_server_running(),
    );

    let capture_backend = select_capture_backend(
        session_kind.clone(),
        &portal_capabilities,
        x11::capture::x11_capture_available(),
    );

    let input_backend = select_input_backend(
        session_kind.clone(),
        &portal_capabilities,
        x11::input_xtest::xtest_is_available(),
    );

    Ok(EnvironmentInfo {
        session_kind,
        compositor,
        desktop_environment,
        capture_backend,
        input_backend,
        semantic_backend: SemanticBackendKind::None,
        portal_capabilities,
        xdg_session_type,
        display: display_var,
        wayland_display,
    })
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn infer_session_kind(
    xdg_session_type: Option<&str>,
    has_display: bool,
    has_wayland_display: bool,
    compositor: Option<&str>,
    x11_server_available: bool,
) -> SessionKind {
    match xdg_session_type.map(|value| value.to_ascii_lowercase()) {
        Some(value) if value == "x11" && has_display && x11_server_available => {
            return SessionKind::X11;
        }
        Some(value) if value == "wayland" && has_wayland_display => {
            return SessionKind::Wayland;
        }
        _ => {}
    }

    if compositor.is_some_and(|value| value.contains("wayland")) {
        SessionKind::Wayland
    } else if has_display && x11_server_available {
        SessionKind::X11
    } else if has_wayland_display {
        SessionKind::Wayland
    } else {
        SessionKind::Unsupported
    }
}

fn select_capture_backend(
    session_kind: SessionKind,
    portal_capabilities: &PortalCapabilities,
    x11_capture_available: bool,
) -> CaptureBackendKind {
    match session_kind {
        SessionKind::X11 => {
            if x11_capture_available {
                CaptureBackendKind::X11
            } else {
                CaptureBackendKind::None
            }
        }
        SessionKind::Wayland => {
            if portal_capabilities.screencast_version.is_some() {
                CaptureBackendKind::PortalPipeWire
            } else if portal_capabilities.screenshot_version.is_some() {
                CaptureBackendKind::PortalScreenshot
            } else {
                CaptureBackendKind::None
            }
        }
        SessionKind::Windows | SessionKind::Unsupported => CaptureBackendKind::None,
    }
}

fn select_input_backend(
    session_kind: SessionKind,
    portal_capabilities: &PortalCapabilities,
    xtest_available: bool,
) -> InputBackendKind {
    match session_kind {
        SessionKind::X11 => {
            if xtest_available {
                InputBackendKind::XTest
            } else {
                InputBackendKind::None
            }
        }
        SessionKind::Wayland => {
            if portal_capabilities.remote_desktop_version.is_some() {
                InputBackendKind::PortalRemoteDesktop
            } else {
                InputBackendKind::None
            }
        }
        SessionKind::Windows | SessionKind::Unsupported => InputBackendKind::None,
    }
}

fn empty_portal_capabilities() -> PortalCapabilities {
    PortalCapabilities {
        screencast_version: None,
        remote_desktop_version: None,
        screenshot_version: None,
        available_source_types: None,
        available_cursor_modes: None,
        available_device_types: None,
    }
}

fn probe_process_names() -> Vec<String> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let pid = file_name.to_string_lossy();
        if !pid.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let comm_path = entry.path().join("comm");
        if let Ok(name) = fs::read_to_string(comm_path) {
            names.push(name.trim().to_string());
        }
    }
    names
}

fn detect_compositor() -> Option<String> {
    let process_names = probe_process_names();
    if process_names
        .iter()
        .any(|name| name.contains("kwin_wayland"))
    {
        return Some("kde-kwin-wayland".to_string());
    }
    if process_names.iter().any(|name| name == "Xorg") {
        return Some("x11-xorg".to_string());
    }
    if process_names
        .iter()
        .any(|name| name.contains("gnome-shell"))
    {
        return Some("gnome-shell".to_string());
    }
    None
}

fn detect_desktop_environment() -> Option<String> {
    let from_env = env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if from_env.is_some() {
        return from_env;
    }

    let process_names = probe_process_names();
    if process_names
        .iter()
        .any(|name| name.contains("plasmashell"))
    {
        return Some("KDE".to_string());
    }
    None
}

async fn probe_portals() -> PortalCapabilities {
    let screencast_version = screencast::version().await.ok();
    let available_source_types = screencast::available_source_types().await.ok();
    let available_cursor_modes = screencast::available_cursor_modes().await.ok();
    let remote_desktop_version = remote_desktop::version().await.ok();
    let available_device_types = remote_desktop::available_device_types().await.ok();
    let screenshot_version = screenshot::version().await.ok();

    PortalCapabilities {
        screencast_version,
        remote_desktop_version,
        screenshot_version,
        available_source_types,
        available_cursor_modes,
        available_device_types,
    }
}

pub fn require_supported_environment(environment: &EnvironmentInfo) -> Result<(), BackendError> {
    let missing_display = match environment.session_kind {
        SessionKind::Unsupported => true,
        SessionKind::X11 => environment.display.is_none(),
        SessionKind::Wayland => environment.wayland_display.is_none(),
        SessionKind::Windows => false,
    };
    if missing_display {
        return Err(BackendError::new(
            BackendErrorCode::UnsupportedEnvironment,
            "No supported Linux display server was detected; set DISPLAY or WAYLAND_DISPLAY from a graphical session and retry",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        infer_session_kind, probe_environment, require_supported_environment,
        select_capture_backend, select_input_backend,
    };
    use serial_test::serial;
    use sky_cua_platform::model::{
        CaptureBackendKind, EnvironmentInfo, InputBackendKind, PortalCapabilities,
        SemanticBackendKind, SessionKind,
    };

    #[tokio::test]
    #[serial]
    async fn probe_without_display_env_returns_unsupported_without_portal_probe() {
        struct EnvRestore {
            display: Option<std::ffi::OsString>,
            wayland_display: Option<std::ffi::OsString>,
        }

        impl Drop for EnvRestore {
            fn drop(&mut self) {
                match self.display.take() {
                    Some(value) => unsafe { std::env::set_var("DISPLAY", value) },
                    None => unsafe { std::env::remove_var("DISPLAY") },
                }
                match self.wayland_display.take() {
                    Some(value) => unsafe { std::env::set_var("WAYLAND_DISPLAY", value) },
                    None => unsafe { std::env::remove_var("WAYLAND_DISPLAY") },
                }
            }
        }

        let _restore = EnvRestore {
            display: std::env::var_os("DISPLAY"),
            wayland_display: std::env::var_os("WAYLAND_DISPLAY"),
        };
        unsafe {
            std::env::remove_var("DISPLAY");
            std::env::remove_var("WAYLAND_DISPLAY");
        }

        let environment = probe_environment()
            .await
            .expect("headless probe should produce an environment");

        assert_eq!(environment.session_kind, SessionKind::Unsupported);
        assert_eq!(environment.capture_backend, CaptureBackendKind::None);
        assert_eq!(environment.input_backend, InputBackendKind::None);
        assert_eq!(environment.portal_capabilities.screencast_version, None);
        assert_eq!(environment.portal_capabilities.remote_desktop_version, None);
        assert_eq!(environment.portal_capabilities.screenshot_version, None);
    }

    #[test]
    fn explicit_x11_session_beats_host_wayland_compositor() {
        let session_kind =
            infer_session_kind(Some("x11"), true, false, Some("kde-kwin-wayland"), true);
        assert_eq!(session_kind, SessionKind::X11);
    }

    #[test]
    fn explicit_wayland_session_stays_wayland() {
        let session_kind = infer_session_kind(Some("wayland"), true, true, Some("x11-xorg"), true);
        assert_eq!(session_kind, SessionKind::Wayland);
    }

    #[test]
    fn unsupported_environment_returns_no_display_error() {
        let environment = EnvironmentInfo {
            session_kind: SessionKind::Unsupported,
            compositor: None,
            desktop_environment: None,
            capture_backend: CaptureBackendKind::None,
            input_backend: InputBackendKind::None,
            semantic_backend: SemanticBackendKind::None,
            portal_capabilities: PortalCapabilities {
                screencast_version: None,
                remote_desktop_version: None,
                screenshot_version: None,
                available_source_types: None,
                available_cursor_modes: None,
                available_device_types: None,
            },
            xdg_session_type: None,
            display: None,
            wayland_display: None,
        };

        let error = require_supported_environment(&environment).expect_err("headless must error");

        assert_eq!(error.code, "UnsupportedEnvironment");
        assert!(error.message.contains("No supported Linux display server"));
    }

    #[test]
    fn compositor_without_display_env_returns_no_display_error() {
        let environment = EnvironmentInfo {
            session_kind: SessionKind::Wayland,
            compositor: Some("kde-kwin-wayland".to_string()),
            desktop_environment: Some("KDE".to_string()),
            capture_backend: CaptureBackendKind::PortalPipeWire,
            input_backend: InputBackendKind::PortalRemoteDesktop,
            semantic_backend: SemanticBackendKind::None,
            portal_capabilities: PortalCapabilities {
                screencast_version: Some(5),
                remote_desktop_version: Some(2),
                screenshot_version: Some(2),
                available_source_types: None,
                available_cursor_modes: None,
                available_device_types: None,
            },
            xdg_session_type: None,
            display: None,
            wayland_display: None,
        };

        let error = require_supported_environment(&environment).expect_err("headless must error");

        assert_eq!(error.code, "UnsupportedEnvironment");
        assert!(error.message.contains("No supported Linux display server"));
    }

    #[test]
    fn x11_session_prefers_x11_capture_and_input_over_portals() {
        let capabilities = PortalCapabilities {
            screencast_version: Some(5),
            remote_desktop_version: Some(2),
            screenshot_version: Some(2),
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        };

        assert_eq!(
            select_capture_backend(SessionKind::X11, &capabilities, true),
            CaptureBackendKind::X11
        );
        assert_eq!(
            select_input_backend(SessionKind::X11, &capabilities, true),
            InputBackendKind::XTest
        );
    }

    #[test]
    fn wayland_session_prefers_portal_backends() {
        let capabilities = PortalCapabilities {
            screencast_version: Some(5),
            remote_desktop_version: Some(2),
            screenshot_version: Some(2),
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        };

        assert_eq!(
            select_capture_backend(SessionKind::Wayland, &capabilities, true),
            CaptureBackendKind::PortalPipeWire
        );
        assert_eq!(
            select_input_backend(SessionKind::Wayland, &capabilities, true),
            InputBackendKind::PortalRemoteDesktop
        );
    }
}
