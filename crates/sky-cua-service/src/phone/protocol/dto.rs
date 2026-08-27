use serde::{Deserialize, Serialize};

// ===========================================================================
// health / capabilities
// ===========================================================================

/// `health` result. `capabilities` returns a superset (see [`CapabilitiesResult`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct HealthResult {
    pub(crate) version: String,
    pub(crate) version_code: u64,
    pub(crate) package: String,
    pub(crate) accessibility_enabled: bool,
    pub(crate) can_perform_gestures: bool,
    pub(crate) can_retrieve_window_content: bool,
    pub(crate) can_take_screenshot: bool,
    pub(crate) notification_listener_enabled: bool,
    pub(crate) native_overlay: bool,
    pub(crate) native_overlay_pass_through: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) privileged_setup: Option<String>,
}

/// `capabilities` result: the health fields plus screenshot/gesture support
/// detail. Flattening `health` keeps the wire object a single flat map while the
/// Rust type composes the shared fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CapabilitiesResult {
    #[serde(flatten)]
    pub(crate) health: HealthResult,
    pub(crate) screenshot_api_level: u32,
    pub(crate) screenshot_supported: bool,
    pub(crate) gesture_supported: bool,
}

// ===========================================================================
// accessibility_tree
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AccessibilityTreeParams {
    pub(crate) max_nodes: u32,
}

/// One node in the companion's flat accessibility list. `bounds` is the raw
/// `[left, top, right, bottom]` device-pixel rect the companion reports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AccessibilityNodeDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) content_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) bounds: Option<[i32; 4]>,
    pub(crate) focusable: bool,
    pub(crate) enabled: bool,
    pub(crate) clickable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AccessibilityTreeResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) activity: Option<String>,
    #[serde(default)]
    pub(crate) nodes: Vec<AccessibilityNodeDto>,
    pub(crate) truncated: bool,
    pub(crate) redacted: bool,
}

// ===========================================================================
// screenshot
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ScreenshotParams {
    pub(crate) include_overlay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ScreenshotResult {
    pub(crate) mime_type: String,
    pub(crate) data_base64: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) contains_native_overlay: bool,
}

// ===========================================================================
// gesture
// ===========================================================================

/// Gesture kind. `tap` uses one point; `swipe` uses two (start, end).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GestureKind {
    Tap,
    Swipe,
}

/// A device-pixel point in a gesture path.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub(crate) struct GesturePoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct GestureParams {
    pub(crate) kind: GestureKind,
    pub(crate) points: Vec<GesturePoint>,
    pub(crate) duration_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GestureResult {
    pub(crate) dispatched: bool,
}

// ===========================================================================
// node_action / global_action / key_event
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct NodeActionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) appshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) view_id: Option<String>,
    pub(crate) action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) args: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NodeActionResult {
    pub(crate) dispatched: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) success: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GlobalActionParams {
    pub(crate) action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GlobalActionResult {
    pub(crate) dispatched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct KeyEventParams {
    pub(crate) key_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) meta_state: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) repeat_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct KeyEventResult {
    pub(crate) dispatched: bool,
}

// ===========================================================================
// cursor_overlay
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct CursorOverlayParams {
    pub(crate) visible: bool,
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CursorOverlayResult {
    pub(crate) shown: bool,
    pub(crate) pass_through: bool,
}

// ===========================================================================
// overlay_active
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OverlayActiveParams {
    pub(crate) active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OverlayActiveResult {
    pub(crate) active: bool,
    pub(crate) glow_supported: bool,
}

// ===========================================================================
// overlay_gesture
// ===========================================================================

/// Reuses [`GesturePoint`] for the device-pixel path. `kind` is the free-form
/// wire string (`tap`/`swipe`/`drag`) rather than [`GestureKind`], because the
/// visual overlay supports `drag`, which the real-input `gesture` method does
/// not.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct OverlayGestureParams {
    pub(crate) kind: String,
    pub(crate) points: Vec<GesturePoint>,
    pub(crate) duration_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OverlayGestureResult {
    pub(crate) animated: bool,
}

// ===========================================================================
// notifications / notification_op
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationsParams {
    pub(crate) max: u32,
}

/// Redaction state of a companion notification event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationRedactionDto {
    None,
    Partial,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationActionDto {
    pub(crate) action_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    pub(crate) is_reply: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationEventDto {
    pub(crate) event_id: String,
    pub(crate) package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) body: Option<String>,
    pub(crate) redaction: NotificationRedactionDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ranking: Option<u32>,
    /// Whether the notification carries a content-intent the agent can open.
    /// Absent on older companions; the host defaults it conservatively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) can_open: Option<bool>,
    /// Whether the notification is user-dismissable (`StatusBarNotification.isClearable`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) can_dismiss: Option<bool>,
    /// Whether the notification is an ongoing/non-clearable event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ongoing: Option<bool>,
    pub(crate) when_ms: u64,
    #[serde(default)]
    pub(crate) actions: Vec<NotificationActionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationsResult {
    pub(crate) listener_enabled: bool,
    #[serde(default)]
    pub(crate) events: Vec<NotificationEventDto>,
    pub(crate) truncated: bool,
}

/// `notification_op` operation kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationOp {
    Open,
    Dismiss,
    Action,
    Reply,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationOpParams {
    pub(crate) event_id: String,
    pub(crate) op: NotificationOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reply_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NotificationOpResult {
    pub(crate) ok: bool,
}

// ===========================================================================
// current_app / app_list / app_op
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CurrentAppResult {
    pub(crate) package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppListParams {
    pub(crate) launchable_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppListEntryDto {
    pub(crate) package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    pub(crate) launchable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppListResult {
    #[serde(default)]
    pub(crate) apps: Vec<AppListEntryDto>,
    pub(crate) truncated: bool,
}

/// `app_op` operation kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppOp {
    Launch,
    OpenIntent,
    ForceStop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppOpParams {
    pub(crate) op: AppOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) intent_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppOpResult {
    pub(crate) ok: bool,
}
