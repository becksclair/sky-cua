#![allow(clippy::empty_line_after_doc_comments)]
//! Session lifecycle: connect (ADB + direct), disconnect, and refresh.

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneBackendKind, PhoneCapabilityProfile, PhoneCapabilityRefreshState,
    PhoneConnectRequest, PhoneConnectionIdentity, PhoneConnectionKind, PhoneDisconnectRequest,
    PhoneDisconnectResponse, PhoneResponse, PhoneScrcpyCapabilities, PhoneSession,
    PhoneSessionSelector, PhoneStatusReport, PhoneTargetDeviceKind,
};

use super::helpers::{
    companion_from_direct_health, is_uuid_format, new_session_id, no_session_diagnostic, now_ms,
    request_wants_scrcpy, selector_ids,
};
use super::{CachedProfile, PhoneManager, SessionEntry};
use crate::phone::adb;
use crate::phone::cursor::PhoneCursorTracker;
use crate::phone::device;
use crate::phone::manager::routing;
use crate::phone::protocol::identity::CompanionBootstrapOptions;
use crate::phone::snapshot::{DEFAULT_SNAPSHOT_CAPACITY, PhoneSnapshotRegistry};

impl PhoneManager {
    pub(crate) async fn connect(&mut self, request: PhoneConnectRequest) -> PhoneResponse {
        let alias_count = usize::from(request.serial.is_some())
            + usize::from(request.device_id.is_some())
            + usize::from(request.alias.is_some());
        if alias_count > 1 {
            return PhoneResponse::Status(PhoneStatusReport {
                enabled: true,
                adb_available: false,
                adb_path: None,
                adb_version: None,
                adb_server_running: None,
                scrcpy_available: false,
                scrcpy_path: None,
                scrcpy_version: None,
                companion_enabled: self.selection.companion_enabled,
                mdns_available: false,
                default_serial: None,
                default_backend: PhoneBackendKind::None,
                sessions: self
                    .sessions
                    .values()
                    .map(|entry| entry.session.clone())
                    .collect(),
                devices: Vec::new(),
                diagnostics: vec![DiagnosticEntry {
                    code: "PhoneActionRejected".into(),
                    message: "phone_connect serial, device_id and alias are mutually exclusive"
                        .into(),
                    details: None,
                }],
            });
        }
        if let Some(alias) = request.alias.as_deref() {
            let Some(target) = self.selection.aliases.get(alias).cloned() else {
                return PhoneResponse::Status(PhoneStatusReport {
                    enabled: true,
                    adb_available: false,
                    adb_path: None,
                    adb_version: None,
                    adb_server_running: None,
                    scrcpy_available: false,
                    scrcpy_path: None,
                    scrcpy_version: None,
                    companion_enabled: self.selection.companion_enabled,
                    mdns_available: false,
                    default_serial: None,
                    default_backend: PhoneBackendKind::None,
                    sessions: self
                        .sessions
                        .values()
                        .map(|entry| entry.session.clone())
                        .collect(),
                    devices: Vec::new(),
                    diagnostics: vec![DiagnosticEntry {
                        code: "PhoneAliasNotFound".into(),
                        message: format!(
                            "phone alias {alias:?} is not configured in [phone.aliases]"
                        ),
                        details: None,
                    }],
                });
            };
            // Prefer direct device_id when the mapped value matches a known
            // direct device; otherwise treat it as an ADB serial. For offline
            // direct devices the provider cache is empty, so also treat a
            // UUID-shaped value as a direct id and probe the direct path
            // first; if that probe fails we fall through to the ADB path only
            // when the value also looks like a serial (contains ':').
            let is_direct = self
                .direct_provider
                .as_ref()
                .is_some_and(|p| p.device(&target).is_some())
                || self.sessions.values().any(|e| {
                    matches!(e.session.connection, Some(PhoneConnectionIdentity::CompanionDirect { device_id: ref id, .. }) if id == &target)
                });
            let is_uuid = is_uuid_format(&target);
            if is_direct || is_uuid {
                let res = self.connect_direct(&target).await;
                // UUID aliases that are offline should surface the direct
                // “device offline” diagnostic, not an ADB mis-route. Only
                // fall back to ADB when the alias was not a known direct
                // device and the value also looks like a host:port serial.
                if is_direct {
                    return res;
                }
                if let PhoneResponse::Connected(_) = res {
                    return res;
                }
                if !target.contains(':') {
                    return res;
                }
                // Fall through to ADB serial handling for host:port-like
                // values that happened to be UUID-shaped but are really
                // serials (defensive; not expected in practice).
            }
            // Treat as ADB serial; reuse the serial connect path without
            // going through the auto-connect/default logic.
            if let Some(session_id) = self.session_id_for_serial(&target) {
                let allow_install = self.operator_auto_install() || request.install_companion;
                self.rebuild_session(
                    &session_id,
                    PhoneCapabilityRefreshState::Refreshed,
                    allow_install,
                    request.backend,
                )
                .await;
                if request_wants_scrcpy(&request)
                    && self
                        .sessions
                        .get(&session_id)
                        .is_some_and(|entry| entry.scrcpy.is_none())
                {
                    self.relaunch_scrcpy_on_reconnect(&session_id).await;
                }
                if let Some(entry) = self.sessions.get(&session_id) {
                    return PhoneResponse::Connected(entry.session.clone());
                }
            }
            // Fall through to the serial connect path by synthesizing a
            // serial request.
            let mut serial_req = request;
            serial_req.alias = None;
            serial_req.serial = Some(target);
            return Box::pin(self.connect(serial_req)).await;
        }
        if let Some(device_id) = request.device_id.as_deref() {
            return self.connect_direct(device_id).await;
        }
        // Wireless auto-connect: when enabled, `adb connect` the configured
        // wireless `default_serial` (a `host:port` form) before resolution, so a
        // wireless link the operator pre-configured is brought up and the device
        // becomes present for `resolve_target_serial`/`serial_is_authorized_device`.
        // Best-effort and idempotent (`adb connect` is a no-op when already
        // connected); a failure here surfaces later as the normal
        // device-unavailable diagnostic. Off by default, so this is skipped
        // entirely unless the operator opted in.
        self.wireless_auto_connect_default().await;

        let Some(serial) = self.resolve_target_serial(request.serial.as_deref()).await else {
            // No serial could be resolved (adb missing, no devices, or an
            // ambiguous multi-device set with no default). Stay honest.
            return PhoneResponse::Status(self.status(false).await);
        };

        // Idempotent reconnect: an existing session for this serial is refreshed
        // rather than duplicated. The silent reconnect install convenience is gated
        // by operator mode plus auto-install, but an explicit `install_companion`
        // request on the reconnect still drives an install.
        if let Some(session_id) = self.session_id_for_serial(&serial) {
            let allow_install = self.operator_auto_install() || request.install_companion;
            self.rebuild_session(
                &session_id,
                PhoneCapabilityRefreshState::Refreshed,
                allow_install,
                request.backend,
            )
            .await;
            // A reconnect that asks for scrcpy must re-establish a mirror that was
            // torn down (crash or operator close) since the original connect:
            // `rebuild_session` re-detects the profile but never touches the scrcpy
            // runtime, so without this the relaunch is silently skipped and scrcpy
            // reports inactive with no diagnostic. Only relaunch when the request
            // wants scrcpy and no live mirror remains; an already-live mirror is
            // left untouched. A relaunch failure surfaces a structured diagnostic.
            if request_wants_scrcpy(&request)
                && self
                    .sessions
                    .get(&session_id)
                    .is_some_and(|entry| entry.scrcpy.is_none())
            {
                self.relaunch_scrcpy_on_reconnect(&session_id).await;
            }
            if let Some(entry) = self.sessions.get(&session_id) {
                return PhoneResponse::Connected(entry.session.clone());
            }
        }

        let connection_kind = adb::classify_connection_kind(&serial);
        // Wireless targets must be connected before any shell command works. A
        // failed `adb connect` (refused/timeout/wrong port) carries the actionable
        // reason in its classified `TransportOutcome`; capture it so the failure
        // survives into the response instead of resurfacing only as a generic
        // device-unavailable diagnostic. Mirrors the `pair_wireless` surfacing.
        let mut connect_failure: Option<DiagnosticEntry> = None;
        if matches!(
            connection_kind,
            PhoneConnectionKind::WirelessDebugging | PhoneConnectionKind::LegacyTcpip
        ) {
            match adb::connect(self.runner.as_ref(), self.configured_adb_path(), &serial).await {
                Ok(outcome) if outcome.success => {}
                Ok(outcome) => {
                    connect_failure = Some(DiagnosticEntry {
                        code: "PhoneConnectFailed".to_string(),
                        message: format!("adb connect did not report success: {}", outcome.message),
                        details: None,
                    });
                }
                Err(error) => {
                    connect_failure = Some(adb::command_error_diagnostic("adb connect", &error));
                }
            }
        }

        // A requested serial must resolve to a present, authorized device before
        // a session is minted. Without this, `phone_connect` optimistically
        // reports success for a bogus or unreachable serial and only fails on the
        // first action. Wireless targets were just `adb connect`ed above, so an
        // unreachable host is absent from the device list here too.
        if !self.serial_is_authorized_device(&serial).await {
            let mut report = self.status(false).await;
            // Surface the `adb connect` failure reason first (the load-bearing
            // diagnostic), so the actionable message is not lost behind the
            // generic unavailable one.
            if let Some(diagnostic) = connect_failure {
                report.diagnostics.push(diagnostic);
            }
            report.diagnostics.push(DiagnosticEntry {
                code: "PhoneDeviceUnavailable".to_string(),
                message: format!(
                    "requested device {serial} is not a connected, authorized adb device"
                ),
                details: None,
            });
            return PhoneResponse::Status(report);
        }

        let session_id = new_session_id(&serial);
        let now = now_ms();
        let mut profile = device::detect_profile_with_path(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &session_id,
            &serial,
            &self.selection.companion_package,
            now,
            PhoneCapabilityRefreshState::Detected,
        )
        .await;
        // Resolve scrcpy and probe its version into the profile (idle until a
        // mirror launches).
        profile.scrcpy = self.detect_scrcpy_capabilities().await;

        // Companion bootstrap (forward + token + probe, plus optional
        // install/update) runs whenever the companion is enabled. A requested
        // input backend such as `adb` controls dispatch preference only; it must
        // not suppress companion observability, accessibility, notifications, or
        // the on-device agent overlay. `allow_install` gates only APK
        // install/update, so an already-installed companion is connected even
        // when auto-install is off or ADB input was requested.
        let (companion_runtime, mut connect_diagnostics) = if self.selection.companion_enabled {
            // The silent auto-install convenience is gated by operator mode; an
            // explicit `install_companion` request always allows the install.
            let allow_install = self.operator_auto_install() || request.install_companion;
            self.bootstrap_companion_with_options(
                &serial,
                &mut profile,
                now,
                CompanionBootstrapOptions {
                    allow_install,
                    force_reinstall: false,
                    allow_downgrade: None,
                },
            )
            .await
        } else {
            (None, Vec::new())
        };

        // Optional scrcpy mirror: launch only when the caller asked for it
        // (explicit `start_scrcpy` or `backend == Scrcpy`). A launch failure never
        // aborts connect; it degrades to ADB/companion with a structured diagnostic
        // and `profile.scrcpy` stays idle. The host overlay plane is left off
        // (`host_window_mapped=false`) until the daemon maps the host window.
        let (scrcpy_runtime, scrcpy_window_title) = if request_wants_scrcpy(&request) {
            let (runtime, title, diagnostic) =
                self.establish_scrcpy_mirror(&serial, &mut profile).await;
            if let Some(diagnostic) = diagnostic {
                connect_diagnostics.push(diagnostic);
            }
            (runtime, title)
        } else {
            (None, None)
        };

        // Capabilities/affordances are computed after the scrcpy launch so they
        // reflect a live mirror (`profile.scrcpy.active`) when one started.
        let capabilities = self.backend_capabilities(&profile);
        routing::populate_actions(&mut profile, &capabilities);

        let managed_process = scrcpy_runtime.is_some();
        let session_backend = self.connect_session_backend(
            request.backend,
            companion_runtime.is_some(),
            managed_process,
        );
        let session = PhoneSession {
            session_id: session_id.clone(),
            serial: serial.clone(),
            connection: Some(PhoneConnectionIdentity::Adb {
                serial: serial.clone(),
                name: profile.model.clone(),
            }),
            connection_kind,
            backend: session_backend,
            capabilities,
            capability_profile: profile.clone(),
            companion: Some(profile.companion.clone()),
            managed_process,
            window_title: scrcpy_window_title,
            created_at_ms: now,
        };

        self.profiles.insert(
            session_id.clone(),
            CachedProfile {
                profile,
                detected_at_ms: now,
            },
        );
        let companion_reachable = companion_runtime.is_some();
        self.sessions.insert(
            session_id.clone(),
            SessionEntry {
                session: session.clone(),
                snapshots: PhoneSnapshotRegistry::new(
                    DEFAULT_SNAPSHOT_CAPACITY,
                    self.selection.capability_cache_ttl_ms,
                ),
                cursor: PhoneCursorTracker::new(&session_id, &serial),
                companion: companion_runtime,
                companion_diagnostics: connect_diagnostics,
                last_overlay_activity_ms: now,
                overlay_active: false,
                scrcpy: scrcpy_runtime,
            },
        );

        // Light the host-owned "agent in control" overlay lease on the device
        // once a session with a reachable companion is established. Best-effort:
        // a glow failure never fails the connect (see `set_companion_overlay_active`).
        if companion_reachable {
            self.set_companion_overlay_active(&session_id, true).await;
        }

        PhoneResponse::Connected(session)
    }

    pub(crate) async fn connect_direct(&mut self, device_id: &str) -> PhoneResponse {
        let Some(provider) = &self.direct_provider else {
            return PhoneResponse::Status(self.status(false).await);
        };
        let Some(snapshot) = provider.device(device_id) else {
            return PhoneResponse::Status(self.status(false).await);
        };
        if let Some(existing) = self.sessions.values().find(|entry| {
            matches!(entry.session.connection, Some(PhoneConnectionIdentity::CompanionDirect { device_id: ref id, link_epoch, .. }) if id == device_id && link_epoch == snapshot.link_epoch)
        }) {
            return PhoneResponse::Connected(existing.session.clone());
        }
        // A newly authenticated epoch supersedes the old direct session. Drop
        // its cached profile/snapshot state so selectors cannot route work to a
        // fenced link after the device reconnects.
        let superseded: Vec<String> = self
            .sessions
            .iter()
            .filter_map(|(id, entry)| {
                matches!(
                    entry.session.connection,
                    Some(PhoneConnectionIdentity::CompanionDirect {
                        device_id: ref id0,
                        ..
                    }) if id0 == device_id
                )
                .then_some(id.clone())
            })
            .collect();
        for id in superseded {
            self.sessions.remove(&id);
            self.profiles.remove(&id);
        }
        let now = now_ms();
        let health = provider
            .dispatch(
                device_id,
                snapshot.link_epoch,
                "companion.status",
                serde_json::json!({}),
                true,
                std::time::Duration::from_secs(5),
            )
            .await
            .ok();
        let companion = companion_from_direct_health(health.as_ref());
        let should_light_direct_overlay =
            self.selection.visible_overlay && companion.rpc_reachable && companion.native_overlay;
        let session_id = format!("direct-{device_id}-{}", snapshot.link_epoch);
        let mut profile = PhoneCapabilityProfile {
            profile_id: format!("{session_id}-profile"),
            session_id: session_id.clone(),
            serial: String::new(),
            detected_at_ms: now,
            stale: false,
            refresh_state: PhoneCapabilityRefreshState::Detected,
            manufacturer: None,
            brand: None,
            model: None,
            device: None,
            target_device_kind: PhoneTargetDeviceKind::UnknownAndroid,
            hyperos_version: None,
            android_sdk: None,
            android_release: None,
            display_size: None,
            density_dpi: None,
            orientation: None,
            display_rotation_degrees: None,
            connection_kind: PhoneConnectionKind::CompanionDirect,
            companion: companion.clone(),
            scrcpy: PhoneScrcpyCapabilities::absent(),
            root_available: false,
            shizuku_available: false,
            device_owner: false,
            available_actions: Vec::new(),
            unavailable_actions: Vec::new(),
            routes: Vec::new(),
        };
        let capabilities = self.backend_capabilities(&profile);
        routing::populate_actions(&mut profile, &capabilities);
        for route in &mut profile.routes {
            route.link_epoch = Some(snapshot.link_epoch);
        }
        let session = PhoneSession {
            session_id: session_id.clone(),
            serial: String::new(),
            connection: Some(PhoneConnectionIdentity::CompanionDirect {
                device_id: device_id.to_owned(),
                link_epoch: snapshot.link_epoch,
                name: None,
                endpoint: None,
            }),
            connection_kind: PhoneConnectionKind::CompanionDirect,
            backend: PhoneBackendKind::Companion,
            capabilities,
            capability_profile: profile.clone(),
            companion: Some(profile.companion.clone()),
            managed_process: false,
            window_title: None,
            created_at_ms: now,
        };
        self.profiles.insert(
            session_id.clone(),
            CachedProfile {
                profile,
                detected_at_ms: now,
            },
        );
        self.sessions.insert(
            session_id.clone(),
            SessionEntry {
                session: session.clone(),
                snapshots: PhoneSnapshotRegistry::new(
                    DEFAULT_SNAPSHOT_CAPACITY,
                    self.selection.capability_cache_ttl_ms,
                ),
                cursor: PhoneCursorTracker::new(&session_id, ""),
                companion: None,
                companion_diagnostics: Vec::new(),
                last_overlay_activity_ms: now,
                overlay_active: false,
                scrcpy: None,
            },
        );
        // Light the persistent "agent in control" overlay for direct sessions as
        // a first-class essential, mirroring the ADB companion path above.
        // Best-effort: a failure never fails the connect.
        if should_light_direct_overlay {
            self.set_companion_overlay_active(&session_id, true).await;
        }
        PhoneResponse::Connected(session)
    }

    /// Tear down a session, scoped to sky-cua-owned state only: drop the cached
    /// profile, snapshot/cursor state, and companion runtime, and for wireless
    /// targets run `adb disconnect` unless `keep_wireless` is set. Never touches
    /// scrcpy/adb processes the operator launched themselves.
    pub(crate) async fn disconnect(
        &mut self,
        request: &PhoneDisconnectRequest,
    ) -> PhoneDisconnectResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            let (session_id, serial) = selector_ids(&request.session);
            return PhoneDisconnectResponse {
                session_id,
                serial,
                disconnected: false,
                diagnostics: vec![no_session_diagnostic(&request.session)],
            };
        };

        // Turn off the "agent in control" edge glow before tearing the session
        // down, while the companion runtime is still reachable. Best-effort: a
        // failure never blocks the disconnect (see `set_companion_overlay_active`).
        self.set_companion_overlay_active(&session_id, false).await;

        let mut entry = self.sessions.remove(&session_id);
        self.profiles.remove(&session_id);
        let serial = entry
            .as_ref()
            .map(|entry| entry.session.serial.clone())
            .unwrap_or_default();

        let mut diagnostics = Vec::new();

        // Stop the sky-cua-managed scrcpy mirror, if this session launched one and
        // we are allowed to stop it. Adopted/external windows are never killed.
        if let Some(runtime) = entry.as_mut().and_then(|entry| entry.scrcpy.take()) {
            diagnostics.extend(Self::stop_managed_scrcpy(runtime).await);
        }

        let connection_kind = entry
            .as_ref()
            .map(|entry| entry.session.connection_kind)
            .unwrap_or(PhoneConnectionKind::Unknown);
        if !request.keep_wireless
            && matches!(
                connection_kind,
                PhoneConnectionKind::WirelessDebugging | PhoneConnectionKind::LegacyTcpip
            )
            && let Err(error) =
                adb::disconnect(self.runner.as_ref(), self.configured_adb_path(), &serial).await
        {
            diagnostics.push(adb::command_error_diagnostic("adb disconnect", &error));
        }

        PhoneDisconnectResponse {
            session_id,
            serial,
            disconnected: true,
            diagnostics,
        }
    }

    /// Invalidate and rebuild a session's capability profile. The previous
    /// session must exist; the rebuilt profile is detected fresh and its action
    /// list recomputed. Returns the refreshed [`PhoneCapabilityProfile`].
    pub(crate) async fn refresh_capabilities(
        &mut self,
        selector: &PhoneSessionSelector,
    ) -> PhoneResponse {
        let Some(session_id) = self.resolve_session_id(selector) else {
            return PhoneResponse::Status(self.status(false).await);
        };
        if let Some((device_id, epoch)) = self.direct_identity(&session_id) {
            let health = match &self.direct_provider {
                Some(provider) => provider
                    .dispatch(
                        &device_id,
                        epoch,
                        "companion.status",
                        serde_json::json!({}),
                        true,
                        std::time::Duration::from_secs(5),
                    )
                    .await
                    .ok(),
                None => None,
            };
            let Some(mut profile) = self
                .profiles
                .get(&session_id)
                .map(|cached| cached.profile.clone())
            else {
                return PhoneResponse::Status(self.status(false).await);
            };
            let now = now_ms();
            profile.detected_at_ms = now;
            profile.stale = false;
            profile.refresh_state = PhoneCapabilityRefreshState::Refreshed;
            profile.companion = companion_from_direct_health(health.as_ref());
            let capabilities = self.backend_capabilities(&profile);
            routing::populate_actions(&mut profile, &capabilities);
            for route in &mut profile.routes {
                route.link_epoch = Some(epoch);
            }
            self.profiles.insert(
                session_id.clone(),
                CachedProfile {
                    profile: profile.clone(),
                    detected_at_ms: now,
                },
            );
            if let Some(entry) = self.sessions.get_mut(&session_id) {
                entry.session.capabilities = capabilities;
                entry.session.capability_profile = profile.clone();
                entry.session.companion = Some(profile.companion.clone());
            }
            return PhoneResponse::Capabilities(profile);
        }
        // `phone_refresh_capabilities` is a silent re-probe, so its install
        // convenience is gated by operator mode (no explicit install request here).
        let allow_install = self.operator_auto_install();
        self.rebuild_session(
            &session_id,
            PhoneCapabilityRefreshState::Refreshed,
            allow_install,
            None,
        )
        .await;
        match self.profiles.get(&session_id) {
            Some(cached) => PhoneResponse::Capabilities(cached.profile.clone()),
            None => PhoneResponse::Status(self.status(false).await),
        }
    }

    /// Re-detect a session's profile in place, preserving its session id/serial
    /// and re-running companion bootstrap. Used by idempotent reconnect and
    /// explicit refresh.
    ///
    /// `allow_install` gates the companion APK install/update step of the
    /// re-bootstrap. Silent refresh paths (reconnect, `phone_refresh_capabilities`)
    /// pass [`PhoneManager::operator_auto_install`] so the install convenience is
    /// gated by operator mode; the explicit `phone_install_companion` tool passes
    /// `true` because an explicit operator request is itself the operator acting.
    pub(crate) async fn rebuild_session(
        &mut self,
        session_id: &str,
        refresh: PhoneCapabilityRefreshState,
        allow_install: bool,
        requested_backend: Option<PhoneBackendKind>,
    ) {
        let Some(serial) = self
            .sessions
            .get(session_id)
            .map(|entry| entry.session.serial.clone())
        else {
            return;
        };
        let now = now_ms();
        let mut profile = device::detect_profile_with_path(
            self.runner.as_ref(),
            self.configured_adb_path(),
            session_id,
            &serial,
            &self.selection.companion_package,
            now,
            refresh,
        )
        .await;
        profile.scrcpy = self.detect_scrcpy_capabilities().await;

        let (companion_runtime, companion_diagnostics) = if self.selection.companion_enabled {
            self.bootstrap_companion_with_options(
                &serial,
                &mut profile,
                now,
                CompanionBootstrapOptions {
                    allow_install,
                    force_reinstall: false,
                    allow_downgrade: None,
                },
            )
            .await
        } else {
            (None, Vec::new())
        };

        let capabilities = self.backend_capabilities(&profile);
        routing::populate_actions(&mut profile, &capabilities);

        let companion_reachable = companion_runtime.is_some();
        let scrcpy_active = self
            .sessions
            .get(session_id)
            .is_some_and(|entry| entry.scrcpy.is_some());
        let session_backend =
            self.connect_session_backend(requested_backend, companion_reachable, scrcpy_active);
        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capability_profile = profile.clone();
            entry.session.capabilities = capabilities;
            entry.session.backend = session_backend;
            entry.session.companion = Some(profile.companion.clone());
            entry.companion = companion_runtime;
            entry.companion_diagnostics = companion_diagnostics;
        }
        self.profiles.insert(
            session_id.to_string(),
            CachedProfile {
                profile,
                detected_at_ms: now,
            },
        );

        // Re-light the edge glow after a (re)bootstrap that re-proved a reachable
        // companion, so an idempotent reconnect or an explicit refresh keeps the
        // "agent in control" signal on. Best-effort.
        if companion_reachable {
            self.set_companion_overlay_active(session_id, true).await;
        }
    }
}
