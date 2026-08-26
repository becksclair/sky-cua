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

use sky_cua_platform::model::{DiagnosticEntry, PhoneCapabilityProfile, PhoneSessionSelector};

use super::companion_probe::install_outcome_diagnostic;
use super::{CompanionRuntime, PhoneManager};
use crate::phone::adb::{forward_tcp, install_replace, uninstall_package};
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
        // CompanionDirect path (first-class): dispatch `overlay_active` over the
        // authenticated `phone-control.v2` ws, not the legacy `adb forward` RPC.
        // Reuse the canonical helper so the identity derivation never drifts.
        let direct = self.direct_identity(session_id);
        if let Some((device_id, epoch)) = direct {
            let already = self
                .sessions
                .get(session_id)
                .is_some_and(|e| e.overlay_active == active);
            if already {
                return;
            }
            // Turning on requires a proven native overlay; turning off is
            // always allowed so an idle expiry can clear a stale glow.
            if active
                && !self
                    .sessions
                    .get(session_id)
                    .is_some_and(|e| e.session.capabilities.phone_native_overlay)
            {
                return;
            }
            let Some(provider) = self.direct_provider.clone() else {
                return;
            };
            match provider
                .dispatch(
                    &device_id,
                    epoch,
                    "overlay_active",
                    serde_json::json!({"active": active}),
                    false,
                    std::time::Duration::from_secs(5),
                )
                .await
            {
                Ok(_) => {
                    if let Some(entry) = self.sessions.get_mut(session_id) {
                        entry.overlay_active = active;
                    }
                }
                Err(error) => {
                    if super::helpers::is_direct_disconnected(&error) {
                        if let Some(entry) = self.sessions.get_mut(session_id) {
                            entry.overlay_active = false;
                        }
                        self.invalidate_companion(session_id);
                    }
                }
            }
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
            // The session advertises a native overlay only when the capability
            // profile proves `native_overlay` reachable (see `backend_capabilities`).
            // Both legacy (`entry.companion`) and direct (`phone_native_overlay`)
            // must be checked so a direct session without overlay support does not
            // spuriously dispatch `overlay_active`.
            let capable = entry.session.capabilities.phone_native_overlay;
            capable && !entry.overlay_active
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
}
