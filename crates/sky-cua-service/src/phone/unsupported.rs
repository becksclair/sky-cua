//! Phone module fallback for non-Unix platforms.
//!
//! Mirrors `src/browser/unsupported.rs`: phone-use depends on `adb`/companion/
//! scrcpy process control and (later) a Unix domain RPC to the companion, so on
//! non-Unix hosts every request is answered with a structured "unsupported on
//! this platform" diagnostic. The public surface (`PhoneManager::new` plus
//! `handle`) matches the Unix module so the daemon dispatch is identical.

use sky_cua_platform::model::{
    DiagnosticEntry, DisplayInfo, PhoneAccessibilityTreeResponse, PhoneActionResponse,
    PhoneAppResponse, PhoneAppResponseKind, PhoneBackendKind, PhoneCapabilityRefreshState,
    PhoneCompanionCapabilities, PhoneCompanionStatusResponse, PhoneDisconnectResponse,
    PhoneListDevicesResponse, PhoneNotificationsResponse, PhonePairWirelessResponse, PhoneRequest,
    PhoneResponse, PhoneSession, PhoneSessionSelector, PhoneStatusReport, PixelSize, RectF,
    WindowInfo,
};

/// Platform-unsupported phone runtime. Holds no state; every request returns an
/// honest unsupported response.
pub(crate) struct PhoneManager;

/// Non-Unix stand-in for the Unix manager's adoption candidate. The daemon only
/// moves `Option<ScrcpyAdoptionCandidate>` values through the manager seam (prime
/// in, prime out); on this platform no window is ever discovered, so the fields
/// are never read. They mirror the Unix shape so the daemon dispatch is identical;
/// the `expect(dead_code)` keeps the non-Unix build clean while preserving that
/// shape.
#[derive(Clone)]
#[expect(dead_code)]
pub(crate) struct ScrcpyAdoptionCandidate {
    pub(crate) serial: String,
    pub(crate) pid: Option<u32>,
    pub(crate) window_title: String,
}

/// Non-Unix stand-in for the Unix manager's window-mapping target. The mapping
/// accessors below always return `None` here (there is no managed scrcpy mirror
/// without Unix process control), so this is never constructed; it exists only so
/// the daemon's mapping path type-checks against the same return type. Its
/// fields are statically read by the daemon's mapping seam (behind a `let Some`
/// that is always `None` here), so it is not dead and needs no `expect`.
pub(crate) struct ScrcpyWindowTarget {
    pub(crate) session_id: String,
    pub(crate) pid: Option<u32>,
    pub(crate) window_title: String,
    pub(crate) device_size: PixelSize,
    pub(crate) rotation_degrees: i32,
}

impl PhoneManager {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn current_time_ms() -> u64 {
        0
    }

    pub(crate) async fn expire_idle_companion_overlays(&mut self, _now_ms: u64) -> Vec<String> {
        Vec::new()
    }

    pub(crate) async fn handle(&mut self, request: PhoneRequest) -> PhoneResponse {
        match request {
            PhoneRequest::ListDevices(_) => PhoneResponse::Devices(devices()),
            PhoneRequest::PairWireless(request) => {
                PhoneResponse::PairedWireless(PhonePairWirelessResponse {
                    paired: false,
                    host_port: request.host_port,
                    serial: None,
                    diagnostics: vec![unsupported_diagnostic()],
                })
            }
            PhoneRequest::Disconnect(request) => {
                PhoneResponse::Disconnected(disconnect(&request.session))
            }
            PhoneRequest::CompanionStatus(request) => {
                PhoneResponse::CompanionStatus(companion_status(&request.session))
            }
            PhoneRequest::AccessibilityTree(request) => {
                PhoneResponse::AccessibilityTree(accessibility(&request.session))
            }
            PhoneRequest::Notifications(request) => {
                PhoneResponse::Notifications(notifications(&request.session))
            }
            PhoneRequest::NotificationOpen(request) => {
                PhoneResponse::Notifications(notifications(&request.session))
            }
            PhoneRequest::NotificationDismiss(request) => {
                PhoneResponse::Notifications(notifications(&request.session))
            }
            PhoneRequest::NotificationAction(request) => {
                PhoneResponse::Notifications(notifications(&request.session))
            }
            PhoneRequest::NotificationReply(request) => {
                PhoneResponse::Notifications(notifications(&request.session))
            }
            PhoneRequest::AppCurrent(request) => {
                PhoneResponse::App(app(&request.session, PhoneAppResponseKind::Current))
            }
            PhoneRequest::AppList(request) => {
                PhoneResponse::App(app(&request.session, PhoneAppResponseKind::List))
            }
            PhoneRequest::AppLaunch(request) => {
                PhoneResponse::App(app(&request.session, PhoneAppResponseKind::Launch))
            }
            PhoneRequest::AppOpenIntent(request) => {
                PhoneResponse::App(app(&request.session, PhoneAppResponseKind::OpenIntent))
            }
            PhoneRequest::AppForceStop(request) => {
                PhoneResponse::App(app(&request.session, PhoneAppResponseKind::ForceStop))
            }
            PhoneRequest::AppInstall(request) => {
                PhoneResponse::App(app(&request.session, PhoneAppResponseKind::Install))
            }
            PhoneRequest::OpenSettings(request) => {
                PhoneResponse::App(app(&request.session, PhoneAppResponseKind::OpenSettings))
            }
            PhoneRequest::Tap(request) => {
                PhoneResponse::Action(action(&request.session, "phone_tap"))
            }
            PhoneRequest::Swipe(request) => {
                PhoneResponse::Action(action(&request.session, "phone_swipe"))
            }
            PhoneRequest::TypeText(request) => {
                PhoneResponse::Action(action(&request.session, "phone_type_text"))
            }
            PhoneRequest::PressKey(request) => {
                PhoneResponse::Action(action(&request.session, "phone_press_key"))
            }
            PhoneRequest::InstallCompanion(request) => {
                PhoneResponse::Action(action(&request.session, "phone_install_companion"))
            }
            PhoneRequest::Status(_)
            | PhoneRequest::Connect(_)
            | PhoneRequest::Observe(_)
            | PhoneRequest::Screenshot(_)
            | PhoneRequest::RefreshCapabilities(_) => PhoneResponse::Status(status()),
        }
    }

    // ===================================================================
    // Daemon-mediated scrcpy host-window surface (no-ops on this platform)
    //
    // The daemon calls these to prime mirror sizing/adoption and to discover
    // and map the managed scrcpy desktop window. Without Unix process control
    // there is never a managed mirror, so every accessor reports "nothing to
    // do" honestly: primers drop their input, queries return `None`/empty, and
    // mapping returns `false`. The signatures mirror the Unix manager so the
    // daemon dispatch is identical across platforms.
    // ===================================================================

    pub(crate) fn set_scrcpy_host_size_default(&mut self, _max_size: Option<u32>) {}

    pub(crate) fn set_scrcpy_adoption_candidate(
        &mut self,
        _candidate: Option<ScrcpyAdoptionCandidate>,
    ) {
    }

    pub(crate) fn find_adoptable_scrcpy_window(
        &self,
        _serial: &str,
        _windows: &[WindowInfo],
    ) -> Option<ScrcpyAdoptionCandidate> {
        None
    }

    pub(crate) fn scrcpy_window_to_map(&self) -> Option<ScrcpyWindowTarget> {
        None
    }

    pub(crate) fn scrcpy_window_to_remap(&self) -> Option<ScrcpyWindowTarget> {
        None
    }

    pub(crate) fn set_scrcpy_window_mapping(
        &mut self,
        _session_id: &str,
        _host_window: &RectF,
        _device_size: PixelSize,
        _rotation_degrees: i32,
    ) -> bool {
        false
    }

    pub(crate) fn clear_scrcpy_window_mapping(&mut self, _session_id: &str) -> bool {
        false
    }

    pub(crate) fn mark_scrcpy_mapping_exhausted(&mut self, _session_id: &str) {}

    pub(crate) fn session_view(&self, _session_id: &str) -> Option<PhoneSession> {
        None
    }

    pub(crate) fn poll_scrcpy_liveness(&mut self) -> Vec<(String, bool)> {
        Vec::new()
    }

    #[cfg(test)]
    pub(crate) fn with_fake_runner_for_tests() -> Self {
        Self
    }
}

/// Non-Unix stand-in for the Unix host-display-aware mirror sizing helper. With
/// no managed scrcpy mirror to size, this always returns `None`, leaving any
/// configured `[phone] max_size` to stand (there is none to apply anyway). The
/// daemon imports this through `crate::phone`, matching the Unix re-export.
pub(crate) fn host_scrcpy_default_max_size(_displays: &[DisplayInfo]) -> Option<u32> {
    None
}

fn unsupported_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneUnsupportedPlatform".to_string(),
        message: "phone-use requires a Unix host for adb/companion/scrcpy process control"
            .to_string(),
        details: None,
    }
}

fn selector_ids(selector: &PhoneSessionSelector) -> (String, String) {
    (
        selector.session_id.clone().unwrap_or_default(),
        selector.serial.clone().unwrap_or_default(),
    )
}

fn status() -> PhoneStatusReport {
    PhoneStatusReport {
        enabled: false,
        adb_available: false,
        adb_path: None,
        adb_version: None,
        adb_server_running: None,
        scrcpy_available: false,
        scrcpy_path: None,
        scrcpy_version: None,
        companion_enabled: false,
        mdns_available: false,
        default_serial: None,
        default_backend: PhoneBackendKind::None,
        sessions: Vec::new(),
        devices: Vec::new(),
        diagnostics: vec![unsupported_diagnostic()],
    }
}

fn devices() -> PhoneListDevicesResponse {
    PhoneListDevicesResponse {
        devices: Vec::new(),
        adb_path: None,
        adb_version: None,
        diagnostics: vec![unsupported_diagnostic()],
    }
}

fn companion_status(selector: &PhoneSessionSelector) -> PhoneCompanionStatusResponse {
    let (session_id, serial) = selector_ids(selector);
    PhoneCompanionStatusResponse {
        session_id,
        serial,
        companion: PhoneCompanionCapabilities::absent("com.skycua.phonecompanion"),
        diagnostics: vec![unsupported_diagnostic()],
    }
}

fn accessibility(selector: &PhoneSessionSelector) -> PhoneAccessibilityTreeResponse {
    let (session_id, serial) = selector_ids(selector);
    PhoneAccessibilityTreeResponse {
        session_id,
        serial,
        backend: PhoneBackendKind::None,
        package_name: None,
        activity: None,
        nodes: Vec::new(),
        truncated: false,
        redacted: false,
        diagnostics: vec![unsupported_diagnostic()],
    }
}

fn notifications(selector: &PhoneSessionSelector) -> PhoneNotificationsResponse {
    let (session_id, serial) = selector_ids(selector);
    PhoneNotificationsResponse {
        session_id,
        serial,
        backend: PhoneBackendKind::None,
        listener_enabled: false,
        events: Vec::new(),
        truncated: false,
        diagnostics: vec![unsupported_diagnostic()],
    }
}

fn app(selector: &PhoneSessionSelector, kind: PhoneAppResponseKind) -> PhoneAppResponse {
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
        diagnostics: vec![unsupported_diagnostic()],
    }
}

fn action(selector: &PhoneSessionSelector, action: &str) -> PhoneActionResponse {
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
        diagnostics: vec![unsupported_diagnostic()],
    }
}

fn disconnect(selector: &PhoneSessionSelector) -> PhoneDisconnectResponse {
    let (session_id, serial) = selector_ids(selector);
    PhoneDisconnectResponse {
        session_id,
        serial,
        disconnected: false,
        diagnostics: vec![unsupported_diagnostic()],
    }
}
