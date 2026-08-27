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

#[allow(unused_imports)]
use super::{
    AppShotEnvelope, AppShotRequired, DiagnosticEntry, PhoneConnectionIdentity, PixelSize, RectF,
};

pub mod actions;
pub mod app;
pub mod capabilities;
pub mod requests;
pub mod responses;
pub mod session;
pub mod sms;

// Re-exports to preserve `use sky_cua_platform::model::phone::{...}` surface.
pub use self::actions::{
    PhoneDoubleTapRequest, PhoneGlobalAction, PhoneGlobalActionRequest, PhoneKeyEventRequest,
    PhoneLongPressRequest, PhoneNodeAction, PhoneNodeActionArgs, PhoneNodeActionRequest,
    PhonePressKeyRequest, PhoneSwipeRequest, PhoneTapRequest, PhoneTypeTextRequest,
};
pub use self::app::{
    PhoneAppCurrentRequest, PhoneAppForceStopRequest, PhoneAppInstallMode, PhoneAppInstallRequest,
    PhoneAppLaunchRequest, PhoneAppListRequest, PhoneAppOpenIntentRequest, PhoneAppResponse,
    PhoneAppResponseKind, PhoneInstallStrategy, PhoneOpenSettingsRequest, PhoneSettingsScreen,
};
pub use self::capabilities::{
    PhoneAvailableAction, PhoneBackendCapabilities, PhoneBackendKind, PhoneCapabilityProfile,
    PhoneCapabilityRefreshState, PhoneCompanionCapabilities, PhoneConnectionKind,
    PhoneScrcpyCapabilities, PhoneTargetDeviceKind, PhoneUnavailableAction,
};
pub use self::requests::{
    PhoneAccessibilityTreeRequest, PhoneCompanionStatusRequest, PhoneConnectRequest,
    PhoneDisconnectRequest, PhoneInstallCompanionRequest, PhoneListDevicesRequest,
    PhoneNotificationActionRequest, PhoneNotificationDismissRequest, PhoneNotificationOpenRequest,
    PhoneNotificationReplyRequest, PhoneNotificationsRequest, PhoneObserveRequest,
    PhonePairWirelessRequest, PhoneRefreshCapabilitiesRequest, PhoneScreenshotRequest,
    PhoneSessionSelector, PhoneStatusRequest,
};
pub use self::responses::{
    PhoneAccessibilityTreeResponse, PhoneActionResponse, PhoneCompanionStatusResponse,
    PhoneDisconnectResponse, PhoneNotificationsResponse, PhoneObserveResponse,
    PhonePairWirelessResponse, PhoneScreenshotResponse,
};
pub use self::session::{
    PhoneAccessibilityNode, PhoneAccessibilitySummary, PhoneAppInfo, PhoneCoordinateMapping,
    PhoneCursorCapabilities, PhoneCursorState, PhoneDevice, PhoneDeviceState, PhoneImage,
    PhoneListDevicesResponse, PhoneNotificationAction, PhoneNotificationEvent,
    PhoneNotificationRedaction, PhonePoint, PhoneSession, PhoneStatusReport,
};
pub use self::sms::{
    PHONE_SMS_QUERY_SCHEMA, PhoneSmsQueryError, PhoneSmsQueryRequest, PhoneSmsQueryResponse,
    PhoneSmsRecord, PhoneSmsScan,
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
    /// Observation-only SMS query. This is intentionally profile-addressed,
    /// not session/serial-addressed, and is routed only through
    /// CompanionDirect.
    SmsQuery(PhoneSmsQueryRequest),
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
    LongPress(PhoneLongPressRequest),
    DoubleTap(PhoneDoubleTapRequest),
    TypeText(PhoneTypeTextRequest),
    PressKey(PhonePressKeyRequest),
    NodeAction(PhoneNodeActionRequest),
    GlobalAction(PhoneGlobalActionRequest),
    KeyEvent(PhoneKeyEventRequest),
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
    SmsQuery(PhoneSmsQueryResponse),
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
            | Self::AppList(_)
            | Self::SmsQuery(_) => true,
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
            | Self::LongPress(_)
            | Self::DoubleTap(_)
            | Self::TypeText(_)
            | Self::PressKey(_)
            | Self::NodeAction(_)
            | Self::GlobalAction(_)
            | Self::KeyEvent(_)
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

#[cfg(test)]
mod tests;
