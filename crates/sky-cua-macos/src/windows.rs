//! CoreGraphics window enumeration, app listing, and target resolution.

mod list;
mod process;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    AppInfo, AppSelector, DisplayInfo, DisplayIntersection, DisplayRef, FocusedApp, WindowInfo,
    WindowTarget,
};

use crate::coords::desktop_rect;
use crate::types::{MacWindow, WINDOW_BACKEND};

/// `client_type` / `toolkit_guess` label. CoreGraphics knows the window, not
/// the toolkit that drew it, so this names the platform and nothing more.
const CLIENT_TYPE: &str = "macos";

/// Enumerates on-screen, normal-layer application windows in front-to-back
/// order. Never requires Accessibility; titles are `None` without Screen
/// Recording permission.
#[must_use]
pub fn enumerate_windows() -> Vec<MacWindow> {
    let raw = list::read_window_list();
    let front_pid = front_process_pid();
    let mut processes: HashMap<i32, process::ProcessInfo> = HashMap::new();
    let mut windows: Vec<MacWindow> = raw
        .into_iter()
        .filter(list::is_application_window)
        .enumerate()
        .map(|(z_index, raw)| {
            let info = processes
                .entry(raw.pid)
                .or_insert_with(|| process::process_info(raw.pid))
                .clone();
            let [x, y, width, height] = raw.bounds;
            MacWindow {
                window_id: raw.window_id,
                pid: raw.pid,
                owner_name: raw.owner_name,
                bundle_id: info.bundle_id,
                executable: info.executable,
                title: raw.title,
                bounds: desktop_rect(x, y, width, height),
                layer: raw.layer,
                on_screen: raw.on_screen,
                focused: false,
                z_index,
            }
        })
        .collect();
    mark_focused(&mut windows, front_pid);
    windows
}

/// Marks the front-most window owned by `front_pid` as focused. When the
/// front application has no qualifying window (for example Finder with only
/// the desktop), nothing is marked; another application's window is never
/// substituted.
fn mark_focused(windows: &mut [MacWindow], front_pid: Option<i32>) {
    let Some(front_pid) = front_pid else {
        return;
    };
    if let Some(window) = windows
        .iter_mut()
        .filter(|window| window.pid == front_pid)
        .min_by_key(|window| window.z_index)
    {
        window.focused = true;
    }
}

/// The pid of the front-most (active) application, read synchronously from
/// Launch Services so it stays current without a main run loop. `None` when
/// no application is front-most.
#[must_use]
pub fn front_process_pid() -> Option<i32> {
    process::front_process_pid()
}

/// The application identifier reported for `window`: the bundle identifier
/// when known, else the owner name.
#[must_use]
pub fn app_id(window: &MacWindow) -> String {
    window
        .bundle_id
        .clone()
        .unwrap_or_else(|| window.owner_name.clone())
}

/// Maps a window to the contract `WindowInfo`, assigning its display from
/// `displays`. `window_id` is the decimal `CGWindowID`.
#[must_use]
pub fn window_to_model(window: &MacWindow, displays: &[DisplayInfo]) -> WindowInfo {
    let (display, display_intersections) = display_assignment(window, displays);
    WindowInfo {
        window_id: window.window_id.to_string(),
        title: window.title.clone(),
        app_id: Some(app_id(window)),
        wm_class: Some(window.owner_name.clone()).filter(|name| !name.is_empty()),
        pid: u32::try_from(window.pid).ok(),
        bounds: Some(window.bounds.clone()),
        display,
        display_intersections,
        workspace: None,
        focused: window.focused,
        hidden: !window.on_screen,
        client_type: Some(CLIENT_TYPE.to_string()),
        backend: WINDOW_BACKEND.to_string(),
        terminal: None,
    }
}

/// Maps a window to the contract `AppInfo`. `app_id` is the bundle identifier
/// when known, else the owner name.
#[must_use]
pub fn window_to_app(window: &MacWindow) -> AppInfo {
    AppInfo {
        app_id: app_id(window),
        name: app_name(window),
        pid: u32::try_from(window.pid).ok(),
        executable: window.executable.clone(),
        desktop_file_id: None,
        app_user_model_id: None,
        window_handle: Some(window.window_id.to_string()),
        toolkit_guess: Some(CLIENT_TYPE.to_string()),
        window_title: window.title.clone(),
        is_focused_candidate: window.focused,
    }
}

/// Maps a window to the contract `FocusedApp`.
#[must_use]
pub fn window_to_focused_app(window: &MacWindow, displays: &[DisplayInfo]) -> FocusedApp {
    let app = window_to_app(window);
    FocusedApp {
        app_id: app.app_id,
        name: app.name,
        pid: app.pid,
        desktop_file_id: None,
        app_user_model_id: None,
        window_handle: app.window_handle,
        toolkit_guess: app.toolkit_guess,
        window_title: app.window_title,
        display: display_assignment(window, displays).0,
    }
}

fn app_name(window: &MacWindow) -> String {
    if !window.owner_name.trim().is_empty() {
        return window.owner_name.clone();
    }
    window
        .executable
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| window.bundle_id.clone())
        .unwrap_or_else(|| format!("pid {}", window.pid))
}

/// The display holding the largest share of `window`, and every display it
/// intersects ordered by intersection area.
fn display_assignment(
    window: &MacWindow,
    displays: &[DisplayInfo],
) -> (Option<DisplayRef>, Vec<DisplayIntersection>) {
    let mut intersections = displays
        .iter()
        .filter_map(|display| DisplayIntersection::from_bounds(display, &window.bounds))
        .collect::<Vec<_>>();
    intersections.sort_by(|left, right| {
        right
            .intersection_area
            .total_cmp(&left.intersection_area)
            .then_with(|| left.display.index.cmp(&right.display.index))
    });
    let display = intersections
        .first()
        .map(|intersection| intersection.display.clone());
    (display, intersections)
}

/// How a selector `name` is matched, strongest first. The strongest rule any
/// window satisfies is the only one applied, so a weaker match never competes
/// with a stronger one.
#[derive(Debug, Clone, Copy)]
enum NameRule {
    /// Equals the owner name, bundle identifier, or executable file name.
    IdentityExact,
    /// Contained in the owner name, bundle identifier, or executable path.
    IdentityContains,
    /// Contained in the window title.
    TitleContains,
}

impl NameRule {
    fn strongest_for(windows: &[MacWindow], name: &str) -> Self {
        if windows.iter().any(|window| identity_equals(window, name)) {
            Self::IdentityExact
        } else if windows.iter().any(|window| identity_contains(window, name)) {
            Self::IdentityContains
        } else {
            Self::TitleContains
        }
    }

    fn matches(self, window: &MacWindow, name: &str) -> bool {
        match self {
            Self::IdentityExact => identity_equals(window, name),
            Self::IdentityContains => identity_contains(window, name),
            Self::TitleContains => window
                .title
                .as_deref()
                .is_some_and(|title| title.to_ascii_lowercase().contains(name)),
        }
    }
}

/// Picks the window an `AppSelector` names. Every supplied field (`app_id`,
/// `window_title`, `name`) must match; `desktop_file_id` has no macOS
/// meaning and is not matched. `name` is matched by the strongest
/// [`NameRule`] any window satisfies: an application named exactly `name`
/// outranks one whose identity merely contains it (`Code` is not `Xcode`),
/// and a title substring counts only when no window matches by application
/// identity. Among the remaining windows the front-most wins. `None` when no
/// field is supplied or no window matches all of them.
#[must_use]
pub fn select_window(windows: &[MacWindow], selector: &AppSelector) -> Option<MacWindow> {
    let app_id = normalized_target(selector.app_id.as_deref());
    let window_title =
        normalized_target(selector.window_title.as_deref()).map(|title| title.to_ascii_lowercase());
    let name = normalized_target(selector.name.as_deref()).map(|name| name.to_ascii_lowercase());
    if app_id.is_none() && window_title.is_none() && name.is_none() {
        return None;
    }
    let name_rule = name
        .as_deref()
        .map(|name| NameRule::strongest_for(windows, name));
    windows
        .iter()
        .filter(|window| {
            let app_id_match = app_id.as_deref().is_none_or(|value| {
                app_id_matches(window, value) || window_id_matches(window, value)
            });
            let title_match = window_title.as_deref().is_none_or(|value| {
                let title = window.title.as_deref().unwrap_or_default();
                !title.is_empty() && title.to_ascii_lowercase().contains(value)
            });
            let name_match = name
                .as_deref()
                .zip(name_rule)
                .is_none_or(|(value, rule)| rule.matches(window, value));
            app_id_match && title_match && name_match
        })
        .min_by_key(|window| window.z_index)
        .cloned()
}

/// Resolves an explicit window target. An exact `window_id` wins over broad
/// metadata; an ambiguous title or app match is an error, never a guess.
pub fn resolve_window_target(
    windows: &[MacWindow],
    target: &WindowTarget,
) -> Result<MacWindow, BackendError> {
    if let Some(window_id) = normalized_target(target.window_id.as_deref()) {
        let matches = windows
            .iter()
            .filter(|window| window_id_matches(window, &window_id))
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("window_id {window_id}"));
    }

    if let Some(value) = normalized_target(target.app_id.as_deref()) {
        let matches = windows
            .iter()
            .filter(|window| app_id_matches(window, &value))
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("app_id {value}"));
    }

    if let Some(pid) = target.pid {
        let matches = windows
            .iter()
            .filter(|window| u32::try_from(window.pid).ok() == Some(pid))
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("pid {pid}"));
    }

    if let Some(wm_class) = normalized_target(target.wm_class.as_deref()) {
        let wm_class = wm_class.to_ascii_lowercase();
        let matches = windows
            .iter()
            .filter(|window| identity_contains(window, &wm_class))
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("wm_class containing {wm_class}"));
    }

    if let Some(title) = normalized_target(target.title.as_deref()) {
        let title_lower = title.to_ascii_lowercase();
        let matches = windows
            .iter()
            .filter(|window| {
                window.title.as_deref().is_some_and(|window_title| {
                    window_title.eq_ignore_ascii_case(&title)
                        || window_title.to_ascii_lowercase().contains(&title_lower)
                })
            })
            .collect::<Vec<_>>();
        return unique_windows_match(matches, &format!("title containing {title}"));
    }

    Err(BackendError::new(
        BackendErrorCode::InvalidRequest,
        "Pass window_id, pid, app_id, wm_class, or title to target a macOS window.",
    ))
}

/// `value` names this window's decimal `CGWindowID`.
fn window_id_matches(window: &MacWindow, value: &str) -> bool {
    value
        .trim()
        .parse::<u32>()
        .is_ok_and(|id| id == window.window_id)
}

/// `value` is this window's bundle identifier or owner name, ignoring case.
fn app_id_matches(window: &MacWindow, value: &str) -> bool {
    let value = value.trim();
    window
        .bundle_id
        .as_deref()
        .is_some_and(|bundle_id| bundle_id.eq_ignore_ascii_case(value))
        || window.owner_name.eq_ignore_ascii_case(value)
}

/// The owner name, bundle identifier, or executable file name equals `name`,
/// ignoring case.
fn identity_equals(window: &MacWindow, name: &str) -> bool {
    let executable_name = window
        .executable
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .and_then(std::ffi::OsStr::to_str);
    [
        Some(window.owner_name.as_str()),
        window.bundle_id.as_deref(),
        executable_name,
    ]
    .into_iter()
    .flatten()
    .any(|value| value.eq_ignore_ascii_case(name))
}

/// The owner name, bundle identifier, or executable path contains the
/// already-lowercased `needle`.
fn identity_contains(window: &MacWindow, needle: &str) -> bool {
    [
        Some(window.owner_name.as_str()),
        window.bundle_id.as_deref(),
        window.executable.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains(needle))
}

fn unique_windows_match(
    matches: Vec<&MacWindow>,
    description: &str,
) -> Result<MacWindow, BackendError> {
    match matches.as_slice() {
        [window] => Ok((*window).clone()),
        [] => Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("No macOS window matched {description}."),
        )),
        windows => {
            let ids = windows
                .iter()
                .map(|window| window.window_id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!(
                    "{description} matched multiple macOS windows ({ids}); add window_id to disambiguate."
                ),
            ))
        }
    }
}

fn normalized_target(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

/// Brings the owning application to the front. Raising the specific window
/// within the application is `AxRuntime::raise_window`.
///
/// This is a request, not a guarantee. Since macOS 14 activation is
/// cooperative: `NSApplicationActivateIgnoringOtherApps` is deprecated and has
/// no effect, and the system may decline or defer a request from a process
/// that is not itself active. `Ok` means only that macOS accepted the
/// request; callers verify the result with [`front_process_pid`].
pub fn activate_application(window: &MacWindow) -> Result<(), BackendError> {
    let requested = objc2::rc::autoreleasepool(|_| {
        NSRunningApplication::runningApplicationWithProcessIdentifier(window.pid).map(
            |application| application.activateWithOptions(NSApplicationActivationOptions::empty()),
        )
    });
    match requested {
        Some(true) => Ok(()),
        None => Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!(
                "macOS application pid {} for window {} is not running",
                window.pid, window.window_id
            ),
        )),
        Some(false) => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!(
                "macOS declined to activate application pid {} for window {}",
                window.pid, window.window_id
            ),
        )),
    }
}
