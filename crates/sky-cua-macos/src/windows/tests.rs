use sky_cua_platform::diagnostics::BackendErrorCode;
use sky_cua_platform::model::{
    AppSelector, CoordinateSpace, DisplayInfo, PixelSize, RectF, WindowTarget,
};

use super::*;
use crate::coords::desktop_rect;

fn window(window_id: u32, pid: i32, owner: &str, bundle: Option<&str>, title: &str) -> MacWindow {
    MacWindow {
        window_id,
        pid,
        owner_name: owner.to_string(),
        bundle_id: bundle.map(ToOwned::to_owned),
        executable: Some(format!("/Applications/{owner}.app/Contents/MacOS/{owner}")),
        title: (!title.is_empty()).then(|| title.to_string()),
        bounds: desktop_rect(0.0, 0.0, 800.0, 600.0),
        layer: 0,
        on_screen: true,
        focused: false,
        z_index: 0,
    }
}

fn in_order(mut windows: Vec<MacWindow>) -> Vec<MacWindow> {
    for (z_index, window) in windows.iter_mut().enumerate() {
        window.z_index = z_index;
    }
    windows
}

fn display(id: &str, index: u32, primary: bool, rect: [f64; 4]) -> DisplayInfo {
    DisplayInfo {
        display_id: id.to_string(),
        name: None,
        index,
        primary,
        logical_rect: RectF {
            x: rect[0],
            y: rect[1],
            width: rect[2],
            height: rect[3],
            space: CoordinateSpace::DesktopLogical,
        },
        pixel_size: Some(PixelSize {
            width: (rect[2] * 2.0) as u32,
            height: (rect[3] * 2.0) as u32,
        }),
        scale_factor: Some(2.0),
        backend: WINDOW_BACKEND.to_string(),
    }
}

#[test]
fn focus_marks_only_the_front_window_of_the_front_process() {
    let mut windows = in_order(vec![
        window(1, 10, "Notes", Some("com.apple.Notes"), "A"),
        window(2, 20, "Safari", Some("com.apple.Safari"), "B"),
        window(3, 20, "Safari", Some("com.apple.Safari"), "C"),
    ]);
    mark_focused(&mut windows, Some(20));
    let focused: Vec<u32> = windows
        .iter()
        .filter(|window| window.focused)
        .map(|window| window.window_id)
        .collect();
    assert_eq!(focused, vec![2]);
}

#[test]
fn front_process_without_windows_focuses_nothing() {
    let mut windows = in_order(vec![window(1, 10, "Notes", None, "A")]);
    mark_focused(&mut windows, Some(99));
    assert!(windows.iter().all(|window| !window.focused));
    mark_focused(&mut windows, None);
    assert!(windows.iter().all(|window| !window.focused));
}

#[test]
fn model_uses_decimal_window_id_and_bundle_app_id() {
    let mut safari = window(4242, 20, "Safari", Some("com.apple.Safari"), "Start Page");
    safari.focused = true;
    let model = window_to_model(&safari, &[]);
    assert_eq!(model.window_id, "4242");
    assert_eq!(model.app_id.as_deref(), Some("com.apple.Safari"));
    assert_eq!(model.wm_class.as_deref(), Some("Safari"));
    assert_eq!(model.pid, Some(20));
    assert!(model.focused);
    assert_eq!(model.backend, WINDOW_BACKEND);

    let app = window_to_app(&safari);
    assert_eq!(app.app_id, "com.apple.Safari");
    assert_eq!(app.name, "Safari");
    assert_eq!(app.window_handle.as_deref(), Some("4242"));
    assert!(app.is_focused_candidate);

    let unbundled = window(7, 30, "tool", None, "");
    assert_eq!(window_to_app(&unbundled).app_id, "tool");
    assert_eq!(window_to_model(&unbundled, &[]).title, None);
}

#[test]
fn assigns_windows_to_largest_display_intersection() {
    let displays = vec![
        display("1", 0, true, [0.0, 0.0, 1728.0, 1117.0]),
        display("2", 1, false, [-1280.0, 0.0, 1280.0, 720.0]),
    ];
    let mut tool = window(5, 22, "Tool", None, "Tool");
    tool.bounds = desktop_rect(-900.0, 10.0, 1000.0, 500.0);

    let model = window_to_model(&tool, &displays);
    assert_eq!(
        model
            .display
            .as_ref()
            .map(|display| display.display_id.as_str()),
        Some("2")
    );
    assert_eq!(model.display_intersections.len(), 2);
    assert_eq!(
        window_to_focused_app(&tool, &displays)
            .display
            .map(|display| display.display_id),
        Some("2".to_string())
    );
}

#[test]
fn select_window_prefers_the_front_most_match() {
    let windows = in_order(vec![
        window(1, 10, "Notes", Some("com.apple.Notes"), "Shopping"),
        window(2, 20, "Safari", Some("com.apple.Safari"), "Docs"),
        window(3, 20, "Safari", Some("com.apple.Safari"), "Mail"),
    ]);
    let by_app = select_window(
        &windows,
        &AppSelector {
            app_id: Some("COM.APPLE.SAFARI".to_string()),
            ..AppSelector::default()
        },
    );
    assert_eq!(by_app.map(|window| window.window_id), Some(2));

    let by_name = select_window(
        &windows,
        &AppSelector {
            name: Some("notes".to_string()),
            ..AppSelector::default()
        },
    );
    assert_eq!(by_name.map(|window| window.window_id), Some(1));

    let by_title = select_window(
        &windows,
        &AppSelector {
            window_title: Some("Mail".to_string()),
            ..AppSelector::default()
        },
    );
    assert_eq!(by_title.map(|window| window.window_id), Some(3));

    assert!(select_window(&windows, &AppSelector::default()).is_none());
}

#[test]
fn select_window_name_prefers_identity_over_a_front_title_match() {
    let windows = in_order(vec![
        window(
            1,
            10,
            "TextEdit",
            Some("com.apple.TextEdit"),
            "chrome notes.txt",
        ),
        window(2, 20, "Google Chrome", Some("com.google.Chrome"), "Inbox"),
    ]);
    let selector = AppSelector {
        name: Some("chrome".to_string()),
        ..AppSelector::default()
    };
    assert_eq!(
        select_window(&windows, &selector).map(|window| window.window_id),
        Some(2)
    );

    // With no identity match anywhere, `name` still falls back to the title.
    let selector = AppSelector {
        name: Some("NOTES.TXT".to_string()),
        ..AppSelector::default()
    };
    assert_eq!(
        select_window(&windows, &selector).map(|window| window.window_id),
        Some(1)
    );
}

#[test]
fn select_window_name_prefers_an_exact_identity_over_a_front_partial_one() {
    let windows = in_order(vec![
        window(1, 10, "Xcode", Some("com.apple.dt.Xcode"), "App.swift"),
        window(2, 20, "Code", Some("com.microsoft.VSCode"), "main.rs"),
    ]);
    let select = |name: &str| {
        select_window(
            &windows,
            &AppSelector {
                name: Some(name.to_string()),
                ..AppSelector::default()
            },
        )
        .map(|window| window.window_id)
    };

    assert_eq!(select("code"), Some(2));
    assert_eq!(select("com.apple.dt.xcode"), Some(1));
    // No application is named exactly this, so the front-most partial
    // identity match still wins.
    assert_eq!(select("cod"), Some(1));
}

#[test]
fn select_window_requires_every_supplied_field() {
    let windows = in_order(vec![
        window(1, 10, "Mail", Some("com.apple.mail"), "Inbox"),
        window(2, 20, "Safari", Some("com.apple.Safari"), "Docs"),
        window(3, 20, "Safari", Some("com.apple.Safari"), "Inbox — Webmail"),
    ]);
    let selector = AppSelector {
        app_id: Some("com.apple.Safari".to_string()),
        window_title: Some("Inbox".to_string()),
        ..AppSelector::default()
    };
    assert_eq!(
        select_window(&windows, &selector).map(|window| window.window_id),
        Some(3)
    );

    let no_safari_inbox = &windows[..2];
    assert!(select_window(no_safari_inbox, &selector).is_none());

    // The title is matched as a case-insensitive substring.
    for title in ["inbox", "WEBMAIL", "Inbox — Webmail"] {
        let any_case = AppSelector {
            app_id: Some("com.apple.Safari".to_string()),
            window_title: Some(title.to_string()),
            ..AppSelector::default()
        };
        assert_eq!(
            select_window(&windows, &any_case).map(|window| window.window_id),
            Some(3),
            "{title}"
        );
    }

    let desktop_file_only = AppSelector {
        desktop_file_id: Some("org.example.App.desktop".to_string()),
        ..AppSelector::default()
    };
    assert!(select_window(&windows, &desktop_file_only).is_none());
}

#[test]
fn resolves_exact_window_id_before_broad_metadata() {
    let windows = vec![
        window(
            0x10,
            10,
            "Shared",
            Some("com.example.shared"),
            "Shared Title",
        ),
        window(0x20, 20, "Other", Some("com.example.other"), "Shared Title"),
    ];

    let resolved = resolve_window_target(
        &windows,
        &WindowTarget {
            window_id: Some("32".to_string()),
            title: Some("Shared Title".to_string()),
            wm_class: Some("shared".to_string()),
            ..WindowTarget::default()
        },
    )
    .unwrap();

    assert_eq!(resolved.window_id, 0x20);
}

#[test]
fn reports_ambiguous_macos_title_targets() {
    let windows = vec![
        window(0x10, 10, "A", None, "Shared Title"),
        window(0x20, 20, "B", None, "Shared Title"),
    ];

    let error = resolve_window_target(
        &windows,
        &WindowTarget {
            title: Some("Shared Title".to_string()),
            ..WindowTarget::default()
        },
    )
    .unwrap_err();

    assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
    assert!(
        error
            .message
            .contains("matched multiple macOS windows (16, 32)")
    );
}

#[test]
fn resolves_macos_title_substrings_case_insensitively() {
    let windows = vec![window(0x10, 10, "TextEdit", None, "Untitled — TextEdit")];

    let resolved = resolve_window_target(
        &windows,
        &WindowTarget {
            title: Some("textedit".to_string()),
            ..WindowTarget::default()
        },
    )
    .unwrap();

    assert_eq!(resolved.window_id, 0x10);
}

#[test]
fn app_id_and_pid_targets_fail_closed_when_ambiguous() {
    let windows = vec![
        window(1, 20, "Safari", Some("com.apple.Safari"), "Docs"),
        window(2, 20, "Safari", Some("com.apple.Safari"), "Mail"),
        window(3, 10, "Notes", Some("com.apple.Notes"), "List"),
    ];
    let ambiguous = resolve_window_target(
        &windows,
        &WindowTarget {
            app_id: Some("com.apple.safari".to_string()),
            ..WindowTarget::default()
        },
    )
    .unwrap_err();
    assert!(ambiguous.message.contains("add window_id to disambiguate"));

    let by_pid = resolve_window_target(
        &windows,
        &WindowTarget {
            pid: Some(10),
            ..WindowTarget::default()
        },
    )
    .unwrap();
    assert_eq!(by_pid.window_id, 3);

    let by_owner = resolve_window_target(
        &windows,
        &WindowTarget {
            app_id: Some("Notes".to_string()),
            ..WindowTarget::default()
        },
    )
    .unwrap();
    assert_eq!(by_owner.window_id, 3);
}

#[test]
fn wm_class_matches_owner_bundle_or_executable() {
    let windows = vec![
        window(1, 20, "Safari", Some("com.apple.Safari"), "Docs"),
        window(3, 10, "Notes", Some("com.apple.Notes"), "List"),
    ];
    let resolved = resolve_window_target(
        &windows,
        &WindowTarget {
            wm_class: Some("APPLE.NOTES".to_string()),
            ..WindowTarget::default()
        },
    )
    .unwrap();
    assert_eq!(resolved.window_id, 3);
}

#[test]
fn unknown_and_empty_targets_are_invalid_requests() {
    let windows = vec![window(1, 20, "Safari", None, "Docs")];
    let missing = resolve_window_target(
        &windows,
        &WindowTarget {
            window_id: Some("999".to_string()),
            ..WindowTarget::default()
        },
    )
    .unwrap_err();
    assert_eq!(missing.message, "No macOS window matched window_id 999.");

    let empty = resolve_window_target(
        &windows,
        &WindowTarget {
            title: Some("   ".to_string()),
            ..WindowTarget::default()
        },
    )
    .unwrap_err();
    assert_eq!(empty.code, BackendErrorCode::InvalidRequest.as_str());
    assert!(empty.message.starts_with("Pass window_id"));
}

#[test]
fn application_window_filter_keeps_only_visible_normal_layer_windows() {
    let base = list::RawWindow {
        window_id: 1,
        pid: 1,
        owner_name: "App".to_string(),
        title: None,
        bounds: [0.0, 0.0, 400.0, 300.0],
        layer: 0,
        alpha: 1.0,
        on_screen: true,
    };
    assert!(list::is_application_window(&base));
    let menu_bar = list::RawWindow {
        layer: 24,
        ..base.clone()
    };
    let transparent = list::RawWindow {
        alpha: 0.0,
        ..base.clone()
    };
    let sliver = list::RawWindow {
        bounds: [0.0, 0.0, 1.0, 300.0],
        ..base.clone()
    };
    assert!(!list::is_application_window(&menu_bar));
    assert!(!list::is_application_window(&transparent));
    assert!(!list::is_application_window(&sliver));

    // A TextEdit autocorrect bubble: untitled, normal layer, a few dozen points.
    let suggestion_bubble = list::RawWindow {
        bounds: [180.0, 173.0, 82.0, 45.0],
        ..base.clone()
    };
    assert!(!list::is_application_window(&suggestion_bubble));
    // A titled window of the same size is a real, if small, window.
    let small_titled = list::RawWindow {
        title: Some("Colors".to_string()),
        ..suggestion_bubble
    };
    assert!(list::is_application_window(&small_titled));
}

mod live {
    use std::time::{Duration, Instant};

    use objc2_app_kit::NSWorkspace;
    use objc2_application_services::{AXError, AXUIElement};
    use objc2_core_foundation::{CFRetained, CFString, CFType};

    use super::super::*;

    fn displays() -> Vec<DisplayInfo> {
        crate::displays::enumerate_displays()
    }

    #[test]
    #[ignore = "live desktop probe"]
    fn live_enumerate_windows() {
        let displays = displays();
        let started = Instant::now();
        let windows = enumerate_windows();
        let first = started.elapsed();
        let started = Instant::now();
        let again = enumerate_windows();
        let second = started.elapsed();
        eprintln!(
            "enumerate_windows: {} windows, first call {first:?}, warm call {second:?} ({} windows)",
            windows.len(),
            again.len()
        );
        for window in &windows {
            let model = window_to_model(window, &displays);
            eprintln!(
                "z={:>2} id={:<6} pid={:<6} focused={:<5} app_id={:<40} title={:?} bounds=({}, {}, {}x{}) display={:?} exe={:?}",
                window.z_index,
                model.window_id,
                window.pid,
                window.focused,
                model.app_id.unwrap_or_default(),
                window.title,
                window.bounds.x,
                window.bounds.y,
                window.bounds.width,
                window.bounds.height,
                model.display.map(|display| display.display_id),
                window.executable,
            );
        }
        let titled = windows
            .iter()
            .filter(|window| window.title.is_some())
            .count();
        eprintln!("titled windows: {titled}/{}", windows.len());
        for raw in super::super::list::read_window_list() {
            if !super::super::list::is_application_window(&raw) {
                eprintln!(
                    "filtered: id={} pid={} owner={:?} layer={} alpha={} bounds={:?} title={:?}",
                    raw.window_id,
                    raw.pid,
                    raw.owner_name,
                    raw.layer,
                    raw.alpha,
                    raw.bounds,
                    raw.title
                );
            }
        }
        let mut timings = Vec::new();
        for _ in 0..20 {
            let started = Instant::now();
            let _ = enumerate_windows();
            timings.push(started.elapsed());
        }
        timings.sort();
        eprintln!(
            "20 warm calls: min {:?} median {:?} max {:?}",
            timings[0], timings[10], timings[19]
        );
    }

    /// Ground truth from a fresh process: `lsappinfo front`.
    fn lsappinfo_front_pid() -> Option<i32> {
        let front = std::process::Command::new("lsappinfo")
            .arg("front")
            .output()
            .ok()?;
        let asn = String::from_utf8_lossy(&front.stdout).trim().to_string();
        let info = std::process::Command::new("lsappinfo")
            .args(["info", "-only", "pid", &asn])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&info.stdout);
        let after = text.split("pid").nth(1)?.split('=').nth(1)?.trim_start();
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }

    fn ax_focused_application_pid(system: &AXUIElement) -> Option<i32> {
        let attribute = CFString::from_static_str("AXFocusedApplication");
        let mut value: *const CFType = std::ptr::null();
        // SAFETY: `value` is a valid out-pointer; a returned value is +1.
        let error =
            unsafe { system.copy_attribute_value(&attribute, std::ptr::NonNull::from(&mut value)) };
        if error != AXError::Success || value.is_null() {
            return None;
        }
        // SAFETY: AXFocusedApplication is an AXUIElement returned at +1.
        let element: CFRetained<AXUIElement> =
            unsafe { CFRetained::from_raw(std::ptr::NonNull::new(value.cast_mut())?.cast()) };
        let mut pid: libc::pid_t = 0;
        // SAFETY: `pid` is a valid out-pointer.
        let error = unsafe { element.pid(std::ptr::NonNull::from(&mut pid)) };
        (error == AXError::Success).then_some(pid)
    }

    /// Samples the front-most application through every candidate source in
    /// one long-lived process with no main run loop. `NSWorkspace` is read
    /// through the same shared instance throughout, as a daemon would.
    #[test]
    #[ignore = "live desktop probe"]
    fn live_front_process_tracks_without_run_loop() {
        let workspace = NSWorkspace::sharedWorkspace();
        // SAFETY: creating the system-wide element has no preconditions.
        let system = unsafe { AXUIElement::new_system_wide() };
        let samples: usize = std::env::var("PROBE_SAMPLES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(10);
        for sample in 0..samples {
            let started = Instant::now();
            let get_front = front_process_pid();
            let get_front_cost = started.elapsed();
            let truth = lsappinfo_front_pid();
            let ns_workspace = workspace
                .frontmostApplication()
                .map(|app| app.processIdentifier());
            let started = Instant::now();
            let ax = ax_focused_application_pid(&system);
            let ax_cost = started.elapsed();
            let windows = enumerate_windows();
            let focused = windows.iter().find(|window| window.focused);
            let top = windows.first();
            // Activation resolves the pid through NSRunningApplication; check
            // that it finds applications launched after this process started.
            let running = get_front.map(|pid| {
                NSRunningApplication::runningApplicationWithProcessIdentifier(pid).map(|app| {
                    app.bundleIdentifier()
                        .map_or_else(|| "<no bundle id>".to_string(), |id| id.to_string())
                })
            });
            eprintln!(
                "sample {sample}: lsappinfo={truth:?} GetFrontProcess={get_front:?} ({get_front_cost:?}) AX={ax:?} ({ax_cost:?}) NSWorkspace={ns_workspace:?} NSRunningApplication={running:?} focused={:?} top_window_owner={:?}",
                focused.map(|window| (
                    window.window_id,
                    window.owner_name.as_str(),
                    window.title.as_deref()
                )),
                top.map(|window| (window.pid, window.owner_name.as_str())),
            );
            std::thread::sleep(Duration::from_secs(3));
        }
    }
}
