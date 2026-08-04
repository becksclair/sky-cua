package com.skycua.phonecompanion.direct

import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.JsonWriter
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.protocol.RpcDispatcher
import com.skycua.phonecompanion.protocol.RpcResponse
import com.skycua.phonecompanion.protocol.TokenStore
import com.skycua.phonecompanion.protocol.Protocol
import java.io.File
import java.util.Base64

fun interface DirectRequestHandler {
    fun dispatch(frame: String, deviceId: String, linkEpoch: String, nowMs: Long): String
}

/** Dispatches authenticated phone-control.v2 request frames through the v1 method handler. */
class DirectRequestDispatcher(
    private val rpc: RpcDispatcher,
    private val contentSender: (() -> ContentTransferSender?)? = null,
    private val contentResolver: DirectContentResolver? = null,
) : DirectRequestHandler {
    override fun dispatch(frame: String, deviceId: String, linkEpoch: String, nowMs: Long): String {
        var requestId: String? = null
        var incomingDevice: String? = null
        var incomingEpoch: Long? = null
        return try {
            val obj = JsonParser.parseObject(frame)
            requestId = obj.string("request_id")
            incomingDevice = obj.string("device_id")
            incomingEpoch = (obj["link_epoch"] as? JsonValue.Num)?.takeIf { it.isIntegral }?.toLong()
            require(obj.string("type") == "request") { "expected request frame" }
            require(!requestId.isNullOrEmpty()) { "missing request_id" }
            require(incomingDevice != null) { "missing device_id" }
            require(incomingEpoch != null) { "missing link_epoch" }
            require(obj["idempotent"] is JsonValue.Bool) { "missing idempotent" }
            val expires = (obj["expires_at_ms"] as? JsonValue.Num)?.takeIf { it.isIntegral }?.toLong() ?: error("missing expires_at_ms")
            val method = obj.string("method") ?: error("missing method")
            val params = obj.obj("params") ?: error("missing params")
            if (incomingDevice != deviceId) {
                errorFrame(requestId, incomingDevice, incomingEpoch, "device_mismatch", "request device does not match authenticated device")
            } else if (incomingEpoch.toString() != linkEpoch) {
                errorFrame(requestId, incomingDevice, incomingEpoch, "epoch_mismatch", "request epoch does not match authenticated epoch")
            } else if (nowMs >= expires) {
                errorFrame(requestId, incomingDevice, incomingEpoch, "expired", "request has expired")
            } else {
                val directContent = handleDirectContent(method, params, incomingEpoch)
                if (directContent != null) successFrame(requestId, deviceId, incomingEpoch, directContent)
                else {
                    val canonical = alias(method, params)
                    val body = JsonWriter.write(jsonObject {
                        put("protocol_version", Protocol.VERSION)
                        put("token", DIRECT_TOKEN)
                        put("id", 1L)
                        put("method", canonical.first)
                        put("params", canonical.second)
                    })
                    when (val response = rpc.handleBody(body, nowMs)) {
                        is RpcResponse.Success -> successFrame(
                            requestId,
                            deviceId,
                            incomingEpoch,
                            externalizeContent(response.result, deviceId, incomingEpoch),
                        )
                        is RpcResponse.Failure -> errorFrame(requestId, deviceId, incomingEpoch, response.code, response.message)
                    }
                }
            }
        } catch (e: Exception) {
            errorFrame(requestId, incomingDevice, incomingEpoch, "bad_request", e.message ?: "malformed request")
        }
    }

    private fun successFrame(requestId: String?, deviceId: String, epoch: Long, result: JsonValue.Obj): String =
        JsonWriter.write(jsonObject {
            put("type", "response"); put("request_id", requestId!!); put("device_id", deviceId); put("link_epoch", epoch); put("result", result)
        })

    private fun handleDirectContent(method: String, params: JsonValue.Obj, epoch: Long): JsonValue.Obj? {
        val resolver = contentResolver ?: return null
        val contentId = params.string("content_id")
        return when (method) {
            "content.describe" -> {
                val content = contentId?.let(resolver::describe)
                    ?.takeIf { it.linkEpoch == epoch }
                    ?: throw IllegalArgumentException("content was not found for this link epoch")
                jsonObject {
                    put("content", jsonObject {
                        put("content_id", content.contentId); put("device_id", content.deviceId); put("link_epoch", content.linkEpoch)
                        put("mime_type", content.mimeType); content.filename?.let { put("filename", it) }
                        put("size_bytes", content.sizeBytes); put("sha256", content.sha256); put("source", content.source)
                        content.expiresAtMs?.let { put("expires_at_ms", it) }; put("persistence", "temporary")
                    })
                    put("released", false)
                }
            }
            "content.release" -> jsonObject {
                put("released", contentId?.let { resolver.release(it, epoch) } == true)
            }
            "content.export" -> {
                val content = contentId?.let(resolver::describe)
                    ?.takeIf { it.linkEpoch == epoch }
                    ?: throw IllegalArgumentException("content was not found for this link epoch")
                val transferred = contentSender?.invoke()?.send(
                    content.file.readBytes(),
                    content.mimeType,
                    content.source,
                    content.contentId,
                ) ?: throw IllegalStateException("content transfer is unavailable")
                jsonObject {
                    put("content", transferred.toJson())
                    put("released", false)
                }
            }
            else -> null
        }
    }

    private fun externalizeContent(
        result: JsonValue.Obj,
        deviceId: String,
        epoch: Long,
    ): JsonValue.Obj {
        val sender = contentSender?.invoke()
        val entries = LinkedHashMap(result.entries)
        val localPath = result.string("_content_path")
        if (localPath != null) {
            entries.remove("_content_path")
            entries.remove("_content_mime")
            entries.remove("_content_source")
            entries.remove("_content_filename")
            val content = contentResolver?.registerLocal(
                File(localPath),
                deviceId,
                epoch,
                result.string("_content_mime") ?: "application/octet-stream",
                result.string("_content_filename"),
                result.string("_content_source") ?: "companion_blob",
            )
            if (content != null) {
                entries["content"] = jsonObject {
                    put("content_id", content.contentId); put("device_id", content.deviceId); put("link_epoch", content.linkEpoch)
                    put("mime_type", content.mimeType); content.filename?.let { put("filename", it) }
                    put("size_bytes", content.sizeBytes); put("sha256", content.sha256); put("source", content.source)
                    content.expiresAtMs?.let { put("expires_at_ms", it) }; put("persistence", "temporary")
                }
            } else {
                entries["content_unavailable"] = JsonValue.Bool(true)
            }
        }
        val genericEncoded = result.string("_content_base64")
        if (genericEncoded != null) {
            entries.remove("_content_base64")
            entries.remove("_content_mime")
            entries.remove("_content_source")
            entries.remove("_content_filename")
            val bytes = runCatching { Base64.getDecoder().decode(genericEncoded) }.getOrNull()
            val ref = bytes?.let {
                sender?.send(
                    it,
                    result.string("_content_mime") ?: "application/octet-stream",
                    result.string("_content_source") ?: "companion_blob",
                )
            }
            if (ref != null) entries["content"] = ref.toJson() else entries["content_unavailable"] = JsonValue.Bool(true)
        }
        val screenshot = result.obj("screenshot")
        if (screenshot != null) {
            val encoded = screenshot.string("data_base64")
            if (encoded != null) {
                val replacement = LinkedHashMap(screenshot.entries); replacement.remove("data_base64")
                val bytes = runCatching { Base64.getDecoder().decode(encoded) }.getOrNull()
                val ref = bytes?.let { sender?.send(it, screenshot.string("mime_type") ?: "application/octet-stream", "screenshot") }
                if (ref != null) replacement["content_ref"] = ref.toJson() else replacement["content_unavailable"] = JsonValue.Bool(true)
                entries["screenshot"] = JsonValue.Obj(replacement)
            }
        }
        val windows = result["windows"]
        if (windows != null) {
            val treeBytes = JsonWriter.write(windows).toByteArray(Charsets.UTF_8)
            if (treeBytes.size > PHONE_CONTENT_MAX_CHUNK_BYTES) sender?.send(treeBytes, "application/json", "companion_blob")?.let { entries["full_tree_content_ref"] = it.toJson() }
        }
        return JsonValue.Obj(entries)
    }

    private fun alias(method: String, params: JsonValue.Obj): Pair<String, JsonValue.Obj> = when (method) {
        "companion.status" -> "health" to params
        "appshot" -> "appshot" to params
        "app.launch" -> "app_op" to jsonObject { put("op", "launch"); params.string("package")?.let { put("package", it) } ?: params.string("package_name")?.let { put("package", it) } }
        "app.open_intent" -> "app_op" to jsonObject { put("op", "open_intent"); params.string("intent_uri")?.let { put("intent_uri", it) } }
        "clipboard" -> "phone_clipboard" to params
        "editor" -> "phone_editor" to params
        "input.text" -> "phone_editor" to jsonObject { put("operation", "insert_text"); params.string("text")?.let { put("text", it) } }
        "input.key" -> "phone_key" to params
        "storage" -> "phone_storage" to params
        "camera" -> "phone_camera" to params
        "app.settings", "open_settings" -> {
            val action = when (params.string("screen")) {
                "accessibility" -> "android.settings.ACCESSIBILITY_SETTINGS"
                "notification_access" -> "android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS"
                else -> null
            }
            "app_op" to jsonObject { put("op", "open_intent"); if (action != null) put("intent_uri", "intent:#Intent;action=$action;end") }
        }
        else -> method to params
    }

    private fun errorFrame(requestId: String?, deviceId: String?, epoch: Long?, code: String, message: String): String =
        JsonWriter.write(jsonObject {
            put("type", "error"); putOpt("request_id", requestId); putOpt("device_id", deviceId); if (epoch != null) put("link_epoch", epoch); put("code", code); put("message", message)
        })

    companion object {
        private const val DIRECT_TOKEN = "direct-authenticated-session"
        fun forHandler(
            handler: com.skycua.phonecompanion.protocol.MethodHandler,
            contentSender: (() -> ContentTransferSender?)? = null,
            contentResolver: DirectContentResolver? = null,
        ): DirectRequestDispatcher {
            val tokens = TokenStore().also { it.install(DIRECT_TOKEN, Long.MAX_VALUE) }
            return DirectRequestDispatcher(RpcDispatcher(handler, tokens), contentSender, contentResolver)
        }
    }
}
