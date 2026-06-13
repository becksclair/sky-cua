use chrono::Utc;

use super::{
    ActionName, ActionOutcome, AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPoint,
    AgentCursorState, AgentCursorSystemCursorBackendKind, AppStateSnapshot, CaptureBackendKind,
    CoordinateSpace, DoctorReadiness, DoctorReport, EnvironmentInfo, InputBackendKind,
    PortalCapabilities, ScrollDirection, SemanticBackendKind, SessionKind, SetupCommandReport,
    ToolAvailability, ToolCapabilities, WindowInfo,
};

pub(super) fn available_capabilities() -> ToolCapabilities {
    let available = || ToolAvailability {
        available: true,
        reason: None,
    };
    ToolCapabilities {
        list_apps: available(),
        get_app_state: available(),
        focus_element: available(),
        activate_element: available(),
        select_element: available(),
        expand_element: available(),
        collapse_element: available(),
        toggle_element: available(),
        click: available(),
        perform_action: available(),
        perform_secondary_action: available(),
        scroll: available(),
        supported_scroll_directions: vec![ScrollDirection::Up, ScrollDirection::Down],
        drag: available(),
        type_text: available(),
        press_key: available(),
        set_value: available(),
    }
}

pub(super) fn environment_info() -> EnvironmentInfo {
    EnvironmentInfo {
        session_kind: SessionKind::Wayland,
        compositor: Some("KWin".to_string()),
        desktop_environment: Some("KDE".to_string()),
        capture_backend: CaptureBackendKind::PortalPipeWire,
        input_backend: InputBackendKind::PortalRemoteDesktop,
        semantic_backend: SemanticBackendKind::Atspi,
        portal_capabilities: PortalCapabilities {
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
        displays: Vec::new(),
    }
}

pub(super) fn doctor_report() -> DoctorReport {
    DoctorReport {
        environment: environment_info(),
        checks: Vec::new(),
        readiness: DoctorReadiness {
            can_register_mcp_tools: true,
            can_build_accessibility_tree: true,
            can_capture_screen: true,
            can_send_input: true,
            can_list_windows: true,
            can_target_windows: true,
            can_inhibit_presence: false,
            can_unlock_session: false,
            recommended_next_step: "Ready".to_string(),
            blockers: Vec::new(),
        },
        platform: None,
        session_env: None,
        portal: None,
        accessibility: None,
        windowing: None,
        input: None,
        browser_integration: None,
        session_presence: None,
    }
}

pub(super) fn setup_command_report() -> SetupCommandReport {
    SetupCommandReport {
        ok: true,
        detail: "ok".to_string(),
    }
}

pub(super) fn window_info() -> WindowInfo {
    WindowInfo {
        window_id: "w1".to_string(),
        title: Some("Test".to_string()),
        app_id: Some("app".to_string()),
        wm_class: None,
        pid: Some(42),
        bounds: None,
        display: None,
        display_intersections: Vec::new(),
        workspace: None,
        focused: true,
        hidden: false,
        client_type: None,
        backend: "kwin".to_string(),
        terminal: None,
    }
}

pub(super) fn action_outcome() -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: "ok".to_string(),
        code: "Ok".to_string(),
        diagnostics: Vec::new(),
        agent_cursor: None,
    }
}

pub(super) fn cursor_state() -> AgentCursorState {
    AgentCursorState {
        visible: true,
        sequence: 1,
        model_point: Some(AgentCursorPoint {
            x: 10.0,
            y: 20.0,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: None,
        }),
        native_point: None,
        snapshot_id: Some("snap-1".to_string()),
        source_action: Some(ActionName::Click),
        updated_at_ms: 42,
    }
}

pub(super) fn cursor_capabilities() -> AgentCursorCapabilities {
    AgentCursorCapabilities {
        backend: AgentCursorBackendKind::WaylandLayerShell,
        visible_overlay: true,
        screenshot_synthetic_cursor: true,
        click_through: true,
        capture_exclusion: false,
        system_cursor_hide_supported: false,
        system_cursor_hidden: false,
        system_cursor_backend: AgentCursorSystemCursorBackendKind::WaylandClientUnsupported,
        needs_user_install: false,
        reason: None,
    }
}

pub(super) fn app_state_snapshot() -> AppStateSnapshot {
    AppStateSnapshot {
        snapshot_id: "snap-1".to_string(),
        created_at: Utc::now(),
        environment: environment_info(),
        capabilities: available_capabilities(),
        focused_app: None,
        capture: None,
        elements: Vec::new(),
        diagnostics: Vec::new(),
        app_guidance: None,
        doctor_report: None,
        agent_cursor: None,
    }
}
