use super::elements::{linux_fallback_snapshot, selector_or_window_summary, window_summary};
use super::*;

#[async_trait::async_trait]
impl DesktopBackend for LinuxDesktopBackend {
    async fn prepare_automation_permissions(&self) -> Result<(), BackendError> {
        self.portal.preauthorize_permissions().await;
        Ok(())
    }

    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        self.probe_environment_with_display_report()
            .await
            .map(|(environment, _display_topology)| environment)
    }

    async fn doctor(&self) -> Result<sky_cua_platform::model::DoctorReport, BackendError> {
        let (environment, display_topology) = self.probe_environment_with_display_report().await?;
        let session_presence = self.session_presence.doctor_report().await;
        Ok(crate::doctor::build_doctor_report_with_session_presence(
            environment,
            Some(display_topology),
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
        let environment = self.probe_environment().await?;
        Ok(crate::setup::setup_window_targeting_report(&environment).await)
    }

    async fn launch_application(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<sky_cua_platform::model::LaunchedApplication, BackendError> {
        // The child INHERITS the daemon environment verbatim. In the isolated
        // desktop that environment is the sandbox (`DISPLAY=:N`, sandbox session
        // bus, `QT_QPA_PLATFORM=xcb`, `GDK_BACKEND=x11`, no `WAYLAND_DISPLAY`),
        // and pure inheritance is the leak-safety guarantee: do NOT set or mutate
        // any display/session variable here, or a launched toolkit app could
        // escape onto the user's live session.
        let mut process = tokio::process::Command::new(command);
        process
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        // Detach the child into its own session so it outlives this request and
        // is never reaped by, or signalled with, the daemon's process group.
        unsafe {
            process.pre_exec(|| {
                // SAFETY: async-signal-safe libc call in the forked child before
                // exec; the only failure mode (already a session leader) is
                // benign for a freshly forked process.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = process.spawn().map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to launch application '{command}': {error}"),
            )
        })?;

        let pid = child.id().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("launched application '{command}' has no pid (already exited)"),
            )
        })?;

        // Do NOT await the child: it is a detached, long-lived desktop app.
        // `setsid` made it a new session/process-group leader (detached from the
        // daemon's controlling terminal and process group) but the daemon stays
        // its parent. Dropping the tokio `Child` hands it to tokio's orphan
        // reaper, which collects it on SIGCHLD when it eventually exits. This
        // relies on `tokio::process` specifically — `std::process::Command` has
        // no auto-reaper and would leak a zombie for the daemon's lifetime.
        std::mem::drop(child);

        Ok(sky_cua_platform::model::LaunchedApplication { pid })
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

    async fn list_displays(&self) -> Result<Vec<DisplayInfo>, BackendError> {
        // Reuse the same discovery (and cache) the doctor/screenshot paths use.
        // An unsupported environment yields an empty topology rather than an
        // error, which callers treat as "topology unknown" and fall back from.
        let (environment, _report) = self.probe_environment_with_display_report().await?;
        Ok(environment.displays)
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
        let (environment, display_topology) = self.probe_environment_with_display_report().await?;
        require_supported_environment(&environment)?;
        let capabilities = Self::capabilities(&environment);
        let session_presence = self.session_presence.doctor_report().await;
        let doctor_report = crate::doctor::build_doctor_report_with_session_presence(
            environment.clone(),
            Some(display_topology.clone()),
            self.session_env_report(),
            Some(session_presence),
        );
        let mut diagnostics = DiagnosticBuilder::new();
        push_display_topology_diagnostics(&environment, &display_topology, &mut diagnostics);
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
                    let capture = self
                        .get_app_state_capture(
                            &snapshot_id,
                            capture_screen,
                            &environment,
                            Some(&window),
                            &mut diagnostics,
                        )
                        .await?;
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
                let capture = self
                    .get_app_state_capture(
                        &snapshot_id,
                        capture_screen,
                        &environment,
                        None,
                        &mut diagnostics,
                    )
                    .await?;
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
                let capture = self
                    .get_app_state_capture(
                        &snapshot_id,
                        capture_screen,
                        &environment,
                        Some(&window),
                        &mut diagnostics,
                    )
                    .await?;
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
            let capture = self
                .get_app_state_capture(
                    &snapshot_id,
                    capture_screen,
                    &environment,
                    None,
                    &mut diagnostics,
                )
                .await?;
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
                let app = app_from_linux_window(&window);
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!(
                        "The selected {} window is visible through the window registry, but no accessible AT-SPI application tree matched it",
                        window.backend
                    ),
                    Some(selector_or_window_summary(Some(selector), &app)),
                );
                let capture = self
                    .get_app_state_capture(
                        &snapshot_id,
                        capture_screen,
                        &environment,
                        Some(&window),
                        &mut diagnostics,
                    )
                    .await?;
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
                let capture = self
                    .get_app_state_capture(
                        &snapshot_id,
                        capture_screen,
                        &environment,
                        Some(&window),
                        &mut diagnostics,
                    )
                    .await?;
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
        let focused_window = registry_windows
            .iter()
            .find(|window| linux_window_matches_app(window, &chosen_app.info));
        focused_app.display = focused_window.and_then(|window| window.display.clone());
        let focused_app = Some(focused_app);

        let (elements, snapshot_diags) = self
            .at_spi_call_with_timeout(snapshot_for_app(&connection, &chosen_app))
            .await?;
        for entry in snapshot_diags {
            diagnostics.push(
                BackendErrorCode::AccessibilityCoverageLimited,
                entry.message,
                entry.details,
            );
        }

        let capture = self
            .get_app_state_capture(
                &snapshot_id,
                capture_screen,
                &environment,
                focused_window,
                &mut diagnostics,
            )
            .await?;

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
    ) -> Result<AppStateSnapshot, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        let snapshot_id = new_snapshot_id();
        let mut environment = self.probe_environment_base().await?;
        let display_topology = self.enrich_environment_displays(&mut environment).await;
        require_supported_environment(&environment)?;
        let capabilities = Self::capabilities(&environment);
        let mut diagnostics = DiagnosticBuilder::new();
        push_display_topology_diagnostics(&environment, &display_topology, &mut diagnostics);

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
            let display_ref = DisplayRef::from(&display);
            capture_scope = CaptureScope::Display;
            capture_display = Some(display_ref.clone());
            capture_target = Some(crate::capture_plan::CaptureRegionTarget {
                desktop_logical_rect: display.logical_rect.clone(),
                capture_scope: CaptureScope::Display,
                display: Some(display_ref),
            });
        } else if let Some(display) = crate::displays::primary_display(&environment.displays) {
            let display_ref = DisplayRef::from(&display);
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
                "Display topology is unavailable, so screenshot fell back to an unscoped desktop capture for an omitted selector.",
                None,
            );
        }

        let mut retried_capture_source_geometry = false;
        let mut capture_plan = match crate::capture_plan::plan_capture(
            &self.portal,
            &snapshot_id,
            CaptureScreenMode::Always,
            &environment,
            capture_target.as_ref(),
            capture_scope.clone(),
            capture_display.clone(),
            true,
            &mut diagnostics,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error)
                if capture_target.is_some()
                    && crate::capture_plan::is_capture_source_geometry_missing(&error) =>
            {
                retried_capture_source_geometry = true;
                diagnostics.push_code(
                    "CaptureSourceGeometryRetry",
                    "RemoteDesktop capture source geometry was missing; resetting the capture session and retrying the targeted screenshot once",
                    Some(error.message.clone()),
                );
                self.portal.reset_session().await;
                crate::capture_plan::plan_capture(
                    &self.portal,
                    &snapshot_id,
                    CaptureScreenMode::Always,
                    &environment,
                    capture_target.as_ref(),
                    capture_scope.clone(),
                    capture_display.clone(),
                    false,
                    &mut diagnostics,
                )
                .await?
            }
            Err(error) => return Err(error),
        };
        if !retried_capture_source_geometry
            && capture_target.is_some()
            && crate::capture_plan::outcome_missing_capture_source_geometry(&capture_plan)
            && environment.input_backend == InputBackendKind::PortalRemoteDesktop
        {
            diagnostics.push_code(
                "CaptureSourceGeometryRetry",
                "RemoteDesktop capture source geometry was missing; resetting the capture session and retrying the targeted screenshot once",
                capture_plan
                    .capture_error
                    .as_ref()
                    .map(|error| error.message.clone()),
            );
            self.portal.reset_session().await;
            capture_plan = crate::capture_plan::plan_capture(
                &self.portal,
                &snapshot_id,
                CaptureScreenMode::Always,
                &environment,
                capture_target.as_ref(),
                capture_scope.clone(),
                capture_display.clone(),
                false,
                &mut diagnostics,
            )
            .await?;
        }
        let mut portal_lifecycle_events = self.portal.take_lifecycle_events().await;
        crate::capture_plan::push_diagnostics(
            &environment,
            capture_plan.capture.as_ref(),
            capture_plan.portal_session_error.as_ref(),
            capture_plan.capture_error.as_ref(),
            &mut diagnostics,
        );
        push_portal_lifecycle_diagnostics(&mut portal_lifecycle_events, &mut diagnostics);
        reject_unactionable_targeted_capture(capture_target.as_ref(), &capture_plan, &environment)?;
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

    async fn execute_cua_action(
        &self,
        mut request: sky_cua_platform::model::CuaActionRequest,
        cancellation: sky_cua_platform::model::CuaCancellation,
    ) -> Result<sky_cua_platform::model::CuaBackendResponse, BackendError> {
        let _ = self.portal.take_lifecycle_events().await;
        let environment = self.probe_environment().await?;
        require_supported_environment(&environment)?;
        if environment.input_backend == InputBackendKind::PortalRemoteDesktop
            && let Some(stream_rect) = self
                .portal
                .primary_stream()
                .await?
                .and_then(|stream| stream.logical_rect)
        {
            cua_desktop_to_portal_stream(&mut request, &stream_rect);
        }
        LinuxActionExecutor::new(self)
            .execute_cua(request, cancellation, environment)
            .await
            .map(|()| sky_cua_platform::model::CuaBackendResponse::Action)
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

    async fn reset_desktop_session_state(&self) {
        // Dropping the cached AT-SPI connection is synchronous (closes the
        // socket, no D-Bus round trip), so it cannot itself hang.
        self.reset_accessibility_connection().await;
        // `reset_session` clears the cached portal session handle
        // synchronously under a write lock *before* attempting a graceful
        // portal `Session.Close()` D-Bus call. That close call can hang on
        // the same unbounded zbus timeout that caused the request this
        // reset is recovering from to wedge in the first place, so bound it
        // here defensively: by the time this timeout could fire the
        // meaningful state (the cached session reference) is already gone,
        // so an elapsed close is a harmless best-effort cleanup, not a
        // correctness issue.
        let _ = tokio::time::timeout(Duration::from_secs(5), self.portal.reset_session()).await;
    }
}

fn cua_desktop_to_portal_stream(
    request: &mut sky_cua_platform::model::CuaActionRequest,
    stream_rect: &RectF,
) {
    use sky_cua_platform::model::CuaActionRequest;
    let map = |x: &mut f64, y: &mut f64| {
        *x -= stream_rect.x;
        *y -= stream_rect.y;
    };
    match request {
        CuaActionRequest::Click { x, y, .. } | CuaActionRequest::Move { x, y, .. } => map(x, y),
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
}

fn push_display_topology_diagnostics(
    environment: &EnvironmentInfo,
    display_topology: &DoctorDisplayTopologyReport,
    diagnostics: &mut DiagnosticBuilder,
) {
    if environment.displays.is_empty() {
        diagnostics.push_code(
            "DisplayTopologyUnavailable",
            "Display topology is unavailable; targeted screenshot crops may not be able to infer capture source geometry.",
            Some(display_probe_details(display_topology)),
        );
    } else if environment.session_kind == sky_cua_platform::model::SessionKind::Wayland
        && display_topology.selected_provider.as_deref() == Some("xrandr")
    {
        diagnostics.push_code(
            "DisplayTopologyInferred",
            "Display topology was inferred from XWayland xrandr while running a Wayland session.",
            Some(display_probe_details(display_topology)),
        );
    }
}

fn display_probe_details(display_topology: &DoctorDisplayTopologyReport) -> String {
    if display_topology.probes.is_empty() {
        return display_topology.detail.clone();
    }
    let probes = display_topology
        .probes
        .iter()
        .map(|probe| {
            let exit_status = probe
                .exit_status
                .map_or_else(|| "none".to_string(), |status| status.to_string());
            let stderr = probe.stderr_snippet.as_deref().unwrap_or("none");
            format!(
                "{}: ok={} timeout={} exit_status={} displays={} stdout_bytes={} stderr={} detail={}",
                probe.provider,
                probe.ok,
                probe.timed_out,
                exit_status,
                probe.display_count,
                probe.stdout_bytes,
                stderr,
                probe.detail
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("{}; {probes}", display_topology.detail)
}

#[cfg(test)]
mod cua_coordinate_tests {
    use super::cua_desktop_to_portal_stream;
    use sky_cua_platform::model::{CoordinateSpace, CuaActionRequest, CuaRequestContext, RectF};

    #[test]
    fn portal_actions_subtract_nonzero_stream_origin() {
        let mut request = CuaActionRequest::Drag {
            context: CuaRequestContext {
                session_id: "session".to_string(),
                turn_id: "turn".to_string(),
                deadline_ms: None,
            },
            from_x: -250.0,
            from_y: 300.0,
            to_x: 450.0,
            to_y: 800.0,
            key: None,
            post_action_sleep_ms: Some(0),
        };
        cua_desktop_to_portal_stream(
            &mut request,
            &RectF {
                x: -320.0,
                y: 180.0,
                width: 1706.67,
                height: 1066.67,
                space: CoordinateSpace::DesktopLogical,
            },
        );
        let CuaActionRequest::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            ..
        } = request
        else {
            panic!("request should remain a drag");
        };
        assert_eq!((from_x, from_y), (70.0, 120.0));
        assert_eq!((to_x, to_y), (770.0, 620.0));
    }
}
