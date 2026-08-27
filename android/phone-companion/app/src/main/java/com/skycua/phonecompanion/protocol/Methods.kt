package com.skycua.phonecompanion.protocol

import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject

/**
 * Typed parameter parsing and result building for each RPC method, plus the
 * validation rules the wire contract requires. Validation failures raise
 * [MethodParamException] carrying a structured error code so the dispatcher can
 * surface them without leaking prose-routed errors.
 */

/** Raised when method parameters fail validation. */
class MethodParamException(
    val code: String,
    override val message: String,
) : Exception(message)

private fun bad(message: String): Nothing =
    throw MethodParamException(Protocol.ErrorCodes.BAD_REQUEST, message)

// ---------------------------------------------------------------------------
// accessibility_tree
// ---------------------------------------------------------------------------

data class AccessibilityTreeParams(val maxNodes: Int) {
    companion object {
        const val DEFAULT_MAX_NODES = 250
        const val HARD_CAP = 5000

        fun parse(params: JsonValue.Obj): AccessibilityTreeParams {
            val requested = params.int("max_nodes") ?: DEFAULT_MAX_NODES
            if (requested <= 0) bad("max_nodes must be positive")
            return AccessibilityTreeParams(minOf(requested, HARD_CAP))
        }
    }
}

/** A single accessibility node in the bounded snapshot. */
data class AccessibilityNode(
    val className: String?,
    val text: String?,
    val contentDesc: String?,
    val bounds: IntArray,
    val focusable: Boolean,
    val enabled: Boolean,
    val clickable: Boolean,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            putOpt("class", className)
            putOpt("text", text)
            putOpt("content_desc", contentDesc)
            put(
                "bounds",
                jsonArray(bounds.map { JsonValue.of(it.toLong()) }),
            )
            put("focusable", focusable)
            put("enabled", enabled)
            put("clickable", clickable)
        }
}

data class AccessibilityTreeResult(
    val packageName: String?,
    val activity: String?,
    val nodes: List<AccessibilityNode>,
    val truncated: Boolean,
    val redacted: Boolean,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            putOpt("package", packageName)
            putOpt("activity", activity)
            put("nodes", jsonArray(nodes.map { it.toJson() }))
            put("truncated", truncated)
            put("redacted", redacted)
        }
}

// ---------------------------------------------------------------------------
// screenshot
// ---------------------------------------------------------------------------

data class ScreenshotParams(val includeOverlay: Boolean) {
    companion object {
        fun parse(params: JsonValue.Obj): ScreenshotParams =
            ScreenshotParams(includeOverlay = params.bool("include_overlay") ?: false)
    }
}

/** Parameters for the bounded universal phone AppShot capture. */
data class AppShotParams(val maxNodes: Int, val deadlineMs: Long) {
    companion object {
        const val DEFAULT_MAX_NODES = 5000
        const val HARD_MAX_NODES = 5000
        const val DEFAULT_DEADLINE_MS = 2000L
        fun parse(params: JsonValue.Obj): AppShotParams {
            val nodes = (params.int("max_nodes") ?: DEFAULT_MAX_NODES).coerceIn(1, HARD_MAX_NODES)
            val deadline = (params.long("deadline_ms") ?: DEFAULT_DEADLINE_MS).coerceIn(1L, DEFAULT_DEADLINE_MS)
            return AppShotParams(nodes, deadline)
        }
    }
}

data class ScreenshotResult(
    val mimeType: String,
    val dataBase64: String,
    val width: Int,
    val height: Int,
    val containsNativeOverlay: Boolean,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            put("mime_type", mimeType)
            put("data_base64", dataBase64)
            put("width", width)
            put("height", height)
            put("contains_native_overlay", containsNativeOverlay)
        }
}

// ---------------------------------------------------------------------------
// gesture
// ---------------------------------------------------------------------------

enum class GestureKind(val wire: String) {
    TAP("tap"),
    SWIPE("swipe"),
}

data class GesturePoint(val x: Int, val y: Int)

data class GestureParams(
    val kind: GestureKind,
    val points: List<GesturePoint>,
    val durationMs: Long,
) {
    companion object {
        const val DEFAULT_DURATION_MS = 50L
        const val MAX_DURATION_MS = 60_000L

        fun parse(params: JsonValue.Obj): GestureParams {
            val kindStr = params.string("kind") ?: bad("missing gesture kind")
            val kind =
                when (kindStr) {
                    GestureKind.TAP.wire -> GestureKind.TAP
                    GestureKind.SWIPE.wire -> GestureKind.SWIPE
                    else -> bad("unknown gesture kind '$kindStr'")
                }

            val pointsArr = params.arr("points") ?: bad("missing gesture points")
            val points =
                pointsArr.items.map { item ->
                    val obj = item as? JsonValue.Obj ?: bad("gesture point must be an object")
                    val x = obj.int("x") ?: bad("gesture point missing x")
                    val y = obj.int("y") ?: bad("gesture point missing y")
                    if (x < 0 || y < 0) bad("gesture coordinates must be non-negative")
                    GesturePoint(x, y)
                }

            when (kind) {
                GestureKind.TAP ->
                    if (points.size != 1) bad("tap requires exactly one point")
                GestureKind.SWIPE ->
                    if (points.size != 2) bad("swipe requires exactly two points")
            }

            val duration = params.long("duration_ms") ?: DEFAULT_DURATION_MS
            if (duration <= 0) bad("duration_ms must be positive")
            if (duration > MAX_DURATION_MS) bad("duration_ms exceeds maximum")

            return GestureParams(kind, points, duration)
        }
    }
}

fun gestureDispatchedResult(): JsonValue.Obj =
    jsonObject { put("dispatched", true) }

// ---------------------------------------------------------------------------
// node_action
// ---------------------------------------------------------------------------

enum class NodeActionKind(val wire: String) {
    CLICK("click"),
    LONG_CLICK("long_click"),
    CONTEXT_CLICK("context_click"),
    DISMISS("dismiss"),
    EXPAND("expand"),
    COLLAPSE("collapse"),
    SCROLL_FORWARD("scroll_forward"),
    SCROLL_BACKWARD("scroll_backward"),
    SCROLL_UP("scroll_up"),
    SCROLL_DOWN("scroll_down"),
    SCROLL_LEFT("scroll_left"),
    SCROLL_RIGHT("scroll_right"),
    PAGE_UP("page_up"),
    PAGE_DOWN("page_down"),
    PAGE_LEFT("page_left"),
    PAGE_RIGHT("page_right"),
    SCROLL_TO_POSITION("scroll_to_position"),
    FOCUS("focus"),
    CLEAR_FOCUS("clear_focus"),
    ACCESSIBILITY_FOCUS("accessibility_focus"),
    CLEAR_ACCESSIBILITY_FOCUS("clear_accessibility_focus"),
    SELECT("select"),
    CLEAR_SELECTION("clear_selection"),
    SHOW_ON_SCREEN("show_on_screen"),
    SET_PROGRESS("set_progress"),
    SET_TEXT("set_text"),
    SET_SELECTION("set_selection"),
    COPY("copy"),
    CUT("cut"),
    PASTE("paste"),
    NEXT_AT_MOVEMENT_GRANULARITY("next_at_movement_granularity"),
    PREVIOUS_AT_MOVEMENT_GRANULARITY("previous_at_movement_granularity"),
    NEXT_HTML_ELEMENT("next_html_element"),
    PREVIOUS_HTML_ELEMENT("previous_html_element"),
    PRESS_AND_HOLD("press_and_hold"),
    IME_ENTER("ime_enter"),
    MOVE_WINDOW("move_window"),
    SHOW_TOOLTIP("show_tooltip"),
    HIDE_TOOLTIP("hide_tooltip"),
}

data class NodeActionParams(
    val appshotId: String?,
    val nodeId: Long?,
    val viewId: String?,
    val action: NodeActionKind,
    val args: JsonValue.Obj?,
) {
    companion object {
        fun parse(params: JsonValue.Obj): NodeActionParams {
            val actionStr = params.string("action") ?: bad("missing node action")
            val action = NodeActionKind.entries.firstOrNull { it.wire == actionStr }
                ?: bad("unknown node action '$actionStr'")
            val appshotId = params.string("appshot_id")
            val nodeId = params.long("node_id")
            val viewId = params.string("view_id")
            if (nodeId == null && viewId.isNullOrEmpty()) {
                bad("node_action requires node_id or view_id")
            }
            val args = params.obj("args")
            return NodeActionParams(appshotId, nodeId, viewId, action, args)
        }
    }
}

fun nodeActionResult(dispatched: Boolean, success: Boolean? = null): JsonValue.Obj =
    jsonObject {
        put("dispatched", dispatched)
        if (success != null) put("success", success)
    }

// ---------------------------------------------------------------------------
// global_action
// ---------------------------------------------------------------------------

enum class GlobalActionKind(val wire: String) {
    BACK("back"),
    HOME("home"),
    RECENTS("recents"),
    NOTIFICATIONS("notifications"),
    QUICK_SETTINGS("quick_settings"),
    POWER_DIALOG("power_dialog"),
    TOGGLE_SPLIT_SCREEN("toggle_split_screen"),
    LOCK_SCREEN("lock_screen"),
    TAKE_SCREENSHOT("take_screenshot"),
    KEYCODE_HEADSETHOOK("keycode_headset_hook"),
    ACCESSIBILITY_BUTTON("accessibility_button"),
    ACCESSIBILITY_BUTTON_CHOOSER("accessibility_button_chooser"),
    ACCESSIBILITY_SHORTCUT("accessibility_shortcut"),
    ACCESSIBILITY_ALL_APPS("accessibility_all_apps"),
    DISMISS_NOTIFICATION_SHADE("dismiss_notification_shade"),
    DPAD_UP("dpad_up"),
    DPAD_DOWN("dpad_down"),
    DPAD_LEFT("dpad_left"),
    DPAD_RIGHT("dpad_right"),
    DPAD_CENTER("dpad_center"),
    MENU("menu"),
}

data class GlobalActionParams(val action: GlobalActionKind) {
    companion object {
        fun parse(params: JsonValue.Obj): GlobalActionParams {
            val actionStr = params.string("action") ?: bad("missing global action")
            val action = GlobalActionKind.entries.firstOrNull { it.wire == actionStr }
                ?: bad("unknown global action '$actionStr'")
            return GlobalActionParams(action)
        }
    }
}

fun globalActionResult(dispatched: Boolean): JsonValue.Obj =
    jsonObject { put("dispatched", dispatched) }

// ---------------------------------------------------------------------------
// key_event
// ---------------------------------------------------------------------------

data class KeyEventParams(
    val keyCode: String,
    val metaState: Int?,
    val repeatCount: Int?,
) {
    companion object {
        fun parse(params: JsonValue.Obj): KeyEventParams {
            val keyCode = params.string("key_code") ?: bad("missing key_code")
            if (keyCode.isEmpty()) bad("key_code must not be empty")
            val metaState = params.int("meta_state")
            val repeatCount = params.int("repeat_count")
            if (repeatCount != null && repeatCount < 0) bad("repeat_count must be non-negative")
            return KeyEventParams(keyCode, metaState, repeatCount)
        }
    }
}

fun keyEventResult(dispatched: Boolean): JsonValue.Obj =
    jsonObject { put("dispatched", dispatched) }

// ---------------------------------------------------------------------------
// cursor_overlay
// ---------------------------------------------------------------------------

data class CursorOverlayParams(
    val visible: Boolean,
    val x: Int,
    val y: Int,
) {
    companion object {
        fun parse(params: JsonValue.Obj): CursorOverlayParams {
            val visible = params.bool("visible") ?: bad("missing visible flag")
            val x = params.int("x") ?: 0
            val y = params.int("y") ?: 0
            if (visible && (x < 0 || y < 0)) bad("cursor coordinates must be non-negative")
            return CursorOverlayParams(visible, x, y)
        }
    }
}

fun cursorOverlayResult(shown: Boolean, passThrough: Boolean): JsonValue.Obj =
    jsonObject {
        put("shown", shown)
        put("pass_through", passThrough)
    }

// ---------------------------------------------------------------------------
// overlay_active
// ---------------------------------------------------------------------------

data class OverlayActiveParams(val active: Boolean) {
    companion object {
        fun parse(params: JsonValue.Obj): OverlayActiveParams {
            val active = params.bool("active") ?: bad("missing active flag")
            return OverlayActiveParams(active)
        }
    }
}

fun overlayActiveResult(active: Boolean, glowSupported: Boolean): JsonValue.Obj =
    jsonObject {
        put("active", active)
        put("glow_supported", glowSupported)
    }

// ---------------------------------------------------------------------------
// overlay_gesture
// ---------------------------------------------------------------------------

/**
 * Visual-only cursor animation for one action. Reuses [GesturePoint] for the
 * device-pixel path. `kind` is parsed as a free-form wire string against
 * {tap, swipe, drag} rather than [GestureKind], because the overlay supports
 * `drag`, which the real-input gesture method does not.
 */
data class OverlayGestureParams(
    val kind: String,
    val points: List<GesturePoint>,
    val durationMs: Long,
) {
    companion object {
        const val KIND_TAP = "tap"
        const val KIND_SWIPE = "swipe"
        const val KIND_DRAG = "drag"
        const val DEFAULT_DURATION_MS = 200L
        const val MAX_DURATION_MS = 60_000L

        fun parse(params: JsonValue.Obj): OverlayGestureParams {
            val kind = params.string("kind") ?: bad("missing overlay gesture kind")
            if (kind != KIND_TAP && kind != KIND_SWIPE && kind != KIND_DRAG) {
                bad("unknown overlay gesture kind '$kind'")
            }

            val pointsArr = params.arr("points") ?: bad("missing overlay gesture points")
            val points =
                pointsArr.items.map { item ->
                    val obj = item as? JsonValue.Obj ?: bad("overlay gesture point must be an object")
                    val x = obj.int("x") ?: bad("overlay gesture point missing x")
                    val y = obj.int("y") ?: bad("overlay gesture point missing y")
                    if (x < 0 || y < 0) bad("overlay gesture coordinates must be non-negative")
                    GesturePoint(x, y)
                }

            when (kind) {
                KIND_TAP ->
                    if (points.isEmpty()) bad("tap requires at least one point")
                KIND_SWIPE, KIND_DRAG ->
                    if (points.size < 2) bad("$kind requires at least two points")
            }

            val duration = params.long("duration_ms") ?: DEFAULT_DURATION_MS
            if (duration <= 0) bad("duration_ms must be positive")
            if (duration > MAX_DURATION_MS) bad("duration_ms exceeds maximum")

            return OverlayGestureParams(kind, points, duration)
        }
    }
}

fun overlayGestureResult(animated: Boolean): JsonValue.Obj =
    jsonObject { put("animated", animated) }

// ---------------------------------------------------------------------------
// notifications
// ---------------------------------------------------------------------------

enum class Redaction(val wire: String) {
    NONE("none"),
    PARTIAL("partial"),
    FULL("full"),
}

data class NotificationAction(
    val actionId: String,
    val title: String,
    val isReply: Boolean,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            put("action_id", actionId)
            put("title", title)
            put("is_reply", isReply)
        }
}

data class NotificationEvent(
    val eventId: String,
    val packageName: String,
    val channel: String?,
    val title: String?,
    val body: String?,
    val redaction: Redaction,
    val ranking: Int?,
    val whenMs: Long,
    val actions: List<NotificationAction>,
    val canOpen: Boolean,
    val canDismiss: Boolean,
    val ongoing: Boolean,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            put("event_id", eventId)
            put("package", packageName)
            putOpt("channel", channel)
            putOpt("title", title)
            putOpt("body", body)
            put("redaction", redaction.wire)
            if (ranking != null) put("ranking", ranking)
            put("when_ms", whenMs)
            put("actions", jsonArray(actions.map { it.toJson() }))
            put("can_open", canOpen)
            put("can_dismiss", canDismiss)
            put("ongoing", ongoing)
        }
}

data class NotificationsParams(val max: Int) {
    companion object {
        const val DEFAULT_MAX = 20
        const val HARD_CAP = 200

        fun parse(params: JsonValue.Obj): NotificationsParams {
            val requested = params.int("max") ?: DEFAULT_MAX
            if (requested <= 0) bad("max must be positive")
            return NotificationsParams(minOf(requested, HARD_CAP))
        }
    }
}

data class NotificationsResult(
    val listenerEnabled: Boolean,
    val events: List<NotificationEvent>,
    val truncated: Boolean,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            put("listener_enabled", listenerEnabled)
            put("events", jsonArray(events.map { it.toJson() }))
            put("truncated", truncated)
        }
}

// ---------------------------------------------------------------------------
// notification_op
// ---------------------------------------------------------------------------

enum class NotificationOp(val wire: String) {
    OPEN("open"),
    DISMISS("dismiss"),
    ACTION("action"),
    REPLY("reply"),
}

data class NotificationOpParams(
    val eventId: String,
    val op: NotificationOp,
    val actionId: String?,
    val replyText: String?,
) {
    companion object {
        fun parse(params: JsonValue.Obj): NotificationOpParams {
            val eventId = params.string("event_id") ?: bad("missing event_id")
            val opStr = params.string("op") ?: bad("missing op")
            val op =
                NotificationOp.entries.firstOrNull { it.wire == opStr }
                    ?: bad("unknown notification op '$opStr'")

            val actionId = params.string("action_id")
            val replyText = params.string("reply_text")

            when (op) {
                NotificationOp.ACTION ->
                    if (actionId.isNullOrEmpty()) bad("action_id is required for action")
                NotificationOp.REPLY -> {
                    if (actionId.isNullOrEmpty()) bad("action_id is required for reply")
                    if (replyText == null) bad("reply_text is required for reply")
                }
                NotificationOp.OPEN, NotificationOp.DISMISS -> Unit
            }

            return NotificationOpParams(eventId, op, actionId, replyText)
        }
    }
}

fun okResult(): JsonValue.Obj = jsonObject { put("ok", true) }

// ---------------------------------------------------------------------------
// current_app
// ---------------------------------------------------------------------------

data class CurrentAppResult(
    val packageName: String,
    val activity: String?,
    val label: String?,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            put("package", packageName)
            putOpt("activity", activity)
            putOpt("label", label)
        }
}

// ---------------------------------------------------------------------------
// app_list
// ---------------------------------------------------------------------------

data class AppListParams(val launchableOnly: Boolean) {
    companion object {
        fun parse(params: JsonValue.Obj): AppListParams =
            AppListParams(launchableOnly = params.bool("launchable_only") ?: true)
    }
}

data class AppEntry(
    val packageName: String,
    val label: String?,
    val launchable: Boolean,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            put("package", packageName)
            putOpt("label", label)
            put("launchable", launchable)
        }
}

data class AppListResult(
    val apps: List<AppEntry>,
    val truncated: Boolean,
) {
    fun toJson(): JsonValue.Obj =
        jsonObject {
            put("apps", jsonArray(apps.map { it.toJson() }))
            put("truncated", truncated)
        }
}

// ---------------------------------------------------------------------------
// app_op
// ---------------------------------------------------------------------------

enum class AppOp(val wire: String) {
    LAUNCH("launch"),
    OPEN_INTENT("open_intent"),
    FORCE_STOP("force_stop"),
}

data class AppOpParams(
    val op: AppOp,
    val packageName: String?,
    val intentUri: String?,
) {
    companion object {
        fun parse(params: JsonValue.Obj): AppOpParams {
            val opStr = params.string("op") ?: bad("missing app op")
            val op =
                AppOp.entries.firstOrNull { it.wire == opStr }
                    ?: bad("unknown app op '$opStr'")

            val packageName = params.string("package")
            val intentUri = params.string("intent_uri")

            when (op) {
                AppOp.LAUNCH, AppOp.FORCE_STOP ->
                    if (packageName.isNullOrEmpty()) bad("package is required for $opStr")
                AppOp.OPEN_INTENT ->
                    if (intentUri.isNullOrEmpty()) bad("intent_uri is required for open_intent")
            }

            return AppOpParams(op, packageName, intentUri)
        }
    }
}

// ---------------------------------------------------------------------------
// health / capabilities
// ---------------------------------------------------------------------------

/** Shared liveness + permission/capability booleans for health and capabilities. */
data class HealthState(
    val version: String,
    val versionCode: Int,
    val packageName: String,
    val accessibilityEnabled: Boolean,
    val canPerformGestures: Boolean,
    val canRetrieveWindowContent: Boolean,
    val canTakeScreenshot: Boolean,
    val notificationListenerEnabled: Boolean,
    val nativeOverlay: Boolean,
    val nativeOverlayPassThrough: Boolean,
    val privilegedSetup: String?,
) {
    fun toHealthJson(): JsonValue.Obj =
        jsonObject {
            put("version", version)
            put("version_code", versionCode)
            put("package", packageName)
            put("accessibility_enabled", accessibilityEnabled)
            put("can_perform_gestures", canPerformGestures)
            put("can_retrieve_window_content", canRetrieveWindowContent)
            put("can_take_screenshot", canTakeScreenshot)
            put("notification_listener_enabled", notificationListenerEnabled)
            put("native_overlay", nativeOverlay)
            put("native_overlay_pass_through", nativeOverlayPassThrough)
            putOpt("privileged_setup", privilegedSetup)
        }
}

data class CapabilitiesState(
    val health: HealthState,
    val screenshotApiLevel: Int,
    val screenshotSupported: Boolean,
    val gestureSupported: Boolean,
    /** Android may recreate START_STICKY services, but does not guarantee it. */
    val processDeathAutorestart: Boolean = false,
) {
    fun toJson(): JsonValue.Obj {
        val base = health.toHealthJson().entries.toMutableMap()
        base["screenshot_api_level"] = JsonValue.of(screenshotApiLevel)
        base["screenshot_supported"] = JsonValue.of(screenshotSupported)
        base["gesture_supported"] = JsonValue.of(gestureSupported)
        base["process_death_autorestart"] = JsonValue.of(processDeathAutorestart)
        return JsonValue.Obj(base)
    }
}
