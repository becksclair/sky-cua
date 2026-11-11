//! macOS `DoctorReport` construction from the probed environment and the TCC
//! grant state. Pure: every input is passed in, so each grant combination is
//! unit-testable.

use sky_cua_platform::model::{
    CaptureBackendKind, DoctorCheck, DoctorDisplayTopologyReport, DoctorPlatformReport,
    DoctorReadiness, DoctorReport, DoctorSessionPresenceReport, DoctorWindowingReport,
    EnvironmentInfo, InputBackendKind, SemanticBackendKind, WindowBackendProbe,
};

use super::{
    HOST_APPLICATION, Permission, PermissionStatus, join_names, missing_permission_message,
    settings_paths,
};

/// Window enumeration provider id in the windowing probe.
const WINDOW_PROVIDER: &str = "core_graphics";

pub(super) fn build(environment: EnvironmentInfo, status: PermissionStatus) -> DoctorReport {
    let semantic_available = environment.semantic_backend != SemanticBackendKind::None;
    let capture_available = environment.capture_backend != CaptureBackendKind::None;
    let input_available = environment.input_backend != InputBackendKind::None;

    let can_build_accessibility_tree = semantic_available && status.accessibility;
    let can_capture_screen = capture_available && status.screen_capture;
    let can_send_input = input_available && status.post_event;
    // The CoreGraphics window list needs no grant; raising a specific window
    // goes through AX and needs Accessibility.
    let can_list_windows = true;
    let can_target_windows = status.accessibility;

    let display_topology = display_topology_report(&environment);

    let mut checks = vec![
        DoctorCheck {
            name: "semantic_backend".to_string(),
            ok: semantic_available,
            detail: format!("{:?}", environment.semantic_backend),
        },
        DoctorCheck {
            name: "capture_backend".to_string(),
            ok: capture_available,
            detail: format!("{:?}", environment.capture_backend),
        },
        DoctorCheck {
            name: "input_backend".to_string(),
            ok: input_available,
            detail: format!("{:?}", environment.input_backend),
        },
    ];
    checks.extend(Permission::ALL.map(|permission| permission_check(permission, status)));
    checks.push(DoctorCheck {
        name: "windowing_backend".to_string(),
        ok: can_list_windows,
        detail: format!("{WINDOW_PROVIDER}={can_list_windows}"),
    });
    checks.push(DoctorCheck {
        name: "display_topology".to_string(),
        ok: display_topology.display_count > 0,
        detail: display_topology.detail.clone(),
    });

    let blockers = readiness_blockers(
        status,
        semantic_available,
        capture_available,
        input_available,
    );
    let recommended_next_step = recommended_next_step(status, &blockers);

    DoctorReport {
        checks,
        readiness: DoctorReadiness {
            can_register_mcp_tools: true,
            can_build_accessibility_tree,
            can_capture_screen,
            can_send_input,
            can_list_windows,
            can_target_windows,
            can_inhibit_presence: false,
            can_unlock_session: false,
            recommended_next_step,
            blockers,
        },
        platform: Some(DoctorPlatformReport {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            session_kind: environment.session_kind.clone(),
            xdg_session_type: environment.xdg_session_type.clone(),
            desktop_environment: environment.desktop_environment.clone(),
            compositor: environment.compositor.clone(),
            display: environment.display.clone(),
            wayland_display: environment.wayland_display.clone(),
        }),
        display_topology: Some(display_topology),
        session_env: None,
        portal: None,
        accessibility: None,
        windowing: Some(windowing_report(
            status,
            can_list_windows,
            can_target_windows,
        )),
        input: None,
        browser_integration: None,
        session_presence: Some(DoctorSessionPresenceReport::unsupported("none")),
        environment,
    }
}

fn permission_check(permission: Permission, status: PermissionStatus) -> DoctorCheck {
    let ok = status.granted(permission);
    DoctorCheck {
        name: permission.check_name().to_string(),
        ok,
        detail: if ok {
            format!("granted to {HOST_APPLICATION}")
        } else {
            missing_permission_message(permission)
        },
    }
}

fn readiness_blockers(
    status: PermissionStatus,
    semantic_available: bool,
    capture_available: bool,
    input_available: bool,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if !status.accessibility {
        blockers.push("macOS Accessibility permission is not granted".to_string());
    } else if !semantic_available {
        blockers.push("macOS Accessibility semantic backend is unavailable".to_string());
    }
    if !status.screen_capture {
        blockers.push("macOS Screen Recording permission is not granted".to_string());
    } else if !capture_available {
        blockers.push("ScreenCaptureKit capture is unavailable".to_string());
    }
    if !status.post_event {
        blockers.push("macOS event-posting permission is not granted".to_string());
    } else if !input_available {
        blockers.push("CGEvent input injection is unavailable".to_string());
    }
    blockers
}

fn recommended_next_step(status: PermissionStatus, blockers: &[String]) -> String {
    let missing = status.missing();
    if missing.is_empty() {
        return if blockers.is_empty() {
            "Computer Use core backends are ready.".to_string()
        } else {
            format!("{}.", blockers.join(". "))
        };
    }
    let mut step = format!(
        "Enable {} for {HOST_APPLICATION} in {}. Run setup_desktop with operation \"accessibility\" to add that application to the list and open System Settings.",
        join_names(&missing),
        settings_paths(&missing),
    );
    if missing
        .iter()
        .any(|permission| permission.grant_requires_restart())
    {
        step.push_str(
            " After granting Screen Recording, quit and reopen that application and restart sky-cua.",
        );
    }
    step
}

fn display_topology_report(environment: &EnvironmentInfo) -> DoctorDisplayTopologyReport {
    let displays = &environment.displays;
    let selected_provider = displays
        .first()
        .map(|display| display.backend.clone())
        .filter(|backend| !backend.is_empty());
    let detail = match &selected_provider {
        Some(provider) => format!("{} display(s) available via {provider}", displays.len()),
        None => format!("{} display(s) available", displays.len()),
    };
    DoctorDisplayTopologyReport {
        display_count: displays.len(),
        selected_provider,
        probes: Vec::new(),
        detail,
    }
}

fn windowing_report(
    status: PermissionStatus,
    can_list_windows: bool,
    can_target_windows: bool,
) -> DoctorWindowingReport {
    let focus_detail = if can_target_windows {
        "AX raise available"
    } else {
        "AX raise needs Accessibility permission"
    };
    let title_detail = if status.screen_capture {
        "window titles readable"
    } else {
        "window titles withheld without Screen Recording permission"
    };
    DoctorWindowingReport {
        probes: vec![WindowBackendProbe {
            id: WINDOW_PROVIDER.to_string(),
            ok: can_list_windows,
            can_list_windows,
            // Application activation needs no TCC grant.
            can_focus_apps: true,
            can_focus_windows: can_target_windows,
            detail: format!("CGWindowList enumeration; {title_detail}; {focus_detail}"),
        }],
        can_list_windows,
        can_focus_windows: can_target_windows,
        detail: if can_target_windows {
            "CoreGraphics lists windows and AX raises an exact window".to_string()
        } else {
            "CoreGraphics lists windows; exact window targeting needs Accessibility permission"
                .to_string()
        },
        note: if status.screen_capture {
            String::new()
        } else {
            "Without Screen Recording permission CoreGraphics omits window titles, so title-based targeting cannot match.".to_string()
        },
    }
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{
        CoordinateSpace, DisplayInfo, PortalCapabilities, RectF, SessionKind,
    };

    use super::*;

    fn environment(displays: Vec<DisplayInfo>) -> EnvironmentInfo {
        EnvironmentInfo {
            session_kind: SessionKind::Macos,
            compositor: None,
            desktop_environment: Some("aqua".to_string()),
            capture_backend: CaptureBackendKind::ScreenCaptureKit,
            input_backend: InputBackendKind::CgEvent,
            semantic_backend: SemanticBackendKind::MacosAccessibility,
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
            displays,
        }
    }

    fn display() -> DisplayInfo {
        DisplayInfo {
            display_id: "1".to_string(),
            name: Some("Built-in Display".to_string()),
            index: 0,
            primary: true,
            logical_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1512.0,
                height: 982.0,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: None,
            scale_factor: Some(2.0),
            backend: "core_graphics".to_string(),
        }
    }

    fn status_from_mask(mask: u8) -> PermissionStatus {
        PermissionStatus {
            accessibility: mask & 1 == 0,
            screen_capture: mask & 2 == 0,
            post_event: mask & 4 == 0,
        }
    }

    fn check<'a>(report: &'a DoctorReport, name: &str) -> &'a DoctorCheck {
        report
            .checks
            .iter()
            .find(|check| check.name == name)
            .unwrap_or_else(|| panic!("missing check {name}"))
    }

    #[test]
    fn every_grant_combination_reports_honest_readiness() {
        for mask in 0..8u8 {
            let status = status_from_mask(mask);
            let report = build(environment(vec![display()]), status);
            let readiness = &report.readiness;
            let missing = status.missing();

            assert_eq!(readiness.can_build_accessibility_tree, status.accessibility);
            assert_eq!(readiness.can_capture_screen, status.screen_capture);
            assert_eq!(readiness.can_send_input, status.post_event);
            assert!(readiness.can_list_windows);
            assert_eq!(readiness.can_target_windows, status.accessibility);
            assert!(readiness.can_register_mcp_tools);
            assert!(!readiness.can_inhibit_presence);
            assert!(!readiness.can_unlock_session);
            assert_eq!(readiness.blockers.len(), missing.len(), "mask {mask}");

            for permission in Permission::ALL {
                let permission_check = check(&report, permission.check_name());
                assert_eq!(permission_check.ok, status.granted(permission));
                if !permission_check.ok {
                    assert!(permission_check.detail.contains(permission.settings_path()));
                }
            }
            for name in ["semantic_backend", "capture_backend", "input_backend"] {
                assert!(check(&report, name).ok);
            }

            if missing.is_empty() {
                assert_eq!(
                    readiness.recommended_next_step,
                    "Computer Use core backends are ready."
                );
            } else {
                let step = &readiness.recommended_next_step;
                assert!(
                    step.contains("System Settings > Privacy & Security"),
                    "{step}"
                );
                assert!(step.contains("setup_desktop"), "{step}");
                for permission in &missing {
                    assert!(step.contains(permission.display_name()), "{step}");
                    assert!(step.contains(permission.settings_path()), "{step}");
                }
                assert_eq!(
                    step.contains("quit and reopen"),
                    !status.screen_capture,
                    "{step}"
                );
            }

            let windowing = report.windowing.as_ref().expect("windowing report");
            assert_eq!(windowing.can_focus_windows, status.accessibility);
            assert_eq!(windowing.note.is_empty(), status.screen_capture);
        }
    }

    #[test]
    fn missing_backend_with_granted_permission_is_a_backend_blocker() {
        let mut env = environment(vec![display()]);
        env.capture_backend = CaptureBackendKind::None;
        let report = build(env, status_from_mask(0));
        assert!(!report.readiness.can_capture_screen);
        assert!(!check(&report, "capture_backend").ok);
        assert_eq!(
            report.readiness.blockers,
            vec!["ScreenCaptureKit capture is unavailable".to_string()]
        );
        assert_eq!(
            report.readiness.recommended_next_step,
            "ScreenCaptureKit capture is unavailable."
        );
    }

    #[test]
    fn missing_permission_wins_over_missing_backend_in_blockers() {
        let mut env = environment(vec![display()]);
        env.semantic_backend = SemanticBackendKind::None;
        let report = build(env, status_from_mask(1));
        assert_eq!(
            report.readiness.blockers,
            vec!["macOS Accessibility permission is not granted".to_string()]
        );
    }

    #[test]
    fn platform_and_optional_sections_are_macos_shaped() {
        let report = build(environment(vec![display()]), status_from_mask(0));
        let platform = report.platform.as_ref().expect("platform");
        assert_eq!(platform.os, std::env::consts::OS);
        assert_eq!(platform.session_kind, SessionKind::Macos);
        assert!(report.portal.is_none());
        assert!(report.accessibility.is_none());
        assert!(report.input.is_none());
        assert!(report.session_env.is_none());
        assert!(report.browser_integration.is_none());
        let presence = report.session_presence.as_ref().expect("session presence");
        assert!(!presence.unlock.ok && !presence.inhibit_lock.ok);
        let topology = report.display_topology.as_ref().expect("display topology");
        assert_eq!(topology.display_count, 1);
        assert_eq!(topology.selected_provider.as_deref(), Some("core_graphics"));
        assert!(check(&report, "display_topology").ok);
    }

    #[test]
    fn no_displays_fails_the_topology_check() {
        let report = build(environment(Vec::new()), status_from_mask(0));
        let topology = report.display_topology.as_ref().expect("display topology");
        assert_eq!(topology.display_count, 0);
        assert_eq!(topology.selected_provider, None);
        assert!(!check(&report, "display_topology").ok);
    }

    #[test]
    fn report_serializes() {
        let report = build(environment(vec![display()]), status_from_mask(7));
        let json = serde_json::to_value(&report).expect("serialize");
        assert_eq!(json["platform"]["session_kind"], "macos");
        assert_eq!(json["environment"]["capture_backend"], "screen_capture_kit");
    }
}
