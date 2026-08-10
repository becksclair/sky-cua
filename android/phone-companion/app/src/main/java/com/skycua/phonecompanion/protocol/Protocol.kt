package com.skycua.phonecompanion.protocol

import com.skycua.phonecompanion.json.JsonParseException
import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.JsonWriter
import com.skycua.phonecompanion.json.jsonObject

/**
 * Wire protocol contract for the Sky Phone Companion RPC, mirroring
 * docs/runtime/phone-companion-protocol.md. This file is the single source of
 * truth on the Android side for envelope shapes, the protocol version, method
 * names, and error codes.
 */
object Protocol {
    const val VERSION: Long = 1

    object Methods {
        const val HEALTH = "health"
        const val CAPABILITIES = "capabilities"
        const val ACCESSIBILITY_TREE = "accessibility_tree"
        const val SCREENSHOT = "screenshot"
        const val APP_SHOT = "appshot"
        const val GESTURE = "gesture"
        const val CURSOR_OVERLAY = "cursor_overlay"
        const val OVERLAY_ACTIVE = "overlay_active"
        const val OVERLAY_GESTURE = "overlay_gesture"
        const val NOTIFICATIONS = "notifications"
        const val NOTIFICATION_OP = "notification_op"
        const val CURRENT_APP = "current_app"
        const val APP_LIST = "app_list"
        const val APP_OP = "app_op"
        const val CLIPBOARD = "phone_clipboard"
        const val EDITOR = "phone_editor"
        const val STORAGE = "phone_storage"
        const val CAMERA = "phone_camera"
        const val KEY = "phone_key"
        /** Direct-only phone-control.v2 method; intentionally absent from ALL. */
        const val SMS_QUERY = "sms.query"

        val ALL =
            setOf(
                HEALTH,
                CAPABILITIES,
                ACCESSIBILITY_TREE,
                SCREENSHOT,
                APP_SHOT,
                GESTURE,
                CURSOR_OVERLAY,
                OVERLAY_ACTIVE,
                OVERLAY_GESTURE,
                NOTIFICATIONS,
                NOTIFICATION_OP,
                CURRENT_APP,
                APP_LIST,
                APP_OP,
                CLIPBOARD,
                EDITOR,
                STORAGE,
                CAMERA,
                KEY,
            )
    }

    /**
     * Error codes the companion may emit. Envelope/transport-level codes
     * ([UNAUTHORIZED], [VERSION_MISMATCH], [UNKNOWN_METHOD], [BAD_REQUEST]) are
     * distinct from per-method application errors (screenshot/notification
     * codes), which the host treats as successful RPCs reporting an operation
     * could not be performed.
     */
    object ErrorCodes {
        // Envelope / auth / dispatch.
        const val UNAUTHORIZED = "unauthorized"
        const val VERSION_MISMATCH = "version_mismatch"
        const val UNKNOWN_METHOD = "unknown_method"
        const val BAD_REQUEST = "bad_request"
        const val INTERNAL = "internal"

        // Screenshot application errors.
        const val SECURE_WINDOW = "secure_window"
        const val UNSUPPORTED_API = "unsupported_api"
        const val DISABLED_SERVICE = "disabled_service"

        // Reserved for the screenshot route but currently unreachable: the
        // AccessibilityService screenshot callback exposes no OEM-policy signal,
        // so no path emits this code. Kept in the contract for forward
        // compatibility (mirrored note in the Rust protocol doc).
        const val OEM_POLICY = "oem_policy"
        const val THROTTLED = "throttled"
        const val TRANSIENT = "transient"

        // notification_op application errors.
        const val GONE = "gone"
        const val REDACTED = "redacted"
        const val PENDING_INTENT_MISSING = "pending_intent_missing"
        const val CANCELED = "canceled"
        const val EXPIRED = "expired"
        const val IMMUTABLE = "immutable"
        const val REPLY_UNAVAILABLE = "reply_unavailable"
        const val OEM_FILTERED = "oem_filtered"
    }
}

/** A decoded request envelope. */
data class RpcRequest(
    val protocolVersion: Long,
    val token: String?,
    val id: Long,
    val method: String,
    val params: JsonValue.Obj,
)

/** A decoded/encoded response envelope. */
sealed class RpcResponse {
    abstract val id: Long

    data class Success(
        override val id: Long,
        val result: JsonValue.Obj,
    ) : RpcResponse()

    data class Failure(
        override val id: Long,
        val code: String,
        val message: String,
    ) : RpcResponse()
}

/** Thrown when an incoming envelope cannot be parsed into an [RpcRequest]. */
class EnvelopeException(
    val code: String,
    override val message: String,
    val id: Long = 0,
) : Exception(message)

/** (De)serialization of request and response envelopes. */
object Envelope {
    /**
     * Parses a request body into an [RpcRequest]. The protocol version is *not*
     * enforced here; the dispatcher checks it after parsing so it can return a
     * structured [Protocol.ErrorCodes.VERSION_MISMATCH] with the request id.
     *
     * @throws EnvelopeException with [Protocol.ErrorCodes.BAD_REQUEST] when the
     *   body is not a JSON object, the id is missing/non-integral, or the method
     *   is missing.
     */
    fun parseRequest(body: String): RpcRequest {
        val obj =
            try {
                JsonParser.parseObject(body)
            } catch (e: JsonParseException) {
                throw EnvelopeException(Protocol.ErrorCodes.BAD_REQUEST, "malformed JSON: ${e.message}")
            }

        val idNum =
            (obj["id"] as? JsonValue.Num)
                ?: throw EnvelopeException(Protocol.ErrorCodes.BAD_REQUEST, "missing or non-numeric id")
        if (!idNum.isIntegral) {
            throw EnvelopeException(Protocol.ErrorCodes.BAD_REQUEST, "id must be an integer")
        }
        val id = idNum.toLong()

        val protocolVersion =
            (obj["protocol_version"] as? JsonValue.Num)?.toLong()
                ?: throw EnvelopeException(
                    Protocol.ErrorCodes.BAD_REQUEST,
                    "missing protocol_version",
                    id,
                )

        val method =
            obj.string("method")
                ?: throw EnvelopeException(Protocol.ErrorCodes.BAD_REQUEST, "missing method", id)

        val token = obj.string("token")
        val params = obj.obj("params") ?: JsonValue.Obj(emptyMap())

        return RpcRequest(
            protocolVersion = protocolVersion,
            token = token,
            id = id,
            method = method,
            params = params,
        )
    }

    fun encodeResponse(response: RpcResponse): String {
        val obj =
            when (response) {
                is RpcResponse.Success ->
                    jsonObject {
                        put("protocol_version", Protocol.VERSION)
                        put("ok", true)
                        put("id", response.id)
                        put("result", response.result)
                    }
                is RpcResponse.Failure ->
                    jsonObject {
                        put("protocol_version", Protocol.VERSION)
                        put("ok", false)
                        put("id", response.id)
                        put(
                            "error",
                            jsonObject {
                                put("code", response.code)
                                put("message", response.message)
                            },
                        )
                    }
            }
        return JsonWriter.write(obj)
    }
}
