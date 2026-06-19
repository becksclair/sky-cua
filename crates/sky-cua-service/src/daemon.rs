use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sky_cua_platform::DESKTOP_LAUNCH_ENV_KEYS;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::BackendErrorCode;
use sky_cua_platform::model::{
    ActionName, ActionRequest, AppStateSnapshot, BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
    BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserRequest, BrowserResponse, CaptureInfo,
    CaptureScreenMode, DiagnosticEntry, DisplayTarget, PhoneBackendKind, PhoneRequest,
    ServiceRequest, ServiceResponse, SessionPresenceAction, SessionPresenceIntent, WindowInfo,
    WindowTarget,
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

pub struct ServiceDaemon {
    backend: Box<dyn DesktopBackend>,
    sessions: SessionStore,
    snapshots: tokio::sync::Mutex<SnapshotManager>,
    overlay: tokio::sync::Mutex<OverlayController>,
    phone: tokio::sync::Mutex<crate::phone::PhoneManager>,
    session_presence_config: SessionPresenceConfig,
    session_presence_held: tokio::sync::Mutex<bool>,
    desktop_lane: tokio::sync::Mutex<()>,
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
            socket_path,
        })
    }

    /// Spawn a background task that hides the agent cursor overlay once it
    /// has been idle past the timeout, even when no further requests arrive
    /// (interrupted or abandoned agent turns must not leave the overlay
    /// shown or the user's cursor hidden).
    pub fn spawn_overlay_idle_watchdog(self: &std::sync::Arc<Self>) -> tokio::task::JoinHandle<()> {
        let daemon = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let diagnostics = daemon.overlay.lock().await.hide_idle_cursor();
                for entry in diagnostics {
                    debug!(
                        code = entry.code,
                        message = entry.message,
                        "overlay idle watchdog"
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

    /// Route a phone request through the service-owned [`PhoneManager`].
    ///
    /// The manager owns session state, the per-session capability-profile cache,
    /// and the `CommandRunner` seam every ADB/companion/scrcpy backend goes
    /// through. Phone control never touches the
    /// desktop session, so this path is intentionally outside the serialized
    /// `desktop_lane`; the manager's own `Mutex` serializes mutable phone state.
    /// Phase 1 backends are stubs: status/devices route through the ADB stub and
    /// every device-bound request returns an honest "not implemented" response
    /// without fabricating a session.
    ///
    /// After the manager handles the request, the daemon mediates the two pieces
    /// of cross-subsystem state the manager cannot reach on its own: it discovers
    /// and maps the managed scrcpy desktop window (the manager owns no desktop
    /// backend), and it draws/hides the host-visible cursor overlay (the manager
    /// owns no overlay). The phone-manager lock and the overlay lock are taken
    /// separately and held only across a read/update, never nested, to avoid
    /// deadlock.
    async fn handle_phone_request(&self, request: PhoneRequest) -> ServiceResponse {
        // Derive the phone-scale scrcpy `--max-size` cap for a mirror-bearing
        // connect OUTSIDE the phone lock (the display probe is slow), then apply
        // it under the SAME lock span as handle() so a concurrent connect on
        // another IPC task cannot race the primed value between priming and
        // launch. Applied only when `[phone] max_size` is unset, so the mirror
        // renders phone-sized without overriding an explicit config.
        let scrcpy_size_default = self.scrcpy_host_size_default_for(&request).await;

        // Scan for a pre-existing scrcpy window to adopt (avoid spawning a second
        // mirror) on a scrcpy-bearing connect, OUTSIDE the phone lock (the
        // `list_windows` enumeration is slow), then prime it under the SAME lock
        // span as handle() so the connect path consumes a candidate that cannot be
        // raced by a concurrent connect on another IPC task.
        let adoption_candidate = self.scrcpy_adoption_candidate_for(&request).await;

        let response = {
            let mut phone = self.phone.lock().await;
            if let Some(default) = scrcpy_size_default {
                phone.set_scrcpy_host_size_default(default);
            }
            if adoption_candidate.is_some() {
                phone.set_scrcpy_adoption_candidate(adoption_candidate);
            }
            let response = phone.handle(request).await;
            // Clear the candidate so it never leaks into a later connect for a
            // different serial.
            phone.set_scrcpy_adoption_candidate(None);
            response
        };

        // Discover and map a freshly-launched managed scrcpy window (connect path).
        // The window mapping keeps the mirror sized/located correctly; the agent
        // cursor itself is now drawn on the device by the companion overlay, so no
        // host-desktop cursor is pushed onto the shared OverlayController.
        self.map_scrcpy_window_if_pending().await;

        // Re-read the connected session so the response reflects any mapping the
        // daemon just applied (host_window_mapped flipped true).
        let response = self.refresh_phone_response_after_mapping(response).await;
        ServiceResponse::Phone { response }
    }

    /// Compute the host-derived phone-scale scrcpy `--max-size` cap for a connect
    /// that will launch a managed mirror, or `None` for any other request.
    ///
    /// The outer `Option` says whether to set the manager field at all: `None`
    /// for non-scrcpy requests, which never read it. The inner `Option<u32>` is
    /// the cap itself — `None` when the host topology is unknown, leaving any
    /// configured `[phone] max_size` to stand. The display probe runs here,
    /// outside the phone lock, so the slow topology probe never widens lock
    /// contention; the caller applies the result under the same lock span as
    /// `handle`, closing the race a separate prime/handle lock pair would open.
    /// Only a scrcpy-bearing `Connect` probes, so the common no-mirror path pays
    /// nothing. A failed probe degrades to an uncapped (full-resolution) mirror
    /// and is logged rather than silently swallowed.
    async fn scrcpy_host_size_default_for(&self, request: &PhoneRequest) -> Option<Option<u32>> {
        let PhoneRequest::Connect(connect) = request else {
            return None;
        };
        if !(connect.start_scrcpy || connect.backend == Some(PhoneBackendKind::Scrcpy)) {
            return None;
        }
        let displays = match self.backend.list_displays().await {
            Ok(displays) => displays,
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "phone scrcpy sizing: list_displays failed; mirror will use full resolution"
                );
                Vec::new()
            }
        };
        Some(host_scrcpy_default_max_size(&displays))
    }

    /// Find a pre-existing scrcpy desktop window the connect path should adopt
    /// instead of spawning a duplicate mirror, or `None` for any request that does
    /// not launch a mirror, an unnamed serial, or when no adoptable window exists.
    ///
    /// The manager owns no desktop backend, so the daemon enumerates `list_windows`
    /// here (outside the phone lock; the scan is slow) and lets the manager match
    /// the deterministic `sky-cua-phone-<safe-serial>` title. Adoption is scoped to
    /// a connect that names an explicit serial: an unspecified serial is resolved
    /// inside the manager (default/single-device), so the deterministic title is
    /// not known pre-handle, and the safe fallback is to launch fresh rather than
    /// guess. A failed enumeration degrades to a fresh launch and is logged.
    async fn scrcpy_adoption_candidate_for(
        &self,
        request: &PhoneRequest,
    ) -> Option<crate::phone::ScrcpyAdoptionCandidate> {
        let PhoneRequest::Connect(connect) = request else {
            return None;
        };
        if !(connect.start_scrcpy || connect.backend == Some(PhoneBackendKind::Scrcpy)) {
            return None;
        }
        let serial = connect
            .serial
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let windows = match self.backend.list_windows().await {
            Ok(windows) => windows,
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "phone scrcpy adoption: list_windows failed; connect will launch a fresh mirror"
                );
                return None;
            }
        };
        self.phone
            .lock()
            .await
            .find_adoptable_scrcpy_window(serial, &windows)
    }

    /// If any session has a live managed scrcpy mirror with no host-window mapping
    /// yet, locate its desktop window and store the content-rect mapping. Bounded
    /// retry: the window can take ~1-2s to register after launch, so this polls a
    /// few times, but gives up rather than blocking forever, leaving
    /// `host_window_mapped=false` honestly.
    ///
    /// When nothing is awaiting an initial map, a single re-query of an
    /// already-mapped session's window catches a stale mapping (the operator
    /// resized the scrcpy window, or the host display scale changed); a drifted
    /// content rect is recomputed and an unchanged one is a no-op. The re-map runs
    /// only on this per-request window-work path the daemon already takes — no new
    /// polling loop.
    async fn map_scrcpy_window_if_pending(&self) {
        const MAX_ATTEMPTS: usize = 10;
        const POLL_INTERVAL: Duration = Duration::from_millis(200);

        let Some(target) = self.phone.lock().await.scrcpy_window_to_map() else {
            // No initial map pending: check whether an already-mapped window
            // drifted (resize / host-scale change) and recompute if so.
            self.remap_scrcpy_window_if_drifted().await;
            return;
        };

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            let windows = match self.backend.list_windows().await {
                Ok(windows) => windows,
                Err(error) => {
                    debug!(
                        code = error.code,
                        message = error.message,
                        "scrcpy window mapping: list_windows failed"
                    );
                    // The backend cannot enumerate windows at all; mapping is not
                    // achievable for this session, so mark the round exhausted to
                    // avoid re-polling on every subsequent phone request.
                    self.phone
                        .lock()
                        .await
                        .mark_scrcpy_mapping_exhausted(&target.session_id);
                    return;
                }
            };
            let Some(window) = select_scrcpy_window(&windows, target.pid, &target.window_title)
            else {
                continue;
            };
            let Some(bounds) = window.bounds.clone() else {
                // Matched a window with no bounds; cannot compute a content rect.
                continue;
            };
            // The manager owns the content-rect math (letterboxing, rotation,
            // fractional scale); the daemon supplies the discovered host rect and
            // the device size/rotation it read from the same target.
            let mapped = self.phone.lock().await.set_scrcpy_window_mapping(
                &target.session_id,
                &bounds,
                target.device_size.clone(),
                target.rotation_degrees,
            );
            if mapped {
                return;
            }
        }

        // The bounded retry round ended without a mapping: the window never
        // registered (or never produced bounds). Mark the round exhausted so the
        // daemon stops re-running this ~2s poll on every future phone request for
        // this session; `host_window_mapped` stays honestly false. A fresh runtime
        // on reconnect/refresh re-arms the attempt.
        self.phone
            .lock()
            .await
            .mark_scrcpy_mapping_exhausted(&target.session_id);
    }

    /// Re-query an already host-mapped scrcpy window once and recompute its
    /// content-rect mapping if the live window rect drifted from the stored one
    /// (the operator resized the scrcpy window, or the host display scale changed).
    ///
    /// Bounded by construction: a single `list_windows` (no retry loop — the window
    /// already registered, since it is mapped) on the per-request path the daemon
    /// already runs. The live bounds are fed back through the manager's idempotent
    /// `set_scrcpy_window_mapping`, so an unchanged rect recomputes the same content
    /// rect and returns without profile churn, while a changed rect rebuilds the
    /// mapping so the host cursor overlay tracks the resized window. A failed
    /// enumeration keeps the existing mapping because the host backend is
    /// temporarily unavailable. A vanished or bounds-less window clears the stale
    /// host mapping so the overlay plane is disabled instead of drawing against an
    /// old rectangle.
    async fn remap_scrcpy_window_if_drifted(&self) {
        let Some(target) = self.phone.lock().await.scrcpy_window_to_remap() else {
            return;
        };
        let windows = match self.backend.list_windows().await {
            Ok(windows) => windows,
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "scrcpy window re-map: list_windows failed; keeping the existing mapping"
                );
                return;
            }
        };
        let Some(window) = select_scrcpy_window(&windows, target.pid, &target.window_title) else {
            let _ = self
                .phone
                .lock()
                .await
                .clear_scrcpy_window_mapping(&target.session_id);
            return;
        };
        let Some(bounds) = window.bounds.clone() else {
            let _ = self
                .phone
                .lock()
                .await
                .clear_scrcpy_window_mapping(&target.session_id);
            return;
        };
        // Idempotent on the manager side: an unchanged content rect is a no-op,
        // a drifted one is recomputed.
        let _ = self.phone.lock().await.set_scrcpy_window_mapping(
            &target.session_id,
            &bounds,
            target.device_size.clone(),
            target.rotation_degrees,
        );
    }

    /// Re-fetch the connected session after a mapping so a `phone_connect`
    /// response reflects `host_window_mapped=true`. Only `Connected` responses
    /// carry a session view to refresh; everything else passes through unchanged.
    async fn refresh_phone_response_after_mapping(
        &self,
        response: sky_cua_platform::model::PhoneResponse,
    ) -> sky_cua_platform::model::PhoneResponse {
        use sky_cua_platform::model::PhoneResponse;
        let PhoneResponse::Connected(session) = &response else {
            return response;
        };
        let refreshed = self.phone.lock().await.session_view(&session.session_id);
        match refreshed {
            Some(session) => PhoneResponse::Connected(session),
            None => response,
        }
    }

    async fn handle_browser_request(&self, request: BrowserRequest) -> ServiceResponse {
        match request {
            BrowserRequest::ListTabs { target } => {
                debug!(?target, "handling browser_list_tabs request");
                ServiceResponse::Browser {
                    response: BrowserResponse::ListTabs {
                        response: crate::browser::list_tabs(target).await,
                    },
                }
            }
            BrowserRequest::Open { target, url } => {
                debug!(?target, ?url, "handling browser_open request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Open {
                        response: crate::browser::open_tab(target, url).await,
                    },
                }
            }
            BrowserRequest::ClaimTab { target, tab_id } => {
                debug!(?target, ?tab_id, "handling browser_claim_tab request");
                ServiceResponse::Browser {
                    response: BrowserResponse::ClaimTab {
                        response: crate::browser::claim_tab(target, tab_id).await,
                    },
                }
            }
            BrowserRequest::MoveMouse {
                target,
                tab_id,
                x,
                y,
                wait_for_arrival,
            } => {
                debug!(
                    ?target,
                    ?tab_id,
                    x,
                    y,
                    wait_for_arrival,
                    "handling browser_move_mouse request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::MoveMouse {
                        response: crate::browser::move_mouse(
                            target,
                            tab_id,
                            x,
                            y,
                            wait_for_arrival,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Navigate {
                target,
                tab_id,
                url,
            } => {
                debug!(?target, ?tab_id, ?url, "handling browser_navigate request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Navigate {
                        response: crate::browser::navigate(target, tab_id, url).await,
                    },
                }
            }
            BrowserRequest::Snapshot {
                target,
                tab_id,
                text_limit,
                element_offset,
                element_limit,
                element_query,
            } => {
                if text_limit.is_some_and(|value| value > BROWSER_SNAPSHOT_MAX_TEXT_LIMIT) {
                    return ServiceResponse::Error {
                        code: "InvalidRequest".to_string(),
                        message: format!(
                            "browser_snapshot text_limit must be at most {BROWSER_SNAPSHOT_MAX_TEXT_LIMIT}"
                        ),
                    };
                }
                if element_limit.is_some_and(|value| value > BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT) {
                    return ServiceResponse::Error {
                        code: "InvalidRequest".to_string(),
                        message: format!(
                            "browser_snapshot element_limit must be at most {BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT}"
                        ),
                    };
                }
                debug!(
                    ?target,
                    ?tab_id,
                    ?text_limit,
                    ?element_offset,
                    ?element_limit,
                    ?element_query,
                    "handling browser_snapshot request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::Snapshot {
                        response: crate::browser::snapshot(
                            target,
                            tab_id,
                            text_limit,
                            element_offset,
                            element_limit,
                            element_query,
                        )
                        .await,
                    },
                }
            }
            BrowserRequest::Screenshot {
                target,
                tab_id,
                include_image_data,
            } => {
                debug!(
                    ?target,
                    ?tab_id,
                    ?include_image_data,
                    "handling browser_screenshot request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::Screenshot {
                        response: crate::browser::screenshot(target, tab_id, include_image_data)
                            .await,
                    },
                }
            }
            BrowserRequest::Click {
                target,
                tab_id,
                x,
                y,
            } => {
                debug!(?target, ?tab_id, x, y, "handling browser_click request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Click {
                        response: crate::browser::click(target, tab_id, x, y).await,
                    },
                }
            }
            BrowserRequest::TypeText {
                target,
                tab_id,
                text,
            } => {
                debug!(?target, ?tab_id, "handling browser_type_text request");
                ServiceResponse::Browser {
                    response: BrowserResponse::TypeText {
                        response: crate::browser::type_text(target, tab_id, text).await,
                    },
                }
            }
            BrowserRequest::PressKey {
                target,
                tab_id,
                key,
            } => {
                debug!(?target, ?tab_id, ?key, "handling browser_press_key request");
                ServiceResponse::Browser {
                    response: BrowserResponse::PressKey {
                        response: crate::browser::press_key(target, tab_id, key).await,
                    },
                }
            }
            BrowserRequest::Scroll {
                target,
                tab_id,
                delta_x,
                delta_y,
                x,
                y,
            } => {
                debug!(
                    ?target,
                    ?tab_id,
                    delta_x,
                    delta_y,
                    x,
                    y,
                    "handling browser_scroll request"
                );
                ServiceResponse::Browser {
                    response: BrowserResponse::Scroll {
                        response: crate::browser::scroll(target, tab_id, delta_x, delta_y, x, y)
                            .await,
                    },
                }
            }
            BrowserRequest::Eval {
                target,
                tab_id,
                expression,
            } => {
                debug!(?target, ?tab_id, "handling browser_eval request");
                ServiceResponse::Browser {
                    response: BrowserResponse::Eval {
                        response: crate::browser::eval(target, tab_id, expression).await,
                    },
                }
            }
            BrowserRequest::Status => self.handle_browser_status_request().await,
        }
    }

    async fn handle_desktop_request(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::Health => unreachable!("health bypasses the desktop request lane"),
            ServiceRequest::Browser { .. } => {
                unreachable!("browser requests bypass the desktop request lane")
            }
            ServiceRequest::Phone { .. } => {
                unreachable!("phone requests bypass the desktop request lane")
            }
            ServiceRequest::SessionPresence { action } => match action {
                SessionPresenceAction::Ensure(intent) => {
                    if !self.session_presence_config.enabled {
                        return session_presence_disabled_response();
                    }
                    match self.backend.ensure_session_presence(intent).await {
                        Ok(status) => {
                            let mut held = self.session_presence_held.lock().await;
                            *held = true;
                            ServiceResponse::SessionPresence { status }
                        }
                        Err(error) => error_response(error.code, error.message),
                    }
                }
                SessionPresenceAction::Release { relock } => {
                    if !self.session_presence_config.enabled {
                        return session_presence_disabled_response();
                    }
                    match self.backend.release_session_presence(relock).await {
                        Ok(status) => {
                            let mut held = self.session_presence_held.lock().await;
                            *held = false;
                            ServiceResponse::SessionPresence { status }
                        }
                        Err(error) => error_response(error.code, error.message),
                    }
                }
                SessionPresenceAction::Status => ServiceResponse::SessionPresence {
                    status: self.backend.session_presence_status().await,
                },
            },
            ServiceRequest::Doctor => match self.backend.doctor().await {
                Ok(report) => ServiceResponse::Doctor {
                    report: Box::new(report),
                },
                Err(error) => error_response(error.code, error.message),
            },
            ServiceRequest::SetupAccessibility => match self.backend.setup_accessibility().await {
                Ok(report) => ServiceResponse::SetupAccessibility {
                    report: Box::new(report),
                },
                Err(error) => error_response(error.code, error.message),
            },
            ServiceRequest::SetupWindowTargeting => {
                match self.backend.setup_window_targeting().await {
                    Ok(report) => ServiceResponse::SetupWindowTargeting {
                        report: Box::new(report),
                    },
                    Err(error) => error_response(error.code, error.message),
                }
            }
            ServiceRequest::ListApps => {
                debug!("handling list_apps request");
                let environment = match self.backend.probe_environment().await {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self.backend.list_apps().await {
                    Ok(apps) => {
                        let diagnostics = self.backend.session_env_diagnostics();
                        ServiceResponse::ListApps {
                            environment,
                            apps,
                            diagnostics,
                        }
                    }
                    Err(error) => ServiceResponse::ListApps {
                        environment,
                        apps: Vec::new(),
                        diagnostics: {
                            let mut diagnostics = self.backend.session_env_diagnostics();
                            diagnostics.push(error.diagnostic());
                            diagnostics
                        },
                    },
                }
            }
            ServiceRequest::ListWindows => {
                debug!("handling list_windows request");
                let environment = match self.backend.probe_environment().await {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self.backend.list_windows().await {
                    Ok(windows) => ServiceResponse::ListWindows {
                        environment,
                        windows,
                        diagnostics: Vec::new(),
                    },
                    Err(error) => ServiceResponse::ListWindows {
                        environment,
                        windows: Vec::new(),
                        diagnostics: vec![error.diagnostic()],
                    },
                }
            }
            ServiceRequest::FocusedWindow => {
                debug!("handling focused_window request");
                let environment = match self.backend.probe_environment().await {
                    Ok(environment) => environment,
                    Err(error) => return error_response(error.code, error.message),
                };
                match self.backend.focused_window().await {
                    Ok(window) => ServiceResponse::FocusedWindow {
                        environment,
                        window: window.map(Box::new),
                        diagnostics: Vec::new(),
                    },
                    Err(error) => ServiceResponse::FocusedWindow {
                        environment,
                        window: None,
                        diagnostics: vec![error.diagnostic()],
                    },
                }
            }
            ServiceRequest::ActivateWindow { target } => {
                debug!(target = ?target, "handling activate_window request");
                match self.backend.activate_window(target).await {
                    Ok(outcome) => ServiceResponse::ActivateWindow { outcome },
                    Err(error) => {
                        let diagnostic = error.diagnostic();
                        ServiceResponse::ActivateWindow {
                            outcome: sky_cua_platform::model::ActionOutcome {
                                success: false,
                                message: error.message.clone(),
                                code: error.code.to_string(),
                                diagnostics: vec![diagnostic],
                                agent_cursor: None,
                            },
                        }
                    }
                }
            }
            ServiceRequest::GetAppState {
                selector,
                capture_screen,
            } => {
                debug!(selector = ?selector, ?capture_screen, "handling get_app_state request");
                let capture_guard = if capture_screen != CaptureScreenMode::Never {
                    Some(self.overlay.lock().await.prepare_for_capture())
                } else {
                    None
                };
                match self.backend.get_app_state(selector, capture_screen).await {
                    Ok(mut snapshot) => {
                        let reused_capture = if capture_screen == CaptureScreenMode::IfChanged {
                            let snapshots = self.snapshots.lock().await;
                            reuse_unchanged_capture(&mut snapshot, snapshots.latest())
                        } else {
                            false
                        };
                        if reused_capture {
                            snapshot.diagnostics.push(DiagnosticEntry {
                                code: "CaptureScreenUnchanged".to_string(),
                                message: "Screen capture matched the previous model-facing image; reusing the previous screenshot path.".to_string(),
                                details: None,
                            });
                        }
                        if let Some(capture_guard) = capture_guard.as_ref() {
                            snapshot
                                .diagnostics
                                .extend(capture_guard.diagnostics.iter().cloned());
                        }
                        {
                            let mut overlay = self.overlay.lock().await;
                            overlay.apply_to_snapshot(&mut snapshot);
                            if let Some(capture_guard) = capture_guard {
                                snapshot
                                    .diagnostics
                                    .extend(overlay.restore_after_capture(capture_guard));
                            }
                        }
                        self.snapshots.lock().await.store(snapshot.clone());
                        ServiceResponse::GetAppState {
                            snapshot: Box::new(snapshot),
                        }
                    }
                    Err(error) => {
                        if let Some(capture_guard) = capture_guard {
                            let _ = self
                                .overlay
                                .lock()
                                .await
                                .restore_after_capture(capture_guard);
                        }
                        error_response(error.code, error.message)
                    }
                }
            }
            ServiceRequest::Screenshot {
                target,
                display_target,
                capture_all_displays,
            } => {
                if screenshot_selector_count(
                    target.as_ref(),
                    display_target.as_ref(),
                    capture_all_displays,
                ) > 1
                {
                    return error_response(
                        BackendErrorCode::InvalidRequest.as_str(),
                        "screenshot accepts exactly one capture selector: window target, display target, or capture_all_displays=true",
                    );
                }
                debug!(
                    target = ?target,
                    display_target = ?display_target,
                    capture_all_displays,
                    "handling screenshot request"
                );
                let capture_guard = Some(self.overlay.lock().await.prepare_for_capture());
                match self
                    .backend
                    .screenshot(target, display_target, capture_all_displays)
                    .await
                {
                    Ok(mut snapshot) => {
                        if let Some(capture_guard) = capture_guard.as_ref() {
                            snapshot
                                .diagnostics
                                .extend(capture_guard.diagnostics.iter().cloned());
                        }
                        {
                            let mut overlay = self.overlay.lock().await;
                            overlay.apply_to_snapshot(&mut snapshot);
                            if let Some(capture_guard) = capture_guard {
                                snapshot
                                    .diagnostics
                                    .extend(overlay.restore_after_capture(capture_guard));
                            }
                        }
                        self.snapshots.lock().await.store(snapshot.clone());
                        ServiceResponse::Screenshot {
                            snapshot: Box::new(snapshot),
                        }
                    }
                    Err(error) => {
                        if let Some(capture_guard) = capture_guard {
                            let _ = self
                                .overlay
                                .lock()
                                .await
                                .restore_after_capture(capture_guard);
                        }
                        error_response(error.code, error.message)
                    }
                }
            }
            ServiceRequest::AgentCursorStatus => {
                let status = self.overlay.lock().await.status();
                agent_cursor_status_response(status, AgentCursorResponseKind::Status)
            }
            ServiceRequest::SetAgentCursor { state } => {
                let status = self.overlay.lock().await.set_state(state);
                agent_cursor_status_response(status, AgentCursorResponseKind::Set)
            }
            ServiceRequest::HideAgentCursor { reason } => {
                let status = self.overlay.lock().await.hide(reason);
                agent_cursor_status_response(status, AgentCursorResponseKind::Hide)
            }
            ServiceRequest::ShowAgentCursor => {
                let status = self.overlay.lock().await.show();
                agent_cursor_status_response(status, AgentCursorResponseKind::Show)
            }
            ServiceRequest::ResetPortalTokens => {
                debug!("handling reset_portal_tokens request");
                match self.backend.reset_portal_tokens().await {
                    Ok(outcome) => ServiceResponse::ResetPortalTokens {
                        cleared: outcome.cleared,
                        token_path: outcome.token_path,
                        dropped_cached_session: outcome.dropped_cached_session,
                    },
                    Err(error) => error_response(error.code, error.message),
                }
            }
            ServiceRequest::ExecuteAction { request } => {
                let request = match self.enrich_action_request(*request).await {
                    Ok(request) => request,
                    Err((code, message)) => return error_response(code, message),
                };
                let mut outcome = route_action(self.backend.as_ref(), request.clone()).await;
                let cursor_diagnostics = self
                    .overlay
                    .lock()
                    .await
                    .update_from_action(&request, &mut outcome);
                outcome.diagnostics.extend(cursor_diagnostics);
                ServiceResponse::ExecuteAction { outcome }
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

    async fn ensure_session_presence_for_request(&self, request: &ServiceRequest) {
        if !self.session_presence_config.enabled || !request_should_hold_presence(request) {
            return;
        }

        let mut held = self.session_presence_held.lock().await;
        if *held {
            return;
        }

        let intent = self.session_presence_config.intent();
        match self.backend.ensure_session_presence(intent).await {
            Ok(status) => {
                debug!(
                    backend = status.backend,
                    supported = status.supported,
                    detail = status.detail,
                    "session presence ensured"
                );
                *held = true;
            }
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "session presence ensure failed"
                );
            }
        }
    }

    async fn release_idle_session_presence_if_needed(&self) {
        if !self.session_presence_config.enabled {
            return;
        }
        if self.sessions.idle_for().await < self.session_presence_config.idle_release {
            return;
        }

        let mut held = self.session_presence_held.lock().await;
        if !*held {
            return;
        }

        match self
            .backend
            .release_session_presence(self.session_presence_config.relock)
            .await
        {
            Ok(status) => {
                debug!(
                    backend = status.backend,
                    supported = status.supported,
                    detail = status.detail,
                    "session presence released after idle timeout"
                );
            }
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "session presence idle release failed"
                );
            }
        }
        *held = false;
    }

    async fn enrich_action_request(
        &self,
        mut request: ActionRequest,
    ) -> Result<ActionRequest, (&'static str, String)> {
        let Some(snapshot_id) = request.snapshot_id.as_deref() else {
            if action_requires_snapshot_context(&request) {
                return Err((
                    "ComputerUseInactive",
                    "Element-targeted actions require a current snapshot_id from get_app_state."
                        .to_string(),
                ));
            }
            request.environment = Some(
                self.backend
                    .probe_environment()
                    .await
                    .map_err(|error| (error.code, error.message))?,
            );
            return Ok(request);
        };
        let snapshot = {
            let snapshots = self.snapshots.lock().await;
            if let Some(snapshot) = snapshots.get_if_latest(snapshot_id) {
                snapshot.clone()
            } else if snapshots.get(snapshot_id).is_some() {
                return Err((
                    "SnapshotStale",
                    format!(
                        "snapshot {snapshot_id} is no longer the latest app state. Re-run get_app_state and retry with the current snapshot_id."
                    ),
                ));
            } else {
                return Err((
                    "SnapshotStale",
                    format!("snapshot {snapshot_id} is not present in the service cache"),
                ));
            }
        };

        request.environment = Some(snapshot.environment.clone());
        request.resolved_capture = snapshot.capture.clone();
        request.resolved_focused_app = snapshot.focused_app.clone();

        request.resolved_element = resolve_action_element(
            &snapshot,
            &request.action,
            request.element_index,
            &request.arguments,
        )?;
        request.resolved_target_element = resolve_target_element(&snapshot, &request.arguments)?;

        Ok(request)
    }

    async fn handle_browser_status_request(&self) -> ServiceResponse {
        debug!("handling browser_status request");
        let integration = {
            let Ok(_desktop_lane) = self.desktop_lane.try_lock() else {
                return ServiceResponse::Browser {
                    response: BrowserResponse::Status {
                        report: crate::browser::browser_status_from_deferred_doctor().await,
                    },
                };
            };
            match self.backend.doctor().await {
                Ok(report) => report.browser_integration,
                Err(error) => return error_response(error.code, error.message),
            }
        };

        ServiceResponse::Browser {
            response: BrowserResponse::Status {
                report: crate::browser::browser_status_from_doctor(integration).await,
            },
        }
    }
}

impl SessionPresenceConfig {
    const DEFAULT_IDLE_RELEASE_SECS: u64 = 90;

    fn from_env() -> Self {
        Self {
            enabled: env_bool("SKY_CUA_PRESENCE_ENABLED", false),
            idle_release: Duration::from_secs(env_u64(
                "SKY_CUA_PRESENCE_IDLE_RELEASE_SECS",
                Self::DEFAULT_IDLE_RELEASE_SECS,
            )),
            unlock: env_bool("SKY_CUA_PRESENCE_UNLOCK", true),
            relock: env_bool("SKY_CUA_PRESENCE_RELOCK", true),
            inhibit_lock: env_bool("SKY_CUA_PRESENCE_INHIBIT_LOCK", true),
            inhibit_suspend: env_bool("SKY_CUA_PRESENCE_INHIBIT_SUSPEND", true),
        }
    }

    #[cfg(test)]
    fn disabled() -> Self {
        Self {
            enabled: false,
            idle_release: Duration::from_secs(Self::DEFAULT_IDLE_RELEASE_SECS),
            unlock: true,
            relock: true,
            inhibit_lock: true,
            inhibit_suspend: true,
        }
    }

    fn intent(&self) -> SessionPresenceIntent {
        SessionPresenceIntent {
            unlock: self.unlock,
            inhibit_lock: self.inhibit_lock,
            inhibit_suspend: self.inhibit_suspend,
        }
    }
}

fn session_presence_disabled_response() -> ServiceResponse {
    error_response(
        sky_cua_platform::BackendErrorCode::ActionUnsupportedForEnvironment.as_str(),
        "session presence is disabled; set SKY_CUA_PRESENCE_ENABLED=1 on the daemon to allow \
         unlock and inhibitor requests",
    )
}

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn reuse_unchanged_capture(
    snapshot: &mut AppStateSnapshot,
    previous: Option<&AppStateSnapshot>,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    let (Some(current_capture), Some(previous_capture)) =
        (snapshot.capture.as_mut(), previous.capture.as_ref())
    else {
        return false;
    };
    if !capture_metadata_compatible_for_reuse(current_capture, previous_capture) {
        return false;
    }
    let Some(current_path) = current_capture.screenshot_path.as_deref() else {
        return false;
    };
    let Some(previous_path) = comparable_previous_screenshot_path(previous_capture) else {
        return false;
    };
    let Ok(current_bytes) = fs::read(current_path) else {
        return false;
    };
    let Ok(previous_bytes) = fs::read(previous_path) else {
        return false;
    };
    if current_bytes != previous_bytes {
        return false;
    }

    current_capture.screenshot_path = previous_capture.screenshot_path.clone();
    current_capture.model_image_bytes = previous_capture.model_image_bytes;
    current_capture.model_image_encode_ms = previous_capture.model_image_encode_ms;
    true
}

fn comparable_previous_screenshot_path(capture: &CaptureInfo) -> Option<String> {
    let screenshot_path = capture.screenshot_path.as_deref()?;
    let path = Path::new(screenshot_path);
    if let Some(raw_path) = decomposited_screenshot_path(path)
        && raw_path.is_file()
    {
        return Some(raw_path.display().to_string());
    }
    Some(screenshot_path.to_string())
}

fn decomposited_screenshot_path(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let raw_stem = stem.strip_suffix(".agent-cursor")?;
    let extension = path.extension()?;
    Some(path.with_file_name(Path::new(raw_stem).with_extension(extension)))
}

fn capture_metadata_compatible_for_reuse(current: &CaptureInfo, previous: &CaptureInfo) -> bool {
    current.backend == previous.backend
        && current.image_backend == previous.image_backend
        && current.coordinate_space == previous.coordinate_space
        && current.pixel_size == previous.pixel_size
        && current.original_pixel_size == previous.original_pixel_size
        && current.logical_to_pixel_scale == previous.logical_to_pixel_scale
        && current.logical_rect == previous.logical_rect
        && current.model_image_format == previous.model_image_format
        && current.model_image_quality == previous.model_image_quality
}

enum AgentCursorResponseKind {
    Status,
    Set,
    Hide,
    Show,
}

fn agent_cursor_status_response(
    status: AgentCursorStatus,
    kind: AgentCursorResponseKind,
) -> ServiceResponse {
    match kind {
        AgentCursorResponseKind::Status => ServiceResponse::AgentCursorStatus {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Set => ServiceResponse::SetAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Hide => ServiceResponse::HideAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Show => ServiceResponse::ShowAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
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

fn action_requires_snapshot_context(request: &ActionRequest) -> bool {
    let has_snapshot_target = request.arguments.get("to_element_index").is_some();
    let has_semantic_selector = request.arguments.get("role").is_some()
        || request.arguments.get("name").is_some()
        || (request.arguments.get("text").is_some() && request.action != ActionName::TypeText)
        || request.arguments.get("states").is_some();

    if request
        .arguments
        .get("element_identifier")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return has_snapshot_target;
    }

    matches!(
        request.action,
        ActionName::FocusElement
            | ActionName::ActivateElement
            | ActionName::SelectElement
            | ActionName::ExpandElement
            | ActionName::CollapseElement
            | ActionName::ToggleElement
            | ActionName::PerformAction
            | ActionName::SetValue
    ) || request.element_index.is_some()
        || has_snapshot_target
        || has_semantic_selector
}

fn request_should_hold_presence(request: &ServiceRequest) -> bool {
    match request {
        ServiceRequest::Health
        | ServiceRequest::Doctor
        | ServiceRequest::AgentCursorStatus
        | ServiceRequest::SessionPresence { .. } => false,
        ServiceRequest::Browser { request } => !matches!(
            request,
            BrowserRequest::Status | BrowserRequest::ListTabs { .. }
        ),
        // Phone control drives an Android device, but a write action (tap/swipe/
        // type/press, an app/notification mutation, connect/pair/install) is an
        // active operation the agent is mid-flow on, so it holds presence the same
        // way a desktop write does. Read-only phone perception (status, listing,
        // observe, screenshot, capability/companion queries) does not.
        ServiceRequest::Phone { request } => phone_request_is_write(request),
        _ => true,
    }
}

/// Whether a phone request mutates device or session state (and therefore holds
/// session presence). Read-only perception/inspection tools return `false`.
fn phone_request_is_write(request: &PhoneRequest) -> bool {
    matches!(
        request,
        PhoneRequest::Connect(_)
            | PhoneRequest::Disconnect(_)
            | PhoneRequest::PairWireless(_)
            | PhoneRequest::Tap(_)
            | PhoneRequest::Swipe(_)
            | PhoneRequest::TypeText(_)
            | PhoneRequest::PressKey(_)
            | PhoneRequest::InstallCompanion(_)
            | PhoneRequest::NotificationOpen(_)
            | PhoneRequest::NotificationDismiss(_)
            | PhoneRequest::NotificationAction(_)
            | PhoneRequest::NotificationReply(_)
            | PhoneRequest::AppLaunch(_)
            | PhoneRequest::AppOpenIntent(_)
            | PhoneRequest::AppForceStop(_)
            | PhoneRequest::AppInstall(_)
            | PhoneRequest::OpenSettings(_)
    )
}

fn screenshot_selector_count(
    target: Option<&WindowTarget>,
    display_target: Option<&DisplayTarget>,
    capture_all_displays: bool,
) -> usize {
    usize::from(target.is_some())
        + usize::from(display_target.is_some())
        + usize::from(capture_all_displays)
}

/// Pick the scrcpy desktop window out of a window list for a managed mirror.
///
/// Matching is by `pid` first (robust: the mirror's process id is the strongest
/// signal and survives title collisions), then falls back to an exact
/// `window_title` match (the `sky-cua-phone-<safe-serial>` slug) when no pid is
/// known or no pid matched. Pure and testable: callers pass a constructed window
/// list and the target's pid/title.
fn select_scrcpy_window<'a>(
    windows: &'a [WindowInfo],
    pid: Option<u32>,
    window_title: &str,
) -> Option<&'a WindowInfo> {
    if let Some(pid) = pid
        && let Some(window) = windows.iter().find(|window| window.pid == Some(pid))
    {
        return Some(window);
    }
    windows
        .iter()
        .find(|window| window.title.as_deref() == Some(window_title))
}

#[cfg(test)]
mod tests {
    use super::{
        OverlayController, ServiceDaemon, SessionPresenceConfig, SessionStore, SnapshotManager,
        action_requires_snapshot_context, request_should_hold_presence, reuse_unchanged_capture,
    };
    use image::{ImageBuffer, Rgba};
    use serde_json::json;
    use sky_cua_platform::backend::DesktopBackend;
    use sky_cua_platform::diagnostics::BackendError;
    use sky_cua_platform::model::{
        ActionName, ActionOutcome, ActionRequest, AgentCursorPoint, AgentCursorState, AppInfo,
        AppSelector, AppStateSnapshot, BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT,
        BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, BrowserRequest, BrowserResponse, BrowserTargetKind,
        CaptureBackendKind, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace,
        DisplayTarget, ElementNode, EnvironmentInfo, InputBackendKind, ModelImageFormat,
        PhoneAppListRequest, PhoneAppResponseKind, PhoneConnectRequest, PhoneListDevicesRequest,
        PhoneRequest, PhoneResponse, PhoneStatusRequest, PhoneTapRequest, PixelSize,
        PortalCapabilities, RectF, SemanticBackendKind, ServiceRequest, ServiceResponse,
        SessionKind, SessionPresenceAction, SessionPresenceIntent, SessionPresenceStatus,
        ToolAvailability, ToolCapabilities, WindowInfo, WindowTarget,
    };
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Notify;

    #[derive(Debug, Clone)]
    struct FakeBackend {
        snapshot: AppStateSnapshot,
        outcome: ActionOutcome,
        presence: Option<Arc<PresenceRecorder>>,
    }

    #[derive(Debug, Default)]
    struct PresenceRecorder {
        ensure_calls: AtomicUsize,
        release_calls: AtomicUsize,
        last_intent: std::sync::Mutex<Option<SessionPresenceIntent>>,
        last_relock: std::sync::Mutex<Option<bool>>,
    }

    #[derive(Debug, Clone)]
    struct BlockingBackend {
        snapshot: AppStateSnapshot,
        outcome: ActionOutcome,
        execute_calls: Arc<AtomicUsize>,
        first_execute_started: Arc<Notify>,
        second_execute_started: Arc<Notify>,
        release_first_execute: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl DesktopBackend for FakeBackend {
        async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
            Ok(self.snapshot.environment.clone())
        }

        async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
            Ok(Vec::new())
        }

        async fn get_app_state(
            &self,
            _selector: Option<AppSelector>,
            _capture_screen: CaptureScreenMode,
        ) -> Result<AppStateSnapshot, BackendError> {
            Ok(self.snapshot.clone())
        }

        async fn execute_action(
            &self,
            _request: ActionRequest,
        ) -> Result<ActionOutcome, BackendError> {
            Ok(self.outcome.clone())
        }

        async fn ensure_session_presence(
            &self,
            intent: SessionPresenceIntent,
        ) -> Result<SessionPresenceStatus, BackendError> {
            if let Some(presence) = &self.presence {
                presence.record_ensure(intent);
            }
            Ok(SessionPresenceStatus {
                backend: "fake".to_string(),
                supported: true,
                unlock_supported: true,
                locked: Some(false),
                lock_inhibited: intent.inhibit_lock,
                suspend_inhibited: intent.inhibit_suspend,
                detail: "fake session presence ensured".to_string(),
            })
        }

        async fn release_session_presence(
            &self,
            relock: bool,
        ) -> Result<SessionPresenceStatus, BackendError> {
            if let Some(presence) = &self.presence {
                presence.record_release(relock);
            }
            Ok(SessionPresenceStatus {
                backend: "fake".to_string(),
                supported: true,
                unlock_supported: true,
                locked: Some(relock),
                lock_inhibited: false,
                suspend_inhibited: false,
                detail: "fake session presence released".to_string(),
            })
        }
    }

    #[async_trait::async_trait]
    impl DesktopBackend for BlockingBackend {
        async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
            Ok(self.snapshot.environment.clone())
        }

        async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
            Ok(Vec::new())
        }

        async fn get_app_state(
            &self,
            _selector: Option<AppSelector>,
            _capture_screen: CaptureScreenMode,
        ) -> Result<AppStateSnapshot, BackendError> {
            Ok(self.snapshot.clone())
        }

        async fn execute_action(
            &self,
            _request: ActionRequest,
        ) -> Result<ActionOutcome, BackendError> {
            let call = self.execute_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == 1 {
                self.first_execute_started.notify_one();
                self.release_first_execute.notified().await;
            } else if call == 2 {
                self.second_execute_started.notify_one();
            }
            Ok(self.outcome.clone())
        }
    }

    fn request(action: ActionName, arguments: serde_json::Value) -> ActionRequest {
        ActionRequest {
            action,
            snapshot_id: None,
            element_index: None,
            arguments,
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: None,
        }
    }

    #[test]
    fn only_activity_requests_trigger_automatic_session_presence() {
        assert!(!request_should_hold_presence(&ServiceRequest::Health));
        assert!(!request_should_hold_presence(&ServiceRequest::Doctor));
        assert!(!request_should_hold_presence(
            &ServiceRequest::AgentCursorStatus
        ));
        assert!(!request_should_hold_presence(&ServiceRequest::Browser {
            request: BrowserRequest::Status,
        }));
        assert!(request_should_hold_presence(&ServiceRequest::Browser {
            request: BrowserRequest::Click {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "tab".to_string(),
                x: 1.0,
                y: 2.0,
            },
        }));
        assert!(request_should_hold_presence(&ServiceRequest::Screenshot {
            target: None,
            display_target: None,
            capture_all_displays: false,
        }));
        assert!(request_should_hold_presence(
            &ServiceRequest::ExecuteAction {
                request: Box::new(request(ActionName::Click, json!({"x": 1.0, "y": 2.0}),)),
            },
        ));
        // Read-only phone perception does not hold presence; a device-mutating
        // phone write (tap) does, like a desktop write.
        assert!(!request_should_hold_presence(&ServiceRequest::Phone {
            request: PhoneRequest::Status(PhoneStatusRequest::default()),
        }));
        assert!(!request_should_hold_presence(&ServiceRequest::Phone {
            request: PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
        }));
        assert!(request_should_hold_presence(&ServiceRequest::Phone {
            request: PhoneRequest::Tap(PhoneTapRequest {
                session: Default::default(),
                phone_snapshot_id: None,
                x: 1.0,
                y: 2.0,
                use_device_coordinates: true,
            }),
        }));
    }

    #[tokio::test]
    async fn phone_requests_route_through_manager_to_matching_response_variants() {
        let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());

        // status -> Status, never a fabricated session.
        match daemon
            .handle(ServiceRequest::Phone {
                request: PhoneRequest::Status(PhoneStatusRequest::default()),
            })
            .await
        {
            ServiceResponse::Phone {
                response: PhoneResponse::Status(report),
            } => {
                assert!(report.sessions.is_empty());
                assert!(!report.adb_available);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // list_devices -> Devices with an honest diagnostic.
        match daemon
            .handle(ServiceRequest::Phone {
                request: PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
            })
            .await
        {
            ServiceResponse::Phone {
                response: PhoneResponse::Devices(response),
            } => {
                assert!(response.devices.is_empty());
                assert!(!response.diagnostics.is_empty());
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // connect -> Status (no live device in Phase 1; never a Connected session).
        match daemon
            .handle(ServiceRequest::Phone {
                request: PhoneRequest::Connect(PhoneConnectRequest::default()),
            })
            .await
        {
            ServiceResponse::Phone {
                response: PhoneResponse::Status(report),
            } => assert!(report.sessions.is_empty()),
            other => panic!("connect must not fabricate a session: {other:?}"),
        }

        // tap with no active session -> Action with no backend and a structured
        // no-session diagnostic (connect must run first).
        match daemon
            .handle(ServiceRequest::Phone {
                request: PhoneRequest::Tap(PhoneTapRequest {
                    session: Default::default(),
                    phone_snapshot_id: Some("snap".to_string()),
                    x: 5.0,
                    y: 6.0,
                    use_device_coordinates: false,
                }),
            })
            .await
        {
            ServiceResponse::Phone {
                response: PhoneResponse::Action(response),
            } => {
                assert_eq!(response.action, "phone_tap");
                assert_eq!(
                    response.backend,
                    sky_cua_platform::model::PhoneBackendKind::None
                );
                assert!(
                    response
                        .diagnostics
                        .iter()
                        .any(|d| d.code == "PhoneNoSession")
                );
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // app_list -> App with kind List.
        match daemon
            .handle(ServiceRequest::Phone {
                request: PhoneRequest::AppList(PhoneAppListRequest::default()),
            })
            .await
        {
            ServiceResponse::Phone {
                response: PhoneResponse::App(response),
            } => assert_eq!(response.kind, PhoneAppResponseKind::List),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn screenshot_rejects_mixed_selectors_at_service_boundary() {
        let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());

        match daemon
            .handle(ServiceRequest::Screenshot {
                target: Some(WindowTarget {
                    window_id: Some("w1".to_string()),
                    ..Default::default()
                }),
                display_target: Some(DisplayTarget {
                    display_id: Some("kwin:HDMI-A-1".to_string()),
                    display_name: None,
                    display_index: None,
                }),
                capture_all_displays: false,
            })
            .await
        {
            ServiceResponse::Error { code, message } => {
                assert_eq!(code, "InvalidRequest");
                assert!(message.contains("exactly one capture selector"));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn snapshotless_physical_actions_do_not_require_cached_snapshot_context() {
        assert!(!action_requires_snapshot_context(&request(
            ActionName::Click,
            json!({"x": 10.0, "y": 20.0}),
        )));
        assert!(!action_requires_snapshot_context(&request(
            ActionName::TypeText,
            json!({"text": "hello"}),
        )));
        assert!(!action_requires_snapshot_context(&request(
            ActionName::PressKey,
            json!({"key": "Enter"}),
        )));
    }

    #[test]
    fn element_and_semantic_actions_require_cached_snapshot_context() {
        let mut click = request(ActionName::Click, json!({}));
        click.element_index = Some(3);
        assert!(action_requires_snapshot_context(&click));

        assert!(action_requires_snapshot_context(&request(
            ActionName::Drag,
            json!({"to_element_index": 4}),
        )));
        assert!(action_requires_snapshot_context(&request(
            ActionName::SetValue,
            json!({"value": "hello"}),
        )));
        assert!(action_requires_snapshot_context(&request(
            ActionName::ActivateElement,
            json!({}),
        )));
    }

    #[test]
    fn direct_backend_ref_only_bypasses_action_target_resolution() {
        assert!(!action_requires_snapshot_context(&request(
            ActionName::PerformAction,
            json!({"element_identifier": ":1.2:/node/3", "action_name": "press"}),
        )));
        assert!(action_requires_snapshot_context(&request(
            ActionName::Drag,
            json!({"element_identifier": ":1.2:/node/3", "to_element_index": 4}),
        )));
    }

    #[tokio::test]
    async fn cursor_status_requests_round_trip_through_daemon_handle() {
        let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
        let state = AgentCursorState {
            visible: true,
            sequence: 99,
            model_point: Some(AgentCursorPoint {
                x: 12.0,
                y: 34.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: Some("stream".to_string()),
            }),
            native_point: None,
            snapshot_id: Some("snap".to_string()),
            source_action: Some(ActionName::Click),
            updated_at_ms: 0,
        };

        match daemon
            .handle(ServiceRequest::SetAgentCursor { state })
            .await
        {
            ServiceResponse::SetAgentCursor {
                state: Some(state),
                diagnostics,
                ..
            } => {
                assert_eq!(state.sequence, 1);
                assert!(diagnostics.is_empty());
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match daemon.handle(ServiceRequest::AgentCursorStatus).await {
            ServiceResponse::AgentCursorStatus {
                capabilities,
                state: Some(state),
                diagnostics,
            } => {
                assert!(capabilities.screenshot_synthetic_cursor);
                assert_eq!(state.sequence, 1);
                assert!(diagnostics.is_empty());
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match daemon
            .handle(ServiceRequest::HideAgentCursor {
                reason: Some("capture".to_string()),
            })
            .await
        {
            ServiceResponse::HideAgentCursor {
                state: Some(state),
                diagnostics,
                ..
            } => {
                assert!(!state.visible);
                assert!(
                    diagnostics
                        .iter()
                        .any(|entry| entry.code == "AgentCursorHidden")
                );
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match daemon.handle(ServiceRequest::ShowAgentCursor).await {
            ServiceResponse::ShowAgentCursor {
                state: Some(state), ..
            } => assert!(state.visible),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn last_client_cleanup_hides_agent_cursor_state() {
        let daemon = daemon_with(snapshot(None, Vec::new()), success_outcome());
        let state = AgentCursorState {
            visible: true,
            sequence: 99,
            model_point: Some(AgentCursorPoint {
                x: 12.0,
                y: 34.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: Some("stream".to_string()),
            }),
            native_point: None,
            snapshot_id: Some("snap".to_string()),
            source_action: Some(ActionName::Click),
            updated_at_ms: 0,
        };

        let _ = daemon
            .handle(ServiceRequest::SetAgentCursor { state })
            .await;
        daemon.hide_agent_cursor_after_last_client().await;

        match daemon.handle(ServiceRequest::AgentCursorStatus).await {
            ServiceResponse::AgentCursorStatus {
                state: Some(state), ..
            } => assert!(!state.visible),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_app_state_attaches_cursor_state_and_synthetic_screenshot() {
        let dir = unique_temp_dir("daemon-get-state");
        let source = dir.join("capture.png");
        ImageBuffer::from_pixel(96, 96, Rgba([240u8, 240, 240, 255]))
            .save(&source)
            .expect("write source image");
        let daemon = daemon_with(
            snapshot(Some(capture_with_path(&source)), Vec::new()),
            success_outcome(),
        );
        let state = AgentCursorState {
            visible: true,
            sequence: 0,
            model_point: Some(AgentCursorPoint {
                x: 48.0,
                y: 48.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: Some("stream".to_string()),
            }),
            native_point: None,
            snapshot_id: Some("snap".to_string()),
            source_action: Some(ActionName::Click),
            updated_at_ms: 0,
        };
        let _ = daemon
            .handle(ServiceRequest::SetAgentCursor { state })
            .await;

        match daemon
            .handle(ServiceRequest::GetAppState {
                selector: None,
                capture_screen: CaptureScreenMode::Always,
            })
            .await
        {
            ServiceResponse::GetAppState { snapshot } => {
                assert!(snapshot.agent_cursor.is_some());
                let capture = snapshot.capture.expect("capture should remain present");
                let output = capture.screenshot_path.expect("synthetic screenshot path");
                assert!(output.ends_with("capture.agent-cursor.png"));
                let rendered = image::open(&output).expect("open output").to_rgba8();
                assert!(
                    rendered
                        .pixels()
                        .any(|pixel| pixel != &Rgba([240u8, 240, 240, 255]))
                );
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_action_updates_cursor_state_for_explicit_click() {
        let daemon = daemon_with(
            snapshot(Some(capture_with_rect()), Vec::new()),
            success_outcome(),
        );
        let _ = daemon
            .handle(ServiceRequest::GetAppState {
                selector: None,
                capture_screen: CaptureScreenMode::Always,
            })
            .await;

        let mut click = request(ActionName::Click, json!({"x": 42.0, "y": 24.0}));
        click.snapshot_id = Some("snap".to_string());

        match daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(click),
            })
            .await
        {
            ServiceResponse::ExecuteAction { outcome } => {
                let state = outcome.agent_cursor.expect("outcome cursor state");
                let point = state.model_point.expect("model point");
                assert_eq!(point.x, 42.0);
                assert_eq!(point.y, 24.0);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match daemon.handle(ServiceRequest::AgentCursorStatus).await {
            ServiceResponse::AgentCursorStatus {
                state: Some(state), ..
            } => assert_eq!(state.sequence, 1),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn service_runtime_health_bypasses_blocked_desktop_request() {
        let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
        let first_started = backend.first_execute_started.clone();
        let release_first = backend.release_first_execute.clone();
        let daemon = Arc::new(daemon_with_backend(Box::new(backend)));

        let action_daemon = daemon.clone();
        let action_task = tokio::spawn(async move {
            let action = request(ActionName::Click, json!({"x": 42.0, "y": 24.0}));
            action_daemon
                .handle(ServiceRequest::ExecuteAction {
                    request: Box::new(action),
                })
                .await
        });

        first_started.notified().await;
        let health = tokio::time::timeout(Duration::from_millis(100), async {
            daemon.handle(ServiceRequest::Health).await
        })
        .await;
        assert!(
            health.is_ok(),
            "health should bypass the blocked desktop lane"
        );

        release_first.notify_one();
        match action_task.await.expect("action task") {
            ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn service_runtime_browser_open_bypasses_blocked_desktop_request() {
        let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
        let first_started = backend.first_execute_started.clone();
        let release_first = backend.release_first_execute.clone();
        let daemon = Arc::new(daemon_with_backend(Box::new(backend)));

        let action_daemon = daemon.clone();
        let action_task = tokio::spawn(async move {
            let action = request(ActionName::Click, json!({"x": 42.0, "y": 24.0}));
            action_daemon
                .handle(ServiceRequest::ExecuteAction {
                    request: Box::new(action),
                })
                .await
        });

        first_started.notified().await;
        let browser_open = tokio::time::timeout(Duration::from_millis(100), async {
            daemon
                .handle(ServiceRequest::Browser {
                    request: BrowserRequest::Open {
                        target: Some(BrowserTargetKind::UserChrome),
                        url: Some("file:///etc/passwd".to_string()),
                    },
                })
                .await
        })
        .await
        .expect("browser_open should bypass the blocked desktop lane");
        match browser_open {
            ServiceResponse::Browser {
                response: BrowserResponse::Open { response },
            } => {
                assert!(response.tab.is_none());
                assert_eq!(response.diagnostics.len(), 1);
                let expected_code = if cfg!(target_os = "windows") {
                    "BrowserBridgeUnsupported"
                } else {
                    "BrowserOpenUrlUnsupported"
                };
                assert_eq!(response.diagnostics[0].code, expected_code);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        release_first.notify_one();
        match action_task.await.expect("action task") {
            ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn service_runtime_browser_status_bypasses_blocked_desktop_request() {
        let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
        let first_started = backend.first_execute_started.clone();
        let release_first = backend.release_first_execute.clone();
        let daemon = Arc::new(daemon_with_backend(Box::new(backend)));

        let action_daemon = daemon.clone();
        let action_task = tokio::spawn(async move {
            let action = request(ActionName::Click, json!({"x": 42.0, "y": 24.0}));
            action_daemon
                .handle(ServiceRequest::ExecuteAction {
                    request: Box::new(action),
                })
                .await
        });

        first_started.notified().await;
        let browser_status = tokio::time::timeout(Duration::from_millis(500), async {
            daemon
                .handle(ServiceRequest::Browser {
                    request: BrowserRequest::Status,
                })
                .await
        })
        .await
        .expect("browser_status should bypass the blocked desktop lane");
        match browser_status {
            ServiceResponse::Browser {
                response: BrowserResponse::Status { report },
            } => {
                assert_eq!(report.browser_integration, None);
                let expected_code = if cfg!(target_os = "windows") {
                    "BrowserBridgeUnsupported"
                } else {
                    "BrowserIntegrationDeferred"
                };
                assert!(
                    report
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == expected_code)
                );
            }
            other => panic!("unexpected response: {other:?}"),
        }

        release_first.notify_one();
        match action_task.await.expect("action task") {
            ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn browser_snapshot_rejects_oversized_text_limit_at_service_boundary() {
        let daemon = daemon_with(
            snapshot(Some(capture_with_rect()), Vec::new()),
            success_outcome(),
        );

        match daemon
            .handle(ServiceRequest::Browser {
                request: BrowserRequest::Snapshot {
                    target: Some(BrowserTargetKind::UserChrome),
                    tab_id: "tab-1".to_string(),
                    text_limit: Some(BROWSER_SNAPSHOT_MAX_TEXT_LIMIT + 1),
                    element_offset: None,
                    element_limit: None,
                    element_query: None,
                },
            })
            .await
        {
            ServiceResponse::Error { code, message } => {
                assert_eq!(code, "InvalidRequest");
                assert!(message.contains(&BROWSER_SNAPSHOT_MAX_TEXT_LIMIT.to_string()));
            }
            other => panic!("expected invalid request response, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn browser_snapshot_rejects_oversized_element_limit_at_service_boundary() {
        let daemon = daemon_with(
            snapshot(Some(capture_with_rect()), Vec::new()),
            success_outcome(),
        );

        match daemon
            .handle(ServiceRequest::Browser {
                request: BrowserRequest::Snapshot {
                    target: Some(BrowserTargetKind::UserChrome),
                    tab_id: "tab-1".to_string(),
                    text_limit: None,
                    element_offset: None,
                    element_limit: Some(BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT + 1),
                    element_query: None,
                },
            })
            .await
        {
            ServiceResponse::Error { code, message } => {
                assert_eq!(code, "InvalidRequest");
                assert!(message.contains(&BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT.to_string()));
            }
            other => panic!("expected invalid request response, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn service_runtime_serializes_desktop_lane_requests() {
        let backend = BlockingBackend::new(snapshot(Some(capture_with_rect()), Vec::new()));
        let first_started = backend.first_execute_started.clone();
        let second_started = backend.second_execute_started.clone();
        let release_first = backend.release_first_execute.clone();
        let daemon = Arc::new(daemon_with_backend(Box::new(backend)));

        let first_daemon = daemon.clone();
        let first_task = tokio::spawn(async move {
            let action = request(ActionName::Click, json!({"x": 1.0, "y": 2.0}));
            first_daemon
                .handle(ServiceRequest::ExecuteAction {
                    request: Box::new(action),
                })
                .await
        });
        first_started.notified().await;

        let second_daemon = daemon.clone();
        let second_task = tokio::spawn(async move {
            let action = request(ActionName::Click, json!({"x": 3.0, "y": 4.0}));
            second_daemon
                .handle(ServiceRequest::ExecuteAction {
                    request: Box::new(action),
                })
                .await
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(100), second_started.notified())
                .await
                .is_err(),
            "second desktop request should wait for the first desktop request"
        );
        release_first.notify_one();
        tokio::time::timeout(Duration::from_secs(1), second_started.notified())
            .await
            .expect("second request should enter after first is released");

        match first_task.await.expect("first task") {
            ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
            other => panic!("unexpected response: {other:?}"),
        }
        match second_task.await.expect("second task") {
            ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn automatic_session_presence_acquires_once_and_releases_after_idle() {
        let presence = Arc::new(PresenceRecorder::default());
        let daemon = daemon_with_backend_and_presence_config(
            Box::new(FakeBackend {
                snapshot: snapshot(None, Vec::new()),
                outcome: success_outcome(),
                presence: Some(presence.clone()),
            }),
            SessionPresenceConfig {
                enabled: true,
                idle_release: Duration::from_millis(5),
                unlock: true,
                relock: true,
                inhibit_lock: true,
                inhibit_suspend: true,
            },
        );

        for _ in 0..2 {
            let action = request(ActionName::Click, json!({"x": 1.0, "y": 2.0}));
            match daemon
                .handle(ServiceRequest::ExecuteAction {
                    request: Box::new(action),
                })
                .await
            {
                ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
                other => panic!("unexpected response: {other:?}"),
            }
        }
        assert_eq!(presence.ensure_calls(), 1);
        assert_eq!(presence.release_calls(), 0);
        assert_eq!(
            presence.last_intent(),
            Some(SessionPresenceIntent {
                unlock: true,
                inhibit_lock: true,
                inhibit_suspend: true,
            })
        );

        tokio::time::sleep(Duration::from_millis(8)).await;
        daemon.release_idle_session_presence_if_needed().await;
        daemon.release_idle_session_presence_if_needed().await;

        assert_eq!(presence.ensure_calls(), 1);
        assert_eq!(presence.release_calls(), 1);
        assert_eq!(presence.last_relock(), Some(true));

        let action = request(ActionName::Click, json!({"x": 3.0, "y": 4.0}));
        let _ = daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(action),
            })
            .await;
        assert_eq!(presence.ensure_calls(), 2);
    }

    #[tokio::test]
    async fn explicit_session_presence_requests_are_rejected_when_disabled() {
        let presence = Arc::new(PresenceRecorder::default());
        let daemon = daemon_with_backend_and_presence_config(
            Box::new(FakeBackend {
                snapshot: snapshot(None, Vec::new()),
                outcome: success_outcome(),
                presence: Some(presence.clone()),
            }),
            SessionPresenceConfig::disabled(),
        );

        for action in [
            SessionPresenceAction::Ensure(SessionPresenceIntent {
                unlock: true,
                inhibit_lock: true,
                inhibit_suspend: true,
            }),
            SessionPresenceAction::Release { relock: false },
        ] {
            match daemon
                .handle(ServiceRequest::SessionPresence { action })
                .await
            {
                ServiceResponse::Error { code, .. } => {
                    assert_eq!(code, "ActionUnsupportedForEnvironment");
                }
                other => panic!("expected a disabled-gate error, got: {other:?}"),
            }
        }
        assert_eq!(presence.ensure_calls(), 0);
        assert_eq!(presence.release_calls(), 0);
        assert!(!*daemon.session_presence_held.lock().await);

        // Status stays available and reports honestly while disabled.
        match daemon
            .handle(ServiceRequest::SessionPresence {
                action: SessionPresenceAction::Status,
            })
            .await
        {
            ServiceResponse::SessionPresence { .. } => {}
            other => panic!("status should not be gated: {other:?}"),
        }
    }

    #[test]
    fn if_changed_reuses_previous_identical_model_capture_path() {
        let dir = unique_temp_dir("if-changed");
        let previous_path = dir.join("previous.jpg");
        let current_path = dir.join("current.jpg");
        std::fs::write(&previous_path, b"same model image").expect("write previous");
        std::fs::write(&current_path, b"same model image").expect("write current");

        let previous = snapshot(Some(capture_with_path(&previous_path)), Vec::new());
        let mut current = snapshot(Some(capture_with_path(&current_path)), Vec::new());

        assert!(reuse_unchanged_capture(&mut current, Some(&previous)));
        assert_eq!(
            current
                .capture
                .expect("capture")
                .screenshot_path
                .expect("path"),
            previous_path.display().to_string()
        );
    }

    #[test]
    fn if_changed_reuses_previous_cursor_capture_when_raw_sibling_matches() {
        let dir = unique_temp_dir("if-changed-agent-cursor");
        let raw_previous_path = dir.join("capture.png");
        let previous_path = dir.join("capture.agent-cursor.png");
        let current_path = dir.join("current.png");
        std::fs::write(&raw_previous_path, b"same raw model image").expect("write raw previous");
        std::fs::write(&previous_path, b"same raw model image plus cursor")
            .expect("write previous");
        std::fs::write(&current_path, b"same raw model image").expect("write current");

        let previous = snapshot(Some(capture_with_path(&previous_path)), Vec::new());
        let mut current = snapshot(Some(capture_with_path(&current_path)), Vec::new());

        assert!(reuse_unchanged_capture(&mut current, Some(&previous)));
        assert_eq!(
            current
                .capture
                .expect("capture")
                .screenshot_path
                .expect("path"),
            previous_path.display().to_string()
        );
    }

    #[test]
    fn if_changed_reuse_keeps_current_original_screenshot_path() {
        let dir = unique_temp_dir("if-changed-original-path");
        let previous_path = dir.join("previous.jpg");
        let previous_original_path = dir.join("previous-original.jpg");
        let current_path = dir.join("current.jpg");
        let current_original_path = dir.join("current-original.jpg");
        std::fs::write(&previous_path, b"same model image").expect("write previous");
        std::fs::write(&current_path, b"same model image").expect("write current");
        let mut previous_capture = capture_with_path(&previous_path);
        previous_capture.original_screenshot_path =
            Some(previous_original_path.display().to_string());
        let mut current_capture = capture_with_path(&current_path);
        current_capture.original_screenshot_path =
            Some(current_original_path.display().to_string());
        let previous = snapshot(Some(previous_capture), Vec::new());
        let mut current = snapshot(Some(current_capture), Vec::new());

        assert!(reuse_unchanged_capture(&mut current, Some(&previous)));

        let capture = current.capture.expect("capture");
        assert_eq!(
            capture.screenshot_path.as_deref(),
            Some(previous_path.to_str().expect("utf-8 previous path"))
        );
        assert_eq!(
            capture.original_screenshot_path.as_deref(),
            Some(current_original_path.to_str().expect("utf-8 current path"))
        );
    }

    #[test]
    fn if_changed_keeps_current_capture_when_image_changed() {
        let dir = unique_temp_dir("if-changed-different");
        let previous_path = dir.join("previous.jpg");
        let current_path = dir.join("current.jpg");
        std::fs::write(&previous_path, b"old model image").expect("write previous");
        std::fs::write(&current_path, b"new model image").expect("write current");

        let previous = snapshot(Some(capture_with_path(&previous_path)), Vec::new());
        let mut current = snapshot(Some(capture_with_path(&current_path)), Vec::new());

        assert!(!reuse_unchanged_capture(&mut current, Some(&previous)));
        assert_eq!(
            current
                .capture
                .expect("capture")
                .screenshot_path
                .expect("path"),
            current_path.display().to_string()
        );
    }

    fn window_info(window_id: &str, title: Option<&str>, pid: Option<u32>) -> WindowInfo {
        WindowInfo {
            window_id: window_id.to_string(),
            title: title.map(str::to_string),
            app_id: None,
            wm_class: None,
            pid,
            bounds: None,
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: false,
            hidden: false,
            client_type: None,
            backend: "test".to_string(),
            terminal: None,
        }
    }

    #[test]
    fn select_scrcpy_window_prefers_pid_over_title() {
        let windows = vec![
            window_info("w-other", Some("sky-cua-phone-dev1"), Some(10)),
            window_info("w-target", Some("some-other-title"), Some(42)),
        ];
        // pid 42 wins even though a different window carries the matching title.
        let picked = super::select_scrcpy_window(&windows, Some(42), "sky-cua-phone-dev1")
            .expect("pid match");
        assert_eq!(picked.window_id, "w-target");
    }

    #[test]
    fn select_scrcpy_window_falls_back_to_title() {
        let windows = vec![
            window_info("w-a", Some("unrelated"), Some(1)),
            window_info("w-b", Some("sky-cua-phone-dev1"), Some(2)),
        ];
        // No pid (or a pid that matches nothing) -> exact title match.
        let by_no_pid =
            super::select_scrcpy_window(&windows, None, "sky-cua-phone-dev1").expect("title match");
        assert_eq!(by_no_pid.window_id, "w-b");
        let by_missing_pid = super::select_scrcpy_window(&windows, Some(999), "sky-cua-phone-dev1")
            .expect("title fallback when pid matches nothing");
        assert_eq!(by_missing_pid.window_id, "w-b");
    }

    #[test]
    fn select_scrcpy_window_returns_none_without_a_match() {
        let windows = vec![window_info("w-a", Some("unrelated"), Some(1))];
        assert!(super::select_scrcpy_window(&windows, Some(42), "sky-cua-phone-dev1").is_none());
        assert!(super::select_scrcpy_window(&[], Some(42), "sky-cua-phone-dev1").is_none());
    }

    fn daemon_with(snapshot: AppStateSnapshot, outcome: ActionOutcome) -> ServiceDaemon {
        daemon_with_backend(Box::new(FakeBackend {
            snapshot,
            outcome,
            presence: None,
        }))
    }

    fn daemon_with_backend(backend: Box<dyn DesktopBackend>) -> ServiceDaemon {
        daemon_with_backend_and_presence_config(backend, SessionPresenceConfig::disabled())
    }

    fn daemon_with_backend_and_presence_config(
        backend: Box<dyn DesktopBackend>,
        session_presence_config: SessionPresenceConfig,
    ) -> ServiceDaemon {
        daemon_with_phone(backend, session_presence_config, test_phone_manager())
    }

    fn daemon_with_phone(
        backend: Box<dyn DesktopBackend>,
        session_presence_config: SessionPresenceConfig,
        phone: crate::phone::PhoneManager,
    ) -> ServiceDaemon {
        ServiceDaemon {
            backend,
            sessions: SessionStore::new(),
            snapshots: tokio::sync::Mutex::new(SnapshotManager::new(8)),
            overlay: tokio::sync::Mutex::new(OverlayController::new_for_tests()),
            phone: tokio::sync::Mutex::new(phone),
            session_presence_config,
            session_presence_held: tokio::sync::Mutex::new(false),
            desktop_lane: tokio::sync::Mutex::new(()),
            socket_path: PathBuf::from("/tmp/sky-cua-test.sock"),
        }
    }

    /// A `PhoneManager` backed by a deterministic, unscripted `FakeCommandRunner`,
    /// so the daemon's phone routing tests never shell out to a real `adb` that
    /// may or may not exist on the test host.
    fn test_phone_manager() -> crate::phone::PhoneManager {
        crate::phone::PhoneManager::with_fake_runner_for_tests()
    }

    impl BlockingBackend {
        fn new(snapshot: AppStateSnapshot) -> Self {
            Self {
                snapshot,
                outcome: success_outcome(),
                execute_calls: Arc::new(AtomicUsize::new(0)),
                first_execute_started: Arc::new(Notify::new()),
                second_execute_started: Arc::new(Notify::new()),
                release_first_execute: Arc::new(Notify::new()),
            }
        }
    }

    impl PresenceRecorder {
        fn record_ensure(&self, intent: SessionPresenceIntent) {
            self.ensure_calls.fetch_add(1, Ordering::SeqCst);
            *self.last_intent.lock().expect("last intent lock") = Some(intent);
        }

        fn record_release(&self, relock: bool) {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
            *self.last_relock.lock().expect("last relock lock") = Some(relock);
        }

        fn ensure_calls(&self) -> usize {
            self.ensure_calls.load(Ordering::SeqCst)
        }

        fn release_calls(&self) -> usize {
            self.release_calls.load(Ordering::SeqCst)
        }

        fn last_intent(&self) -> Option<SessionPresenceIntent> {
            *self.last_intent.lock().expect("last intent lock")
        }

        fn last_relock(&self) -> Option<bool> {
            *self.last_relock.lock().expect("last relock lock")
        }
    }

    fn success_outcome() -> ActionOutcome {
        ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        }
    }

    fn snapshot(capture: Option<CaptureInfo>, elements: Vec<ElementNode>) -> AppStateSnapshot {
        AppStateSnapshot {
            snapshot_id: "snap".to_string(),
            created_at: chrono::Utc::now(),
            environment: environment(),
            capabilities: available_capabilities(),
            focused_app: None,
            capture,
            elements,
            diagnostics: Vec::new(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        }
    }

    fn environment() -> EnvironmentInfo {
        EnvironmentInfo {
            session_kind: SessionKind::Wayland,
            compositor: Some("KWin".to_string()),
            desktop_environment: Some("KDE".to_string()),
            capture_backend: CaptureBackendKind::PortalPipeWire,
            input_backend: InputBackendKind::PortalRemoteDesktop,
            semantic_backend: SemanticBackendKind::Atspi,
            portal_capabilities: PortalCapabilities {
                screencast_version: Some(5),
                remote_desktop_version: Some(2),
                screenshot_version: Some(1),
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

    fn capture_with_rect() -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("stream".to_string()),
            source_type: Some(1),
            mapping_id: Some("mapping".to_string()),
            source_logical_rect: None,
            logical_rect: Some(RectF {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            pixel_size: Some(PixelSize {
                width: 400,
                height: 200,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }

    fn capture_with_path(path: &Path) -> CaptureInfo {
        let mut capture = capture_with_rect();
        capture.screenshot_path = Some(path.display().to_string());
        capture.model_image_format = None;
        capture
    }

    fn available_capabilities() -> ToolCapabilities {
        let available = || ToolAvailability {
            available: true,
            reason: None,
        };
        ToolCapabilities {
            list_apps: available(),
            get_app_state: available(),
            focus_element: available(),
            activate_element: available(),
            select_element: available(),
            expand_element: available(),
            collapse_element: available(),
            toggle_element: available(),
            click: available(),
            perform_action: available(),
            perform_secondary_action: available(),
            scroll: available(),
            supported_scroll_directions: vec![
                sky_cua_platform::model::ScrollDirection::Up,
                sky_cua_platform::model::ScrollDirection::Down,
            ],
            drag: available(),
            type_text: available(),
            press_key: available(),
            set_value: available(),
        }
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-daemon-agent-cursor-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
