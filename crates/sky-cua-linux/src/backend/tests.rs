use super::action_runtime::{scroll_target_value, vertical_scrollbar_for_point};
use super::elements::{
    fallback_window_elements_with_x11_detail, linux_fallback_snapshot, linux_window_elements,
    x11_window_elements,
};
use super::{
    AppInfo, AppSelector, DISPLAY_TOPOLOGY_CACHE_TTL, DisplayTopologyCache, ENVIRONMENT_CACHE_TTL,
    LinuxDesktopBackend, SessionEnvCache, cached_display_topology, merge_session_env_reports,
    reject_unactionable_targeted_capture, require_screenshot_image,
};
use crate::app_match::{
    app_from_linux_window, best_x11_window_match, matches_selector, select_x11_window,
    selector_summary, x11_window_matches_app,
};
use crate::capture_plan::{CapturePlanOutcome, CaptureRegionTarget, should_attempt_x11_capture};
use crate::windowing::LinuxWindowInfo;
use crate::x11::windowing::{X11WindowInfo, X11WindowRegion};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::test_support::wayland_pipewire_environment;
use sky_cua_platform::model::{
    CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace, DisplayInfo,
    DisplayRef, DoctorDisplayTopologyReport, DoctorSessionEnvRepair, DoctorSessionEnvReport,
    ElementNode, ElementNumericValueReadback, EnvironmentInfo, InputBackendKind, PixelSize,
    PortalCapabilities, RectF, ScrollDirection, SemanticBackendKind, SessionKind,
};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

#[test]
fn multi_action_cached_path_does_not_rerun_proc_or_systemctl_hydration() {
    let now = Instant::now();
    let backend = LinuxDesktopBackend {
        session_env: Arc::new(StdMutex::new(SessionEnvCache {
            report: DoctorSessionEnvReport::default(),
            hydrated_at: now,
        })),
        ..LinuxDesktopBackend::new()
    };
    let mut hydration_runs = 0;

    for action_offset in [
        Duration::ZERO,
        Duration::from_millis(100),
        Duration::from_millis(200),
    ] {
        backend.refresh_session_env_if_stale_with(now + action_offset, || {
            hydration_runs += 1;
            panic!("cached CUA actions must not walk /proc or invoke systemctl")
        });
    }

    assert_eq!(hydration_runs, 0);
}

#[test]
fn expired_environment_cache_rehydrates_before_backend_reselection() {
    let now = Instant::now();
    let backend = LinuxDesktopBackend {
        session_env: Arc::new(StdMutex::new(SessionEnvCache {
            report: DoctorSessionEnvReport::default(),
            hydrated_at: now - ENVIRONMENT_CACHE_TTL,
        })),
        ..LinuxDesktopBackend::new()
    };
    let mut hydration_runs = 0;

    let report = backend.refresh_session_env_if_stale_with(now, || {
        hydration_runs += 1;
        DoctorSessionEnvReport {
            notes: vec!["fresh /proc and systemctl hydration".to_string()],
            ..DoctorSessionEnvReport::default()
        }
    });

    assert_eq!(hydration_runs, 1);
    assert_eq!(report.notes, ["fresh /proc and systemctl hydration"]);
}

#[test]
fn screenshot_no_image_preserves_portal_error() {
    let portal_error = BackendError::new(
        BackendErrorCode::PortalApprovalPending,
        "operator approval is pending",
    );

    let error = require_screenshot_image(None, Some(&portal_error), None).unwrap_err();

    assert_eq!(error.code, BackendErrorCode::PortalApprovalPending.as_str());
    assert_eq!(error.message, "operator approval is pending");
}

fn test_element(
    element_index: usize,
    parent_index: Option<usize>,
    role: &str,
    bounds: Option<RectF>,
) -> ElementNode {
    ElementNode {
        element_index,
        parent_index,
        role: role.to_string(),
        name: None,
        description: None,
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags: Vec::new(),
        semantic_actions: Vec::new(),
        bounds,
        backend_ref: None,
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> RectF {
    RectF {
        x,
        y,
        width,
        height,
        space: CoordinateSpace::DesktopLogical,
    }
}

fn test_display(display_id: &str) -> DisplayInfo {
    DisplayInfo {
        display_id: display_id.to_string(),
        name: Some(display_id.to_string()),
        index: 0,
        primary: true,
        logical_rect: rect(0.0, 0.0, 1920.0, 1080.0),
        pixel_size: None,
        scale_factor: Some(1.0),
        backend: "test".to_string(),
    }
}

fn test_window(bounds: Option<RectF>, display: Option<DisplayRef>) -> LinuxWindowInfo {
    LinuxWindowInfo {
        window_id: "window-1".to_string(),
        title: Some("Test window".to_string()),
        app_id: Some("test.desktop".to_string()),
        wm_class: None,
        pid: Some(42),
        bounds,
        display,
        display_intersections: Vec::new(),
        workspace: None,
        focused: true,
        hidden: false,
        client_type: None,
        backend: "kwin".to_string(),
        terminal: None,
        terminal_target_sessions: Vec::new(),
    }
}

#[test]
fn get_app_state_capture_candidates_prefer_window_then_display() {
    let mut environment = wayland_pipewire_environment();
    environment.displays = vec![test_display("kwin:eDP-1")];
    let display_ref = DisplayRef::from(&environment.displays[0]);
    let window = test_window(Some(rect(100.0, 80.0, 640.0, 480.0)), Some(display_ref));
    let mut diagnostics = DiagnosticBuilder::new();

    let candidates =
        super::get_app_state_capture_candidates(&environment, Some(&window), &mut diagnostics);

    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].target.capture_scope, CaptureScope::Window);
    assert_eq!(candidates[0].label, "window");
    assert_eq!(candidates[1].target.capture_scope, CaptureScope::Display);
    assert_eq!(candidates[1].label, "display");
    assert!(diagnostics.finish().is_empty());
}

#[test]
fn get_app_state_capture_candidates_fall_back_to_display_without_window_bounds() {
    let mut environment = wayland_pipewire_environment();
    environment.displays = vec![test_display("kwin:eDP-1")];
    let display_ref = DisplayRef::from(&environment.displays[0]);
    let window = test_window(None, Some(display_ref));
    let mut diagnostics = DiagnosticBuilder::new();

    let candidates =
        super::get_app_state_capture_candidates(&environment, Some(&window), &mut diagnostics);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].target.capture_scope, CaptureScope::Display);
    let entries = diagnostics.finish();
    assert!(
        entries
            .iter()
            .any(|entry| entry.code == "GetAppStateCaptureScopeFallback")
    );
}

#[test]
fn get_app_state_capture_candidates_do_not_add_virtual_desktop_fallback() {
    let environment = wayland_pipewire_environment();
    let mut diagnostics = DiagnosticBuilder::new();

    let candidates = super::get_app_state_capture_candidates(&environment, None, &mut diagnostics);

    assert!(candidates.is_empty());
    assert!(diagnostics.finish().is_empty());
}

fn test_display_topology_report(display_count: usize) -> DoctorDisplayTopologyReport {
    DoctorDisplayTopologyReport {
        display_count,
        selected_provider: Some("test".to_string()),
        probes: Vec::new(),
        detail: format!("test topology with {display_count} display(s)"),
    }
}

fn vertical_scrollbar(index: usize, parent_index: usize, current: f64) -> ElementNode {
    let mut node = test_element(
        index,
        Some(parent_index),
        "scroll bar",
        Some(rect(95.0, 0.0, 5.0, 100.0)),
    );
    node.state_flags.push("vertical".to_string());
    node.semantic_actions.push("set_value".to_string());
    node.backend_ref = Some(format!(":1.1:/scrollbar/{index}"));
    node.numeric_value = Some(ElementNumericValueReadback {
        current,
        minimum: 0.0,
        maximum: 100.0,
        minimum_increment: 10.0,
        text: None,
    });
    node
}

#[test]
fn vertical_scrollbar_for_point_uses_containing_scroll_ancestor() {
    let elements = vec![
        test_element(0, None, "application", Some(rect(0.0, 0.0, 200.0, 200.0))),
        test_element(
            1,
            Some(0),
            "scroll pane",
            Some(rect(10.0, 20.0, 90.0, 80.0)),
        ),
        vertical_scrollbar(2, 1, 0.0),
    ];

    let (_, node) = vertical_scrollbar_for_point(&elements, 40.0, 50.0)
        .expect("point inside scroll pane should resolve scrollbar");

    assert_eq!(node.element_index, 2);
}

#[test]
fn scroll_target_value_maps_downward_delta_to_larger_value() {
    let node = vertical_scrollbar(0, 0, 20.0);

    assert_eq!(scroll_target_value(&node, Some(-180.0), -1), Some(40.0));
    assert_eq!(scroll_target_value(&node, Some(120.0), -1), Some(10.0));
}

#[test]
fn merge_session_env_reports_deduplicates_repeated_refreshes() {
    let repair = DoctorSessionEnvRepair {
        key: "WAYLAND_DISPLAY".to_string(),
        source: "systemd-user".to_string(),
        value: Some("wayland-0".to_string()),
    };
    let mut current = DoctorSessionEnvReport {
        repaired: vec![repair.clone()],
        path_changed: false,
        final_path: Some("/tmp:/usr/bin".to_string()),
        notes: vec!["systemd note".to_string()],
    };
    let latest = DoctorSessionEnvReport {
        repaired: vec![repair],
        path_changed: true,
        final_path: Some("/tmp:/usr/bin:/bin".to_string()),
        notes: vec!["systemd note".to_string()],
    };

    merge_session_env_reports(&mut current, latest);

    assert_eq!(current.repaired.len(), 1);
    assert_eq!(current.notes.len(), 1);
    assert!(current.path_changed);
    assert_eq!(current.final_path.as_deref(), Some("/tmp:/usr/bin:/bin"));
}

#[test]
fn display_topology_cache_expires_after_short_ttl() {
    let now = Instant::now();
    let cache = Arc::new(StdMutex::new(Some(DisplayTopologyCache {
        updated_at: now - Duration::from_millis(500),
        displays: vec![test_display("test:primary")],
        report: test_display_topology_report(1),
    })));

    let cached = cached_display_topology(&cache, now).expect("fresh cache should be used");
    assert_eq!(cached.displays[0].display_id, "test:primary");
    assert_eq!(cached.report.display_count, 1);

    *cache.lock().expect("cache lock") = Some(DisplayTopologyCache {
        updated_at: now - DISPLAY_TOPOLOGY_CACHE_TTL - Duration::from_millis(1),
        displays: vec![test_display("test:stale")],
        report: test_display_topology_report(1),
    });

    assert!(cached_display_topology(&cache, now).is_none());
}

#[test]
fn display_topology_cache_preserves_empty_probe_report() {
    let now = Instant::now();
    let report = test_display_topology_report(0);
    let cache = Arc::new(StdMutex::new(Some(DisplayTopologyCache {
        updated_at: now,
        displays: Vec::new(),
        report: report.clone(),
    })));

    let cached = cached_display_topology(&cache, now).expect("empty topology is cached");

    assert!(cached.displays.is_empty());
    assert_eq!(cached.report, report);
}

#[test]
fn capture_never_disables_x11_still_capture() {
    let environment = EnvironmentInfo {
        session_kind: SessionKind::X11,
        compositor: Some("x11-xorg".to_string()),
        desktop_environment: None,
        capture_backend: CaptureBackendKind::X11,
        input_backend: InputBackendKind::XTest,
        semantic_backend: SemanticBackendKind::Atspi,
        portal_capabilities: PortalCapabilities {
            screencast_version: None,
            remote_desktop_version: None,
            screenshot_version: None,
            available_source_types: None,
            available_cursor_modes: None,
            available_device_types: None,
        },
        xdg_session_type: Some("x11".to_string()),
        display: Some(":0".to_string()),
        wayland_display: None,
        displays: Vec::new(),
    };

    assert!(!should_attempt_x11_capture(
        CaptureScreenMode::Never,
        &environment
    ));
    assert!(should_attempt_x11_capture(
        CaptureScreenMode::Always,
        &environment
    ));
}

#[test]
fn targeted_screenshot_rejects_unactionable_fallback_snapshot() {
    let target = CaptureRegionTarget {
        desktop_logical_rect: RectF {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
            space: CoordinateSpace::DesktopLogical,
        },
        capture_scope: CaptureScope::Window,
        display: None,
    };
    let outcome = CapturePlanOutcome {
        capture: Some(CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalScreenshot),
            capture_scope: CaptureScope::Window,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            logical_to_pixel_scale: Some(1.0),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }),
        portal_session_error: None,
        capture_error: Some(BackendError::new(
            BackendErrorCode::CaptureSourceGeometryMissing,
            "targeted screenshot requires capture source geometry",
        )),
    };

    let environment = wayland_pipewire_environment();
    let error = reject_unactionable_targeted_capture(Some(&target), &outcome, &environment)
        .expect_err("must reject");

    assert_eq!(
        error.code,
        BackendErrorCode::CaptureSourceGeometryMissing.as_str()
    );
    assert!(error.message.contains("source geometry"));
}

#[test]
fn targeted_screenshot_rejects_unactionable_pipewire_failure_fallback() {
    let target = CaptureRegionTarget {
        desktop_logical_rect: RectF {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
            space: CoordinateSpace::DesktopLogical,
        },
        capture_scope: CaptureScope::Window,
        display: None,
    };
    let outcome = CapturePlanOutcome {
        capture: Some(CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalScreenshot),
            capture_scope: CaptureScope::Window,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            logical_to_pixel_scale: Some(1.0),
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }),
        portal_session_error: None,
        capture_error: Some(BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            "remote fd closed unexpectedly",
        )),
    };

    let environment = wayland_pipewire_environment();
    let error = reject_unactionable_targeted_capture(Some(&target), &outcome, &environment)
        .expect_err("must reject");

    assert_eq!(
        error.code,
        BackendErrorCode::CaptureSourceGeometryMissing.as_str()
    );
    assert!(error.message.contains("source geometry"));
}

#[test]
fn targeted_screenshot_rejects_direct_screenshot_portal_capture_for_remote_desktop_input() {
    let target = CaptureRegionTarget {
        desktop_logical_rect: RectF {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
            space: CoordinateSpace::DesktopLogical,
        },
        capture_scope: CaptureScope::Window,
        display: None,
    };
    let outcome = CapturePlanOutcome {
        capture: Some(CaptureInfo {
            backend: CaptureBackendKind::PortalScreenshot,
            image_backend: Some(CaptureBackendKind::PortalScreenshot),
            capture_scope: CaptureScope::Window,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            logical_to_pixel_scale: Some(1.0),
            screenshot_path: Some("/tmp/capture.png".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }),
        portal_session_error: None,
        capture_error: None,
    };
    let environment = wayland_pipewire_environment();

    let error = reject_unactionable_targeted_capture(Some(&target), &outcome, &environment)
        .expect_err("Portal RemoteDesktop screenshot capture should be rejected");

    assert_eq!(
        error.code,
        BackendErrorCode::CaptureSourceGeometryMissing.as_str()
    );
    assert!(error.message.contains("source geometry"));
}

#[test]
fn targeted_screenshot_allows_direct_screenshot_portal_capture_for_linux_virtual_input() {
    let target = CaptureRegionTarget {
        desktop_logical_rect: RectF {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
            space: CoordinateSpace::DesktopLogical,
        },
        capture_scope: CaptureScope::Window,
        display: None,
    };
    let outcome = CapturePlanOutcome {
        capture: Some(CaptureInfo {
            backend: CaptureBackendKind::PortalScreenshot,
            image_backend: Some(CaptureBackendKind::PortalScreenshot),
            capture_scope: CaptureScope::Window,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            logical_to_pixel_scale: Some(1.0),
            screenshot_path: Some("/tmp/capture.png".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }),
        portal_session_error: None,
        capture_error: None,
    };
    let mut environment = wayland_pipewire_environment();
    environment.input_backend = InputBackendKind::LinuxVirtualInput;

    reject_unactionable_targeted_capture(Some(&target), &outcome, &environment)
        .expect("direct Screenshot portal capture should remain actionable");
}

#[test]
fn targeted_screenshot_allows_pipewire_to_screenshot_fallback_for_linux_virtual_input() {
    let target = CaptureRegionTarget {
        desktop_logical_rect: RectF {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
            space: CoordinateSpace::DesktopLogical,
        },
        capture_scope: CaptureScope::Window,
        display: None,
    };
    let outcome = CapturePlanOutcome {
        capture: Some(CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalScreenshot),
            capture_scope: CaptureScope::Window,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("116".to_string()),
            source_type: Some(1),
            mapping_id: None,
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            original_pixel_size: Some(PixelSize {
                width: 1280,
                height: 720,
            }),
            logical_to_pixel_scale: Some(1.0),
            screenshot_path: Some("/tmp/capture.png".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }),
        portal_session_error: None,
        capture_error: None,
    };
    let mut environment = wayland_pipewire_environment();
    environment.input_backend = InputBackendKind::LinuxVirtualInput;

    reject_unactionable_targeted_capture(Some(&target), &outcome, &environment)
            .expect("PipeWire-primary Screenshot-fallback capture should remain actionable with LinuxVirtualInput");
}

#[test]
fn matches_app_selector_by_window_title() {
    let app = AppInfo {
        app_id: "app-1".to_string(),
        name: "zenity".to_string(),
        pid: Some(123),
        executable: Some("zenity".to_string()),
        desktop_file_id: Some("zenity.desktop".to_string()),
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: Some("GTK".to_string()),
        window_title: Some("sky-cua zenity smoke".to_string()),
        is_focused_candidate: false,
    };
    let selector = AppSelector {
        app_id: None,
        desktop_file_id: Some("zenity.desktop".to_string()),
        window_title: Some("zenity smoke".to_string()),
        name: None,
    };
    assert!(matches_selector(&app, &selector));
}

#[test]
fn matches_selector_case_insensitively_for_titles_and_names() {
    let app = AppInfo {
        app_id: "app-1".to_string(),
        name: "Zenity".to_string(),
        pid: Some(123),
        executable: Some("zenity".to_string()),
        desktop_file_id: Some("zenity.desktop".to_string()),
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: Some("GTK".to_string()),
        window_title: Some("Sky-CUA Pointer Smoke".to_string()),
        is_focused_candidate: false,
    };
    let selector = AppSelector {
        app_id: None,
        desktop_file_id: None,
        window_title: Some("pointer smoke".to_string()),
        name: Some("zenity".to_string()),
    };
    assert!(matches_selector(&app, &selector));
}

#[test]
fn summarizes_selector_fields() {
    let selector = AppSelector {
        app_id: Some("app-1".to_string()),
        desktop_file_id: None,
        window_title: Some("demo".to_string()),
        name: None,
    };
    assert_eq!(
        selector_summary(&selector),
        "app_id=app-1, window_title=demo"
    );
}

#[test]
fn matches_x11_window_to_accessible_app_by_pid() {
    let app = AppInfo {
        app_id: "accessible-1".to_string(),
        name: "Discord".to_string(),
        pid: Some(1234),
        executable: Some("discord".to_string()),
        desktop_file_id: Some("discord.desktop".to_string()),
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: Some("Electron".to_string()),
        window_title: Some("@Sky - Discord".to_string()),
        is_focused_candidate: false,
    };
    let window = X11WindowInfo {
        window_id: "0x2400006".to_string(),
        instance_name: Some("discord".to_string()),
        class_name: Some("discord".to_string()),
        app: AppInfo {
            app_id: "x11:0x2400006".to_string(),
            name: "discord".to_string(),
            pid: Some(1234),
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("@Sky - Discord".to_string()),
            is_focused_candidate: true,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };
    assert!(x11_window_matches_app(&window, &app));
}

#[test]
fn creates_a_synthetic_root_element_for_x11_fallback_windows() {
    let window = X11WindowInfo {
        window_id: "0x3800030".to_string(),
        instance_name: Some("xmessage".to_string()),
        class_name: Some("Xmessage".to_string()),
        app: AppInfo {
            app_id: "x11:0x3800030".to_string(),
            name: "Xmessage".to_string(),
            pid: None,
            executable: None,
            desktop_file_id: Some("xmessage.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("sky-cua xmessage probe".to_string()),
            is_focused_candidate: true,
        },
        bounds: Some(RectF {
            x: 100.0,
            y: 200.0,
            width: 320.0,
            height: 180.0,
            space: CoordinateSpace::DesktopLogical,
        }),
        workspace: None,
        child_regions: vec![
            X11WindowRegion {
                window_id: "0x3800031".to_string(),
                parent_window_id: None,
                depth: 1,
                name: None,
                bounds: RectF {
                    x: 100.0,
                    y: 200.0,
                    width: 320.0,
                    height: 180.0,
                    space: CoordinateSpace::DesktopLogical,
                },
            },
            X11WindowRegion {
                window_id: "0x3800032".to_string(),
                parent_window_id: Some("0x3800031".to_string()),
                depth: 2,
                name: Some("OK".to_string()),
                bounds: RectF {
                    x: 180.0,
                    y: 330.0,
                    width: 48.0,
                    height: 24.0,
                    space: CoordinateSpace::DesktopLogical,
                },
            },
        ],
    };

    let elements = x11_window_elements(&window);
    assert_eq!(elements.len(), 3);
    assert_eq!(elements[0].role, "window");
    assert_eq!(
        elements[0].bounds.as_ref().map(|rect| rect.width),
        Some(320.0)
    );
    assert!(elements[0].state_flags.iter().any(|flag| flag == "focused"));
    assert_eq!(elements[1].role, "x11_container");
    assert!(
        elements[1]
            .state_flags
            .iter()
            .any(|flag| flag == "container")
    );
    assert_eq!(elements[2].role, "x11_action_region");
    assert_eq!(elements[2].parent_index, Some(1));
    assert!(
        elements[2]
            .state_flags
            .iter()
            .any(|flag| flag == "action_like")
    );
}

#[test]
fn registry_fallback_prefers_refreshed_x11_child_regions() {
    let linux_window = LinuxWindowInfo {
        window_id: "0x3800030".to_string(),
        title: Some("sky-cua xmessage probe".to_string()),
        app_id: Some("xmessage.desktop".to_string()),
        wm_class: Some("Xmessage".to_string()),
        pid: None,
        bounds: Some(RectF {
            x: 100.0,
            y: 200.0,
            width: 320.0,
            height: 180.0,
            space: CoordinateSpace::DesktopLogical,
        }),
        display: None,
        display_intersections: Vec::new(),
        workspace: None,
        focused: true,
        hidden: false,
        client_type: Some("xwayland".to_string()),
        backend: "x11".to_string(),
        terminal: None,
        terminal_target_sessions: Vec::new(),
    };
    let x11_window = X11WindowInfo {
        window_id: "0x3800030".to_string(),
        instance_name: Some("xmessage".to_string()),
        class_name: Some("Xmessage".to_string()),
        app: app_from_linux_window(&linux_window),
        bounds: linux_window.bounds.clone(),
        workspace: None,
        child_regions: vec![X11WindowRegion {
            window_id: "0x3800032".to_string(),
            parent_window_id: Some("0x3800030".to_string()),
            depth: 1,
            name: Some("OK".to_string()),
            bounds: RectF {
                x: 180.0,
                y: 330.0,
                width: 48.0,
                height: 24.0,
                space: CoordinateSpace::DesktopLogical,
            },
        }],
    };

    let elements = fallback_window_elements_with_x11_detail(&linux_window, Some(&x11_window));

    assert_eq!(elements.len(), 2);
    assert_eq!(elements[1].role, "x11_action_region");
    assert_eq!(elements[1].parent_index, Some(0));
}

#[test]
fn emits_only_the_honest_window_anchor_for_kwin_fallback_windows() {
    let window = LinuxWindowInfo {
        window_id: "kwin:{tidal-window}".to_string(),
        title: Some("TIDAL Hi-Fi".to_string()),
        app_id: Some("tidal-hifi.desktop".to_string()),
        wm_class: Some("TIDAL".to_string()),
        pid: Some(4242),
        bounds: Some(RectF {
            x: 100.0,
            y: 80.0,
            width: 1280.0,
            height: 820.0,
            space: CoordinateSpace::DesktopLogical,
        }),
        display: None,
        display_intersections: Vec::new(),
        workspace: None,
        focused: true,
        hidden: false,
        client_type: Some("wayland".to_string()),
        backend: "kwin".to_string(),
        terminal: None,
        terminal_target_sessions: Vec::new(),
    };

    let elements = linux_window_elements(&window);
    // Only the real window bounds are surfaced. No synthetic media-player
    // geometry ("sidebar", "row band", "action cluster") is fabricated,
    // because inventing sub-elements a Wayland app never exposed misleads
    // the agent into semantic targeting that cannot work; the honest signal
    // is the screenshot + snapshot_id pixel path.
    assert_eq!(elements.len(), 1);
    let anchor = &elements[0];
    assert_eq!(anchor.role, "window");
    assert_eq!(anchor.parent_index, None);
    assert_eq!(
        anchor.bounds.as_ref().map(|bounds| bounds.width),
        Some(1280.0)
    );
    assert!(
        anchor
            .state_flags
            .iter()
            .any(|flag| flag == "vision_anchor")
    );
    assert!(
        anchor
            .state_flags
            .iter()
            .any(|flag| flag == "physical_target")
    );
    assert!(anchor.semantic_actions.is_empty());
    // The description must steer the agent to the snapshot_id pixel path.
    let description = anchor.description.as_deref().unwrap_or_default();
    assert!(description.contains("snapshot_id"));
    assert!(description.contains("screenshot"));
    // No fabricated candidate roles survive.
    assert!(
        !elements
            .iter()
            .any(|element| element.role.starts_with("wayland_"))
    );
}

#[test]
fn linux_fallback_snapshot_preserves_doctor_report() {
    let environment = wayland_pipewire_environment();
    let capabilities = LinuxDesktopBackend::capabilities(&environment);
    assert_eq!(
        capabilities.supported_scroll_directions,
        vec![
            ScrollDirection::Up,
            ScrollDirection::Down,
            ScrollDirection::Left,
            ScrollDirection::Right,
        ]
    );
    let report =
        crate::doctor::build_doctor_report(environment.clone(), DoctorSessionEnvReport::default());
    let window = LinuxWindowInfo {
        window_id: "kwin:{tidal-window}".to_string(),
        title: Some("TIDAL Hi-Fi".to_string()),
        app_id: Some("tidal-hifi.desktop".to_string()),
        wm_class: Some("TIDAL".to_string()),
        pid: Some(4242),
        bounds: Some(RectF {
            x: 100.0,
            y: 80.0,
            width: 1280.0,
            height: 820.0,
            space: CoordinateSpace::DesktopLogical,
        }),
        display: None,
        display_intersections: Vec::new(),
        workspace: None,
        focused: true,
        hidden: false,
        client_type: Some("wayland".to_string()),
        backend: "kwin".to_string(),
        terminal: None,
        terminal_target_sessions: Vec::new(),
    };

    let snapshot = linux_fallback_snapshot(
        "snap-1".to_string(),
        environment,
        capabilities,
        None,
        DiagnosticBuilder::new(),
        Some(report.clone()),
        window,
    );

    assert_eq!(snapshot.doctor_report, Some(report));
}

#[test]
fn registry_window_app_does_not_invent_executable() {
    let app = app_from_linux_window(&LinuxWindowInfo {
        window_id: "kwin:{tidal-window}".to_string(),
        title: Some("TIDAL Hi-Fi".to_string()),
        app_id: Some("tidal-hifi.desktop".to_string()),
        wm_class: Some("TIDAL".to_string()),
        pid: Some(4242),
        bounds: None,
        display: None,
        display_intersections: Vec::new(),
        workspace: None,
        focused: true,
        hidden: false,
        client_type: Some("wayland".to_string()),
        backend: "kwin".to_string(),
        terminal: None,
        terminal_target_sessions: Vec::new(),
    });

    assert_eq!(app.desktop_file_id.as_deref(), Some("tidal-hifi.desktop"));
    assert_eq!(app.executable, None);
}

#[test]
fn matches_x11_window_to_accessible_app_by_class_when_titles_do_not_help() {
    let app = AppInfo {
        app_id: "accessible-2".to_string(),
        name: "Code".to_string(),
        pid: None,
        executable: Some("code".to_string()),
        desktop_file_id: Some("code.desktop".to_string()),
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: Some("Electron".to_string()),
        window_title: Some("workspace-a".to_string()),
        is_focused_candidate: false,
    };
    let window = X11WindowInfo {
        window_id: "0x500001".to_string(),
        instance_name: Some("code".to_string()),
        class_name: Some("Code".to_string()),
        app: AppInfo {
            app_id: "x11:0x500001".to_string(),
            name: "Code".to_string(),
            pid: None,
            executable: None,
            desktop_file_id: None,
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("totally different title".to_string()),
            is_focused_candidate: false,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };

    assert!(x11_window_matches_app(&window, &app));
}

#[test]
fn does_not_match_an_x11_window_by_title_alone() {
    let app = AppInfo {
        app_id: "accessible-2b".to_string(),
        name: "kaccess".to_string(),
        pid: None,
        executable: Some("kaccess".to_string()),
        desktop_file_id: Some("kaccess.desktop".to_string()),
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: Some("Qt".to_string()),
        window_title: Some("sky-cua xmessage probe".to_string()),
        is_focused_candidate: false,
    };
    let window = X11WindowInfo {
        window_id: "0x500002".to_string(),
        instance_name: Some("xmessage".to_string()),
        class_name: Some("Xmessage".to_string()),
        app: AppInfo {
            app_id: "x11:0x500002".to_string(),
            name: "Xmessage".to_string(),
            pid: None,
            executable: Some("xmessage".to_string()),
            desktop_file_id: Some("xmessage.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("sky-cua xmessage probe".to_string()),
            is_focused_candidate: true,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };

    assert!(!x11_window_matches_app(&window, &app));
}

#[test]
fn selector_prefers_exact_window_title_over_broader_desktop_match() {
    let selector = AppSelector {
        app_id: None,
        desktop_file_id: Some("xmessage.desktop".to_string()),
        window_title: Some("selector beta".to_string()),
        name: None,
    };
    let alpha = X11WindowInfo {
        window_id: "0x500010".to_string(),
        instance_name: Some("xmessage".to_string()),
        class_name: Some("Xmessage".to_string()),
        app: AppInfo {
            app_id: "x11:0x500010".to_string(),
            name: "Xmessage".to_string(),
            pid: None,
            executable: Some("xmessage".to_string()),
            desktop_file_id: Some("xmessage.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("sky-cua selector alpha".to_string()),
            is_focused_candidate: true,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };
    let beta = X11WindowInfo {
        window_id: "0x500011".to_string(),
        instance_name: Some("xmessage".to_string()),
        class_name: Some("Xmessage".to_string()),
        app: AppInfo {
            app_id: "x11:0x500011".to_string(),
            name: "Xmessage".to_string(),
            pid: None,
            executable: Some("xmessage".to_string()),
            desktop_file_id: Some("xmessage.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("sky-cua selector beta".to_string()),
            is_focused_candidate: false,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };

    let matched =
        select_x11_window(&[alpha, beta.clone()], &selector).expect("selector should match");
    assert_eq!(matched.window_id, beta.window_id);
}

#[test]
fn selector_prefers_focused_x11_window_when_only_desktop_id_is_given() {
    let selector = AppSelector {
        app_id: None,
        desktop_file_id: Some("discord.desktop".to_string()),
        window_title: None,
        name: None,
    };
    let background = X11WindowInfo {
        window_id: "0x500012".to_string(),
        instance_name: Some("discord".to_string()),
        class_name: Some("discord".to_string()),
        app: AppInfo {
            app_id: "x11:0x500012".to_string(),
            name: "discord".to_string(),
            pid: None,
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("Friends - Discord".to_string()),
            is_focused_candidate: false,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };
    let focused = X11WindowInfo {
        window_id: "0x500013".to_string(),
        instance_name: Some("discord".to_string()),
        class_name: Some("discord".to_string()),
        app: AppInfo {
            app_id: "x11:0x500013".to_string(),
            name: "discord".to_string(),
            pid: None,
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("Project Foxglove - Discord".to_string()),
            is_focused_candidate: true,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };

    let matched = select_x11_window(&[background, focused.clone()], &selector)
        .expect("selector should match");
    assert_eq!(matched.window_id, focused.window_id);
}

#[test]
fn prefers_the_best_x11_window_match_when_multiple_windows_share_a_process() {
    let app = AppInfo {
        app_id: "accessible-3".to_string(),
        name: "Discord".to_string(),
        pid: Some(4321),
        executable: Some("discord".to_string()),
        desktop_file_id: Some("discord.desktop".to_string()),
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: Some("Electron".to_string()),
        window_title: Some("Project Foxglove - Discord".to_string()),
        is_focused_candidate: false,
    };
    let weaker = X11WindowInfo {
        window_id: "0x600001".to_string(),
        instance_name: Some("discord".to_string()),
        class_name: Some("discord".to_string()),
        app: AppInfo {
            app_id: "x11:0x600001".to_string(),
            name: "discord".to_string(),
            pid: Some(4321),
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("Friends - Discord".to_string()),
            is_focused_candidate: false,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };
    let stronger = X11WindowInfo {
        window_id: "0x600002".to_string(),
        instance_name: Some("discord".to_string()),
        class_name: Some("discord".to_string()),
        app: AppInfo {
            app_id: "x11:0x600002".to_string(),
            name: "discord".to_string(),
            pid: Some(4321),
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("Project Foxglove - Discord".to_string()),
            is_focused_candidate: true,
        },
        bounds: None,
        workspace: None,
        child_regions: Vec::new(),
    };

    let windows = [weaker.clone(), stronger.clone()];
    let matched = best_x11_window_match(&windows, &app).expect("a best match should be found");
    assert_eq!(matched.window_id, stronger.window_id);
}
