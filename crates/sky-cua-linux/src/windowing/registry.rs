pub use cosmic::COSMIC_WAYLAND_BACKEND;
pub use gnome_extension::GNOME_SHELL_EXTENSION_BACKEND;
pub use gnome_introspect::GNOME_SHELL_INTROSPECT_BACKEND;
pub use hyprland::HYPRLAND_BACKEND;
pub use i3::I3_BACKEND;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::EnvironmentInfo;

use super::probe::{BackendDescriptor, BackendProbe};
use super::terminal::enrich_terminal_windows;
use super::types::LinuxWindowInfo;
use super::{cosmic, gnome_extension, gnome_introspect, hyprland, i3};
use crate::kwin::{self, KWinWindowInfo};
use crate::x11::windowing::{self, X11WindowInfo};

pub const KWIN_BACKEND: &str = "kwin";
pub const X11_BACKEND: &str = "x11";
pub const WINDOW_PERMISSION_HINT: &str = "Computer Use could not access a supported window list backend. Targeted window input requires session-bus access plus GNOME Shell Introspect, the GNOME Shell extension, the COSMIC Wayland helper, KWin/Plasma DBus scripting, Hyprland hyprctl, i3-msg, or X11 window metadata.";

#[derive(Debug, Clone, Copy)]
enum BackendKind {
    GnomeExtension,
    GnomeIntrospect,
    Cosmic,
    Kwin,
    Hyprland,
    I3,
    X11,
}

const BACKEND_ORDER: &[BackendKind] = &[
    BackendKind::GnomeExtension,
    BackendKind::GnomeIntrospect,
    BackendKind::Cosmic,
    BackendKind::Kwin,
    BackendKind::Hyprland,
    BackendKind::I3,
    BackendKind::X11,
];

const DESCRIPTORS: &[BackendDescriptor] = &[
    BackendDescriptor {
        id: GNOME_SHELL_EXTENSION_BACKEND,
        failure_label: "GNOME Shell extension",
        list_note: "Window list came from the GNOME Shell extension. Terminal windows may include terminal process context when the process tree is readable.",
        missing_hint: "On GNOME, run setup_window_targeting to install the optional GNOME Shell extension backend.",
        can_exact_focus: true,
    },
    BackendDescriptor {
        id: GNOME_SHELL_INTROSPECT_BACKEND,
        failure_label: "GNOME Shell Introspect",
        list_note: "Window list came from GNOME Shell Introspect. Terminal windows may include terminal process context when the process tree is readable.",
        missing_hint: "On GNOME, ensure org.gnome.Shell.Introspect is available on the session bus.",
        can_exact_focus: false,
    },
    BackendDescriptor {
        id: COSMIC_WAYLAND_BACKEND,
        failure_label: "COSMIC helper",
        list_note: "Window list came from the COSMIC Wayland helper. Terminal windows may include terminal process context when the process tree is readable.",
        missing_hint: "On COSMIC, ensure the bundled sky-cua-cosmic-helper is present and can connect to the session.",
        can_exact_focus: true,
    },
    BackendDescriptor {
        id: KWIN_BACKEND,
        failure_label: "KWin",
        list_note: "Window list came from KWin/Plasma DBus scripting. Terminal windows may include terminal process context when the process tree is readable.",
        missing_hint: "On KDE/Plasma, ensure KWin exposes org.kde.KWin scripting on the session bus.",
        can_exact_focus: true,
    },
    BackendDescriptor {
        id: HYPRLAND_BACKEND,
        failure_label: "Hyprland",
        list_note: "Window list came from Hyprland hyprctl. Terminal windows may include terminal process context when the process tree is readable.",
        missing_hint: "On Hyprland, ensure hyprctl is available in the session.",
        can_exact_focus: true,
    },
    BackendDescriptor {
        id: I3_BACKEND,
        failure_label: "i3",
        list_note: "Window list came from i3-msg. Terminal windows may include terminal process context when the process tree is readable.",
        missing_hint: "On i3, ensure i3-msg can reach the active i3 IPC socket.",
        can_exact_focus: true,
    },
    BackendDescriptor {
        id: X11_BACKEND,
        failure_label: "X11",
        list_note: "Window list came from X11 EWMH metadata. Terminal windows may include terminal process context when the process tree is readable.",
        missing_hint: "On X11/XWayland, ensure DISPLAY is set and xprop/xdotool can reach the X server.",
        can_exact_focus: true,
    },
];

pub fn descriptors() -> &'static [BackendDescriptor] {
    DESCRIPTORS
}
pub fn descriptor(id: &str) -> Option<&'static BackendDescriptor> {
    DESCRIPTORS.iter().find(|descriptor| descriptor.id == id)
}
pub fn backend_can_exact_focus(id: &str) -> bool {
    descriptor(id).is_some_and(|descriptor| descriptor.can_exact_focus)
}

/// True when window targeting on this environment relies on the bundled GNOME
/// Shell extension: GNOME sessions, or an unknown session with no desktop
/// signal (the historical default). Non-GNOME sessions (KDE/KWin, COSMIC,
/// Hyprland, sway/i3, X11) use their own window backend and need no GNOME
/// Shell extension install/enable.
pub fn window_targeting_uses_gnome_extension(environment: &EnvironmentInfo) -> bool {
    backend_can_list_in_environment(BackendKind::GnomeExtension, environment)
}

pub async fn discover_windows(
    environment: &EnvironmentInfo,
) -> Result<Vec<LinuxWindowInfo>, BackendError> {
    discover_all_windows(environment).await
}

pub async fn discover_app_windows(
    environment: &EnvironmentInfo,
) -> Result<Vec<LinuxWindowInfo>, BackendError> {
    discover_all_windows(environment).await
}

async fn discover_all_windows(
    environment: &EnvironmentInfo,
) -> Result<Vec<LinuxWindowInfo>, BackendError> {
    let mut errors = Vec::new();
    let mut windows = Vec::new();
    for backend in BACKEND_ORDER {
        if !backend_can_list_in_environment(*backend, environment) {
            errors.push(format!(
                "{} skipped for this desktop environment",
                backend.failure_label()
            ));
            continue;
        }
        match list_windows_for(*backend, environment).await {
            Ok(mut backend_windows) if !backend_windows.is_empty() => {
                windows.append(&mut backend_windows);
            }
            Ok(_) => errors.push(format!("{} returned no windows", backend.failure_label())),
            Err(error) => errors.push(format!("{} failed: {error}", backend.failure_label())),
        }
    }
    if !windows.is_empty() {
        finalize_window_list(&mut windows, false);
        crate::displays::assign_window_displays(&mut windows, &environment.displays);
        return Ok(windows);
    }
    Err(BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!("{WINDOW_PERMISSION_HINT} Details: {}", errors.join("; ")),
    ))
}

pub async fn discover_activation_windows(
    environment: &EnvironmentInfo,
) -> Result<Vec<LinuxWindowInfo>, BackendError> {
    let mut errors = Vec::new();
    let mut windows = Vec::new();
    for backend in BACKEND_ORDER {
        if !backend_can_list_in_environment(*backend, environment)
            || !backend_can_exact_focus_in_environment(*backend, environment)
        {
            continue;
        }
        match list_windows_for(*backend, environment).await {
            Ok(mut backend_windows) if !backend_windows.is_empty() => {
                windows.append(&mut backend_windows);
            }
            Ok(_) => errors.push(format!("{} returned no windows", backend.failure_label())),
            Err(error) => errors.push(format!("{} failed: {error}", backend.failure_label())),
        }
    }
    if windows.is_empty() {
        return Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!(
                "No exact window-activation backend returned windows. Details: {}",
                errors.join("; ")
            ),
        ));
    }
    finalize_window_list(&mut windows, true);
    crate::displays::assign_window_displays(&mut windows, &environment.displays);
    Ok(windows)
}

pub async fn verify_window_focused(
    environment: &EnvironmentInfo,
    expected: &LinuxWindowInfo,
) -> Result<LinuxWindowInfo, BackendError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1_000);
    let mut last_focused = None;
    loop {
        if let Some(mut focused) = focused_window_override() {
            if same_focus_target_strong(&focused, expected) {
                // The override sources (e.g. the COSMIC helper) know window
                // identity and bounds but not the display topology, so attach
                // the display from the environment's own discovery. Targeted
                // captures rely on the display ref to resolve source geometry.
                crate::displays::assign_window_displays(
                    std::slice::from_mut(&mut focused),
                    &environment.displays,
                );
                return Ok(focused);
            }
            last_focused = Some(focused);
        }
        // KWin fast path: the watcher-cached active-window uuid answers the
        // poll without the full discovery fan-out (a /proc walk plus one
        // gdbus subprocess per candidate query and per window, every tick).
        // A mismatch or readback failure falls through to full discovery so
        // cross-backend identity matching still applies.
        if expected.backend == KWIN_BACKEND
            && let Some(focused) = kwin_focused_window_matches(expected).await
        {
            return Ok(focused);
        }
        // On COSMIC the override above already answered with the authoritative
        // focus signal (including the post-activation fallback); re-listing
        // would spawn a second helper process with the same wait and return the
        // same cosmic window, since cosmic is the only window backend in that
        // session and there is no cross-backend alias to match. Skip it and
        // re-check the override next tick so the focus-verification poll budget
        // is not consumed by a redundant helper round trip.
        if expected.backend != COSMIC_WAYLAND_BACKEND {
            let windows = discover_windows(environment).await?;
            if let Some(focused) = windows.iter().find(|window| window.focused) {
                if same_focus_target(focused, expected, &windows) {
                    return Ok(focused.clone());
                }
                last_focused = Some(focused.clone());
            }
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    }
    Err(BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        match last_focused {
            Some(window) => format!(
                "window activation was sent for {} window {}, but focus verification saw {} window {}",
                expected.backend, expected.window_id, window.backend, window.window_id
            ),
            None => format!(
                "window activation was sent for {} window {}, but no focused window was reported before verification timed out",
                expected.backend, expected.window_id
            ),
        },
    ))
}

/// Compare the compositor-reported active-window uuid against the expected
/// KWin window id. Returns the verified window on an exact match, `None` on
/// mismatch or when readback is unavailable.
///
/// The returned window is the caller's `expected` snapshot with `focused`
/// forced true — identity fields (backend, window_id) are authoritative, but
/// bounds/title/workspace are activation-time state, not a fresh readback.
/// The watcher cache can also lag the compositor by an in-flight
/// `windowActivated` signal (sub-frame), bounded by the caller's poll
/// deadline; full discovery on the next tick self-corrects.
async fn kwin_focused_window_matches(expected: &LinuxWindowInfo) -> Option<LinuxWindowInfo> {
    let active_uuid = crate::kwin_script::active_window_uuid()
        .await
        .ok()
        .flatten()?;
    let expected_uuid = expected.window_id.strip_prefix("kwin:")?;
    (crate::kwin_script::normalize_uuid(expected_uuid) == active_uuid).then(|| {
        let mut focused = expected.clone();
        focused.focused = true;
        focused
    })
}

async fn list_windows_for(
    backend: BackendKind,
    environment: &EnvironmentInfo,
) -> Result<Vec<LinuxWindowInfo>, BackendError> {
    match backend {
        BackendKind::GnomeExtension => gnome_extension::list_windows().await.map_err(anyhow_error),
        BackendKind::GnomeIntrospect => {
            gnome_introspect::list_windows().await.map_err(anyhow_error)
        }
        BackendKind::Cosmic => cosmic::list_windows().map_err(anyhow_error),
        BackendKind::Kwin => kwin::discover_windows(environment)
            .await
            .map(|windows| windows.into_iter().map(linux_window_from_kwin).collect()),
        BackendKind::Hyprland => hyprland::list_windows().map_err(anyhow_error),
        BackendKind::I3 => i3::list_windows().map_err(anyhow_error),
        BackendKind::X11 => windowing::discover_windows()
            .map(|windows| windows.into_iter().map(linux_window_from_x11).collect()),
    }
}

pub async fn activate_window(window: &LinuxWindowInfo) -> Result<(), BackendError> {
    ensure_backend_can_exact_focus(window)?;
    match window.backend.as_str() {
        GNOME_SHELL_EXTENSION_BACKEND => gnome_extension::activate_window(&window.window_id).await,
        GNOME_SHELL_INTROSPECT_BACKEND => {
            let app_id = window
                .app_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    BackendError::new(
                        BackendErrorCode::ActionUnsupportedForEnvironment,
                        "GNOME Shell can only focus by app_id; the matched window has no app_id",
                    )
                })?;
            gnome_introspect::focus_app(app_id)
                .await
                .map_err(anyhow_error)
        }
        COSMIC_WAYLAND_BACKEND => cosmic::activate_window(&window.window_id),
        KWIN_BACKEND => kwin::activate_window(&window.window_id).await,
        HYPRLAND_BACKEND => hyprland::activate_window(&window.window_id),
        I3_BACKEND => i3::activate_window(&window.window_id),
        X11_BACKEND => crate::x11::input_xtest::window_activate(&window.window_id),
        backend => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("Unsupported window backend for activation: {backend}"),
        )),
    }
}

fn ensure_backend_can_exact_focus(window: &LinuxWindowInfo) -> Result<(), BackendError> {
    if backend_can_exact_focus(&window.backend) {
        return Ok(());
    }
    Err(BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!(
            "{} can list window {}, but exact activation is not supported by this backend. Use the GNOME Shell extension, COSMIC helper, Hyprland, i3, or X11 backend for exact activation.",
            window.backend, window.window_id
        ),
    ))
}

fn backend_can_exact_focus_in_environment(
    backend: BackendKind,
    environment: &EnvironmentInfo,
) -> bool {
    if !backend_can_exact_focus(backend.id()) {
        return false;
    }
    match backend {
        BackendKind::Kwin => kwin::kwin_exact_activation_available(environment),
        _ => true,
    }
}

fn backend_can_list_in_environment(backend: BackendKind, environment: &EnvironmentInfo) -> bool {
    match backend {
        BackendKind::GnomeExtension | BackendKind::GnomeIntrospect => {
            environment_matches_or_unknown(environment, &["gnome"])
        }
        BackendKind::Cosmic => environment_matches_or_unknown(environment, &["cosmic"]),
        BackendKind::Kwin => {
            environment_matches_or_unknown(environment, &["kde", "plasma", "kwin"])
        }
        BackendKind::Hyprland => environment_matches_or_unknown(environment, &["hyprland"]),
        BackendKind::I3 => environment_matches_or_unknown(environment, &["i3", "sway"]),
        BackendKind::X11 => {
            environment.xdg_session_type.as_deref() == Some("x11") || environment.display.is_some()
        }
    }
}

fn environment_matches_or_unknown(environment: &EnvironmentInfo, needles: &[&str]) -> bool {
    !has_desktop_signal(environment) || environment_matches(environment, needles)
}

fn has_desktop_signal(environment: &EnvironmentInfo) -> bool {
    environment
        .desktop_environment
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || environment
            .compositor
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn environment_matches(environment: &EnvironmentInfo, needles: &[&str]) -> bool {
    let matches_value = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::to_ascii_lowercase)
            .is_some_and(|value| needles.iter().any(|needle| value.contains(needle)))
    };
    matches_value(&environment.desktop_environment) || matches_value(&environment.compositor)
}

pub fn focused_window_override() -> Option<LinuxWindowInfo> {
    cosmic::focused_window().ok().flatten()
}

pub fn probe_backends(environment: &EnvironmentInfo) -> Vec<BackendProbe> {
    BACKEND_ORDER
        .iter()
        .map(|backend| probe_backend(*backend, environment))
        .collect()
}

fn probe_backend(backend: BackendKind, environment: &EnvironmentInfo) -> BackendProbe {
    if !backend_can_list_in_environment(backend, environment) {
        return skipped_probe(backend);
    }

    match backend {
        BackendKind::GnomeExtension => gnome_extension::probe(),
        BackendKind::GnomeIntrospect => gnome_introspect::probe(),
        BackendKind::Cosmic => cosmic::probe(),
        BackendKind::Kwin => probe_kwin(environment),
        BackendKind::Hyprland => hyprland::probe(),
        BackendKind::I3 => i3::probe(),
        BackendKind::X11 => probe_x11(),
    }
}

fn skipped_probe(backend: BackendKind) -> BackendProbe {
    BackendProbe {
        id: backend.id(),
        ok: false,
        can_list_windows: false,
        can_focus_apps: false,
        can_focus_windows: false,
        detail: "Skipped because this backend does not match the current desktop environment"
            .to_string(),
    }
}

fn probe_kwin(environment: &EnvironmentInfo) -> BackendProbe {
    let can_list = kwin::kwin_window_query_available(environment);
    let can_activate = kwin::kwin_exact_activation_available(environment);
    BackendProbe {
        id: KWIN_BACKEND,
        ok: can_list,
        can_list_windows: can_list,
        can_focus_apps: can_activate,
        can_focus_windows: can_activate,
        detail: if can_activate {
            "KWin window query, scripted exact activation, and active-window readback are available"
                .to_string()
        } else if can_list {
            "KWin window query is available, but KWin scripting activation is unavailable"
                .to_string()
        } else {
            "KWin window query is not available for this session".to_string()
        },
    }
}

fn probe_x11() -> BackendProbe {
    let ok = windowing::x11_window_query_available();
    BackendProbe {
        id: X11_BACKEND,
        ok,
        can_list_windows: ok,
        can_focus_apps: ok,
        can_focus_windows: ok,
        detail: if ok {
            "X11 window query is available".to_string()
        } else {
            "X11 window query is unavailable".to_string()
        },
    }
}

fn linux_window_from_kwin(window: KWinWindowInfo) -> LinuxWindowInfo {
    LinuxWindowInfo {
        window_id: window.window_id,
        title: window.app.window_title,
        app_id: window
            .app
            .desktop_file_id
            .clone()
            .or(Some(window.app.app_id)),
        wm_class: window.resource_class.or(window.resource_name),
        pid: window.app.pid,
        bounds: window.bounds,
        display: None,
        display_intersections: Vec::new(),
        workspace: window.workspace,
        focused: window.app.is_focused_candidate,
        hidden: false,
        client_type: Some("wayland".to_string()),
        backend: KWIN_BACKEND.to_string(),
        terminal: None,
        terminal_target_sessions: Vec::new(),
    }
}

fn linux_window_from_x11(window: X11WindowInfo) -> LinuxWindowInfo {
    LinuxWindowInfo {
        window_id: window.window_id,
        title: window.app.window_title,
        app_id: window
            .app
            .desktop_file_id
            .clone()
            .or(Some(window.app.app_id)),
        wm_class: window.class_name.or(window.instance_name),
        pid: window.app.pid,
        bounds: window.bounds,
        display: None,
        display_intersections: Vec::new(),
        workspace: window.workspace,
        focused: window.app.is_focused_candidate,
        hidden: false,
        client_type: window
            .app
            .toolkit_guess
            .map(|value| value.to_ascii_lowercase()),
        backend: X11_BACKEND.to_string(),
        terminal: None,
        terminal_target_sessions: Vec::new(),
    }
}

fn dedupe_windows(windows: &mut Vec<LinuxWindowInfo>) {
    let mut unique = Vec::new();
    // XWayland alias absorption is one-to-one: a kept COSMIC entry absorbs at
    // most one X11 entry of the same WM_CLASS (its own toplevel). When the
    // COSMIC helper missed a toplevel, a second same-class X11 window is a
    // distinct surface and must stay in the list instead of being dropped.
    let mut alias_absorbed: Vec<bool> = Vec::new();
    for window in windows.drain(..) {
        if let Some(existing) = unique
            .iter()
            .position(|existing: &LinuxWindowInfo| same_window_identity_core(existing, &window))
        {
            if window.focused && !unique[existing].focused {
                unique[existing] = window;
            }
            continue;
        }
        if let Some(existing) =
            unique
                .iter()
                .zip(&alias_absorbed)
                .position(|(existing, absorbed)| {
                    !absorbed && crate::app_match::xwayland_window_alias(existing, &window)
                })
        {
            // The COSMIC entry is always kept: it carries the logical bounds
            // this dedupe prefers, and replacing it with the X11 entry would
            // leave the slot x11-typed while still marked absorbed. Both
            // backends report the same focused state for the same surface.
            alias_absorbed[existing] = true;
            continue;
        }
        alias_absorbed.push(false);
        unique.push(window);
    }
    *windows = unique;
}

fn dedupe_backend_window_ids(windows: &mut Vec<LinuxWindowInfo>) {
    let mut seen = std::collections::HashSet::new();
    windows.retain(|window| seen.insert((window.backend.clone(), window.window_id.clone())));
}

fn finalize_window_list(windows: &mut Vec<LinuxWindowInfo>, preserve_backend_ids: bool) {
    if preserve_backend_ids {
        dedupe_backend_window_ids(windows);
    } else {
        dedupe_windows(windows);
    }
    enrich_terminal_windows(windows);
    windows.sort_by(|a, b| {
        (!a.focused, a.backend.as_str(), a.window_id.as_str()).cmp(&(
            !b.focused,
            b.backend.as_str(),
            b.window_id.as_str(),
        ))
    });
}

/// Identity without the XWayland alias tier: same backend + window id, or the
/// pid/title/bounds triple. The alias tier is one-to-one per surface and is
/// handled separately by `dedupe_windows` so a second same-class X11 toplevel
/// is not silently dropped.
fn same_window_identity_core(left: &LinuxWindowInfo, right: &LinuxWindowInfo) -> bool {
    (left.backend == right.backend && left.window_id == right.window_id)
        || (left.pid.is_some()
            && left.pid == right.pid
            && left.title.is_some()
            && left.title == right.title
            && left.bounds.is_some()
            && left.bounds == right.bounds)
}

fn same_window_identity(left: &LinuxWindowInfo, right: &LinuxWindowInfo) -> bool {
    same_window_identity_core(left, right) || crate::app_match::xwayland_window_alias(left, right)
}

fn same_focus_target_strong(left: &LinuxWindowInfo, right: &LinuxWindowInfo) -> bool {
    (left.backend == right.backend && left.window_id == right.window_id)
        || same_window_identity(left, right)
        || (left.pid.is_some() && left.pid == right.pid && optional_same(&left.title, &right.title))
}

fn cross_backend_focus_alias(left: &LinuxWindowInfo, right: &LinuxWindowInfo) -> bool {
    left.backend != right.backend
        && optional_same(&left.title, &right.title)
        && (optional_same(&left.app_id, &right.app_id)
            || optional_same(&left.wm_class, &right.wm_class))
}

pub(crate) fn unique_cross_backend_focus_target<'a>(
    candidates: &'a [LinuxWindowInfo],
    expected: &LinuxWindowInfo,
) -> Option<&'a LinuxWindowInfo> {
    let mut matches = candidates.iter().filter(|candidate| {
        candidate.backend != expected.backend
            && (same_focus_target_strong(candidate, expected)
                || cross_backend_focus_alias(candidate, expected))
    });
    let candidate = matches.next()?;
    matches.next().is_none().then_some(candidate)
}

pub(crate) fn same_focus_target(
    focused: &LinuxWindowInfo,
    expected: &LinuxWindowInfo,
    candidates: &[LinuxWindowInfo],
) -> bool {
    same_focus_target_strong(focused, expected)
        || unique_cross_backend_focus_target(candidates, expected).is_some_and(|candidate| {
            candidate.backend == focused.backend && candidate.window_id == focused.window_id
        })
}

fn optional_same(left: &Option<String>, right: &Option<String>) -> bool {
    left.as_deref()
        .zip(right.as_deref())
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn anyhow_error(error: anyhow::Error) -> BackendError {
    BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!("{error:#}"),
    )
}

impl BackendKind {
    fn id(self) -> &'static str {
        match self {
            BackendKind::GnomeExtension => GNOME_SHELL_EXTENSION_BACKEND,
            BackendKind::GnomeIntrospect => GNOME_SHELL_INTROSPECT_BACKEND,
            BackendKind::Cosmic => COSMIC_WAYLAND_BACKEND,
            BackendKind::Kwin => KWIN_BACKEND,
            BackendKind::Hyprland => HYPRLAND_BACKEND,
            BackendKind::I3 => I3_BACKEND,
            BackendKind::X11 => X11_BACKEND,
        }
    }
    fn failure_label(self) -> &'static str {
        descriptor(self.id())
            .map(|item| item.failure_label)
            .unwrap_or(self.id())
    }
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{
        AppInfo, CaptureBackendKind, CoordinateSpace, EnvironmentInfo, InputBackendKind,
        PortalCapabilities, RectF, SemanticBackendKind, SessionKind,
    };

    use super::*;

    fn app(id: &str) -> AppInfo {
        AppInfo {
            app_id: id.to_string(),
            name: "Terminal".to_string(),
            pid: Some(42),
            executable: Some("ghostty".to_string()),
            desktop_file_id: Some("com.mitchellh.ghostty.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("XWayland".to_string()),
            window_title: Some("Terminal".to_string()),
            is_focused_candidate: true,
        }
    }

    fn linux_window(backend: &str, window_id: &str) -> LinuxWindowInfo {
        LinuxWindowInfo {
            window_id: window_id.to_string(),
            title: Some("Terminal".to_string()),
            app_id: Some("com.mitchellh.ghostty.desktop".to_string()),
            wm_class: Some("Ghostty".to_string()),
            pid: Some(42),
            bounds: Some(RectF {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: false,
            hidden: false,
            client_type: None,
            backend: backend.to_string(),
            terminal: None,
            terminal_target_sessions: Vec::new(),
        }
    }

    fn wayland_environment() -> EnvironmentInfo {
        EnvironmentInfo {
            session_kind: SessionKind::Wayland,
            compositor: Some("kde-kwin-wayland".to_string()),
            desktop_environment: Some("KDE".to_string()),
            capture_backend: CaptureBackendKind::PortalPipeWire,
            input_backend: InputBackendKind::PortalRemoteDesktop,
            semantic_backend: SemanticBackendKind::Atspi,
            portal_capabilities: PortalCapabilities {
                screencast_version: Some(5),
                remote_desktop_version: Some(2),
                screenshot_version: Some(2),
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

    #[test]
    fn window_targeting_uses_gnome_extension_only_for_gnome_or_unknown() {
        let mut kde = wayland_environment();
        assert!(
            !window_targeting_uses_gnome_extension(&kde),
            "KDE/KWin must not take the GNOME Shell extension path"
        );

        kde.desktop_environment = Some("GNOME".to_string());
        kde.compositor = Some("gnome-shell".to_string());
        assert!(
            window_targeting_uses_gnome_extension(&kde),
            "GNOME sessions use the bundled extension"
        );

        let mut unknown = wayland_environment();
        unknown.desktop_environment = None;
        unknown.compositor = None;
        assert!(
            window_targeting_uses_gnome_extension(&unknown),
            "an unknown session with no desktop signal keeps the historical GNOME default"
        );
    }

    #[test]
    fn converts_x11_window_to_linux_window() {
        let converted = linux_window_from_x11(X11WindowInfo {
            window_id: "0x1".to_string(),
            instance_name: Some("ghostty".to_string()),
            class_name: Some("Ghostty".to_string()),
            app: app("x11:0x1"),
            bounds: Some(RectF {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            workspace: Some(2),
            child_regions: Vec::new(),
        });
        assert_eq!(converted.window_id, "0x1");
        assert_eq!(
            converted.app_id.as_deref(),
            Some("com.mitchellh.ghostty.desktop")
        );
        assert_eq!(converted.wm_class.as_deref(), Some("Ghostty"));
        assert_eq!(converted.client_type.as_deref(), Some("xwayland"));
        assert_eq!(converted.workspace, Some(2));
        assert!(converted.focused);
    }

    #[test]
    fn backend_order_prefers_extension_before_fallbacks() {
        assert_eq!(BACKEND_ORDER[0].id(), GNOME_SHELL_EXTENSION_BACKEND);
        assert_eq!(BACKEND_ORDER[1].id(), GNOME_SHELL_INTROSPECT_BACKEND);
    }

    #[test]
    fn kwin_descriptor_advertises_exact_focus() {
        assert!(backend_can_exact_focus(KWIN_BACKEND));
    }

    #[test]
    fn activation_guard_rejects_listing_only_backend() {
        let mut window = LinuxWindowInfo {
            window_id: "1".to_string(),
            title: None,
            app_id: None,
            wm_class: None,
            pid: None,
            bounds: None,
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: false,
            hidden: false,
            client_type: None,
            backend: GNOME_SHELL_INTROSPECT_BACKEND.to_string(),
            terminal: None,
            terminal_target_sessions: Vec::new(),
        };

        assert!(ensure_backend_can_exact_focus(&window).is_err());
        window.backend = GNOME_SHELL_EXTENSION_BACKEND.to_string();
        assert!(ensure_backend_can_exact_focus(&window).is_ok());
    }

    #[test]
    fn activation_descriptor_filter_skips_listing_only_backends() {
        let backends = BACKEND_ORDER
            .iter()
            .map(|backend| backend.id())
            .filter(|id| backend_can_exact_focus(id))
            .collect::<Vec<_>>();

        assert!(!backends.contains(&GNOME_SHELL_INTROSPECT_BACKEND));
        assert!(backends.contains(&KWIN_BACKEND));
        assert!(backends.contains(&GNOME_SHELL_EXTENSION_BACKEND));
        assert!(backends.contains(&X11_BACKEND));
    }

    #[test]
    fn runtime_activation_filter_keeps_static_listing_only_backends_out() {
        let environment = wayland_environment();

        assert!(!backend_can_exact_focus_in_environment(
            BackendKind::GnomeIntrospect,
            &environment
        ));
    }

    #[test]
    fn listing_filter_skips_gnome_backends_on_kde() {
        let environment = wayland_environment();

        assert!(!backend_can_list_in_environment(
            BackendKind::GnomeExtension,
            &environment
        ));
        assert!(!backend_can_list_in_environment(
            BackendKind::GnomeIntrospect,
            &environment
        ));
        assert!(backend_can_list_in_environment(
            BackendKind::Kwin,
            &environment
        ));
    }

    #[test]
    fn listing_filter_keeps_gnome_backends_on_gnome() {
        let mut environment = wayland_environment();
        environment.desktop_environment = Some("GNOME".to_string());
        environment.compositor = Some("gnome-shell".to_string());

        assert!(backend_can_list_in_environment(
            BackendKind::GnomeExtension,
            &environment
        ));
        assert!(backend_can_list_in_environment(
            BackendKind::GnomeIntrospect,
            &environment
        ));
        assert!(!backend_can_list_in_environment(
            BackendKind::Kwin,
            &environment
        ));
    }

    #[test]
    fn listing_filter_allows_probe_fallback_when_desktop_is_unknown() {
        let mut environment = wayland_environment();
        environment.desktop_environment = None;
        environment.compositor = None;

        assert!(backend_can_list_in_environment(
            BackendKind::GnomeExtension,
            &environment
        ));
        assert!(backend_can_list_in_environment(
            BackendKind::Kwin,
            &environment
        ));
    }

    #[test]
    fn probe_filter_skips_incompatible_desktop_backends() {
        let environment = wayland_environment();

        let gnome_probe = probe_backend(BackendKind::GnomeIntrospect, &environment);
        assert_eq!(gnome_probe.id, GNOME_SHELL_INTROSPECT_BACKEND);
        assert!(!gnome_probe.ok);
        assert!(!gnome_probe.can_list_windows);
        assert!(gnome_probe.detail.contains("Skipped"));

        let kwin_probe = probe_backend(BackendKind::Kwin, &environment);
        assert_eq!(kwin_probe.id, KWIN_BACKEND);
        assert!(!kwin_probe.detail.contains("Skipped"));
        // Scripted activation and focused-window readback share the same
        // availability: KWin scripting over the session bus.
        assert_eq!(kwin_probe.can_focus_windows, kwin_probe.can_focus_apps);
    }

    #[test]
    fn activation_dedupe_preserves_backend_specific_handles() {
        let mut windows = vec![
            linux_window(GNOME_SHELL_EXTENSION_BACKEND, "1"),
            linux_window(X11_BACKEND, "0x1"),
            linux_window(X11_BACKEND, "0x1"),
        ];

        dedupe_backend_window_ids(&mut windows);

        assert_eq!(windows.len(), 2);
        assert!(windows.iter().any(|window| window.backend == X11_BACKEND));
        assert!(
            windows
                .iter()
                .any(|window| window.backend == GNOME_SHELL_EXTENSION_BACKEND)
        );
    }

    #[test]
    fn discovery_dedupe_preserves_focused_duplicate() {
        let mut unfocused = linux_window(GNOME_SHELL_EXTENSION_BACKEND, "1");
        let mut focused = linux_window(KWIN_BACKEND, "kwin:1");
        unfocused.focused = false;
        focused.focused = true;
        let mut windows = vec![unfocused, focused];

        dedupe_windows(&mut windows);

        assert_eq!(windows.len(), 1);
        assert!(windows[0].focused);
        assert_eq!(windows[0].backend, KWIN_BACKEND);
    }

    #[test]
    fn discovery_dedupe_collapses_xwayland_alias_keeping_cosmic() {
        // An XWayland window appears twice: cosmic-comp surfaces its WM_CLASS
        // instance as the foreign-toplevel app_id, and the X11 backend reports
        // the same window with a `<stem>.desktop` app_id + physical-pixel
        // bounds. Neither pid nor bounds match, so the alias must be collapsed
        // by the WM_CLASS identity; the COSMIC entry (logical bounds, first in
        // backend order) is kept.
        let mut cosmic = linux_window(COSMIC_WAYLAND_BACKEND, "1");
        cosmic.app_id = Some("kwrite".to_string());
        cosmic.wm_class = None;
        cosmic.pid = None;

        let mut x11 = linux_window(X11_BACKEND, "0x800007");
        x11.app_id = Some("kwrite.desktop".to_string());
        x11.wm_class = Some("kwrite".to_string());
        x11.pid = Some(4242);

        let mut windows = vec![cosmic.clone(), x11.clone()];
        dedupe_windows(&mut windows);

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].backend, COSMIC_WAYLAND_BACKEND);
        assert_eq!(windows[0].app_id.as_deref(), Some("kwrite"));
    }

    #[test]
    fn xwayland_alias_requires_a_wm_class_signal() {
        let mut cosmic = linux_window(COSMIC_WAYLAND_BACKEND, "1");
        cosmic.app_id = Some("com.mitchellh.ghostty".to_string());
        cosmic.wm_class = None;
        cosmic.pid = None;

        let mut x11 = linux_window(X11_BACKEND, "0x800007");
        x11.app_id = Some("kwrite.desktop".to_string());
        x11.wm_class = Some("kwrite".to_string());

        assert!(!same_window_identity(&cosmic, &x11));
    }

    #[test]
    fn discovery_dedupe_preserves_two_distinct_windows_of_same_xwayland_app() {
        // Two real toplevels of the same XWayland app: the COSMIC helper lists
        // each once (WM_CLASS as app_id, no PID) and the X11 backend lists each
        // once (`.desktop` app_id, PID). Alias absorption is one-to-one, so
        // both surfaces survive as their COSMIC entries.
        let cosmic_window = |id: &str| {
            let mut window = linux_window(COSMIC_WAYLAND_BACKEND, id);
            window.app_id = Some("kwrite".to_string());
            window.wm_class = None;
            window.pid = None;
            window
        };
        let x11_window = |id: &str| {
            let mut window = linux_window(X11_BACKEND, id);
            window.app_id = Some("kwrite.desktop".to_string());
            window.wm_class = Some("kwrite".to_string());
            window.pid = Some(4242);
            window
        };
        let mut windows = vec![
            cosmic_window("1"),
            cosmic_window("2"),
            x11_window("0x800007"),
            x11_window("0x800008"),
        ];
        dedupe_windows(&mut windows);

        assert_eq!(windows.len(), 2);
        assert!(
            windows
                .iter()
                .all(|window| window.backend == COSMIC_WAYLAND_BACKEND)
        );
        assert_eq!(
            windows
                .iter()
                .map(|window| window.window_id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }

    #[test]
    fn discovery_dedupe_keeps_second_x11_when_cosmic_missed_a_toplevel() {
        // If the COSMIC helper missed one toplevel, one cosmic entry aliases
        // two X11 entries of the same WM_CLASS. The second X11 entry describes
        // a distinct surface and must stay rather than being silently dropped.
        let mut cosmic = linux_window(COSMIC_WAYLAND_BACKEND, "1");
        cosmic.app_id = Some("kwrite".to_string());
        cosmic.wm_class = None;
        cosmic.pid = None;

        let x11_window = |id: &str| {
            let mut window = linux_window(X11_BACKEND, id);
            window.app_id = Some("kwrite.desktop".to_string());
            window.wm_class = Some("kwrite".to_string());
            window.pid = Some(4242);
            window
        };

        let mut windows = vec![cosmic, x11_window("0x800007"), x11_window("0x800008")];
        dedupe_windows(&mut windows);

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].backend, COSMIC_WAYLAND_BACKEND);
        assert_eq!(windows[1].backend, X11_BACKEND);
        assert_eq!(windows[1].window_id, "0x800008");
    }

    #[test]
    fn focus_verification_correlates_xwayland_handles_by_class_and_title() {
        let mut x11 = linux_window(X11_BACKEND, "0x3600030");
        x11.pid = None;
        x11.app_id = Some("xmessage.desktop".to_string());
        x11.wm_class = Some("Xmessage".to_string());
        x11.title = Some("sky-cua xmessage probe".to_string());

        let mut kwin = linux_window(KWIN_BACKEND, "kwin:{uuid}");
        kwin.pid = Some(4242);
        kwin.app_id = Some("kwin:{uuid}".to_string());
        kwin.wm_class = Some("xmessage".to_string());
        kwin.title = Some("sky-cua xmessage probe".to_string());

        assert!(same_focus_target(&x11, &kwin, &[x11.clone()]));
        kwin.title = Some("a different xmessage window".to_string());
        assert!(!same_focus_target(&x11, &kwin, &[x11.clone()]));
    }

    #[test]
    fn focus_verification_rejects_duplicate_cross_backend_aliases() {
        let mut expected = linux_window(KWIN_BACKEND, "kwin:{expected}");
        expected.pid = None;
        expected.app_id = Some("kwin:{expected}".to_string());
        expected.wm_class = Some("xmessage".to_string());
        expected.title = Some("duplicate title".to_string());

        let mut first = linux_window(X11_BACKEND, "0x100");
        first.pid = None;
        first.app_id = Some("xmessage.desktop".to_string());
        first.wm_class = Some("Xmessage".to_string());
        first.title = expected.title.clone();
        first.focused = true;
        let mut second = first.clone();
        second.window_id = "0x200".to_string();
        second.focused = false;

        assert!(!same_focus_target(
            &first,
            &expected,
            &[first.clone(), second]
        ));
    }
}
