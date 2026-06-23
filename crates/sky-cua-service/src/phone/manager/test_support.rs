//! Test-only `PhoneManager` constructors and fixtures (split from
//! `manager/mod.rs` to keep it under the god-file threshold).

use super::*;
use crate::phone::command::FakeCommandRunner;

impl PhoneManager {
    /// Test-only access to the current epoch-millis clock.
    pub(crate) fn now_ms_for_tests() -> u64 {
        now_ms()
    }

    /// A manager backed by an unscripted [`FakeCommandRunner`] and the default
    /// resolved selection. The daemon's phone routing tests use this so they
    /// never depend on a real `adb` binary being present (or absent).
    pub(crate) fn with_fake_runner_for_tests() -> Self {
        Self::with_runner(Arc::new(FakeCommandRunner::new()), default_selection())
    }

    /// Test-only profile insertion exposed to the sibling `tests` module.
    pub(crate) fn insert_profile_for_tests(
        &mut self,
        profile: PhoneCapabilityProfile,
        detected_at_ms: u64,
    ) {
        self.insert_profile(profile, detected_at_ms);
    }

    /// Test-only `phone_list_devices` invocation exposed to the sibling `tests`
    /// module. Drives the real device-list path (adb probe + primary-target
    /// marking/ordering) so the `primary_target_models` wiring is exercised
    /// end-to-end against a scripted runner.
    pub(crate) async fn list_devices_for_tests(
        &self,
    ) -> sky_cua_platform::model::PhoneListDevicesResponse {
        self.list_devices(false).await
    }

    /// Test-only minimal detached [`PhoneCapabilityProfile`] (no registered
    /// session) the caller can mutate before passing to `cursor_capabilities` /
    /// `backend_capabilities`. Mirrors the absent-profile shape used elsewhere in
    /// the test fixtures.
    pub(crate) fn detached_profile_for_tests(&self) -> PhoneCapabilityProfile {
        PhoneCapabilityProfile {
            profile_id: "detached-profile".to_string(),
            session_id: "detached".to_string(),
            serial: "detached".to_string(),
            detected_at_ms: now_ms(),
            stale: false,
            refresh_state: PhoneCapabilityRefreshState::Detected,
            manufacturer: None,
            brand: None,
            model: None,
            device: None,
            target_device_kind: sky_cua_platform::model::PhoneTargetDeviceKind::UnknownAndroid,
            hyperos_version: None,
            android_sdk: None,
            android_release: None,
            display_size: None,
            density_dpi: None,
            orientation: None,
            display_rotation_degrees: None,
            connection_kind: PhoneConnectionKind::Usb,
            companion: sky_cua_platform::model::PhoneCompanionCapabilities::absent(
                self.selection.companion_package.clone(),
            ),
            scrcpy: sky_cua_platform::model::PhoneScrcpyCapabilities::absent(),
            root_available: false,
            shizuku_available: false,
            device_owner: false,
            available_actions: Vec::new(),
            unavailable_actions: Vec::new(),
        }
    }

    /// Test-only cache lookup exposed to the sibling `tests` module.
    pub(crate) fn cached_profile_for_tests(
        &self,
        session_id: &str,
        now_ms: u64,
    ) -> Option<PhoneCapabilityProfile> {
        self.cached_profile(session_id, now_ms)
    }

    /// Test-only: set the expected packaged-APK SHA-256 in the resolved selection
    /// before a connect, standing in for build metadata next to the packaged APK.
    /// Used by the companion-identity reporting tests to drive `apk_sha256` onto
    /// the capability report.
    pub(crate) fn set_companion_apk_sha256_for_tests(&mut self, sha256: &str) {
        self.selection.companion_apk_sha256 = Some(sha256.to_string());
    }

    /// Test-only: overwrite a session's cached + session-view `display_size`,
    /// standing in for a live orientation/resolution change. Used by the snapshot
    /// orientation/resolution-rejection tests to drift the profile away from the
    /// size a previously registered snapshot was captured at.
    pub(crate) fn set_display_size_for_tests(
        &mut self,
        session_id: &str,
        size: sky_cua_platform::model::PixelSize,
    ) {
        if let Some(cached) = self.profiles.get_mut(session_id) {
            cached.profile.display_size = Some(size.clone());
        }
        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capability_profile.display_size = Some(size);
        }
    }

    /// Test-only: overwrite a session's cached + session-view live display
    /// rotation, standing in for a device held in landscape while `wm size`
    /// continues to report the natural portrait panel size.
    pub(crate) fn set_display_rotation_for_tests(
        &mut self,
        session_id: &str,
        rotation_degrees: Option<i32>,
    ) {
        if let Some(cached) = self.profiles.get_mut(session_id) {
            cached.profile.display_rotation_degrees = rotation_degrees;
            cached.profile.orientation =
                rotation_degrees.map(|degrees| match degrees.rem_euclid(180) {
                    90 => "landscape".to_string(),
                    _ => "portrait".to_string(),
                });
        }
        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capability_profile.display_rotation_degrees = rotation_degrees;
            entry.session.capability_profile.orientation =
                rotation_degrees.map(|degrees| match degrees.rem_euclid(180) {
                    90 => "landscape".to_string(),
                    _ => "portrait".to_string(),
                });
        }
    }

    /// Test-only: register a session whose cached profile reports a reachable,
    /// gesture-capable companion and whose runtime holds a [`CompanionClient`] dialing
    /// `companion_port` with `token`. Used by the app-management routing tests
    /// to drive companion-preferred dispatch and ADB fallback against an
    /// in-process fake companion server, without standing up the full
    /// install/forward/probe bootstrap.
    pub(crate) fn insert_companion_session_for_tests(
        &mut self,
        session_id: &str,
        serial: &str,
        companion_port: u16,
        token: &str,
        detected_at_ms: u64,
    ) {
        let mut companion = sky_cua_platform::model::PhoneCompanionCapabilities::absent(
            self.selection.companion_package.clone(),
        );
        companion.installed = true;
        companion.rpc_reachable = true;
        companion.gesture_dispatch = true;
        companion.screenshot = true;
        companion.accessibility_tree = true;
        companion.notifications = true;
        companion.native_overlay = true;
        companion.native_overlay_pass_through = true;
        let mut profile = PhoneCapabilityProfile {
            profile_id: format!("{session_id}-profile"),
            session_id: session_id.to_string(),
            serial: serial.to_string(),
            detected_at_ms,
            stale: false,
            refresh_state: PhoneCapabilityRefreshState::Detected,
            manufacturer: None,
            brand: None,
            model: None,
            device: None,
            target_device_kind: sky_cua_platform::model::PhoneTargetDeviceKind::UnknownAndroid,
            hyperos_version: None,
            android_sdk: None,
            android_release: None,
            display_size: None,
            density_dpi: None,
            orientation: None,
            display_rotation_degrees: None,
            connection_kind: PhoneConnectionKind::Usb,
            companion,
            scrcpy: sky_cua_platform::model::PhoneScrcpyCapabilities::absent(),
            root_available: false,
            shizuku_available: false,
            device_owner: false,
            available_actions: Vec::new(),
            unavailable_actions: Vec::new(),
        };
        let capabilities = self.backend_capabilities(&profile);
        super::routing::populate_actions(&mut profile, &capabilities);

        let session = PhoneSession {
            session_id: session_id.to_string(),
            serial: serial.to_string(),
            connection_kind: PhoneConnectionKind::Usb,
            backend: PhoneBackendKind::Companion,
            capabilities,
            capability_profile: profile.clone(),
            companion: Some(profile.companion.clone()),
            managed_process: false,
            window_title: None,
            created_at_ms: detected_at_ms,
        };
        self.profiles.insert(
            session_id.to_string(),
            CachedProfile {
                profile,
                detected_at_ms,
            },
        );
        self.sessions.insert(
            session_id.to_string(),
            SessionEntry {
                session,
                snapshots: PhoneSnapshotRegistry::new(
                    DEFAULT_SNAPSHOT_CAPACITY,
                    self.selection.capability_cache_ttl_ms,
                ),
                cursor: PhoneCursorTracker::new(session_id.to_string(), serial.to_string()),
                companion: Some(CompanionRuntime {
                    client: CompanionClient::new(companion_port, token),
                }),
                companion_diagnostics: Vec::new(),
                last_overlay_activity_ms: detected_at_ms,
                overlay_active: self.selection.visible_overlay,
                scrcpy: None,
            },
        );
    }

    /// Test-only: register a minimal session that owns a managed scrcpy runtime.
    ///
    /// The runtime pairs a test-constructed managed [`scrcpy::ScrcpyProcess`] with a
    /// real but harmless long-lived child (`sleep`) standing in for the scrcpy
    /// process, so `phone_disconnect`'s stop path can actually `kill` something
    /// without spawning real scrcpy. Returns the session id so the caller can drive
    /// disconnect against it.
    pub(crate) fn insert_scrcpy_session_for_tests(&mut self, serial: &str) -> String {
        let session_id = format!("phone-sess-scrcpy-{serial}");
        let process = scrcpy::ScrcpyProcess::managed(424_242, serial, scrcpy::ScrcpyCodec::H265);
        // A real child we are allowed to kill, standing in for the scrcpy process.
        let child = tokio::process::Command::new("sleep")
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .expect("spawn stand-in child process for scrcpy test");

        let mut profile = PhoneCapabilityProfile {
            profile_id: format!("{session_id}-profile"),
            session_id: session_id.clone(),
            serial: serial.to_string(),
            detected_at_ms: now_ms(),
            stale: false,
            refresh_state: PhoneCapabilityRefreshState::Detected,
            manufacturer: None,
            brand: None,
            model: None,
            device: None,
            target_device_kind: sky_cua_platform::model::PhoneTargetDeviceKind::UnknownAndroid,
            hyperos_version: None,
            android_sdk: None,
            android_release: None,
            display_size: None,
            density_dpi: None,
            orientation: None,
            display_rotation_degrees: None,
            connection_kind: PhoneConnectionKind::Usb,
            companion: sky_cua_platform::model::PhoneCompanionCapabilities::absent(
                self.selection.companion_package.clone(),
            ),
            scrcpy: scrcpy::active_capabilities(Some("4.0".to_string()), &process, false),
            root_available: false,
            shizuku_available: false,
            device_owner: false,
            available_actions: Vec::new(),
            unavailable_actions: Vec::new(),
        };
        let capabilities = self.backend_capabilities(&profile);
        super::routing::populate_actions(&mut profile, &capabilities);

        let session = PhoneSession {
            session_id: session_id.clone(),
            serial: serial.to_string(),
            connection_kind: PhoneConnectionKind::Usb,
            backend: PhoneBackendKind::Scrcpy,
            capabilities,
            capability_profile: profile.clone(),
            companion: Some(profile.companion.clone()),
            managed_process: true,
            window_title: Some(process.window_title.clone()),
            created_at_ms: now_ms(),
        };
        let detected_at_ms = now_ms();
        self.profiles.insert(
            session_id.clone(),
            CachedProfile {
                profile,
                detected_at_ms,
            },
        );
        self.sessions.insert(
            session_id.clone(),
            SessionEntry {
                session,
                snapshots: PhoneSnapshotRegistry::new(
                    DEFAULT_SNAPSHOT_CAPACITY,
                    self.selection.capability_cache_ttl_ms,
                ),
                cursor: PhoneCursorTracker::new(session_id.clone(), serial.to_string()),
                companion: None,
                companion_diagnostics: Vec::new(),
                last_overlay_activity_ms: detected_at_ms,
                overlay_active: false,
                scrcpy: Some(ScrcpyRuntime {
                    process,
                    child: Some(child),
                    mapping: None,
                    mapping_attempts_exhausted: false,
                }),
            },
        );
        session_id
    }

    /// Test-only: replace a session's managed scrcpy stand-in child with one that
    /// has already exited, standing in for a mirror that crashed (or whose window
    /// the operator closed) mid-session. Spawns a short-lived child (`true`) and
    /// awaits its exit before swapping it in, so the runtime's
    /// `child.try_wait()` reports `Ok(Some(_))` exactly as the liveness watchdog
    /// expects. The cached profile is left reporting the mirror as still
    /// active/mapped (the pre-crash state the watchdog must downgrade). A no-op
    /// when the session has no managed runtime.
    pub(crate) async fn kill_scrcpy_child_for_tests(&mut self, session_id: &str) {
        let mut child = tokio::process::Command::new("true")
            .kill_on_drop(true)
            .spawn()
            .expect("spawn short-lived stand-in child for crashed scrcpy test");
        // Await the exit so the child is reaped and `try_wait` is deterministic.
        let _ = child.wait().await;
        if let Some(entry) = self.sessions.get_mut(session_id)
            && let Some(runtime) = entry.scrcpy.as_mut()
        {
            runtime.child = Some(child);
        }
    }

    /// Test-only: register a session that ADOPTED an existing scrcpy window (a
    /// previous run's mirror, or one the operator left up). Ownership is
    /// `Adopted` and there is no child we own (`child: None`), so the liveness
    /// watchdog must never `try_wait` it and disconnect must never kill it. The
    /// profile carries a known `display_size`/`orientation` so the host-window
    /// mapping path can compute a content rect against it. Returns the session id.
    pub(crate) fn insert_adopted_scrcpy_session_for_tests(
        &mut self,
        serial: &str,
        device_size: sky_cua_platform::model::PixelSize,
        orientation: &str,
    ) -> String {
        let session_id = format!("phone-sess-adopted-{serial}");
        let window_title = scrcpy::scrcpy_window_title(serial);
        let process = scrcpy::ScrcpyProcess::adopted(Some(99), &window_title, serial);

        let mut profile = PhoneCapabilityProfile {
            profile_id: format!("{session_id}-profile"),
            session_id: session_id.clone(),
            serial: serial.to_string(),
            detected_at_ms: now_ms(),
            stale: false,
            refresh_state: PhoneCapabilityRefreshState::Detected,
            manufacturer: None,
            brand: None,
            model: None,
            device: None,
            target_device_kind: sky_cua_platform::model::PhoneTargetDeviceKind::UnknownAndroid,
            hyperos_version: None,
            android_sdk: None,
            android_release: None,
            display_size: Some(device_size.clone()),
            density_dpi: None,
            orientation: Some(orientation.to_string()),
            display_rotation_degrees: None,
            connection_kind: PhoneConnectionKind::Usb,
            companion: sky_cua_platform::model::PhoneCompanionCapabilities::absent(
                self.selection.companion_package.clone(),
            ),
            scrcpy: scrcpy::active_capabilities(None, &process, false),
            root_available: false,
            shizuku_available: false,
            device_owner: false,
            available_actions: Vec::new(),
            unavailable_actions: Vec::new(),
        };
        let capabilities = self.backend_capabilities(&profile);
        super::routing::populate_actions(&mut profile, &capabilities);

        let session = PhoneSession {
            session_id: session_id.clone(),
            serial: serial.to_string(),
            connection_kind: PhoneConnectionKind::Usb,
            backend: PhoneBackendKind::Scrcpy,
            capabilities,
            capability_profile: profile.clone(),
            companion: Some(profile.companion.clone()),
            managed_process: false,
            window_title: Some(window_title),
            created_at_ms: now_ms(),
        };
        let detected_at_ms = now_ms();
        self.profiles.insert(
            session_id.clone(),
            CachedProfile {
                profile,
                detected_at_ms,
            },
        );
        self.sessions.insert(
            session_id.clone(),
            SessionEntry {
                session,
                snapshots: PhoneSnapshotRegistry::new(
                    DEFAULT_SNAPSHOT_CAPACITY,
                    self.selection.capability_cache_ttl_ms,
                ),
                cursor: PhoneCursorTracker::new(session_id.clone(), serial.to_string()),
                companion: None,
                companion_diagnostics: Vec::new(),
                last_overlay_activity_ms: detected_at_ms,
                overlay_active: false,
                scrcpy: Some(ScrcpyRuntime {
                    process,
                    child: None,
                    mapping: None,
                    mapping_attempts_exhausted: false,
                }),
            },
        );
        session_id
    }

    /// Test-only: map a device-pixel point into host-desktop coordinates through
    /// the session's stored scrcpy content-rect mapping, returned as a plain
    /// `(x, y)` tuple. Used by the re-map tests to assert the scrcpy window
    /// mapping recomputes after a window resize. Returns `None` when the session
    /// is not host-mapped.
    pub(crate) fn device_point_to_host_for_tests(
        &self,
        session_id: &str,
        x: f64,
        y: f64,
    ) -> Option<(f64, f64)> {
        self.scrcpy_device_to_host_for_tests(session_id, x, y)
    }

    /// Test-only: the ownership of a session's tracked scrcpy process, if any.
    /// Used by the adoption tests to assert an adopted session is `Adopted`.
    /// `pub(in crate::phone)` because it returns the phone-private
    /// [`scrcpy::ScrcpyOwnership`]; the sibling `tests` module reaches it through
    /// that path.
    pub(in crate::phone) fn scrcpy_ownership_for_tests(
        &self,
        session_id: &str,
    ) -> Option<scrcpy::ScrcpyOwnership> {
        self.sessions
            .get(session_id)?
            .scrcpy
            .as_ref()
            .map(|runtime| runtime.process.ownership)
    }

    /// Test-only: whether a session's tracked scrcpy runtime owns a child we
    /// control (`Some` for a managed mirror, `None` for an adopted/external
    /// window). Used by the adoption tests to assert the adopted runtime carries
    /// no child the liveness watchdog could `try_wait`.
    pub(crate) fn scrcpy_has_owned_child_for_tests(&self, session_id: &str) -> Option<bool> {
        self.sessions
            .get(session_id)?
            .scrcpy
            .as_ref()
            .map(|runtime| runtime.child.is_some())
    }

    /// Test-only: register a managed scrcpy session whose profile carries a known
    /// `display_size`/`orientation`, so the host-window-mapping path
    /// (`scrcpy_window_to_map` -> `set_scrcpy_window_mapping`) has a real device
    /// rectangle and rotation to compute the content rect against. The mirror
    /// starts active but unmapped (`host_window_mapped=false`), matching the state
    /// immediately after a managed launch.
    pub(crate) fn insert_mappable_scrcpy_session_for_tests(
        &mut self,
        serial: &str,
        device_size: sky_cua_platform::model::PixelSize,
        orientation: &str,
    ) -> String {
        self.insert_mappable_scrcpy_session_with_rotation_for_tests(
            serial,
            device_size,
            orientation,
            None,
        )
    }

    /// Test-only: like [`insert_mappable_scrcpy_session_for_tests`] but also
    /// records the profile's live `display_rotation_degrees` quarter, so the
    /// host-window-mapping path exercises the exact-quarter rotation (notably
    /// 180/270, which the orientation label alone cannot distinguish) instead of
    /// the label-derived fallback.
    ///
    /// [`insert_mappable_scrcpy_session_for_tests`]:
    /// Self::insert_mappable_scrcpy_session_for_tests
    pub(crate) fn insert_mappable_scrcpy_session_with_rotation_for_tests(
        &mut self,
        serial: &str,
        device_size: sky_cua_platform::model::PixelSize,
        orientation: &str,
        rotation_degrees: Option<i32>,
    ) -> String {
        let session_id = self.insert_scrcpy_session_for_tests(serial);
        let orientation = orientation.to_string();
        if let Some(entry) = self.sessions.get_mut(&session_id) {
            entry.session.capability_profile.display_size = Some(device_size.clone());
            entry.session.capability_profile.orientation = Some(orientation.clone());
            entry.session.capability_profile.display_rotation_degrees = rotation_degrees;
        }
        if let Some(cached) = self.profiles.get_mut(&session_id) {
            cached.profile.display_size = Some(device_size);
            cached.profile.orientation = Some(orientation);
            cached.profile.display_rotation_degrees = rotation_degrees;
        }
        session_id
    }
}
