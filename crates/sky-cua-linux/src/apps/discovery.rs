use std::fs;
use std::path::PathBuf;

use atspi::CoordType;
use atspi::State;
use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
use atspi_connection::AccessibilityConnection;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{AppInfo, CoordinateSpace, RectF};
use tracing::debug;
use zbus::fdo::DBusProxy;

#[derive(Debug, Clone)]
pub struct DiscoveredApp {
    pub info: AppInfo,
    pub object_ref: atspi::ObjectRefOwned,
    /// Populated only by exact-window discovery. `None` means top-level
    /// enumeration was not requested or was incomplete and must not be used
    /// to infer a unique accessibility window.
    pub top_levels: Option<Vec<AccessibleTopLevel>>,
}

#[derive(Debug, Clone)]
pub struct AccessibleTopLevel {
    pub object_ref: atspi::ObjectRefOwned,
    pub title: String,
    pub active: bool,
    pub focused: bool,
    pub bounds: Option<RectF>,
}

/// Enumerate AT-SPI application roots.
///
/// `enumerate_top_levels` enables per-app top-level enumeration; it is required
/// only for window correlation (`match_window_accessibility`), where the
/// PID-scoped or PID-less title search needs each app's frame/window children.
/// Generic discovery (`list_apps`, plain `get_app_state`, semantic scroll)
/// must leave it off: enumerating every app's top levels costs several D-Bus
/// round trips per child and the data is never consumed there.
pub async fn discover_apps(
    connection: &AccessibilityConnection,
    window_pid: Option<u32>,
    enumerate_top_levels: bool,
) -> Result<Vec<DiscoveredApp>, BackendError> {
    debug!("discovering AT-SPI applications");
    let root = connection
        .root_accessible_on_registry()
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityUnavailable,
                format!("failed to read AT-SPI registry root: {error}"),
            )
        })?;
    let app_refs = root.get_children().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityUnavailable,
            format!("failed to enumerate AT-SPI applications: {error}"),
        )
    })?;
    debug!(
        count = app_refs.len(),
        "enumerated AT-SPI application roots"
    );

    let dbus_proxy = DBusProxy::new(connection.connection())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityUnavailable,
                format!("failed to open D-Bus proxy on accessibility bus: {error}"),
            )
        })?;

    let mut apps = Vec::new();
    for object_ref in app_refs {
        let accessible = match object_ref
            .as_accessible_proxy(connection.connection())
            .await
        {
            Ok(proxy) => proxy,
            Err(_) => continue,
        };

        let name = accessible
            .name()
            .await
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Unnamed App".to_string());
        let proxies = accessible.proxies().await.ok();
        let toolkit_guess = if let Some(proxies) = proxies.as_ref() {
            if let Ok(proxy) = proxies.application().await {
                proxy
                    .toolkit_name()
                    .await
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            } else {
                None
            }
        } else {
            None
        };
        let destination = accessible.inner().destination().clone();
        let pid = dbus_proxy
            .get_connection_unix_process_id(destination.clone())
            .await
            .ok();
        let executable = pid.and_then(read_executable);
        let desktop_file_id = executable
            .as_deref()
            .map(guess_desktop_file_id)
            .or_else(|| {
                Some(format!(
                    "{}.desktop",
                    normalize_name(&name).replace(' ', "-")
                ))
            });
        // `best_window_like_title` is only consumed by app-list/selector
        // scoring. When top-level enumeration is requested (the PID-less
        // correlation path), `match_window_accessibility` reads `top_levels`
        // directly, so the DFS title walk would be a second pass over the same
        // children for no consumer; skip it to avoid doubling the per-app
        // D-Bus round trips.
        let window_title = if window_pid.is_none() && !enumerate_top_levels {
            best_window_like_title(&accessible, connection.connection()).await
        } else {
            None
        };
        let top_levels = if enumerate_top_levels
            && window_pid.is_none_or(|window_pid| pid == Some(window_pid))
        {
            collect_top_levels(&accessible, connection.connection()).await
        } else {
            None
        };

        let app_id = format!(
            "{}:{}",
            accessible.inner().destination(),
            accessible.inner().path()
        );
        apps.push(DiscoveredApp {
            info: AppInfo {
                app_id,
                name,
                pid,
                executable,
                desktop_file_id,
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess,
                window_title,
                is_focused_candidate: false,
            },
            object_ref,
            top_levels,
        });
    }

    debug!(count = apps.len(), "finished AT-SPI app discovery");
    Ok(apps)
}

async fn collect_top_levels(
    accessible: &atspi::proxy::accessible::AccessibleProxy<'_>,
    connection: &zbus::Connection,
) -> Option<Vec<AccessibleTopLevel>> {
    let mut top_levels = Vec::new();
    for object_ref in accessible.get_children().await.unwrap_or_default() {
        let child = object_ref.as_accessible_proxy(connection).await.ok()?;
        let role = child.get_role_name().await.ok()?.to_ascii_lowercase();
        if !role.contains("frame")
            && !role.contains("window")
            && !role.contains("dialog")
            && !role.contains("alert")
        {
            continue;
        }
        let title = child.name().await.ok()?;
        let states = child.get_state().await.ok();
        let bounds = if let Ok(proxies) = child.proxies().await
            && let Ok(component) = proxies.component().await
        {
            component
                .get_extents(CoordType::Screen)
                .await
                .ok()
                .map(|(x, y, width, height)| RectF {
                    x: f64::from(x),
                    y: f64::from(y),
                    width: f64::from(width),
                    height: f64::from(height),
                    space: CoordinateSpace::DesktopLogical,
                })
        } else {
            None
        };
        top_levels.push(AccessibleTopLevel {
            object_ref,
            title,
            active: states
                .as_ref()
                .is_some_and(|states| states.contains(State::Active)),
            focused: states
                .as_ref()
                .is_some_and(|states| states.contains(State::Focused)),
            bounds,
        });
    }
    Some(top_levels)
}

async fn best_window_like_title(
    accessible: &atspi::proxy::accessible::AccessibleProxy<'_>,
    connection: &zbus::Connection,
) -> Option<String> {
    let mut best: Option<(i32, String)> = None;
    let mut stack = accessible
        .get_children()
        .await
        .ok()?
        .into_iter()
        .map(|child| (child, 1usize))
        .collect::<Vec<_>>();

    while let Some((child_ref, depth)) = stack.pop() {
        if depth > 4 {
            continue;
        }
        let child = match child_ref.as_accessible_proxy(connection).await {
            Ok(child) => child,
            Err(_) => continue,
        };
        let child_name = child.name().await.ok().unwrap_or_default();
        let role_name = child
            .get_role_name()
            .await
            .ok()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let states = child.get_state().await.ok();

        if !child_name.trim().is_empty() {
            let mut score = 0i32;
            if role_name.contains("frame")
                || role_name.contains("window")
                || role_name.contains("dialog")
                || role_name.contains("alert")
            {
                score += 20;
            }
            if let Some(states) = states.as_ref() {
                if states.contains(State::Focused) {
                    score += 50;
                }
                if states.contains(State::Active) {
                    score += 15;
                }
            }
            score += 5;
            score -= i32::try_from(depth).unwrap_or(0);

            match best.as_ref() {
                Some((best_score, _)) if *best_score >= score => {}
                _ => best = Some((score, child_name.clone())),
            }
        }

        let children = child.get_children().await.unwrap_or_default();
        for grandchild in children.into_iter().rev() {
            stack.push((grandchild, depth + 1));
        }
    }

    best.map(|(_, title)| title)
}

fn read_executable(pid: u32) -> Option<String> {
    let path = PathBuf::from(format!("/proc/{pid}/exe"));
    fs::read_link(path).ok().and_then(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
    })
}

fn guess_desktop_file_id(executable: &str) -> String {
    format!("{}.desktop", executable.to_lowercase())
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
