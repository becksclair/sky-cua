mod logind;
mod screensaver;

use std::sync::Arc;

use sky_cua_platform::diagnostics::BackendError;
use sky_cua_platform::model::{
    DoctorCheck, DoctorSessionPresenceReport, SessionPresenceIntent, SessionPresenceStatus,
};
use tokio::sync::RwLock;
use zbus::zvariant::{OwnedFd, OwnedObjectPath};

const BACKEND_NAME: &str = "systemd-logind+screensaver";

#[derive(Debug, Clone)]
pub struct SessionPresenceManager {
    inner: Arc<RwLock<SessionPresenceState>>,
}

#[derive(Debug, Default)]
struct SessionPresenceState {
    system_connection: Option<zbus::Connection>,
    session_id: Option<String>,
    session_path: Option<OwnedObjectPath>,
    sleep_inhibitor: Option<OwnedFd>,
    lock_inhibitor: Option<screensaver::LockInhibitor>,
}

impl SessionPresenceManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(SessionPresenceState::default())),
        }
    }

    pub async fn ensure(&self, intent: SessionPresenceIntent) -> SessionPresenceStatus {
        let mut state = self.inner.write().await;
        let mut details = Vec::new();

        if intent.unlock {
            match ensure_logind_session(&mut state).await {
                Ok(session) if session.locked => {
                    match logind::unlock_session(
                        &system_connection(&mut state)
                            .await
                            .expect("connection already resolved"),
                        &session.id,
                    )
                    .await
                    {
                        Ok(()) => details.push(format!("requested UnlockSession({})", session.id)),
                        Err(error) => details.push(error.message),
                    }
                }
                Ok(session) => details.push(format!("session {} is already unlocked", session.id)),
                Err(error) => details.push(error.message),
            }
        }

        if intent.inhibit_suspend && state.sleep_inhibitor.is_none() {
            match system_connection(&mut state).await {
                Ok(connection) => match logind::inhibit_suspend(&connection).await {
                    Ok(fd) => {
                        state.sleep_inhibitor = Some(fd);
                        details.push("acquired logind sleep inhibitor".to_string());
                    }
                    Err(error) => details.push(error.message),
                },
                Err(error) => details.push(error.message),
            }
        }

        if intent.inhibit_lock && state.lock_inhibitor.is_none() {
            match screensaver::inhibit_lock().await {
                Ok(inhibitor) => {
                    let cookie = inhibitor.cookie;
                    state.lock_inhibitor = Some(inhibitor);
                    details.push(format!("acquired ScreenSaver inhibitor cookie {cookie}"));
                }
                Err(error) => details.push(error.message),
            }
        }

        status_from_state(&mut state, details).await
    }

    pub async fn release(&self, relock: bool) -> SessionPresenceStatus {
        let mut state = self.inner.write().await;
        let mut details = Vec::new();

        state.sleep_inhibitor = None;
        details.push("released logind sleep inhibitor".to_string());

        if let Some(inhibitor) = state.lock_inhibitor.take() {
            let cookie = inhibitor.cookie;
            match screensaver::uninhibit(&inhibitor).await {
                Ok(()) => details.push(format!("released ScreenSaver inhibitor cookie {cookie}")),
                Err(error) => details.push(error.message),
            }
        } else {
            details.push("no ScreenSaver inhibitor was held".to_string());
        }

        if relock {
            match ensure_logind_session(&mut state).await {
                Ok(session) => {
                    match logind::lock_session(
                        &system_connection(&mut state)
                            .await
                            .expect("connection already resolved"),
                        &session.id,
                    )
                    .await
                    {
                        Ok(()) => details.push(format!("requested LockSession({})", session.id)),
                        Err(error) => details.push(error.message),
                    }
                }
                Err(error) => details.push(error.message),
            }
        }

        status_from_state(&mut state, details).await
    }

    pub async fn status(&self) -> SessionPresenceStatus {
        let mut state = self.inner.write().await;
        status_from_state(&mut state, Vec::new()).await
    }

    pub async fn doctor_report(&self) -> DoctorSessionPresenceReport {
        let mut state = self.inner.write().await;

        let (unlock, lock_state_readable) = match ensure_logind_session(&mut state).await {
            Ok(session) => {
                let detail = format!(
                    "logind session {} LockedHint={}",
                    session.id, session.locked
                );
                (
                    DoctorCheck {
                        name: "unlock".to_string(),
                        ok: true,
                        detail: detail.clone(),
                    },
                    DoctorCheck {
                        name: "lock_state_readable".to_string(),
                        ok: true,
                        detail,
                    },
                )
            }
            Err(error) => {
                let detail = error.message;
                (
                    DoctorCheck {
                        name: "unlock".to_string(),
                        ok: false,
                        detail: detail.clone(),
                    },
                    DoctorCheck {
                        name: "lock_state_readable".to_string(),
                        ok: false,
                        detail,
                    },
                )
            }
        };

        let inhibit_suspend = match system_connection(&mut state).await {
            Ok(_) => DoctorCheck {
                name: "inhibit_suspend".to_string(),
                ok: true,
                detail: "system bus and logind manager are reachable".to_string(),
            },
            Err(error) => DoctorCheck {
                name: "inhibit_suspend".to_string(),
                ok: false,
                detail: error.message,
            },
        };

        let inhibit_lock = match screensaver::session_connection().await {
            Ok(connection) => match screensaver::name_has_owner(&connection).await {
                Ok(true) => DoctorCheck {
                    name: "inhibit_lock".to_string(),
                    ok: true,
                    detail: format!(
                        "{} is owned on the session bus",
                        screensaver::SCREENSAVER_DEST
                    ),
                },
                Ok(false) => DoctorCheck {
                    name: "inhibit_lock".to_string(),
                    ok: false,
                    detail: format!(
                        "{} is not owned on the session bus",
                        screensaver::SCREENSAVER_DEST
                    ),
                },
                Err(error) => DoctorCheck {
                    name: "inhibit_lock".to_string(),
                    ok: false,
                    detail: error.message,
                },
            },
            Err(error) => DoctorCheck {
                name: "inhibit_lock".to_string(),
                ok: false,
                detail: error.message,
            },
        };

        DoctorSessionPresenceReport {
            backend: BACKEND_NAME.to_string(),
            unlock,
            inhibit_lock,
            inhibit_suspend,
            lock_state_readable,
        }
    }
}

impl Default for SessionPresenceManager {
    fn default() -> Self {
        Self::new()
    }
}

async fn status_from_state(
    state: &mut SessionPresenceState,
    mut details: Vec<String>,
) -> SessionPresenceStatus {
    let mut locked = None;
    let mut unlock_supported = false;

    match ensure_logind_session(state).await {
        Ok(session) => {
            locked = Some(session.locked);
            unlock_supported = true;
            if details.is_empty() {
                details.push(format!(
                    "session {} LockedHint={}",
                    session.id, session.locked
                ));
            }
        }
        Err(error) => {
            details.push(error.message);
        }
    }

    SessionPresenceStatus {
        backend: BACKEND_NAME.to_string(),
        supported: true,
        unlock_supported,
        locked,
        lock_inhibited: state.lock_inhibitor.is_some(),
        suspend_inhibited: state.sleep_inhibitor.is_some(),
        detail: detail_string(details),
    }
}

async fn ensure_logind_session(
    state: &mut SessionPresenceState,
) -> Result<logind::ResolvedSession, BackendError> {
    let connection = system_connection(state).await?;
    let session = logind::resolve_session(&connection).await?;
    state.session_id = Some(session.id.clone());
    state.session_path = Some(session.path.clone());
    Ok(session)
}

async fn system_connection(
    state: &mut SessionPresenceState,
) -> Result<zbus::Connection, BackendError> {
    if state.system_connection.is_none() {
        state.system_connection = Some(logind::system_connection().await?);
    }
    Ok(state
        .system_connection
        .as_ref()
        .expect("system connection set above")
        .clone())
}

fn detail_string(details: Vec<String>) -> String {
    if details.is_empty() {
        "session presence is ready".to_string()
    } else {
        details.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_reports_linux_presence_backend_as_supported() {
        let manager = SessionPresenceManager::new();
        let status = manager.status().await;

        assert_eq!(status.backend, BACKEND_NAME);
        assert!(status.supported);
    }

    #[tokio::test]
    async fn empty_ensure_and_release_are_idempotent() {
        let manager = SessionPresenceManager::new();
        let intent = SessionPresenceIntent::default();

        let ensured = manager.ensure(intent).await;
        let released_once = manager.release(false).await;
        let released_twice = manager.release(false).await;

        assert!(ensured.supported);
        assert!(!released_once.lock_inhibited);
        assert!(!released_once.suspend_inhibited);
        assert!(!released_twice.lock_inhibited);
        assert!(!released_twice.suspend_inhibited);
    }
}
