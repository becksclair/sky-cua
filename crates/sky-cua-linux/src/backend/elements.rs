use super::*;

pub(super) fn normalize_correlated_atspi_bounds(
    elements: &mut [ElementNode],
    window_bounds: Option<&RectF>,
) -> (usize, bool) {
    let Some(window_bounds) = window_bounds.filter(|bounds| {
        bounds.space == sky_cua_platform::model::CoordinateSpace::DesktopLogical
            && bounds.width > 0.0
            && bounds.height > 0.0
    }) else {
        return (0, false);
    };
    let Some(root_index) = elements
        .iter()
        .position(|element| element.parent_index.is_none())
    else {
        return (0, false);
    };
    let Some(root_bounds) = elements[root_index].bounds.as_ref() else {
        return (0, false);
    };
    if root_bounds.space != sky_cua_platform::model::CoordinateSpace::DesktopLogical {
        return (0, false);
    }

    if let Some((scale_x, scale_y)) = correlated_coordinate_scale(root_bounds, window_bounds) {
        let root_x = root_bounds.x;
        let root_y = root_bounds.y;
        for element in elements.iter_mut() {
            let Some(bounds) = element.bounds.as_mut() else {
                continue;
            };
            bounds.x = window_bounds.x + (bounds.x - root_x) * scale_x;
            bounds.y = window_bounds.y + (bounds.y - root_y) * scale_y;
            bounds.width *= scale_x;
            bounds.height *= scale_y;
        }
        elements[root_index].bounds = Some(window_bounds.clone());
        return (0, true);
    }
    if !dimensions_approximately_match(root_bounds, window_bounds) {
        return (0, false);
    }

    let positioned_count = elements
        .iter()
        .filter(|element| element.bounds.is_some())
        .count();
    let zero_origin_count = elements
        .iter()
        .filter_map(|element| element.bounds.as_ref())
        .filter(|bounds| near_zero(bounds.x) && near_zero(bounds.y))
        .count();
    if zero_origin_count < 2 || zero_origin_count * 2 < positioned_count {
        return (0, false);
    }

    let translate_local_coordinates = near_zero(root_bounds.x)
        && near_zero(root_bounds.y)
        && (!near_zero(window_bounds.x) || !near_zero(window_bounds.y));
    let offset_x = window_bounds.x - root_bounds.x;
    let offset_y = window_bounds.y - root_bounds.y;
    let mut omitted = 0;
    for (index, element) in elements.iter_mut().enumerate() {
        if index == root_index {
            element.bounds = Some(window_bounds.clone());
            continue;
        }
        let Some(bounds) = element.bounds.as_mut() else {
            continue;
        };
        if near_zero(bounds.x) && near_zero(bounds.y) {
            element.bounds = None;
            omitted += 1;
        } else if translate_local_coordinates {
            bounds.x += offset_x;
            bounds.y += offset_y;
        }
    }
    (omitted, false)
}

fn correlated_coordinate_scale(root: &RectF, window: &RectF) -> Option<(f64, f64)> {
    let scale_x = window.width / root.width;
    let scale_y = window.height / root.height;
    let uniform_tolerance = scale_x.abs().max(scale_y.abs()) * 0.02;
    (scale_x.is_finite()
        && scale_y.is_finite()
        && (0.25..=4.0).contains(&scale_x)
        && (0.25..=4.0).contains(&scale_y)
        && (scale_x - scale_y).abs() <= uniform_tolerance
        && (scale_x - 1.0).abs() > 0.02)
        .then_some((scale_x, scale_y))
}

fn dimensions_approximately_match(left: &RectF, right: &RectF) -> bool {
    let width_tolerance = (right.width * 0.02).max(2.0);
    let height_tolerance = (right.height * 0.02).max(2.0);
    (left.width - right.width).abs() <= width_tolerance
        && (left.height - right.height).abs() <= height_tolerance
}

fn near_zero(value: f64) -> bool {
    value.abs() <= 0.5
}

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

pub(super) fn fallback_window_elements(
    window: &linux_windowing::LinuxWindowInfo,
) -> Vec<ElementNode> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::CoordinateSpace;

    fn element(index: usize, parent_index: Option<usize>, bounds: RectF) -> ElementNode {
        ElementNode {
            element_index: index,
            parent_index,
            role: "fixture".to_string(),
            name: None,
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: Vec::new(),
            semantic_actions: Vec::new(),
            bounds: Some(bounds),
            backend_ref: None,
        }
    }

    fn bounds(x: f64, y: f64, width: f64, height: f64) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
            space: CoordinateSpace::DesktopLogical,
        }
    }

    #[test]
    fn zero_origin_gtk_tree_keeps_window_root_and_omits_unsafe_children() {
        let window = bounds(670.0, 409.0, 366.0, 248.0);
        let mut elements = vec![
            element(0, None, bounds(0.0, 0.0, 366.0, 248.0)),
            element(1, Some(0), bounds(0.0, 0.0, 294.0, 57.0)),
            element(2, Some(1), bounds(0.0, 0.0, 153.0, 44.0)),
        ];

        assert_eq!(
            normalize_correlated_atspi_bounds(&mut elements, Some(&window)),
            (2, false)
        );
        assert_eq!(elements[0].bounds.as_ref(), Some(&window));
        assert_eq!(elements[1].bounds, None);
        assert_eq!(elements[2].bounds, None);
    }

    #[test]
    fn valid_screen_coordinates_are_preserved() {
        let window = bounds(670.0, 409.0, 366.0, 248.0);
        let original = vec![
            element(0, None, window.clone()),
            element(1, Some(0), bounds(700.0, 450.0, 294.0, 57.0)),
            element(2, Some(1), bounds(850.0, 590.0, 153.0, 44.0)),
        ];
        let mut elements = original.clone();

        assert_eq!(
            normalize_correlated_atspi_bounds(&mut elements, Some(&window)),
            (0, false)
        );
        assert_eq!(elements, original);
    }

    #[test]
    fn xwayland_physical_tree_is_scaled_to_compositor_logical_bounds() {
        let window = bounds(0.0, 0.0, 1706.6666666667, 1066.6666666667);
        let mut elements = vec![
            element(0, None, bounds(0.0, 0.0, 2560.0, 1600.0)),
            element(1, Some(0), bounds(678.0, 162.0, 590.0, 314.0)),
        ];

        assert_eq!(
            normalize_correlated_atspi_bounds(&mut elements, Some(&window)),
            (0, true)
        );
        assert_eq!(elements[0].bounds.as_ref(), Some(&window));
        let button = elements[1].bounds.as_ref().expect("button bounds");
        assert!((button.x - 452.0).abs() < 0.01);
        assert!((button.y - 108.0).abs() < 0.01);
        assert!((button.width - 393.3333333333).abs() < 0.01);
        assert!((button.height - 209.3333333333).abs() < 0.01);
    }
}
