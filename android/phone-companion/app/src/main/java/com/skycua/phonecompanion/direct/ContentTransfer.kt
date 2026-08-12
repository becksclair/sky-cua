package com.skycua.phonecompanion.direct

import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.JsonWriter
import com.skycua.phonecompanion.json.jsonObject
import java.security.MessageDigest
import java.util.UUID

const val PHONE_CONTENT_MAX_CHUNK_BYTES = 256 * 1024

data class DirectContentRef(
    val contentId: String,
    val deviceId: String,
    val linkEpoch: LinkEpoch,
    val mimeType: String,
    val sizeBytes: Long,
    val sha256: String,
    val source: String,
    val expiresAtMs: Long? = null,
) {
    fun toJson(): JsonValue.Obj = jsonObject {
        put("content_id", contentId); put("device_id", deviceId); put("link_epoch", linkEpoch)
        put("mime_type", mimeType); put("size_bytes", sizeBytes); put("sha256", sha256); put("source", source)
        put("persistence", "temporary"); expiresAtMs?.let { put("expires_at_ms", it) }
    }
}

object DirectContentChunkCodec {
    fun encode(transferId: String, epoch: LinkEpoch, index: Long, offset: Long, payload: ByteArray): ByteArray {
        val id = transferId.toByteArray(Charsets.UTF_8)
        require(id.isNotEmpty() && id.size <= 255) { "transfer id must be 1..255 UTF-8 bytes" }
        require(payload.size <= PHONE_CONTENT_MAX_CHUNK_BYTES) { "payload exceeds 256KiB" }
        val out = ByteArray(1 + id.size + 8 + 8 + 8 + 4 + payload.size)
        var p = 0
        out[p++] = id.size.toByte(); id.copyInto(out, p); p += id.size
        fun putLong(value: Long) { java.nio.ByteBuffer.wrap(out, p, 8).putLong(value); p += 8 }
        putLong(epoch.toBinaryCarrier()); putLong(index); putLong(offset)
        java.nio.ByteBuffer.wrap(out, p, 4).putInt(payload.size); p += 4
        payload.copyInto(out, p)
        return out
    }
}

class ContentTransferSender(
    private val socket: DirectSocket,
    private val deviceId: () -> String,
    private val epoch: () -> LinkEpoch,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
    private val idFactory: () -> String = { UUID.randomUUID().toString() },
    private val onTransportFailure: () -> Unit = {},
) {
    private val active = LinkedHashSet<String>()
    private val controlQueue = ArrayDeque<() -> Boolean>()

    @Synchronized fun enqueueControl(send: () -> Boolean): Boolean {
        if (controlQueue.size >= 16) return false
        controlQueue.addLast(send); return true
    }

    private fun drainOneControl() {
        val control = synchronized(this) { controlQueue.removeFirstOrNull() }
        control?.invoke()
    }

    fun send(bytes: ByteArray, mimeType: String, source: String, contentId: String = idFactory()): DirectContentRef? {
        val currentEpoch = epoch()
        val transferId = idFactory()
        val hash = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
        val ref = DirectContentRef(contentId, deviceId(), currentEpoch, mimeType, bytes.size.toLong(), hash, source, nowMs() + 15 * 60 * 1000)
        val chunkCount = if (bytes.isEmpty()) 0L else (bytes.size + PHONE_CONTENT_MAX_CHUNK_BYTES - 1L) / PHONE_CONTENT_MAX_CHUNK_BYTES
        synchronized(this) { active += transferId }
        try {
            if (!socket.sendText(JsonWriter.write(jsonObject {
                put("type", "content_declare"); put("transfer_id", transferId); put("device_id", ref.deviceId); put("link_epoch", currentEpoch)
                put("content", ref.toJson()); put("chunk_bytes", PHONE_CONTENT_MAX_CHUNK_BYTES); put("chunk_count", chunkCount)
            }))) throw IllegalStateException("content declaration rejected")
            var offset = 0L; var index = 0L
            while (offset < bytes.size) {
                val end = minOf(bytes.size.toLong(), offset + PHONE_CONTENT_MAX_CHUNK_BYTES).toInt()
                if (!socket.sendBinary(DirectContentChunkCodec.encode(transferId, currentEpoch, index, offset, bytes.copyOfRange(offset.toInt(), end)))) throw IllegalStateException("content chunk rejected")
                offset = end.toLong(); index++
                drainOneControl()
            }
            if (!socket.sendText(JsonWriter.write(jsonObject {
                put("type", "content_commit"); put("transfer_id", transferId); put("link_epoch", currentEpoch)
                put("size_bytes", ref.sizeBytes); put("sha256", ref.sha256)
            }))) throw IllegalStateException("content commit rejected")
            synchronized(this) { active.remove(transferId) }
            return ref
        } catch (_: Throwable) {
            synchronized(this) { active.remove(transferId) }
            onTransportFailure()
            return null
        }
    }

    @Synchronized fun onDisconnected() { active.clear() }
    @Synchronized fun activeTransferCount(): Int = active.size
}
