#![allow(clippy::empty_line_after_doc_comments)]
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
//! - this file: state, dispatch table, and small primes;
//! - [`host`][]: status, device listing, direct reconciliation, and Appshot helpers;
//! - [`lifecycle`][]: `connect`/`connect_direct`/`disconnect`/`refresh`/`rebuild`;
//! - [`selection`][]: session selection, profile cache, and backend capabilities;
//! - [`helpers`][]: pure helpers (IDs, diagnostics, direct-health mapping);
//! - [`routing`][]: backend selection plus the coordinate/text/key/screenshot/
//!   observe execution paths;
//! - [`apps`][]: app management, notifications, accessibility, companion status,
//!   and settings.

mod apps;
mod appshot;
mod capture;
mod capture_screenshot;
mod companion_lane;
mod companion_probe;
mod features;
pub(crate) mod helpers;
pub(crate) mod host;
pub(crate) mod lifecycle;
mod routing;
mod routing_backend;
mod scrcpy_lane;
pub(crate) mod selection;
mod signals;

// Re-export helpers so existing `super::now_ms` etc continue to resolve.
pub(crate) use helpers::{
    default_selection, mutation_selector, no_companion_diagnostic, no_session_diagnostic, now_ms,
    phone_request_activity_selector, schema_sms_failure, selector_ids, sms_direct_error_code,
    sms_direct_error_message, sms_page_contract_valid,
};

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
use std::collections::HashMap;
use std::sync::Arc;

use sky_cua_platform::config::{ResolvedPhoneSelection, resolved_phone_selection};
use sky_cua_platform::model::{
    DiagnosticEntry, PHONE_SMS_QUERY_SCHEMA, PhoneCapabilityProfile, PhoneObserveRequest,
    PhoneRequest, PhoneResponse, PhoneSession, PhoneSmsQueryRequest, PhoneSmsQueryResponse,
};

use super::command::{CommandRunner, RealCommandRunner};
use super::cursor::PhoneCursorTracker;
use super::direct::DirectRuntimeHandle;
use super::direct::provider::CompanionDirectProvider;
use super::protocol;
use super::protocol::client::CompanionClient;
use super::scrcpy;
use super::snapshot::PhoneSnapshotRegistry;

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
pub(crate) struct ActionContext {
    pub(crate) session_id: String,
    pub(crate) serial: String,
    pub(crate) profile: PhoneCapabilityProfile,
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

    /// Route a single phone request to the appropriate backend.
    ///
    /// This is the one entry point the daemon calls. The match arm set is 1:1
    /// with [`PhoneRequest`]; every arm returns the contractually-correct
    /// [`PhoneResponse`] variant.
    pub(crate) async fn handle(&mut self, request: PhoneRequest) -> PhoneResponse {
        self.drain_direct_events();
        self.reconcile_direct_sessions();
        // NodeAction carries its AppShot id at the top level (`request.appshot_id`)
        // plus the flattened `session.appshot_id`; the generic `mutation_selector`
        // check would shadow the top-level one and always return `Missing`, so
        // handle it separately and honour the `view_id` fallback.
        if let PhoneRequest::NodeAction(req) = &request {
            if let Some(session_id) = self.resolve_session_id(&req.session)
                && self.direct_identity(&session_id).is_some()
                && let Some(reason) = self.node_action_appshot_rejection_reason(req, &session_id)
            {
                let fresh = self
                    .observe(PhoneObserveRequest {
                        session: req.session.clone(),
                        ..PhoneObserveRequest::default()
                    })
                    .await;
                if let Some(appshot) = fresh.appshot.clone() {
                    self.appshots
                        .insert(appshot.appshot_id.clone(), (*appshot).clone());
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
        }
        if let Some(selector) = mutation_selector(&request)
            && !matches!(request, PhoneRequest::NodeAction(_))
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
                    .insert(appshot.appshot_id.clone(), (*appshot).clone());
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
            PhoneRequest::LongPress(request) => {
                PhoneResponse::Action(self.long_press(request).await)
            }
            PhoneRequest::DoubleTap(request) => {
                PhoneResponse::Action(self.double_tap(request).await)
            }
            PhoneRequest::NodeAction(request) => {
                PhoneResponse::Action(self.node_action(request).await)
            }
            PhoneRequest::GlobalAction(request) => {
                PhoneResponse::Action(self.global_action(request).await)
            }
            PhoneRequest::KeyEvent(request) => PhoneResponse::Action(self.key_event(request).await),
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
}

#[cfg(test)]
mod test_support;
