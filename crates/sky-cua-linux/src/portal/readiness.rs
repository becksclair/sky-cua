//! Keep the daemon from starting xdg-desktop-portal before the graphical
//! session exists, and explain a missing capture backend.
//!
//! D-Bus activation starts xdg-desktop-portal through systemd with the user
//! manager's environment. Before the desktop session imports
//! `XDG_CURRENT_DESKTOP` into that environment (Plasma does so in
//! `startplasma`), the portal cannot select its desktop backend, falls back to
//! gtk for everything and exposes no ScreenCast, RemoteDesktop or Screenshot
//! implementation for the rest of the session. A daemon spawned at boot by a
//! user service (an agent gateway, for example) used to trigger exactly that
//! with its first capability probe.

use std::sync::Mutex;
use std::time::Duration;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::DoctorCaptureUnavailable;
use tracing::debug;

use crate::portal::session::session_bus;

const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_KEY: &str = "XDG_CURRENT_DESKTOP";
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Why the most recent environment probe found no capture backend; `None`
/// when it selected one.
static LAST_CAPTURE_UNAVAILABLE: Mutex<Option<CaptureUnavailable>> = Mutex::new(None);

/// Structured cause for a missing capture backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureUnavailable {
    /// The probe skipped the portal because the session has not named itself.
    SessionNotReady,
    /// The running portal was started without `XDG_CURRENT_DESKTOP`.
    StartedWithoutDesktop { pid: u32 },
    /// The portal exposes no capture implementation for another reason.
    NoCaptureInterface,
}

impl CaptureUnavailable {
    fn code(self) -> BackendErrorCode {
        match self {
            Self::SessionNotReady => BackendErrorCode::PortalSessionNotReady,
            Self::StartedWithoutDesktop { .. } | Self::NoCaptureInterface => {
                BackendErrorCode::PortalBackendMissing
            }
        }
    }

    fn message(self) -> String {
        match self {
            Self::SessionNotReady => format!(
                "Screen capture is unavailable until the graphical session is ready: sky-cua did not start xdg-desktop-portal because the systemd user environment has no {DESKTOP_KEY} yet, and a portal started now would load no ScreenCast backend. Retry once the desktop session has finished starting."
            ),
            Self::StartedWithoutDesktop { pid } => format!(
                "Screen capture is unavailable: xdg-desktop-portal (pid {pid}) was started without {DESKTOP_KEY}, so it loaded no ScreenCast, RemoteDesktop or Screenshot backend. Restart it with `systemctl --user restart xdg-desktop-portal`."
            ),
            Self::NoCaptureInterface => "Screen capture is unavailable: the desktop portal exposes no ScreenCast or Screenshot implementation and no X11 capture lane applies."
                .to_string(),
        }
    }

    pub(crate) fn to_error(self) -> BackendError {
        BackendError::new(self.code(), self.message())
    }

    pub(crate) fn to_doctor(self) -> DoctorCaptureUnavailable {
        DoctorCaptureUnavailable {
            code: self.code().as_str().to_string(),
            message: self.message(),
        }
    }
}

/// Whether the environment probe may query the portal frontend. A running
/// portal is always safe to query. An absent one is only activated once the
/// systemd user environment names the desktop; when that environment cannot
/// be read (no systemd user manager, a private bus) activation is allowed as
/// before.
pub(crate) async fn portal_probe_allowed() -> bool {
    tokio::time::timeout(CALL_TIMEOUT, probe_allowed_inner())
        .await
        .unwrap_or(true)
}

async fn probe_allowed_inner() -> bool {
    let Ok(connection) = session_bus().await else {
        return true;
    };
    if portal_has_owner(&connection).await.unwrap_or(true) {
        return true;
    }
    let ready = session_ready_on(&connection).await;
    if !ready {
        debug!("deferring portal probe: the systemd user environment has no {DESKTOP_KEY} yet");
    }
    ready
}

/// Record the outcome of an environment probe: why no capture backend was
/// selected, or that one was. Read back by capture errors and doctor.
pub(crate) async fn record_capture_availability(capture_available: bool, probe_deferred: bool) {
    let cause = if capture_available {
        None
    } else if probe_deferred {
        Some(CaptureUnavailable::SessionNotReady)
    } else {
        match tokio::time::timeout(CALL_TIMEOUT, portal_started_without_desktop())
            .await
            .ok()
            .flatten()
        {
            // Restarting only helps once the user environment names the
            // desktop; before that a restart recreates the same portal.
            Some(_)
                if !tokio::time::timeout(CALL_TIMEOUT, session_ready())
                    .await
                    .unwrap_or(true) =>
            {
                Some(CaptureUnavailable::SessionNotReady)
            }
            Some(pid) => Some(CaptureUnavailable::StartedWithoutDesktop { pid }),
            None => Some(CaptureUnavailable::NoCaptureInterface),
        }
    };
    *LAST_CAPTURE_UNAVAILABLE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = cause;
}

/// Forget the recorded cause (the environment probe found no display at all).
pub(crate) fn clear_capture_unavailable() {
    *LAST_CAPTURE_UNAVAILABLE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Whether the systemd user environment names the desktop; `true` when it
/// cannot be read.
async fn session_ready() -> bool {
    match session_bus().await {
        Ok(connection) => session_ready_on(&connection).await,
        Err(_) => true,
    }
}

async fn session_ready_on(connection: &zbus::Connection) -> bool {
    user_manager_environment(connection)
        .await
        .is_none_or(|environment| desktop_named(environment.iter().map(String::as_str)))
}

/// The cause recorded by the most recent environment probe.
pub(crate) fn last_capture_unavailable() -> Option<CaptureUnavailable> {
    *LAST_CAPTURE_UNAVAILABLE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The error a capture request fails with when no capture backend exists.
pub(crate) fn capture_backend_unavailable_error() -> BackendError {
    last_capture_unavailable()
        .unwrap_or(CaptureUnavailable::NoCaptureInterface)
        .to_error()
}

async fn portal_has_owner(connection: &zbus::Connection) -> Option<bool> {
    let proxy = zbus::fdo::DBusProxy::new(connection).await.ok()?;
    let name = zbus::names::BusName::try_from(PORTAL_BUS_NAME).ok()?;
    proxy.name_has_owner(name).await.ok()
}

async fn user_manager_environment(connection: &zbus::Connection) -> Option<Vec<String>> {
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .await
    .ok()?;
    proxy.get_property::<Vec<String>>("Environment").await.ok()
}

fn desktop_named<'a>(assignments: impl IntoIterator<Item = &'a str>) -> bool {
    assignments.into_iter().any(|assignment| {
        assignment
            .strip_prefix(DESKTOP_KEY)
            .and_then(|rest| rest.strip_prefix('='))
            .is_some_and(|value| !value.trim().is_empty())
    })
}

/// The running portal's pid when its environment lacks the desktop name.
async fn portal_started_without_desktop() -> Option<u32> {
    let connection = session_bus().await.ok()?;
    let proxy = zbus::fdo::DBusProxy::new(&connection).await.ok()?;
    let name = zbus::names::BusName::try_from(PORTAL_BUS_NAME).ok()?;
    let pid = proxy.get_connection_unix_process_id(name).await.ok()?;
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let assignments = environ
        .split(|byte| *byte == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok());
    (!desktop_named(assignments)).then_some(pid)
}

#[cfg(test)]
mod tests {
    use super::{CaptureUnavailable, desktop_named};

    #[test]
    fn causes_map_to_distinct_codes_and_actionable_messages() {
        let deferred = CaptureUnavailable::SessionNotReady.to_error();
        assert_eq!(deferred.code, "PortalSessionNotReady");
        assert!(
            deferred
                .message
                .contains("until the graphical session is ready")
        );

        let stale = CaptureUnavailable::StartedWithoutDesktop { pid: 4242 }.to_doctor();
        assert_eq!(stale.code, "PortalBackendMissing");
        assert!(stale.message.contains("pid 4242"));
        assert!(
            stale
                .message
                .contains("systemctl --user restart xdg-desktop-portal")
        );

        assert_eq!(
            CaptureUnavailable::NoCaptureInterface.to_error().code,
            "PortalBackendMissing"
        );
    }

    #[test]
    fn desktop_named_requires_a_non_empty_value() {
        assert!(desktop_named(["PATH=/usr/bin", "XDG_CURRENT_DESKTOP=KDE"]));
        assert!(!desktop_named([
            "PATH=/usr/bin",
            "XDG_RUNTIME_DIR=/run/user/1000"
        ]));
        assert!(!desktop_named(["XDG_CURRENT_DESKTOP="]));
        assert!(!desktop_named(["XDG_CURRENT_DESKTOP_EXTRA=KDE"]));
    }
}
