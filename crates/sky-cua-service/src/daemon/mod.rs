use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sky_cua_platform::DESKTOP_LAUNCH_ENV_KEYS;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    ActionName, ActionRequest, AppShotCapture, AppStateSnapshot, BROWSER_CONTROL_CAPABILITY_V1,
    BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT, BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserRequest,
    BrowserResponse, BrowserSessionIdentity, CUA_SERVICE_DEFAULT_MOUSE_SIZE_PX, CaptureInfo,
    CaptureScreenMode, CoordinateSpace, CuaActionRequest, CuaBackendResponse, CuaCancelStatus,
    CuaCancellation, CuaRequestContext, CuaScreenshot, DiagnosticEntry, DisplayTarget,
    EnvironmentInfo, InputBackendKind, PhoneBackendKind, PhoneRequest, PhoneRequestContext, RectF,
    ServiceRequest, ServiceResponse, SessionPresenceAction, SessionPresenceIntent, WindowInfo,
    WindowTarget, browser_control_mode_capability, browser_eval_enabled,
};
use tracing::debug;

use crate::action_router::route_action;
use crate::approval_store::ApprovalStore;
use crate::backend_factory::create_backend;
use crate::diagnostics::error_response;
use crate::element_resolver::{resolve_action_element, resolve_target_element};
use crate::overlay::{AgentCursorStatus, OverlayController};
use crate::phone::host_scrcpy_default_max_size;
use crate::session_store::SessionStore;
use crate::snapshot_manager::SnapshotManager;

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

/// A capability refresh may include the complete guarded AT-SPI repair path:
/// initial connect (2s), unit inspection (2s), restart (10s), and one retry
/// (2s). Keep modest headroom around those nested deadlines without allowing a
/// future backend regression to wedge the sole refresher forever.
#[cfg(not(test))]
const HEALTH_CAPABILITY_REFRESH_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(test)]
const HEALTH_CAPABILITY_REFRESH_TIMEOUT: Duration = Duration::from_millis(50);

const HEALTH_CAPABILITY_HEALTHY_REFRESH: Duration = Duration::from_secs(30);
const HEALTH_CAPABILITY_DEGRADED_REFRESH_MIN: Duration = Duration::from_secs(30);
const HEALTH_CAPABILITY_DEGRADED_REFRESH_MAX: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
struct HealthCapabilitySnapshot {
    input_backend: InputBackendKind,
}

impl Default for HealthCapabilitySnapshot {
    fn default() -> Self {
        Self {
            input_backend: InputBackendKind::None,
        }
    }
}

fn health_capability_refresh_delay(consecutive_degraded: u32) -> Duration {
    if consecutive_degraded == 0 {
        return HEALTH_CAPABILITY_HEALTHY_REFRESH;
    }
    let exponent = consecutive_degraded.saturating_sub(1).min(4);
    let seconds = HEALTH_CAPABILITY_DEGRADED_REFRESH_MIN
        .as_secs()
        .saturating_mul(1_u64 << exponent)
        .min(HEALTH_CAPABILITY_DEGRADED_REFRESH_MAX.as_secs());
    Duration::from_secs(seconds)
}

pub struct ServiceDaemon {
    backend: Box<dyn DesktopBackend>,
    sessions: SessionStore,
    snapshots: tokio::sync::Mutex<SnapshotManager>,
    overlay: tokio::sync::Mutex<OverlayController>,
    phone: tokio::sync::Mutex<crate::phone::PhoneManager>,
    phone_direct: tokio::sync::Mutex<Option<crate::phone::DirectRuntime>>,
    last_phone_request_context: std::sync::Mutex<Option<PhoneRequestContext>>,
    session_presence_config: SessionPresenceConfig,
    session_presence_held: tokio::sync::Mutex<bool>,
    desktop_lane: tokio::sync::Mutex<()>,
    browser_eval_enabled: bool,
    socket_path: PathBuf,
    cua_cancellations: std::sync::Mutex<HashMap<(String, String), CuaCancellation>>,
    cua_screenshot_planes: std::sync::Mutex<HashMap<String, CuaScreenshotCoordinatePlane>>,
    browser_control_mode: Result<crate::browser::BrowserControlMode, DiagnosticEntry>,
    browser_control_runtime: Option<std::sync::Arc<crate::browser::BrowserControlRuntime>>,
    browser_control_startup_diagnostics: std::sync::Mutex<Vec<DiagnosticEntry>>,
    health_capability_snapshot: std::sync::RwLock<HealthCapabilitySnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
struct CuaScreenshotCoordinatePlane {
    desktop_rect: RectF,
    width: u32,
    height: u32,
}

impl CuaScreenshotCoordinatePlane {
    fn to_desktop(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.desktop_rect.x + (x * self.desktop_rect.width / f64::from(self.width)),
            self.desktop_rect.y + (y * self.desktop_rect.height / f64::from(self.height)),
        )
    }
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

fn desktop_capture_focus_window_ids(capture: &AppShotCapture) -> Vec<String> {
    let AppShotCapture::Desktop {
        window_id,
        semantic_projection,
        ..
    } = capture
    else {
        return Vec::new();
    };
    let mut ids = vec![window_id.clone()];
    for key in ["window_handle", "window_id"] {
        if let Some(alias) = semantic_projection
            .pointer(&format!("/focused_app/{key}"))
            .and_then(serde_json::Value::as_str)
            .filter(|alias| !alias.is_empty())
            && !ids.iter().any(|known| known == alias)
        {
            ids.push(alias.to_string());
        }
    }
    ids
}

mod agent_cursor;
mod appshot;
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
        let browser_control_mode = crate::browser::browser_control_mode();
        let browser_control_runtime = browser_control_mode
            .as_ref()
            .ok()
            .copied()
            .filter(|mode| mode.uses_persistent_actor())
            .map(crate::browser::BrowserControlRuntime::new_with_mode);
        let phone_selection = sky_cua_platform::config::resolved_phone_selection()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        let phone_direct = crate::phone::DirectRuntime::start(&phone_selection).await?;
        let mut phone = crate::phone::PhoneManager::new();
        if let Some(runtime) = phone_direct.as_ref() {
            phone.set_direct_runtime(Some(runtime.handle()));
        }
        Ok(Self {
            backend,
            sessions: SessionStore::new(),
            snapshots: tokio::sync::Mutex::new(SnapshotManager::new(8)),
            overlay: tokio::sync::Mutex::new(OverlayController::new(&socket_path)),
            phone: tokio::sync::Mutex::new(phone),
            phone_direct: tokio::sync::Mutex::new(phone_direct),
            last_phone_request_context: std::sync::Mutex::new(None),
            session_presence_config: SessionPresenceConfig::from_env(),
            session_presence_held: tokio::sync::Mutex::new(false),
            desktop_lane: tokio::sync::Mutex::new(()),
            browser_eval_enabled: browser_eval_enabled(),
            socket_path,
            cua_cancellations: std::sync::Mutex::new(HashMap::new()),
            cua_screenshot_planes: std::sync::Mutex::new(HashMap::new()),
            browser_control_mode,
            browser_control_runtime,
            browser_control_startup_diagnostics: std::sync::Mutex::new(Vec::new()),
            health_capability_snapshot: std::sync::RwLock::new(HealthCapabilitySnapshot::default()),
        })
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn effective_browser_control_mode(
        &self,
    ) -> Result<crate::browser::BrowserControlMode, &DiagnosticEntry> {
        self.browser_control_mode.as_ref().copied()
    }

    #[cfg(unix)]
    pub(crate) async fn shutdown_browser_control(&self) {
        if let Some(runtime) = &self.browser_control_runtime {
            runtime.shutdown().await;
        }
    }

    pub(crate) async fn shutdown_phone_direct(&self) {
        if let Some(runtime) = self.phone_direct.lock().await.take() {
            runtime.shutdown().await;
        }
    }

    /// Returns whether the explicitly configured Direct listener successfully
    /// started with this daemon. This is runtime-owned state: idle lifecycle
    /// decisions must not re-read mutable process configuration after startup.
    pub(crate) async fn phone_direct_listener_active(&self) -> bool {
        self.phone_direct.lock().await.is_some()
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn record_browser_control_startup_diagnostic(&self, diagnostic: DiagnosticEntry) {
        let mut diagnostics = self
            .browser_control_startup_diagnostics
            .lock()
            .expect("browser control startup diagnostics poisoned");
        if !diagnostics
            .iter()
            .any(|entry| entry.code == diagnostic.code)
        {
            diagnostics.push(diagnostic);
        }
    }

    fn append_browser_control_startup_diagnostics(
        &self,
        report: &mut sky_cua_platform::model::BrowserStatusReport,
    ) {
        report.diagnostics.extend(
            self.browser_control_startup_diagnostics
                .lock()
                .expect("browser control startup diagnostics poisoned")
                .iter()
                .cloned(),
        );
    }

    #[cfg(unix)]
    pub(crate) fn codex_browser_backend(
        &self,
    ) -> std::sync::Arc<dyn crate::codex_browser_compat::CodexBrowserBackend> {
        if self
            .browser_control_mode
            .as_ref()
            .is_ok_and(|mode| mode.uses_persistent_actor())
            && let Some(runtime) = &self.browser_control_runtime
        {
            return runtime.clone();
        }
        std::sync::Arc::new(crate::codex_browser_compat::UnavailableCodexBrowserBackend::new())
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

    /// Refresh the dynamic desktop capability snapshot independently of Health
    /// callers. Health is a launcher liveness RPC with a 250ms client budget;
    /// it must never inherit portal or accessibility-bus latency. One daemon-
    /// owned task runs this loop sequentially, so a retry storm cannot fan out
    /// cold backend probes and caller cancellation cannot interrupt repair.
    pub fn spawn_health_capability_refresher(
        self: &std::sync::Arc<Self>,
    ) -> tokio::task::JoinHandle<()> {
        let daemon = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut consecutive_degraded = 0_u32;
            loop {
                let healthy = daemon.refresh_health_capability_snapshot().await;
                consecutive_degraded = if healthy {
                    0
                } else {
                    consecutive_degraded.saturating_add(1)
                };
                tokio::time::sleep(health_capability_refresh_delay(consecutive_degraded)).await;
            }
        })
    }

    async fn refresh_health_capability_snapshot(&self) -> bool {
        let refreshed = tokio::time::timeout(
            HEALTH_CAPABILITY_REFRESH_TIMEOUT,
            self.backend.probe_environment(),
        )
        .await;
        let (input_backend, healthy) = match refreshed {
            Ok(Ok(environment)) => (
                environment.input_backend,
                environment.semantic_backend != sky_cua_platform::model::SemanticBackendKind::None,
            ),
            Ok(Err(error)) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "health capability refresh failed"
                );
                (InputBackendKind::None, false)
            }
            Err(_) => {
                debug!(
                    timeout_ms = HEALTH_CAPABILITY_REFRESH_TIMEOUT.as_millis(),
                    "health capability refresh exceeded its deadline"
                );
                (InputBackendKind::None, false)
            }
        };
        let mut snapshot = self
            .health_capability_snapshot
            .write()
            .expect("health capability snapshot poisoned");
        snapshot.input_backend = input_backend;
        healthy
    }

    fn health_input_backend(&self) -> InputBackendKind {
        self.health_capability_snapshot
            .read()
            .expect("health capability snapshot poisoned")
            .input_backend
            .clone()
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
            phone_direct: tokio::sync::Mutex::new(None),
            last_phone_request_context: std::sync::Mutex::new(None),
            session_presence_config: SessionPresenceConfig::disabled(),
            session_presence_held: tokio::sync::Mutex::new(false),
            desktop_lane: tokio::sync::Mutex::new(()),
            browser_eval_enabled: false,
            socket_path: PathBuf::from("/tmp/sky-cua-test.sock"),
            cua_cancellations: std::sync::Mutex::new(HashMap::new()),
            cua_screenshot_planes: std::sync::Mutex::new(HashMap::new()),
            browser_control_mode: Ok(crate::browser::BrowserControlMode::Legacy),
            browser_control_runtime: None,
            browser_control_startup_diagnostics: std::sync::Mutex::new(Vec::new()),
            health_capability_snapshot: std::sync::RwLock::new(HealthCapabilitySnapshot::default()),
        })
    }

    pub async fn handle(&self, request: ServiceRequest) -> ServiceResponse {
        self.sessions.touch().await;
        self.ensure_session_presence_for_request(&request).await;
        match request {
            ServiceRequest::Health => {
                let mut capabilities =
                    sky_cua_platform::model::cua_service_capabilities_for_input_backend(
                        &self.health_input_backend(),
                    );
                if let Ok(mode) = self.browser_control_mode.as_ref().copied() {
                    capabilities.push(BROWSER_CONTROL_CAPABILITY_V1.to_owned());
                    capabilities.push(browser_control_mode_capability(match mode {
                        crate::browser::BrowserControlMode::Legacy => {
                            sky_cua_platform::config::BrowserControlMode::Legacy
                        }
                        crate::browser::BrowserControlMode::Hybrid => {
                            sky_cua_platform::config::BrowserControlMode::Hybrid
                        }
                        crate::browser::BrowserControlMode::Strict => {
                            sky_cua_platform::config::BrowserControlMode::Strict
                        }
                    }));
                }
                ServiceResponse::Health {
                    ok: true,
                    service_socket: self.socket_path.display().to_string(),
                    protocol_version: sky_cua_platform::model::CUA_SERVICE_PROTOCOL_VERSION,
                    service_version: sky_cua_platform::model::CUA_SERVICE_VERSION.to_string(),
                    capabilities,
                    desktop_env: desktop_env_values_present(),
                    browser_env: crate::browser::browser_env_values_present(),
                }
            }
            ServiceRequest::Click {
                context,
                x,
                y,
                mouse_button,
                click_count,
                key,
                post_action_sleep_ms,
            } => {
                self.handle_cua_action(CuaActionRequest::Click {
                    context,
                    x,
                    y,
                    mouse_button,
                    click_count,
                    key,
                    post_action_sleep_ms,
                })
                .await
            }
            ServiceRequest::Drag {
                context,
                from_x,
                from_y,
                to_x,
                to_y,
                key,
                post_action_sleep_ms,
            } => {
                self.handle_cua_action(CuaActionRequest::Drag {
                    context,
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    key,
                    post_action_sleep_ms,
                })
                .await
            }
            ServiceRequest::GetScreenshot {
                context,
                mouse_size_px,
            } => self.handle_cua_screenshot(context, mouse_size_px).await,
            ServiceRequest::Move {
                context,
                x,
                y,
                key,
                post_action_sleep_ms,
            } => {
                self.handle_cua_action(CuaActionRequest::Move {
                    context,
                    x,
                    y,
                    key,
                    post_action_sleep_ms,
                })
                .await
            }
            ServiceRequest::PressKey {
                context,
                key,
                post_action_sleep_ms,
            } => {
                self.handle_cua_action(CuaActionRequest::PressKey {
                    context,
                    key,
                    post_action_sleep_ms,
                })
                .await
            }
            ServiceRequest::Scroll {
                context,
                direction,
                pixels,
                x,
                y,
                key,
                post_action_sleep_ms,
            } => {
                self.handle_cua_action(CuaActionRequest::Scroll {
                    context,
                    direction,
                    pixels,
                    x,
                    y,
                    key,
                    post_action_sleep_ms,
                })
                .await
            }
            ServiceRequest::TypeText {
                context,
                text,
                post_action_sleep_ms,
            } => {
                self.handle_cua_action(CuaActionRequest::TypeText {
                    context,
                    text,
                    post_action_sleep_ms,
                })
                .await
            }
            ServiceRequest::CancelTurn {
                session_id,
                turn_id,
                reason,
            } => self.handle_cua_cancel(session_id, turn_id, reason),
            ServiceRequest::Browser {
                request,
                identity,
                context,
            } => {
                self.handle_browser_request(request, identity, context)
                    .await
            }
            ServiceRequest::CancelBrowserOperation {
                connection_id,
                operation_id,
                reason: _,
            } => {
                #[cfg(not(unix))]
                {
                    let _ = (connection_id, operation_id);
                    error_response(
                        "BrowserControlUnsupported",
                        "browser cancellation is unavailable without Unix browser control",
                    )
                }
                #[cfg(unix)]
                {
                    let Some(runtime) = &self.browser_control_runtime else {
                        return error_response(
                            "BrowserControlUnavailable",
                            "browser cancellation requires hybrid/strict control runtime",
                        );
                    };
                    match runtime
                        .cancel_mcp_operation(&connection_id, &operation_id)
                        .await
                    {
                        Ok(status) => ServiceResponse::Error {
                            ok: true,
                            code: "BrowserCancellationAcknowledged".to_owned(),
                            message: format!("{status:?}"),
                            session_id: None,
                            turn_id: None,
                            retry: None,
                        },
                        Err(diagnostic) => error_response(&diagnostic.code, &diagnostic.message),
                    }
                }
            }
            ServiceRequest::BrowserClientDisconnected { connection_id } => {
                #[cfg(unix)]
                if let Some(runtime) = &self.browser_control_runtime {
                    runtime.mcp_client_disconnected(&connection_id).await;
                }
                #[cfg(not(unix))]
                let _ = connection_id;
                ServiceResponse::Error {
                    ok: true,
                    code: "BrowserClientDisconnectAcknowledged".to_owned(),
                    message: "browser client disconnect processed".to_owned(),
                    session_id: None,
                    turn_id: None,
                    retry: None,
                }
            }
            ServiceRequest::Phone { request, context } => {
                self.handle_phone_request(request, context).await
            }
            ServiceRequest::PhoneDirectCreateEnrollment => {
                let runtime = self.phone_direct.lock().await;
                match runtime.as_ref() {
                    Some(runtime) => ServiceResponse::PhoneDirectEnrollment {
                        payload: Box::new(runtime.create_enrollment()),
                    },
                    None => error_response(
                        "PhoneDirectUnavailable",
                        "Companion Direct enrollment requires explicitly enabled phone-control.v2 configuration",
                    ),
                }
            }
            request => {
                let _desktop_lane = self.desktop_lane.lock().await;
                self.handle_desktop_request(request).await
            }
        }
    }

    async fn handle_cua_action(&self, action: CuaActionRequest) -> ServiceResponse {
        let action = self.action_in_desktop_plane(action);
        let context = action.context().clone();
        if let Err(message) = context.validate() {
            return cua_error_response(
                "SKY_CUA_INVALID_CONTEXT",
                message,
                Some(&context),
                Some("never"),
            );
        }
        if let Err(message) = validate_cua_action(&action) {
            return cua_error_response(
                "SKY_CUA_INVALID_ARGUMENT",
                message,
                Some(&context),
                Some("never"),
            );
        }
        if let Some(response) = self.validate_cua_context_appshot(&context).await {
            return response;
        }
        let deadline_at =
            tokio::time::Instant::now() + Duration::from_millis(u64::from(context.deadline_ms()));

        let turn_key = context.turn_key();
        let cancellation = {
            let mut cancellations = self
                .cua_cancellations
                .lock()
                .expect("CUA cancellation registry should not be poisoned");
            if cancellations.contains_key(&turn_key) {
                return cua_error_response(
                    "SKY_CUA_DUPLICATE_ACTIVE_TURN",
                    "an action with this session_id and turn_id is already active",
                    Some(&context),
                    Some("never"),
                );
            }
            let cancellation = CuaCancellation::new();
            cancellations.insert(turn_key.clone(), cancellation.clone());
            cancellation
        };
        debug!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            deadline_ms = context.deadline_ms(),
            "cua action started"
        );

        let desktop_lane = tokio::select! {
            lane = self.desktop_lane.lock() => lane,
            () = tokio::time::sleep_until(deadline_at) => {
                cancellation.cancel();
                self.cua_cancellations
                    .lock()
                    .expect("CUA cancellation registry should not be poisoned")
                    .remove(&turn_key);
                return cua_error_response(
                    "SKY_CUA_DEADLINE_EXCEEDED",
                    "the CUA action deadline elapsed while waiting for the desktop lane",
                    Some(&context),
                    Some("never"),
                );
            }
        };
        let backend_future = self
            .backend
            .execute_cua_action(action.clone(), cancellation.clone());
        tokio::pin!(backend_future);
        let deadline = tokio::time::sleep_until(deadline_at);
        tokio::pin!(deadline);
        let (result, deadline_elapsed) = tokio::select! {
            result = &mut backend_future => (result, false),
            () = &mut deadline => {
                cancellation.cancel();
                debug!(
                    session_id = %context.session_id,
                    turn_id = %context.turn_id,
                    "cua action deadline elapsed; waiting for backend cleanup"
                );
                (backend_future.await, true)
            }
        };
        drop(desktop_lane);
        let response = match result {
            Err(error) => {
                if error.code == "CuaActionOutcomeUnknown" {
                    cua_error_response(
                        "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
                        error.message,
                        Some(&context),
                        Some("never"),
                    )
                } else if deadline_elapsed {
                    cua_error_response(
                        "SKY_CUA_DEADLINE_EXCEEDED",
                        "the CUA action deadline elapsed; backend cleanup completed",
                        Some(&context),
                        Some("never"),
                    )
                } else if cancellation.is_cancelled() {
                    cua_error_response(
                        "SKY_CUA_TURN_CANCELLED",
                        "the CUA turn was cancelled before the action completed",
                        Some(&context),
                        Some("never"),
                    )
                } else {
                    cua_error_response(
                        cua_error_code(&error),
                        error.message,
                        Some(&context),
                        Some("never"),
                    )
                }
            }
            Ok(CuaBackendResponse::Screenshots(screenshots)) => ServiceResponse::GetScreenshot {
                ok: true,
                screenshots,
            },
            Ok(CuaBackendResponse::Action)
                if (deadline_elapsed || cancellation.is_cancelled())
                    && !cua_action_is_idempotent(&action) =>
            {
                debug!(
                    session_id = %context.session_id,
                    turn_id = %context.turn_id,
                    "cua action cancellation arrived after backend completion"
                );
                cua_error_response(
                    "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
                    "the action may have completed before cancellation was observed",
                    Some(&context),
                    Some("never"),
                )
            }
            Ok(CuaBackendResponse::Action) if deadline_elapsed => cua_error_response(
                "SKY_CUA_DEADLINE_EXCEEDED",
                "the CUA action deadline elapsed; backend cleanup completed",
                Some(&context),
                Some("never"),
            ),
            Ok(CuaBackendResponse::Action) => cua_action_response(&action),
        };
        self.cua_cancellations
            .lock()
            .expect("CUA cancellation registry should not be poisoned")
            .remove(&turn_key);
        debug!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            cancelled = cancellation.is_cancelled(),
            "cua action finished"
        );
        response
    }

    async fn validate_cua_context_appshot(
        &self,
        context: &CuaRequestContext,
    ) -> Option<ServiceResponse> {
        let (reason, target, focus_window_ids) = {
            let snapshots = self.snapshots.lock().await;
            match context
                .appshot_id
                .as_deref()
                .and_then(|id| snapshots.appshot(id))
            {
                None if context.appshot_id.is_none() => (
                    sky_cua_platform::model::AppShotRejectionReason::Missing,
                    None,
                    Vec::new(),
                ),
                None => (
                    sky_cua_platform::model::AppShotRejectionReason::Stale,
                    None,
                    Vec::new(),
                ),
                Some(appshot) => {
                    let focus_window_ids = desktop_capture_focus_window_ids(&appshot.capture);
                    let target = match &appshot.capture {
                        sky_cua_platform::model::AppShotCapture::Desktop { window_id, .. } => {
                            Some(WindowTarget {
                                window_id: Some(window_id.clone()),
                                ..Default::default()
                            })
                        }
                        _ => None,
                    };
                    let session_ok = appshot
                        .action_snapshot
                        .session_id
                        .as_deref()
                        .is_none_or(|session| session == context.session_id);
                    if snapshots.is_latest(&appshot.action_snapshot.snapshot_id) && session_ok {
                        (
                            sky_cua_platform::model::AppShotRejectionReason::WrongTarget,
                            target,
                            focus_window_ids,
                        )
                    } else if !session_ok {
                        (
                            sky_cua_platform::model::AppShotRejectionReason::WrongSession,
                            target,
                            focus_window_ids,
                        )
                    } else {
                        (
                            sky_cua_platform::model::AppShotRejectionReason::Stale,
                            target,
                            focus_window_ids,
                        )
                    }
                }
            }
        };
        if reason == sky_cua_platform::model::AppShotRejectionReason::WrongTarget
            && let Ok(Some(focused)) = self.backend.focused_window().await
            && focus_window_ids
                .iter()
                .any(|window_id| window_id == &focused.window_id)
        {
            return None;
        }
        let frontmost = target.is_none();
        let request_id = format!("recovery-{}", sky_cua_platform::snapshot::new_snapshot_id());
        let response = self
            .handle_appshot_capture(request_id, target, frontmost, Default::default())
            .await;
        let Some(mut fresh_appshot) = (match response {
            ServiceResponse::AppShotCapture { result } => result.appshot,
            _ => None,
        }) else {
            return Some(error_response(
                "AppShotRequired",
                "desktop CUA action requires a fresh exact-window AppShot",
            ));
        };
        fresh_appshot.trigger = sky_cua_platform::model::AppShotTrigger::Recovery;
        fresh_appshot.action_snapshot.session_id = Some(context.session_id.clone());
        self.snapshots
            .lock()
            .await
            .store_appshot((*fresh_appshot).clone());
        Some(ServiceResponse::AppShotRequired {
            rejection: Box::new(sky_cua_platform::model::AppShotRequired {
                code: "AppShotRequired".to_string(),
                reason,
                message:
                    "desktop CUA actions require a present, fresh AppShot for the exact window"
                        .to_string(),
                fresh_appshot,
            }),
        })
    }

    async fn handle_cua_screenshot(
        &self,
        context: Option<CuaRequestContext>,
        mouse_size_px: Option<u32>,
    ) -> ServiceResponse {
        if let Some(context) = context.as_ref()
            && let Err(message) = context.validate()
        {
            return cua_error_response(
                "SKY_CUA_INVALID_CONTEXT",
                message,
                Some(context),
                Some("never"),
            );
        }
        if mouse_size_px.is_some_and(|size| size > 128) {
            return cua_error_response(
                "SKY_CUA_INVALID_ARGUMENT",
                "mouse_size_px must be between 0 and 128",
                context.as_ref(),
                Some("never"),
            );
        }
        let plane_key = context
            .as_ref()
            .map(|context| context.session_id.clone())
            .unwrap_or_default();
        self.cua_screenshot_planes
            .lock()
            .expect("CUA screenshot planes should not be poisoned")
            .remove(&plane_key);
        let deadline_ms = context
            .as_ref()
            .map(CuaRequestContext::deadline_ms)
            .unwrap_or(sky_cua_platform::model::CUA_SERVICE_MAX_DEADLINE_MS);
        let deadline_at =
            tokio::time::Instant::now() + Duration::from_millis(u64::from(deadline_ms));
        let capture_guard = match tokio::time::timeout_at(deadline_at, self.overlay.lock()).await {
            Ok(mut overlay) => overlay.prepare_for_capture(),
            Err(_) => {
                return cua_error_response(
                    "SKY_CUA_DEADLINE_EXCEEDED",
                    "the screenshot deadline elapsed while waiting for overlay capture state",
                    context.as_ref(),
                    Some("never"),
                );
            }
        };
        let desktop_lane =
            match tokio::time::timeout_at(deadline_at, self.desktop_lane.lock()).await {
                Ok(lane) => lane,
                Err(_) => {
                    let _ = self
                        .overlay
                        .lock()
                        .await
                        .restore_after_capture(capture_guard);
                    return cua_error_response(
                        "SKY_CUA_DEADLINE_EXCEEDED",
                        "the screenshot deadline elapsed while waiting for the desktop lane",
                        context.as_ref(),
                        Some("never"),
                    );
                }
            };
        let result =
            tokio::time::timeout_at(deadline_at, self.backend.screenshot(None, None)).await;
        drop(desktop_lane);
        match result {
            Err(_) => {
                let _ = self
                    .overlay
                    .lock()
                    .await
                    .restore_after_capture(capture_guard);
                cua_error_response(
                    "SKY_CUA_DEADLINE_EXCEEDED",
                    "the screenshot deadline elapsed",
                    context.as_ref(),
                    Some("never"),
                )
            }
            Ok(Err(error)) => {
                let _ = self
                    .overlay
                    .lock()
                    .await
                    .restore_after_capture(capture_guard);
                cua_error_response(
                    cua_error_code(&error),
                    error.message,
                    context.as_ref(),
                    Some("never"),
                )
            }
            Ok(Ok(mut snapshot)) => {
                {
                    let mut overlay = self.overlay.lock().await;
                    overlay.apply_to_snapshot_with_cursor_size(
                        &mut snapshot,
                        Some(mouse_size_px.unwrap_or(CUA_SERVICE_DEFAULT_MOUSE_SIZE_PX)),
                    );
                    snapshot
                        .diagnostics
                        .extend(overlay.restore_after_capture(capture_guard));
                }
                match screenshot_from_snapshot(snapshot) {
                    Ok((screenshot, plane)) => {
                        if let Some(plane) = plane {
                            self.cua_screenshot_planes
                                .lock()
                                .expect("CUA screenshot planes should not be poisoned")
                                .insert(plane_key, plane);
                        }
                        ServiceResponse::GetScreenshot {
                            ok: true,
                            screenshots: vec![screenshot],
                        }
                    }
                    Err(error) => cua_error_response(
                        "SKY_CUA_INTERNAL",
                        error,
                        context.as_ref(),
                        Some("never"),
                    ),
                }
            }
        }
    }

    fn action_in_desktop_plane(&self, mut action: CuaActionRequest) -> CuaActionRequest {
        let session_id = &action.context().session_id;
        let planes = self
            .cua_screenshot_planes
            .lock()
            .expect("CUA screenshot planes should not be poisoned");
        let plane = planes.get(session_id).or_else(|| planes.get("")).cloned();
        drop(planes);
        let Some(plane) = plane else {
            return action;
        };
        let map = |x: &mut f64, y: &mut f64| {
            (*x, *y) = plane.to_desktop(*x, *y);
        };
        match &mut action {
            CuaActionRequest::Click { x, y, .. } | CuaActionRequest::Move { x, y, .. } => {
                map(x, y);
            }
            CuaActionRequest::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                ..
            } => {
                map(from_x, from_y);
                map(to_x, to_y);
            }
            CuaActionRequest::Scroll { x, y, .. } => {
                if let (Some(x), Some(y)) = (x.as_mut(), y.as_mut()) {
                    map(x, y);
                }
            }
            CuaActionRequest::PressKey { .. } | CuaActionRequest::TypeText { .. } => {}
        }
        action
    }

    fn handle_cua_cancel(
        &self,
        session_id: String,
        turn_id: String,
        reason: String,
    ) -> ServiceResponse {
        if session_id.trim().is_empty() || turn_id.trim().is_empty() {
            return cua_error_response_parts(
                "SKY_CUA_CANCEL_TURN_INVALID_CONTEXT",
                "session_id and turn_id must be non-empty",
                Some(session_id),
                Some(turn_id),
                Some("never"),
            );
        }
        if reason.trim().is_empty() || reason.chars().count() > 256 {
            return cua_error_response_parts(
                "SKY_CUA_CANCEL_TURN_INVALID_REASON",
                "reason must be between 1 and 256 characters",
                Some(session_id),
                Some(turn_id),
                Some("never"),
            );
        }
        let key = (session_id.clone(), turn_id.clone());
        let status = self
            .cua_cancellations
            .lock()
            .expect("CUA cancellation registry should not be poisoned")
            .get(&key)
            .map(|cancellation| {
                if cancellation.is_cancelled() {
                    CuaCancelStatus::AlreadyCancelled
                } else {
                    cancellation.cancel();
                    CuaCancelStatus::CancelRequested
                }
            })
            .unwrap_or(CuaCancelStatus::NotFound);
        debug!(
            session_id = %session_id,
            turn_id = %turn_id,
            ?status,
            "cua turn cancellation requested"
        );
        ServiceResponse::CancelTurn {
            ok: true,
            session_id,
            turn_id,
            status,
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
        self.with_desktop_deadline_until(
            tokio::time::Instant::now() + desktop_request_deadline(),
            future,
        )
        .await
    }

    async fn with_desktop_deadline_until<T>(
        &self,
        deadline: tokio::time::Instant,
        future: impl std::future::Future<Output = Result<T, BackendError>>,
    ) -> Result<T, BackendError> {
        match tokio::time::timeout_at(deadline, future).await {
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

fn validate_cua_action(action: &CuaActionRequest) -> Result<(), &'static str> {
    let sleep_ms = action.post_action_sleep_ms();
    if sleep_ms.is_some_and(|value| value > 30_000) {
        return Err("post_action_sleep_ms must be between 0 and 30000");
    }
    let finite = |value: f64| value.is_finite();
    match action {
        CuaActionRequest::Click {
            x,
            y,
            click_count,
            key,
            ..
        } => {
            if !finite(*x) || !finite(*y) {
                return Err("coordinates must be finite numbers");
            }
            if click_count.is_some_and(|count| !(1..=100).contains(&count)) {
                return Err("click_count must be between 1 and 100");
            }
            if key.as_deref().is_some_and(|key| key.trim().is_empty()) {
                return Err("key must be non-empty when provided");
            }
        }
        CuaActionRequest::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            key,
            ..
        } => {
            if !finite(*from_x) || !finite(*from_y) || !finite(*to_x) || !finite(*to_y) {
                return Err("coordinates must be finite numbers");
            }
            if key.as_deref().is_some_and(|key| key.trim().is_empty()) {
                return Err("key must be non-empty when provided");
            }
        }
        CuaActionRequest::Move { x, y, key, .. } => {
            if !finite(*x) || !finite(*y) {
                return Err("coordinates must be finite numbers");
            }
            if key.as_deref().is_some_and(|key| key.trim().is_empty()) {
                return Err("key must be non-empty when provided");
            }
        }
        CuaActionRequest::PressKey { key, .. } => {
            if key.trim().is_empty() {
                return Err("key must be non-empty");
            }
        }
        CuaActionRequest::Scroll {
            pixels, x, y, key, ..
        } => {
            if pixels.is_some_and(|pixels| !(1..=10_000).contains(&pixels)) {
                return Err("pixels must be between 1 and 10000");
            }
            if x.is_some_and(|value| !finite(value)) || y.is_some_and(|value| !finite(value)) {
                return Err("coordinates must be finite numbers");
            }
            if x.is_some() != y.is_some() {
                return Err("scroll origin requires both x and y");
            }
            if key.as_deref().is_some_and(|key| key.trim().is_empty()) {
                return Err("key must be non-empty when provided");
            }
        }
        CuaActionRequest::TypeText { .. } => {}
    }
    Ok(())
}

fn cua_action_is_idempotent(action: &CuaActionRequest) -> bool {
    matches!(action, CuaActionRequest::Move { .. })
}

fn cua_action_response(action: &CuaActionRequest) -> ServiceResponse {
    let context = action.context();
    let session_id = context.session_id.clone();
    let turn_id = context.turn_id.clone();
    match action {
        CuaActionRequest::Click { .. } => ServiceResponse::Click {
            ok: true,
            session_id,
            turn_id,
        },
        CuaActionRequest::Drag { .. } => ServiceResponse::Drag {
            ok: true,
            session_id,
            turn_id,
        },
        CuaActionRequest::Move { .. } => ServiceResponse::Move {
            ok: true,
            session_id,
            turn_id,
        },
        CuaActionRequest::PressKey { .. } => ServiceResponse::PressKey {
            ok: true,
            session_id,
            turn_id,
        },
        CuaActionRequest::Scroll { .. } => ServiceResponse::Scroll {
            ok: true,
            session_id,
            turn_id,
        },
        CuaActionRequest::TypeText { .. } => ServiceResponse::TypeText {
            ok: true,
            session_id,
            turn_id,
        },
    }
}

fn cua_error_code(error: &BackendError) -> &'static str {
    match error.code {
        "InvalidRequest" => "SKY_CUA_INVALID_ARGUMENT",
        "ActionUnsupportedForEnvironment" | "UnsupportedEnvironment" => {
            "SKY_CUA_TARGET_UNAVAILABLE"
        }
        "ServiceUnavailable" => "SKY_CUA_SERVICE_RESTART_REQUIRED",
        "CuaActionOutcomeUnknown" => "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
        _ => "SKY_CUA_INTERNAL",
    }
}

fn cua_error_response(
    code: &'static str,
    message: impl Into<String>,
    context: Option<&CuaRequestContext>,
    retry: Option<&'static str>,
) -> ServiceResponse {
    cua_error_response_parts(
        code,
        message,
        context.map(|context| context.session_id.clone()),
        context.map(|context| context.turn_id.clone()),
        retry,
    )
}

fn cua_error_response_parts(
    code: &'static str,
    message: impl Into<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    retry: Option<&'static str>,
) -> ServiceResponse {
    let session_id = session_id.filter(|value| !value.trim().is_empty());
    let turn_id = turn_id.filter(|value| !value.trim().is_empty());
    ServiceResponse::Error {
        ok: false,
        code: code.to_string(),
        message: message.into(),
        session_id,
        turn_id,
        retry: retry.map(str::to_string),
    }
}

fn screenshot_from_snapshot(
    snapshot: AppStateSnapshot,
) -> Result<(CuaScreenshot, Option<CuaScreenshotCoordinatePlane>), String> {
    use base64::Engine;
    let desktop_rect = screenshot_desktop_rect(&snapshot);
    let capture = snapshot
        .capture
        .ok_or_else(|| "screenshot capture did not produce image metadata".to_string())?;
    let source_path = capture
        .screenshot_path
        .ok_or_else(|| "screenshot capture did not produce a file path".to_string())?;
    let source_bytes = std::fs::read(&source_path)
        .map_err(|error| format!("failed to read screenshot capture: {error}"))?;
    let (output_path, webp_bytes, width, height) =
        screenshot_bytes_as_webp(std::path::Path::new(&source_path), source_bytes)?;
    let bytes_base64 = base64::engine::general_purpose::STANDARD.encode(&webp_bytes);
    let screenshot = CuaScreenshot {
        filepath: output_path.display().to_string(),
        bytes_base64,
        mime_type: "image/webp".to_string(),
        width,
        height,
    };
    let plane = desktop_rect.and_then(|desktop_rect| {
        (width > 0 && height > 0).then_some(CuaScreenshotCoordinatePlane {
            desktop_rect,
            width,
            height,
        })
    });
    Ok((screenshot, plane))
}

fn screenshot_bytes_as_webp(
    source_path: &std::path::Path,
    source_bytes: Vec<u8>,
) -> Result<(std::path::PathBuf, Vec<u8>, u32, u32), String> {
    let source_format = image::guess_format(&source_bytes)
        .map_err(|error| format!("failed to identify screenshot capture: {error}"))?;
    let output_path = source_path.with_extension("webp");

    if source_format == image::ImageFormat::WebP {
        let (width, height) = image::ImageReader::with_format(
            std::io::Cursor::new(&source_bytes),
            image::ImageFormat::WebP,
        )
        .into_dimensions()
        .map_err(|error| format!("failed to read WebP screenshot dimensions: {error}"))?;
        if output_path != source_path {
            std::fs::write(&output_path, &source_bytes)
                .map_err(|error| format!("failed to persist WebP screenshot: {error}"))?;
        }
        return Ok((output_path, source_bytes, width, height));
    }

    let image = image::load_from_memory_with_format(&source_bytes, source_format)
        .map_err(|error| format!("failed to decode screenshot capture: {error}"))?;
    let width = image.width();
    let height = image.height();
    let rgb = image.to_rgb8();
    let webp = webp::Encoder::from_rgb(rgb.as_raw(), width, height).encode(85.0);
    let webp_bytes = webp.to_vec();
    std::fs::write(&output_path, &webp_bytes)
        .map_err(|error| format!("failed to persist WebP screenshot: {error}"))?;
    Ok((output_path, webp_bytes, width, height))
}

fn screenshot_desktop_rect(snapshot: &AppStateSnapshot) -> Option<RectF> {
    snapshot
        .capture
        .as_ref()
        .and_then(|capture| {
            capture
                .logical_rect
                .as_ref()
                .or(capture.source_logical_rect.as_ref())
        })
        .filter(|rect| rect.space == CoordinateSpace::DesktopLogical)
        .cloned()
        .or_else(|| virtual_desktop_rect(&snapshot.environment))
}

fn virtual_desktop_rect(environment: &EnvironmentInfo) -> Option<RectF> {
    let first = environment.displays.first()?;
    let (mut left, mut top, mut right, mut bottom) = (
        first.logical_rect.x,
        first.logical_rect.y,
        first.logical_rect.right(),
        first.logical_rect.bottom(),
    );
    for display in &environment.displays[1..] {
        left = left.min(display.logical_rect.x);
        top = top.min(display.logical_rect.y);
        right = right.max(display.logical_rect.right());
        bottom = bottom.max(display.logical_rect.bottom());
    }
    Some(RectF {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
        space: CoordinateSpace::DesktopLogical,
    })
}

#[cfg(test)]
mod screenshot_conversion_tests {
    use super::screenshot_bytes_as_webp;

    fn temporary_path(extension: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sky-cua-screenshot-{}-{unique}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    fn existing_webp_is_forwarded_byte_for_byte_without_rewriting() {
        let path = temporary_path("webp");
        let pixels = [255_u8, 0, 0, 0, 255, 0];
        let original = webp::Encoder::from_rgb(&pixels, 2, 1).encode(85.0).to_vec();
        std::fs::write(&path, &original).expect("test WebP should be written");

        let (output_path, output, width, height) =
            screenshot_bytes_as_webp(&path, original.clone()).expect("WebP should pass through");

        assert_eq!(output_path, path);
        assert_eq!(output, original);
        assert_eq!((width, height), (2, 1));
        std::fs::remove_file(&path).expect("test WebP should be removed");
    }
}

#[cfg(test)]
mod desktop_capture_focus_tests {
    use super::{AppShotCapture, CoordinateSpace, RectF, desktop_capture_focus_window_ids};

    #[test]
    fn retains_native_and_compositor_window_aliases_from_one_appshot() {
        let capture = AppShotCapture::Desktop {
            app_id: "python.desktop".to_string(),
            window_id: "0x3400007".to_string(),
            title: Some("sky-cua pointer smoke".to_string()),
            bounds: RectF {
                x: 0.0,
                y: 0.0,
                width: 2560.0,
                height: 1600.0,
                space: CoordinateSpace::DesktopLogical,
            },
            semantic_projection: serde_json::json!({
                "focused_app": {
                    "window_handle": "kwin:{same-xwayland-window}"
                }
            }),
        };

        assert_eq!(
            desktop_capture_focus_window_ids(&capture),
            vec![
                "0x3400007".to_string(),
                "kwin:{same-xwayland-window}".to_string()
            ]
        );
    }
}
