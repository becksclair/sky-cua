package com.skycua.phonecompanion.protocol

import com.skycua.phonecompanion.json.JsonValue

/**
 * Application error thrown by a [MethodHandler] when a well-formed RPC reports
 * that the requested operation could not be performed (secure window, expired
 * notification, etc.). The dispatcher turns this into an `ok:false` envelope
 * with the method-specific error code. The host treats these as successful RPCs,
 * never as a transport failure.
 */
class MethodApplicationException(
    val code: String,
    override val message: String,
) : Exception(message)

/**
 * Implemented by the Android runtime to perform each method against real device
 * state. Unit tests provide a fake implementation so the dispatcher and protocol
 * can be validated without a device. Handlers may throw
 * [MethodApplicationException] for per-method application errors and
 * [MethodParamException] for invalid parameters.
 */
interface MethodHandler {
    fun health(): HealthState

    fun capabilities(): CapabilitiesState

    fun accessibilityTree(params: AccessibilityTreeParams): AccessibilityTreeResult

    fun screenshot(params: ScreenshotParams): ScreenshotResult

    fun gesture(params: GestureParams): JsonValue.Obj

    fun cursorOverlay(params: CursorOverlayParams): JsonValue.Obj

    fun overlayActive(params: OverlayActiveParams): JsonValue.Obj

    fun overlayGesture(params: OverlayGestureParams): JsonValue.Obj

    fun notifications(params: NotificationsParams): NotificationsResult

    fun notificationOp(params: NotificationOpParams): JsonValue.Obj

    fun currentApp(): CurrentAppResult

    fun appList(params: AppListParams): AppListResult

    fun appOp(params: AppOpParams): JsonValue.Obj
}

/**
 * Validates auth and protocol version, parses method params, routes to a
 * [MethodHandler], and produces an [RpcResponse]. The token check runs before
 * method dispatch, exactly as the wire contract requires.
 */
class RpcDispatcher(
    private val handler: MethodHandler,
    private val tokenStore: TokenStore,
) {
    /** Parses a raw request body, dispatches it, and returns the response. */
    fun handleBody(body: String, nowMs: Long): RpcResponse =
        try {
            val request = Envelope.parseRequest(body)
            dispatch(request, nowMs)
        } catch (e: EnvelopeException) {
            RpcResponse.Failure(e.id, e.code, e.message)
        }

    fun dispatch(request: RpcRequest, nowMs: Long): RpcResponse {
        // 1. Protocol version is checked first so an unknown protocol never
        //    proceeds to auth or dispatch.
        if (request.protocolVersion != Protocol.VERSION) {
            return RpcResponse.Failure(
                request.id,
                Protocol.ErrorCodes.VERSION_MISMATCH,
                "unsupported protocol version ${request.protocolVersion}",
            )
        }

        // 2. Token is validated on every call, before method dispatch.
        if (!tokenStore.isValid(request.token, nowMs)) {
            return RpcResponse.Failure(
                request.id,
                Protocol.ErrorCodes.UNAUTHORIZED,
                "missing, wrong, or expired token",
            )
        }

        // 3. Method dispatch.
        return try {
            val result = invoke(request)
            RpcResponse.Success(request.id, result)
        } catch (e: MethodParamException) {
            RpcResponse.Failure(request.id, e.code, e.message)
        } catch (e: MethodApplicationException) {
            RpcResponse.Failure(request.id, e.code, e.message)
        }
    }

    private fun invoke(request: RpcRequest): JsonValue.Obj =
        when (request.method) {
            Protocol.Methods.HEALTH -> handler.health().toHealthJson()
            Protocol.Methods.CAPABILITIES -> handler.capabilities().toJson()
            Protocol.Methods.ACCESSIBILITY_TREE ->
                handler.accessibilityTree(AccessibilityTreeParams.parse(request.params)).toJson()
            Protocol.Methods.SCREENSHOT ->
                handler.screenshot(ScreenshotParams.parse(request.params)).toJson()
            Protocol.Methods.GESTURE ->
                handler.gesture(GestureParams.parse(request.params))
            Protocol.Methods.CURSOR_OVERLAY ->
                handler.cursorOverlay(CursorOverlayParams.parse(request.params))
            Protocol.Methods.OVERLAY_ACTIVE ->
                handler.overlayActive(OverlayActiveParams.parse(request.params))
            Protocol.Methods.OVERLAY_GESTURE ->
                handler.overlayGesture(OverlayGestureParams.parse(request.params))
            Protocol.Methods.NOTIFICATIONS ->
                handler.notifications(NotificationsParams.parse(request.params)).toJson()
            Protocol.Methods.NOTIFICATION_OP ->
                handler.notificationOp(NotificationOpParams.parse(request.params))
            Protocol.Methods.CURRENT_APP -> handler.currentApp().toJson()
            Protocol.Methods.APP_LIST ->
                handler.appList(AppListParams.parse(request.params)).toJson()
            Protocol.Methods.APP_OP ->
                handler.appOp(AppOpParams.parse(request.params))
            else ->
                throw MethodParamException(
                    Protocol.ErrorCodes.UNKNOWN_METHOD,
                    "unknown method '${request.method}'",
                )
        }
}
