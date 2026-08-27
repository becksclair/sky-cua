use serde::{Deserialize, Serialize};

use crate::model::{AppShotEnvelope, DiagnosticEntry};

use super::capabilities::PhoneBackendKind;
use super::requests::PhoneSessionSelector;
use super::session::PhoneAppInfo;

fn is_false(value: &bool) -> bool {
    !*value
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
