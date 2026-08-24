//! Companion lifecycle: install/update decisioning, `adb forward` + token
//! provisioning, the capability probe, and the `phone_companion_status` /
//! `phone_install_companion` tools.
//!
//! `phone_connect`/`phone_refresh_capabilities` call [`PhoneManager::bootstrap_companion`]
//! to decide install/update from installed-vs-expected identity (never trusting a
//! same-name package with a different signature), optionally run `adb install -r`,
//! set up port forwarding, deliver an ephemeral token through the setup intent,
//! and probe `capabilities`. Any failure degrades to ADB baseline without
//! aborting the session.

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneActionResponse, PhoneBackendKind, PhoneCapabilityProfile,
    PhoneCapabilityRefreshState, PhoneCompanionCapabilities, PhoneCompanionStatusResponse,
    PhoneInstallCompanionRequest, PhoneSessionSelector, PhoneSettingsScreen,
};

use super::{CompanionRuntime, PhoneManager, no_session_diagnostic, selector_ids};
use crate::phone::adb::{
    ACCESSIBILITY_SERVICE_CLASS_SUFFIX, InstallOutcome, NOTIFICATION_LISTENER_CLASS_SUFFIX,
    SecureServiceOutcome, SecureServiceState, ensure_notification_listener,
    ensure_secure_list_service, forward_tcp, install_replace, uninstall_package,
};
use crate::phone::protocol;
use crate::phone::protocol::client::CompanionClient;
use crate::phone::protocol::identity::{
    self, CompanionBootstrapOptions, CompanionInstallDecision, ExpectedCompanion,
    InstalledCompanion,
};

impl PhoneManager {
    // ===================================================================
    // Phone-side agent overlay lifecycle (companion-reachable sessions only)
    // ===================================================================

    /// Toggle the companion's persistent "agent in control" breathing edge glow
    /// for a session, best-effort.
    ///
    /// The glow is drawn on the device by the companion's accessibility-service
    /// overlay; it is a purely visual signal that a phone session is held, so a
    /// failure must never abort the connect/disconnect that triggered it. A
    /// transport failure drops the companion runtime and marks the profile stale,
    /// mirroring `companion_gesture`: companion-only actions fail closed while
    /// fallback-capable families may degrade. A per-method error (e.g. the
    /// accessibility service unavailable, reported via `glow_supported=false`) is
    /// swallowed: the session is still usable without the glow. A session with no
    /// reachable companion is a no-op.
    pub(super) async fn set_companion_overlay_active(&mut self, session_id: &str, active: bool) {
        // When the on-device visible overlay is disabled in config, the host never
        // issues the companion's visible-overlay calls, so the edge glow stays off.
        // The cursor capability report (`cursor_capabilities`) carries the resolved
        // `visible_overlay=false` state honestly. Default is enabled, so this is a
        // no-op unless the operator opted out.
        if !self.selection.visible_overlay {
            return;
        }
        let Some(entry) = self.sessions.get_mut(session_id) else {
            return;
        };
        if entry.overlay_active == active {
            return;
        }
        let Some(runtime) = entry.companion.as_mut() else {
            return;
        };
        match runtime.client.overlay_active(active).await {
            Ok(_) => {
                entry.overlay_active = active;
            }
            Err(error) if error.is_fallback() => {
                entry.overlay_active = false;
                entry.companion = None;
                self.invalidate_companion(session_id);
            }
            Err(_) => {}
        }
    }

    /// Clear the companion's persistent active overlay for sessions that have
    /// seen no phone activity for the configured idle window. The session remains
    /// valid; a later session-bound request relights the overlay.
    pub(crate) async fn expire_idle_companion_overlays(&mut self, now_ms: u64) -> Vec<String> {
        let expired = self
            .sessions
            .iter()
            .filter(|(_, entry)| {
                entry.overlay_active
                    && now_ms.saturating_sub(entry.last_overlay_activity_ms)
                        >= super::COMPANION_OVERLAY_IDLE_TIMEOUT_MS
            })
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>();

        for session_id in &expired {
            self.set_companion_overlay_active(session_id, false).await;
        }

        expired
    }

    pub(super) async fn touch_companion_overlay_activity(
        &mut self,
        selector: &PhoneSessionSelector,
        now_ms: u64,
    ) {
        let Some(session_id) = self.resolve_session_id(selector) else {
            return;
        };
        let should_relight = if let Some(entry) = self.sessions.get_mut(&session_id) {
            entry.last_overlay_activity_ms = now_ms;
            entry.companion.is_some() && !entry.overlay_active
        } else {
            false
        };
        if should_relight {
            self.set_companion_overlay_active(&session_id, true).await;
        }
    }

    // ===================================================================
    // Companion bootstrap (called from connect/refresh)
    // ===================================================================

    /// Bootstrap the companion for a serial: decide install/update from installed
    /// vs. expected identity, optionally `adb install -r`, set up `adb forward`,
    /// provision an ephemeral token through the setup intent, and probe
    /// `capabilities`. On success the profile's companion fields are populated and
    /// a [`CompanionRuntime`] is returned; any failure degrades to ADB baseline
    /// (companion fields stay absent, `rpc_reachable=false`) without aborting the
    /// session.
    ///
    /// Returns the live runtime (when reachable) plus the structured diagnostics
    /// the bootstrap produced. The companion install/update goes through the
    /// shared ADB-lane [`install_replace`] primitive and the captured
    /// [`InstallOutcome`] is surfaced as a structured diagnostic (success class or
    /// `INSTALL_FAILED_*` failure class) rather than being discarded, and the RPC
    /// port forward goes through the shared [`forward_tcp`] primitive.
    /// Bootstrap the companion for a connecting session: read the installed
    /// identity, optionally install/update the APK (`allow_install`), set up the
    /// RPC forward, deliver a fresh session token through the setup intent, and
    /// probe capabilities. Only the install/update step is gated by
    /// `allow_install`; the forward + token + probe always run so an
    /// already-installed companion is connected whenever it is enabled.
    pub(super) async fn bootstrap_companion_with_options(
        &mut self,
        serial: &str,
        profile: &mut PhoneCapabilityProfile,
        now: u64,
        options: CompanionBootstrapOptions,
    ) -> (Option<CompanionRuntime>, Vec<DiagnosticEntry>) {
        // Direct-only cut (2026-08-25): when direct is enabled, the legacy RPC
        // transport (`adb forward` + `SetupActivity` token) is dead — Android has
        // no `SetupActivity` and `CompanionClient` is a stub that always
        // `rpc_reachable=false`. Fast-path to absent without the 2s retry.
        if self.selection.direct_enabled {
            let diagnostics = vec![
                DiagnosticEntry {
                    code: "PhoneCompanionRpcRemoved".to_string(),
                    message:
                        "companion RPC bootstrap skipped: direct_enabled=true, use CompanionDirect"
                            .to_string(),
                    details: None,
                },
                crate::phone::protocol::not_implemented_diagnostic(),
            ];
            profile.companion =
                crate::phone::protocol::absent_companion(&self.selection.companion_package);
            return (None, diagnostics);
        }
        let package = self.selection.companion_package.clone();
        let allow_downgrade = options
            .allow_downgrade
            .unwrap_or(self.selection.companion_allow_downgrade);
        // Expected identity comes from the build-metadata sidecar bundled next to
        // the packaged APK (version, signing-cert SHA-256, APK SHA-256); env and
        // machine-config values override it. Without this, the expected cert is
        // always absent and the signature check has nothing to compare against.
        // The expected APK SHA-256 is report-only (the installed APK's hash is not
        // obtainable from `dumpsys`); the cert is the real check.
        let metadata = identity::load_companion_metadata(&self.selection.companion_apk_path);
        let expected = ExpectedCompanion {
            package_name: package.clone(),
            version_name: metadata.version_name.clone(),
            version_code: metadata.version_code,
            cert_sha256: self
                .selection
                .companion_expected_cert_sha256
                .clone()
                .or_else(|| metadata.cert_sha256.clone()),
            apk_sha256: self
                .selection
                .companion_apk_sha256
                .clone()
                .or_else(|| metadata.apk_sha256.clone()),
            apk_path: self.selection.companion_apk_path.clone(),
            allow_downgrade,
        };

        let mut diagnostics = Vec::new();

        // Read installed identity (best-effort). A missing package yields the
        // default "absent" InstalledCompanion via None.
        let installed = self.read_installed_companion(serial, &package).await;
        let mut decision =
            identity::decide_install(installed.as_ref(), &expected, options.allow_downgrade);
        if options.force_reinstall && matches!(decision, CompanionInstallDecision::UpToDate) {
            decision = CompanionInstallDecision::Update {
                reason: "explicit force_reinstall requested".to_string(),
            };
        }

        let install_allowed = options.allow_install;
        let mut auto_install_attempted = false;
        if decision.requires_install() && install_allowed {
            auto_install_attempted = true;
            // Reuse the shared ADB-lane install primitive and CAPTURE the outcome.
            let adb_path = self.configured_adb_path();
            let install_outcome = install_replace(
                self.runner.as_ref(),
                adb_path,
                serial,
                &expected.apk_path,
                expected.allow_downgrade,
            )
            .await;
            match install_outcome {
                Ok(outcome)
                    if options.force_reinstall
                        && outcome.failure_class.as_deref()
                            == Some("INSTALL_FAILED_UPDATE_INCOMPATIBLE") =>
                {
                    diagnostics.push(install_outcome_diagnostic(&decision, &outcome));
                    match uninstall_package(self.runner.as_ref(), adb_path, serial, &package).await
                    {
                        Ok(uninstall) if uninstall.success => {
                            diagnostics.push(DiagnosticEntry {
                                code: "PhoneCompanionReplacedIncompatiblePackage".to_string(),
                                message: "removed existing companion package after Android \
                                          rejected an update with incompatible signatures"
                                    .to_string(),
                                details: None,
                            });
                            match install_replace(
                                self.runner.as_ref(),
                                adb_path,
                                serial,
                                &expected.apk_path,
                                expected.allow_downgrade,
                            )
                            .await
                            {
                                Ok(retry) => {
                                    diagnostics.push(install_outcome_diagnostic(&decision, &retry));
                                }
                                Err(error) => {
                                    diagnostics.push(crate::phone::adb::command_error_diagnostic(
                                        "adb install -r",
                                        &error,
                                    ));
                                }
                            }
                        }
                        Ok(uninstall) => diagnostics.push(DiagnosticEntry {
                            code: "PhoneCompanionUninstallFailed".to_string(),
                            message: "could not remove incompatible existing companion package"
                                .to_string(),
                            details: (!uninstall.message.is_empty()).then_some(uninstall.message),
                        }),
                        Err(error) => diagnostics.push(
                            crate::phone::adb::command_error_diagnostic("adb uninstall", &error),
                        ),
                    }
                }
                Ok(outcome) => diagnostics.push(install_outcome_diagnostic(&decision, &outcome)),
                Err(error) => diagnostics.push(crate::phone::adb::command_error_diagnostic(
                    "adb install -r",
                    &error,
                )),
            }
        }

        if matches!(
            decision,
            CompanionInstallDecision::RefuseSignatureMismatch { .. }
        ) {
            // A same-name package whose readable signing cert differs from the
            // packaged companion is an impostor; never trust or replace it. (An
            // unreadable cert is not a mismatch and does not reach here.)
            diagnostics.push(DiagnosticEntry {
                code: decision.code().to_string(),
                message: "refusing to trust a same-name companion package whose signing \
                          certificate does not match the packaged companion; explicit operator \
                          reinstall required"
                    .to_string(),
                details: None,
            });
            profile.companion = protocol::capabilities_unreachable(
                &package,
                installed.as_ref().unwrap_or(&InstalledCompanion::default()),
                expected.cert_sha256.as_deref(),
                expected.apk_sha256.as_deref(),
                false,
            );
            return (None, diagnostics);
        }

        // Enable the companion's required secure-settings services (accessibility
        // + notification listener) as part of an install-bearing bootstrap, so a
        // freshly deployed companion is immediately usable instead of requiring a
        // manual trip through Android settings. Gated on the same decision that
        // authorized the APK install/update, and run only after the signature gate
        // above, never for an untrusted package. The grant is verified against the
        // companion's health probe below; a read-merge-write never clobbers the
        // user's existing services.
        if install_allowed {
            self.ensure_companion_permissions(serial, &package, &mut diagnostics)
                .await;
        }

        // Set up the forward through the shared ADB-lane primitive and deliver the
        // token through the setup intent.
        let port = 47683_u16;
        match forward_tcp(
            self.runner.as_ref(),
            self.configured_adb_path(),
            serial,
            port,
            port,
        )
        .await
        {
            Ok(outcome) if outcome.success => {}
            Ok(outcome) => {
                diagnostics.push(DiagnosticEntry {
                    code: "PhoneCompanionForwardFailed".to_string(),
                    message: format!(
                        "adb forward for the companion RPC port failed: {}",
                        outcome.message
                    ),
                    details: None,
                });
                profile.companion = protocol::absent_companion(&package);
                return (None, diagnostics);
            }
            Err(error) => {
                diagnostics.push(crate::phone::adb::command_error_diagnostic(
                    "adb forward",
                    &error,
                ));
                profile.companion = protocol::absent_companion(&package);
                return (None, diagnostics);
            }
        }

        // Deliver the ephemeral session token to the companion `SetupActivity`
        // directly as an intent extra. (A pushed file does not work: Android 11+
        // per-app storage mount namespaces make a shell-written file under
        // `/sdcard/Android/data/<pkg>/` unreadable by the app, so the RPC server
        // never started.) A failed `am start` means the token was not delivered,
        // so surface a distinct diagnostic rather than a confusing later
        // `unauthorized`. The token is never logged or echoed into any diagnostic.
        let token = identity::generate_token(now, 900_000);
        let setup_argv = identity::setup_intent_argv(serial, &package, &token);
        let setup_ref: Vec<&str> = setup_argv.iter().map(String::as_str).collect();
        let setup_intent_ok = match self.runner.run(&self.adb_program(), &setup_ref).await {
            Ok(output) if output.success() => true,
            Ok(output) => {
                diagnostics.push(DiagnosticEntry {
                    code: "PhoneCompanionSetupIntentFailed".to_string(),
                    message: format!(
                        "companion setup intent (am start .SetupActivity) exited with status {}; \
                         the session token may not have been delivered",
                        output.status.map_or(-1, |c| c)
                    ),
                    details: None,
                });
                false
            }
            Err(error) => {
                diagnostics.push(DiagnosticEntry {
                    code: "PhoneCompanionSetupIntentFailed".to_string(),
                    message: format!(
                        "companion setup intent (am start .SetupActivity) could not run: {error}; \
                         the session token may not have been delivered"
                    ),
                    details: None,
                });
                false
            }
        };

        // Probe capabilities over the forwarded RPC endpoint. When the setup
        // intent ran, the companion starts its RPC server and installs the token
        // asynchronously, so the first probe can race the server's bind or the
        // token install. Retry a few times on a transport (connection refused) or
        // `unauthorized` failure in that case, so an installed companion comes up
        // on the first connect instead of only on a later reconnect. If the setup
        // intent failed there is no server coming up, so skip the wait.
        let mut client = CompanionClient::new(port, token.token.clone());
        let mut caps_result = client.capabilities().await;
        if setup_intent_ok {
            for _ in 0..5 {
                let racing = matches!(
                    &caps_result,
                    Err(error) if error.is_fallback() || error.code() == "unauthorized"
                );
                if !racing {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                caps_result = client.capabilities().await;
            }
        }
        let runtime = match caps_result {
            Ok(caps) => {
                profile.companion = protocol::capabilities_from_response(
                    &caps,
                    Some(&token),
                    // The installed cert the host parsed during
                    // `read_installed_companion`, so the reachable report carries
                    // `installed_cert_sha256` like the unreachable path.
                    installed.as_ref().and_then(|c| c.cert_sha256.as_deref()),
                    expected.cert_sha256.as_deref(),
                    expected.apk_sha256.as_deref(),
                    auto_install_attempted,
                    allow_downgrade,
                );
                Some(CompanionRuntime { client })
            }
            Err(caps_error) if !caps_error.is_fallback() => {
                // The companion is reachable and answered, but could not serve the
                // richer `capabilities` method (e.g. an older companion build that
                // only implements `health`). Fall back to the lightweight `health`
                // probe and build capabilities from its permission booleans, with
                // screenshot/gesture support unknown.
                diagnostics.push(DiagnosticEntry {
                    code: caps_error.code().to_string(),
                    message: format!(
                        "companion capabilities probe unavailable; retrying health: {caps_error}"
                    ),
                    details: None,
                });
                match client.health().await {
                    Ok(health) => {
                        profile.companion = protocol::capabilities_from_health(
                            &health,
                            installed.as_ref().and_then(|c| c.version_name.clone()),
                        );
                        Some(CompanionRuntime { client })
                    }
                    Err(health_error) => {
                        diagnostics.push(DiagnosticEntry {
                            code: health_error.code().to_string(),
                            message: format!("companion health probe failed: {health_error}"),
                            details: None,
                        });
                        profile.companion = protocol::capabilities_unreachable(
                            &package,
                            installed.as_ref().unwrap_or(&InstalledCompanion::default()),
                            expected.cert_sha256.as_deref(),
                            expected.apk_sha256.as_deref(),
                            true,
                        );
                        None
                    }
                }
            }
            Err(error) => {
                // Transport/auth/version failure: the RPC is unreachable. Degrade
                // to ADB baseline without claiming companion success.
                diagnostics.push(DiagnosticEntry {
                    code: error.code().to_string(),
                    message: format!("companion capability probe failed: {error}"),
                    details: None,
                });
                profile.companion = protocol::capabilities_unreachable(
                    &package,
                    installed.as_ref().unwrap_or(&InstalledCompanion::default()),
                    self.selection.companion_expected_cert_sha256.as_deref(),
                    expected.apk_sha256.as_deref(),
                    true,
                );
                None
            }
        };

        // Verify the services we enabled actually took. The companion's health
        // booleans are the ground truth, so this only runs when the companion is
        // reachable (a degraded/unreachable bootstrap already surfaced its own
        // diagnostic) and only after an install-bearing bootstrap that attempted
        // the grant. Some OEM builds gate a sideloaded app's accessibility behind
        // a manual confirmation the adb write cannot satisfy; a still-disabled
        // service yields an actionable diagnostic and (for accessibility) opens
        // the on-device settings screen so setup can be finished by hand.
        // Permission verification is not gated on install: the companion may have
        // been installed earlier and permissions revoked while the session was
        // disconnected. A reachable companion RPC is sufficient to probe and
        // report gaps, and to re-ensure permissions for the steady state.
        if runtime.is_some() {
            self.ensure_companion_permissions(serial, &package, &mut diagnostics)
                .await;
            self.flag_companion_permission_gaps(serial, &profile.companion, &mut diagnostics)
                .await;
        }

        (runtime, diagnostics)
    }

    /// Enable the companion's required secure-settings services on a connecting
    /// device as part of an install-bearing bootstrap. The companion needs its
    /// accessibility service (gestures, tree, screenshots, and the cursor
    /// overlay) and its notification listener present in the corresponding
    /// secure-settings lists; both are writable by the adb `shell` user. Each
    /// enable is a read-merge-write that preserves the user's existing services.
    /// Newly enabled services and hard write rejections are surfaced as
    /// structured diagnostics; the already-enabled steady state stays silent so
    /// repeat deploys are quiet. The post-probe [`flag_companion_permission_gaps`]
    /// proves the grants actually bound.
    async fn ensure_companion_permissions(
        &self,
        serial: &str,
        package: &str,
        diagnostics: &mut Vec<DiagnosticEntry>,
    ) {
        let accessibility = format!("{package}/{package}{ACCESSIBILITY_SERVICE_CLASS_SUFFIX}");
        let notification = format!("{package}/{package}{NOTIFICATION_LISTENER_CLASS_SUFFIX}");

        // Accessibility binds immediately from a secure-settings read-merge-write
        // plus the global `accessibility_enabled` flag; there is no stable `cmd`
        // equivalent across Android versions.
        match ensure_secure_list_service(
            self.runner.as_ref(),
            self.configured_adb_path(),
            serial,
            "enabled_accessibility_services",
            &accessibility,
            true,
        )
        .await
        {
            Ok(outcome) => {
                if let Some(diagnostic) =
                    permission_outcome_diagnostic("accessibility service", &outcome)
                {
                    diagnostics.push(diagnostic);
                }
            }
            Err(error) => diagnostics.push(crate::phone::adb::command_error_diagnostic(
                "adb shell settings get/put secure enabled_accessibility_services",
                &error,
            )),
        }

        // The notification listener is bound through `cmd notification
        // allow_listener`: a bare settings write can leave the entry present but
        // unbound until the next reconcile, which would make the health probe
        // spuriously report it off.
        match ensure_notification_listener(
            self.runner.as_ref(),
            self.configured_adb_path(),
            serial,
            &notification,
        )
        .await
        {
            Ok(outcome) => {
                if let Some(diagnostic) =
                    permission_outcome_diagnostic("notification listener", &outcome)
                {
                    diagnostics.push(diagnostic);
                }
            }
            Err(error) => diagnostics.push(crate::phone::adb::command_error_diagnostic(
                "adb shell cmd notification allow_listener",
                &error,
            )),
        }
    }

    /// After an install-bearing bootstrap enabled the companion's secure-settings
    /// services, flag any the reachable companion still reports disabled.
    ///
    /// `settings put` succeeds, but some OEM builds (notably Samsung One UI)
    /// ignore an adb-written accessibility grant until the operator clears a
    /// "Restricted settings" confirmation by hand. The companion health booleans
    /// are the ground truth, so a still-disabled service yields an actionable
    /// diagnostic; accessibility — which gates gestures, the tree, screenshots,
    /// and the cursor overlay — also best-effort opens the on-device Accessibility
    /// screen so the operator sees exactly where to finish setup.
    async fn flag_companion_permission_gaps(
        &self,
        serial: &str,
        companion: &PhoneCompanionCapabilities,
        diagnostics: &mut Vec<DiagnosticEntry>,
    ) {
        if !companion.accessibility_enabled {
            diagnostics.push(DiagnosticEntry {
                code: "PhoneCompanionAccessibilityManualSetup".to_string(),
                message: "the companion accessibility service was enabled over adb but the device \
                          still reports it off; some builds (e.g. Samsung One UI) gate a \
                          sideloaded app's accessibility behind a manual 'Restricted settings' \
                          confirmation. Enable 'Sky Phone Companion' under Settings > \
                          Accessibility to finish setup."
                    .to_string(),
                details: None,
            });
            // Best-effort: surface the exact on-device screen so the operator can
            // act. A failure here only forgoes the convenience, so it is swallowed.
            let _ = crate::phone::adb::open_settings(
                self.runner.as_ref(),
                self.configured_adb_path(),
                serial,
                PhoneSettingsScreen::Accessibility,
                None,
            )
            .await;
        }
        if !companion.notification_listener_enabled {
            diagnostics.push(DiagnosticEntry {
                code: "PhoneCompanionNotificationManualSetup".to_string(),
                message: "the companion notification listener was enabled over adb but the device \
                          still reports it off; grant 'Sky Phone Companion notifications' under \
                          Settings > Notification access to enable the notification tools."
                    .to_string(),
                details: None,
            });
            let _ = crate::phone::adb::open_settings(
                self.runner.as_ref(),
                self.configured_adb_path(),
                serial,
                PhoneSettingsScreen::NotificationAccess,
                None,
            )
            .await;
        }
    }

    /// Best-effort read of the installed companion's version/cert. Returns `None`
    /// when the package is not installed.
    ///
    /// Presence is confirmed via `pm path`; identity (versionCode + signing
    /// certificate SHA-256) is then extracted from `dumpsys package <pkg>` so the
    /// install decision's cert/downgrade guards have real metadata to compare
    /// against. Extraction is conservative: a missing or unparseable field stays
    /// `None`, so the happy path (no expected cert configured) is never regressed
    /// and a configured `companion_expected_cert_sha256` only enforces a refusal
    /// when an installed cert is actually read and differs.
    async fn read_installed_companion(
        &self,
        serial: &str,
        package: &str,
    ) -> Option<InstalledCompanion> {
        let path_argv = ["-s", serial, "shell", "pm", "path", package];
        let path_output = self
            .runner
            .run(&self.adb_program(), &path_argv)
            .await
            .ok()?;
        if !path_output.success() || !path_output.stdout_string().contains("package:") {
            return None;
        }

        // Best-effort identity extraction. A failure to run or parse leaves the
        // fields `None` (the prior behavior) rather than blocking the install.
        let dump_argv = ["-s", serial, "shell", "dumpsys", "package", package];
        let installed = match self.runner.run(&self.adb_program(), &dump_argv).await {
            Ok(output) if output.success() => parse_installed_companion(&output.stdout_string()),
            _ => InstalledCompanion::default(),
        };
        Some(installed)
    }

    /// The resolved adb program string (config/env/PATH), used for the companion
    /// bootstrap calls that build their own argv.
    fn adb_program(&self) -> String {
        crate::phone::command::resolve_adb_path(self.configured_adb_path())
    }

    // ===================================================================
    // Companion status / install
    // ===================================================================

    /// `phone_companion_status`: report the cached companion identity/capability
    /// for a session, or the configured-expected absent shape when no session
    /// resolves.
    pub(super) async fn companion_status(
        &self,
        selector: &PhoneSessionSelector,
    ) -> PhoneCompanionStatusResponse {
        if let Some(session_id) = self.resolve_session_id(selector)
            && let Some(cached) = self.profiles.get(&session_id)
        {
            if let Some((device_id, epoch)) = self.direct_identity(&session_id)
                && let Some(provider) = &self.direct_provider
                && let Err(error) = provider
                    .dispatch(
                        &device_id,
                        epoch,
                        "companion.status",
                        serde_json::json!({}),
                        true,
                        std::time::Duration::from_secs(5),
                    )
                    .await
            {
                return PhoneCompanionStatusResponse {
                    session_id,
                    serial: String::new(),
                    companion: cached.profile.companion.clone(),
                    diagnostics: vec![DiagnosticEntry {
                        code: "PhoneCompanionDirectDispatchFailed".into(),
                        message: format!("CompanionDirect status failed: {error:?}"),
                        details: None,
                    }],
                };
            }
            // Surface the most recent companion bootstrap diagnostics (install
            // outcome class, forward/probe failures) rather than discarding them.
            let diagnostics = self.companion_bootstrap_diagnostics(&session_id);
            return PhoneCompanionStatusResponse {
                session_id,
                serial: cached.profile.serial.clone(),
                companion: cached.profile.companion.clone(),
                diagnostics,
            };
        }
        let (session_id, serial) = selector_ids(selector);
        PhoneCompanionStatusResponse {
            session_id,
            serial,
            companion: protocol::absent_companion(&self.selection.companion_package),
            diagnostics: vec![protocol::not_implemented_diagnostic()],
        }
    }

    /// `phone_install_companion`: explicit reinstall/update + re-bootstrap.
    pub(super) async fn install_companion(
        &mut self,
        request: PhoneInstallCompanionRequest,
    ) -> PhoneActionResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            return self.app_action_to_action_response(&request.session, "phone_install_companion");
        };
        let Some(serial) = self
            .sessions
            .get(&session_id)
            .map(|entry| entry.session.serial.clone())
        else {
            return self.app_action_to_action_response(&request.session, "phone_install_companion");
        };
        let now = super::now_ms();
        let mut profile = crate::phone::device::detect_profile_with_path(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &session_id,
            &serial,
            &self.selection.companion_package,
            now,
            PhoneCapabilityRefreshState::Refreshed,
        )
        .await;
        profile.scrcpy = self.detect_scrcpy_capabilities().await;
        let (companion_runtime, companion_diagnostics) = if self.selection.companion_enabled {
            self.bootstrap_companion_with_options(
                &serial,
                &mut profile,
                now,
                CompanionBootstrapOptions {
                    allow_install: true,
                    force_reinstall: request.force_reinstall,
                    allow_downgrade: Some(
                        request.allow_downgrade || self.selection.companion_allow_downgrade,
                    ),
                },
            )
            .await
        } else {
            (None, Vec::new())
        };
        let capabilities = self.backend_capabilities(&profile);
        super::routing::populate_actions(&mut profile, &capabilities);
        let reachable = companion_runtime.is_some();
        if let Some(entry) = self.sessions.get_mut(&session_id) {
            entry.session.capability_profile = profile.clone();
            entry.session.capabilities = capabilities;
            entry.session.companion = Some(profile.companion.clone());
            entry.companion = companion_runtime;
            entry.companion_diagnostics = companion_diagnostics;
        }
        self.profiles.insert(
            session_id.clone(),
            super::CachedProfile {
                profile: profile.clone(),
                detected_at_ms: now,
            },
        );
        if reachable {
            self.set_companion_overlay_active(&session_id, true).await;
        }
        let (serial, profile_id, reachable) = self
            .profiles
            .get(&session_id)
            .map(|cached| {
                (
                    cached.profile.serial.clone(),
                    cached.profile.profile_id.clone(),
                    cached.profile.companion.rpc_reachable,
                )
            })
            .unwrap_or_default();

        // Surface the structured install/forward/probe outcome captured during the
        // re-bootstrap (the install class is the load-bearing diagnostic here),
        // then the unreachable summary when the RPC endpoint did not come up.
        let mut diagnostics = self.companion_bootstrap_diagnostics(&session_id);
        if !reachable {
            diagnostics.push(DiagnosticEntry {
                code: "PhoneCompanionUnreachable".to_string(),
                message: "companion reinstall ran but the RPC endpoint is not reachable"
                    .to_string(),
                details: None,
            });
        }
        PhoneActionResponse {
            session_id,
            serial,
            action: "phone_install_companion".to_string(),
            backend: if reachable {
                PhoneBackendKind::Companion
            } else {
                PhoneBackendKind::Adb
            },
            capability_profile_id: profile_id,
            profile_refresh_state: PhoneCapabilityRefreshState::Refreshed,
            phone_snapshot_id: None,
            cursor: None,
            diagnostics,
        }
    }

    /// The structured diagnostics the most recent companion bootstrap produced for
    /// a session (install outcome class, forward/probe failures), cloned from the
    /// session entry. Empty when no session or no bootstrap diagnostics exist.
    fn companion_bootstrap_diagnostics(&self, session_id: &str) -> Vec<DiagnosticEntry> {
        self.sessions
            .get(session_id)
            .map(|entry| entry.companion_diagnostics.clone())
            .unwrap_or_default()
    }

    fn app_action_to_action_response(
        &self,
        selector: &PhoneSessionSelector,
        action: &str,
    ) -> PhoneActionResponse {
        let (session_id, serial) = selector_ids(selector);
        PhoneActionResponse {
            session_id,
            serial,
            action: action.to_string(),
            backend: PhoneBackendKind::None,
            capability_profile_id: String::new(),
            profile_refresh_state: PhoneCapabilityRefreshState::Stale,
            phone_snapshot_id: None,
            cursor: None,
            diagnostics: vec![no_session_diagnostic(selector)],
        }
    }
}

/// Build a structured diagnostic for a captured companion [`InstallOutcome`].
///
/// On success the code is the install decision code (`CompanionInstall`/
/// `CompanionUpdate`); on failure the load-bearing `INSTALL_FAILED_*` class from
/// adb is used as the code so clients route on the field, with the bounded adb
/// message in `details`.
fn install_outcome_diagnostic(
    decision: &CompanionInstallDecision,
    outcome: &InstallOutcome,
) -> DiagnosticEntry {
    if outcome.success {
        DiagnosticEntry {
            code: decision.code().to_string(),
            message: "companion APK install/update succeeded".to_string(),
            details: None,
        }
    } else {
        DiagnosticEntry {
            code: outcome
                .failure_class
                .clone()
                .unwrap_or_else(|| "PhoneCompanionInstallFailed".to_string()),
            message: "companion APK install/update failed".to_string(),
            details: (!outcome.message.is_empty()).then(|| outcome.message.clone()),
        }
    }
}

/// Map a companion secure-settings enable [`SecureServiceOutcome`] to a structured
/// diagnostic. A freshly enabled service and a hard write rejection are surfaced;
/// the already-enabled steady state returns `None` so repeat deploys stay quiet.
fn permission_outcome_diagnostic(
    label: &str,
    outcome: &SecureServiceOutcome,
) -> Option<DiagnosticEntry> {
    match outcome.state {
        SecureServiceState::AlreadyEnabled => None,
        SecureServiceState::Enabled => Some(DiagnosticEntry {
            code: "PhoneCompanionPermissionEnabled".to_string(),
            message: format!("enabled the companion {label} via secure settings"),
            details: None,
        }),
        SecureServiceState::WriteRejected => Some(DiagnosticEntry {
            code: "PhoneCompanionPermissionWriteRejected".to_string(),
            message: format!(
                "could not enable the companion {label} over adb; a manual grant in Android \
                 settings is required"
            ),
            details: (!outcome.message.is_empty()).then(|| outcome.message.clone()),
        }),
    }
}

/// Best-effort parse of `dumpsys package <pkg>` output into an
/// [`InstalledCompanion`]. Conservative by design: any field that is absent or
/// unparseable stays `None`, matching the prior "presence only" behavior and
/// keeping the no-expected-cert default path unchanged. Only when both an
/// installed cert and a configured expected cert are present (and differ) does
/// the install decision refuse on signature mismatch.
fn parse_installed_companion(dump: &str) -> InstalledCompanion {
    InstalledCompanion {
        version_name: parse_version_name(dump),
        version_code: parse_version_code(dump),
        cert_sha256: parse_signing_cert_sha256(dump),
    }
}

/// Extract `versionCode=<digits>` (the first occurrence). `dumpsys package`
/// renders it as e.g. `versionCode=4201 minSdk=26 targetSdk=34`.
fn parse_version_code(dump: &str) -> Option<u64> {
    for line in dump.lines() {
        if let Some(rest) = line.split("versionCode=").nth(1) {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(code) = digits.parse::<u64>() {
                return Some(code);
            }
        }
    }
    None
}

/// Extract `versionName=<token>` (the first occurrence), trimmed to the first
/// whitespace. `dumpsys package` renders it as e.g. `versionName=1.4.2`.
fn parse_version_name(dump: &str) -> Option<String> {
    for line in dump.lines() {
        if let Some(rest) = line.split("versionName=").nth(1) {
            let value = rest.split_whitespace().next().unwrap_or_default();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Extract a signing-certificate SHA-256 digest. `dumpsys package` cert lines
/// vary by Android version; this looks for a line that references a certificate/
/// signature/SHA-256 label and pulls the first 64-hex-character token out of it
/// (separators like `:` are stripped). Returns a lowercase hex string, or `None`
/// when no such token is present.
fn parse_signing_cert_sha256(dump: &str) -> Option<String> {
    for line in dump.lines() {
        let lower = line.to_ascii_lowercase();
        let cert_context = lower.contains("sha-256")
            || lower.contains("sha256")
            || lower.contains("cert")
            || lower.contains("signature")
            || lower.contains("signing");
        if !cert_context {
            continue;
        }
        if let Some(digest) = extract_hex64(line) {
            return Some(digest);
        }
    }
    None
}

/// Find a 64-hex-character digest on a line, ignoring `:` separators between
/// bytes. Returns the lowercase hex (no separators) for the first maximal hex
/// run that is exactly 64 nibbles long; a shorter or longer run is rejected so an
/// 80-hex blob does not yield a bogus 64-char prefix.
fn extract_hex64(line: &str) -> Option<String> {
    let mut run = String::new();
    for ch in line.chars() {
        if ch.is_ascii_hexdigit() {
            run.push(ch.to_ascii_lowercase());
        } else if ch == ':' {
            // Byte separator inside a digest; keep accumulating the run.
            continue;
        } else {
            if run.len() == 64 {
                return Some(run);
            }
            run.clear();
        }
    }
    // A digest at end-of-line has no trailing separator to flush it.
    (run.len() == 64).then_some(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP_WITH_CERT_AND_VERSION: &str = "\
Packages:
  Package [com.sky.companion] (a1b2c3):
    userId=10234
    versionCode=4201 minSdk=26 targetSdk=34
    versionName=1.4.2
    signatures=[Signature]
    Signing KeySet: 12
    SHA-256 cert digest: aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99
    dataDir=/data/user/0/com.sky.companion
";

    #[test]
    fn parses_version_code_and_name() {
        let installed = parse_installed_companion(DUMP_WITH_CERT_AND_VERSION);
        assert_eq!(installed.version_code, Some(4201));
        assert_eq!(installed.version_name.as_deref(), Some("1.4.2"));
    }

    #[test]
    fn parses_signing_cert_sha256_stripping_separators() {
        let installed = parse_installed_companion(DUMP_WITH_CERT_AND_VERSION);
        assert_eq!(
            installed.cert_sha256.as_deref(),
            Some("aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899")
        );
    }

    #[test]
    fn missing_fields_stay_none_conservatively() {
        // Presence-only dump with no versionCode and no cert digest: every
        // optional field is None, so the happy path is never regressed.
        let dump = "Packages:\n  Package [com.sky.companion] (deadbeef):\n    dataDir=/data\n";
        let installed = parse_installed_companion(dump);
        assert_eq!(installed, InstalledCompanion::default());
    }

    #[test]
    fn ignores_non_digest_hex_in_unrelated_lines() {
        // A 64-hex token only counts when its line references a cert/signature.
        let dump = "    userId=10234\n    randomBlob=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
        assert_eq!(parse_signing_cert_sha256(dump), None);
    }
}
