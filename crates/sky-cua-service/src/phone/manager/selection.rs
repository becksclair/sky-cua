#![allow(clippy::empty_line_after_doc_comments)]
//! Session selection, profile cache, and backend capability helpers.

use sky_cua_platform::model::{
    PhoneBackendCapabilities, PhoneBackendKind, PhoneCapabilityProfile,
    PhoneCapabilityRefreshState, PhoneConnectionIdentity, PhoneConnectionKind, PhoneResponse,
    PhoneSessionSelector, PixelSize,
};

use super::helpers::{DefaultBackendKind, now_ms};
use super::{ActionContext, CachedProfile, PhoneManager};
use crate::phone::manager::routing;
use crate::phone::{adb, protocol};

impl PhoneManager {
    pub(crate) fn cached_profile(
        &self,
        session_id: &str,
        now_ms: u64,
    ) -> Option<PhoneCapabilityProfile> {
        let cached = self.profiles.get(session_id)?;
        let mut profile = cached.profile.clone();
        let age = now_ms.saturating_sub(cached.detected_at_ms);
        if age > self.selection.capability_cache_ttl_ms {
            profile.stale = true;
            profile.refresh_state = PhoneCapabilityRefreshState::Stale;
        } else if matches!(
            profile.refresh_state,
            PhoneCapabilityRefreshState::Detected | PhoneCapabilityRefreshState::Refreshed
        ) {
            // Within TTL: a freshly detected/refreshed profile is being reused by
            // a request other than the one that detected/refreshed it. The stored
            // value stays Detected/Refreshed; only this per-request clone flips.
            profile.refresh_state = PhoneCapabilityRefreshState::Reused;
        }
        Some(profile)
    }

    /// Mark a session's cached profile stale when a freshly captured frame's size
    /// no longer matches the profile's expected screenshot extent. Android `wm
    /// size` reports the natural/unrotated display size, while `screencap` returns
    /// the live rotated frame. Compare against the rotation-adjusted size so a
    /// legitimate landscape capture is not mistaken for drift.
    ///
    /// A `None`/unknown cached `display_size` is never treated as drift (there is no
    /// baseline to compare against), and a matching size is a no-op.
    pub(crate) fn mark_profile_stale_for_drift(
        &mut self,
        session_id: &str,
        fresh_size: &PixelSize,
    ) -> bool {
        if let Some(cached) = self.profiles.get_mut(session_id)
            && let Some(expected) = Self::expected_capture_size(&cached.profile)
            && expected != *fresh_size
        {
            cached.profile.stale = true;
            cached.profile.refresh_state = PhoneCapabilityRefreshState::Stale;
            return true;
        }
        false
    }

    pub(crate) fn expected_capture_size(profile: &PhoneCapabilityProfile) -> Option<PixelSize> {
        let mut size = profile.display_size.clone()?;
        if matches!(
            profile
                .display_rotation_degrees
                .unwrap_or(0)
                .rem_euclid(360),
            90 | 270
        ) {
            std::mem::swap(&mut size.width, &mut size.height);
        }
        Some(size)
    }

    /// Insert or replace a cached profile.
    #[cfg(test)]
    pub(crate) fn insert_profile(&mut self, profile: PhoneCapabilityProfile, detected_at_ms: u64) {
        self.profiles.insert(
            profile.session_id.clone(),
            CachedProfile {
                profile,
                detected_at_ms,
            },
        );
    }

    // ===================================================================
    // Helpers shared by the routing/apps children
    // ===================================================================

    /// The configured adb path from the resolved selection, if any. Threaded into
    /// every ADB wrapper so config/env overrides win over `PATH`.
    pub(crate) fn configured_adb_path(&self) -> Option<&str> {
        self.selection.adb_path.as_deref()
    }

    /// `adb connect` the configured wireless `default_serial` when
    /// `[phone] wireless_auto_connect` is enabled and the default is a wireless
    /// `host:port` target. A no-op when disabled (the default), when no default
    /// serial is configured, or when the default is a USB/emulator serial. The
    /// outcome is intentionally not surfaced here: a failed link resurfaces as the
    /// normal device-unavailable diagnostic once resolution/authorization runs.
    pub(crate) async fn wireless_auto_connect_default(&self) {
        if !self.selection.wireless_auto_connect {
            return;
        }
        let Some(default) = self.selection.default_serial.as_deref() else {
            return;
        };
        let default = default.trim();
        if default.is_empty() {
            return;
        }
        if matches!(
            adb::classify_connection_kind(default),
            PhoneConnectionKind::WirelessDebugging | PhoneConnectionKind::LegacyTcpip
        ) {
            let _ = adb::connect(self.runner.as_ref(), self.configured_adb_path(), default).await;
        }
    }

    /// Whether the host should auto-install/update the companion APK without an
    /// explicit operator request. This is one of the operator-mode privileged
    /// conveniences (`adb install -r`), so it requires BOTH
    /// `[phone] companion_operator_mode` and `[phone] companion_auto_install`. Both
    /// default to `true`, so default behavior is unchanged; turning operator mode
    /// off suppresses the silent install convenience while an explicit
    /// `phone_install_companion`/`install_companion` request still installs.
    pub(crate) fn operator_auto_install(&self) -> bool {
        self.selection.companion_operator_mode && self.selection.companion_auto_install
    }

    /// The default backend kind to advertise for new sessions and status.
    pub(crate) fn default_backend(&self) -> PhoneBackendKind {
        self.selection.default_backend_kind()
    }

    /// Resolve the backend reported on `phone_connect`. Explicit backend
    /// requests are treated as force requests: report the requested backend only
    /// if the connect path actually established the required runtime, otherwise
    /// report `None` with the diagnostics gathered by the failed setup.
    pub(crate) fn connect_session_backend(
        &self,
        requested: Option<PhoneBackendKind>,
        companion_reachable: bool,
        scrcpy_active: bool,
    ) -> PhoneBackendKind {
        match requested.unwrap_or(PhoneBackendKind::Auto) {
            PhoneBackendKind::Auto | PhoneBackendKind::None => self.default_backend(),
            PhoneBackendKind::Adb => PhoneBackendKind::Adb,
            PhoneBackendKind::Companion if companion_reachable => PhoneBackendKind::Companion,
            PhoneBackendKind::Scrcpy if scrcpy_active => PhoneBackendKind::Scrcpy,
            PhoneBackendKind::Companion | PhoneBackendKind::Scrcpy => PhoneBackendKind::None,
        }
    }

    /// Resolve a `(session_id, serial)` selector to a known session id. Prefers an
    /// explicit `session_id`, then a `serial` lookup, then — when exactly one
    /// session exists — that single session.
    pub(crate) fn resolve_session_id(&self, selector: &PhoneSessionSelector) -> Option<String> {
        // Alias is mutually exclusive with the other selectors. Fail closed
        // instead of silently ignoring one (schema enforces this for MCP, but
        // direct ServiceRequest callers bypass it).
        if selector.alias.is_some()
            && (selector.serial.is_some()
                || selector.device_id.is_some()
                || selector.session_id.is_some())
        {
            return None;
        }
        if let Some(session_id) = selector.session_id.as_deref()
            && self.sessions.contains_key(session_id)
        {
            return Some(session_id.to_string());
        }
        if let Some(serial) = selector.serial.as_deref()
            && let Some(session_id) = self.session_id_for_serial(serial)
        {
            return Some(session_id);
        }
        if let Some(device_id) = selector.device_id.as_deref()
            && let Some((session_id, _)) = self.sessions.iter().find(|(_, entry)| {
                matches!(entry.session.connection, Some(PhoneConnectionIdentity::CompanionDirect { device_id: ref id, .. }) if id == device_id)
            })
        {
            return Some(session_id.clone());
        }
        if let Some(alias) = selector.alias.as_deref()
            && let Some(target) = self.selection.aliases.get(alias)
            && let Some((session_id, _)) = self.sessions.iter().find(|(_, entry)| {
                matches!(entry.session.connection, Some(PhoneConnectionIdentity::CompanionDirect { device_id: ref id, .. }) if id == target)
            })
        {
            return Some(session_id.clone());
        }
        if let Some(alias) = selector.alias.as_deref()
            && let Some(target) = self.selection.aliases.get(alias)
            && let Some(session_id) = self.session_id_for_serial(target)
        {
            return Some(session_id);
        }
        if selector.session_id.is_none()
            && selector.serial.is_none()
            && selector.device_id.is_none()
            && selector.alias.is_none()
            && self.sessions.len() == 1
        {
            return self.sessions.keys().next().cloned();
        }
        None
    }

    /// The session id whose serial matches, if any.
    pub(crate) fn session_id_for_serial(&self, serial: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|(_, entry)| entry.session.serial == serial)
            .map(|(id, _)| id.clone())
    }

    /// The serial for a known session id, or an empty string when no session is
    /// registered under that id. Shared by the routing/apps/signals children so
    /// the open-coded `sessions.get(..).map(..serial.clone()).unwrap_or_default()`
    /// pattern lives in one place.
    pub(crate) fn serial_of(&self, session_id: &str) -> String {
        self.sessions
            .get(session_id)
            .map(|entry| entry.session.serial.clone())
            .unwrap_or_default()
    }

    pub(crate) fn direct_identity(&self, session_id: &str) -> Option<(String, u64)> {
        let entry = self.sessions.get(session_id)?;
        match entry.session.connection.as_ref()? {
            PhoneConnectionIdentity::CompanionDirect {
                device_id,
                link_epoch,
                ..
            } => Some((device_id.clone(), *link_epoch)),
            _ => None,
        }
    }

    /// Resolve a selector into an [`ActionContext`], pulling the cached profile and
    /// marking staleness against the cache TTL. Returns `None` when no session
    /// resolves. Shared by the routing and capture children.
    pub(crate) fn action_context(&self, selector: &PhoneSessionSelector) -> Option<ActionContext> {
        let session_id = self.resolve_session_id(selector)?;
        let serial = self.sessions.get(&session_id)?.session.serial.clone();
        let profile = self.cached_profile(&session_id, now_ms())?;
        Some(ActionContext {
            session_id,
            serial,
            profile,
        })
    }

    /// Resolve an [`ActionContext`] and silently refresh a stale capability
    /// profile before returning it. Real phone workflows often spend longer than
    /// the short capability TTL reading app UI; the next observe/tap should
    /// re-prove the companion instead of degrading to ADB and hiding companion
    /// pointer actions from the agent.
    pub(crate) async fn fresh_action_context(
        &mut self,
        selector: &PhoneSessionSelector,
    ) -> Option<ActionContext> {
        let session_id = self.resolve_session_id(selector)?;
        let serial = self.sessions.get(&session_id)?.session.serial.clone();
        let profile = self.cached_profile(&session_id, now_ms())?;
        if !profile.stale {
            return Some(ActionContext {
                session_id,
                serial,
                profile,
            });
        }

        if profile.connection_kind == PhoneConnectionKind::CompanionDirect {
            if !matches!(
                self.refresh_capabilities(selector).await,
                PhoneResponse::Capabilities(_)
            ) {
                return None;
            }
            let mut profile = self.profiles.get(&session_id)?.profile.clone();
            profile.refresh_state = PhoneCapabilityRefreshState::Reused;
            return Some(ActionContext {
                session_id,
                serial,
                profile,
            });
        }

        let stored_profile_stale = self
            .profiles
            .get(&session_id)
            .is_some_and(|cached| cached.profile.stale);
        if !stored_profile_stale && self.refresh_live_companion_session(&session_id).await {
            let mut profile = self.profiles.get(&session_id)?.profile.clone();
            if matches!(
                profile.refresh_state,
                PhoneCapabilityRefreshState::Detected | PhoneCapabilityRefreshState::Refreshed
            ) {
                profile.refresh_state = PhoneCapabilityRefreshState::Reused;
            }
            return Some(ActionContext {
                session_id,
                serial,
                profile,
            });
        }

        let allow_install = self.operator_auto_install();
        let requested_backend = self
            .sessions
            .get(&session_id)
            .map(|entry| entry.session.backend)
            .filter(|backend| !matches!(backend, PhoneBackendKind::Auto | PhoneBackendKind::None));
        self.rebuild_session(
            &session_id,
            PhoneCapabilityRefreshState::Refreshed,
            allow_install,
            requested_backend,
        )
        .await;
        let mut profile = self.profiles.get(&session_id)?.profile.clone();
        if matches!(
            profile.refresh_state,
            PhoneCapabilityRefreshState::Detected | PhoneCapabilityRefreshState::Refreshed
        ) {
            profile.refresh_state = PhoneCapabilityRefreshState::Reused;
        }
        Some(ActionContext {
            session_id,
            serial,
            profile,
        })
    }

    /// Refresh a stale profile through the already-authorized companion RPC lane
    /// without launching the companion setup activity. This is the normal
    /// long-running agent loop: a session can outlive the short capability TTL
    /// while the user app is foregrounded, and the next observe/tap must not bring
    /// the setup UI to the front just to re-prove capabilities. If the cached
    /// token is expired/invalid or the RPC lane is gone, return `false` so the
    /// full bootstrap path can mint and deliver a fresh token.
    pub(crate) async fn refresh_live_companion_session(&mut self, session_id: &str) -> bool {
        let caps = {
            let Some(entry) = self.sessions.get_mut(session_id) else {
                return false;
            };
            let Some(runtime) = entry.companion.as_mut() else {
                return false;
            };
            match runtime.client.capabilities().await {
                Ok(caps) => caps,
                Err(_) => return false,
            }
        };

        let now = now_ms();
        let Some(existing_profile) = self
            .sessions
            .get(session_id)
            .map(|entry| entry.session.capability_profile.clone())
        else {
            return false;
        };
        let mut profile = existing_profile;
        let previous = profile.companion.clone();
        profile.detected_at_ms = now;
        profile.refresh_state = PhoneCapabilityRefreshState::Reused;
        profile.stale = false;
        profile.companion = protocol::capabilities_from_response(
            &caps,
            None,
            previous.installed_cert_sha256.as_deref(),
            previous.expected_cert_sha256.as_deref(),
            previous.apk_sha256.as_deref(),
            false,
            previous.allow_downgrade,
        );
        profile.companion.rpc_token_expires_at_ms = previous.rpc_token_expires_at_ms;
        profile.companion.expected_version = previous.expected_version;
        profile.scrcpy = self.detect_scrcpy_capabilities().await;
        let capabilities = self.backend_capabilities(&profile);
        routing::populate_actions(&mut profile, &capabilities);

        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capability_profile = profile.clone();
            entry.session.capabilities = capabilities;
            entry.session.companion = Some(profile.companion.clone());
        }
        self.profiles.insert(
            session_id.to_string(),
            CachedProfile {
                profile,
                detected_at_ms: now,
            },
        );
        true
    }

    /// Resolve the serial a `phone_connect` should target: the explicit request
    /// serial, else the configured default, else the single connected device when
    /// exactly one is present. `None` means the target is ambiguous or absent.
    pub(crate) async fn resolve_target_serial(&self, requested: Option<&str>) -> Option<String> {
        if let Some(serial) = requested.map(str::trim).filter(|s| !s.is_empty()) {
            return Some(serial.to_string());
        }
        if let Some(default) = self.selection.default_serial.as_deref() {
            let trimmed = default.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        // Fall back to the single connected ADB device, if exactly one is in a
        // usable `device` state.
        let listed = self.list_devices(false).await;
        let usable: Vec<_> = listed
            .devices
            .iter()
            .filter(|device| {
                matches!(
                    device.state,
                    sky_cua_platform::model::PhoneDeviceState::Device
                )
            })
            .collect();
        if usable.len() == 1 {
            return Some(usable[0].serial.clone());
        }
        None
    }

    /// Observe-path cache-invalidation triggers, kept bounded (one cheap device
    /// list, run only on `phone_observe`, never per action).
    ///
    /// Wireless drop: when the session serial is no longer a connected, authorized
    /// adb device (the wireless link dropped, or the cable was pulled), the cached
    /// profile is marked stale so subsequent routing re-proves backends instead of
    /// dispatching against a vanished device.
    ///
    /// TODO(permission re-probe): the companion's on-device permission grants
    /// (accessibility/gesture/screenshot/notification listener) can be revoked
    /// while a session is live, leaving the cached profile advertising capabilities
    /// the companion can no longer serve. Re-probing the companion `health` here
    /// (observe-only) and marking the profile stale when a cached permission
    /// boolean flips is the intended trigger, but a correct re-probe must reconcile
    /// the `capabilities_from_health` derivation and the existing transport-failure
    /// invalidation in `companion_gesture`/`companion_screenshot`, so it is left for
    /// a dedicated change rather than bolted on as a fragile half-measure. The
    /// wireless-drop trigger below is implemented in full.
    pub(crate) async fn invalidate_on_observe_triggers(&mut self, session_id: &str, serial: &str) {
        // Direct sessions are fenced by their authenticated link epoch and have
        // no ADB serial. An empty serial is therefore not an ADB disconnect and
        // must not force the direct profile through the ADB rebuild path.
        if self.sessions.get(session_id).is_some_and(|entry| {
            entry.session.connection_kind == PhoneConnectionKind::CompanionDirect
        }) {
            return;
        }
        if !self.serial_is_authorized_device(serial).await
            && let Some(cached) = self.profiles.get_mut(session_id)
        {
            cached.profile.stale = true;
            cached.profile.refresh_state = PhoneCapabilityRefreshState::Stale;
        }
    }

    /// True when `serial` appears in `adb devices` in the usable `Device` state.
    /// Used by `connect` to reject bogus or unreachable serials before minting a
    /// session instead of failing only on the first action.
    pub(crate) async fn serial_is_authorized_device(&self, serial: &str) -> bool {
        self.list_devices(false).await.devices.iter().any(|device| {
            device.serial == serial
                && matches!(
                    device.state,
                    sky_cua_platform::model::PhoneDeviceState::Device
                )
        })
    }

    /// Build the quick backend-availability summary from a detected profile. ADB
    /// is available whenever a serial resolved; companion/scrcpy availability and
    /// the per-action affordances come from the profile's capability fields.
    pub(crate) fn backend_capabilities(
        &self,
        profile: &PhoneCapabilityProfile,
    ) -> PhoneBackendCapabilities {
        let companion = &profile.companion;
        let companion_up = !profile.stale && companion.rpc_reachable;
        let companion_gestures = companion_up && companion.gesture_dispatch;
        let scrcpy_up = !profile.stale && profile.scrcpy.active;
        let adb_up = profile.connection_kind != PhoneConnectionKind::CompanionDirect
            && !profile.serial.is_empty();
        PhoneBackendCapabilities {
            adb: adb_up,
            companion: companion_up,
            scrcpy: scrcpy_up,
            screenshot: (adb_up || (companion_up && companion.screenshot)),
            gestures: companion_gestures,
            text_input: adb_up || companion_up,
            key_input: adb_up || companion_up,
            accessibility_tree: companion_up && companion.accessibility_tree,
            notifications: companion_up && companion.notifications,
            app_management: adb_up || companion_up,
            // Host-visible only when the companion overlay is reachable to draw
            // it AND a scrcpy mirror is mapped to display the device overlay on
            // the host; the host no longer draws the phone cursor itself. The
            // config `visible_overlay` toggle gates both visible planes: with it
            // off, the host suppresses every companion visible-overlay call, so the
            // session must not advertise an overlay it never lights.
            host_visible_overlay: self.selection.visible_overlay
                && scrcpy_up
                && profile.scrcpy.host_window_mapped
                && companion_up
                && companion.native_overlay,
            screenshot_synthetic_cursor: self.selection.screenshot_cursor,
            phone_native_overlay: self.selection.visible_overlay
                && companion_up
                && companion.native_overlay,
        }
    }
}
