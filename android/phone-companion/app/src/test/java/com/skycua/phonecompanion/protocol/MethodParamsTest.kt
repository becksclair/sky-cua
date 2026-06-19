package com.skycua.phonecompanion.protocol

import com.skycua.phonecompanion.json.JsonParser
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MethodParamsTest {
    private fun params(json: String) = JsonParser.parseObject(json)

    private fun expectBadRequest(block: () -> Unit) {
        val ex = runCatching(block).exceptionOrNull()
        assertTrue("expected MethodParamException, got $ex", ex is MethodParamException)
        assertEquals(Protocol.ErrorCodes.BAD_REQUEST, (ex as MethodParamException).code)
    }

    // --- gesture validation ---------------------------------------------------

    @Test
    fun parsesTapGesture() {
        val g = GestureParams.parse(params("""{"kind":"tap","points":[{"x":5,"y":6}]}"""))
        assertEquals(GestureKind.TAP, g.kind)
        assertEquals(1, g.points.size)
        assertEquals(GesturePoint(5, 6), g.points[0])
        assertEquals(GestureParams.DEFAULT_DURATION_MS, g.durationMs)
    }

    @Test
    fun parsesSwipeGestureWithDuration() {
        val g =
            GestureParams.parse(
                params("""{"kind":"swipe","points":[{"x":1,"y":2},{"x":3,"y":4}],"duration_ms":120}"""),
            )
        assertEquals(GestureKind.SWIPE, g.kind)
        assertEquals(2, g.points.size)
        assertEquals(120L, g.durationMs)
    }

    @Test
    fun tapRequiresExactlyOnePoint() {
        expectBadRequest {
            GestureParams.parse(params("""{"kind":"tap","points":[{"x":1,"y":2},{"x":3,"y":4}]}"""))
        }
    }

    @Test
    fun swipeRequiresTwoPoints() {
        expectBadRequest {
            GestureParams.parse(params("""{"kind":"swipe","points":[{"x":1,"y":2}]}"""))
        }
    }

    @Test
    fun gestureRejectsUnknownKind() {
        expectBadRequest { GestureParams.parse(params("""{"kind":"pinch","points":[]}""")) }
    }

    @Test
    fun gestureRejectsNegativeCoordinates() {
        expectBadRequest {
            GestureParams.parse(params("""{"kind":"tap","points":[{"x":-1,"y":2}]}"""))
        }
    }

    @Test
    fun gestureRejectsNonPositiveDuration() {
        expectBadRequest {
            GestureParams.parse(params("""{"kind":"tap","points":[{"x":1,"y":2}],"duration_ms":0}"""))
        }
    }

    @Test
    fun gestureRejectsExcessiveDuration() {
        expectBadRequest {
            GestureParams.parse(
                params("""{"kind":"tap","points":[{"x":1,"y":2}],"duration_ms":999999}"""),
            )
        }
    }

    // --- notification op validation ------------------------------------------

    @Test
    fun parsesNotificationOpen() {
        val op = NotificationOpParams.parse(params("""{"event_id":"evt-1","op":"open"}"""))
        assertEquals(NotificationOp.OPEN, op.op)
        assertEquals("evt-1", op.eventId)
    }

    @Test
    fun actionRequiresActionId() {
        expectBadRequest {
            NotificationOpParams.parse(params("""{"event_id":"evt-1","op":"action"}"""))
        }
    }

    @Test
    fun replyRequiresActionIdAndText() {
        expectBadRequest {
            NotificationOpParams.parse(
                params("""{"event_id":"evt-1","op":"reply","action_id":"action-0"}"""),
            )
        }
    }

    @Test
    fun parsesValidReply() {
        val op =
            NotificationOpParams.parse(
                params(
                    """{"event_id":"evt-1","op":"reply","action_id":"action-0","reply_text":"hi"}""",
                ),
            )
        assertEquals(NotificationOp.REPLY, op.op)
        assertEquals("action-0", op.actionId)
        assertEquals("hi", op.replyText)
    }

    @Test
    fun rejectsUnknownNotificationOp() {
        expectBadRequest {
            NotificationOpParams.parse(params("""{"event_id":"evt-1","op":"snooze"}"""))
        }
    }

    @Test
    fun missingEventIdIsBadRequest() {
        expectBadRequest { NotificationOpParams.parse(params("""{"op":"open"}""")) }
    }

    // --- app op validation ----------------------------------------------------

    @Test
    fun parsesAppLaunch() {
        val op = AppOpParams.parse(params("""{"op":"launch","package":"com.example"}"""))
        assertEquals(AppOp.LAUNCH, op.op)
        assertEquals("com.example", op.packageName)
    }

    @Test
    fun launchRequiresPackage() {
        expectBadRequest { AppOpParams.parse(params("""{"op":"launch"}""")) }
    }

    @Test
    fun openIntentRequiresUri() {
        expectBadRequest { AppOpParams.parse(params("""{"op":"open_intent"}""")) }
    }

    @Test
    fun forceStopRequiresPackage() {
        expectBadRequest { AppOpParams.parse(params("""{"op":"force_stop"}""")) }
    }

    @Test
    fun parsesOpenIntent() {
        val op =
            AppOpParams.parse(
                params("""{"op":"open_intent","intent_uri":"https://example.com"}"""),
            )
        assertEquals(AppOp.OPEN_INTENT, op.op)
        assertEquals("https://example.com", op.intentUri)
    }

    @Test
    fun rejectsUnknownAppOp() {
        expectBadRequest { AppOpParams.parse(params("""{"op":"reboot"}""")) }
    }

    // --- app list / notifications / accessibility tree bounds ----------------

    @Test
    fun appListDefaultsLaunchableOnlyTrue() {
        assertTrue(AppListParams.parse(params("{}")).launchableOnly)
        assertEquals(false, AppListParams.parse(params("""{"launchable_only":false}""")).launchableOnly)
    }

    @Test
    fun notificationsClampsToHardCap() {
        val p = NotificationsParams.parse(params("""{"max":100000}"""))
        assertEquals(NotificationsParams.HARD_CAP, p.max)
    }

    @Test
    fun notificationsRejectsNonPositiveMax() {
        expectBadRequest { NotificationsParams.parse(params("""{"max":0}""")) }
    }

    @Test
    fun accessibilityTreeClampsToHardCap() {
        val p = AccessibilityTreeParams.parse(params("""{"max_nodes":100000}"""))
        assertEquals(AccessibilityTreeParams.HARD_CAP, p.maxNodes)
    }

    @Test
    fun accessibilityTreeDefaults() {
        assertEquals(
            AccessibilityTreeParams.DEFAULT_MAX_NODES,
            AccessibilityTreeParams.parse(params("{}")).maxNodes,
        )
    }

    @Test
    fun cursorOverlayRejectsNegativeWhenVisible() {
        expectBadRequest {
            CursorOverlayParams.parse(params("""{"visible":true,"x":-1,"y":2}"""))
        }
    }

    @Test
    fun cursorOverlayHidePermitsMissingCoordinates() {
        val p = CursorOverlayParams.parse(params("""{"visible":false}"""))
        assertEquals(false, p.visible)
    }

    // --- overlay_active validation -------------------------------------------

    @Test
    fun parsesOverlayActive() {
        assertTrue(OverlayActiveParams.parse(params("""{"active":true}""")).active)
        assertEquals(false, OverlayActiveParams.parse(params("""{"active":false}""")).active)
    }

    @Test
    fun overlayActiveRequiresActiveFlag() {
        expectBadRequest { OverlayActiveParams.parse(params("{}")) }
    }

    // --- overlay_gesture validation ------------------------------------------

    @Test
    fun parsesOverlayTap() {
        val g = OverlayGestureParams.parse(params("""{"kind":"tap","points":[{"x":3,"y":4}]}"""))
        assertEquals(OverlayGestureParams.KIND_TAP, g.kind)
        assertEquals(1, g.points.size)
        assertEquals(GesturePoint(3, 4), g.points[0])
        assertEquals(OverlayGestureParams.DEFAULT_DURATION_MS, g.durationMs)
    }

    @Test
    fun parsesOverlayDragWithMultiplePoints() {
        val g =
            OverlayGestureParams.parse(
                params(
                    """{"kind":"drag","points":[{"x":1,"y":1},{"x":2,"y":2},{"x":3,"y":3}],"duration_ms":300}""",
                ),
            )
        assertEquals(OverlayGestureParams.KIND_DRAG, g.kind)
        assertEquals(3, g.points.size)
        assertEquals(300L, g.durationMs)
    }

    @Test
    fun overlayGestureRejectsUnknownKind() {
        expectBadRequest {
            OverlayGestureParams.parse(params("""{"kind":"pinch","points":[{"x":1,"y":2}]}"""))
        }
    }

    @Test
    fun overlayTapRejectsEmptyPoints() {
        expectBadRequest { OverlayGestureParams.parse(params("""{"kind":"tap","points":[]}""")) }
    }

    @Test
    fun overlaySwipeRequiresAtLeastTwoPoints() {
        expectBadRequest {
            OverlayGestureParams.parse(params("""{"kind":"swipe","points":[{"x":1,"y":2}]}"""))
        }
    }

    @Test
    fun overlayGestureRejectsNegativeCoordinates() {
        expectBadRequest {
            OverlayGestureParams.parse(params("""{"kind":"tap","points":[{"x":-1,"y":2}]}"""))
        }
    }

    @Test
    fun overlayGestureRejectsExcessiveDuration() {
        expectBadRequest {
            OverlayGestureParams.parse(
                params("""{"kind":"tap","points":[{"x":1,"y":2}],"duration_ms":999999}"""),
            )
        }
    }
}
