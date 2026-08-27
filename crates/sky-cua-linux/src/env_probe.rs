use std::env;
use std::fs;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    CaptureBackendKind, EnvironmentInfo, InputBackendKind, PortalCapabilities, SemanticBackendKind,
    SessionKind,
};
use tracing::debug;

use crate::portal::{remote_desktop, screencast, screenshot};
use crate::virtual_input;
use crate::x11;

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
            displays: Vec::new(),
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

    let virtual_input_available = virtual_input::probe_virtual_input().is_ok();
    let input_backend = select_input_backend(
        session_kind.clone(),
        &portal_capabilities,
        desktop_environment.as_deref(),
        compositor.as_deref(),
        x11::input_xtest::xtest_is_available(),
        virtual_input_available,
    );

    // PipeWire frame capture runs through the combined RemoteDesktop session,
    // so it is only reachable when input is routed through that portal. Gate
    // the capture backend on the resolved input backend rather than
    // advertising a PipeWire lane that can never produce a frame.
    let capture_backend = select_capture_backend(
        session_kind.clone(),
        &portal_capabilities,
        x11::capture::x11_capture_available(),
        input_backend.clone(),
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
        displays: Vec::new(),
    })
}

pub(crate) fn non_empty_env(name: &str) -> Option<String> {
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

    if has_wayland_display || compositor.is_some_and(|value| value.contains("wayland")) {
        SessionKind::Wayland
    } else if has_display && x11_server_available {
        SessionKind::X11
    } else {
        SessionKind::Unsupported
    }
}

fn select_capture_backend(
    session_kind: SessionKind,
    portal_capabilities: &PortalCapabilities,
    x11_capture_available: bool,
    input_backend: InputBackendKind,
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
            // PipeWire frame capture is performed through the combined
            // RemoteDesktop+ScreenCast session, so it is only reachable when
            // input also runs through the RemoteDesktop portal. When input
            // falls back to virtual input, the Screenshot portal is the honest
            // primary capture lane instead of a PipeWire lane that can never
            // produce a frame.
            if portal_capabilities.screencast_version.is_some()
                && input_backend == InputBackendKind::PortalRemoteDesktop
            {
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
    desktop_environment: Option<&str>,
    compositor: Option<&str>,
    xtest_available: bool,
    virtual_input_available: bool,
) -> InputBackendKind {
    if let Some(override_backend) = input_backend_override(
        session_kind.clone(),
        xtest_available,
        virtual_input_available,
    ) {
        return override_backend;
    }

    match session_kind {
        SessionKind::X11 => {
            if xtest_available {
                InputBackendKind::XTest
            } else {
                InputBackendKind::None
            }
        }
        SessionKind::Wayland => {
            let remote_desktop_available = portal_capabilities.remote_desktop_version.is_some();
            if remote_desktop_available
                && should_prefer_portal_input(desktop_environment, compositor)
            {
                InputBackendKind::PortalRemoteDesktop
            } else if virtual_input_available {
                InputBackendKind::LinuxVirtualInput
            } else if remote_desktop_available {
                InputBackendKind::PortalRemoteDesktop
            } else {
                InputBackendKind::None
            }
        }
        SessionKind::Windows | SessionKind::Unsupported => InputBackendKind::None,
    }
}

fn should_prefer_portal_input(desktop_environment: Option<&str>, compositor: Option<&str>) -> bool {
    fn matches_portal_first_desktop(value: &str) -> bool {
        let value = value.to_ascii_lowercase();
        value.contains("kde")
            || value.contains("plasma")
            || value.contains("kwin")
            || value.contains("gnome")
            || value.contains("cosmic")
    }

    desktop_environment.is_some_and(matches_portal_first_desktop)
        || compositor.is_some_and(matches_portal_first_desktop)
}

fn input_backend_override(
    session_kind: SessionKind,
    xtest_available: bool,
    virtual_input_available: bool,
) -> Option<InputBackendKind> {
    let value = env::var("SKY_CUA_INPUT_BACKEND").ok()?;
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "auto" {
        return None;
    }
    Some(match normalized.as_str() {
        "portal" | "remote-desktop" | "remote_desktop" => {
            if session_kind == SessionKind::Wayland {
                InputBackendKind::PortalRemoteDesktop
            } else {
                InputBackendKind::None
            }
        }
        "x11" | "xtest" => {
            if session_kind == SessionKind::X11 && xtest_available {
                InputBackendKind::XTest
            } else {
                InputBackendKind::None
            }
        }
        "linux-virtual" | "linux_virtual" | "virtual" | "ydotool" => {
            if virtual_input_available {
                InputBackendKind::LinuxVirtualInput
            } else {
                InputBackendKind::None
            }
        }
        "none" => InputBackendKind::None,
        _ => return None,
    })
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
    if process_names
        .iter()
        .any(|name| name.contains("cosmic-comp"))
    {
        return Some("cosmic-comp".to_string());
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
    use serial_test::serial;
    use sky_cua_platform::model::{
        CaptureBackendKind, EnvironmentInfo, InputBackendKind, PortalCapabilities,
        SemanticBackendKind, SessionKind,
    };

    use super::{
        infer_session_kind, probe_environment, require_supported_environment,
        select_capture_backend, select_input_backend,
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
    fn ssh_tty_with_wayland_display_prefers_wayland_over_xwayland_display() {
        let session_kind = infer_session_kind(Some("tty"), true, true, None, true);
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
            displays: Vec::new(),
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
            displays: Vec::new(),
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
            select_capture_backend(
                SessionKind::X11,
                &capabilities,
                true,
                InputBackendKind::XTest
            ),
            CaptureBackendKind::X11
        );
        assert_eq!(
            select_input_backend(SessionKind::X11, &capabilities, None, None, true, true),
            InputBackendKind::XTest
        );
    }

    #[test]
    fn kde_wayland_session_prefers_portal_capture_and_remote_desktop_input() {
        let capabilities = PortalCapabilities {
            screencast_version: Some(5),
            remote_desktop_version: Some(2),
            screenshot_version: Some(2),
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        };

        assert_eq!(
            select_capture_backend(
                SessionKind::Wayland,
                &capabilities,
                true,
                InputBackendKind::PortalRemoteDesktop
            ),
            CaptureBackendKind::PortalPipeWire
        );
        assert_eq!(
            select_input_backend(
                SessionKind::Wayland,
                &capabilities,
                Some("KDE"),
                Some("kde-kwin-wayland"),
                true,
                true
            ),
            InputBackendKind::PortalRemoteDesktop
        );
    }

    #[test]
    fn wayland_portal_preferred_for_known_desktops() {
        let capabilities = PortalCapabilities {
            screencast_version: Some(5),
            remote_desktop_version: Some(2),
            screenshot_version: Some(2),
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        };

        assert_eq!(
            select_input_backend(
                SessionKind::Wayland,
                &capabilities,
                Some("COSMIC"),
                None,
                false,
                true
            ),
            InputBackendKind::PortalRemoteDesktop
        );
        assert_eq!(
            select_input_backend(
                SessionKind::Wayland,
                &capabilities,
                Some("GNOME"),
                Some("gnome-shell"),
                false,
                true
            ),
            InputBackendKind::PortalRemoteDesktop
        );
        assert_eq!(
            select_input_backend(
                SessionKind::Wayland,
                &capabilities,
                Some("GNOME"),
                Some("gnome-shell"),
                false,
                false
            ),
            InputBackendKind::PortalRemoteDesktop
        );
    }

    #[test]
    fn wayland_without_remote_desktop_input_uses_screenshot_capture() {
        // COSMIC advertises ScreenCast but not RemoteDesktop, so input falls
        // back to LinuxVirtualInput and PipeWire frame capture (which rides the
        // combined RemoteDesktop session) is unreachable. The Screenshot portal
        // is the honest primary capture lane in that state.
        let capabilities = PortalCapabilities {
            screencast_version: Some(4),
            remote_desktop_version: None,
            screenshot_version: Some(2),
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        };

        assert_eq!(
            select_capture_backend(
                SessionKind::Wayland,
                &capabilities,
                true,
                InputBackendKind::LinuxVirtualInput
            ),
            CaptureBackendKind::PortalScreenshot
        );
    }

    #[test]
    fn unknown_wayland_desktop_falls_back_to_linux_virtual_input() {
        let capabilities = PortalCapabilities {
            screencast_version: Some(5),
            remote_desktop_version: Some(2),
            screenshot_version: Some(2),
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        };

        assert_eq!(
            select_input_backend(
                SessionKind::Wayland,
                &capabilities,
                Some("Sway"),
                None,
                false,
                true
            ),
            InputBackendKind::LinuxVirtualInput
        );
    }

    #[test]
    fn wayland_without_remote_desktop_uses_linux_virtual_input_when_available() {
        let capabilities = PortalCapabilities {
            screencast_version: Some(5),
            remote_desktop_version: None,
            screenshot_version: Some(2),
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        };

        assert_eq!(
            select_input_backend(SessionKind::Wayland, &capabilities, None, None, false, true),
            InputBackendKind::LinuxVirtualInput
        );
        assert_eq!(
            select_input_backend(
                SessionKind::Wayland,
                &capabilities,
                None,
                None,
                false,
                false
            ),
            InputBackendKind::None
        );
    }
}
