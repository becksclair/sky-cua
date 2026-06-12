use super::test_support::available_capabilities;
use super::{
    ActionName, ActionOutcome, ActionRequest, AgentCursorBackendKind, AgentCursorCapabilities,
    AgentCursorPlane, AgentCursorPoint, AgentCursorState, AgentCursorSystemCursorBackendKind,
    AppStateSnapshot, CaptureBackendKind, CaptureInfo, CoordinateSpace, DoctorCheck,
    DoctorReadiness, DoctorReport, DoctorSessionEnvRepair, DoctorSessionEnvReport, ElementNode,
    ElementNumericValueReadback, ElementTextReadback, ElementTextSelection, EnvironmentInfo,
    InputBackendKind, ModelImageFormat, PixelSize, PortalCapabilities, RectF, SemanticBackendKind,
    ServiceRequest, ServiceResponse, SessionKind, SetupCommandReport, WindowInfo, WindowTarget,
    WindowTargetingSetupReport,
};
use chrono::Utc;
use serde_json::json;

#[test]
fn boxed_execute_action_preserves_wire_shape() {
    let rendered = serde_json::to_value(ServiceRequest::ExecuteAction {
        request: Box::new(ActionRequest {
            action: ActionName::Click,
            snapshot_id: Some("snap-1".to_string()),
            element_index: Some(7),
            arguments: json!({"x": 10, "y": 20}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: None,
        }),
    })
    .expect("service request should serialize");

    assert_eq!(
        rendered,
        json!({
            "type": "execute_action",
            "request": {
                "action": "click",
                "snapshot_id": "snap-1",
                "element_index": 7,
                "arguments": {"x": 10, "y": 20}
            }
        })
    );
}

#[test]
fn window_target_normalizes_host_default_empty_values() {
    let mut target = WindowTarget {
        window_id: Some("  ".to_string()),
        pid: Some(0),
        tty: Some("".to_string()),
        terminal_pid: Some(0),
        terminal_command: Some("\t".to_string()),
        terminal_cwd: Some("".to_string()),
        app_id: Some(" chromium.desktop ".to_string()),
        wm_class: Some("".to_string()),
        title: Some("".to_string()),
    };

    target.normalize_empty_fields();

    assert_eq!(target.app_id.as_deref(), Some("chromium.desktop"));
    assert_eq!(target.pid, None);
    assert_eq!(target.terminal_pid, None);
    assert!(target.has_target());
}

#[test]
fn window_target_zero_process_ids_do_not_count_as_targets() {
    let mut target = WindowTarget {
        pid: Some(0),
        terminal_pid: Some(0),
        ..WindowTarget::default()
    };

    assert!(!target.has_target());
    target.normalize_empty_fields();
    assert_eq!(target.pid, None);
    assert_eq!(target.terminal_pid, None);
}

#[test]
fn window_target_extracts_present_argument_fields_only() {
    let target = WindowTarget::from_argument_fields(&json!({
        "window_id": "",
        "pid": 0,
        "tty": "",
        "terminal_pid": 0,
        "terminal_command": "",
        "terminal_cwd": "",
        "app_id": " chromium.desktop ",
        "wm_class": "",
        "title": "",
        "text": "large untargeted keyboard payload"
    }))
    .expect("target arguments parse")
    .expect("app_id remains a target");

    assert_eq!(target.app_id.as_deref(), Some("chromium.desktop"));
    assert_eq!(target.pid, None);
    assert_eq!(target.terminal_pid, None);
}

#[test]
fn window_target_argument_extraction_ignores_empty_defaults() {
    let target = WindowTarget::from_argument_fields(&json!({
        "window_id": "",
        "pid": 0,
        "tty": null,
        "terminal_pid": 0,
        "terminal_command": " ",
        "terminal_cwd": "",
        "wm_class": "",
        "title": ""
    }))
    .expect("target arguments parse");

    assert_eq!(target, None);
}

#[test]
fn get_app_state_capture_screen_defaults_to_if_changed_on_wire() {
    let rendered = serde_json::to_value(ServiceRequest::GetAppState {
        selector: None,
        capture_screen: Default::default(),
    })
    .expect("service request should serialize");

    assert_eq!(rendered, json!({"type": "get_app_state"}));

    let parsed: ServiceRequest =
        serde_json::from_value(json!({"type": "get_app_state"})).expect("request parses");
    assert_eq!(
        parsed,
        ServiceRequest::GetAppState {
            selector: None,
            capture_screen: Default::default(),
        }
    );
}

#[test]
fn element_node_deserializes_old_json_without_readback_fields() {
    let node: ElementNode = serde_json::from_value(json!({
        "element_index": 1,
        "parent_index": null,
        "role": "text",
        "name": "Search",
        "description": null,
        "value": null,
        "state_flags": ["focused"],
        "semantic_actions": ["set_value"],
        "bounds": null,
        "backend_ref": ":1.2:/node/1"
    }))
    .expect("old element JSON should remain readable");

    assert_eq!(node.text, None);
    assert_eq!(node.numeric_value, None);
    assert!(!node.supports_editable_text);
}

#[test]
fn element_node_serializes_readback_and_skips_absent_defaults() {
    let without_readback = ElementNode {
        element_index: 1,
        parent_index: None,
        role: "text".to_string(),
        name: Some("Search".to_string()),
        description: None,
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags: vec!["focused".to_string()],
        semantic_actions: vec!["set_value".to_string()],
        bounds: None,
        backend_ref: None,
    };
    let rendered = serde_json::to_value(&without_readback).expect("serialize element");
    assert!(rendered.get("text").is_none());
    assert!(rendered.get("numeric_value").is_none());
    assert!(rendered.get("supports_editable_text").is_none());

    let with_readback = ElementNode {
        value: Some("hello".to_string()),
        text: Some(ElementTextReadback {
            character_count: 5,
            caret_offset: Some(5),
            content: Some("hello".to_string()),
            content_suppressed: false,
            truncated: false,
            selections: vec![ElementTextSelection {
                start_offset: 0,
                end_offset: 5,
            }],
        }),
        numeric_value: Some(ElementNumericValueReadback {
            current: 5.0,
            minimum: 0.0,
            maximum: 10.0,
            minimum_increment: 1.0,
            text: Some("5".to_string()),
        }),
        supports_editable_text: true,
        ..without_readback
    };
    let rendered = serde_json::to_value(&with_readback).expect("serialize element");

    assert_eq!(rendered["value"], "hello");
    assert_eq!(rendered["text"]["content"], "hello");
    assert_eq!(rendered["text"]["selections"][0]["end_offset"], 5);
    assert_eq!(rendered["numeric_value"]["text"], "5");
    assert_eq!(rendered["supports_editable_text"], true);
}

#[test]
fn boxed_get_app_state_preserves_wire_shape() {
    let rendered = serde_json::to_value(ServiceResponse::GetAppState {
        snapshot: Box::new(AppStateSnapshot {
            snapshot_id: "snap-1".to_string(),
            created_at: Utc::now(),
            focused_app: None,
            environment: EnvironmentInfo {
                session_kind: SessionKind::Wayland,
                compositor: Some("KWin".to_string()),
                desktop_environment: Some("KDE".to_string()),
                wayland_display: Some("wayland-0".to_string()),
                display: None,
                xdg_session_type: Some("wayland".to_string()),
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
            },
            capabilities: available_capabilities(),
            elements: Vec::new(),
            diagnostics: Vec::new(),
            capture: Some(CaptureInfo {
                backend: CaptureBackendKind::PortalPipeWire,
                image_backend: Some(CaptureBackendKind::PortalPipeWire),
                stream_id: Some("42".to_string()),
                source_type: Some(1),
                mapping_id: None,
                screenshot_path: Some("/tmp/snap.jpg".to_string()),
                original_screenshot_path: Some("/tmp/snap.png".to_string()),
                pixel_size: Some(PixelSize {
                    width: 1920,
                    height: 1080,
                }),
                original_pixel_size: Some(PixelSize {
                    width: 3840,
                    height: 2160,
                }),
                coordinate_space: Some(CoordinateSpace::StreamPixels),
                logical_rect: Some(RectF {
                    x: 0.0,
                    y: 0.0,
                    width: 3840.0,
                    height: 2160.0,
                    space: CoordinateSpace::DesktopLogical,
                }),
                logical_to_pixel_scale: Some(0.5),
                model_image_format: Some(ModelImageFormat::Jpeg),
                model_image_quality: Some(85),
                model_image_bytes: Some(1234),
                model_image_encode_ms: Some(7),
            }),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        }),
    })
    .expect("service response should serialize");

    assert_eq!(rendered["type"], "get_app_state");
    assert_eq!(rendered["snapshot"]["snapshot_id"], "snap-1");
    assert_eq!(
        rendered["snapshot"]["capture"]["screenshot_path"],
        "/tmp/snap.jpg"
    );
    assert!(rendered.get("snapshot").is_some());
    assert!(rendered["snapshot"].get("doctor_report").is_none());
    assert!(rendered["snapshot"].get("agent_cursor").is_none());
}

#[test]
fn boxed_get_app_state_includes_doctor_report_when_present() {
    let report = DoctorReport {
        environment: EnvironmentInfo {
            session_kind: SessionKind::Wayland,
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
        },
        checks: vec![DoctorCheck {
            name: "semantic_backend".to_string(),
            ok: true,
            detail: "Atspi".to_string(),
        }],
        readiness: DoctorReadiness {
            can_register_mcp_tools: true,
            can_build_accessibility_tree: true,
            can_capture_screen: true,
            can_send_input: true,
            can_list_windows: false,
            can_target_windows: false,
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
    };
    let rendered = serde_json::to_value(ServiceResponse::GetAppState {
        snapshot: Box::new(AppStateSnapshot {
            snapshot_id: "snap-1".to_string(),
            created_at: Utc::now(),
            focused_app: None,
            environment: report.environment.clone(),
            capabilities: available_capabilities(),
            elements: Vec::new(),
            diagnostics: Vec::new(),
            capture: None,
            app_guidance: None,
            doctor_report: Some(report),
            agent_cursor: None,
        }),
    })
    .expect("service response should serialize");

    assert!(rendered["snapshot"].get("doctor_report").is_some());
    assert_eq!(
        rendered["snapshot"]["doctor_report"]["readiness"]["can_build_accessibility_tree"],
        true
    );
}

#[test]
fn doctor_report_deserializes_without_session_env() {
    let value = serde_json::json!({
        "environment": {
            "session_kind": "unsupported",
            "compositor": null,
            "desktop_environment": null,
            "capture_backend": "none",
            "input_backend": "none",
            "semantic_backend": "none",
            "portal_capabilities": {
                "screencast_version": null,
                "remote_desktop_version": null,
                "screenshot_version": null,
                "available_source_types": null,
                "available_cursor_modes": null,
                "available_device_types": null
            },
            "xdg_session_type": null,
            "display": null,
            "wayland_display": null
        },
        "checks": [],
        "readiness": {
            "can_register_mcp_tools": true,
            "can_build_accessibility_tree": false,
            "can_capture_screen": false,
            "can_send_input": false,
            "recommended_next_step": "not ready",
            "blockers": []
        }
    });

    let report: DoctorReport =
        serde_json::from_value(value).expect("old doctor JSON should deserialize");

    assert!(report.session_env.is_none());
    assert!(!report.readiness.can_inhibit_presence);
    assert!(!report.readiness.can_unlock_session);
    assert!(report.session_presence.is_none());
}

#[test]
fn doctor_report_serializes_populated_session_env() {
    let report = DoctorSessionEnvReport {
        repaired: vec![DoctorSessionEnvRepair {
            key: "WAYLAND_DISPLAY".to_string(),
            source: "systemd-user".to_string(),
            value: Some("wayland-0".to_string()),
        }],
        path_changed: true,
        final_path: Some("/usr/bin:/bin".to_string()),
        notes: Vec::new(),
    };

    let value = serde_json::to_value(report).expect("session env report should serialize");

    assert_eq!(value["repaired"][0]["key"], "WAYLAND_DISPLAY");
    assert_eq!(value["path_changed"], true);
    assert_eq!(value["final_path"], "/usr/bin:/bin");
}

#[test]
fn window_targeting_report_skips_permissions_hint_when_none() {
    let report = WindowTargetingSetupReport {
        extension_dir: "/tmp/ext".to_string(),
        wrote_files: true,
        enable_command: SetupCommandReport {
            ok: true,
            detail: "enabled".to_string(),
        },
        windows: vec![WindowInfo {
            window_id: "w1".to_string(),
            title: Some("Test".to_string()),
            app_id: Some("app".to_string()),
            wm_class: None,
            pid: Some(42),
            bounds: None,
            workspace: None,
            focused: false,
            hidden: false,
            client_type: None,
            backend: "gnome".to_string(),
            terminal: None,
        }],
        windows_error: None,
        requires_shell_reload: false,
        message: "ok".to_string(),
        permissions_hint: None,
    };
    let rendered = serde_json::to_value(&report).expect("serialize");
    assert!(rendered.get("permissions_hint").is_none());
}

#[test]
fn window_targeting_report_includes_permissions_hint_when_present() {
    let report = WindowTargetingSetupReport {
        extension_dir: "/tmp/ext".to_string(),
        wrote_files: true,
        enable_command: SetupCommandReport {
            ok: true,
            detail: "enabled".to_string(),
        },
        windows: Vec::new(),
        windows_error: Some("dbus error".to_string()),
        requires_shell_reload: false,
        message: "failed".to_string(),
        permissions_hint: Some("Check permissions".to_string()),
    };
    let rendered = serde_json::to_value(&report).expect("serialize");
    assert_eq!(
        rendered["permissions_hint"].as_str(),
        Some("Check permissions")
    );
}

#[test]
fn agent_cursor_contract_serializes_snake_case_and_skips_absent_optional_fields() {
    let state = AgentCursorState {
        visible: true,
        sequence: 7,
        model_point: Some(AgentCursorPoint {
            x: 40.0,
            y: 25.5,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: Some("stream-1".to_string()),
        }),
        native_point: None,
        snapshot_id: Some("snap-1".to_string()),
        source_action: Some(ActionName::Click),
        updated_at_ms: 1_714_000_000_000,
    };
    let rendered = serde_json::to_value(&state).expect("cursor state should serialize");

    assert_eq!(rendered["model_point"]["coordinate_space"], "stream_pixels");
    assert_eq!(rendered["source_action"], "click");
    assert!(rendered.get("native_point").is_none());
    assert_eq!(
        serde_json::to_value(AgentCursorPlane::ScreenshotSynthetic).expect("serialize plane"),
        json!("screenshot_synthetic")
    );
}

#[test]
fn agent_cursor_capabilities_report_backend_as_snake_case() {
    let rendered = serde_json::to_value(AgentCursorCapabilities {
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
    })
    .expect("capabilities should serialize");

    assert_eq!(rendered["backend"], "wayland_layer_shell");
    assert_eq!(rendered["system_cursor_hide_supported"], false);
    assert_eq!(rendered["system_cursor_hidden"], false);
    assert_eq!(
        rendered["system_cursor_backend"],
        "wayland_client_unsupported"
    );
    assert!(rendered.get("reason").is_none());
    assert_eq!(
        serde_json::to_value(AgentCursorBackendKind::GnomeShellExtension)
            .expect("serialize backend"),
        json!("gnome_shell_extension")
    );
    assert_eq!(
        serde_json::to_value(AgentCursorSystemCursorBackendKind::GnomeShellExtension)
            .expect("serialize system cursor backend"),
        json!("gnome_shell_extension")
    );
    assert_eq!(
        serde_json::to_value(AgentCursorSystemCursorBackendKind::HyprlandConfig)
            .expect("serialize system cursor backend"),
        json!("hyprland_config")
    );
    assert_eq!(
        serde_json::to_value(AgentCursorSystemCursorBackendKind::CosmicCompBridge)
            .expect("serialize system cursor backend"),
        json!("cosmic_comp_bridge")
    );
    assert_eq!(
        serde_json::to_value(AgentCursorSystemCursorBackendKind::CosmicTransparentXcursor)
            .expect("serialize system cursor backend"),
        json!("cosmic_transparent_xcursor")
    );

    let old: AgentCursorCapabilities = serde_json::from_value(json!({
        "backend": "wayland_layer_shell",
        "visible_overlay": true,
        "screenshot_synthetic_cursor": true,
        "click_through": true,
        "capture_exclusion": false,
        "needs_user_install": false
    }))
    .expect("old capabilities without system cursor fields should deserialize");
    assert!(!old.system_cursor_hide_supported);
    assert!(!old.system_cursor_hidden);
    assert_eq!(
        old.system_cursor_backend,
        AgentCursorSystemCursorBackendKind::None
    );
}

#[test]
fn action_outcome_skips_absent_cursor_and_accepts_old_wire_shape() {
    let rendered = serde_json::to_value(ActionOutcome {
        success: true,
        message: "ok".to_string(),
        code: "Ok".to_string(),
        diagnostics: Vec::new(),
        agent_cursor: None,
    })
    .expect("outcome should serialize");

    assert!(rendered.get("agent_cursor").is_none());

    let old: ActionOutcome = serde_json::from_value(json!({
        "success": true,
        "message": "ok",
        "code": "Ok",
        "diagnostics": []
    }))
    .expect("old outcomes without cursor should deserialize");
    assert_eq!(old.agent_cursor, None);
}

#[test]
fn app_state_snapshot_accepts_old_wire_shape_without_agent_cursor() {
    let old = json!({
        "snapshot_id": "snap-old",
        "created_at": "2026-05-14T19:00:00Z",
        "environment": {
            "session_kind": "wayland",
            "compositor": "KWin",
            "desktop_environment": "KDE",
            "capture_backend": "portal_pipe_wire",
            "input_backend": "portal_remote_desktop",
            "semantic_backend": "atspi",
            "portal_capabilities": {
                "screencast_version": 5,
                "remote_desktop_version": 2,
                "screenshot_version": 1,
                "available_source_types": null,
                "available_cursor_modes": null,
                "available_device_types": null
            },
            "xdg_session_type": "wayland",
            "display": null,
            "wayland_display": "wayland-0"
        },
        "capabilities": available_capabilities(),
        "focused_app": null,
        "capture": null,
        "elements": [],
        "diagnostics": [],
        "app_guidance": null
    });

    let snapshot: AppStateSnapshot =
        serde_json::from_value(old).expect("old snapshot should deserialize");
    assert_eq!(snapshot.agent_cursor, None);
}

#[test]
fn agent_cursor_service_requests_preserve_json_wire_shape() {
    let state = AgentCursorState {
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
    };
    let rendered = serde_json::to_value(ServiceRequest::SetAgentCursor { state })
        .expect("request should serialize");

    assert_eq!(rendered["type"], "set_agent_cursor");
    assert_eq!(
        rendered["state"]["model_point"]["coordinate_space"],
        "stream_pixels"
    );
    assert!(rendered["state"]["model_point"].get("mapping_id").is_none());
}
