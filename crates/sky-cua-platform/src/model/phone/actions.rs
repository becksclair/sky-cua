use serde::{Deserialize, Serialize};

use super::requests::PhoneSessionSelector;

// Helpers local to this module (mirrors parent helpers).
fn is_false(value: &bool) -> bool {
    !*value
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneLongPressRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_snapshot_id: Option<String>,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_device_coordinates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneDoubleTapRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_snapshot_id: Option<String>,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_device_coordinates: bool,
}

/// Semantic accessibility node action. Mirrors
/// `AccessibilityNodeInfo.ACTION_*` plus `AccessibilityAction` extensions.
/// Wire values are `snake_case` of the constant sans `ACTION_` prefix.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneNodeAction {
    Click,
    LongClick,
    ContextClick,
    Dismiss,
    Expand,
    Collapse,
    ScrollForward,
    ScrollBackward,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    PageUp,
    PageDown,
    PageLeft,
    PageRight,
    ScrollToPosition,
    Focus,
    ClearFocus,
    AccessibilityFocus,
    ClearAccessibilityFocus,
    Select,
    ClearSelection,
    ShowOnScreen,
    SetProgress,
    SetText,
    SetSelection,
    Copy,
    Cut,
    Paste,
    NextAtMovementGranularity,
    PreviousAtMovementGranularity,
    NextHtmlElement,
    PreviousHtmlElement,
    PressAndHold,
    ImeEnter,
    MoveWindow,
    ShowTooltip,
    HideTooltip,
}

/// Optional bundle arguments for node actions that require them. Only the
/// fields relevant to the chosen `action` are read; others are ignored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneNodeActionArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_start: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_end: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement_granularity: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extend_selection: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html_element: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub press_and_hold_duration_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_amount: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneNodeActionRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    /// AppShot id that produced the node. Required unless `view_id` fallback is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appshot_id: Option<String>,
    /// Stable per-capture node id from `interactiveWindowSnapshots`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<i64>,
    /// Fallback resource id for playground harness (`viewIdResourceName`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    pub action: PhoneNodeAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<PhoneNodeActionArgs>,
}

/// Global accessibility action. Mirrors `AccessibilityService.GLOBAL_ACTION_*`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneGlobalAction {
    Back,
    Home,
    Recents,
    Notifications,
    QuickSettings,
    PowerDialog,
    ToggleSplitScreen,
    LockScreen,
    TakeScreenshot,
    KeycodeHeadsetHook,
    AccessibilityButton,
    AccessibilityButtonChooser,
    AccessibilityShortcut,
    AccessibilityAllApps,
    DismissNotificationShade,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    DpadCenter,
    Menu,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneGlobalActionRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    pub action: PhoneGlobalAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneKeyEventRequest {
    #[serde(default, flatten)]
    pub session: PhoneSessionSelector,
    /// Android keycode name (`KEYCODE_VOLUME_UP`) or integer string (`24`).
    pub key_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_state: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_count: Option<u32>,
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
