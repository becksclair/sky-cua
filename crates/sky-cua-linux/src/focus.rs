use atspi::AccessibilityConnection;
use atspi::State;
use atspi::proxy::accessible::ObjectRefExt;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

use crate::apps::discovery::DiscoveredApp;

pub async fn pick_focused_app(
    connection: &AccessibilityConnection,
    apps: &[DiscoveredApp],
) -> Result<Option<DiscoveredApp>, BackendError> {
    let mut best: Option<(usize, i32)> = None;
    for (index, app) in apps.iter().enumerate() {
        let score = focus_score(connection, app).await? + app_preference_score(app);
        if let Some((_, current_best)) = best {
            if score > current_best {
                best = Some((index, score));
            }
        } else if score > i32::MIN {
            best = Some((index, score));
        }
    }
    Ok(best.map(|(index, _)| {
        let mut app = apps[index].clone();
        app.info.is_focused_candidate = true;
        app
    }))
}

async fn focus_score(
    connection: &AccessibilityConnection,
    app: &DiscoveredApp,
) -> Result<i32, BackendError> {
    let mut score = 0;
    let mut stack = vec![app.object_ref.clone()];
    let mut visited = 0usize;
    while let Some(object_ref) = stack.pop() {
        visited += 1;
        if visited > 128 {
            break;
        }
        let accessible = object_ref
            .as_accessible_proxy(connection.connection())
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!("failed to inspect accessible object for focus detection: {error}"),
                )
            })?;
        let states = accessible.get_state().await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!("failed to read accessibility states: {error}"),
            )
        })?;
        if states.contains(State::Focused) {
            return Ok(100);
        }
        if states.contains(State::Active) {
            score = score.max(10);
        }
        let children = accessible.get_children().await.unwrap_or_default();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    Ok(score)
}

fn app_preference_score(app: &DiscoveredApp) -> i32 {
    let mut score = 0;
    if app.info.window_title.is_some() {
        score += 25;
    }
    if let Some(executable) = app.info.executable.as_deref() {
        let executable = executable.to_ascii_lowercase();
        if executable.contains("service")
            || executable.contains("proxy")
            || executable.contains("menu")
            || executable.contains("portal")
            || executable.contains("daemon")
            || executable.contains("ksmserver")
        {
            score -= 20;
        } else {
            score += 5;
        }
    }
    if app.info.name.eq_ignore_ascii_case("unnamed") {
        score -= 2;
    } else {
        score += 2;
    }
    score
}
