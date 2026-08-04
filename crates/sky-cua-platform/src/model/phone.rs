//! Phone-use contract spine.
//!
//! This module mirrors the `browser` family at every layer: a tagged
//! `PhoneRequest`/`PhoneResponse` pair plus supporting structs/enums that model
//! Android device sessions, capability profiles, snapshots, cursor planes,
//! notifications, and app management.
//!
//! Backends (ADB, the Android companion, scrcpy) are implemented in later lanes
//! behind these types. The contract is intentionally explicit: clients must read
//! structured fields (capability profile id, the backend that handled an action,
//! cursor planes, mapping ids, available/unavailable actions, notification ids,
//! companion signing fields) rather than inferring runtime state from prose.
//!
//! Images are encoded the same way `BrowserScreenshotResponse` does:
//! `data_base64` + `mime_type` + optional `width`/`height`. There is no shared
//! `ImagePayload` type, so phone responses carry a `PhoneImage` payload with the
//! same field shape.

use serde::{Deserialize, Serialize};

use super::{
    AppShotEnvelope, AppShotRequired, DiagnosticEntry, PhoneConnectionIdentity, PixelSize, RectF,
};

/// Normalized caller lane for a Phone request entering through MCP.
///
/// This is same-user provenance for attribution and tab/device ownership, not
/// an authorization boundary. Callers outside the explicitly supported host
/// adapters use [`PhoneCallerProvenance::DirectMcp`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneCallerProvenance {
    CodexDesktop,
    #[serde(rename = "openclaw")]
    OpenClaw,
    #[serde(rename = "opencode")]
    OpenCode,
    DirectMcp,
}

/// MCP `initialize.clientInfo`, retained verbatim after non-empty validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneMcpClientInfo {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Logical caller identity propagated with a Phone service request.
///
/// Codex metadata is preserved exactly. Generic MCP callers receive a stable
/// session id for the initialized MCP process and a fresh turn id per
/// `tools/call`; `identity_synthetic` makes that distinction explicit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneRequestContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_provenance: Option<PhoneCallerProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_synthetic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_info: Option<PhoneMcpClientInfo>,
}

/// Tagged request envelope for every `phone_*` MCP tool. The variant set is 1:1
/// with the canonical tool list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhoneRequest {
    Observe(PhoneObserveRequest),
    Status(PhoneStatusRequest),
    ListDevices(PhoneListDevicesRequest),
    RefreshCapabilities(PhoneRefreshCapabilitiesRequest),
    PairWireless(PhonePairWirelessRequest),
    Connect(PhoneConnectRequest),
    Disconnect(PhoneDisconnectRequest),
    Screenshot(PhoneScreenshotRequest),
    Tap(PhoneTapRequest),
    Swipe(PhoneSwipeRequest),
    TypeText(PhoneTypeTextRequest),
    PressKey(PhonePressKeyRequest),
    InstallCompanion(PhoneInstallCompanionRequest),
    CompanionStatus(PhoneCompanionStatusRequest),
    AccessibilityTree(PhoneAccessibilityTreeRequest),
    Notifications(PhoneNotificationsRequest),
    NotificationOpen(PhoneNotificationOpenRequest),
    NotificationDismiss(PhoneNotificationDismissRequest),
    NotificationAction(PhoneNotificationActionRequest),
    NotificationReply(PhoneNotificationReplyRequest),
    AppCurrent(PhoneAppCurrentRequest),
    AppList(PhoneAppListRequest),
    AppLaunch(PhoneAppLaunchRequest),
    AppOpenIntent(PhoneAppOpenIntentRequest),
    AppForceStop(PhoneAppForceStopRequest),
    AppInstall(PhoneAppInstallRequest),
    OpenSettings(PhoneOpenSettingsRequest),
    Content(super::PhoneFeatureCall<super::PhoneContentRequest>),
    Clipboard(super::PhoneFeatureCall<super::PhoneClipboardRequest>),
    Editor(super::PhoneFeatureCall<super::PhoneEditorRequest>),
    Camera(super::PhoneFeatureCall<super::PhoneCameraRequest>),
    Storage(super::PhoneFeatureCall<super::PhoneStorageRequest>),
}

/// Tagged response envelope. Several request variants share a response variant
/// (every coordinate/text/key/app action returns `Action`/`App`), mirroring how
/// the browser family collapses click/type/scroll into `BrowserActionResponse`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
// boxing would churn the wire/model contract; revisit if size matters
#[allow(clippy::large_enum_variant)]
pub enum PhoneResponse {
    Observe(PhoneObserveResponse),
    Status(PhoneStatusReport),
    Devices(PhoneListDevicesResponse),
    Capabilities(PhoneCapabilityProfile),
    PairedWireless(PhonePairWirelessResponse),
    Connected(PhoneSession),
    Disconnected(PhoneDisconnectResponse),
    Screenshot(PhoneScreenshotResponse),
    Action(PhoneActionResponse),
    CompanionStatus(PhoneCompanionStatusResponse),
    AccessibilityTree(PhoneAccessibilityTreeResponse),
    Notifications(PhoneNotificationsResponse),
    App(PhoneAppResponse),
    AppShotRequired(Box<AppShotRequired>),
    Content(super::PhoneContentResponse),
    Clipboard(super::PhoneClipboardResponse),
    Editor(super::PhoneEditorResponse),
    Camera(super::PhoneCameraResponse),
    Storage(super::PhoneStorageResponse),
    FeatureError(super::PhoneFeatureError),
}

impl PhoneRequest {
    /// Whether this request converges to the same state on repetition and is
    /// therefore safe for the client to retry after an ambiguous failure.
    ///
    /// Reads/listings (`Observe`, `Status`, `ListDevices`, `Screenshot`,
    /// `CompanionStatus`, `AccessibilityTree`, `Notifications`, `AppCurrent`,
    /// `AppList`) are idempotent, as are `RefreshCapabilities` (a silent
    /// re-probe of cached capability state) and `Connect`/`Disconnect` (the
    /// manager reconciles to one session per serial rather than duplicating,
    /// and disconnecting an already-gone session is a documented no-op).
    /// `PairWireless` carries a one-time pairing code that is consumed on
    /// first use, so a blind retry cannot safely repeat it. Every gesture,
    /// text/key input, notification action, app-management action, and
    /// companion install compounds on repetition and is non-idempotent.
    #[must_use]
    pub fn is_idempotent(&self) -> bool {
        match self {
            Self::Observe(_)
            | Self::Status(_)
            | Self::ListDevices(_)
            | Self::RefreshCapabilities(_)
            | Self::Connect(_)
            | Self::Disconnect(_)
            | Self::Screenshot(_)
            | Self::CompanionStatus(_)
            | Self::AccessibilityTree(_)
            | Self::Notifications(_)
            | Self::AppCurrent(_)
            | Self::AppList(_) => true,
            Self::Content(call) => {
                matches!(call.request, super::PhoneContentRequest::Describe { .. })
            }
            Self::Clipboard(call) => matches!(
                call.request,
                super::PhoneClipboardRequest::Get | super::PhoneClipboardRequest::Changes { .. }
            ),
            Self::Editor(call) => matches!(call.request, super::PhoneEditorRequest::Context),
            Self::Camera(call) => matches!(
                call.request,
                super::PhoneCameraRequest::Enumerate
                    | super::PhoneCameraRequest::Capabilities { .. }
                    | super::PhoneCameraRequest::PreviewFrame { .. }
            ),
            Self::Storage(call) => matches!(
                call.request,
                super::PhoneStorageRequest::Roots
                    | super::PhoneStorageRequest::List { .. }
                    | super::PhoneStorageRequest::Stat { .. }
                    | super::PhoneStorageRequest::Read { .. }
                    | super::PhoneStorageRequest::Hash { .. }
                    | super::PhoneStorageRequest::Search { .. }
                    | super::PhoneStorageRequest::Thumbnail { .. }
                    | super::PhoneStorageRequest::Metadata { .. }
                    | super::PhoneStorageRequest::ListSafRoots
            ),
            Self::PairWireless(_)
            | Self::Tap(_)
            | Self::Swipe(_)
            | Self::TypeText(_)
            | Self::PressKey(_)
            | Self::InstallCompanion(_)
            | Self::NotificationOpen(_)
            | Self::NotificationDismiss(_)
            | Self::NotificationAction(_)
            | Self::NotificationReply(_)
            | Self::AppLaunch(_)
            | Self::AppOpenIntent(_)
            | Self::AppForceStop(_)
            | Self::AppInstall(_)
            | Self::OpenSettings(_) => false,
        }
    }
}

// ===========================================================================
// Image payload
// ===========================================================================

/// Inline image payload. Field shape matches `BrowserScreenshotResponse` so the
/// MCP client encodes phone screenshots exactly like browser screenshots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneImage {
    pub mime_type: String,
    pub data_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

// ===========================================================================
// Backend / connection / target enums
// ===========================================================================

/// How the host is transported to the device.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneConnectionKind {
    Usb,
    Emulator,
    /// Legacy `adb tcpip 5555` then `adb connect host:5555`.
    LegacyTcpip,
    /// Android 11+ wireless debugging via `adb pair`.
    WirelessDebugging,
    /// Phone-initiated `phone-control.v2` link; no ADB serial exists.
    CompanionDirect,
    Unknown,
}

/// The backend family that handled (or would handle) an operation. Every action
/// response states which backend actually serviced it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneBackendKind {
    /// Auto-routing: the service picks the best available backend.
    Auto,
    /// ADB baseline (required): shell screencap/input, install, forward.
    Adb,
    /// Android companion app: native gestures, accessibility, notifications.
    Companion,
    /// scrcpy mirror/control acceleration.
    Scrcpy,
    /// No backend could service the operation.
    None,
}

/// Coarse classification of the connected device for compatibility lanes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneTargetDeviceKind {
    GalaxyS26Ultra,
    RedmiTablet,
    Emulator,
    UnknownAndroid,
}

/// Lifecycle of the cached capability profile relative to the latest request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneCapabilityRefreshState {
    /// Detected fresh during this request.
    Detected,
    /// Reused an unexpired cached profile.
    Reused,
    /// Opportunistically refreshed an expired profile.
    Refreshed,
    /// Reused, but the cache TTL has elapsed and availability is not re-proven.
    Stale,
}

// ===========================================================================
// Capability profile, companion, scrcpy capabilities
// ===========================================================================

/// Per-session companion app capability and identity report. Identity fields
/// (`package_name`, versions, cert/apk hashes) let `phone_connect` decide
/// install/update/refuse before any backend RPC happens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCompanionCapabilities {
    pub installed: bool,
    pub package_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_cert_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_cert_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_sha256: Option<String>,
    pub signature_matches_expected: bool,
    pub allow_downgrade: bool,
    pub auto_install_attempted: bool,
    pub rpc_reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_token_expires_at_ms: Option<u64>,
    pub accessibility_enabled: bool,
    pub can_perform_gestures: bool,
    pub can_retrieve_window_content: bool,
    pub can_take_screenshot: bool,
    pub notification_listener_enabled: bool,
    pub native_overlay: bool,
    pub native_overlay_pass_through: bool,
    pub gesture_dispatch: bool,
    pub screenshot: bool,
    pub accessibility_tree: bool,
    pub notifications: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privileged_setup: Option<String>,
}

impl PhoneCompanionCapabilities {
    /// A companion that is not installed and exposes nothing. Identity fields
    /// still carry the expected package/cert so callers can reason about
    /// install/update before the APK exists.
    #[must_use]
    pub fn absent(package_name: impl Into<String>) -> Self {
        Self {
            installed: false,
            package_name: package_name.into(),
            installed_version: None,
            expected_version: None,
            installed_cert_sha256: None,
            expected_cert_sha256: None,
            apk_sha256: None,
            signature_matches_expected: false,
            allow_downgrade: false,
            auto_install_attempted: false,
            rpc_reachable: false,
            rpc_token_expires_at_ms: None,
            accessibility_enabled: false,
            can_perform_gestures: false,
            can_retrieve_window_content: false,
            can_take_screenshot: false,
            notification_listener_enabled: false,
            native_overlay: false,
            native_overlay_pass_through: false,
            gesture_dispatch: false,
            screenshot: false,
            accessibility_tree: false,
            notifications: false,
            privileged_setup: None,
        }
    }
}

/// scrcpy acceleration capability for this session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneScrcpyCapabilities {
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub active: bool,
    pub host_window_mapped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PhoneScrcpyCapabilities {
    /// scrcpy not installed or not in use.
    #[must_use]
    pub fn absent() -> Self {
        Self {
            installed: false,
            version: None,
            active: false,
            host_window_mapped: false,
            window_title: None,
            video_codec: None,
            reason: None,
        }
    }
}

/// Backend availability summary for a session, distinct from the full
/// capability profile. This is the quick "what can this session do" view that
/// rides on `PhoneSession`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneBackendCapabilities {
    pub adb: bool,
    pub companion: bool,
    pub scrcpy: bool,
    pub screenshot: bool,
    pub gestures: bool,
    pub text_input: bool,
    pub key_input: bool,
    pub accessibility_tree: bool,
    pub notifications: bool,
    pub app_management: bool,
    pub host_visible_overlay: bool,
    pub screenshot_synthetic_cursor: bool,
    pub phone_native_overlay: bool,
}

/// Structured, cached description of what a device/session can do right now.
/// Detected during `phone_connect`, invalidated on reconnect, companion
/// install/update, permission/orientation/display change, RPC failure, wireless
/// disconnect, and explicit `phone_refresh_capabilities`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCapabilityProfile {
    pub profile_id: String,
    pub session_id: String,
    pub serial: String,
    pub detected_at_ms: u64,
    pub stale: bool,
    pub refresh_state: PhoneCapabilityRefreshState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub target_device_kind: PhoneTargetDeviceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hyperos_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_sdk: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_release: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_size: Option<PixelSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density_dpi: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    /// The device's live screen rotation as an exact quarter turn
    /// (0/90/180/270), read from the `dumpsys` rotation probe. `orientation` is
    /// the coarse portrait/landscape label for humans; this carries the precise
    /// quarter the host content-rect math needs so 180/270 are not collapsed
    /// back into the label's two states. `None` when no live rotation was
    /// probed, in which case consumers fall back to the label-derived quarter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_rotation_degrees: Option<i32>,
    pub connection_kind: PhoneConnectionKind,
    pub companion: PhoneCompanionCapabilities,
    pub scrcpy: PhoneScrcpyCapabilities,
    pub root_available: bool,
    pub shizuku_available: bool,
    pub device_owner: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_actions: Vec<PhoneAvailableAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_actions: Vec<PhoneUnavailableAction>,
    /// Provider-specific truth for each operation. `available_actions` and
    /// `unavailable_actions` remain the compact agent-facing projection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<super::PhoneCapabilityRoute>,
}

// ===========================================================================
// Action affordances
// ===========================================================================

/// An action the agent can take right now, with the backend that would service
/// it. The `action` string is the canonical tool name (e.g. `phone_tap`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAvailableAction {
    pub action: String,
    pub backend: PhoneBackendKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// An action that is not currently possible, with a structured reason so the
/// agent understands why (disabled permission, missing companion, wrong API
/// level, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneUnavailableAction {
    pub action: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

// ===========================================================================
// Session, cursor, coordinate mapping
// ===========================================================================

/// A live phone session. Created by `phone_connect`, referenced by every
/// follow-up tool through `session_id`/`serial`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneSession {
    pub session_id: String,
    /// Present for ADB-backed compatibility sessions. Direct sessions omit it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
    /// Typed transport identity. New callers should use this instead of
    /// inferring transport or identity from `serial`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<PhoneConnectionIdentity>,
    pub connection_kind: PhoneConnectionKind,
    pub backend: PhoneBackendKind,
    pub capabilities: PhoneBackendCapabilities,
    pub capability_profile: PhoneCapabilityProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion: Option<PhoneCompanionCapabilities>,
    pub managed_process: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    pub created_at_ms: u64,
}

/// Which cursor planes are live for a session/snapshot. Mirrors the three-plane
/// design: host-visible overlay, screenshot-synthetic marker, phone-native
/// accessibility overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCursorCapabilities {
    pub host_visible_overlay: bool,
    pub screenshot_synthetic_cursor: bool,
    pub phone_native_overlay: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_overlay_reason: Option<String>,
}

/// Cursor position state after an action, in device coordinates plus the
/// snapshot it was captured against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCursorState {
    pub visible: bool,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_point: Option<PhonePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_point: Option<PhonePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action: Option<String>,
    pub updated_at_ms: u64,
}

/// A 2D point used for cursor/tap coordinates. The plane (device, screenshot, or
/// host) is implied by the field that holds it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PhonePoint {
    pub x: f64,
    pub y: f64,
}

/// Data to translate between device pixels, screenshot pixels, and host desktop
/// pixels for a captured snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCoordinateMapping {
    pub mapping_id: String,
    pub session_id: String,
    pub serial: String,
    pub device_rect: RectF,
    pub screenshot_rect: RectF,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_window_rect: Option<RectF>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_content_rect: Option<RectF>,
    pub rotation_degrees: i32,
    pub captured_at_ms: u64,
}

// ===========================================================================
// Device listing / status
// ===========================================================================

/// State of a device as ADB reports it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneDeviceState {
    Device,
    Unauthorized,
    Offline,
    NoPermissions,
    Connecting,
    Bootloader,
    Recovery,
    Unknown,
}

/// One device as seen by `phone_list_devices`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneDevice {
    /// Present for ADB-discovered devices. Direct devices omit it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
    /// Stable Companion identity for a direct device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Current authenticated link epoch for a direct device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_epoch: Option<u64>,
    /// Explicit transport identity; avoids synthetic ADB serials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<PhoneConnectionIdentity>,
    pub state: PhoneDeviceState,
    pub connection_kind: PhoneConnectionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_id: Option<String>,
    /// Whether this device's `model` matches one of the operator's configured
    /// `[phone] primary_target_models`. Set by the host device-list path (not the
    /// ADB wire parse); primaries are surfaced first in the listing. Defaults to
    /// `false` and is omitted from the wire when unset.
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary: bool,
}

/// Response for `phone_list_devices`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneListDevicesResponse {
    pub devices: Vec<PhoneDevice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// Response for `phone_status`: host tooling readiness plus active sessions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneStatusReport {
    pub enabled: bool,
    pub adb_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adb_server_running: Option<bool>,
    pub scrcpy_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrcpy_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrcpy_version: Option<String>,
    pub companion_enabled: bool,
    pub mdns_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_serial: Option<String>,
    pub default_backend: PhoneBackendKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<PhoneSession>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<PhoneDevice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

// ===========================================================================
// Accessibility / notifications / apps
// ===========================================================================

/// Bounded accessibility summary embedded in `phone_observe`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAccessibilitySummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    pub node_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headline_texts: Vec<String>,
    pub truncated: bool,
    pub redacted: bool,
}

/// One accessibility tree node. A flat parent-indexed list mirrors the desktop
/// `ElementNode` shape without recreating that desktop-specific type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneAccessibilityNode {
    pub node_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<RectF>,
    pub clickable: bool,
    pub focusable: bool,
    pub enabled: bool,
    pub redacted: bool,
}

/// Redaction state of a notification's content.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneNotificationRedaction {
    None,
    Partial,
    Full,
}

/// One notification action button exposed for `phone_notification_action`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationAction {
    pub action_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub supports_inline_reply: bool,
}

/// One notification event. `event_id` is the stable handle required by all
/// notification action tools; an action must reference a fresh observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationEvent {
    pub event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub package_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub redaction: PhoneNotificationRedaction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    pub ongoing: bool,
    pub can_open: bool,
    pub can_dismiss: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PhoneNotificationAction>,
    pub posted_at_ms: u64,
}

/// Foreground or launchable app description.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAppInfo {
    pub package_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<u64>,
    pub launchable: bool,
    pub system_app: bool,
}

// ===========================================================================
// Requests
// ===========================================================================

/// Common session selector. Tools accept either a `session_id` (preferred) or a
/// raw `serial`; the service resolves either to an active session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneSessionSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Canonical AppShot required before a state-changing phone operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneObserveRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<PhoneBackendKind>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub include_image_data: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_accessibility: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_notifications: bool,
}

impl Default for PhoneObserveRequest {
    fn default() -> Self {
        Self {
            session: PhoneSessionSelector::default(),
            backend: None,
            include_image_data: true,
            include_accessibility: false,
            include_notifications: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneStatusRequest {
    #[serde(default, skip_serializing_if = "is_false")]
    pub refresh_devices: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneListDevicesRequest {
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_mdns: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneRefreshCapabilitiesRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhonePairWirelessRequest {
    /// `host:port` of the pairing endpoint shown on the device.
    pub host_port: String,
    /// One-time pairing code. Never logged, stored, or echoed in responses.
    pub pairing_code: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneConnectRequest {
    /// USB serial, emulator serial, or `host:port` wireless target. Unset means
    /// the configured default or the single connected device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Stable Companion device id. Mutually exclusive with `serial`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<PhoneBackendKind>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub install_companion: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub start_scrcpy: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneDisconnectRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_wireless: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneScreenshotRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<PhoneBackendKind>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub include_image_data: bool,
}

impl Default for PhoneScreenshotRequest {
    fn default() -> Self {
        Self {
            session: PhoneSessionSelector::default(),
            backend: None,
            include_image_data: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneTapRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    /// Snapshot the coordinates were read against. Required unless
    /// `use_device_coordinates` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_snapshot_id: Option<String>,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_device_coordinates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneSwipeRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_snapshot_id: Option<String>,
    pub start_x: f64,
    pub start_y: f64,
    pub end_x: f64,
    pub end_y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_device_coordinates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneTypeTextRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhonePressKeyRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    /// Android keycode name or number (e.g. `KEYCODE_BACK`, `4`, `home`).
    pub key: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneInstallCompanionRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "is_false")]
    pub force_reinstall: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_downgrade: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCompanionStatusRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAccessibilityTreeRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationsRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationOpenRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationDismissRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationActionRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub event_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationReplyRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub event_id: String,
    pub action_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAppCurrentRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAppListRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_system: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAppLaunchRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub package_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAppOpenIntentRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    /// Activity component, deep link, or intent URI to launch.
    pub intent_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAppForceStopRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub package_name: String,
}

/// Install strategy for `phone_app_install`. Single APK is the default;
/// split/multi-package paths are modeled explicitly per the requirements matrix.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PhoneAppInstallMode {
    #[default]
    Single,
    /// `adb install-multiple` for split APKs of one package.
    Multiple,
    /// `adb install-multi-package` for several packages at once.
    MultiPackage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneAppInstallRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    /// Host-side APK path(s). Single mode uses the first entry.
    pub apk_paths: Vec<String>,
    #[serde(default)]
    pub mode: PhoneAppInstallMode,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reinstall: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_downgrade: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_test_apk: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub grant_runtime_permissions: bool,
}

/// Which Android settings screen `phone_open_settings` should open.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneSettingsScreen {
    Accessibility,
    NotificationAccess,
    OverlayPermission,
    AppDetails,
    WirelessDebugging,
    BatteryOptimization,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneOpenSettingsRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub screen: PhoneSettingsScreen,
    /// Target package when the screen is app-scoped (e.g. `AppDetails`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
}

// ===========================================================================
// Responses
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneObserveResponse {
    pub session: PhoneSession,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appshot: Option<Box<AppShotEnvelope>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_image: Option<PhoneImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_app: Option<PhoneAppInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_summary: Option<PhoneAccessibilitySummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_notifications: Vec<PhoneNotificationEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<PhoneCursorState>,
    pub backend: PhoneBackendKind,
    pub capability_profile_id: String,
    /// Whether the profile that drove this observation was freshly detected,
    /// reused from cache, opportunistically refreshed, or stale. Mirrors the
    /// freshness gate carried by [`PhoneActionResponse`].
    pub profile_refresh_state: PhoneCapabilityRefreshState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_actions: Vec<PhoneAvailableAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_actions: Vec<PhoneUnavailableAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneScreenshotResponse {
    pub session_id: String,
    pub serial: String,
    pub phone_snapshot_id: String,
    pub backend: PhoneBackendKind,
    pub capability_profile_id: String,
    /// Freshness of the profile in force when this capture was taken. The
    /// returned snapshot feeds later coordinate actions, so the disposition
    /// (detected/reused/refreshed/stale) travels with it.
    pub profile_refresh_state: PhoneCapabilityRefreshState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_image: Option<PhoneImage>,
    pub device_size: PixelSize,
    pub coordinate_mapping: PhoneCoordinateMapping,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<PhoneCursorState>,
    pub cursor_capabilities: PhoneCursorCapabilities,
    pub capture_contains_native_overlay: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// Result of a coordinate/text/key action. `backend` states who actually
/// serviced it; `capability_profile_id` records which profile was in force.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneActionResponse {
    pub session_id: String,
    pub serial: String,
    pub action: String,
    pub backend: PhoneBackendKind,
    pub capability_profile_id: String,
    pub profile_refresh_state: PhoneCapabilityRefreshState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<PhoneCursorState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhonePairWirelessResponse {
    pub paired: bool,
    pub host_port: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneDisconnectResponse {
    pub session_id: String,
    pub serial: String,
    pub disconnected: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCompanionStatusResponse {
    pub session_id: String,
    pub serial: String,
    pub companion: PhoneCompanionCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneAccessibilityTreeResponse {
    pub session_id: String,
    pub serial: String,
    pub backend: PhoneBackendKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<PhoneAccessibilityNode>,
    pub truncated: bool,
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneNotificationsResponse {
    pub session_id: String,
    pub serial: String,
    pub backend: PhoneBackendKind,
    pub listener_enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<PhoneNotificationEvent>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// What an app-management tool did. `kind` records which app tool produced this
/// (`current`, `list`, `launch`, `open_intent`, `force_stop`, `install`,
/// `open_settings`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneAppResponseKind {
    Current,
    List,
    Launch,
    OpenIntent,
    ForceStop,
    Install,
    OpenSettings,
}

/// Which ADB install path actually serviced a `phone_app_install` request, so
/// the caller can tell a single-APK update from a split or multi-package
/// install instead of inferring it from the request it sent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneInstallStrategy {
    /// `adb install` of a single APK.
    Single,
    /// `adb install-multiple` of split APKs for one package.
    Multiple,
    /// `adb install-multi-package` of several packages at once.
    MultiPackage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneAppResponse {
    pub session_id: String,
    pub serial: String,
    pub kind: PhoneAppResponseKind,
    pub backend: PhoneBackendKind,
    pub success: bool,
    /// Destination AppShot for source-free launch/open-intent/settings flows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_appshot: Option<Box<AppShotEnvelope>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_app: Option<PhoneAppInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<PhoneAppInfo>,
    pub truncated: bool,
    /// For `kind = install` on success, which ADB install path ran. `None` for
    /// non-install responses or when the strategy is not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_strategy: Option<PhoneInstallStrategy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

// ===========================================================================
// serde helpers
// ===========================================================================

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests;
