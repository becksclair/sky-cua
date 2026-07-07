use super::*;

pub(super) fn linux_fallback_snapshot(
    snapshot_id: String,
    environment: EnvironmentInfo,
    capabilities: ToolCapabilities,
    capture: Option<sky_cua_platform::model::CaptureInfo>,
    diagnostics: DiagnosticBuilder,
    doctor_report: Option<DoctorReport>,
    window: linux_windowing::LinuxWindowInfo,
) -> AppStateSnapshot {
    AppStateSnapshot {
        snapshot_id,
        created_at: chrono::Utc::now(),
        environment,
        capabilities,
        focused_app: Some(LinuxDesktopBackend::focused_from_linux_window(&window)),
        capture,
        elements: fallback_window_elements(&window),
        diagnostics: diagnostics.finish(),
        app_guidance: None,
        doctor_report,
        agent_cursor: None,
    }
}

fn fallback_window_elements(window: &linux_windowing::LinuxWindowInfo) -> Vec<ElementNode> {
    let x11_window = refreshed_x11_window_for_linux_window(window);
    fallback_window_elements_with_x11_detail(window, x11_window.as_ref())
}

pub(super) fn fallback_window_elements_with_x11_detail(
    window: &linux_windowing::LinuxWindowInfo,
    x11_window: Option<&X11WindowInfo>,
) -> Vec<ElementNode> {
    x11_window
        .map(x11_window_elements)
        .filter(|elements| !elements.is_empty())
        .unwrap_or_else(|| linux_window_elements(window))
}

fn refreshed_x11_window_for_linux_window(
    window: &linux_windowing::LinuxWindowInfo,
) -> Option<X11WindowInfo> {
    if window.backend != "x11" {
        return None;
    }
    windowing::discover_windows()
        .ok()?
        .into_iter()
        .find(|candidate| candidate.window_id == window.window_id)
}

pub(super) fn linux_window_elements(window: &linux_windowing::LinuxWindowInfo) -> Vec<ElementNode> {
    let Some(bounds) = window.bounds.clone() else {
        return Vec::new();
    };

    let mut root_state_flags = vec![
        "native_window_fallback".to_string(),
        "physical_target".to_string(),
        "vision_anchor".to_string(),
        "container".to_string(),
        "content_like".to_string(),
    ];
    if window.focused {
        root_state_flags.push("focused".to_string());
        root_state_flags.push("active".to_string());
    }
    let app = app_from_linux_window(window);

    let elements = vec![ElementNode {
        element_index: 0,
        parent_index: None,
        role: "window".to_string(),
        name: app.window_title.clone().or_else(|| Some(app.name.clone())),
        description: Some(format!(
            "{} window surfaced from the window registry without a matching AT-SPI tree, so no semantic elements are available for this window. Do not guess at sub-elements: capture this window, read the target's pixel position off the screenshot, and click with desktop_pointer using this capture's snapshot_id plus those x/y pixels (the snapshot_id translates screenshot pixels to the screen for you).",
            window.backend
        )),
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags: root_state_flags,
        semantic_actions: Vec::new(),
        bounds: Some(bounds.clone()),
        backend_ref: None,
    }];

    elements
}

pub(super) fn x11_window_elements(window: &X11WindowInfo) -> Vec<ElementNode> {
    let Some(bounds) = window.bounds.clone() else {
        return Vec::new();
    };

    let mut state_flags = Vec::new();
    if window.app.is_focused_candidate {
        state_flags.push("focused".to_string());
        state_flags.push("active".to_string());
    }
    state_flags.push("native_window_fallback".to_string());
    state_flags.push("x11_fallback".to_string());
    state_flags.push("physical_target".to_string());

    let mut elements = vec![ElementNode {
        element_index: 0,
        parent_index: None,
        role: "window".to_string(),
        name: window
            .app
            .window_title
            .clone()
            .or_else(|| Some(window.app.name.clone())),
        description: Some(
            "X11/XWayland window surfaced without a matching AT-SPI tree; physical actions can still target its bounds"
                .to_string(),
        ),
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags,
        semantic_actions: Vec::new(),
        bounds: Some(bounds.clone()),
        backend_ref: None,
    }];

    let child_counts = window.child_regions.iter().fold(
        std::collections::HashMap::<String, usize>::new(),
        |mut counts, region| {
            if let Some(parent_window_id) = region.parent_window_id.as_ref() {
                *counts.entry(parent_window_id.clone()).or_default() += 1;
            }
            counts
        },
    );
    let mut index_by_window_id =
        std::collections::HashMap::from([(window.window_id.clone(), 0usize)]);
    for region in &window.child_regions {
        if region.bounds.width < 8.0 || region.bounds.height < 8.0 {
            continue;
        }

        let parent_index = region
            .parent_window_id
            .as_ref()
            .and_then(|window_id| index_by_window_id.get(window_id).copied())
            .or(Some(0));
        let element_index = elements.len();
        let has_children = child_counts
            .get(&region.window_id)
            .copied()
            .unwrap_or_default()
            > 0;
        let role = x11_region_role(region, has_children, &bounds);
        let mut state_flags = vec!["x11_fallback".to_string(), "physical_target".to_string()];
        if has_children {
            state_flags.push("container".to_string());
        } else {
            state_flags.push("leaf".to_string());
        }
        if role == "x11_action_region" {
            state_flags.push("action_like".to_string());
        }
        elements.push(ElementNode {
            element_index,
            parent_index,
            role: role.to_string(),
            name: region.name.clone(),
            description: Some(x11_region_description(region, role)),
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags,
            semantic_actions: Vec::new(),
            bounds: Some(region.bounds.clone()),
            backend_ref: None,
        });
        index_by_window_id.insert(region.window_id.clone(), element_index);
    }

    elements
}

fn x11_region_role(
    region: &crate::x11::windowing::X11WindowRegion,
    has_children: bool,
    root_bounds: &sky_cua_platform::model::RectF,
) -> &'static str {
    if has_children {
        return "x11_container";
    }

    let center_y = region.bounds.y + (region.bounds.height / 2.0);
    let root_mid_y = root_bounds.y + (root_bounds.height / 2.0);
    let small_relative_width = region.bounds.width <= root_bounds.width * 0.4;
    let small_relative_height = region.bounds.height <= root_bounds.height * 0.5;
    if center_y >= root_mid_y && small_relative_width && small_relative_height {
        "x11_action_region"
    } else {
        "x11_leaf_region"
    }
}

fn x11_region_description(region: &crate::x11::windowing::X11WindowRegion, role: &str) -> String {
    let role_hint = match role {
        "x11_container" => "container-like region",
        "x11_action_region" => "small lower leaf region that may behave like an actionable control",
        _ => "leaf region",
    };
    format!(
        "Recovered from the X11 window tree at depth {} as a {}; physical actions can target this region, but no semantic AT-SPI interface is available",
        region.depth, role_hint
    )
}

pub(super) fn window_summary(app: &AppInfo) -> String {
    selector_summary(&AppSelector {
        app_id: Some(app.app_id.clone()),
        desktop_file_id: app.desktop_file_id.clone(),
        window_title: app.window_title.clone(),
        name: Some(app.name.clone()),
    })
}

pub(super) fn selector_or_window_summary(selector: Option<&AppSelector>, app: &AppInfo) -> String {
    match selector {
        Some(selector) => format!(
            "{}, matched_x11_window={}",
            selector_summary(selector),
            window_summary(app)
        ),
        None => window_summary(app),
    }
}
