package com.skycua.phonecompanion.protocol

import com.skycua.phonecompanion.json.JsonValue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/** A fake handler returning deterministic results and a controllable error. */
private class FakeHandler : MethodHandler {
    var screenshotError: MethodApplicationException? = null
    var lastGesture: GestureParams? = null

    override fun health(): HealthState =
        HealthState(
            "0.1.0", 1, "com.skycua.phonecompanion",
            accessibilityEnabled = true,
            canPerformGestures = true,
            canRetrieveWindowContent = true,
            canTakeScreenshot = true,
            notificationListenerEnabled = true,
            nativeOverlay = true,
            nativeOverlayPassThrough = true,
            privilegedSetup = null,
        )

    override fun capabilities(): CapabilitiesState =
        CapabilitiesState(health(), 34, screenshotSupported = true, gestureSupported = true)

    override fun accessibilityTree(params: AccessibilityTreeParams): AccessibilityTreeResult =
        AccessibilityTreeResult("com.example", null, emptyList(), truncated = false, redacted = false)

    override fun screenshot(params: ScreenshotParams): ScreenshotResult {
        screenshotError?.let { throw it }
        return ScreenshotResult("image/png", "AAAA", 100, 200, false)
    }

    override fun gesture(params: GestureParams): JsonValue.Obj {
        lastGesture = params
        return gestureDispatchedResult()
    }

    override fun cursorOverlay(params: CursorOverlayParams): JsonValue.Obj =
        cursorOverlayResult(params.visible, true)

    var lastOverlayActive: OverlayActiveParams? = null
    var lastOverlayGesture: OverlayGestureParams? = null

    override fun overlayActive(params: OverlayActiveParams): JsonValue.Obj {
        lastOverlayActive = params
        return overlayActiveResult(active = params.active, glowSupported = true)
    }

    override fun overlayGesture(params: OverlayGestureParams): JsonValue.Obj {
        lastOverlayGesture = params
        return overlayGestureResult(animated = true)
    }

    override fun notifications(params: NotificationsParams): NotificationsResult =
        NotificationsResult(listenerEnabled = true, events = emptyList(), truncated = false)

    override fun notificationOp(params: NotificationOpParams): JsonValue.Obj = okResult()

    override fun currentApp(): CurrentAppResult = CurrentAppResult("com.android.chrome", null, "Chrome")

    override fun appList(params: AppListParams): AppListResult =
        AppListResult(emptyList(), truncated = false)

    override fun appOp(params: AppOpParams): JsonValue.Obj = okResult()
}

class DispatcherTest {
    private lateinit var handler: FakeHandler
    private lateinit var tokens: TokenStore
    private lateinit var dispatcher: RpcDispatcher
    private val now = 1_000L

    @Test fun smsQueryIsNotPartOfLegacyV1MethodRegistry() {
        assertTrue(!Protocol.Methods.ALL.contains(Protocol.Methods.SMS_QUERY))
    }

    @Before
    fun setUp() {
        handler = FakeHandler()
        tokens = TokenStore().apply { install("secret", expiresAtMs = 10_000) }
        dispatcher = RpcDispatcher(handler, tokens)
    }

    private fun request(
        method: String,
        params: String = "{}",
        token: String? = "secret",
        version: Long = 1,
        id: Long = 1,
    ): String {
        val tokenPart = if (token == null) "" else ""","token":"$token""""
        return """{"protocol_version":$version,"id":$id,"method":"$method","params":$params$tokenPart}"""
    }

    @Test
    fun dispatchesHealthSuccessfully() {
        val response = dispatcher.handleBody(request("health"), now)
        assertTrue(response is RpcResponse.Success)
        assertEquals(1L, response.id)
    }

    @Test
    fun rejectsWrongToken() {
        val response = dispatcher.handleBody(request("health", token = "wrong"), now)
        assertTrue(response is RpcResponse.Failure)
        assertEquals(Protocol.ErrorCodes.UNAUTHORIZED, (response as RpcResponse.Failure).code)
    }

    @Test
    fun rejectsMissingToken() {
        val response = dispatcher.handleBody(request("health", token = null), now)
        assertEquals(
            Protocol.ErrorCodes.UNAUTHORIZED,
            (response as RpcResponse.Failure).code,
        )
    }

    @Test
    fun rejectsExpiredToken() {
        val response = dispatcher.handleBody(request("health"), nowMs = 999_999)
        assertEquals(
            Protocol.ErrorCodes.UNAUTHORIZED,
            (response as RpcResponse.Failure).code,
        )
    }

    @Test
    fun rejectsVersionMismatchBeforeAuth() {
        // Wrong version + wrong token: version is checked first.
        val response = dispatcher.handleBody(request("health", token = "wrong", version = 2), now)
        assertEquals(
            Protocol.ErrorCodes.VERSION_MISMATCH,
            (response as RpcResponse.Failure).code,
        )
    }

    @Test
    fun unknownMethodIsStructuredError() {
        val response = dispatcher.handleBody(request("teleport"), now)
        assertEquals(
            Protocol.ErrorCodes.UNKNOWN_METHOD,
            (response as RpcResponse.Failure).code,
        )
    }

    @Test
    fun applicationErrorIsPropagatedWithMethodCode() {
        handler.screenshotError =
            MethodApplicationException(Protocol.ErrorCodes.SECURE_WINDOW, "secure")
        val response = dispatcher.handleBody(request("screenshot"), now)
        assertEquals(
            Protocol.ErrorCodes.SECURE_WINDOW,
            (response as RpcResponse.Failure).code,
        )
    }

    @Test
    fun invalidGestureParamsAreBadRequest() {
        val response =
            dispatcher.handleBody(request("gesture", params = """{"kind":"tap","points":[]}"""), now)
        assertEquals(
            Protocol.ErrorCodes.BAD_REQUEST,
            (response as RpcResponse.Failure).code,
        )
    }

    @Test
    fun echoesRequestId() {
        val response = dispatcher.handleBody(request("health", id = 42), now)
        assertEquals(42L, response.id)
    }

    @Test
    fun gestureParamsReachHandler() {
        dispatcher.handleBody(
            request("gesture", params = """{"kind":"tap","points":[{"x":7,"y":8}]}"""),
            now,
        )
        assertEquals(GesturePoint(7, 8), handler.lastGesture!!.points[0])
    }

    @Test
    fun routesOverlayActiveToHandler() {
        val response =
            dispatcher.handleBody(request("overlay_active", params = """{"active":true}"""), now)
        assertTrue(response is RpcResponse.Success)
        assertEquals(true, handler.lastOverlayActive!!.active)
    }

    @Test
    fun overlayActiveMissingFlagIsBadRequest() {
        val response = dispatcher.handleBody(request("overlay_active", params = "{}"), now)
        assertEquals(
            Protocol.ErrorCodes.BAD_REQUEST,
            (response as RpcResponse.Failure).code,
        )
    }

    @Test
    fun routesOverlayGestureToHandler() {
        val response =
            dispatcher.handleBody(
                request(
                    "overlay_gesture",
                    params = """{"kind":"swipe","points":[{"x":1,"y":2},{"x":9,"y":9}],"duration_ms":250}""",
                ),
                now,
            )
        assertTrue(response is RpcResponse.Success)
        val gesture = handler.lastOverlayGesture!!
        assertEquals(OverlayGestureParams.KIND_SWIPE, gesture.kind)
        assertEquals(2, gesture.points.size)
        assertEquals(250L, gesture.durationMs)
    }

    @Test
    fun overlayGestureUnknownKindIsBadRequest() {
        val response =
            dispatcher.handleBody(
                request("overlay_gesture", params = """{"kind":"pinch","points":[{"x":1,"y":2}]}"""),
                now,
            )
        assertEquals(
            Protocol.ErrorCodes.BAD_REQUEST,
            (response as RpcResponse.Failure).code,
        )
    }
}
