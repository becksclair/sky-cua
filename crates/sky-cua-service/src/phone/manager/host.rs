#![allow(clippy::empty_line_after_doc_comments)]
//! Host status, device listing, direct reconciliation, and Appshot helpers.

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneCapabilityRefreshState, PhoneConnectionIdentity, PhoneConnectionKind,
    PhoneDeviceState, PhoneListDevicesResponse, PhonePairWirelessRequest,
    PhonePairWirelessResponse, PhoneSessionSelector, PhoneStatusReport,
};

use super::helpers::{now_ms, phone_disabled_diagnostic};
use super::{PhoneManager, ScrcpyAdoptionCandidate};
use crate::phone::{adb, scrcpy};

impl PhoneManager {
    pub(crate) fn drain_direct_events(&mut self) {
        let Some(events) = &mut self.direct_events else {
            return;
        };
        let mut changed = Vec::new();
        while let Ok(event) = events.try_recv() {
            if event.event == "capability_changed" {
                changed.push((event.device_id, event.link_epoch));
            }
        }
        for (device_id, epoch) in changed {
            let sessions: Vec<String> = self.sessions.iter().filter_map(|(session_id, entry)| {
                matches!(entry.session.connection, Some(PhoneConnectionIdentity::CompanionDirect { device_id: ref id, link_epoch, .. }) if id == &device_id && link_epoch == epoch)
                    .then_some(session_id.clone())
            }).collect();
            for session_id in sessions {
                if let Some(cached) = self.profiles.get_mut(&session_id) {
                    cached.profile.stale = true;
                    cached.profile.refresh_state = PhoneCapabilityRefreshState::Stale;
                }
                if let Some(entry) = self.sessions.get_mut(&session_id) {
                    entry.session.capability_profile.stale = true;
                    entry.session.capability_profile.refresh_state =
                        PhoneCapabilityRefreshState::Stale;
                }
            }
        }
    }

    pub(crate) fn appshot_rejection_reason(
        &self,
        selector: &PhoneSessionSelector,
        session_id: &str,
    ) -> Option<sky_cua_platform::model::AppShotRejectionReason> {
        use sky_cua_platform::model::{AppShotCapture, AppShotRejectionReason};

        let Some(id) = selector.appshot_id.as_deref() else {
            return Some(AppShotRejectionReason::Missing);
        };
        let Some(shot) = self.appshots.get(id) else {
            return Some(AppShotRejectionReason::Stale);
        };
        // Enforce AppShot freshness window (tunable SKY_CUA_PHONE_APPSHOT_TTL_MS, default 30s, clamped >=1s).
        // Coordinate snapshots have their own PhoneSnapshotRegistry TTL; AppShots are separate and must not be recycled.
        let captured_ms = shot.captured_at.timestamp_millis() as u64;
        let age_ms = now_ms().saturating_sub(captured_ms);
        if age_ms > self.selection.appshot_ttl_ms {
            return Some(AppShotRejectionReason::Expired);
        }
        let Some((device_id, epoch)) = self.direct_identity(session_id) else {
            return Some(AppShotRejectionReason::WrongSession);
        };
        // Alias is mutually exclusive with device_id/serial in the selector.
        if selector.alias.is_some() && (selector.device_id.is_some() || selector.serial.is_some()) {
            return Some(AppShotRejectionReason::WrongTarget);
        }
        if selector
            .device_id
            .as_deref()
            .is_some_and(|value| value != device_id)
        {
            return Some(AppShotRejectionReason::WrongTarget);
        }
        if selector
            .serial
            .as_deref()
            .is_some_and(|value| value != self.serial_of(session_id))
        {
            return Some(AppShotRejectionReason::WrongTarget);
        }
        if let Some(alias) = selector.alias.as_deref() {
            let Some(target) = self.selection.aliases.get(alias) else {
                return Some(AppShotRejectionReason::WrongTarget);
            };
            if target != &device_id && target != &self.serial_of(session_id) {
                return Some(AppShotRejectionReason::WrongTarget);
            }
        }
        match &shot.capture {
            AppShotCapture::Phone {
                device_id: shot_device,
                ..
            } if shot_device != &device_id => Some(AppShotRejectionReason::WrongTarget),
            AppShotCapture::Phone { link_epoch, .. } if *link_epoch != epoch => {
                Some(AppShotRejectionReason::WrongEpoch)
            }
            AppShotCapture::Phone { .. }
                if shot.action_snapshot.session_id.as_deref() != Some(session_id) =>
            {
                Some(AppShotRejectionReason::WrongSession)
            }
            AppShotCapture::Phone { .. } => None,
            _ => Some(AppShotRejectionReason::WrongSurface),
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn appshot_matches(
        &self,
        selector: &PhoneSessionSelector,
        session_id: &str,
    ) -> bool {
        self.appshot_rejection_reason(selector, session_id)
            .is_none()
    }

    /// NodeAction-specific AppShot fence. The request carries two `appshot_id`
    /// fields at the same JSON level (flattened `session.appshot_id` and the
    /// top-level `request.appshot_id` for the node's AppShot). The MCP client
    /// populates the top-level one, so the generic `mutation_selector` check on
    /// `session.appshot_id` would shadow it and always return `Missing`.
    /// This helper checks the top-level `request.appshot_id` and honours the
    /// `view_id` fallback (appshot not required when a stable resource id is
    /// supplied).
    pub(crate) fn node_action_appshot_rejection_reason(
        &self,
        request: &sky_cua_platform::model::PhoneNodeActionRequest,
        session_id: &str,
    ) -> Option<sky_cua_platform::model::AppShotRejectionReason> {
        use sky_cua_platform::model::{AppShotCapture, AppShotRejectionReason};

        // view_id fallback: playground's stable `com.skycua.phonecompanion:id/...`
        // does not need an AppShot; the companion will `findNodeByViewId` directly.
        let Some(id) = request
            .appshot_id
            .as_deref()
            .or(request.session.appshot_id.as_deref())
        else {
            if request.view_id.is_some() {
                return None;
            }
            return Some(AppShotRejectionReason::Missing);
        };
        let Some(shot) = self.appshots.get(id) else {
            return Some(AppShotRejectionReason::Stale);
        };
        let captured_ms = shot.captured_at.timestamp_millis() as u64;
        let age_ms = now_ms().saturating_sub(captured_ms);
        if age_ms > self.selection.appshot_ttl_ms {
            return Some(AppShotRejectionReason::Expired);
        }
        let Some((device_id, epoch)) = self.direct_identity(session_id) else {
            return Some(AppShotRejectionReason::WrongSession);
        };
        // Reuse the same target/epoch/session checks as the generic fence.
        let selector = &request.session;
        if selector.alias.is_some() && (selector.device_id.is_some() || selector.serial.is_some()) {
            return Some(AppShotRejectionReason::WrongTarget);
        }
        if selector
            .device_id
            .as_deref()
            .is_some_and(|value| value != device_id)
        {
            return Some(AppShotRejectionReason::WrongTarget);
        }
        if selector
            .serial
            .as_deref()
            .is_some_and(|value| value != self.serial_of(session_id))
        {
            return Some(AppShotRejectionReason::WrongTarget);
        }
        if let Some(alias) = selector.alias.as_deref() {
            let Some(target) = self.selection.aliases.get(alias) else {
                return Some(AppShotRejectionReason::WrongTarget);
            };
            if target != &device_id && target != &self.serial_of(session_id) {
                return Some(AppShotRejectionReason::WrongTarget);
            }
        }
        match &shot.capture {
            AppShotCapture::Phone {
                device_id: shot_device,
                ..
            } if shot_device != &device_id => Some(AppShotRejectionReason::WrongTarget),
            AppShotCapture::Phone { link_epoch, .. } if *link_epoch != epoch => {
                Some(AppShotRejectionReason::WrongEpoch)
            }
            AppShotCapture::Phone { .. }
                if shot.action_snapshot.session_id.as_deref() != Some(session_id) =>
            {
                Some(AppShotRejectionReason::WrongSession)
            }
            AppShotCapture::Phone { .. } => None,
            _ => Some(AppShotRejectionReason::WrongSurface),
        }
    }

    pub(crate) async fn attach_destination_appshot(
        &mut self,
        selector: &PhoneSessionSelector,
        response: &mut sky_cua_platform::model::PhoneAppResponse,
    ) {
        if !response.success {
            return;
        }
        if let Some(session_id) = self.resolve_session_id(selector)
            && self.direct_identity(&session_id).is_some()
            && let Ok(appshot) = self.direct_appshot(&session_id).await
        {
            self.appshots
                .insert(appshot.appshot_id.clone(), appshot.clone());
            response.destination_appshot = Some(Box::new(appshot));
        }
    }

    pub(crate) fn current_time_ms() -> u64 {
        now_ms()
    }

    /// Prime the host-derived phone-scale scrcpy `--max-size` cap.
    ///
    /// Called by the daemon before a scrcpy-bearing connect with a value derived
    /// from the host display topology (see
    /// [`crate::phone::host_scrcpy_default_max_size`]). It is applied to the
    /// launch spec only when `[phone] max_size` is unset, so an explicit config
    /// override always wins. A `None` clears any previously primed default.
    pub(crate) fn set_scrcpy_host_size_default(&mut self, max_size: Option<u32>) {
        self.scrcpy_host_size_default = max_size;
    }

    /// Prime (or clear) the pre-existing scrcpy window the connect path should
    /// adopt instead of spawning a fresh mirror.
    ///
    /// The manager owns no desktop backend, so the daemon scans `list_windows`
    /// for a window carrying the deterministic `sky-cua-phone-<safe-serial>`
    /// title before a scrcpy-bearing connect and primes the match here. The
    /// connect path consumes it for the matching serial; `None` clears any prior
    /// candidate so a connect with no adoptable window launches fresh. Applied
    /// under the same lock span as `handle`, matching the size-default prime.
    pub(crate) fn set_scrcpy_adoption_candidate(
        &mut self,
        candidate: Option<ScrcpyAdoptionCandidate>,
    ) {
        self.scrcpy_adoption_candidate = candidate;
    }

    /// The primed adoption candidate for `serial`, if one was found and not
    /// already claimed by a live session. Returns `None` (so connect launches
    /// fresh) when no candidate is primed, the candidate is for a different
    /// serial, or a managed mirror is already tracked for this serial — adoption
    /// must never shadow a mirror we already own.
    pub(crate) fn adoption_candidate_for(&self, serial: &str) -> Option<ScrcpyAdoptionCandidate> {
        let candidate = self.scrcpy_adoption_candidate.as_ref()?;
        if candidate.serial != serial {
            return None;
        }
        if self.has_managed_scrcpy_for_serial(serial) {
            return None;
        }
        Some(candidate.clone())
    }

    /// Whether any tracked session already owns a managed (sky-cua-launched)
    /// scrcpy mirror for `serial`. Adoption is skipped when one exists so a
    /// reconnect never adopts a window whose live process we already control.
    pub(crate) fn has_managed_scrcpy_for_serial(&self, serial: &str) -> bool {
        self.sessions.values().any(|entry| {
            entry.session.serial == serial
                && entry.scrcpy.as_ref().is_some_and(|runtime| {
                    runtime.process.ownership == scrcpy::ScrcpyOwnership::Managed
                })
        })
    }

    /// Find a pre-existing scrcpy desktop window for `serial` the connect path can
    /// adopt, by matching the deterministic `sky-cua-phone-<safe-serial>` title in
    /// the daemon-supplied window list.
    ///
    /// The manager owns no desktop backend, so the daemon enumerates windows and
    /// passes them here; the manager owns the title derivation and the
    /// already-managed guard. Returns `None` when no window carries the title or a
    /// managed mirror for this serial is already tracked (never shadow a process we
    /// own). The matched window's pid (when exposed) is carried so the daemon can
    /// later map it by pid first, matching the launch path.
    pub(crate) fn find_adoptable_scrcpy_window(
        &self,
        serial: &str,
        windows: &[sky_cua_platform::model::WindowInfo],
    ) -> Option<ScrcpyAdoptionCandidate> {
        if self.has_managed_scrcpy_for_serial(serial) {
            return None;
        }
        let title = scrcpy::scrcpy_window_title(serial);
        let window = windows
            .iter()
            .find(|window| window.title.as_deref() == Some(title.as_str()))?;
        Some(ScrcpyAdoptionCandidate {
            serial: serial.to_string(),
            pid: window.pid,
            window_title: title,
        })
    }

    pub(crate) fn reconcile_direct_sessions(&mut self) {
        let Some(provider) = &self.direct_provider else {
            return;
        };
        let stale: Vec<String> = self
            .sessions
            .iter()
            .filter_map(|(id, entry)| {
                let PhoneConnectionIdentity::CompanionDirect {
                    device_id,
                    link_epoch,
                    ..
                } = entry.session.connection.as_ref()?
                else {
                    return None;
                };
                let current = provider.device(device_id);
                (current.is_none_or(|snapshot| snapshot.link_epoch != *link_epoch))
                    .then_some(id.clone())
            })
            .collect();
        {
            let stale_set: std::collections::HashSet<&str> =
                stale.iter().map(|s| s.as_str()).collect();
            self.appshots.retain(|_, shot| {
                shot.action_snapshot
                    .session_id
                    .as_deref()
                    .is_none_or(|id| !stale_set.contains(id))
            });
        }
        for id in &stale {
            self.sessions.remove(id);
            self.profiles.remove(id);
        }
    }

    pub(crate) async fn status(&self, refresh_devices: bool) -> PhoneStatusReport {
        let mut report = adb::probe_host_with_path(
            self.runner.as_ref(),
            self.configured_adb_path(),
            self.selection.enabled,
            self.selection.companion_enabled,
        )
        .await;
        report.default_serial = self.selection.default_serial.clone();
        report.default_backend = self.default_backend();
        // Resolve scrcpy and probe its version so status reports the real
        // accelerator path/version, not a hard-coded absence.
        let resolution = scrcpy::resolve_scrcpy(&self.selection);
        if let Some(path) = resolution.path() {
            report.scrcpy_available = true;
            report.scrcpy_path = Some(path.to_string());
            report.scrcpy_version = scrcpy::probe_version(self.runner.as_ref(), path).await;
        }
        report.sessions = self
            .sessions
            .values()
            .map(|entry| entry.session.clone())
            .collect();
        if refresh_devices {
            let devices = self.list_devices(false).await;
            report.devices = devices.devices;
            report.diagnostics.extend(devices.diagnostics);
        }
        if self.selection.direct_enabled {
            let candidates = crate::phone::direct::lan::cached_enumerate_lan_candidates();
            let listen = self
                .selection
                .direct_listen_addr
                .clone()
                .unwrap_or_else(|| "0.0.0.0:0 (wildcard)".to_string());
            let advertised = self
                .selection
                .direct_advertised_endpoint
                .clone()
                .unwrap_or_else(|| "(not set)".to_string());
            let cand_str = if candidates.is_empty() {
                "none".to_string()
            } else {
                candidates
                    .iter()
                    .map(|c| format!("{}:{}", c.iface, c.ip))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let details = serde_json::json!({
                "direct_listen_addr": self.selection.direct_listen_addr,
                "direct_advertised_endpoint": self.selection.direct_advertised_endpoint,
                "candidates": candidates.iter().map(|c| serde_json::json!({"iface": c.iface, "ip": c.ip.to_string()})).collect::<Vec<_>>(),
            })
            .to_string();
            report.diagnostics.push(sky_cua_platform::model::DiagnosticEntry {
                code: "DirectLanCandidates".to_string(),
                message: format!(
                    "direct listen={} advertised={} candidates=[{}] (use ws://<lan-ip>:<port>/phone/control; tether is rndis0/usb0 192.168.42.x)",
                    listen, advertised, cand_str
                ),
                details: Some(details),
            });
        }
        report
    }

    pub(crate) async fn disabled_status(&self) -> PhoneStatusReport {
        let mut report = self.status(false).await;
        report.diagnostics.push(phone_disabled_diagnostic());
        report
    }

    /// Resolve scrcpy and, when present, probe its version, returning the idle
    /// capability shape for a freshly-detected profile. A missing binary yields a
    /// structured `missing` capability with a reason instead of bare absence, so
    /// the agent sees why the accelerator is unavailable.
    pub(crate) async fn detect_scrcpy_capabilities(
        &self,
    ) -> sky_cua_platform::model::PhoneScrcpyCapabilities {
        match scrcpy::resolve_scrcpy(&self.selection) {
            scrcpy::ScrcpyResolution::Found { path } => {
                let version = scrcpy::probe_version(self.runner.as_ref(), &path).await;
                scrcpy::installed_idle(version)
            }
            scrcpy::ScrcpyResolution::Missing { reason } => scrcpy::missing_capabilities(reason),
        }
    }

    /// Device listing routed through ADB, with the operator's configured
    /// `[phone] primary_target_models` surfaced first: each device whose `model`
    /// matches a configured target is marked `primary=true` and sorted ahead of
    /// the rest (stable within each group, preserving adb's order otherwise). With
    /// no configured targets this is a no-op, so the listing is byte-identical to
    /// the raw adb order.
    pub(crate) async fn list_devices(&self, include_mdns: bool) -> PhoneListDevicesResponse {
        let mut response = adb::list_devices_with_path(
            self.runner.as_ref(),
            self.configured_adb_path(),
            include_mdns,
        )
        .await;
        if let Some(provider) = &self.direct_provider {
            for direct in provider.list_devices() {
                response.devices.push(sky_cua_platform::model::PhoneDevice {
                    serial: String::new(),
                    device_id: Some(direct.device_id.clone()),
                    link_epoch: Some(direct.link_epoch),
                    connection: Some(PhoneConnectionIdentity::CompanionDirect {
                        device_id: direct.device_id,
                        link_epoch: direct.link_epoch,
                        name: None,
                        endpoint: direct.peer_addr.map(|addr| addr.to_string()),
                    }),
                    state: PhoneDeviceState::Device,
                    connection_kind: PhoneConnectionKind::CompanionDirect,
                    model: None,
                    product: None,
                    device: None,
                    transport_id: None,
                    primary: false,
                    alias: None,
                });
            }
        }
        self.mark_primary_targets(&mut response.devices);
        self.populate_aliases(&mut response.devices);
        response
    }

    /// Mark and front-load the operator's configured primary target devices.
    ///
    /// A device is primary when its reported `model` matches one of
    /// `[phone] primary_target_models` (case-insensitive, trimmed). Primaries are
    /// marked `primary=true` and stably sorted ahead of non-primaries; the relative
    /// order within each group is left as adb reported it. An empty target list
    /// leaves every device untouched, so default behavior is identical.
    pub(crate) fn mark_primary_targets(
        &self,
        devices: &mut [sky_cua_platform::model::PhoneDevice],
    ) {
        if self.selection.primary_target_models.is_empty() {
            return;
        }
        for device in devices.iter_mut() {
            device.primary = device.model.as_deref().is_some_and(|model| {
                self.selection
                    .primary_target_models
                    .iter()
                    .any(|target| target.trim().eq_ignore_ascii_case(model.trim()))
            });
        }
        // Stable partition: primaries keep adb's order, then non-primaries keep
        // theirs. `sort_by_key` is stable, so `!primary` (false < true) front-loads
        // the matches without reordering within either group.
        devices.sort_by_key(|device| !device.primary);
    }

    pub(crate) fn populate_aliases(&self, devices: &mut [sky_cua_platform::model::PhoneDevice]) {
        if self.selection.aliases.is_empty() {
            return;
        }
        // Build reverse lookup: target value -> alias (first alias wins on collision).
        let mut reverse: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for (alias, target) in &self.selection.aliases {
            reverse.entry(target.as_str()).or_insert(alias.as_str());
        }
        for device in devices.iter_mut() {
            if let Some(alias) = device
                .device_id
                .as_deref()
                .and_then(|id| reverse.get(id).copied())
            {
                device.alias = Some(alias.to_string());
                continue;
            }
            if !device.serial.is_empty()
                && let Some(alias) = reverse.get(device.serial.as_str()).copied()
            {
                device.alias = Some(alias.to_string());
            }
        }
    }

    /// Run the real `adb pair host:port code` flow. The pairing code never
    /// appears in the response or any diagnostic; only its presence/absence and
    /// adb's bounded, code-free message are surfaced.
    pub(crate) async fn pair_wireless(
        &self,
        request: &PhonePairWirelessRequest,
    ) -> PhonePairWirelessResponse {
        match adb::pair_wireless(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &request.host_port,
            &request.pairing_code,
        )
        .await
        {
            Ok(outcome) => PhonePairWirelessResponse {
                paired: outcome.success,
                host_port: request.host_port.clone(),
                serial: outcome.success.then(|| request.host_port.clone()),
                diagnostics: if outcome.success {
                    Vec::new()
                } else {
                    vec![DiagnosticEntry {
                        code: "PhonePairFailed".to_string(),
                        message: format!("adb pair did not report success: {}", outcome.message),
                        details: None,
                    }]
                },
            },
            Err(error) => PhonePairWirelessResponse {
                paired: false,
                host_port: request.host_port.clone(),
                serial: None,
                diagnostics: vec![adb::command_error_diagnostic("adb pair", &error)],
            },
        }
    }
}
