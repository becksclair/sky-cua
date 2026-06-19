package com.skycua.phonecompanion.protocol

import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class ResultSerializationTest {
    @Test
    fun healthResultMatchesContract() {
        val health =
            HealthState(
                version = "1.2.0",
                versionCode = 12,
                packageName = "com.skycua.phonecompanion",
                accessibilityEnabled = true,
                canPerformGestures = true,
                canRetrieveWindowContent = true,
                canTakeScreenshot = true,
                notificationListenerEnabled = true,
                nativeOverlay = true,
                nativeOverlayPassThrough = true,
                privilegedSetup = "shizuku",
            )
        val obj = JsonParser.parseObject(JsonWriter.write(health.toHealthJson()))
        assertEquals("1.2.0", obj.string("version"))
        assertEquals(12L, obj.long("version_code"))
        assertEquals("com.skycua.phonecompanion", obj.string("package"))
        assertEquals(true, obj.bool("accessibility_enabled"))
        assertEquals(true, obj.bool("can_perform_gestures"))
        assertEquals(true, obj.bool("can_retrieve_window_content"))
        assertEquals(true, obj.bool("can_take_screenshot"))
        assertEquals(true, obj.bool("notification_listener_enabled"))
        assertEquals(true, obj.bool("native_overlay"))
        assertEquals(true, obj.bool("native_overlay_pass_through"))
        assertEquals("shizuku", obj.string("privileged_setup"))
    }

    @Test
    fun healthOmitsPrivilegedSetupWhenNull() {
        val health = sampleHealth(privilegedSetup = null)
        val obj = JsonParser.parseObject(JsonWriter.write(health.toHealthJson()))
        assertNull(obj["privileged_setup"])
    }

    @Test
    fun capabilitiesAddsScreenshotAndGestureDetail() {
        val caps =
            CapabilitiesState(
                health = sampleHealth(),
                screenshotApiLevel = 34,
                screenshotSupported = true,
                gestureSupported = true,
            )
        val obj = JsonParser.parseObject(JsonWriter.write(caps.toJson()))
        assertEquals(34L, obj.long("screenshot_api_level"))
        assertEquals(true, obj.bool("screenshot_supported"))
        assertEquals(true, obj.bool("gesture_supported"))
        // health fields carried through.
        assertEquals(true, obj.bool("accessibility_enabled"))
    }

    @Test
    fun accessibilityTreeSerializesBoundsAndFlags() {
        val node =
            AccessibilityNode(
                className = "android.widget.Button",
                text = "OK",
                contentDesc = "Confirm",
                bounds = intArrayOf(10, 20, 110, 70),
                focusable = true,
                enabled = true,
                clickable = true,
            )
        val result =
            AccessibilityTreeResult("com.example", ".MainActivity", listOf(node), false, false)
        val obj = JsonParser.parseObject(JsonWriter.write(result.toJson()))
        assertEquals("com.example", obj.string("package"))
        val nodes = obj.arr("nodes")!!
        val first = nodes.items[0] as com.skycua.phonecompanion.json.JsonValue.Obj
        val bounds = first.arr("bounds")!!.items.map { (it as com.skycua.phonecompanion.json.JsonValue.Num).toInt() }
        assertEquals(listOf(10, 20, 110, 70), bounds)
        assertEquals(true, first.bool("clickable"))
        assertEquals(false, obj.bool("truncated"))
    }

    @Test
    fun notificationEventSerializesActionsAndRedaction() {
        val event =
            NotificationEvent(
                eventId = "evt-123",
                packageName = "com.example.chat",
                channel = "messages",
                title = "Alice",
                body = "see you at 5",
                redaction = Redaction.NONE,
                ranking = 3,
                whenMs = 1718600000000L,
                actions = listOf(NotificationAction("action-0", "Reply", true)),
                canOpen = true,
                canDismiss = true,
                ongoing = false,
            )
        val obj = JsonParser.parseObject(JsonWriter.write(event.toJson()))
        assertEquals("evt-123", obj.string("event_id"))
        assertEquals("com.example.chat", obj.string("package"))
        assertEquals("none", obj.string("redaction"))
        assertEquals(3L, obj.long("ranking"))
        assertEquals(1718600000000L, obj.long("when_ms"))
        assertEquals(true, obj.bool("can_open"))
        assertEquals(true, obj.bool("can_dismiss"))
        assertEquals(false, obj.bool("ongoing"))
        val action = obj.arr("actions")!!.items[0] as com.skycua.phonecompanion.json.JsonValue.Obj
        assertEquals("action-0", action.string("action_id"))
        assertEquals(true, action.bool("is_reply"))
    }

    @Test
    fun notificationEventOmitsRankingWhenNull() {
        val event =
            NotificationEvent(
                eventId = "evt-1",
                packageName = "com.example",
                channel = null,
                title = null,
                body = null,
                redaction = Redaction.FULL,
                ranking = null,
                whenMs = 0,
                actions = emptyList(),
                canOpen = false,
                canDismiss = true,
                ongoing = true,
            )
        val obj = JsonParser.parseObject(JsonWriter.write(event.toJson()))
        assertNull(obj["ranking"])
        assertNull(obj["title"])
        assertEquals("full", obj.string("redaction"))
        assertEquals(false, obj.bool("can_open"))
        assertEquals(true, obj.bool("can_dismiss"))
        assertEquals(true, obj.bool("ongoing"))
    }

    @Test
    fun screenshotResultMatchesContract() {
        val result = ScreenshotResult("image/png", "AAAA", 1080, 2400, false)
        val obj = JsonParser.parseObject(JsonWriter.write(result.toJson()))
        assertEquals("image/png", obj.string("mime_type"))
        assertEquals("AAAA", obj.string("data_base64"))
        assertEquals(1080L, obj.long("width"))
        assertEquals(2400L, obj.long("height"))
        assertFalse(obj.bool("contains_native_overlay")!!)
    }

    @Test
    fun appListResultSerializes() {
        val result =
            AppListResult(
                apps = listOf(AppEntry("com.example", "Example", true)),
                truncated = false,
            )
        val obj = JsonParser.parseObject(JsonWriter.write(result.toJson()))
        val app = obj.arr("apps")!!.items[0] as com.skycua.phonecompanion.json.JsonValue.Obj
        assertEquals("com.example", app.string("package"))
        assertEquals("Example", app.string("label"))
        assertEquals(true, app.bool("launchable"))
    }

    @Test
    fun currentAppResultOmitsOptionalFields() {
        val obj = JsonParser.parseObject(JsonWriter.write(CurrentAppResult("com.x", null, null).toJson()))
        assertEquals("com.x", obj.string("package"))
        assertNull(obj["activity"])
        assertNull(obj["label"])
    }

    @Test
    fun cursorOverlayResultSerializes() {
        val obj = JsonParser.parseObject(JsonWriter.write(cursorOverlayResult(true, true)))
        assertEquals(true, obj.bool("shown"))
        assertEquals(true, obj.bool("pass_through"))
    }

    @Test
    fun gestureDispatchedResultSerializes() {
        val obj = JsonParser.parseObject(JsonWriter.write(gestureDispatchedResult()))
        assertEquals(true, obj.bool("dispatched"))
    }

    private fun sampleHealth(privilegedSetup: String? = "shizuku") =
        HealthState(
            version = "0.1.0",
            versionCode = 1,
            packageName = "com.skycua.phonecompanion",
            accessibilityEnabled = true,
            canPerformGestures = true,
            canRetrieveWindowContent = true,
            canTakeScreenshot = true,
            notificationListenerEnabled = true,
            nativeOverlay = true,
            nativeOverlayPassThrough = true,
            privilegedSetup = privilegedSetup,
        )
}
