use sky_cua_platform::AppInfo;
use sky_cua_platform::model::AppSelector;

use crate::apps::discovery::DiscoveredApp;
use crate::windowing as linux_windowing;
use crate::x11::windowing::X11WindowInfo;

pub(crate) fn select_app(apps: &[DiscoveredApp], selector: &AppSelector) -> Option<DiscoveredApp> {
    apps.iter()
        .filter_map(|app| selector_match_score(&app.info, selector).map(|score| (score, app)))
        .max_by_key(|(score, app)| (*score, app.info.is_focused_candidate))
        .map(|(_, app)| app.clone())
}

pub(crate) fn select_linux_window(
    windows: &[linux_windowing::LinuxWindowInfo],
    selector: &AppSelector,
) -> Option<linux_windowing::LinuxWindowInfo> {
    windows
        .iter()
        .filter_map(|window| {
            let app = app_from_linux_window(window);
            selector_match_score(&app, selector).map(|score| (score, window))
        })
        .max_by_key(|(score, window)| (*score, window.focused))
        .map(|(_, window)| window.clone())
}

pub(crate) fn preferred_linux_window(
    windows: &[linux_windowing::LinuxWindowInfo],
) -> Option<linux_windowing::LinuxWindowInfo> {
    windows
        .iter()
        .find(|window| window.focused)
        .cloned()
        .or_else(|| windows.first().cloned())
}

pub(crate) fn enrich_accessible_apps_from_windows(
    apps: &mut [DiscoveredApp],
    windows: &[linux_windowing::LinuxWindowInfo],
) {
    for app in apps {
        let Some(window) = best_linux_window_match(windows, &app.info) else {
            continue;
        };
        let window_app = app_from_linux_window(window);

        if app.info.pid.is_none() {
            app.info.pid = window_app.pid;
        }
        if app.info.executable.is_none() {
            app.info.executable = window_app.executable.clone();
        }
        // Adopt the matched window's authoritative desktop identity. An AT-SPI
        // app's own `app_id` is an opaque bus ref (":1.16:/org/a11y/...") and
        // its `desktop_file_id` is guessed from the executable name (e.g.
        // "pixy-hid.desktop"), so neither round-trips to the identity an agent
        // reads from list_resources(windows)/capture_desktop
        // ("ai.emeet.pixy-control.desktop"). Once a window is matched (now
        // reliably by client PID), the window's identity is authoritative, so
        // rewriting these lets `select_app` resolve the tree under the app_id
        // the agent already holds. Tree building and actions resolve through
        // `object_ref`/`backend_ref`, never this string, so the rewrite is safe.
        // Guard on the window carrying a real app_id (never the synthesized
        // "backend:window_id" fallback).
        if window.app_id.is_some() {
            app.info.app_id = window_app.app_id.clone();
        }
        if window_app.desktop_file_id.is_some() {
            app.info.desktop_file_id = window_app.desktop_file_id.clone();
        }
        if app.info.toolkit_guess.is_none() {
            app.info.toolkit_guess = window_app.toolkit_guess.clone();
        }
        if app.info.window_title.is_none() {
            app.info.window_title = window_app.window_title.clone();
        }
        if app.info.name.eq_ignore_ascii_case("Unnamed") {
            app.info.name = window_app.name.clone();
        }
        if !app.info.is_focused_candidate && window_app.is_focused_candidate {
            app.info.is_focused_candidate = true;
        }
    }
}

pub(crate) fn merge_app_lists(
    apps: &[DiscoveredApp],
    windows: &[linux_windowing::LinuxWindowInfo],
) -> Vec<AppInfo> {
    let mut merged = apps.iter().map(|app| app.info.clone()).collect::<Vec<_>>();
    for window in windows {
        if !merged
            .iter()
            .any(|app| linux_window_matches_app(window, app))
        {
            merged.push(app_from_linux_window(window));
        }
    }
    merged
}

/// A COSMIC foreign-toplevel and an X11 window describe the same XWayland
/// surface. cosmic-comp surfaces an XWayland window's WM_CLASS instance name as
/// its foreign-toplevel `app_id` (e.g. `kwrite`), while the X11 backend reports
/// that name in `wm_class` and derives a `<stem>.desktop` `app_id`. COSMIC
/// reports no PID for these surfaces and X11 reports physical-pixel bounds, so
/// the PID/title/bounds identity in `dedupe_windows` never matches them. Match
/// the WM_CLASS alias instead so a single XWayland toplevel is not listed
/// twice (once by the COSMIC helper, once by the X11 EWMH backend).
pub(crate) fn xwayland_window_alias(
    left: &linux_windowing::LinuxWindowInfo,
    right: &linux_windowing::LinuxWindowInfo,
) -> bool {
    let (cosmic, x11) = match (left.backend.as_str(), right.backend.as_str()) {
        (
            crate::windowing::registry::COSMIC_WAYLAND_BACKEND,
            crate::windowing::registry::X11_BACKEND,
        ) => (left, right),
        (
            crate::windowing::registry::X11_BACKEND,
            crate::windowing::registry::COSMIC_WAYLAND_BACKEND,
        ) => (right, left),
        _ => return false,
    };
    let Some(cosmic_app) = cosmic.app_id.as_deref().map(normalize_match_key) else {
        return false;
    };
    let x11_class = x11.wm_class.as_deref().map(normalize_match_key);
    let x11_app_stem = x11.app_id.as_deref().map(normalize_desktop_id_stem);
    x11_class.as_deref() == Some(cosmic_app.as_str())
        || x11_app_stem.as_deref() == Some(cosmic_app.as_str())
}

fn best_linux_window_match<'a>(
    windows: &'a [linux_windowing::LinuxWindowInfo],
    app: &AppInfo,
) -> Option<&'a linux_windowing::LinuxWindowInfo> {
    windows
        .iter()
        .filter_map(|window| linux_window_match_score(window, app).map(|score| (score, window)))
        .max_by_key(|(score, window)| (*score, window.focused))
        .map(|(_, window)| window)
}

pub(crate) fn linux_window_matches_app(
    window: &linux_windowing::LinuxWindowInfo,
    app: &AppInfo,
) -> bool {
    linux_window_match_score(window, app).is_some()
}

fn linux_window_match_score(
    window: &linux_windowing::LinuxWindowInfo,
    app: &AppInfo,
) -> Option<i32> {
    let window_app = app_from_linux_window(window);
    if app.app_id == window_app.app_id {
        return Some(1_000);
    }
    if let (Some(window_pid), Some(app_pid)) = (window.pid, app.pid)
        && window_pid == app_pid
    {
        return Some(900);
    }

    let mut score = 0i32;
    let mut identity_signals = 0u8;

    let window_title = window_app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let app_title = app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let window_name = normalize_match_key(&window_app.name);
    let app_name = normalize_match_key(&app.name);
    let window_executable = window_app.executable.as_deref().map(normalize_match_key);
    let app_executable = app.executable.as_deref().map(normalize_match_key);
    let window_desktop = window_app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let app_desktop = app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let window_resource_name = window.app_id.as_deref().map(normalize_match_key);
    let window_resource_class = window.wm_class.as_deref().map(normalize_match_key);

    // An exact title match adds score but is deliberately NOT a standalone
    // identity signal: generic/shared captions ("Settings", "Untitled", a
    // document name) would otherwise correlate unrelated apps. The reliable
    // cross-toolkit correlator for Wayland windows whose app_id/name differ
    // from the AT-SPI app name is the client PID, which KWin's getWindowInfo
    // provides and the PID rule above consumes.
    if !window_title.is_empty()
        && !app_title.is_empty()
        && normalize_match_key(&window_title) == normalize_match_key(&app_title)
    {
        score += 400;
    }

    if window_desktop.is_some() && window_desktop == app_desktop {
        score += 240;
        identity_signals += 1;
    }

    if window_executable.is_some() && window_executable == app_executable {
        score += 220;
        identity_signals += 1;
    }

    if window_name == app_name {
        score += 180;
        identity_signals += 1;
    }

    if window_resource_name.as_ref().is_some_and(|resource| {
        resource == &app_name
            || app_executable
                .as_ref()
                .is_some_and(|executable| executable == resource)
            || app_desktop
                .as_ref()
                .is_some_and(|desktop| desktop == resource)
    }) {
        score += 170;
        identity_signals += 1;
    }

    if window_resource_class.as_ref().is_some_and(|resource| {
        resource == &app_name
            || app_executable
                .as_ref()
                .is_some_and(|executable| executable == resource)
            || app_desktop
                .as_ref()
                .is_some_and(|desktop| desktop == resource)
    }) {
        score += 170;
        identity_signals += 1;
    }

    if !window_title.is_empty()
        && !app_title.is_empty()
        && (window_title.contains(&app_title) || app_title.contains(&window_title))
    {
        score += 120;
    }

    if window.focused {
        score += 5;
    }

    (identity_signals > 0 && score > 0).then_some(score)
}

/// Returns apps that match the given window. If none match, falls back to the
/// original list so the caller can still proceed with the full set.
pub(crate) fn apps_matching_window_or_all(
    apps: &[DiscoveredApp],
    window: &linux_windowing::LinuxWindowInfo,
) -> Vec<DiscoveredApp> {
    if apps.is_empty() {
        return Vec::new();
    }
    let filtered: Vec<DiscoveredApp> = apps
        .iter()
        .filter(|app| linux_window_matches_app(window, &app.info))
        .cloned()
        .collect();
    if filtered.is_empty() {
        apps.to_vec()
    } else {
        filtered
    }
}

pub(crate) fn app_from_linux_window(window: &linux_windowing::LinuxWindowInfo) -> AppInfo {
    let name = window
        .app_id
        .as_deref()
        .or(window.wm_class.as_deref())
        .or(window.title.as_deref())
        .unwrap_or("Window")
        .to_string();
    AppInfo {
        app_id: window
            .app_id
            .clone()
            .unwrap_or_else(|| format!("{}:{}", window.backend, window.window_id)),
        name,
        pid: window.pid,
        executable: None,
        desktop_file_id: window.app_id.as_ref().and_then(|value| {
            let value = value.trim();
            if value.is_empty() || value.contains(':') || value.contains('/') {
                // The `:`/`/` guards exclude synthesized `backend:window_id`
                // fallbacks and opaque bus/object references.
                None
            } else if value.ends_with(".desktop") {
                Some(value.to_string())
            } else {
                // Freedesktop convention: the Wayland app_id is the desktop-file
                // basename without the ".desktop" suffix. Native Wayland apps
                // use the reverse-DNS basename (`com.mitchellh.ghostty` ->
                // `com.mitchellh.ghostty.desktop`), while an XWayland toplevel
                // surfaces its WM_CLASS instance name (`kwrite` ->
                // `kwrite.desktop`) — both round-trip to the installed desktop
                // file the agent already holds.
                Some(format!("{value}.desktop"))
            }
        }),
        app_user_model_id: None,
        window_handle: Some(window.window_id.clone()),
        toolkit_guess: window.client_type.clone(),
        window_title: window.title.clone(),
        is_focused_candidate: window.focused,
    }
}

#[cfg(test)]
pub(crate) fn select_x11_window(
    windows: &[X11WindowInfo],
    selector: &AppSelector,
) -> Option<X11WindowInfo> {
    windows
        .iter()
        .filter_map(|window| {
            selector_match_score(&window.app, selector).map(|score| (score, window))
        })
        .max_by_key(|(score, window)| (*score, window.app.is_focused_candidate))
        .map(|(_, window)| window.clone())
}

#[cfg(test)]
pub(crate) fn x11_window_matches_app(window: &X11WindowInfo, app: &AppInfo) -> bool {
    x11_match_score(window, app).is_some()
}

pub(crate) fn best_x11_window_match<'a>(
    windows: &'a [X11WindowInfo],
    app: &AppInfo,
) -> Option<&'a X11WindowInfo> {
    windows
        .iter()
        .filter_map(|window| x11_match_score(window, app).map(|score| (score, window)))
        .max_by_key(|(score, window)| (*score, window.app.is_focused_candidate))
        .map(|(_, window)| window)
}

fn x11_match_score(window: &X11WindowInfo, app: &AppInfo) -> Option<i32> {
    if app.app_id == window.app.app_id {
        return Some(1_000);
    }

    if let (Some(window_pid), Some(app_pid)) = (window.app.pid, app.pid)
        && window_pid == app_pid
    {
        return Some(900);
    }

    let window_title = window
        .app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let app_title = app
        .window_title
        .as_deref()
        .map(normalize_match_key)
        .unwrap_or_default();
    let window_name = normalize_match_key(&window.app.name);
    let app_name = normalize_match_key(&app.name);
    let window_executable = window.app.executable.as_deref().map(normalize_match_key);
    let app_executable = app.executable.as_deref().map(normalize_match_key);
    let window_desktop = window
        .app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let app_desktop = app
        .desktop_file_id
        .as_deref()
        .map(normalize_desktop_id_stem);
    let window_instance = window.instance_name.as_deref().map(normalize_match_key);
    let window_class = window.class_name.as_deref().map(normalize_match_key);

    let mut score = 0i32;
    let mut identity_signals = 0u8;
    if let (Some(window_title), Some(app_title)) =
        (window.app.window_title.as_ref(), app.window_title.as_ref())
        && normalize_match_key(window_title) == normalize_match_key(app_title)
    {
        score += 400;
    }

    if let (Some(window_desktop_file_id), Some(app_desktop_file_id)) = (
        window.app.desktop_file_id.as_ref(),
        app.desktop_file_id.as_ref(),
    ) && normalize_match_key(window_desktop_file_id) == normalize_match_key(app_desktop_file_id)
        && normalize_match_key(&window.app.name) == normalize_match_key(&app.name)
    {
        score += 260;
        identity_signals += 1;
    }

    if window_desktop.is_some() && window_desktop == app_desktop {
        score += 240;
        identity_signals += 1;
    }

    if window_executable.is_some() && window_executable == app_executable {
        score += 220;
        identity_signals += 1;
    }

    if window_name == app_name {
        score += 180;
        identity_signals += 1;
    }

    if window_instance.as_ref().is_some_and(|instance| {
        instance == &app_name
            || app_executable
                .as_ref()
                .is_some_and(|executable| executable == instance)
            || app_desktop
                .as_ref()
                .is_some_and(|desktop| desktop == instance)
    }) {
        score += 170;
        identity_signals += 1;
    }

    if window_class.as_ref().is_some_and(|class_name| {
        class_name == &app_name
            || app_executable
                .as_ref()
                .is_some_and(|executable| executable == class_name)
            || app_desktop
                .as_ref()
                .is_some_and(|desktop| desktop == class_name)
    }) {
        score += 170;
        identity_signals += 1;
    }

    if !window_title.is_empty()
        && !app_title.is_empty()
        && (window_title.contains(&app_title) || app_title.contains(&window_title))
    {
        score += 120;
    }

    if window.app.is_focused_candidate {
        score += 5;
    }

    if identity_signals == 0 {
        return None;
    }

    if obvious_service_app(app) {
        score -= 40;
    }

    (score > 0).then_some(score)
}

fn obvious_service_app(app: &AppInfo) -> bool {
    [
        app.executable.as_deref(),
        app.desktop_file_id.as_deref(),
        Some(app.name.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(normalize_match_key)
    .any(|value| {
        [
            "service",
            "proxy",
            "menu",
            "portal",
            "daemon",
            "ksmserver",
            "kaccess",
            "kglobalaccel",
            "kded",
            "xembedsniproxy",
            "gmenudbusmenuproxy",
        ]
        .into_iter()
        .any(|needle| value.contains(needle))
    })
}

fn normalize_match_key(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_desktop_id_stem(value: &str) -> String {
    normalize_match_key(value.trim_end_matches(".desktop"))
}

pub(crate) fn matches_selector(app: &AppInfo, selector: &AppSelector) -> bool {
    selector
        .app_id
        .as_ref()
        .is_none_or(|wanted| &app.app_id == wanted)
        && selector
            .desktop_file_id
            .as_ref()
            .is_none_or(|wanted| app.desktop_file_id.as_ref() == Some(wanted))
        && selector.window_title.as_ref().is_none_or(|wanted| {
            app.window_title.as_ref().is_some_and(|title| {
                title
                    .to_ascii_lowercase()
                    .contains(&wanted.to_ascii_lowercase())
            })
        })
        && selector.name.as_ref().is_none_or(|wanted| {
            app.name
                .to_ascii_lowercase()
                .contains(&wanted.to_ascii_lowercase())
        })
}

pub(crate) fn selector_match_score(app: &AppInfo, selector: &AppSelector) -> Option<i32> {
    if !matches_selector(app, selector) {
        return None;
    }

    let mut score = 0i32;

    if let Some(app_id) = selector.app_id.as_ref()
        && &app.app_id == app_id
    {
        score += 10_000;
    }

    if let Some(desktop_file_id) = selector.desktop_file_id.as_ref()
        && app.desktop_file_id.as_ref() == Some(desktop_file_id)
    {
        score += 2_000;
    }

    if let Some(window_title) = selector.window_title.as_ref() {
        let wanted = normalize_match_key(window_title);
        let actual = app
            .window_title
            .as_deref()
            .map(normalize_match_key)
            .unwrap_or_default();
        if actual == wanted {
            score += 1_500;
        } else if actual.contains(&wanted) {
            score += 800;
        }
    }

    if let Some(name) = selector.name.as_ref() {
        let wanted = normalize_match_key(name);
        let actual = normalize_match_key(&app.name);
        if actual == wanted {
            score += 1_000;
        } else if actual.contains(&wanted) {
            score += 500;
        }
    }

    if app.is_focused_candidate {
        score += 25;
    }

    if app
        .window_title
        .as_ref()
        .is_some_and(|title| !title.trim().is_empty())
    {
        score += 5;
    }

    Some(score)
}

pub(crate) fn selector_summary(selector: &AppSelector) -> String {
    let mut parts = Vec::new();
    if let Some(app_id) = selector.app_id.as_ref() {
        parts.push(format!("app_id={app_id}"));
    }
    if let Some(desktop_file_id) = selector.desktop_file_id.as_ref() {
        parts.push(format!("desktop_file_id={desktop_file_id}"));
    }
    if let Some(window_title) = selector.window_title.as_ref() {
        parts.push(format!("window_title={window_title}"));
    }
    if let Some(name) = selector.name.as_ref() {
        parts.push(format!("name={name}"));
    }
    if parts.is_empty() {
        "<empty selector>".to_string()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::AppInfo;
    use sky_cua_platform::model::{AppSelector, RectF};
    use zbus::names::UniqueName;
    use zbus::zvariant::ObjectPath;

    use super::{
        app_from_linux_window, apps_matching_window_or_all, enrich_accessible_apps_from_windows,
        linux_window_matches_app, merge_app_lists, preferred_linux_window, select_app,
        select_linux_window,
    };
    use crate::apps::discovery::DiscoveredApp;
    use crate::windowing::LinuxWindowInfo;

    fn object_ref(path: &str) -> atspi::ObjectRefOwned {
        atspi::ObjectRef::new_owned(
            UniqueName::try_from(":1.7".to_string()).expect("unique name should parse"),
            ObjectPath::try_from(path.to_string()).expect("object path should parse"),
        )
    }

    fn app_info(app_id: &str, name: &str) -> AppInfo {
        AppInfo {
            app_id: app_id.to_string(),
            name: name.to_string(),
            pid: None,
            executable: None,
            desktop_file_id: None,
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: None,
            window_title: None,
            is_focused_candidate: false,
        }
    }

    fn discovered_app(app_id: &str, name: &str, path: &str) -> DiscoveredApp {
        DiscoveredApp {
            info: app_info(app_id, name),
            object_ref: object_ref(path),
            top_levels: None,
        }
    }

    fn linux_window(window_id: &str, app_id: &str, focused: bool) -> LinuxWindowInfo {
        LinuxWindowInfo {
            window_id: window_id.to_string(),
            title: Some(format!("{app_id} main window")),
            app_id: Some(app_id.to_string()),
            wm_class: Some(app_id.trim_end_matches(".desktop").to_string()),
            pid: Some(4242),
            bounds: Some(RectF {
                x: 10.0,
                y: 20.0,
                width: 640.0,
                height: 480.0,
                space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
            }),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "kwin".to_string(),
            terminal: None,
            terminal_target_sessions: Vec::new(),
        }
    }

    #[test]
    fn client_pid_correlates_wayland_window_to_atspi_app_when_names_differ() {
        // Reproduces the emeet-cam case: a GTK4 Wayland window whose app_id
        // (`ai.emeet.pixy-control`) and wm_class differ from the AT-SPI app
        // name (`pixy-hid`, the prgname). The reliable correlator is the client
        // PID that KWin's getWindowInfo now supplies; with it populated the
        // window resolves to its real accessibility tree instead of falling
        // back to synthetic geometry. Title alone must NOT be enough (guarded
        // by linux_window_match_rejects_title_only_similarity).
        let mut window = linux_window("kwin:{pixy}", "ai.emeet.pixy-control", true);
        window.pid = Some(228_756);
        window.title = Some("Cam Kontrol".to_string());

        let mut app = app_info("pixy-hid", "pixy-hid");
        app.pid = Some(228_756);
        app.window_title = Some("Cam Kontrol".to_string());
        assert!(
            linux_window_matches_app(&window, &app),
            "matching client PID should correlate the window to its AT-SPI app"
        );

        // A different PID with no other aligning signal must not match, even
        // when the caption happens to collide.
        let mut other = app_info("pixy-hid", "pixy-hid");
        other.pid = Some(999_999);
        other.window_title = Some("Cam Kontrol".to_string());
        window.app_id = None;
        window.wm_class = None;
        assert!(
            !linux_window_matches_app(&window, &other),
            "a differing PID with only a shared caption must not match"
        );
    }

    #[test]
    fn select_app_prefers_focused_candidate_when_selector_scores_tie() {
        let background = discovered_app("app-1", "Demo", "/org/a11y/demo/background");
        let mut focused = discovered_app("app-2", "Demo", "/org/a11y/demo/focused");
        focused.info.is_focused_candidate = true;
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: None,
            window_title: None,
            name: Some("demo".to_string()),
        };

        let selected = select_app(&[background, focused.clone()], &selector)
            .expect("selector should match both apps");

        assert_eq!(selected.info.app_id, focused.info.app_id);
    }

    #[test]
    fn select_linux_window_prefers_focused_window_when_selector_scores_tie() {
        let background = linux_window("kwin:{background}", "demo.desktop", false);
        let focused = linux_window("kwin:{focused}", "demo.desktop", true);
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: Some("demo.desktop".to_string()),
            window_title: None,
            name: None,
        };

        let selected = select_linux_window(&[background, focused.clone()], &selector)
            .expect("selector should match both windows");

        assert_eq!(selected.window_id, focused.window_id);
    }

    #[test]
    fn enrich_adopts_window_identity_so_window_app_id_resolves_to_the_tree() {
        // The emeet-cam shape: an AT-SPI app discovered with an opaque bus-ref
        // app_id and a prgname-guessed desktop_file_id, matched to its window
        // purely by client PID. After enrichment it must carry the window's
        // authoritative app_id/desktop_file_id so a selector using the window
        // app_id (what agents read from list_resources(windows)) resolves here.
        let mut app = discovered_app(
            ":1.16:/org/a11y/atspi/accessible/root",
            "pixy-hid",
            "/org/a11y/pixy/app",
        );
        app.info.desktop_file_id = Some("pixy-hid.desktop".to_string());
        app.info.pid = Some(321_330);

        let mut window = linux_window("kwin:{pixy}", "ai.emeet.pixy-control.desktop", true);
        window.pid = Some(321_330);
        window.wm_class = Some("ai.emeet.pixy-control".to_string());
        window.title = Some("Cam Kontrol".to_string());

        enrich_accessible_apps_from_windows(std::slice::from_mut(&mut app), &[window]);

        assert_eq!(app.info.app_id, "ai.emeet.pixy-control.desktop");
        assert_eq!(
            app.info.desktop_file_id.as_deref(),
            Some("ai.emeet.pixy-control.desktop")
        );

        let selector = AppSelector {
            app_id: Some("ai.emeet.pixy-control.desktop".to_string()),
            desktop_file_id: None,
            window_title: None,
            name: None,
        };
        assert!(
            select_app(std::slice::from_ref(&app), &selector).is_some(),
            "the window app_id must now resolve to the enriched AT-SPI app"
        );
    }

    #[test]
    fn enrich_accessible_apps_from_windows_fills_missing_registry_metadata() {
        let mut app = discovered_app("accessible-1", "Unnamed", "/org/a11y/demo/app");
        app.info.desktop_file_id = Some("demo.desktop".to_string());
        let window = linux_window("kwin:{demo}", "demo.desktop", true);

        enrich_accessible_apps_from_windows(std::slice::from_mut(&mut app), &[window]);

        assert_eq!(app.info.pid, Some(4242));
        assert_eq!(app.info.toolkit_guess.as_deref(), Some("wayland"));
        assert_eq!(
            app.info.window_title.as_deref(),
            Some("demo.desktop main window")
        );
        assert_eq!(app.info.name, "demo.desktop");
        assert!(app.info.is_focused_candidate);
    }

    #[test]
    fn merge_app_lists_adds_only_unmatched_registry_windows() {
        let mut app = discovered_app("accessible-1", "Demo", "/org/a11y/demo/app");
        app.info.desktop_file_id = Some("demo.desktop".to_string());
        let matching_window = linux_window("kwin:{demo}", "demo.desktop", false);
        let extra_window = linux_window("kwin:{extra}", "extra.desktop", true);

        let merged = merge_app_lists(&[app], &[matching_window, extra_window.clone()]);

        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|app| app.app_id == "accessible-1"));
        assert!(
            merged
                .iter()
                .any(|app| app.app_id == app_from_linux_window(&extra_window).app_id)
        );
    }

    #[test]
    fn preferred_linux_window_uses_focused_window_then_first_window() {
        let first = linux_window("kwin:{first}", "first.desktop", false);
        let focused = linux_window("kwin:{focused}", "focused.desktop", true);

        assert_eq!(
            preferred_linux_window(&[first.clone(), focused.clone()])
                .expect("focused window should be selected")
                .window_id,
            focused.window_id
        );
        assert_eq!(
            preferred_linux_window(std::slice::from_ref(&first))
                .expect("first window should be selected")
                .window_id,
            first.window_id
        );
        assert!(preferred_linux_window(&[]).is_none());
    }

    #[test]
    fn linux_window_match_accepts_pid_or_resource_class_identity() {
        let pid_window = linux_window("kwin:{pid}", "other.desktop", false);
        let mut app = app_info("accessible-1", "Demo");
        app.pid = Some(4242);
        assert!(linux_window_matches_app(&pid_window, &app));

        let mut class_window = linux_window("kwin:{class}", "demo.desktop", false);
        class_window.pid = None;
        class_window.app_id = None;
        class_window.wm_class = Some("org.example.Demo".to_string());
        let mut app = app_info("accessible-2", "org example demo");
        app.pid = None;
        assert!(linux_window_matches_app(&class_window, &app));
    }

    #[test]
    fn linux_window_match_rejects_title_only_similarity() {
        let mut window = linux_window("kwin:{title}", "unrelated.desktop", false);
        window.pid = None;
        window.app_id = None;
        window.wm_class = None;
        window.title = Some("Shared Project".to_string());
        let mut app = app_info("accessible-3", "Different App");
        app.window_title = Some("Shared Project".to_string());

        assert!(!linux_window_matches_app(&window, &app));
    }

    #[test]
    fn app_from_linux_window_derives_desktop_file_id_from_reverse_dns_app_id() {
        // COSMIC exposes a reverse-DNS Wayland app_id (not a `.desktop`-suffixed
        // id), which the freedesktop convention maps back to the installed
        // desktop file by appending the suffix.
        let window = linux_window("cosmic:123", "com.mitchellh.ghostty", true);
        let app = app_from_linux_window(&window);
        assert_eq!(
            app.desktop_file_id.as_deref(),
            Some("com.mitchellh.ghostty.desktop")
        );
    }

    #[test]
    fn app_from_linux_window_derives_desktop_file_id_from_xwayland_wm_class() {
        // An XWayland toplevel surfaces its WM_CLASS instance as the app_id
        // (no reverse-DNS dots); the freedesktop convention still appends the
        // `.desktop` suffix so the id round-trips to `kwrite.desktop`.
        let window = linux_window("cosmic:123", "kwrite", true);
        let app = app_from_linux_window(&window);
        assert_eq!(app.desktop_file_id.as_deref(), Some("kwrite.desktop"));
    }

    #[test]
    fn app_from_linux_window_leaves_desktop_file_id_none_without_a_desktop_app_id() {
        let mut window = linux_window("cosmic:123", "com.mitchellh.ghostty", true);
        window.app_id = None;
        let app = app_from_linux_window(&window);
        assert_eq!(app.desktop_file_id, None);
    }

    #[test]
    fn app_from_linux_window_rejects_synthesized_bus_refs() {
        let mut window = linux_window("cosmic:123", "com.mitchellh.ghostty", true);
        window.app_id = Some(":1.16:/org/a11y/atspi/accessible/root".to_string());
        let app = app_from_linux_window(&window);
        assert_eq!(app.desktop_file_id, None);
    }

    #[test]
    fn apps_matching_window_or_all_keeps_matching_apps_and_falls_back_when_none_match() {
        let matching = discovered_app("accessible-1", "Demo", "/org/a11y/demo/app");
        let unrelated = discovered_app("accessible-2", "Other", "/org/a11y/other/app");
        let window = linux_window("kwin:{demo}", "demo.desktop", true);

        let filtered = apps_matching_window_or_all(&[matching.clone(), unrelated.clone()], &window);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].info.app_id, matching.info.app_id);

        let fallback = apps_matching_window_or_all(std::slice::from_ref(&unrelated), &window);
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].info.app_id, unrelated.info.app_id);
    }
}
