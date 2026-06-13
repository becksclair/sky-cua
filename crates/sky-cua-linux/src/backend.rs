use atspi::AccessibilityConnection;
use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    ActionOutcome, ActionRequest, AppSelector, AppStateSnapshot, CaptureScope, CaptureScreenMode,
    DiagnosticEntry, DisplayInfo, DisplayTarget, DoctorReport, ElementNode, EnvironmentInfo,
    FocusedApp, InputBackendKind, RectF, ScrollDirection, SemanticBackendKind, ToolAvailability,
    ToolCapabilities, WindowTarget,
};
use sky_cua_platform::{AppInfo, new_snapshot_id};

use crate::actions::runtime::{
    SemanticActionInvocation, SemanticAtspiAction, SemanticSetValueResult,
};
use crate::actions::{LinuxActionExecutor, runtime::LinuxActionRuntime};
use crate::app_match::{
    app_from_linux_window, best_x11_window_match, enrich_accessible_apps_from_windows,
    linux_window_matches_app, merge_app_lists, preferred_linux_window, select_app,
    select_linux_window, selector_match_score, selector_summary,
};
use crate::app_policy::{AppActionPolicies, ResolvedSetValueFallbackPolicy};
use crate::apps::discovery::{DiscoveredApp, discover_apps};
use crate::atspi::{actions as atspi_actions, connect, snapshot::snapshot_for_app};
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

const DISPLAY_TOPOLOGY_CACHE_TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
struct DisplayTopologyCache {
    updated_at: Instant,
    displays: Vec<DisplayInfo>,
}

#[derive(Debug, Clone)]
pub struct LinuxDesktopBackend {
    portal: RemoteDesktopSessionManager,
    atspi: Arc<Mutex<Option<AccessibilityConnection>>>,
    app_policies: AppActionPolicies,
    session_env: Arc<StdMutex<DoctorSessionEnvReport>>,
    session_presence: SessionPresenceManager,
    virtual_input: Arc<OnceLock<LinuxVirtualInput>>,
    display_topology: Arc<StdMutex<Option<DisplayTopologyCache>>>,
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
            app_policies: AppActionPolicies::load_from_repo().unwrap_or_else(|error| {
                warn!(
                    message = %error,
                    "failed to load app action policies; heuristics-driven set_value fallback will stay disabled"
                );
                AppActionPolicies::default()
            }),
            session_env: Arc::new(StdMutex::new(session_env_report)),
            session_presence: SessionPresenceManager::new(),
            virtual_input: Arc::new(OnceLock::new()),
            display_topology: Arc::new(StdMutex::new(None)),
        }
    }

    async fn enrich_environment_displays(&self, environment: &mut EnvironmentInfo) {
        if !environment.displays.is_empty() {
            return;
        }
        if let Some(displays) = cached_display_topology(&self.display_topology, Instant::now()) {
            environment.displays = displays;
            return;
        }

        let displays = crate::displays::discover_displays(environment).await;
        store_display_topology(&self.display_topology, displays.clone());
        environment.displays = displays;
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
            .map(|report| report.clone())
            .unwrap_or_default()
    }

    fn refresh_session_env(&self) -> DoctorSessionEnvReport {
        let latest = session_env::hydrate_session_env();
        if let Ok(mut report) = self.session_env.lock() {
            merge_session_env_reports(&mut report, latest);
            return report.clone();
        }
        latest
    }

    async fn accessibility_connection(&self) -> Result<AccessibilityConnection, BackendError> {
        let mut guard = self.atspi.lock().await;
        if let Some(connection) = guard.as_ref() {
            return Ok(connection.clone());
        }

        let connection = connect().await?;
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
        let connection = self.accessibility_connection().await?;
        match discover_apps(&connection).await {
            Ok(apps) => Ok((connection, apps)),
            Err(error) if is_retryable_accessibility_error(&error) => {
                self.reset_accessibility_connection().await;
                let connection = self.accessibility_connection().await?;
                let apps = discover_apps(&connection).await?;
                Ok((connection, apps))
            }
            Err(error) => Err(error),
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
            supported_scroll_directions: vec![ScrollDirection::Up, ScrollDirection::Down],
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
) -> Option<Vec<DisplayInfo>> {
    let cache = cache.lock().ok()?;
    let cached = cache.as_ref()?;
    now.checked_duration_since(cached.updated_at)
        .is_some_and(|age| age <= DISPLAY_TOPOLOGY_CACHE_TTL)
        .then(|| cached.displays.clone())
}

fn store_display_topology(
    cache: &Arc<StdMutex<Option<DisplayTopologyCache>>>,
    displays: Vec<DisplayInfo>,
) {
    if let Ok(mut cache) = cache.lock() {
        *cache = Some(DisplayTopologyCache {
            updated_at: Instant::now(),
            displays,
        });
    }
}

#[async_trait::async_trait]
impl DesktopBackend for LinuxDesktopBackend {
    async fn prepare_automation_permissions(&self) -> Result<(), BackendError> {
        self.portal.preauthorize_permissions().await;
        Ok(())
    }

    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        self.refresh_session_env();
        let mut environment = probe_environment().await?;
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
        environment.semantic_backend = if require_supported_environment(&environment).is_ok()
            && self.accessibility_connection().await.is_ok()
        {
            SemanticBackendKind::Atspi
        } else {
            SemanticBackendKind::None
        };
        self.enrich_environment_displays(&mut environment).await;
        Ok(environment)
    }

    async fn doctor(&self) -> Result<sky_cua_platform::model::DoctorReport, BackendError> {
        let environment = self.probe_environment().await?;
        let session_presence = self.session_presence.doctor_report().await;
        Ok(crate::doctor::build_doctor_report_with_session_presence(
            environment,
            self.session_env_report(),
            Some(session_presence),
        ))
    }

    async fn setup_accessibility(
        &self,
    ) -> Result<sky_cua_platform::model::AccessibilitySetupReport, BackendError> {
        crate::setup::setup_accessibility_report(|| async { self.doctor().await }).await
    }

    async fn setup_window_targeting(
        &self,
    ) -> Result<sky_cua_platform::model::WindowTargetingSetupReport, BackendError> {
        Ok(crate::setup::setup_window_targeting_report().await)
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        let registry_windows = linux_windowing::discover_app_windows(&environment)
            .await
            .unwrap_or_default();
        let mut atspi_apps = match self.discover_accessible_apps().await {
            Ok((_, apps)) => apps,
            Err(error) => {
                if registry_windows.is_empty() {
                    return Err(error);
                }
                Vec::new()
            }
        };
        enrich_accessible_apps_from_windows(&mut atspi_apps, &registry_windows);
        Ok(merge_app_lists(&atspi_apps, &registry_windows))
    }

    fn session_env_diagnostics(&self) -> Vec<DiagnosticEntry> {
        let report = self.session_env_report();
        session_env::session_env_diagnostic(&report)
            .into_iter()
            .collect()
    }

    async fn list_windows(&self) -> Result<Vec<sky_cua_platform::model::WindowInfo>, BackendError> {
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        linux_windowing::discover_windows(&environment)
            .await
            .map(|windows| windows.into_iter().map(Into::into).collect())
    }

    async fn focused_window(
        &self,
    ) -> Result<Option<sky_cua_platform::model::WindowInfo>, BackendError> {
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        if let Some(window) = linux_windowing::focused_window_override() {
            let mut windows = vec![window];
            crate::displays::assign_window_displays(&mut windows, &environment.displays);
            return Ok(windows.pop().map(Into::into));
        }
        let windows = linux_windowing::discover_windows(&environment).await?;
        if let Some(window) = windows.iter().find(|window| window.focused) {
            return Ok(Some(window.clone().into()));
        }
        Ok(None)
    }

    async fn activate_window(
        &self,
        target: sky_cua_platform::model::WindowTarget,
    ) -> Result<ActionOutcome, BackendError> {
        if !target.has_target() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "activate_window requires one of window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd",
            ));
        }
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        let windows = linux_windowing::discover_activation_windows(&environment).await?;
        let window = linux_windowing::resolve_window_target(&windows, &target.into())?;
        linux_windowing::activate_window(window).await?;
        let focused = linux_windowing::verify_window_focused(&environment, window).await?;
        Ok(success_with_diagnostics(
            format!("Activated {} window {}.", window.backend, window.window_id),
            vec![DiagnosticEntry {
                code: "WindowFocusVerified".to_string(),
                message: format!(
                    "Focus verification matched {} window {}.",
                    focused.backend, focused.window_id
                ),
                details: None,
            }],
        ))
    }

    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        let snapshot_id = new_snapshot_id();
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        let capabilities = Self::capabilities(&environment);
        let session_presence = self.session_presence.doctor_report().await;
        let doctor_report = crate::doctor::build_doctor_report_with_session_presence(
            environment.clone(),
            self.session_env_report(),
            Some(session_presence),
        );
        let mut diagnostics = DiagnosticBuilder::new();
        if let Some(diagnostic) = doctor_report
            .session_env
            .as_ref()
            .and_then(session_env::session_env_diagnostic)
        {
            diagnostics.push_code(diagnostic.code, diagnostic.message, diagnostic.details);
        }
        if !doctor_report.readiness.can_build_accessibility_tree {
            diagnostics.push(
                BackendErrorCode::AccessibilityUnavailable,
                "Semantic accessibility is unavailable; Computer Use will fall back to window and screenshot anchors where possible.",
                Some(doctor_report.readiness.recommended_next_step.clone()),
            );
        }
        let registry_windows = linux_windowing::discover_app_windows(&environment)
            .await
            .unwrap_or_default();

        let capture_plan = crate::capture_plan::plan_capture(
            &self.portal,
            &snapshot_id,
            capture_screen,
            &environment,
            None,
            CaptureScope::Unknown,
            None,
            &mut diagnostics,
        )
        .await?;
        let capture = capture_plan.capture;
        let portal_session_error = capture_plan.portal_session_error;
        let capture_error = capture_plan.capture_error;
        let mut portal_lifecycle_events = self.portal.take_lifecycle_events().await;

        let (connection, mut apps) = match self.discover_accessible_apps().await {
            Ok(result) => result,
            Err(error) => {
                diagnostics.push(
                    BackendErrorCode::AccessibilityUnavailable,
                    error.message.clone(),
                    None,
                );
                let fallback_window = selector
                    .as_ref()
                    .and_then(|selector| select_linux_window(&registry_windows, selector))
                    .or_else(|| preferred_linux_window(&registry_windows));
                crate::capture_plan::push_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&mut portal_lifecycle_events, &mut diagnostics);
                if let Some(window) = fallback_window {
                    let app = app_from_linux_window(&window);
                    diagnostics.push(
                        BackendErrorCode::AccessibilityCoverageLimited,
                        format!(
                            "The selected {} window is visible through the window registry, but no AT-SPI application tree was available for it",
                            window.backend
                        ),
                        Some(selector_or_window_summary(selector.as_ref(), &app)),
                    );
                    return Ok(linux_fallback_snapshot(
                        snapshot_id,
                        environment,
                        capabilities,
                        capture,
                        diagnostics,
                        Some(doctor_report),
                        window,
                    ));
                }
                return Ok(AppStateSnapshot {
                    snapshot_id,
                    created_at: chrono::Utc::now(),
                    environment,
                    capabilities,
                    focused_app: None,
                    capture,
                    elements: Vec::new(),
                    diagnostics: diagnostics.finish(),
                    app_guidance: None,
                    doctor_report: Some(doctor_report),
                    agent_cursor: None,
                });
            }
        };

        enrich_accessible_apps_from_windows(&mut apps, &registry_windows);
        if apps.is_empty() {
            diagnostics.push(
                BackendErrorCode::AccessibilityCoverageLimited,
                "AT-SPI returned no accessible applications",
                None,
            );
            crate::capture_plan::push_diagnostics(
                &environment,
                capture.as_ref(),
                portal_session_error.as_ref(),
                capture_error.as_ref(),
                &mut diagnostics,
            );
            push_portal_lifecycle_diagnostics(&mut portal_lifecycle_events, &mut diagnostics);
            if let Some(window) = selector
                .as_ref()
                .and_then(|selector| select_linux_window(&registry_windows, selector))
                .or_else(|| preferred_linux_window(&registry_windows))
            {
                let app = app_from_linux_window(&window);
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!(
                        "The selected {} window is visible through the window registry, but no accessible AT-SPI application tree matched it",
                        window.backend
                    ),
                    Some(selector_or_window_summary(selector.as_ref(), &app)),
                );
                return Ok(linux_fallback_snapshot(
                    snapshot_id,
                    environment,
                    capabilities,
                    capture,
                    diagnostics,
                    Some(doctor_report),
                    window,
                ));
            }
            return Ok(AppStateSnapshot {
                snapshot_id,
                created_at: chrono::Utc::now(),
                environment,
                capabilities,
                focused_app: None,
                capture,
                elements: Vec::new(),
                diagnostics: diagnostics.finish(),
                app_guidance: None,
                doctor_report: Some(doctor_report),
                agent_cursor: None,
            });
        }

        let chosen_app: DiscoveredApp = if let Some(selector) = selector.as_ref() {
            if let Some(app) = select_app(&apps, selector) {
                app
            } else if let Some(window) = select_linux_window(&registry_windows, selector) {
                crate::capture_plan::push_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&mut portal_lifecycle_events, &mut diagnostics);
                let app = app_from_linux_window(&window);
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!(
                        "The selected {} window is visible through the window registry, but no accessible AT-SPI application tree matched it",
                        window.backend
                    ),
                    Some(selector_or_window_summary(Some(selector), &app)),
                );
                return Ok(linux_fallback_snapshot(
                    snapshot_id,
                    environment,
                    capabilities,
                    capture,
                    diagnostics,
                    Some(doctor_report),
                    window,
                ));
            } else {
                return Err(BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "no accessible application matched selector {}",
                        selector_summary(selector)
                    ),
                ));
            }
        } else {
            if let Some(window) = preferred_linux_window(&registry_windows)
                && !apps
                    .iter()
                    .any(|app| linux_window_matches_app(&window, &app.info))
            {
                let app = app_from_linux_window(&window);
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!(
                        "The focused {} window is visible through the window registry, but no accessible AT-SPI application tree matched it",
                        window.backend
                    ),
                    Some(window_summary(&app)),
                );
                crate::capture_plan::push_diagnostics(
                    &environment,
                    capture.as_ref(),
                    portal_session_error.as_ref(),
                    capture_error.as_ref(),
                    &mut diagnostics,
                );
                push_portal_lifecycle_diagnostics(&mut portal_lifecycle_events, &mut diagnostics);
                return Ok(linux_fallback_snapshot(
                    snapshot_id,
                    environment,
                    capabilities,
                    capture,
                    diagnostics,
                    Some(doctor_report),
                    window,
                ));
            }

            pick_focused_app_with_fallback(&connection, apps, &registry_windows, &mut diagnostics)
                .await
        };
        let mut focused_app = Self::focused_from_app(&chosen_app.info);
        focused_app.display = registry_windows
            .iter()
            .find(|window| linux_window_matches_app(window, &chosen_app.info))
            .and_then(|window| window.display.clone());
        let focused_app = Some(focused_app);

        let (elements, snapshot_diags) = snapshot_for_app(&connection, &chosen_app).await?;
        for entry in snapshot_diags {
            diagnostics.push(
                BackendErrorCode::AccessibilityCoverageLimited,
                entry.message,
                entry.details,
            );
        }

        crate::capture_plan::push_diagnostics(
            &environment,
            capture.as_ref(),
            portal_session_error.as_ref(),
            capture_error.as_ref(),
            &mut diagnostics,
        );
        push_portal_lifecycle_diagnostics(&mut portal_lifecycle_events, &mut diagnostics);

        Ok(AppStateSnapshot {
            snapshot_id,
            created_at: chrono::Utc::now(),
            environment,
            capabilities,
            focused_app,
            capture,
            elements,
            diagnostics: diagnostics.finish(),
            app_guidance: None,
            doctor_report: Some(doctor_report),
            agent_cursor: None,
        })
    }

    async fn screenshot(
        &self,
        target: Option<WindowTarget>,
        display_target: Option<DisplayTarget>,
        capture_all_displays: bool,
    ) -> Result<AppStateSnapshot, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        let snapshot_id = new_snapshot_id();
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        let capabilities = Self::capabilities(&environment);
        let mut diagnostics = DiagnosticBuilder::new();

        let mut target_window = None;
        let mut capture_target = None;
        let mut capture_scope = CaptureScope::Unknown;
        let mut capture_display = None;
        if let Some(target) = target {
            let windows = linux_windowing::discover_activation_windows(&environment).await?;
            let matched = linux_windowing::resolve_window_target(&windows, &target.into())?;
            linux_windowing::activate_window(matched).await?;
            let focused = linux_windowing::verify_window_focused(&environment, matched).await?;
            diagnostics.push_code(
                "WindowFocusVerified",
                format!(
                    "Focus verification matched {} window {} before screenshot capture.",
                    focused.backend, focused.window_id
                ),
                None,
            );
            let bounds = focused.bounds.clone().ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "matched {} window {} did not report bounds for targeted screenshot capture",
                        focused.backend, focused.window_id
                    ),
                )
            })?;
            capture_scope = CaptureScope::Window;
            capture_display = focused.display.clone();
            capture_target = Some(crate::capture_plan::CaptureRegionTarget {
                desktop_logical_rect: bounds,
                capture_scope: CaptureScope::Window,
                display: focused.display.clone(),
            });
            target_window = Some(focused);
        } else if let Some(display_target) = display_target {
            let display =
                crate::displays::resolve_display_target(&environment.displays, &display_target)?;
            let display_ref = crate::displays::display_ref(&display);
            capture_scope = CaptureScope::Display;
            capture_display = Some(display_ref.clone());
            capture_target = Some(crate::capture_plan::CaptureRegionTarget {
                desktop_logical_rect: display.logical_rect.clone(),
                capture_scope: CaptureScope::Display,
                display: Some(display_ref),
            });
        } else if capture_all_displays {
            capture_scope = CaptureScope::AllDisplays;
        } else if let Some(display) = crate::displays::primary_display(&environment.displays) {
            let display_ref = crate::displays::display_ref(&display);
            capture_scope = CaptureScope::PrimaryDisplay;
            capture_display = Some(display_ref.clone());
            capture_target = Some(crate::capture_plan::CaptureRegionTarget {
                desktop_logical_rect: display.logical_rect.clone(),
                capture_scope: CaptureScope::PrimaryDisplay,
                display: Some(display_ref),
            });
        } else {
            diagnostics.push(
                BackendErrorCode::CaptureBackendDowngraded,
                "Display topology is unavailable, so screenshot fell back to the legacy raw desktop capture for an omitted selector.",
                None,
            );
        }

        let capture_plan = crate::capture_plan::plan_capture(
            &self.portal,
            &snapshot_id,
            CaptureScreenMode::Always,
            &environment,
            capture_target.as_ref(),
            capture_scope,
            capture_display,
            &mut diagnostics,
        )
        .await?;
        let mut portal_lifecycle_events = self.portal.take_lifecycle_events().await;
        crate::capture_plan::push_diagnostics(
            &environment,
            capture_plan.capture.as_ref(),
            capture_plan.portal_session_error.as_ref(),
            capture_plan.capture_error.as_ref(),
            &mut diagnostics,
        );
        push_portal_lifecycle_diagnostics(&mut portal_lifecycle_events, &mut diagnostics);
        require_screenshot_image(
            capture_plan.capture.as_ref(),
            capture_plan.portal_session_error.as_ref(),
            capture_plan.capture_error.as_ref(),
        )?;

        let focused_app = target_window.as_ref().map(Self::focused_from_linux_window);

        Ok(AppStateSnapshot {
            snapshot_id,
            created_at: chrono::Utc::now(),
            environment,
            capabilities,
            focused_app,
            capture: capture_plan.capture,
            elements: Vec::new(),
            diagnostics: diagnostics.finish(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        })
    }

    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        LinuxActionExecutor::new(self).execute(request).await
    }

    async fn reset_portal_tokens(
        &self,
    ) -> Result<sky_cua_platform::model::PortalTokenResetOutcome, BackendError> {
        self.portal.reset_persisted_tokens().await
    }

    async fn ensure_session_presence(
        &self,
        intent: sky_cua_platform::model::SessionPresenceIntent,
    ) -> Result<sky_cua_platform::model::SessionPresenceStatus, BackendError> {
        Ok(self.session_presence.ensure(intent).await)
    }

    async fn release_session_presence(
        &self,
        relock: bool,
    ) -> Result<sky_cua_platform::model::SessionPresenceStatus, BackendError> {
        Ok(self.session_presence.release(relock).await)
    }

    async fn session_presence_status(&self) -> sky_cua_platform::model::SessionPresenceStatus {
        self.session_presence.status().await
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

#[async_trait::async_trait]
impl LinuxActionRuntime for LinuxDesktopBackend {
    async fn semantic_grab_focus(&self, backend_ref: &str) -> Result<bool, BackendError> {
        let connection = self.accessibility_connection().await?;
        atspi_actions::grab_focus(&connection, backend_ref).await
    }

    async fn semantic_perform(
        &self,
        backend_ref: &str,
        action: SemanticAtspiAction,
    ) -> Result<bool, BackendError> {
        let connection = self.accessibility_connection().await?;
        match action {
            SemanticAtspiAction::Activate => {
                atspi_actions::activate(&connection, backend_ref).await
            }
            SemanticAtspiAction::Select => atspi_actions::select(&connection, backend_ref).await,
            SemanticAtspiAction::Expand => atspi_actions::expand(&connection, backend_ref).await,
            SemanticAtspiAction::Collapse => {
                atspi_actions::collapse(&connection, backend_ref).await
            }
            SemanticAtspiAction::Toggle => atspi_actions::toggle(&connection, backend_ref).await,
        }
    }

    async fn semantic_invoke_default(&self, backend_ref: &str) -> Result<bool, BackendError> {
        let connection = self.accessibility_connection().await?;
        atspi_actions::invoke_default_action(&connection, backend_ref).await
    }

    async fn semantic_invoke_secondary(&self, backend_ref: &str) -> Result<bool, BackendError> {
        let connection = self.accessibility_connection().await?;
        atspi_actions::invoke_secondary_action(&connection, backend_ref).await
    }

    async fn semantic_available_actions(
        &self,
        backend_ref: &str,
    ) -> Result<Vec<String>, BackendError> {
        let connection = self.accessibility_connection().await?;
        atspi_actions::available_actions(&connection, backend_ref).await
    }

    async fn semantic_invoke_action_by_index(
        &self,
        backend_ref: &str,
        action_index: i32,
    ) -> Result<SemanticActionInvocation, BackendError> {
        let connection = self.accessibility_connection().await?;
        let result =
            atspi_actions::invoke_action_by_index(&connection, backend_ref, action_index).await?;
        Ok(SemanticActionInvocation {
            action_index: result.action_index,
            action_name: result.action_name,
            ok: result.ok,
        })
    }

    async fn semantic_set_value(
        &self,
        backend_ref: &str,
        value: &str,
    ) -> Result<SemanticSetValueResult, BackendError> {
        let connection = self.accessibility_connection().await?;
        match atspi_actions::set_value(&connection, backend_ref, value).await? {
            atspi_actions::SetValueResult::EditableText => Ok(SemanticSetValueResult::EditableText),
            atspi_actions::SetValueResult::Numeric { value } => {
                Ok(SemanticSetValueResult::Numeric { value })
            }
        }
    }

    async fn semantic_scroll_vertical_at(
        &self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
        app: Option<&FocusedApp>,
    ) -> Result<bool, BackendError> {
        let (connection, apps) = self.discover_accessible_apps().await?;
        let preferred_selector = app.map(selector_from_focused_app);
        let mut selected: Option<(f64, String, f64)> = None;

        for candidate_app in apps.iter().filter(|candidate_app| {
            preferred_selector.as_ref().is_none_or(|selector| {
                selector_match_score(&candidate_app.info, selector).is_some()
            })
        }) {
            let (elements, _) = snapshot_for_app(&connection, candidate_app).await?;
            let Some((area, scrollbar)) = vertical_scrollbar_for_point(&elements, x, y) else {
                continue;
            };
            let (Some(backend_ref), Some(target_value)) = (
                scrollbar.backend_ref.as_ref(),
                scroll_target_value(scrollbar, delta_y, steps),
            ) else {
                continue;
            };
            if selected
                .as_ref()
                .is_none_or(|(selected_area, _, _)| area < *selected_area)
            {
                selected = Some((area, backend_ref.clone(), target_value));
            }
        }

        let Some((_, backend_ref, target_value)) = selected else {
            return Ok(false);
        };

        atspi_actions::set_value(&connection, &backend_ref, &target_value.to_string()).await?;
        Ok(true)
    }

    fn resolve_set_value_fallback_policy(
        &self,
        app: Option<&FocusedApp>,
    ) -> Option<ResolvedSetValueFallbackPolicy> {
        self.app_policies.resolve_set_value_fallback(app)
    }

    async fn focus_window_target_for_keyboard(
        &self,
        request: &ActionRequest,
    ) -> Result<Option<linux_windowing::LinuxWindowInfo>, BackendError> {
        self.focus_window_target_for_keyboard(request).await
    }

    fn matched_x11_window_for_request(&self, request: &ActionRequest) -> Option<X11WindowInfo> {
        matched_x11_window_for_request(request)
    }

    fn xtest_is_available(&self) -> bool {
        input_xtest::xtest_is_available()
    }

    fn activate_x11_window(&self, window: Option<&X11WindowInfo>) {
        activate_x11_window(window);
    }

    async fn portal_click_at(
        &self,
        x: f64,
        y: f64,
        button: MouseButton,
    ) -> Result<(), BackendError> {
        self.portal.click_at(x, y, button).await
    }

    async fn portal_drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
        self.portal.drag(from, to).await
    }

    async fn portal_scroll_vertical_at(
        &self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
    ) -> Result<(), BackendError> {
        self.portal.scroll_vertical_at(x, y, delta_y, steps).await
    }

    async fn portal_scroll_vertical_smooth(&self, delta_y: f64) -> Result<(), BackendError> {
        self.portal.scroll_vertical_smooth(delta_y).await
    }

    async fn portal_scroll_vertical_discrete(&self, steps: i32) -> Result<(), BackendError> {
        self.portal.scroll_vertical_discrete(steps).await
    }

    async fn portal_send_text(&self, text: &str) -> Result<(), BackendError> {
        self.portal.send_text(text).await
    }

    async fn portal_press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        self.portal.press_key_sequence(keys).await
    }

    async fn portal_press_key_sequence_portal_only(
        &self,
        keys: &[String],
    ) -> Result<(), BackendError> {
        self.portal.press_key_sequence_portal_only(keys).await
    }

    async fn portal_take_lifecycle_diagnostics(&self) -> Vec<DiagnosticEntry> {
        let mut events = self.portal.take_lifecycle_events().await;
        portal_lifecycle_diagnostics(&mut events)
    }

    async fn portal_reset_session(&self) {
        self.portal.reset_session().await;
    }

    fn xtest_pointer_move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
        input_xtest::pointer_move_absolute(x, y)
    }

    fn xtest_pointer_button(&self, button: MouseButton, pressed: bool) -> Result<(), BackendError> {
        input_xtest::pointer_button(x11_mouse_button(button), pressed)
    }

    fn xtest_click(&self, button: MouseButton) -> Result<(), BackendError> {
        input_xtest::click(x11_mouse_button(button))
    }

    fn xtest_scroll_vertical(
        &self,
        delta_y: Option<f64>,
        steps: Option<i32>,
    ) -> Result<(), BackendError> {
        input_xtest::scroll_vertical(delta_y, steps)
    }

    fn xtest_send_text_to_target(
        &self,
        window_id: Option<&str>,
        text: &str,
    ) -> Result<(), BackendError> {
        input_xtest::send_text_to_target(window_id, text)
    }

    fn xtest_press_key_sequence_to_target(
        &self,
        window_id: Option<&str>,
        keys: &[String],
    ) -> Result<(), BackendError> {
        input_xtest::press_key_sequence_to_target(window_id, keys)
    }

    fn virtual_click_at(&self, x: f64, y: f64, button: MouseButton) -> Result<(), BackendError> {
        self.cached_virtual_input()?.click_at(x, y, button)
    }

    fn virtual_pointer_mapping_diagnostic(
        &self,
        x: f64,
        y: f64,
    ) -> Result<Option<DiagnosticEntry>, BackendError> {
        let virtual_input = self.cached_virtual_input()?;
        Ok(Some(DiagnosticEntry {
            code: "LinuxVirtualInputPointerMapping".to_string(),
            message: "Linux virtual input pointer coordinate mapping.".to_string(),
            details: Some(virtual_input.pointer_mapping_details(x, y)),
        }))
    }

    fn virtual_drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
        self.cached_virtual_input()?.drag(from, to)
    }

    fn virtual_scroll_vertical(&self, steps: i32) -> Result<(), BackendError> {
        self.cached_virtual_input()?.scroll_vertical(steps)
    }

    fn virtual_scroll_vertical_at(&self, x: f64, y: f64, steps: i32) -> Result<(), BackendError> {
        self.cached_virtual_input()?.scroll_vertical_at(x, y, steps)
    }

    fn virtual_type_text(&self, text: &str) -> Result<(), BackendError> {
        self.cached_virtual_input()?.type_text(text)
    }

    fn virtual_press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        self.cached_virtual_input()?.press_key_sequence(keys)
    }
}

fn x11_mouse_button(button: MouseButton) -> X11MouseButton {
    match button {
        MouseButton::Left => X11MouseButton::Left,
        MouseButton::Middle => X11MouseButton::Middle,
        MouseButton::Right => X11MouseButton::Right,
    }
}

fn is_retryable_accessibility_error(error: &BackendError) -> bool {
    error.code == BackendErrorCode::AccessibilityUnavailable.as_str()
        && error.message.contains("Resource temporarily unavailable")
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
            "Linux virtual input is pointer-only; text and key actions require a usable ydotool daemon"
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

fn success_with_diagnostics(
    message: impl Into<String>,
    diagnostics: Vec<sky_cua_platform::model::DiagnosticEntry>,
) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: message.into(),
        code: "Ok".to_string(),
        diagnostics,
        agent_cursor: None,
    }
}

fn activate_x11_window(window: Option<&X11WindowInfo>) {
    if let Some(window) = window
        && let Err(error) = input_xtest::window_activate(&window.window_id)
    {
        warn!(
            "X11 window activation failed before input fallback; continuing with pointer injection: {}",
            error.message
        );
    }
}

fn matched_x11_window_for_request(request: &ActionRequest) -> Option<X11WindowInfo> {
    let app = request.resolved_focused_app.as_ref()?;
    if !windowing::x11_window_query_available() {
        return None;
    }

    let windows = windowing::discover_windows().ok()?;
    let app = AppInfo {
        app_id: app.app_id.clone(),
        name: app.name.clone(),
        pid: app.pid,
        executable: None,
        desktop_file_id: app.desktop_file_id.clone(),
        app_user_model_id: None,
        window_handle: None,
        toolkit_guess: app.toolkit_guess.clone(),
        window_title: app.window_title.clone(),
        is_focused_candidate: true,
    };
    best_x11_window_match(&windows, &app).cloned()
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

fn portal_lifecycle_diagnostics(
    events: &mut Vec<PortalLifecycleEvent>,
) -> Vec<sky_cua_platform::model::DiagnosticEntry> {
    events
        .drain(..)
        .map(|event| sky_cua_platform::model::DiagnosticEntry {
            code: event.code.to_string(),
            message: event.message,
            details: event.details,
        })
        .collect()
}

fn push_portal_lifecycle_diagnostics(
    events: &mut Vec<PortalLifecycleEvent>,
    diagnostics: &mut DiagnosticBuilder,
) {
    for event in events.drain(..) {
        diagnostics.push_code(event.code, event.message, event.details);
    }
}

fn selector_from_focused_app(app: &FocusedApp) -> AppSelector {
    AppSelector {
        app_id: Some(app.app_id.clone()),
        desktop_file_id: app.desktop_file_id.clone(),
        window_title: app.window_title.clone(),
        name: Some(app.name.clone()),
    }
}

fn vertical_scrollbar_for_point(
    elements: &[ElementNode],
    x: f64,
    y: f64,
) -> Option<(f64, &ElementNode)> {
    elements
        .iter()
        .filter(|node| is_vertical_value_scrollbar(node))
        .filter_map(|node| {
            scroll_ancestor_area_containing_point(elements, node, x, y).map(|area| (area, node))
        })
        .min_by(|(left, _), (right, _)| left.total_cmp(right))
}

fn is_vertical_value_scrollbar(node: &ElementNode) -> bool {
    node.role == "scroll bar"
        && node
            .numeric_value
            .as_ref()
            .is_some_and(|value| value.maximum > value.minimum)
        && node.state_flags.iter().any(|state| state == "vertical")
        && node
            .semantic_actions
            .iter()
            .any(|action| action == "set_value")
}

fn scroll_ancestor_area_containing_point(
    elements: &[ElementNode],
    node: &ElementNode,
    x: f64,
    y: f64,
) -> Option<f64> {
    let mut parent_index = node.parent_index;
    while let Some(index) = parent_index {
        let parent = elements.get(index)?;
        if let Some(bounds) = parent.bounds.as_ref()
            && bounds_contains(bounds, x, y)
        {
            return Some(bounds.width * bounds.height);
        }
        parent_index = parent.parent_index;
    }
    node.bounds
        .as_ref()
        .filter(|bounds| bounds_contains(bounds, x, y))
        .map(|bounds| bounds.width * bounds.height)
}

fn bounds_contains(bounds: &RectF, x: f64, y: f64) -> bool {
    bounds.width > 0.0
        && bounds.height > 0.0
        && x >= bounds.x
        && x <= bounds.x + bounds.width
        && y >= bounds.y
        && y <= bounds.y + bounds.height
}

fn scroll_target_value(node: &ElementNode, delta_y: Option<f64>, steps: i32) -> Option<f64> {
    let value = node.numeric_value.as_ref()?;
    let steps =
        crate::actions::targeting::virtual_scroll_steps_from_delta(delta_y).unwrap_or(steps);
    if steps == 0 {
        return None;
    }
    let increment = if value.minimum_increment > 0.0 {
        value.minimum_increment
    } else {
        ((value.maximum - value.minimum) / 10.0).max(1.0)
    };
    Some((value.current + f64::from(steps) * increment).clamp(value.minimum, value.maximum))
}

fn linux_fallback_snapshot(
    snapshot_id: String,
    environment: EnvironmentInfo,
    capabilities: ToolCapabilities,
    capture: Option<sky_cua_platform::model::CaptureInfo>,
    diagnostics: DiagnosticBuilder,
    doctor_report: Option<DoctorReport>,
    window: linux_windowing::LinuxWindowInfo,
) -> AppStateSnapshot {
    AppStateSnapshot {
        snapshot_id,
        created_at: chrono::Utc::now(),
        environment,
        capabilities,
        focused_app: Some(LinuxDesktopBackend::focused_from_linux_window(&window)),
        capture,
        elements: fallback_window_elements(&window),
        diagnostics: diagnostics.finish(),
        app_guidance: None,
        doctor_report,
        agent_cursor: None,
    }
}

fn fallback_window_elements(window: &linux_windowing::LinuxWindowInfo) -> Vec<ElementNode> {
    let x11_window = refreshed_x11_window_for_linux_window(window);
    fallback_window_elements_with_x11_detail(window, x11_window.as_ref())
}

fn fallback_window_elements_with_x11_detail(
    window: &linux_windowing::LinuxWindowInfo,
    x11_window: Option<&X11WindowInfo>,
) -> Vec<ElementNode> {
    x11_window
        .map(x11_window_elements)
        .filter(|elements| !elements.is_empty())
        .unwrap_or_else(|| linux_window_elements(window))
}

fn refreshed_x11_window_for_linux_window(
    window: &linux_windowing::LinuxWindowInfo,
) -> Option<X11WindowInfo> {
    if window.backend != "x11" {
        return None;
    }
    windowing::discover_windows()
        .ok()?
        .into_iter()
        .find(|candidate| candidate.window_id == window.window_id)
}

fn linux_window_elements(window: &linux_windowing::LinuxWindowInfo) -> Vec<ElementNode> {
    let Some(bounds) = window.bounds.clone() else {
        return Vec::new();
    };

    let mut root_state_flags = vec![
        "native_window_fallback".to_string(),
        "physical_target".to_string(),
        "vision_anchor".to_string(),
        "container".to_string(),
        "content_like".to_string(),
    ];
    if window.focused {
        root_state_flags.push("focused".to_string());
        root_state_flags.push("active".to_string());
    }
    let app = app_from_linux_window(window);

    let mut elements = vec![ElementNode {
        element_index: 0,
        parent_index: None,
        role: "window".to_string(),
        name: app.window_title.clone().or_else(|| Some(app.name.clone())),
        description: Some(format!(
            "{} window surfaced from the window registry without a matching AT-SPI tree. The child regions below are geometric anchors only: use them to narrow the search space, then confirm the real target on the screenshot before clicking, dragging, or typing.",
            window.backend
        )),
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags: root_state_flags,
        semantic_actions: Vec::new(),
        bounds: Some(bounds.clone()),
        backend_ref: None,
    }];

    if bounds.width < 220.0 || bounds.height < 180.0 {
        return elements;
    }

    let top_band_height = (bounds.height * 0.13).clamp(44.0, 96.0);
    let content_y = bounds.y + top_band_height;
    let content_height = (bounds.height - top_band_height).max(40.0);
    let sidebar_width = if bounds.width >= 520.0 {
        (bounds.width * 0.23).clamp(140.0, 320.0)
    } else {
        0.0
    };
    let space = bounds.space.clone();
    let main_x = bounds.x + sidebar_width;
    let main_width = (bounds.width - sidebar_width).max(120.0);
    let header_bounds = RectF {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: top_band_height,
        space: space.clone(),
    };
    let search_width = (bounds.width * 0.38)
        .clamp(180.0, 480.0)
        .min(bounds.width - 32.0);
    let search_height = (top_band_height * 0.62).clamp(28.0, 52.0);
    let search_bounds = RectF {
        x: bounds.x + ((bounds.width - search_width) / 2.0),
        y: bounds.y + ((top_band_height - search_height) / 2.0),
        width: search_width,
        height: search_height,
        space: space.clone(),
    };
    let toolbar_width = (bounds.width * 0.18).clamp(120.0, 260.0);
    let toolbar_bounds = RectF {
        x: bounds.x + bounds.width - toolbar_width - 12.0,
        y: bounds.y,
        width: toolbar_width,
        height: top_band_height,
        space: space.clone(),
    };

    push_kwin_anchor(
        &mut elements,
        0,
        "wayland_header_band",
        Some("Top band".to_string()),
        "Heuristic top band derived from the KWin window bounds. It often contains app navigation, tabs, or a search bar, but it is not a semantic tree node. Use the screenshot to verify the real control before acting.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "container",
            "action_like",
        ],
        header_bounds,
    );

    push_kwin_anchor(
        &mut elements,
        1,
        "wayland_search_candidate",
        Some("Search candidate".to_string()),
        "Heuristic search/text-entry candidate carved out of the top band. Treat it as a likely text-target anchor only; confirm the visible search field on the screenshot before clicking or typing.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "leaf",
            "search_like",
            "text_like",
        ],
        search_bounds,
    );

    push_kwin_anchor(
        &mut elements,
        1,
        "wayland_toolbar_candidate",
        Some("Action strip candidate".to_string()),
        "Heuristic action strip on the right side of the top band. It often contains buttons or profile controls, but the screenshot must confirm the actual target.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "leaf",
            "action_like",
        ],
        toolbar_bounds,
    );

    if sidebar_width > 0.0 {
        let sidebar_bounds = RectF {
            x: bounds.x,
            y: content_y,
            width: sidebar_width,
            height: content_height,
            space: space.clone(),
        };
        let sidebar_index = push_kwin_anchor(
            &mut elements,
            0,
            "wayland_sidebar_region",
            Some("Sidebar candidate".to_string()),
            "Heuristic left-side navigation rail derived from the window geometry. It is useful for orienting around libraries, playlists, or side panels, but the screenshot should confirm the visible list or rail before interaction.",
            vec![
                "kwin_fallback",
                "physical_target",
                "vision_anchor",
                "container",
                "navigation_like",
                "list_like",
            ],
            sidebar_bounds.clone(),
        );
        let sidebar_list_height =
            (sidebar_bounds.height * 0.72).clamp(120.0, sidebar_bounds.height);
        let sidebar_list_bounds = RectF {
            x: sidebar_bounds.x,
            y: sidebar_bounds.y + 8.0,
            width: sidebar_bounds.width,
            height: sidebar_list_height - 8.0,
            space: space.clone(),
        };
        let sidebar_list_index = push_kwin_anchor(
            &mut elements,
            sidebar_index,
            "wayland_list_candidate",
            Some("Sidebar list candidate".to_string()),
            "Heuristic list-like region inside the sidebar. Use it as a structural hint for playlist or library rails, then verify the actual rows on the screenshot before clicking or scrolling.",
            vec![
                "kwin_fallback",
                "physical_target",
                "vision_anchor",
                "leaf",
                "list_like",
                "navigation_like",
            ],
            sidebar_list_bounds.clone(),
        );
        let sidebar_row_band_count =
            ((sidebar_list_bounds.height / 96.0).floor() as usize).clamp(2, 5);
        let sidebar_row_band_height = (sidebar_list_bounds.height * 0.16)
            .clamp(46.0, 104.0)
            .min((sidebar_list_bounds.height - 12.0).max(40.0));
        let sidebar_row_band_gap = ((sidebar_list_bounds.height
            - sidebar_row_band_height * sidebar_row_band_count as f64)
            / (sidebar_row_band_count as f64 + 1.0))
            .clamp(6.0, 28.0);
        for band in 0..sidebar_row_band_count {
            let row_y = sidebar_list_bounds.y
                + sidebar_row_band_gap
                + band as f64 * (sidebar_row_band_height + sidebar_row_band_gap);
            push_kwin_anchor(
                &mut elements,
                sidebar_list_index,
                "wayland_row_band_candidate",
                Some(format!("Sidebar row band candidate {}", band + 1)),
                "Heuristic visible row band inside the sidebar list. It often contains playlist or library rows with text and context actions. Verify the row text on the screenshot before clicking, right-clicking, or dragging.",
                vec![
                    "kwin_fallback",
                    "physical_target",
                    "vision_anchor",
                    "leaf",
                    "list_like",
                    "row_like",
                    "text_like",
                ],
                RectF {
                    x: sidebar_list_bounds.x + 4.0,
                    y: row_y,
                    width: (sidebar_list_bounds.width - 8.0).max(40.0),
                    height: sidebar_row_band_height,
                    space: sidebar_list_bounds.space.clone(),
                },
            );
        }
    }

    let main_bounds = RectF {
        x: main_x,
        y: content_y,
        width: main_width,
        height: content_height,
        space: space.clone(),
    };
    let main_index = push_kwin_anchor(
        &mut elements,
        0,
        "wayland_main_region",
        Some("Main content candidate".to_string()),
        "Heuristic main content region. This usually contains the primary page or detail view. Use it to orient the screenshot search, not as a promise about semantics.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "container",
            "content_like",
        ],
        main_bounds.clone(),
    );
    let list_candidate_height = (main_bounds.height * 0.68).clamp(140.0, main_bounds.height);
    let main_list_bounds = RectF {
        x: main_bounds.x + 8.0,
        y: main_bounds.y + 8.0,
        width: (main_bounds.width - 16.0).max(40.0),
        height: (list_candidate_height - 8.0).max(40.0),
        space,
    };
    let main_list_index = push_kwin_anchor(
        &mut elements,
        main_index,
        "wayland_list_candidate",
        Some("Main list candidate".to_string()),
        "Heuristic list/grid region inside the main content area. This is often where search results, playlists, or tracks appear. Confirm the visible rows or tiles on the screenshot before clicking, scrolling, or dragging.",
        vec![
            "kwin_fallback",
            "physical_target",
            "vision_anchor",
            "leaf",
            "list_like",
            "content_like",
        ],
        main_list_bounds.clone(),
    );
    let main_row_band_count = ((main_list_bounds.height / 108.0).floor() as usize).clamp(2, 5);
    let visible_row_band_height = (main_list_bounds.height * 0.11)
        .clamp(44.0, 104.0)
        .min((main_list_bounds.height - 12.0).max(40.0));
    let row_band_width = (main_list_bounds.width - 8.0).max(56.0);
    let main_row_band_gap = ((main_list_bounds.height
        - visible_row_band_height * main_row_band_count as f64)
        / (main_row_band_count as f64 + 1.0))
        .clamp(8.0, 32.0);
    for band in 0..main_row_band_count {
        let row_y = main_list_bounds.y
            + main_row_band_gap
            + band as f64 * (visible_row_band_height + main_row_band_gap);
        push_kwin_anchor(
            &mut elements,
            main_list_index,
            "wayland_row_band_candidate",
            Some(format!("Main row band candidate {}", band + 1)),
            "Heuristic visible row band inside the main list/grid area. It often contains track or result rows with text and row-level actions. Use the screenshot to confirm the real row before clicking or opening a context menu.",
            vec![
                "kwin_fallback",
                "physical_target",
                "vision_anchor",
                "leaf",
                "list_like",
                "row_like",
                "text_like",
            ],
            RectF {
                x: main_list_bounds.x,
                y: row_y,
                width: row_band_width,
                height: visible_row_band_height,
                space: main_bounds.space.clone(),
            },
        );
    }
    let action_cluster_width = (main_bounds.width * 0.18).clamp(64.0, 180.0);
    let action_cluster_height = (visible_row_band_height * 1.25).clamp(44.0, 120.0);
    for band in 0..main_row_band_count {
        let row_y = main_list_bounds.y
            + main_row_band_gap
            + band as f64 * (visible_row_band_height + main_row_band_gap);
        push_kwin_anchor(
            &mut elements,
            main_list_index,
            "wayland_action_cluster_candidate",
            Some(format!("Main action cluster candidate {}", band + 1)),
            "Heuristic right-edge action cluster inside a visible row area. This often lines up with overflow buttons, kebab menus, or row-level actions. Confirm the visible affordance on the screenshot before clicking or right-clicking.",
            vec![
                "kwin_fallback",
                "physical_target",
                "vision_anchor",
                "leaf",
                "action_like",
                "menu_like",
            ],
            RectF {
                x: (main_bounds.x + main_bounds.width - action_cluster_width - 12.0)
                    .max(main_bounds.x + 8.0),
                y: (row_y - 2.0).max(main_list_bounds.y),
                width: action_cluster_width,
                height: action_cluster_height
                    .min((main_list_bounds.y + main_list_bounds.height - row_y).max(32.0)),
                space: main_bounds.space.clone(),
            },
        );
    }

    elements
}

fn push_kwin_anchor(
    elements: &mut Vec<ElementNode>,
    parent_index: usize,
    role: &str,
    name: Option<String>,
    description: &str,
    state_flags: Vec<&str>,
    bounds: RectF,
) -> usize {
    let element_index = elements.len();
    elements.push(ElementNode {
        element_index,
        parent_index: Some(parent_index),
        role: role.to_string(),
        name,
        description: Some(description.to_string()),
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags: state_flags
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect(),
        semantic_actions: Vec::new(),
        bounds: Some(bounds),
        backend_ref: None,
    });
    element_index
}

fn x11_window_elements(window: &X11WindowInfo) -> Vec<ElementNode> {
    let Some(bounds) = window.bounds.clone() else {
        return Vec::new();
    };

    let mut state_flags = Vec::new();
    if window.app.is_focused_candidate {
        state_flags.push("focused".to_string());
        state_flags.push("active".to_string());
    }
    state_flags.push("native_window_fallback".to_string());
    state_flags.push("x11_fallback".to_string());
    state_flags.push("physical_target".to_string());

    let mut elements = vec![ElementNode {
        element_index: 0,
        parent_index: None,
        role: "window".to_string(),
        name: window
            .app
            .window_title
            .clone()
            .or_else(|| Some(window.app.name.clone())),
        description: Some(
            "X11/XWayland window surfaced without a matching AT-SPI tree; physical actions can still target its bounds"
                .to_string(),
        ),
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags,
        semantic_actions: Vec::new(),
        bounds: Some(bounds.clone()),
        backend_ref: None,
    }];

    let child_counts = window.child_regions.iter().fold(
        std::collections::HashMap::<String, usize>::new(),
        |mut counts, region| {
            if let Some(parent_window_id) = region.parent_window_id.as_ref() {
                *counts.entry(parent_window_id.clone()).or_default() += 1;
            }
            counts
        },
    );
    let mut index_by_window_id =
        std::collections::HashMap::from([(window.window_id.clone(), 0usize)]);
    for region in &window.child_regions {
        if region.bounds.width < 8.0 || region.bounds.height < 8.0 {
            continue;
        }

        let parent_index = region
            .parent_window_id
            .as_ref()
            .and_then(|window_id| index_by_window_id.get(window_id).copied())
            .or(Some(0));
        let element_index = elements.len();
        let has_children = child_counts
            .get(&region.window_id)
            .copied()
            .unwrap_or_default()
            > 0;
        let role = x11_region_role(region, has_children, &bounds);
        let mut state_flags = vec!["x11_fallback".to_string(), "physical_target".to_string()];
        if has_children {
            state_flags.push("container".to_string());
        } else {
            state_flags.push("leaf".to_string());
        }
        if role == "x11_action_region" {
            state_flags.push("action_like".to_string());
        }
        elements.push(ElementNode {
            element_index,
            parent_index,
            role: role.to_string(),
            name: region.name.clone(),
            description: Some(x11_region_description(region, role)),
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags,
            semantic_actions: Vec::new(),
            bounds: Some(region.bounds.clone()),
            backend_ref: None,
        });
        index_by_window_id.insert(region.window_id.clone(), element_index);
    }

    elements
}

fn x11_region_role(
    region: &crate::x11::windowing::X11WindowRegion,
    has_children: bool,
    root_bounds: &sky_cua_platform::model::RectF,
) -> &'static str {
    if has_children {
        return "x11_container";
    }

    let center_y = region.bounds.y + (region.bounds.height / 2.0);
    let root_mid_y = root_bounds.y + (root_bounds.height / 2.0);
    let small_relative_width = region.bounds.width <= root_bounds.width * 0.4;
    let small_relative_height = region.bounds.height <= root_bounds.height * 0.5;
    if center_y >= root_mid_y && small_relative_width && small_relative_height {
        "x11_action_region"
    } else {
        "x11_leaf_region"
    }
}

fn x11_region_description(region: &crate::x11::windowing::X11WindowRegion, role: &str) -> String {
    let role_hint = match role {
        "x11_container" => "container-like region",
        "x11_action_region" => "small lower leaf region that may behave like an actionable control",
        _ => "leaf region",
    };
    format!(
        "Recovered from the X11 window tree at depth {} as a {}; physical actions can target this region, but no semantic AT-SPI interface is available",
        region.depth, role_hint
    )
}

fn window_summary(app: &AppInfo) -> String {
    selector_summary(&AppSelector {
        app_id: Some(app.app_id.clone()),
        desktop_file_id: app.desktop_file_id.clone(),
        window_title: app.window_title.clone(),
        name: Some(app.name.clone()),
    })
}

fn selector_or_window_summary(selector: Option<&AppSelector>, app: &AppInfo) -> String {
    match selector {
        Some(selector) => format!(
            "{}, matched_x11_window={}",
            selector_summary(selector),
            window_summary(app)
        ),
        None => window_summary(app),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppInfo, AppSelector, DISPLAY_TOPOLOGY_CACHE_TTL, DisplayTopologyCache,
        LinuxDesktopBackend, cached_display_topology, fallback_window_elements_with_x11_detail,
        linux_fallback_snapshot, linux_window_elements, merge_session_env_reports,
        require_screenshot_image, scroll_target_value, vertical_scrollbar_for_point,
        x11_window_elements,
    };
    use crate::app_match::{
        app_from_linux_window, best_x11_window_match, matches_selector, select_x11_window,
        selector_summary, x11_window_matches_app,
    };
    use crate::capture_plan::should_attempt_x11_capture;
    use crate::windowing::LinuxWindowInfo;
    use crate::x11::windowing::{X11WindowInfo, X11WindowRegion};
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
    use sky_cua_platform::model::{
        CaptureBackendKind, CaptureScreenMode, CoordinateSpace, DisplayInfo,
        DoctorSessionEnvRepair, DoctorSessionEnvReport, ElementNode, ElementNumericValueReadback,
        EnvironmentInfo, InputBackendKind, PortalCapabilities, RectF, SemanticBackendKind,
        SessionKind,
    };
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};

    fn wayland_pipewire_environment() -> EnvironmentInfo {
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
    fn screenshot_no_image_preserves_portal_error() {
        let portal_error = BackendError::new(
            BackendErrorCode::PortalApprovalPending,
            "operator approval is pending",
        );

        let error = require_screenshot_image(None, Some(&portal_error), None).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::PortalApprovalPending.as_str());
        assert_eq!(error.message, "operator approval is pending");
    }

    fn test_element(
        element_index: usize,
        parent_index: Option<usize>,
        role: &str,
        bounds: Option<RectF>,
    ) -> ElementNode {
        ElementNode {
            element_index,
            parent_index,
            role: role.to_string(),
            name: None,
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: Vec::new(),
            semantic_actions: Vec::new(),
            bounds,
            backend_ref: None,
        }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
            space: CoordinateSpace::DesktopLogical,
        }
    }

    fn test_display(display_id: &str) -> DisplayInfo {
        DisplayInfo {
            display_id: display_id.to_string(),
            name: Some(display_id.to_string()),
            index: 0,
            primary: true,
            logical_rect: rect(0.0, 0.0, 1920.0, 1080.0),
            pixel_size: None,
            scale_factor: Some(1.0),
            backend: "test".to_string(),
        }
    }

    fn vertical_scrollbar(index: usize, parent_index: usize, current: f64) -> ElementNode {
        let mut node = test_element(
            index,
            Some(parent_index),
            "scroll bar",
            Some(rect(95.0, 0.0, 5.0, 100.0)),
        );
        node.state_flags.push("vertical".to_string());
        node.semantic_actions.push("set_value".to_string());
        node.backend_ref = Some(format!(":1.1:/scrollbar/{index}"));
        node.numeric_value = Some(ElementNumericValueReadback {
            current,
            minimum: 0.0,
            maximum: 100.0,
            minimum_increment: 10.0,
            text: None,
        });
        node
    }

    #[test]
    fn vertical_scrollbar_for_point_uses_containing_scroll_ancestor() {
        let elements = vec![
            test_element(0, None, "application", Some(rect(0.0, 0.0, 200.0, 200.0))),
            test_element(
                1,
                Some(0),
                "scroll pane",
                Some(rect(10.0, 20.0, 90.0, 80.0)),
            ),
            vertical_scrollbar(2, 1, 0.0),
        ];

        let (_, node) = vertical_scrollbar_for_point(&elements, 40.0, 50.0)
            .expect("point inside scroll pane should resolve scrollbar");

        assert_eq!(node.element_index, 2);
    }

    #[test]
    fn scroll_target_value_maps_downward_delta_to_larger_value() {
        let node = vertical_scrollbar(0, 0, 20.0);

        assert_eq!(scroll_target_value(&node, Some(-180.0), -1), Some(40.0));
        assert_eq!(scroll_target_value(&node, Some(120.0), -1), Some(10.0));
    }

    #[test]
    fn merge_session_env_reports_deduplicates_repeated_refreshes() {
        let repair = DoctorSessionEnvRepair {
            key: "WAYLAND_DISPLAY".to_string(),
            source: "systemd-user".to_string(),
            value: Some("wayland-0".to_string()),
        };
        let mut current = DoctorSessionEnvReport {
            repaired: vec![repair.clone()],
            path_changed: false,
            final_path: Some("/tmp:/usr/bin".to_string()),
            notes: vec!["systemd note".to_string()],
        };
        let latest = DoctorSessionEnvReport {
            repaired: vec![repair],
            path_changed: true,
            final_path: Some("/tmp:/usr/bin:/bin".to_string()),
            notes: vec!["systemd note".to_string()],
        };

        merge_session_env_reports(&mut current, latest);

        assert_eq!(current.repaired.len(), 1);
        assert_eq!(current.notes.len(), 1);
        assert!(current.path_changed);
        assert_eq!(current.final_path.as_deref(), Some("/tmp:/usr/bin:/bin"));
    }

    #[test]
    fn display_topology_cache_expires_after_short_ttl() {
        let now = Instant::now();
        let cache = Arc::new(StdMutex::new(Some(DisplayTopologyCache {
            updated_at: now - Duration::from_millis(500),
            displays: vec![test_display("test:primary")],
        })));

        let cached = cached_display_topology(&cache, now).expect("fresh cache should be used");
        assert_eq!(cached[0].display_id, "test:primary");

        *cache.lock().expect("cache lock") = Some(DisplayTopologyCache {
            updated_at: now - DISPLAY_TOPOLOGY_CACHE_TTL - Duration::from_millis(1),
            displays: vec![test_display("test:stale")],
        });

        assert!(cached_display_topology(&cache, now).is_none());
    }

    #[test]
    fn capture_never_disables_x11_still_capture() {
        let environment = EnvironmentInfo {
            session_kind: SessionKind::X11,
            compositor: Some("x11-xorg".to_string()),
            desktop_environment: None,
            capture_backend: CaptureBackendKind::X11,
            input_backend: InputBackendKind::XTest,
            semantic_backend: SemanticBackendKind::Atspi,
            portal_capabilities: PortalCapabilities {
                screencast_version: None,
                remote_desktop_version: None,
                screenshot_version: None,
                available_source_types: None,
                available_cursor_modes: None,
                available_device_types: None,
            },
            xdg_session_type: Some("x11".to_string()),
            display: Some(":0".to_string()),
            wayland_display: None,
            displays: Vec::new(),
        };

        assert!(!should_attempt_x11_capture(
            CaptureScreenMode::Never,
            &environment
        ));
        assert!(should_attempt_x11_capture(
            CaptureScreenMode::Always,
            &environment
        ));
    }

    #[test]
    fn matches_app_selector_by_window_title() {
        let app = AppInfo {
            app_id: "app-1".to_string(),
            name: "zenity".to_string(),
            pid: Some(123),
            executable: Some("zenity".to_string()),
            desktop_file_id: Some("zenity.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("GTK".to_string()),
            window_title: Some("sky-cua zenity smoke".to_string()),
            is_focused_candidate: false,
        };
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: Some("zenity.desktop".to_string()),
            window_title: Some("zenity smoke".to_string()),
            name: None,
        };
        assert!(matches_selector(&app, &selector));
    }

    #[test]
    fn matches_selector_case_insensitively_for_titles_and_names() {
        let app = AppInfo {
            app_id: "app-1".to_string(),
            name: "Zenity".to_string(),
            pid: Some(123),
            executable: Some("zenity".to_string()),
            desktop_file_id: Some("zenity.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("GTK".to_string()),
            window_title: Some("Sky-CUA Pointer Smoke".to_string()),
            is_focused_candidate: false,
        };
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: None,
            window_title: Some("pointer smoke".to_string()),
            name: Some("zenity".to_string()),
        };
        assert!(matches_selector(&app, &selector));
    }

    #[test]
    fn summarizes_selector_fields() {
        let selector = AppSelector {
            app_id: Some("app-1".to_string()),
            desktop_file_id: None,
            window_title: Some("demo".to_string()),
            name: None,
        };
        assert_eq!(
            selector_summary(&selector),
            "app_id=app-1, window_title=demo"
        );
    }

    #[test]
    fn matches_x11_window_to_accessible_app_by_pid() {
        let app = AppInfo {
            app_id: "accessible-1".to_string(),
            name: "Discord".to_string(),
            pid: Some(1234),
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("Electron".to_string()),
            window_title: Some("@Sky - Discord".to_string()),
            is_focused_candidate: false,
        };
        let window = X11WindowInfo {
            window_id: "0x2400006".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x2400006".to_string(),
                name: "discord".to_string(),
                pid: Some(1234),
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("@Sky - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };
        assert!(x11_window_matches_app(&window, &app));
    }

    #[test]
    fn creates_a_synthetic_root_element_for_x11_fallback_windows() {
        let window = X11WindowInfo {
            window_id: "0x3800030".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: AppInfo {
                app_id: "x11:0x3800030".to_string(),
                name: "Xmessage".to_string(),
                pid: None,
                executable: None,
                desktop_file_id: Some("xmessage.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua xmessage probe".to_string()),
                is_focused_candidate: true,
            },
            bounds: Some(RectF {
                x: 100.0,
                y: 200.0,
                width: 320.0,
                height: 180.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            workspace: None,
            child_regions: vec![
                X11WindowRegion {
                    window_id: "0x3800031".to_string(),
                    parent_window_id: None,
                    depth: 1,
                    name: None,
                    bounds: RectF {
                        x: 100.0,
                        y: 200.0,
                        width: 320.0,
                        height: 180.0,
                        space: CoordinateSpace::DesktopLogical,
                    },
                },
                X11WindowRegion {
                    window_id: "0x3800032".to_string(),
                    parent_window_id: Some("0x3800031".to_string()),
                    depth: 2,
                    name: Some("OK".to_string()),
                    bounds: RectF {
                        x: 180.0,
                        y: 330.0,
                        width: 48.0,
                        height: 24.0,
                        space: CoordinateSpace::DesktopLogical,
                    },
                },
            ],
        };

        let elements = x11_window_elements(&window);
        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0].role, "window");
        assert_eq!(
            elements[0].bounds.as_ref().map(|rect| rect.width),
            Some(320.0)
        );
        assert!(elements[0].state_flags.iter().any(|flag| flag == "focused"));
        assert_eq!(elements[1].role, "x11_container");
        assert!(
            elements[1]
                .state_flags
                .iter()
                .any(|flag| flag == "container")
        );
        assert_eq!(elements[2].role, "x11_action_region");
        assert_eq!(elements[2].parent_index, Some(1));
        assert!(
            elements[2]
                .state_flags
                .iter()
                .any(|flag| flag == "action_like")
        );
    }

    #[test]
    fn registry_fallback_prefers_refreshed_x11_child_regions() {
        let linux_window = LinuxWindowInfo {
            window_id: "0x3800030".to_string(),
            title: Some("sky-cua xmessage probe".to_string()),
            app_id: Some("xmessage.desktop".to_string()),
            wm_class: Some("Xmessage".to_string()),
            pid: None,
            bounds: Some(RectF {
                x: 100.0,
                y: 200.0,
                width: 320.0,
                height: 180.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: Some("xwayland".to_string()),
            backend: "x11".to_string(),
            terminal: None,
        };
        let x11_window = X11WindowInfo {
            window_id: "0x3800030".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: app_from_linux_window(&linux_window),
            bounds: linux_window.bounds.clone(),
            workspace: None,
            child_regions: vec![X11WindowRegion {
                window_id: "0x3800032".to_string(),
                parent_window_id: Some("0x3800030".to_string()),
                depth: 1,
                name: Some("OK".to_string()),
                bounds: RectF {
                    x: 180.0,
                    y: 330.0,
                    width: 48.0,
                    height: 24.0,
                    space: CoordinateSpace::DesktopLogical,
                },
            }],
        };

        let elements = fallback_window_elements_with_x11_detail(&linux_window, Some(&x11_window));

        assert_eq!(elements.len(), 2);
        assert_eq!(elements[1].role, "x11_action_region");
        assert_eq!(elements[1].parent_index, Some(0));
    }

    #[test]
    fn creates_structural_anchor_regions_for_kwin_fallback_windows() {
        let window = LinuxWindowInfo {
            window_id: "kwin:{tidal-window}".to_string(),
            title: Some("TIDAL Hi-Fi".to_string()),
            app_id: Some("tidal-hifi.desktop".to_string()),
            wm_class: Some("TIDAL".to_string()),
            pid: Some(4242),
            bounds: Some(RectF {
                x: 100.0,
                y: 80.0,
                width: 1280.0,
                height: 820.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "kwin".to_string(),
            terminal: None,
        };

        let elements = linux_window_elements(&window);
        assert!(elements.len() >= 8);
        assert_eq!(elements[0].role, "window");
        assert!(
            elements[0]
                .state_flags
                .iter()
                .any(|flag| flag == "vision_anchor")
        );
        assert_eq!(elements[1].role, "wayland_header_band");
        assert_eq!(elements[1].parent_index, Some(0));
        assert!(
            elements[1]
                .description
                .as_deref()
                .is_some_and(|description| description.contains("screenshot"))
        );
        assert!(elements.iter().any(|element| {
            element.role == "wayland_search_candidate"
                && element.state_flags.iter().any(|flag| flag == "search_like")
                && element.state_flags.iter().any(|flag| flag == "text_like")
        }));
        assert!(elements.iter().any(|element| {
            element.role == "wayland_sidebar_region"
                && element
                    .state_flags
                    .iter()
                    .any(|flag| flag == "navigation_like")
        }));
        assert!(elements.iter().any(|element| {
            element.role == "wayland_list_candidate"
                && element.state_flags.iter().any(|flag| flag == "list_like")
                && element
                    .state_flags
                    .iter()
                    .any(|flag| flag == "vision_anchor")
        }));
        assert!(elements.iter().any(|element| {
            element.role == "wayland_row_band_candidate"
                && element.state_flags.iter().any(|flag| flag == "row_like")
                && element.state_flags.iter().any(|flag| flag == "text_like")
        }));
        assert!(
            elements
                .iter()
                .filter(|element| element.role == "wayland_row_band_candidate")
                .count()
                >= 4
        );
        assert!(elements.iter().any(|element| {
            element.role == "wayland_action_cluster_candidate"
                && element.state_flags.iter().any(|flag| flag == "action_like")
                && element.state_flags.iter().any(|flag| flag == "menu_like")
        }));
        assert!(
            elements
                .iter()
                .filter(|element| element.role == "wayland_action_cluster_candidate")
                .count()
                >= 2
        );
        assert!(
            elements
                .iter()
                .any(|element| element.role == "wayland_main_region")
        );
    }

    #[test]
    fn linux_fallback_snapshot_preserves_doctor_report() {
        let environment = wayland_pipewire_environment();
        let capabilities = LinuxDesktopBackend::capabilities(&environment);
        let report = crate::doctor::build_doctor_report(
            environment.clone(),
            DoctorSessionEnvReport::default(),
        );
        let window = LinuxWindowInfo {
            window_id: "kwin:{tidal-window}".to_string(),
            title: Some("TIDAL Hi-Fi".to_string()),
            app_id: Some("tidal-hifi.desktop".to_string()),
            wm_class: Some("TIDAL".to_string()),
            pid: Some(4242),
            bounds: Some(RectF {
                x: 100.0,
                y: 80.0,
                width: 1280.0,
                height: 820.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "kwin".to_string(),
            terminal: None,
        };

        let snapshot = linux_fallback_snapshot(
            "snap-1".to_string(),
            environment,
            capabilities,
            None,
            DiagnosticBuilder::new(),
            Some(report.clone()),
            window,
        );

        assert_eq!(snapshot.doctor_report, Some(report));
    }

    #[test]
    fn registry_window_app_does_not_invent_executable() {
        let app = app_from_linux_window(&LinuxWindowInfo {
            window_id: "kwin:{tidal-window}".to_string(),
            title: Some("TIDAL Hi-Fi".to_string()),
            app_id: Some("tidal-hifi.desktop".to_string()),
            wm_class: Some("TIDAL".to_string()),
            pid: Some(4242),
            bounds: None,
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "kwin".to_string(),
            terminal: None,
        });

        assert_eq!(app.desktop_file_id.as_deref(), Some("tidal-hifi.desktop"));
        assert_eq!(app.executable, None);
    }

    #[test]
    fn matches_x11_window_to_accessible_app_by_class_when_titles_do_not_help() {
        let app = AppInfo {
            app_id: "accessible-2".to_string(),
            name: "Code".to_string(),
            pid: None,
            executable: Some("code".to_string()),
            desktop_file_id: Some("code.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("Electron".to_string()),
            window_title: Some("workspace-a".to_string()),
            is_focused_candidate: false,
        };
        let window = X11WindowInfo {
            window_id: "0x500001".to_string(),
            instance_name: Some("code".to_string()),
            class_name: Some("Code".to_string()),
            app: AppInfo {
                app_id: "x11:0x500001".to_string(),
                name: "Code".to_string(),
                pid: None,
                executable: None,
                desktop_file_id: None,
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("totally different title".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };

        assert!(x11_window_matches_app(&window, &app));
    }

    #[test]
    fn does_not_match_an_x11_window_by_title_alone() {
        let app = AppInfo {
            app_id: "accessible-2b".to_string(),
            name: "kaccess".to_string(),
            pid: None,
            executable: Some("kaccess".to_string()),
            desktop_file_id: Some("kaccess.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("Qt".to_string()),
            window_title: Some("sky-cua xmessage probe".to_string()),
            is_focused_candidate: false,
        };
        let window = X11WindowInfo {
            window_id: "0x500002".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: AppInfo {
                app_id: "x11:0x500002".to_string(),
                name: "Xmessage".to_string(),
                pid: None,
                executable: Some("xmessage".to_string()),
                desktop_file_id: Some("xmessage.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua xmessage probe".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };

        assert!(!x11_window_matches_app(&window, &app));
    }

    #[test]
    fn selector_prefers_exact_window_title_over_broader_desktop_match() {
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: Some("xmessage.desktop".to_string()),
            window_title: Some("selector beta".to_string()),
            name: None,
        };
        let alpha = X11WindowInfo {
            window_id: "0x500010".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: AppInfo {
                app_id: "x11:0x500010".to_string(),
                name: "Xmessage".to_string(),
                pid: None,
                executable: Some("xmessage".to_string()),
                desktop_file_id: Some("xmessage.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua selector alpha".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };
        let beta = X11WindowInfo {
            window_id: "0x500011".to_string(),
            instance_name: Some("xmessage".to_string()),
            class_name: Some("Xmessage".to_string()),
            app: AppInfo {
                app_id: "x11:0x500011".to_string(),
                name: "Xmessage".to_string(),
                pid: None,
                executable: Some("xmessage".to_string()),
                desktop_file_id: Some("xmessage.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("sky-cua selector beta".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };

        let matched =
            select_x11_window(&[alpha, beta.clone()], &selector).expect("selector should match");
        assert_eq!(matched.window_id, beta.window_id);
    }

    #[test]
    fn selector_prefers_focused_x11_window_when_only_desktop_id_is_given() {
        let selector = AppSelector {
            app_id: None,
            desktop_file_id: Some("discord.desktop".to_string()),
            window_title: None,
            name: None,
        };
        let background = X11WindowInfo {
            window_id: "0x500012".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x500012".to_string(),
                name: "discord".to_string(),
                pid: None,
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Friends - Discord".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };
        let focused = X11WindowInfo {
            window_id: "0x500013".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x500013".to_string(),
                name: "discord".to_string(),
                pid: None,
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Project Foxglove - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };

        let matched = select_x11_window(&[background, focused.clone()], &selector)
            .expect("selector should match");
        assert_eq!(matched.window_id, focused.window_id);
    }

    #[test]
    fn prefers_the_best_x11_window_match_when_multiple_windows_share_a_process() {
        let app = AppInfo {
            app_id: "accessible-3".to_string(),
            name: "Discord".to_string(),
            pid: Some(4321),
            executable: Some("discord".to_string()),
            desktop_file_id: Some("discord.desktop".to_string()),
            app_user_model_id: None,
            window_handle: None,
            toolkit_guess: Some("Electron".to_string()),
            window_title: Some("Project Foxglove - Discord".to_string()),
            is_focused_candidate: false,
        };
        let weaker = X11WindowInfo {
            window_id: "0x600001".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x600001".to_string(),
                name: "discord".to_string(),
                pid: Some(4321),
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Friends - Discord".to_string()),
                is_focused_candidate: false,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };
        let stronger = X11WindowInfo {
            window_id: "0x600002".to_string(),
            instance_name: Some("discord".to_string()),
            class_name: Some("discord".to_string()),
            app: AppInfo {
                app_id: "x11:0x600002".to_string(),
                name: "discord".to_string(),
                pid: Some(4321),
                executable: Some("discord".to_string()),
                desktop_file_id: Some("discord.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("XWayland".to_string()),
                window_title: Some("Project Foxglove - Discord".to_string()),
                is_focused_candidate: true,
            },
            bounds: None,
            workspace: None,
            child_regions: Vec::new(),
        };

        let windows = [weaker.clone(), stronger.clone()];
        let matched = best_x11_window_match(&windows, &app).expect("a best match should be found");
        assert_eq!(matched.window_id, stronger.window_id);
    }
}
