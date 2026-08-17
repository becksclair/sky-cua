use atspi_connection::AccessibilityConnection;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    ActionOutcome, ActionRequest, AppSelector, AppStateSnapshot, CaptureBackendKind, CaptureInfo,
    CaptureScope, CaptureScreenMode, DiagnosticEntry, DisplayInfo, DisplayRef, DisplayTarget,
    DoctorDisplayTopologyReport, DoctorReport, ElementNode, EnvironmentInfo, FocusedApp,
    InputBackendKind, RectF, ScrollDirection, SemanticBackendKind, ToolAvailability,
    ToolCapabilities, WindowTarget,
};
use sky_cua_platform::{AppInfo, new_snapshot_id};

use crate::actions::runtime::{
    SemanticActionInvocation, SemanticAtspiAction, SemanticSetValueResult,
};
use crate::actions::{LinuxActionExecutor, runtime::LinuxActionRuntime, success_with_diagnostics};
use crate::app_match::{
    app_from_linux_window, best_x11_window_match, enrich_accessible_apps_from_windows,
    linux_window_matches_app, merge_app_lists, preferred_linux_window, select_app,
    select_linux_window, selector_match_score, selector_summary,
};
use crate::app_policy::{AppActionPolicies, ResolvedSetValueFallbackPolicy};
use crate::apps::discovery::{DiscoveredApp, discover_apps};
use crate::apps::window_correlation::{WindowAccessibilityMatch, match_window_accessibility};
use crate::atspi::{
    RepairCoordinator, actions as atspi_actions, connect_attempt, connect_with_repair,
    snapshot::{snapshot_for_app, snapshot_for_top_level},
};
use crate::env_probe::{probe_environment, require_supported_environment};
use crate::focus::pick_focused_app_with_fallback;
use crate::portal::remote_desktop::{
    MouseButton, PortalLifecycleEvent, RemoteDesktopSessionManager,
};
use crate::session_env;
use crate::session_presence::SessionPresenceManager;
use crate::virtual_input::{LinuxVirtualInput, virtual_input_keyboard_available};
use crate::windowing as linux_windowing;
use crate::x11::input_xtest::{self, X11MouseButton};
use crate::x11::windowing::{self, X11WindowInfo};
use sky_cua_platform::model::DoctorSessionEnvReport;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::warn;

const DISPLAY_TOPOLOGY_CACHE_TTL: Duration = Duration::from_secs(10);
const ENVIRONMENT_CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
struct DisplayTopologyCache {
    updated_at: Instant,
    displays: Vec<DisplayInfo>,
    report: DoctorDisplayTopologyReport,
}

#[derive(Debug, Clone)]
struct SessionEnvCache {
    report: DoctorSessionEnvReport,
    hydrated_at: Instant,
}

#[derive(Debug, Clone)]
pub struct LinuxDesktopBackend {
    portal: RemoteDesktopSessionManager,
    atspi: Arc<Mutex<Option<AccessibilityConnection>>>,
    atspi_repair: RepairCoordinator,
    app_policies: AppActionPolicies,
    session_env: Arc<StdMutex<SessionEnvCache>>,
    session_presence: SessionPresenceManager,
    virtual_input: Arc<OnceLock<LinuxVirtualInput>>,
    display_topology: Arc<StdMutex<Option<DisplayTopologyCache>>>,
    environment_cache: Arc<StdMutex<Option<(EnvironmentInfo, Instant)>>>,
}

impl Default for LinuxDesktopBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxDesktopBackend {
    #[must_use]
    pub fn new() -> Self {
        let session_env_report = session_env::hydrate_session_env();
        let session_env_hydrated_at = Instant::now();
        // Warm the KWin scripting-reachability cache on a plain thread at
        // construction so even callers that supply a pre-built environment
        // (bypassing probe_environment's awaited warmup) find the cache
        // seeded instead of running the cold-start gdbus probe inline on a
        // runtime worker. Gated on a cheap KDE session hint; the precise
        // environment-based gate still applies in probe_environment.
        if std::env::var("XDG_CURRENT_DESKTOP")
            .map(|desktop| desktop.to_ascii_lowercase().contains("kde"))
            .unwrap_or(false)
            || std::env::var_os("KDE_FULL_SESSION").is_some()
        {
            std::thread::spawn(crate::kwin::warm_scripting_probe);
        }
        Self {
            portal: RemoteDesktopSessionManager::new(),
            atspi: Arc::new(Mutex::new(None)),
            atspi_repair: RepairCoordinator::new(),
            app_policies: AppActionPolicies::load_from_repo().unwrap_or_else(|error| {
                warn!(
                    message = %error,
                    "failed to load app action policies; heuristics-driven set_value fallback will stay disabled"
                );
                AppActionPolicies::default()
            }),
            session_env: Arc::new(StdMutex::new(SessionEnvCache {
                report: session_env_report,
                hydrated_at: session_env_hydrated_at,
            })),
            session_presence: SessionPresenceManager::new(),
            virtual_input: Arc::new(OnceLock::new()),
            display_topology: Arc::new(StdMutex::new(None)),
            environment_cache: Arc::new(StdMutex::new(None)),
        }
    }

    async fn enrich_environment_displays(
        &self,
        environment: &mut EnvironmentInfo,
    ) -> DoctorDisplayTopologyReport {
        if !environment.displays.is_empty() {
            return crate::displays::display_topology_report_from_environment(environment);
        }
        if let Some(cached) = cached_display_topology(&self.display_topology, Instant::now()) {
            environment.displays = cached.displays;
            return cached.report;
        }

        let outcome = crate::displays::discover_display_topology(environment).await;
        store_display_topology(
            &self.display_topology,
            outcome.displays.clone(),
            outcome.report.clone(),
        );
        environment.displays = outcome.displays;
        outcome.report
    }

    async fn probe_environment_base(&self) -> Result<EnvironmentInfo, BackendError> {
        let now = Instant::now();
        if let Some((cached, cached_at)) = self.environment_cache.lock().unwrap().as_ref()
            && now.duration_since(*cached_at) < ENVIRONMENT_CACHE_TTL
        {
            return Ok(cached.clone());
        }
        self.refresh_session_env_if_stale(now);
        let mut environment = probe_environment().await?;
        environment.semantic_backend = if require_supported_environment(&environment).is_ok()
            && self.accessibility_connection().await.is_ok()
        {
            SemanticBackendKind::Atspi
        } else {
            SemanticBackendKind::None
        };
        *self.environment_cache.lock().unwrap() = Some((environment.clone(), now));
        // Warm the KWin scripting-reachability cache off-worker once, awaited
        // so the cache is guaranteed seeded before any downstream sync
        // capability probe could run the cold-start gdbus subprocess on a
        // runtime worker. After the first call this returns immediately.
        if crate::kwin::kwin_window_query_available(&environment) {
            static SCRIPTING_PROBE_WARMUP: tokio::sync::OnceCell<()> =
                tokio::sync::OnceCell::const_new();
            SCRIPTING_PROBE_WARMUP
                .get_or_init(|| async {
                    let _ = tokio::task::spawn_blocking(crate::kwin::warm_scripting_probe).await;
                })
                .await;
        }
        Ok(environment)
    }

    async fn probe_environment_with_display_report(
        &self,
    ) -> Result<(EnvironmentInfo, DoctorDisplayTopologyReport), BackendError> {
        let mut environment = self.probe_environment_base().await?;
        let display_topology = self.enrich_environment_displays(&mut environment).await;
        Ok((environment, display_topology))
    }

    fn cached_virtual_input(&self) -> Result<&LinuxVirtualInput, BackendError> {
        if let Some(vi) = self.virtual_input.get() {
            return Ok(vi);
        }
        let vi = LinuxVirtualInput::new()?;
        let _ = self.virtual_input.set(vi);
        self.virtual_input.get().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "virtual_input cache was not set after initialization".to_string(),
            )
        })
    }

    fn session_env_report(&self) -> DoctorSessionEnvReport {
        self.session_env
            .lock()
            .map(|cache| cache.report.clone())
            .unwrap_or_default()
    }

    fn refresh_session_env_if_stale(&self, now: Instant) -> DoctorSessionEnvReport {
        self.refresh_session_env_if_stale_with(now, session_env::hydrate_session_env)
    }

    fn refresh_session_env_if_stale_with(
        &self,
        now: Instant,
        hydrate: impl FnOnce() -> DoctorSessionEnvReport,
    ) -> DoctorSessionEnvReport {
        if let Ok(mut cache) = self.session_env.lock() {
            if now
                .checked_duration_since(cache.hydrated_at)
                .is_some_and(|age| age < ENVIRONMENT_CACHE_TTL)
            {
                return cache.report.clone();
            }
            let latest = hydrate();
            merge_session_env_reports(&mut cache.report, latest);
            cache.hydrated_at = now;
            return cache.report.clone();
        }
        hydrate()
    }

    async fn accessibility_connection(&self) -> Result<AccessibilityConnection, BackendError> {
        let mut guard = self.atspi.lock().await;
        if let Some(connection) = guard.as_ref() {
            return Ok(connection.clone());
        }

        let connection =
            connect_with_repair(&self.atspi_repair, connect_attempt, connect_attempt).await?;
        *guard = Some(connection.clone());
        Ok(connection)
    }

    async fn reset_accessibility_connection(&self) {
        let mut guard = self.atspi.lock().await;
        *guard = None;
    }

    async fn focus_window_target_for_keyboard(
        &self,
        request: &ActionRequest,
    ) -> Result<Option<linux_windowing::LinuxWindowInfo>, BackendError> {
        let Some(target) = window_target_from_arguments(&request.arguments)? else {
            return Ok(None);
        };
        let probed_environment;
        let environment = match request.environment.as_ref() {
            Some(environment) => environment,
            None => {
                probed_environment = self.probe_environment().await?;
                &probed_environment
            }
        };
        require_supported_environment(environment)?;
        let windows = linux_windowing::discover_activation_windows(environment).await?;
        let target_window = linux_windowing::resolve_window_target(&windows, &target.into())?;

        if target_window.backend == "x11" {
            if !input_xtest::xtest_is_available() {
                return Err(BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    "targeted X11 keyboard input requires XTest/xdotool window activation",
                ));
            }
            input_xtest::window_activate(&target_window.window_id)?;
            return Ok(Some(target_window.clone()));
        }

        if environment.input_backend == InputBackendKind::None {
            return Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!(
                    "matched {} window {}, but no session input backend is available after native activation",
                    target_window.backend, target_window.window_id
                ),
            ));
        }

        linux_windowing::activate_window(target_window).await?;
        let _ = linux_windowing::verify_window_focused(environment, target_window).await?;
        Ok(Some(target_window.clone()))
    }

    async fn discover_accessible_apps(
        &self,
    ) -> Result<(AccessibilityConnection, Vec<DiscoveredApp>), BackendError> {
        // Generic app discovery never consumes per-app top levels; see
        // `discover_apps` for why top-level enumeration is correlation-only.
        self.discover_accessible_apps_for_window_pid(None, false)
            .await
    }

    async fn discover_accessible_apps_for_window_pid(
        &self,
        window_pid: Option<u32>,
        collect_top_levels: bool,
    ) -> Result<(AccessibilityConnection, Vec<DiscoveredApp>), BackendError> {
        let connection = self.accessibility_connection().await?;
        match self
            .at_spi_call_with_timeout(discover_apps(&connection, window_pid, collect_top_levels))
            .await
        {
            Ok(apps) => Ok((connection, apps)),
            Err(error) if is_retryable_accessibility_error(&error) => {
                self.reset_accessibility_connection().await;
                let connection = self.accessibility_connection().await?;
                let apps = self
                    .at_spi_call_with_timeout(discover_apps(
                        &connection,
                        window_pid,
                        collect_top_levels,
                    ))
                    .await?;
                Ok((connection, apps))
            }
            Err(error) => Err(error),
        }
    }

    /// Bound a single AT-SPI zbus call (app discovery walk, per-app element
    /// snapshot) to [`at_spi_walk_timeout`] as defense in depth alongside the
    /// server-side desktop request deadline in `sky-cua-service`. zbus 5.14
    /// has no default method timeout, so an unresponsive AT-SPI bus can hang
    /// a call forever; on elapse this drops the awaited future (abandoning a
    /// pure read — safe, nothing here mutates persisted state) and resets
    /// the cached connection so the next call reconnects instead of reusing
    /// a connection that may be talking to a wedged peer.
    async fn at_spi_call_with_timeout<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, BackendError>>,
    ) -> Result<T, BackendError> {
        match tokio::time::timeout(at_spi_walk_timeout(), future).await {
            Ok(result) => result,
            Err(_) => {
                self.reset_accessibility_connection().await;
                Err(BackendError::new(
                    BackendErrorCode::AccessibilityUnavailable,
                    format!(
                        "AT-SPI call exceeded the {:?} walk timeout and was abandoned; the accessibility connection was reset",
                        at_spi_walk_timeout()
                    ),
                ))
            }
        }
    }

    fn capabilities(environment: &EnvironmentInfo) -> ToolCapabilities {
        let semantic_ready = environment.semantic_backend == SemanticBackendKind::Atspi;
        let window_listing_ready = linux_windowing::probe_backends(environment)
            .iter()
            .any(|probe| probe.can_list_windows);
        let physical_ready = environment.input_backend != InputBackendKind::None;
        let keyboard_ready = keyboard_input_ready(environment);

        ToolCapabilities {
            list_apps: ToolAvailability {
                available: semantic_ready || window_listing_ready,
                reason: (!(semantic_ready || window_listing_ready))
                    .then(|| "Neither AT-SPI nor a window-query fallback is available".to_string()),
            },
            get_app_state: ToolAvailability {
                available: semantic_ready || window_listing_ready,
                reason: (!(semantic_ready || window_listing_ready))
                    .then(|| "Neither AT-SPI nor a window-query fallback is available".to_string()),
            },
            focus_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            activate_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            select_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            expand_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            collapse_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            toggle_element: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            click: ToolAvailability {
                available: semantic_ready || physical_ready,
                reason: (!(semantic_ready || physical_ready))
                    .then(|| "No semantic or physical input backend is available".to_string()),
            },
            perform_action: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready).then(|| "AT-SPI is unavailable".to_string()),
            },
            perform_secondary_action: ToolAvailability {
                available: physical_ready || semantic_ready,
                reason: (!(physical_ready || semantic_ready))
                    .then(|| "No semantic or physical input backend is available".to_string()),
            },
            scroll: ToolAvailability {
                available: physical_ready,
                reason: (!physical_ready)
                    .then(|| "No physical input backend is available".to_string()),
            },
            supported_scroll_directions: vec![
                ScrollDirection::Up,
                ScrollDirection::Down,
                ScrollDirection::Left,
                ScrollDirection::Right,
            ],
            drag: ToolAvailability {
                available: physical_ready,
                reason: (!physical_ready)
                    .then(|| "No physical input backend is available".to_string()),
            },
            type_text: ToolAvailability {
                available: keyboard_ready,
                reason: (!keyboard_ready)
                    .then(|| keyboard_input_unavailable_reason(environment).to_string()),
            },
            press_key: ToolAvailability {
                available: keyboard_ready,
                reason: (!keyboard_ready)
                    .then(|| keyboard_input_unavailable_reason(environment).to_string()),
            },
            set_value: ToolAvailability {
                available: semantic_ready,
                reason: (!semantic_ready)
                    .then(|| "AT-SPI semantic editing interfaces are unavailable".to_string()),
            },
        }
    }

    fn focused_from_app(app: &AppInfo) -> FocusedApp {
        FocusedApp {
            app_id: app.app_id.clone(),
            name: app.name.clone(),
            pid: app.pid,
            desktop_file_id: app.desktop_file_id.clone(),
            app_user_model_id: app.app_user_model_id.clone(),
            window_handle: app.window_handle.clone(),
            toolkit_guess: app.toolkit_guess.clone(),
            window_title: app.window_title.clone(),
            display: None,
        }
    }

    fn focused_from_linux_window(window: &linux_windowing::LinuxWindowInfo) -> FocusedApp {
        let app = app_from_linux_window(window);
        FocusedApp {
            display: window.display.clone(),
            ..Self::focused_from_app(&app)
        }
    }

    async fn get_app_state_capture(
        &self,
        snapshot_id: &str,
        capture_screen: CaptureScreenMode,
        environment: &EnvironmentInfo,
        target_window: Option<&linux_windowing::LinuxWindowInfo>,
        diagnostics: &mut DiagnosticBuilder,
    ) -> Result<Option<CaptureInfo>, BackendError> {
        if capture_screen == CaptureScreenMode::Never {
            return Ok(None);
        }
        let candidates = get_app_state_capture_candidates(environment, target_window, diagnostics);
        if candidates.is_empty() {
            diagnostics.push_code(
                "GetAppStateCaptureUnavailable",
                "get_app_state did not attach a screenshot because no target window, target display, or primary display geometry was available.",
                Some("Refresh desktop/window state, then call capture_desktop (optionally with a window or display selector) to capture a single screen.".to_string()),
            );
            return Ok(None);
        }

        for (index, candidate) in candidates.iter().enumerate() {
            if index > 0 {
                diagnostics.push_code(
                    "GetAppStateCaptureScopeFallback",
                    format!(
                        "get_app_state could not use a narrower capture target; trying {} capture.",
                        candidate.label
                    ),
                    None,
                );
            }

            let mut capture_plan = match crate::capture_plan::plan_capture(
                &self.portal,
                snapshot_id,
                capture_screen,
                environment,
                Some(&candidate.target),
                candidate.target.capture_scope.clone(),
                candidate.target.display.clone(),
                true,
                diagnostics,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(error) if crate::capture_plan::is_capture_source_geometry_missing(&error) => {
                    diagnostics.push_code(
                        "CaptureSourceGeometryRetry",
                        "RemoteDesktop capture source geometry was missing; resetting the capture session and retrying the scoped get_app_state screenshot once",
                        Some(error.message.clone()),
                    );
                    self.portal.reset_session().await;
                    match crate::capture_plan::plan_capture(
                        &self.portal,
                        snapshot_id,
                        capture_screen,
                        environment,
                        Some(&candidate.target),
                        candidate.target.capture_scope.clone(),
                        candidate.target.display.clone(),
                        false,
                        diagnostics,
                    )
                    .await
                    {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            diagnostics.push_code(
                                error.code,
                                "Scoped get_app_state screenshot capture failed",
                                Some(error.message),
                            );
                            let mut events = self.portal.take_lifecycle_events().await;
                            push_portal_lifecycle_diagnostics(&mut events, diagnostics);
                            continue;
                        }
                    }
                }
                Err(error) => {
                    diagnostics.push_code(
                        error.code,
                        "Scoped get_app_state screenshot capture failed",
                        Some(error.message),
                    );
                    let mut events = self.portal.take_lifecycle_events().await;
                    push_portal_lifecycle_diagnostics(&mut events, diagnostics);
                    continue;
                }
            };

            if crate::capture_plan::outcome_missing_capture_source_geometry(&capture_plan)
                && environment.input_backend == InputBackendKind::PortalRemoteDesktop
            {
                diagnostics.push_code(
                    "CaptureSourceGeometryRetry",
                    "RemoteDesktop capture source geometry was missing; resetting the capture session and retrying the scoped get_app_state screenshot once",
                    capture_plan
                        .capture_error
                        .as_ref()
                        .map(|error| error.message.clone()),
                );
                self.portal.reset_session().await;
                capture_plan = match crate::capture_plan::plan_capture(
                    &self.portal,
                    snapshot_id,
                    capture_screen,
                    environment,
                    Some(&candidate.target),
                    candidate.target.capture_scope.clone(),
                    candidate.target.display.clone(),
                    false,
                    diagnostics,
                )
                .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        diagnostics.push_code(
                            error.code,
                            "Scoped get_app_state screenshot capture failed",
                            Some(error.message),
                        );
                        let mut events = self.portal.take_lifecycle_events().await;
                        push_portal_lifecycle_diagnostics(&mut events, diagnostics);
                        continue;
                    }
                };
            }

            let mut events = self.portal.take_lifecycle_events().await;
            crate::capture_plan::push_diagnostics(
                environment,
                capture_plan.capture.as_ref(),
                capture_plan.portal_session_error.as_ref(),
                capture_plan.capture_error.as_ref(),
                diagnostics,
            );
            push_portal_lifecycle_diagnostics(&mut events, diagnostics);

            if let Err(error) = reject_unactionable_targeted_capture(
                Some(&candidate.target),
                &capture_plan,
                environment,
            ) {
                diagnostics.push_code(
                    error.code,
                    "Scoped get_app_state screenshot was not actionable",
                    Some(error.message),
                );
                continue;
            }

            if capture_plan
                .capture
                .as_ref()
                .and_then(|capture| capture.screenshot_path.as_deref())
                .is_some_and(|path| !path.trim().is_empty())
            {
                return Ok(capture_plan.capture);
            }
        }

        diagnostics.push_code(
            "GetAppStateCaptureUnavailable",
            "get_app_state did not attach a screenshot because no scoped window/display capture could be produced.",
            Some("Use capture_desktop(window_id=...) or capture_desktop(display_id=...) for explicit single-screen visual capture.".to_string()),
        );
        Ok(None)
    }
}

mod action_runtime;
mod desktop_backend;
mod elements;
pub(crate) use elements::{dimensions_approximately_match, near_zero};

#[cfg(test)]
mod tests;

struct GetAppStateCaptureCandidate {
    target: crate::capture_plan::CaptureRegionTarget,
    label: &'static str,
}

fn get_app_state_capture_candidates(
    environment: &EnvironmentInfo,
    target_window: Option<&linux_windowing::LinuxWindowInfo>,
    diagnostics: &mut DiagnosticBuilder,
) -> Vec<GetAppStateCaptureCandidate> {
    let mut candidates = Vec::new();
    let mut display_candidate_id: Option<String> = None;

    if let Some(window) = target_window {
        if let Some(bounds) = window.bounds.clone() {
            candidates.push(GetAppStateCaptureCandidate {
                target: crate::capture_plan::CaptureRegionTarget {
                    desktop_logical_rect: bounds,
                    capture_scope: CaptureScope::Window,
                    display: window.display.clone(),
                },
                label: "window",
            });
        } else {
            diagnostics.push_code(
                "GetAppStateCaptureScopeFallback",
                format!(
                    "Selected {} window {} did not report bounds; trying display-scoped get_app_state capture.",
                    window.backend, window.window_id
                ),
                None,
            );
        }

        if let Some(display_ref) = &window.display {
            if let Some(display) = display_for_ref(environment, display_ref) {
                display_candidate_id = Some(display.display_id.clone());
                candidates.push(display_candidate(display, CaptureScope::Display, "display"));
            } else {
                diagnostics.push_code(
                    "GetAppStateCaptureScopeFallback",
                    format!(
                        "Selected window display {} was not present in environment.displays; trying primary-display capture.",
                        display_ref.display_id
                    ),
                    None,
                );
            }
        }
    }

    if let Some(primary) = crate::displays::primary_display(&environment.displays)
        && display_candidate_id.as_deref() != Some(primary.display_id.as_str())
    {
        candidates.push(display_candidate(
            &primary,
            CaptureScope::PrimaryDisplay,
            "primary display",
        ));
    }

    candidates
}

fn display_for_ref<'a>(
    environment: &'a EnvironmentInfo,
    display_ref: &DisplayRef,
) -> Option<&'a DisplayInfo> {
    environment
        .displays
        .iter()
        .find(|display| display.display_id == display_ref.display_id)
}

fn display_candidate(
    display: &DisplayInfo,
    capture_scope: CaptureScope,
    label: &'static str,
) -> GetAppStateCaptureCandidate {
    let display_ref = DisplayRef::from(display);
    GetAppStateCaptureCandidate {
        target: crate::capture_plan::CaptureRegionTarget {
            desktop_logical_rect: display.logical_rect.clone(),
            capture_scope,
            display: Some(display_ref),
        },
        label,
    }
}

fn merge_session_env_reports(current: &mut DoctorSessionEnvReport, latest: DoctorSessionEnvReport) {
    for repair in latest.repaired {
        if !current.repaired.contains(&repair) {
            current.repaired.push(repair);
        }
    }
    current.path_changed |= latest.path_changed;
    if latest.final_path.is_some() {
        current.final_path = latest.final_path;
    }
    for note in latest.notes {
        if !current.notes.contains(&note) {
            current.notes.push(note);
        }
    }
}

fn cached_display_topology(
    cache: &Arc<StdMutex<Option<DisplayTopologyCache>>>,
    now: Instant,
) -> Option<DisplayTopologyCache> {
    let cache = cache.lock().ok()?;
    let cached = cache.as_ref()?;
    now.checked_duration_since(cached.updated_at)
        .is_some_and(|age| age <= DISPLAY_TOPOLOGY_CACHE_TTL)
        .then(|| cached.clone())
}

fn store_display_topology(
    cache: &Arc<StdMutex<Option<DisplayTopologyCache>>>,
    displays: Vec<DisplayInfo>,
    report: DoctorDisplayTopologyReport,
) {
    if let Ok(mut cache) = cache.lock() {
        *cache = Some(DisplayTopologyCache {
            updated_at: Instant::now(),
            displays,
            report,
        });
    }
}

fn require_screenshot_image(
    capture: Option<&sky_cua_platform::model::CaptureInfo>,
    portal_session_error: Option<&BackendError>,
    capture_error: Option<&BackendError>,
) -> Result<(), BackendError> {
    if capture
        .and_then(|capture| capture.screenshot_path.as_deref())
        .is_some_and(|path| !path.trim().is_empty())
    {
        return Ok(());
    }
    if let Some(error) = capture_error.or(portal_session_error) {
        return Err(BackendError {
            code: error.code,
            message: error.message.clone(),
        });
    }
    Err(BackendError::new(
        BackendErrorCode::Internal,
        "screenshot capture did not produce an image",
    ))
}

fn reject_unactionable_targeted_capture(
    capture_target: Option<&crate::capture_plan::CaptureRegionTarget>,
    capture_plan: &crate::capture_plan::CapturePlanOutcome,
    environment: &EnvironmentInfo,
) -> Result<(), BackendError> {
    if capture_target.is_none() {
        return Ok(());
    }
    let screenshot_fallback_without_source_geometry =
        capture_plan.capture.as_ref().is_some_and(|capture| {
            capture.screenshot_path.is_some()
                && capture.image_backend == Some(CaptureBackendKind::PortalScreenshot)
                && capture.source_logical_rect.is_none()
        });
    if screenshot_fallback_without_source_geometry {
        if environment.input_backend == InputBackendKind::PortalRemoteDesktop {
            return Err(BackendError::new(
                BackendErrorCode::CaptureSourceGeometryMissing,
                "targeted screenshot produced an image without capture source geometry for subsequent pixel actions",
            ));
        }
        // LinuxVirtualInput and other non-portal input backends can act on the
        // fallback screenshot using its own pixel_size, so the missing
        // source_logical_rect is not fatal.
        return Ok(());
    }
    if !crate::capture_plan::outcome_missing_capture_source_geometry(capture_plan) {
        return Ok(());
    }
    if let Some(error) = capture_plan.capture_error.as_ref() {
        return Err(BackendError {
            code: error.code,
            message: error.message.clone(),
        });
    }
    Err(BackendError::new(
        BackendErrorCode::CaptureSourceGeometryMissing,
        "targeted screenshot produced an image without capture source geometry for subsequent pixel actions",
    ))
}

fn is_retryable_accessibility_error(error: &BackendError) -> bool {
    error.code == BackendErrorCode::AccessibilityUnavailable.as_str()
        && error.message.contains("Resource temporarily unavailable")
}

/// Deadline for a single AT-SPI zbus call (app discovery, element snapshot).
/// Overridable via `SKY_CUA_AT_SPI_WALK_TIMEOUT_MS` so tests can exercise the
/// timeout path without waiting out the production default.
fn at_spi_walk_timeout() -> Duration {
    static TIMEOUT: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        std::env::var("SKY_CUA_AT_SPI_WALK_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(10))
    })
}

fn keyboard_input_ready(environment: &EnvironmentInfo) -> bool {
    match environment.input_backend {
        InputBackendKind::PortalRemoteDesktop | InputBackendKind::XTest => true,
        InputBackendKind::LinuxVirtualInput => virtual_input_keyboard_available(),
        InputBackendKind::None
        | InputBackendKind::SendInput
        | InputBackendKind::WindowsMessages => false,
    }
}

fn keyboard_input_unavailable_reason(environment: &EnvironmentInfo) -> &'static str {
    match environment.input_backend {
        InputBackendKind::LinuxVirtualInput => {
            "Linux virtual input keyboard actions require the privileged input helper or a usable ydotool daemon"
        }
        InputBackendKind::None => "No physical input backend is available",
        InputBackendKind::SendInput | InputBackendKind::WindowsMessages => {
            "Windows input backends are unavailable on Linux"
        }
        InputBackendKind::PortalRemoteDesktop | InputBackendKind::XTest => {
            unreachable!(
                "keyboard_input_ready returns true for PortalRemoteDesktop and XTest, \
                 so keyboard_input_unavailable_reason should never be called for them"
            )
        }
    }
}

fn window_target_from_arguments(
    arguments: &serde_json::Value,
) -> Result<Option<sky_cua_platform::model::WindowTarget>, BackendError> {
    sky_cua_platform::model::WindowTarget::from_argument_fields(arguments).map_err(|error| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("invalid window target arguments: {error}"),
        )
    })
}

fn push_portal_lifecycle_diagnostics(
    events: &mut Vec<PortalLifecycleEvent>,
    diagnostics: &mut DiagnosticBuilder,
) {
    for event in events.drain(..) {
        diagnostics.push_code(event.code, event.message, event.details);
    }
}
