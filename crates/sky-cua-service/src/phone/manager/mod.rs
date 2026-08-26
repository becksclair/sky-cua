//! `PhoneManager`: the service-owned phone runtime the daemon dispatches into.
//!
//! Mirrors the role `crate::browser` plays for the browser family, but as a
//! single owned object the daemon holds behind a `tokio::sync::Mutex` (phone
//! state — sessions, the capability cache, managed scrcpy/companion processes —
//! is mutable and must be serialized).
//!
//! The manager owns the [`CommandRunner`] seam every backend goes through, a
//! per-session [`PhoneCapabilityProfile`] cache, and per-session snapshot/cursor
//! state. `phone_connect` detects a profile and builds a session; observation and
//! action tools route deterministically (companion -> scrcpy -> ADB) from that
//! cached profile, and every response states the backend that handled it plus the
//! capability profile id in force.
//!
//! The implementation is split across this module's children to keep each file
//! under the god-file threshold:
//! - this file: state, lifecycle (`connect`/`disconnect`/`pair`), the cache, and
//!   the dispatch table;
//! - [`routing`]: backend selection plus the coordinate/text/key/screenshot/
//!   observe execution paths;
//! - [`apps`]: app management, notifications, accessibility, companion status,
//!   and settings.

mod apps;
mod appshot;
mod capture;
mod companion_lane;
mod features;
mod routing;
mod scrcpy_lane;
mod signals;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sky_cua_platform::config::{ResolvedPhoneSelection, resolved_phone_selection};
use sky_cua_platform::model::{
    DiagnosticEntry, PHONE_SMS_QUERY_SCHEMA, PhoneBackendCapabilities, PhoneBackendKind,
    PhoneCapabilityProfile, PhoneCapabilityRefreshState, PhoneCompanionCapabilities,
    PhoneConnectRequest, PhoneConnectionIdentity, PhoneConnectionKind, PhoneDeviceState,
    PhoneDisconnectRequest, PhoneDisconnectResponse, PhoneListDevicesResponse, PhoneObserveRequest,
    PhonePairWirelessRequest, PhonePairWirelessResponse, PhoneRequest, PhoneResponse,
    PhoneScrcpyCapabilities, PhoneSession, PhoneSessionSelector, PhoneSmsQueryError,
    PhoneSmsQueryRequest, PhoneSmsQueryResponse, PhoneStatusReport, PhoneTargetDeviceKind,
    PixelSize,
};

use super::adb;
use super::command::{CommandRunner, RealCommandRunner};
use super::cursor::PhoneCursorTracker;
use super::device;
use super::direct::provider::CompanionDirectProvider;
use super::direct::{DirectRuntimeError, DirectRuntimeHandle};
use super::protocol;
use super::protocol::client::CompanionClient;
use super::protocol::identity::CompanionBootstrapOptions;
use super::scrcpy;
use super::snapshot::{DEFAULT_SNAPSHOT_CAPACITY, PhoneSnapshotRegistry};

const COMPANION_OVERLAY_IDLE_TIMEOUT_MS: u64 = 20_000;

/// One cached capability profile plus the wall-clock time it was detected, used
/// to compute staleness against the resolved cache TTL.
struct CachedProfile {
    profile: PhoneCapabilityProfile,
    detected_at_ms: u64,
}

/// A resolved action target: the session id/serial and the cached profile
/// (already staleness-marked). The routing and capture children build one from a
/// selector before dispatching, so backend selection reads a single consistent
/// snapshot of session state.
pub(super) struct ActionContext {
    pub(super) session_id: String,
    pub(super) serial: String,
    pub(super) profile: PhoneCapabilityProfile,
}

/// Per-session runtime state the manager owns alongside the cached profile: the
/// public [`PhoneSession`] handed back to callers, the bounded snapshot registry
/// coordinate actions validate against, the cursor tracker (one per serial, never
/// shared), an optional live companion RPC client + session token, and the
/// structured diagnostics the most recent companion bootstrap produced (install
/// outcome, forward/probe failures) so `phone_companion_status` and
/// `phone_install_companion` can surface them instead of discarding them.
struct SessionEntry {
    session: PhoneSession,
    snapshots: PhoneSnapshotRegistry,
    cursor: PhoneCursorTracker,
    companion: Option<CompanionRuntime>,
    companion_diagnostics: Vec<DiagnosticEntry>,
    /// Last host-side use of this session that should keep the phone-native
    /// "agent in control" overlay lit. A watchdog clears the overlay after a
    /// bounded idle window even if the MCP client exits without disconnecting.
    last_overlay_activity_ms: u64,
    /// Whether the manager believes the companion's persistent active overlay is
    /// currently lit for this session. This avoids repeated off calls and lets a
    /// later action relight the overlay after the idle watchdog cleared it.
    overlay_active: bool,
    /// The managed scrcpy mirror for this session, present only when `phone_connect`
    /// launched one and it stayed up. `None` means no sky-cua-owned mirror is
    /// running (scrcpy was not requested, did not resolve, or failed to launch).
    scrcpy: Option<ScrcpyRuntime>,
}

/// A reachable companion endpoint: the RPC client carries the ephemeral session
/// token (already delivered to the app via the setup intent) on every call.
/// Present only when `adb forward` + token provisioning + a `capabilities` probe
/// all succeeded during connect.
struct CompanionRuntime {
    client: CompanionClient,
}

/// A live scrcpy mirror backing a session: the ownership/liveness record plus the
/// owned [`tokio::process::Child`] we control, when sky-cua launched it.
///
/// `child` is `Some` only for a *managed* mirror this service spawned; that child
/// is spawned with `kill_on_drop`, so a daemon crash cannot orphan the mirror
/// window even if the orderly stop path never runs, and `phone_disconnect` can
/// terminate it. For an *adopted* (or external) window we mapped but did not
/// launch, there is no child we own, so `child` is `None`: the liveness watchdog
/// never `try_wait`s it (we cannot poll a process we do not own) and disconnect
/// never kills it. Ownership is the authority for what may be stopped; `child`
/// presence mirrors it.
struct ScrcpyRuntime {
    process: scrcpy::ScrcpyProcess,
    child: Option<tokio::process::Child>,
    /// The host-window content-rect mapping, present once the daemon has located
    /// the desktop window and computed the letterboxed content box. `None` until
    /// the window is mapped; presence is what flips `scrcpy.host_window_mapped`
    /// true and unlocks the host-visible cursor overlay plane. The content rect
    /// is the only geometry overlay math needs (the full host window rect, with
    /// decorations, is consumed inline by the daemon when it computes this).
    mapping: Option<scrcpy::ScrcpyContentRect>,
    /// Whether the daemon's bounded window-mapping retry round already ran and
    /// failed for this mirror. Set once after an exhausted retry round so
    /// `scrcpy_window_to_map` stops re-offering the same never-mapping window on
    /// every subsequent phone request (the desktop window never registered, or
    /// has no bounds). Re-armed (reset to `false`) only when a fresh
    /// [`ScrcpyRuntime`] is built on reconnect/refresh.
    mapping_attempts_exhausted: bool,
}

/// The service-owned phone runtime. See module docs for responsibilities.
pub(crate) struct PhoneManager {
    runner: Arc<dyn CommandRunner>,
    selection: ResolvedPhoneSelection,
    /// Per-session capability profile cache, keyed by `session_id`.
    profiles: HashMap<String, CachedProfile>,
    /// Per-session runtime state, keyed by `session_id`.
    sessions: HashMap<String, SessionEntry>,
    /// Host-derived phone-scale scrcpy `--max-size` cap, primed by the daemon
    /// from the display topology before a scrcpy-bearing connect. Applied only
    /// when `[phone] max_size` is unset; `None` leaves the configured value
    /// (possibly also unset) untouched. The manager owns no desktop backend, so
    /// this is the daemon's one channel for host-display-aware mirror sizing.
    scrcpy_host_size_default: Option<u32>,
    /// A pre-existing scrcpy window the daemon found (by the deterministic
    /// `sky-cua-phone-<safe-serial>` title) before a scrcpy-bearing connect, so
    /// the connect path adopts it instead of spawning a second mirror. The
    /// manager owns no desktop backend, so the daemon scans windows and primes
    /// this candidate; the connect path consumes it for the matching serial.
    /// `None` means no adoptable window was found, so connect launches fresh.
    scrcpy_adoption_candidate: Option<ScrcpyAdoptionCandidate>,
    /// Optional ADB-independent CompanionDirect seam. It is kept separate
    /// from serial-keyed sessions until the public projection is promoted.
    direct_provider: Option<CompanionDirectProvider>,
    direct_events: Option<tokio::sync::broadcast::Receiver<super::direct::DirectDeviceEvent>>,
    appshots: HashMap<String, sky_cua_platform::model::AppShotEnvelope>,
}

/// A pre-existing scrcpy desktop window the daemon discovered for a serial, primed
/// onto the manager before a connect so the connect path can adopt it (map it,
/// skip the spawn) rather than launching a duplicate mirror.
#[derive(Clone)]
pub(crate) struct ScrcpyAdoptionCandidate {
    /// The serial whose deterministic window title the daemon matched.
    pub(crate) serial: String,
    /// The matched window's OS pid, when the backend exposed one.
    pub(crate) pid: Option<u32>,
    /// The matched window's title (the `sky-cua-phone-<safe-serial>` slug).
    pub(crate) window_title: String,
}

impl PhoneManager {
    /// Construct the manager the daemon owns: a real command runner plus the
    /// resolved phone selection. A config error degrades to the default selection
    /// rather than failing daemon startup, matching how the browser bridge
    /// tolerates missing configuration.
    pub(crate) fn new() -> Self {
        let selection = resolved_phone_selection().unwrap_or_else(|_| default_selection());
        Self::with_runner(Arc::new(RealCommandRunner), selection)
    }

    /// Construct with an explicit runner and selection. The daemon uses
    /// [`PhoneManager::new`]; tests use this to inject a
    /// [`super::command::FakeCommandRunner`] and a deterministic selection.
    pub(crate) fn with_runner(
        runner: Arc<dyn CommandRunner>,
        selection: ResolvedPhoneSelection,
    ) -> Self {
        Self {
            runner,
            selection,
            profiles: HashMap::new(),
            sessions: HashMap::new(),
            scrcpy_host_size_default: None,
            scrcpy_adoption_candidate: None,
            direct_provider: None,
            direct_events: None,
            appshots: HashMap::new(),
        }
    }

    pub(crate) fn set_direct_runtime(&mut self, runtime: Option<DirectRuntimeHandle>) {
        self.direct_provider = runtime.map(CompanionDirectProvider::new);
        self.direct_events = self
            .direct_provider
            .as_ref()
            .map(CompanionDirectProvider::subscribe);
    }

    #[cfg(test)]
    pub(crate) fn direct_provider(&self) -> Option<CompanionDirectProvider> {
        self.direct_provider.clone()
    }

    fn drain_direct_events(&mut self) {
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

    fn appshot_rejection_reason(
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
    fn appshot_matches(&self, selector: &PhoneSessionSelector, session_id: &str) -> bool {
        self.appshot_rejection_reason(selector, session_id)
            .is_none()
    }

    async fn attach_destination_appshot(
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
    fn adoption_candidate_for(&self, serial: &str) -> Option<ScrcpyAdoptionCandidate> {
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
    fn has_managed_scrcpy_for_serial(&self, serial: &str) -> bool {
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

    /// Route a single phone request to the appropriate backend.
    ///
    /// This is the one entry point the daemon calls. The match arm set is 1:1
    /// with [`PhoneRequest`]; every arm returns the contractually-correct
    /// [`PhoneResponse`] variant.
    pub(crate) async fn handle(&mut self, request: PhoneRequest) -> PhoneResponse {
        self.drain_direct_events();
        self.reconcile_direct_sessions();
        if let Some(selector) = mutation_selector(&request)
            && let Some(session_id) = self.resolve_session_id(selector)
            && self.direct_identity(&session_id).is_some()
            && let Some(reason) = self.appshot_rejection_reason(selector, &session_id)
        {
            let fresh = self
                .observe(PhoneObserveRequest {
                    session: selector.clone(),
                    ..PhoneObserveRequest::default()
                })
                .await;
            if let Some(appshot) = fresh.appshot.clone() {
                self.appshots
                    .insert(appshot.appshot_id.clone(), (*appshot.clone()).clone());
                return PhoneResponse::AppShotRequired(Box::new(
                    sky_cua_platform::model::AppShotRequired {
                        code: "AppShotRequired".into(),
                        reason,
                        message: "capture a fresh phone AppShot and retry; no device mutation was performed".into(),
                        fresh_appshot: appshot,
                    },
                ));
            }
            return PhoneResponse::FeatureError(sky_cua_platform::model::PhoneFeatureError {
                code: "AppShotCaptureFailed".into(),
                message:
                    "a fresh phone AppShot could not be captured; no device mutation was performed"
                        .into(),
            });
        }
        if let Some(selector) = phone_request_activity_selector(&request) {
            self.touch_companion_overlay_activity(selector, now_ms())
                .await;
        }
        match request {
            PhoneRequest::Status(request) => {
                PhoneResponse::Status(self.status(request.refresh_devices).await)
            }
            PhoneRequest::SmsQuery(request) => {
                PhoneResponse::SmsQuery(self.sms_query(request).await)
            }
            _request if !self.selection.enabled => {
                PhoneResponse::Status(self.disabled_status().await)
            }
            PhoneRequest::ListDevices(request) => {
                PhoneResponse::Devices(self.list_devices(request.include_mdns).await)
            }
            PhoneRequest::PairWireless(request) => {
                PhoneResponse::PairedWireless(self.pair_wireless(&request).await)
            }
            PhoneRequest::Connect(request) => self.connect(request).await,
            PhoneRequest::Disconnect(request) => {
                PhoneResponse::Disconnected(self.disconnect(&request).await)
            }
            PhoneRequest::RefreshCapabilities(request) => {
                self.refresh_capabilities(&request.session).await
            }
            PhoneRequest::Observe(request) => PhoneResponse::Observe(self.observe(request).await),
            PhoneRequest::Screenshot(request) => self.screenshot(request).await,
            PhoneRequest::Tap(request) => PhoneResponse::Action(self.tap(request).await),
            PhoneRequest::Swipe(request) => PhoneResponse::Action(self.swipe(request).await),
            PhoneRequest::TypeText(request) => PhoneResponse::Action(self.type_text(request).await),
            PhoneRequest::PressKey(request) => PhoneResponse::Action(self.press_key(request).await),
            PhoneRequest::CompanionStatus(request) => {
                PhoneResponse::CompanionStatus(self.companion_status(&request.session).await)
            }
            PhoneRequest::InstallCompanion(request) => {
                PhoneResponse::Action(self.install_companion(request).await)
            }
            PhoneRequest::AccessibilityTree(request) => {
                PhoneResponse::AccessibilityTree(self.accessibility_tree(request).await)
            }
            PhoneRequest::Notifications(request) => {
                PhoneResponse::Notifications(self.notifications(request).await)
            }
            PhoneRequest::NotificationOpen(request) => {
                PhoneResponse::Notifications(self.notification_open(request).await)
            }
            PhoneRequest::NotificationDismiss(request) => {
                PhoneResponse::Notifications(self.notification_dismiss(request).await)
            }
            PhoneRequest::NotificationAction(request) => {
                PhoneResponse::Notifications(self.notification_action(request).await)
            }
            PhoneRequest::NotificationReply(request) => {
                PhoneResponse::Notifications(self.notification_reply(request).await)
            }
            PhoneRequest::AppCurrent(request) => {
                PhoneResponse::App(self.app_current(&request.session).await)
            }
            PhoneRequest::AppList(request) => PhoneResponse::App(self.app_list(request).await),
            PhoneRequest::AppLaunch(request) => {
                let selector = request.session.clone();
                let mut response = self.app_launch(request).await;
                self.attach_destination_appshot(&selector, &mut response)
                    .await;
                PhoneResponse::App(response)
            }
            PhoneRequest::AppOpenIntent(request) => {
                let selector = request.session.clone();
                let mut response = self.app_open_intent(request).await;
                self.attach_destination_appshot(&selector, &mut response)
                    .await;
                PhoneResponse::App(response)
            }
            PhoneRequest::AppForceStop(request) => {
                PhoneResponse::App(self.app_force_stop(request).await)
            }
            PhoneRequest::AppInstall(request) => {
                PhoneResponse::App(self.app_install(request).await)
            }
            PhoneRequest::OpenSettings(request) => {
                let selector = request.session.clone();
                let mut response = self.open_settings(request).await;
                self.attach_destination_appshot(&selector, &mut response)
                    .await;
                PhoneResponse::App(response)
            }
            PhoneRequest::Content(call) => self
                .phone_content(call)
                .await
                .map(PhoneResponse::Content)
                .unwrap_or_else(PhoneResponse::FeatureError),
            PhoneRequest::Clipboard(call) => self
                .phone_clipboard(call)
                .await
                .map(PhoneResponse::Clipboard)
                .unwrap_or_else(PhoneResponse::FeatureError),
            PhoneRequest::Editor(call) => self
                .phone_editor(call)
                .await
                .map(PhoneResponse::Editor)
                .unwrap_or_else(PhoneResponse::FeatureError),
            PhoneRequest::Camera(call) => self
                .phone_camera(call)
                .await
                .map(PhoneResponse::Camera)
                .unwrap_or_else(PhoneResponse::FeatureError),
            PhoneRequest::Storage(call) => self
                .phone_storage(call)
                .await
                .map(PhoneResponse::Storage)
                .unwrap_or_else(PhoneResponse::FeatureError),
        }
    }

    /// Execute the named, observation-only SMS lane. This deliberately does
    /// not call `resolve_session_id`: profile resolution owns device identity,
    /// and the only accepted transport is the authenticated direct provider.
    async fn sms_query(&self, request: PhoneSmsQueryRequest) -> PhoneSmsQueryResponse {
        let failure = |code: &str, message: String| -> PhoneSmsQueryResponse {
            schema_sms_failure(&request.profile, code, message)
        };

        if !self.selection.enabled {
            return failure("PHONE_DISABLED", "phone support is disabled".into());
        }

        if request.profile.trim().is_empty()
            || request.start_ms >= request.end_ms
            || !(1..=500).contains(&request.limit)
        {
            return failure(
                "INVALID_ARGUMENT",
                "profile must be non-empty, start_ms must be before end_ms, and limit must be 1..500".into(),
            );
        }
        let Some(profile) = self.selection.profiles.get(&request.profile) else {
            return failure(
                "PHONE_PROFILE_NOT_FOUND",
                format!("named phone profile {:?} was not found", request.profile),
            );
        };
        if profile.device_id.trim().is_empty()
            || profile.transport != "companion_direct"
            || profile.access != "observation_only"
            || !profile
                .required_capabilities
                .iter()
                .any(|capability| capability == "sms.read")
        {
            return failure(
                "PHONE_PROFILE_INVALID",
                format!(
                    "named phone profile {:?} is not a valid SMS observation profile",
                    request.profile
                ),
            );
        }
        let Some(provider) = self.direct_provider.as_ref() else {
            return failure(
                "DIRECT_DEVICE_REQUIRED",
                "SMS observation requires an authenticated CompanionDirect device".into(),
            );
        };
        let Some(snapshot) = provider.device(&profile.device_id) else {
            return failure(
                "DEVICE_OFFLINE",
                format!("CompanionDirect device {:?} is offline", profile.device_id),
            );
        };
        if !snapshot.capabilities.contains("sms.read") {
            return failure(
                "SMS_CAPABILITY_UNAVAILABLE",
                format!(
                    "CompanionDirect device {:?} does not advertise sms.read",
                    profile.device_id
                ),
            );
        }

        let params = serde_json::json!({
            "start_ms": request.start_ms,
            "end_ms": request.end_ms,
            "limit": request.limit,
            "cursor": request.cursor,
        });
        let result = provider
            .dispatch(
                &profile.device_id,
                snapshot.link_epoch,
                "sms.query",
                params,
                true,
                std::time::Duration::from_secs(30),
            )
            .await;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                return failure(
                    sms_direct_error_code(&error),
                    sms_direct_error_message(error),
                );
            }
        };
        let mut response: PhoneSmsQueryResponse = match serde_json::from_value(value) {
            Ok(response) => response,
            Err(error) => {
                return failure(
                    "PROTOCOL_ERROR",
                    format!("invalid sms.query response: {error}"),
                );
            }
        };
        // The host is the final whole-request boundary: even if a future
        // companion returns an accidental partial cursor/error combination,
        // do not surface it as a successful page.
        if response.error.is_some() {
            response.messages.clear();
            response.next_cursor = None;
            response.scan = None;
            response.schema = PHONE_SMS_QUERY_SCHEMA.to_owned();
            return response;
        }
        if !sms_page_contract_valid(&response, request.limit) {
            return failure(
                "PROTOCOL_ERROR",
                "sms.query returned an invalid or contradictory page".into(),
            );
        }
        response.schema = PHONE_SMS_QUERY_SCHEMA.to_owned();
        response.profile = request.profile;
        response.device_id = Some(profile.device_id.clone());
        response.transport = Some("companion_direct".to_owned());
        response.access = Some("observation_only".to_owned());
        response
    }

    fn reconcile_direct_sessions(&mut self) {
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
        for id in &stale {
            self.appshots
                .retain(|_, shot| shot.action_snapshot.session_id.as_deref() != Some(id));
            self.sessions.remove(id);
            self.profiles.remove(id);
        }
    }

    // ===================================================================
    // Host status / device listing
    // ===================================================================

    /// Host-tooling status routed through ADB, annotated with the resolved
    /// companion-enabled flag, default serial/backend, active sessions, and the
    /// resolved scrcpy path/version.
    async fn status(&self, refresh_devices: bool) -> PhoneStatusReport {
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

    async fn disabled_status(&self) -> PhoneStatusReport {
        let mut report = self.status(false).await;
        report.diagnostics.push(phone_disabled_diagnostic());
        report
    }

    /// Resolve scrcpy and, when present, probe its version, returning the idle
    /// capability shape for a freshly-detected profile. A missing binary yields a
    /// structured `missing` capability with a reason instead of bare absence, so
    /// the agent sees why the accelerator is unavailable.
    async fn detect_scrcpy_capabilities(&self) -> sky_cua_platform::model::PhoneScrcpyCapabilities {
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
    async fn list_devices(&self, include_mdns: bool) -> PhoneListDevicesResponse {
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
    fn mark_primary_targets(&self, devices: &mut [sky_cua_platform::model::PhoneDevice]) {
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

    fn populate_aliases(&self, devices: &mut [sky_cua_platform::model::PhoneDevice]) {
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
    async fn pair_wireless(&self, request: &PhonePairWirelessRequest) -> PhonePairWirelessResponse {
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

    // ===================================================================
    // Connect / disconnect / refresh
    // ===================================================================

    /// Create (or reuse) a session for the target serial.
    ///
    /// Resolves the serial, runs `adb connect` for wireless `host:port` targets,
    /// detects and caches a [`PhoneCapabilityProfile`], optionally bootstraps the
    /// companion (install/update + `adb forward` + token + capability probe) when
    /// companion support is enabled, assembles the available/unavailable action
    /// list, and registers the session. On a fatal resolution failure the honest
    /// host-status view is returned rather than a fabricated session.
    async fn connect(&mut self, request: PhoneConnectRequest) -> PhoneResponse {
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

    async fn connect_direct(&mut self, device_id: &str) -> PhoneResponse {
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
            companion,
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
        PhoneResponse::Connected(session)
    }

    /// Tear down a session, scoped to sky-cua-owned state only: drop the cached
    /// profile, snapshot/cursor state, and companion runtime, and for wireless
    /// targets run `adb disconnect` unless `keep_wireless` is set. Never touches
    /// scrcpy/adb processes the operator launched themselves.
    async fn disconnect(&mut self, request: &PhoneDisconnectRequest) -> PhoneDisconnectResponse {
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
    async fn refresh_capabilities(&mut self, selector: &PhoneSessionSelector) -> PhoneResponse {
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
    async fn rebuild_session(
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

    // ===================================================================
    // Profile cache
    // ===================================================================

    /// Look up a cached capability profile, marking freshness against the cache
    /// TTL. Backends call this before acting; a stale profile must reject backends
    /// no longer proven.
    ///
    /// The stored value keeps the refresh state it was cached with
    /// (`Detected`/`Refreshed`), so the connect/refresh request that detected or
    /// refreshed the profile reads back as such — those responses return the
    /// stored profile directly, not this per-request clone. Every later request
    /// resolves through here:
    /// - within TTL, a `Detected`/`Refreshed` cached state reports `Reused` on the
    ///   returned clone (this is the common case: an action after connect);
    /// - past TTL, the clone reports `Stale` (and `stale=true`).
    ///
    /// An already-`Stale`/`Reused` cached state is returned unchanged within TTL.
    fn cached_profile(&self, session_id: &str, now_ms: u64) -> Option<PhoneCapabilityProfile> {
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
    pub(super) fn mark_profile_stale_for_drift(
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

    pub(super) fn expected_capture_size(profile: &PhoneCapabilityProfile) -> Option<PixelSize> {
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
    fn insert_profile(&mut self, profile: PhoneCapabilityProfile, detected_at_ms: u64) {
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
    fn configured_adb_path(&self) -> Option<&str> {
        self.selection.adb_path.as_deref()
    }

    /// `adb connect` the configured wireless `default_serial` when
    /// `[phone] wireless_auto_connect` is enabled and the default is a wireless
    /// `host:port` target. A no-op when disabled (the default), when no default
    /// serial is configured, or when the default is a USB/emulator serial. The
    /// outcome is intentionally not surfaced here: a failed link resurfaces as the
    /// normal device-unavailable diagnostic once resolution/authorization runs.
    async fn wireless_auto_connect_default(&self) {
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
    fn operator_auto_install(&self) -> bool {
        self.selection.companion_operator_mode && self.selection.companion_auto_install
    }

    /// The default backend kind to advertise for new sessions and status.
    fn default_backend(&self) -> PhoneBackendKind {
        self.selection.default_backend_kind()
    }

    /// Resolve the backend reported on `phone_connect`. Explicit backend
    /// requests are treated as force requests: report the requested backend only
    /// if the connect path actually established the required runtime, otherwise
    /// report `None` with the diagnostics gathered by the failed setup.
    fn connect_session_backend(
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
    fn resolve_session_id(&self, selector: &PhoneSessionSelector) -> Option<String> {
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
    fn session_id_for_serial(&self, serial: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|(_, entry)| entry.session.serial == serial)
            .map(|(id, _)| id.clone())
    }

    /// The serial for a known session id, or an empty string when no session is
    /// registered under that id. Shared by the routing/apps/signals children so
    /// the open-coded `sessions.get(..).map(..serial.clone()).unwrap_or_default()`
    /// pattern lives in one place.
    pub(super) fn serial_of(&self, session_id: &str) -> String {
        self.sessions
            .get(session_id)
            .map(|entry| entry.session.serial.clone())
            .unwrap_or_default()
    }

    pub(super) fn direct_identity(&self, session_id: &str) -> Option<(String, u64)> {
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
    pub(super) fn action_context(&self, selector: &PhoneSessionSelector) -> Option<ActionContext> {
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
    pub(super) async fn fresh_action_context(
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
    async fn refresh_live_companion_session(&mut self, session_id: &str) -> bool {
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
    async fn resolve_target_serial(&self, requested: Option<&str>) -> Option<String> {
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
    async fn invalidate_on_observe_triggers(&mut self, session_id: &str, serial: &str) {
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
    async fn serial_is_authorized_device(&self, serial: &str) -> bool {
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
    fn backend_capabilities(&self, profile: &PhoneCapabilityProfile) -> PhoneBackendCapabilities {
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

/// Whether a `phone_connect` request asks for a managed scrcpy mirror: an
/// explicit `start_scrcpy` flag, or a request that selects the scrcpy backend.
fn request_wants_scrcpy(request: &PhoneConnectRequest) -> bool {
    request.start_scrcpy || request.backend == Some(PhoneBackendKind::Scrcpy)
}

fn is_uuid_format(value: &str) -> bool {
    // Strict 8-4-4-4-12 hex, case-insensitive. Avoids the loose
    // `len>=32 && contains('-')` that would mis-route an ADB serial that
    // happens to contain a hyphen.
    if value.len() != 36 {
        return false;
    }
    let bytes = value.as_bytes();
    const HYPHENS: [usize; 4] = [8, 13, 18, 23];
    for (i, b) in bytes.iter().enumerate() {
        if HYPHENS.contains(&i) {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Current wall-clock time in milliseconds since the Unix epoch.
pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Mint a session id from a serial: a sanitized serial plus the canonical
/// platform snapshot/uuid minter for uniqueness.
fn new_session_id(serial: &str) -> String {
    let sanitized: String = serial
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!(
        "phone-sess-{sanitized}-{}",
        sky_cua_platform::snapshot::new_snapshot_id()
    )
}

/// The default resolved selection used when config loading fails at startup.
fn default_selection() -> ResolvedPhoneSelection {
    sky_cua_platform::config::resolve_phone_selection(
        &sky_cua_platform::config::PhoneConfig::default(),
    )
}

/// Pull `(session_id, serial)` strings out of a selector, substituting empty
/// strings when the caller named neither.
pub(super) fn selector_ids(selector: &PhoneSessionSelector) -> (String, String) {
    (
        selector.session_id.clone().unwrap_or_default(),
        selector
            .serial
            .clone()
            .or_else(|| selector.device_id.clone())
            .unwrap_or_default(),
    )
}

/// Structured diagnostic for a tool that requires an active session when none
/// resolves from the selector.
pub(super) fn no_session_diagnostic(selector: &PhoneSessionSelector) -> DiagnosticEntry {
    let (session_id, serial) = selector_ids(selector);
    DiagnosticEntry {
        code: "PhoneNoSession".to_string(),
        message: format!(
            "no active phone session for selector (session_id={session_id:?}, serial={serial:?}); call phone_connect first"
        ),
        details: None,
    }
}

pub(super) fn phone_disabled_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneUseDisabled".to_string(),
        message: "phone-use is disabled by configuration; enable [phone].enabled before using device-control tools".to_string(),
        details: None,
    }
}

/// Structured diagnostic when a companion action is attempted with no live
/// companion runtime for the session.
pub(super) fn no_companion_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: super::protocol::error_codes::DISABLED_SERVICE.to_string(),
        message: "no reachable companion for this session".to_string(),
        details: None,
    }
}

fn companion_from_direct_health(health: Option<&serde_json::Value>) -> PhoneCompanionCapabilities {
    let string = |key| health?.get(key)?.as_str().map(str::to_owned);
    let boolean = |key| {
        health
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    let mut companion = PhoneCompanionCapabilities::absent(
        string("package")
            .as_deref()
            .unwrap_or("com.skycua.phonecompanion"),
    );
    companion.installed = true;
    companion.rpc_reachable = health.is_some();
    companion.installed_version = string("version");
    companion.accessibility_enabled = boolean("accessibility_enabled");
    companion.can_perform_gestures = boolean("can_perform_gestures");
    companion.can_retrieve_window_content = boolean("can_retrieve_window_content");
    companion.can_take_screenshot = boolean("can_take_screenshot");
    companion.notification_listener_enabled = boolean("notification_listener_enabled");
    companion.native_overlay = boolean("native_overlay");
    companion.native_overlay_pass_through = boolean("native_overlay_pass_through");
    companion.gesture_dispatch = companion.can_perform_gestures;
    companion.screenshot = companion.can_take_screenshot;
    companion.accessibility_tree = companion.can_retrieve_window_content;
    companion.notifications = companion.notification_listener_enabled;
    companion
}

fn schema_sms_failure(profile: &str, code: &str, message: String) -> PhoneSmsQueryResponse {
    PhoneSmsQueryResponse {
        schema: PHONE_SMS_QUERY_SCHEMA.to_owned(),
        profile: profile.to_owned(),
        device_id: None,
        transport: None,
        access: None,
        messages: Vec::new(),
        next_cursor: None,
        scan: None,
        error: Some(PhoneSmsQueryError {
            code: code.to_owned(),
            message,
        }),
    }
}

fn sms_page_contract_valid(response: &PhoneSmsQueryResponse, limit: u32) -> bool {
    let Some(scan) = response.scan.as_ref() else {
        return false;
    };
    !scan.snapshot
        && scan.has_more != scan.exhausted_as_observed
        && scan.has_more == response.next_cursor.is_some()
        && response.messages.len() <= limit as usize
        && response
            .next_cursor
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty())
}

fn sms_direct_error_code(error: &DirectRuntimeError) -> &'static str {
    match error {
        DirectRuntimeError::NotConnected
        | DirectRuntimeError::LinkEpochChanged { .. }
        | DirectRuntimeError::Disconnected => "DEVICE_OFFLINE",
        DirectRuntimeError::DeadlineExceeded => "DEADLINE_EXCEEDED",
        DirectRuntimeError::Protocol(_) => "PROTOCOL_ERROR",
        DirectRuntimeError::Remote { code, .. } => match code.as_str() {
            "SMS_PERMISSION_NOT_GRANTED" => "SMS_PERMISSION_NOT_GRANTED",
            "SMS_PERMISSION_RESTRICTED" => "SMS_PERMISSION_RESTRICTED",
            "SMS_PROVIDER_UNAVAILABLE" => "SMS_PROVIDER_UNAVAILABLE",
            "INVALID_ARGUMENT" => "INVALID_ARGUMENT",
            "INVALID_CURSOR" => "INVALID_CURSOR",
            "CURSOR_QUERY_MISMATCH" => "CURSOR_QUERY_MISMATCH",
            "SMS_QUERY_FAILED" => "SMS_QUERY_FAILED",
            "DEADLINE_EXCEEDED" => "DEADLINE_EXCEEDED",
            "PROTOCOL_ERROR" => "PROTOCOL_ERROR",
            _ => "SMS_QUERY_FAILED",
        },
    }
}

fn sms_direct_error_message(error: DirectRuntimeError) -> String {
    match error {
        DirectRuntimeError::Remote { message, .. } => message,
        other => format!("CompanionDirect SMS query failed: {other:?}"),
    }
}

fn phone_request_activity_selector(request: &PhoneRequest) -> Option<&PhoneSessionSelector> {
    match request {
        PhoneRequest::SmsQuery(_)
        | PhoneRequest::Status(_)
        | PhoneRequest::ListDevices(_)
        | PhoneRequest::PairWireless(_)
        | PhoneRequest::Connect(_)
        | PhoneRequest::Disconnect(_) => None,
        PhoneRequest::Observe(request) => Some(&request.session),
        PhoneRequest::RefreshCapabilities(request) => Some(&request.session),
        PhoneRequest::Screenshot(request) => Some(&request.session),
        PhoneRequest::Tap(request) => Some(&request.session),
        PhoneRequest::Swipe(request) => Some(&request.session),
        PhoneRequest::TypeText(request) => Some(&request.session),
        PhoneRequest::PressKey(request) => Some(&request.session),
        PhoneRequest::InstallCompanion(request) => Some(&request.session),
        PhoneRequest::CompanionStatus(request) => Some(&request.session),
        PhoneRequest::AccessibilityTree(request) => Some(&request.session),
        PhoneRequest::Notifications(request) => Some(&request.session),
        PhoneRequest::NotificationOpen(request) => Some(&request.session),
        PhoneRequest::NotificationDismiss(request) => Some(&request.session),
        PhoneRequest::NotificationAction(request) => Some(&request.session),
        PhoneRequest::NotificationReply(request) => Some(&request.session),
        PhoneRequest::AppCurrent(request) => Some(&request.session),
        PhoneRequest::AppList(request) => Some(&request.session),
        PhoneRequest::AppLaunch(request) => Some(&request.session),
        PhoneRequest::AppOpenIntent(request) => Some(&request.session),
        PhoneRequest::AppForceStop(request) => Some(&request.session),
        PhoneRequest::AppInstall(request) => Some(&request.session),
        PhoneRequest::OpenSettings(request) => Some(&request.session),
        PhoneRequest::Content(call) => Some(&call.session),
        PhoneRequest::Clipboard(call) => Some(&call.session),
        PhoneRequest::Editor(call) => Some(&call.session),
        PhoneRequest::Camera(call) => Some(&call.session),
        PhoneRequest::Storage(call) => Some(&call.session),
    }
}

fn mutation_selector(request: &PhoneRequest) -> Option<&PhoneSessionSelector> {
    match request {
        PhoneRequest::AppLaunch(_)
        | PhoneRequest::AppOpenIntent(_)
        | PhoneRequest::OpenSettings(_) => None,
        _ if request.is_idempotent() => None,
        _ => phone_request_activity_selector(request),
    }
}

/// Extend `ResolvedPhoneSelection` with the backend-kind parse the manager needs.
trait DefaultBackendKind {
    fn default_backend_kind(&self) -> PhoneBackendKind;
}

impl DefaultBackendKind for ResolvedPhoneSelection {
    fn default_backend_kind(&self) -> PhoneBackendKind {
        match self.default_backend.as_deref().map(str::trim) {
            Some("adb") => PhoneBackendKind::Adb,
            Some("companion") => PhoneBackendKind::Companion,
            Some("scrcpy") => PhoneBackendKind::Scrcpy,
            Some("auto") => PhoneBackendKind::Auto,
            // Unset/unknown: no backend is selected by default. Status reports
            // `None` until a session picks a concrete backend.
            _ => PhoneBackendKind::None,
        }
    }
}

#[cfg(test)]
mod test_support;
