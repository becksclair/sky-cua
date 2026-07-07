use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sky_cua_platform::DESKTOP_LAUNCH_ENV_KEYS;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    ActionName, ActionRequest, AppStateSnapshot, BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
    BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserRequest, BrowserResponse, CaptureInfo,
    CaptureScreenMode, DiagnosticEntry, DisplayTarget, PhoneBackendKind, PhoneRequest,
    ServiceRequest, ServiceResponse, SessionPresenceAction, SessionPresenceIntent, WindowInfo,
    WindowTarget, browser_eval_enabled,
};

use crate::action_router::route_action;
use crate::approval_store::ApprovalStore;
use crate::backend_factory::create_backend;
use crate::diagnostics::error_response;
use crate::element_resolver::{resolve_action_element, resolve_target_element};
use crate::overlay::{AgentCursorStatus, OverlayController};
use crate::phone::host_scrcpy_default_max_size;
use crate::session_store::SessionStore;
use crate::snapshot_manager::SnapshotManager;
use tracing::debug;

/// Deadline for a single read-only desktop backend call made while holding
/// `desktop_lane` (see [`ServiceDaemon::with_desktop_deadline`]). Bounds the
/// unbounded zbus AT-SPI calls and PipeWire portal hangs that would
/// otherwise wedge every subsequent desktop request behind the shared lane.
/// Overridable via `SKY_CUA_DESKTOP_REQUEST_DEADLINE_MS` so tests can
/// exercise the timeout path without waiting out the production default.
/// Kept below the MCP host's own patience (~15s per call, but hosts retry)
/// and the Rust client's socket read timeout (60s, `service_launcher.rs`).
fn desktop_request_deadline() -> Duration {
    static DEADLINE: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *DEADLINE.get_or_init(|| {
        std::env::var("SKY_CUA_DESKTOP_REQUEST_DEADLINE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(50))
    })
}

/// Cap on how long the elapsed-deadline recovery path (dropping cached
/// AT-SPI/portal session handles) is allowed to run. The recovery itself
/// makes a best-effort D-Bus close call that could, in principle, hit the
/// same unbounded-zbus-timeout hang this whole mechanism exists to route
/// around; bounding it keeps the lane-freeing guarantee intact even if the
/// backend's own cleanup misbehaves.
const DESKTOP_DEADLINE_RESET_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ServiceDaemon {
    backend: Box<dyn DesktopBackend>,
    sessions: SessionStore,
    snapshots: tokio::sync::Mutex<SnapshotManager>,
    overlay: tokio::sync::Mutex<OverlayController>,
    phone: tokio::sync::Mutex<crate::phone::PhoneManager>,
    session_presence_config: SessionPresenceConfig,
    session_presence_held: tokio::sync::Mutex<bool>,
    desktop_lane: tokio::sync::Mutex<()>,
    browser_eval_enabled: bool,
    socket_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionPresenceConfig {
    enabled: bool,
    idle_release: Duration,
    unlock: bool,
    relock: bool,
    inhibit_lock: bool,
    inhibit_suspend: bool,
}

mod agent_cursor;
mod browser;
mod capture_reuse;
mod desktop;
mod phone;
mod session_presence;

#[cfg(test)]
mod tests;

impl ServiceDaemon {
    pub async fn new(socket_path: PathBuf) -> std::io::Result<Self> {
        ApprovalStore::initialize()?;
        let backend = create_backend();
        if let Err(error) = backend.prepare_automation_permissions().await {
            debug!(
                code = error.code,
                message = error.message,
                "desktop backend automation permission preparation did not complete"
            );
        }
        Ok(Self {
            backend,
            sessions: SessionStore::new(),
            snapshots: tokio::sync::Mutex::new(SnapshotManager::new(8)),
            overlay: tokio::sync::Mutex::new(OverlayController::new(&socket_path)),
            phone: tokio::sync::Mutex::new(crate::phone::PhoneManager::new()),
            session_presence_config: SessionPresenceConfig::from_env(),
            session_presence_held: tokio::sync::Mutex::new(false),
            desktop_lane: tokio::sync::Mutex::new(()),
            browser_eval_enabled: browser_eval_enabled(),
            socket_path,
        })
    }

    /// Spawn a background task that asks the service-owned overlay controller
    /// to clean up once the agent cursor overlay has been idle past the
    /// timeout. Interrupted, abandoned, or explicitly ended agent sessions must
    /// not leave the overlay shown or the user's cursor hidden.
    pub fn spawn_overlay_idle_watchdog(self: &std::sync::Arc<Self>) -> tokio::task::JoinHandle<()> {
        let daemon = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let diagnostics = daemon.overlay.lock().await.hide_idle_overlay();
                for entry in diagnostics {
                    debug!(
                        code = entry.code,
                        message = entry.message,
                        "overlay idle cleanup"
                    );
                }
            }
        })
    }

    /// Spawn a background task that releases session-presence inhibitors once
    /// the daemon has been idle past the configured timeout.
    pub fn spawn_session_presence_watchdog(
        self: &std::sync::Arc<Self>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if !self.session_presence_config.enabled {
            return None;
        }
        let daemon = std::sync::Arc::clone(self);
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                daemon.release_idle_session_presence_if_needed().await;
            }
        }))
    }

    /// Spawn a background task that detects a managed scrcpy mirror that died
    /// mid-session (crash or the operator closing the mirror window) and downgrades
    /// its capabilities so routing stops treating it as a live mirror.
    ///
    /// After the post-spawn liveness check nothing else re-polls the scrcpy child,
    /// so without this `scrcpy.active`/`host_window_mapped` would stay stuck true
    /// and the daemon would keep offering the dead window for mapping. Each tick
    /// polls the manager, which downgrades any crashed mirror's capabilities and
    /// tears down its runtime. The agent cursor is drawn on the device by the
    /// companion overlay, not on the host desktop, so a dead mirror no longer
    /// implies a host-desktop overlay to hide here.
    pub fn spawn_scrcpy_liveness_watchdog(
        self: &std::sync::Arc<Self>,
    ) -> tokio::task::JoinHandle<()> {
        let daemon = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                // Polling downgrades crashed mirrors and tears down their runtimes;
                // the returned crash list is no longer consumed for overlay hiding.
                let _ = daemon.phone.lock().await.poll_scrcpy_liveness();
            }
        })
    }

    /// Spawn a background task that clears the phone-native companion
    /// "agent in control" indicator after 20s of session inactivity. This is a
    /// visual lease, not a session teardown: later phone activity can relight the
    /// indicator without forcing agents to reconnect.
    pub fn spawn_phone_overlay_idle_watchdog(
        self: &std::sync::Arc<Self>,
    ) -> tokio::task::JoinHandle<()> {
        let daemon = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let expired = daemon
                    .phone
                    .lock()
                    .await
                    .expire_idle_companion_overlays(crate::phone::PhoneManager::current_time_ms())
                    .await;
                for session_id in expired {
                    debug!(
                        session_id,
                        "phone companion active overlay expired after idle timeout"
                    );
                }
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests() -> std::io::Result<Self> {
        Ok(Self {
            backend: crate::backend_factory::create_backend(),
            sessions: SessionStore::new(),
            snapshots: tokio::sync::Mutex::new(SnapshotManager::new(8)),
            overlay: tokio::sync::Mutex::new(OverlayController::new_for_tests()),
            phone: tokio::sync::Mutex::new(crate::phone::PhoneManager::new()),
            session_presence_config: SessionPresenceConfig::disabled(),
            session_presence_held: tokio::sync::Mutex::new(false),
            desktop_lane: tokio::sync::Mutex::new(()),
            browser_eval_enabled: false,
            socket_path: PathBuf::from("/tmp/sky-cua-test.sock"),
        })
    }

    pub async fn handle(&self, request: ServiceRequest) -> ServiceResponse {
        self.sessions.touch().await;
        self.ensure_session_presence_for_request(&request).await;
        match request {
            ServiceRequest::Health => ServiceResponse::Health {
                ok: true,
                service_socket: self.socket_path.display().to_string(),
                desktop_env: desktop_env_values_present(),
                browser_env: crate::browser::browser_env_values_present(),
            },
            ServiceRequest::Browser { request } => self.handle_browser_request(request).await,
            ServiceRequest::Phone { request } => self.handle_phone_request(request).await,
            request => {
                let _desktop_lane = self.desktop_lane.lock().await;
                self.handle_desktop_request(request).await
            }
        }
    }

    /// Bound a single desktop backend call to [`desktop_request_deadline`]
    /// while `desktop_lane` is held.
    ///
    /// Cancel-safety: this wraps only the leaf backend future via
    /// `tokio::time::timeout`, never the surrounding request-handling arm.
    /// Every caller in `handle_desktop_request` either (a) has no
    /// synchronous state that needs restoring around the call (`Doctor`,
    /// `ListApps`, `ListWindows`, `FocusedWindow`, `SetupAccessibility`,
    /// `SetupWindowTargeting`), or (b) already has an existing `Err(error)`
    /// arm that performs the required compensation (`GetAppState` and
    /// `Screenshot` restore the capture-hidden overlay cursor on error). By
    /// racing only the inner future we always get a `Result` back
    /// synchronously and that existing error handling runs unconditionally
    /// — nothing about the caller's own future is ever dropped or
    /// abandoned mid-flight, so there is no risk of leaving, e.g., the
    /// overlay cursor stuck hidden. On elapse the awaited backend future
    /// itself IS dropped (that's the whole point — it frees the lane), but
    /// every desktop backend call reachable from a read-only request is a
    /// pure read with no cross-await mutation of persisted state, so
    /// abandoning it mid-poll cannot corrupt anything (see the plan 017
    /// cancel-safety audit). Mutating requests (`ExecuteAction`,
    /// `LaunchApplication`, `SessionPresence::Ensure/Release`,
    /// `ActivateWindow`, `ResetPortalTokens`) intentionally do NOT go
    /// through this helper.
    async fn with_desktop_deadline<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, BackendError>>,
    ) -> Result<T, BackendError> {
        match tokio::time::timeout(desktop_request_deadline(), future).await {
            Ok(result) => result,
            Err(_) => {
                // Best-effort session reset, itself bounded (see
                // `DESKTOP_DEADLINE_RESET_TIMEOUT`) so a wedged close call
                // cannot re-extend the lane hold this is meant to end.
                let _ = tokio::time::timeout(
                    DESKTOP_DEADLINE_RESET_TIMEOUT,
                    self.backend.reset_desktop_session_state(),
                )
                .await;
                Err(BackendError::new(
                    BackendErrorCode::DesktopRequestDeadlineExceeded,
                    format!(
                        "desktop request exceeded the {:?} deadline and was abandoned; backend session state was reset so the next request starts clean",
                        desktop_request_deadline()
                    ),
                ))
            }
        }
    }

    pub async fn hide_agent_cursor_after_last_client(&self) {
        let mut overlay = self.overlay.lock().await;
        let _ = overlay.hide(Some("last IPC client disconnected".to_string()));
    }

    pub async fn idle_for(&self) -> std::time::Duration {
        self.sessions.idle_for().await
    }
}

fn desktop_env_values_present() -> BTreeMap<String, String> {
    DESKTOP_LAUNCH_ENV_KEYS
        .iter()
        .copied()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (key.to_string(), value))
        })
        .collect()
}
