//! App-management tools: current foreground app, launchable inventory, launch,
//! open-intent, force-stop, install (single/split/multi-package), and settings.
//!
//! App management routes companion-first where the plan allows (foreground app),
//! but the launchable inventory and the mutating launches go through ADB because
//! Android package visibility can hide apps from a normal companion query and ADB
//! is the reliable baseline. Install reports which strategy it used and a
//! structured failure class on error.

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneAppForceStopRequest, PhoneAppInfo, PhoneAppInstallMode,
    PhoneAppInstallRequest, PhoneAppLaunchRequest, PhoneAppListRequest, PhoneAppOpenIntentRequest,
    PhoneAppResponse, PhoneAppResponseKind, PhoneBackendKind, PhoneInstallStrategy,
    PhoneOpenSettingsRequest, PhoneSessionSelector,
};

use super::{PhoneManager, no_session_diagnostic, selector_ids};
use crate::phone::adb;
use crate::phone::companion::protocol::AppOp;

impl PhoneManager {
    // ===================================================================
    // App management
    // ===================================================================

    /// `phone_app_current`: companion `current_app` when reachable, else ADB
    /// `dumpsys` foreground parse.
    pub(super) async fn app_current(
        &mut self,
        selector: &PhoneSessionSelector,
    ) -> PhoneAppResponse {
        let Some(session_id) = self.resolve_session_id(selector) else {
            return app_no_session(selector, PhoneAppResponseKind::Current);
        };
        let serial = self.serial_of(&session_id);

        if let Some(app) = self.companion_current_app(&session_id).await {
            return PhoneAppResponse {
                session_id,
                serial,
                kind: PhoneAppResponseKind::Current,
                backend: PhoneBackendKind::Companion,
                success: true,
                current_app: Some(app),
                apps: Vec::new(),
                truncated: false,
                install_strategy: None,
                diagnostics: Vec::new(),
            };
        }

        match adb::foreground_app(self.runner.as_ref(), self.configured_adb_path(), &serial).await {
            Ok(Some(app)) => PhoneAppResponse {
                session_id,
                serial,
                kind: PhoneAppResponseKind::Current,
                backend: PhoneBackendKind::Adb,
                success: true,
                current_app: Some(PhoneAppInfo {
                    package_name: app.package,
                    label: None,
                    activity: app.activity,
                    version_name: None,
                    version_code: None,
                    launchable: true,
                    system_app: false,
                }),
                apps: Vec::new(),
                truncated: false,
                install_strategy: None,
                diagnostics: Vec::new(),
            },
            Ok(None) => app_failure(
                session_id,
                serial,
                PhoneAppResponseKind::Current,
                DiagnosticEntry {
                    code: "PhoneForegroundUnknown".to_string(),
                    message: "no foreground app could be determined".to_string(),
                    details: None,
                },
            ),
            Err(error) => app_failure(
                session_id,
                serial,
                PhoneAppResponseKind::Current,
                adb::command_error_diagnostic("adb shell dumpsys window", &error),
            ),
        }
    }

    /// `phone_app_list`: ADB `pm list packages` (companion visibility can hide
    /// apps, so the ADB inventory is the reliable baseline).
    pub(super) async fn app_list(&mut self, request: PhoneAppListRequest) -> PhoneAppResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            return app_no_session(&request.session, PhoneAppResponseKind::List);
        };
        let serial = self.serial_of(&session_id);
        match adb::list_packages(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &serial,
            request.include_system,
        )
        .await
        {
            Ok(packages) => {
                let limit = request.limit.unwrap_or(packages.len());
                let truncated = packages.len() > limit;
                let apps = packages
                    .into_iter()
                    .take(limit)
                    .map(|package_name| PhoneAppInfo {
                        package_name,
                        label: None,
                        activity: None,
                        version_name: None,
                        version_code: None,
                        launchable: false,
                        system_app: request.include_system,
                    })
                    .collect();
                PhoneAppResponse {
                    session_id,
                    serial,
                    kind: PhoneAppResponseKind::List,
                    backend: PhoneBackendKind::Adb,
                    success: true,
                    current_app: None,
                    apps,
                    truncated,
                    install_strategy: None,
                    diagnostics: Vec::new(),
                }
            }
            Err(error) => app_failure(
                session_id,
                serial,
                PhoneAppResponseKind::List,
                adb::command_error_diagnostic("adb shell pm list packages", &error),
            ),
        }
    }

    /// `phone_app_launch`: companion `app_op(launch)` when the session has a
    /// reachable companion, else the ADB monkey launcher intent. A companion
    /// transport/per-method failure degrades to the ADB path so a launch is never
    /// silently lost.
    pub(super) async fn app_launch(&mut self, request: PhoneAppLaunchRequest) -> PhoneAppResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            return app_no_session(&request.session, PhoneAppResponseKind::Launch);
        };
        let serial = self.serial_of(&session_id);

        if self.companion_app_management(&session_id)
            && let Some(success) = self
                .companion_app_op(
                    &session_id,
                    AppOp::Launch,
                    Some(request.package_name.clone()),
                    None,
                )
                .await
        {
            return companion_app_result(
                session_id,
                serial,
                PhoneAppResponseKind::Launch,
                success,
                "phone_app_launch",
            );
        }

        let outcome = adb::launch_package(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &serial,
            &request.package_name,
        )
        .await;
        self.simple_app_result(session_id, serial, PhoneAppResponseKind::Launch, outcome)
    }

    /// `phone_app_open_intent`: companion `app_op(open_intent)` when reachable,
    /// else ADB `am start VIEW`. Same companion-preferred / ADB-fallback routing
    /// as launch.
    pub(super) async fn app_open_intent(
        &mut self,
        request: PhoneAppOpenIntentRequest,
    ) -> PhoneAppResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            return app_no_session(&request.session, PhoneAppResponseKind::OpenIntent);
        };
        let serial = self.serial_of(&session_id);

        if self.companion_app_management(&session_id)
            && let Some(success) = self
                .companion_app_op(
                    &session_id,
                    AppOp::OpenIntent,
                    request.package_name.clone(),
                    Some(request.intent_uri.clone()),
                )
                .await
        {
            return companion_app_result(
                session_id,
                serial,
                PhoneAppResponseKind::OpenIntent,
                success,
                "phone_app_open_intent",
            );
        }

        let outcome = adb::start_intent(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &serial,
            &request.intent_uri,
            request.package_name.as_deref(),
        )
        .await;
        self.simple_app_result(
            session_id,
            serial,
            PhoneAppResponseKind::OpenIntent,
            outcome,
        )
    }

    /// `phone_app_force_stop`: ADB `am force-stop`.
    pub(super) async fn app_force_stop(
        &mut self,
        request: PhoneAppForceStopRequest,
    ) -> PhoneAppResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            return app_no_session(&request.session, PhoneAppResponseKind::ForceStop);
        };
        let serial = self.serial_of(&session_id);
        let outcome = adb::force_stop(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &serial,
            &request.package_name,
        )
        .await;
        self.simple_app_result(session_id, serial, PhoneAppResponseKind::ForceStop, outcome)
    }

    /// `phone_app_install`: ADB single / split / multi-package install, reporting
    /// the install strategy and a structured failure class on error.
    pub(super) async fn app_install(
        &mut self,
        request: PhoneAppInstallRequest,
    ) -> PhoneAppResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            return app_no_session(&request.session, PhoneAppResponseKind::Install);
        };
        let serial = self.serial_of(&session_id);
        if request.apk_paths.is_empty() {
            return app_failure(
                session_id,
                serial,
                PhoneAppResponseKind::Install,
                DiagnosticEntry {
                    code: "PhoneInstallNoApk".to_string(),
                    message: "no APK path supplied for phone_app_install".to_string(),
                    details: None,
                },
            );
        }

        // Map the requested install mode to the strategy the response echoes, so
        // the caller can tell a single-APK install from a split/multi-package one
        // instead of inferring it from the request it sent. Set on the success arm
        // only; failures and other kinds leave `install_strategy` as `None`.
        let strategy = match request.mode {
            PhoneAppInstallMode::Single => PhoneInstallStrategy::Single,
            PhoneAppInstallMode::Multiple => PhoneInstallStrategy::Multiple,
            PhoneAppInstallMode::MultiPackage => PhoneInstallStrategy::MultiPackage,
        };

        let result = match request.mode {
            PhoneAppInstallMode::Single => {
                adb::install_single(
                    self.runner.as_ref(),
                    self.configured_adb_path(),
                    &serial,
                    &request.apk_paths[0],
                    request.reinstall,
                    request.allow_downgrade,
                    request.allow_test_apk,
                    request.grant_runtime_permissions,
                )
                .await
            }
            PhoneAppInstallMode::Multiple => {
                adb::install_multiple(
                    self.runner.as_ref(),
                    self.configured_adb_path(),
                    &serial,
                    &request.apk_paths,
                    request.reinstall,
                    request.allow_downgrade,
                    request.allow_test_apk,
                    request.grant_runtime_permissions,
                )
                .await
            }
            PhoneAppInstallMode::MultiPackage => {
                adb::install_multi_package(
                    self.runner.as_ref(),
                    self.configured_adb_path(),
                    &serial,
                    &request.apk_paths,
                    request.reinstall,
                    request.allow_downgrade,
                    request.allow_test_apk,
                    request.grant_runtime_permissions,
                )
                .await
            }
        };

        match result {
            Ok(outcome) if outcome.success => PhoneAppResponse {
                session_id,
                serial,
                kind: PhoneAppResponseKind::Install,
                backend: PhoneBackendKind::Adb,
                success: true,
                current_app: None,
                apps: Vec::new(),
                truncated: false,
                install_strategy: Some(strategy),
                diagnostics: Vec::new(),
            },
            Ok(outcome) => app_failure(
                session_id,
                serial,
                PhoneAppResponseKind::Install,
                DiagnosticEntry {
                    code: outcome
                        .failure_class
                        .unwrap_or_else(|| "PhoneInstallFailed".to_string()),
                    message: outcome.message,
                    details: None,
                },
            ),
            Err(error) => app_failure(
                session_id,
                serial,
                PhoneAppResponseKind::Install,
                adb::command_error_diagnostic("adb install", &error),
            ),
        }
    }

    /// `phone_open_settings`: ADB `am start` for the requested settings screen.
    pub(super) async fn open_settings(
        &mut self,
        request: PhoneOpenSettingsRequest,
    ) -> PhoneAppResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            return app_no_session(&request.session, PhoneAppResponseKind::OpenSettings);
        };
        let serial = self.serial_of(&session_id);
        let outcome = adb::open_settings(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &serial,
            request.screen,
            request.package_name.as_deref(),
        )
        .await;
        self.simple_app_result(
            session_id,
            serial,
            PhoneAppResponseKind::OpenSettings,
            outcome,
        )
    }

    // ===================================================================
    // App helpers
    // ===================================================================

    /// Whether companion app management (`app_op`) should be preferred for a
    /// session: a fresh cached profile with a reachable companion. Launch and
    /// open-intent route through the companion first; the ADB path is the
    /// fallback. Force-stop never consults this (a non-privileged companion cannot
    /// force-stop).
    fn companion_app_management(&self, session_id: &str) -> bool {
        self.profiles
            .get(session_id)
            .map(|cached| !cached.profile.stale && cached.profile.companion.rpc_reachable)
            .unwrap_or(false)
            && self
                .sessions
                .get(session_id)
                .is_some_and(|entry| entry.companion.is_some())
    }

    /// Dispatch an `app_op` through the live companion RPC client.
    ///
    /// Returns `Some(success)` when the companion handled the op (the launch was
    /// dispatched or definitively rejected by the app); `None` when the caller
    /// should fall back to ADB. A fallback-worthy transport/auth/version failure
    /// drops the companion runtime and invalidates the cached capability so later
    /// actions re-route to ADB; a per-method application error (e.g. the companion
    /// cannot satisfy this op) also falls back to ADB without claiming success.
    async fn companion_app_op(
        &mut self,
        session_id: &str,
        op: AppOp,
        package: Option<String>,
        intent_uri: Option<String>,
    ) -> Option<bool> {
        let entry = self.sessions.get_mut(session_id)?;
        let runtime = entry.companion.as_mut()?;
        match runtime.client.app_op(op, package, intent_uri).await {
            Ok(result) => Some(result.ok),
            Err(error) => {
                if error.is_fallback() {
                    entry.companion = None;
                    self.invalidate_companion(session_id);
                }
                // Per-method or transport failure: fall back to the ADB path.
                None
            }
        }
    }

    /// Foreground app from the companion, if reachable.
    async fn companion_current_app(&mut self, session_id: &str) -> Option<PhoneAppInfo> {
        let entry = self.sessions.get_mut(session_id)?;
        let runtime = entry.companion.as_mut()?;
        let app = runtime.client.current_app().await.ok()?;
        Some(PhoneAppInfo {
            package_name: app.package,
            label: app.label,
            activity: app.activity,
            version_name: None,
            version_code: None,
            launchable: true,
            system_app: false,
        })
    }

    /// Current foreground app for `phone_observe`, companion-first then ADB.
    pub(super) async fn current_app_info(
        &mut self,
        session_id: &str,
        serial: &str,
    ) -> Result<Option<PhoneAppInfo>, DiagnosticEntry> {
        if let Some(app) = self.companion_current_app(session_id).await {
            return Ok(Some(app));
        }
        match adb::foreground_app(self.runner.as_ref(), self.configured_adb_path(), serial).await {
            Ok(Some(app)) => Ok(Some(PhoneAppInfo {
                package_name: app.package,
                label: None,
                activity: app.activity,
                version_name: None,
                version_code: None,
                launchable: true,
                system_app: false,
            })),
            Ok(None) => Ok(None),
            Err(error) => Err(adb::command_error_diagnostic(
                "adb shell dumpsys window",
                &error,
            )),
        }
    }

    /// Build an app response from a simple ADB `InputOutcome`.
    fn simple_app_result(
        &self,
        session_id: String,
        serial: String,
        kind: PhoneAppResponseKind,
        outcome: Result<adb::InputOutcome, crate::phone::command::CommandError>,
    ) -> PhoneAppResponse {
        match outcome {
            Ok(outcome) if outcome.success => PhoneAppResponse {
                session_id,
                serial,
                kind,
                backend: PhoneBackendKind::Adb,
                success: true,
                current_app: None,
                apps: Vec::new(),
                truncated: false,
                install_strategy: None,
                diagnostics: Vec::new(),
            },
            Ok(outcome) => app_failure(
                session_id,
                serial,
                kind,
                DiagnosticEntry {
                    code: "PhoneAppActionFailed".to_string(),
                    message: outcome.message,
                    details: None,
                },
            ),
            Err(error) => app_failure(
                session_id,
                serial,
                kind,
                adb::command_error_diagnostic("adb", &error),
            ),
        }
    }
}

fn app_no_session(selector: &PhoneSessionSelector, kind: PhoneAppResponseKind) -> PhoneAppResponse {
    let (session_id, serial) = selector_ids(selector);
    PhoneAppResponse {
        session_id,
        serial,
        kind,
        backend: PhoneBackendKind::None,
        success: false,
        current_app: None,
        apps: Vec::new(),
        truncated: false,
        install_strategy: None,
        diagnostics: vec![no_session_diagnostic(selector)],
    }
}

/// Build an app response for a companion-handled op. On success the backend is
/// `Companion`; a companion `ok=false` is a structured app-action failure.
fn companion_app_result(
    session_id: String,
    serial: String,
    kind: PhoneAppResponseKind,
    success: bool,
    action: &str,
) -> PhoneAppResponse {
    if success {
        PhoneAppResponse {
            session_id,
            serial,
            kind,
            backend: PhoneBackendKind::Companion,
            success: true,
            current_app: None,
            apps: Vec::new(),
            truncated: false,
            install_strategy: None,
            diagnostics: Vec::new(),
        }
    } else {
        app_failure(
            session_id,
            serial,
            kind,
            DiagnosticEntry {
                code: "PhoneAppActionFailed".to_string(),
                message: format!("companion reported {action} did not succeed"),
                details: None,
            },
        )
    }
}

fn app_failure(
    session_id: String,
    serial: String,
    kind: PhoneAppResponseKind,
    diagnostic: DiagnosticEntry,
) -> PhoneAppResponse {
    PhoneAppResponse {
        session_id,
        serial,
        kind,
        backend: PhoneBackendKind::None,
        success: false,
        current_app: None,
        apps: Vec::new(),
        truncated: false,
        install_strategy: None,
        diagnostics: vec![diagnostic],
    }
}
