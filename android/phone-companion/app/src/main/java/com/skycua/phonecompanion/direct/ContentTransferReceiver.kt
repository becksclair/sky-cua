package com.skycua.phonecompanion.direct

import android.content.Context
import com.skycua.phonecompanion.json.JsonValue
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.util.UUID

data class ReceivedContent(
    val contentId: String,
    val deviceId: String,
    val linkEpoch: LinkEpoch,
    val mimeType: String,
    val filename: String?,
    val sizeBytes: Long,
    val sha256: String,
    val expiresAtMs: Long?,
    val file: File,
    val source: String = "companion_blob",
)

interface DirectContentResolver {
    fun resolve(reference: JsonValue.Obj, expectedEpoch: LinkEpoch): ReceivedContent?
    fun describe(contentId: String): ReceivedContent?
    fun release(contentId: String, expectedEpoch: LinkEpoch): Boolean
    fun registerLocal(
        file: File,
        deviceId: String,
        linkEpoch: LinkEpoch,
        mimeType: String,
        filename: String?,
        source: String,
    ): ReceivedContent = error("local content registration is unavailable")
}

object DirectContentRegistry {
    private val items = LinkedHashMap<String, ReceivedContent>()
    @Synchronized fun put(content: ReceivedContent) { items[content.contentId] = content }
    @Synchronized fun get(contentId: String): ReceivedContent? = items[contentId]?.takeIf { it.file.isFile }
    @Synchronized fun remove(contentId: String) { items.remove(contentId) }
}

/** Receives finite host-to-phone transfers into private temporary files. */
class ContentTransferReceiver(
    context: Context,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
) : DirectContentResolver {
    private data class Active(
        val transferId: String,
        val content: ReceivedContent,
        val temporary: File,
        val output: FileOutputStream,
        val digest: MessageDigest,
        val chunkBytes: Int,
        val chunkCount: Long,
        var nextIndex: Long = 0,
        var nextOffset: Long = 0,
    )

    private val root = File(context.noBackupFilesDir, "direct-content").apply { mkdirs() }
    private val active = LinkedHashMap<String, Active>()
    private val committed = LinkedHashMap<String, ReceivedContent>()

    @Synchronized
    override fun registerLocal(
        file: File,
        deviceId: String,
        linkEpoch: LinkEpoch,
        mimeType: String,
        filename: String?,
        source: String,
    ): ReceivedContent {
        cleanupExpired()
        require(file.isFile) { "captured content file is unavailable" }
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                digest.update(buffer, 0, read)
            }
        }
        val target = File(root, UUID.randomUUID().toString())
        if (!file.renameTo(target)) {
            file.copyTo(target)
            file.delete()
        }
        val content = ReceivedContent(
            UUID.randomUUID().toString(),
            deviceId,
            linkEpoch,
            mimeType,
            filename,
            target.length(),
            digest.digest().joinToString("") { "%02x".format(it) },
            nowMs() + 15 * 60 * 1000,
            target,
            source,
        )
        committed[content.contentId] = content
        DirectContentRegistry.put(content)
        return content
    }

    @Synchronized
    fun receiveControl(frame: JsonValue.Obj, authenticatedDeviceId: String, epoch: LinkEpoch): Boolean {
        cleanupExpired()
        return when (frame.string("type")) {
            "content_declare" -> { declare(frame, authenticatedDeviceId, epoch); true }
            "content_commit" -> { commit(frame, epoch); true }
            "content_abort" -> { abort(frame.string("transfer_id") ?: error("missing transfer_id")); true }
            else -> false
        }
    }

    @Synchronized
    fun receiveChunk(bytes: ByteArray, epoch: LinkEpoch) {
        require(bytes.isNotEmpty()) { "binary chunk is empty" }
        val idLength = bytes[0].toInt() and 0xff
        val headerLength = 1 + idLength + 8 + 8 + 8 + 4
        require(idLength > 0 && bytes.size >= headerLength) { "binary chunk header is invalid" }
        val transferId = bytes.copyOfRange(1, 1 + idLength).toString(Charsets.UTF_8)
        val buffer = ByteBuffer.wrap(bytes, 1 + idLength, bytes.size - 1 - idLength)
        val chunkEpoch = LinkEpoch.fromBinaryCarrier(buffer.long)
        val index = buffer.long
        val offset = buffer.long
        val length = buffer.int
        require(chunkEpoch == epoch) { "binary chunk epoch mismatch" }
        require(length in 0..PHONE_CONTENT_MAX_CHUNK_BYTES && bytes.size == headerLength + length) { "binary chunk length mismatch" }
        val transfer = active[transferId] ?: error("binary chunk has no declaration")
        require(index == transfer.nextIndex && offset == transfer.nextOffset) { "binary chunks must be sequential" }
        require(length <= transfer.chunkBytes && offset + length <= transfer.content.sizeBytes) { "binary chunk exceeds declaration" }
        transfer.output.write(bytes, headerLength, length)
        transfer.digest.update(bytes, headerLength, length)
        transfer.nextIndex++
        transfer.nextOffset += length
    }

    @Synchronized
    override fun resolve(reference: JsonValue.Obj, expectedEpoch: LinkEpoch): ReceivedContent? {
        cleanupExpired()
        val contentId = reference.string("content_id") ?: return null
        val item = committed[contentId] ?: return null
        if (item.linkEpoch != expectedEpoch || reference.linkEpoch("link_epoch") != expectedEpoch) return null
        if (reference.long("size_bytes") != item.sizeBytes || reference.string("sha256") != item.sha256) return null
        return item.takeIf { it.file.isFile }
    }

    @Synchronized
    override fun describe(contentId: String): ReceivedContent? {
        cleanupExpired()
        return committed[contentId]?.takeIf { it.file.isFile }
    }

    @Synchronized
    override fun release(contentId: String, expectedEpoch: LinkEpoch): Boolean {
        cleanupExpired()
        val content = committed[contentId]?.takeIf { it.linkEpoch == expectedEpoch } ?: return false
        committed.remove(contentId)
        DirectContentRegistry.remove(contentId)
        return content.file.delete() || !content.file.exists()
    }

    @Synchronized
    fun abortEpoch(epoch: LinkEpoch) {
        active.values.filter { it.content.linkEpoch == epoch }.map { it.transferId }.forEach(::abort)
        committed.values.filter { it.linkEpoch == epoch }.forEach { DirectContentRegistry.remove(it.contentId); it.file.delete() }
        committed.entries.removeAll { it.value.linkEpoch == epoch }
    }

    private fun declare(frame: JsonValue.Obj, authenticatedDeviceId: String, epoch: LinkEpoch) {
        val transferId = frame.string("transfer_id") ?: error("missing transfer_id")
        require(transferId.toByteArray().size in 1..255 && transferId !in active) { "invalid or duplicate transfer_id" }
        require(frame.string("device_id") == authenticatedDeviceId && frame.linkEpoch("link_epoch") == epoch) { "content declaration identity mismatch" }
        val content = frame.obj("content") ?: error("missing content")
        val contentId = content.string("content_id") ?: error("missing content_id")
        require(content.string("device_id") == authenticatedDeviceId && content.linkEpoch("link_epoch") == epoch) { "content identity mismatch" }
        val size = content.long("size_bytes") ?: error("missing size_bytes")
        require(size >= 0) { "invalid content size" }
        val sha = content.string("sha256") ?: error("missing sha256")
        require(sha.length == 64 && sha.all { it in '0'..'9' || it in 'a'..'f' }) { "invalid sha256" }
        val chunkBytes = frame.int("chunk_bytes") ?: error("missing chunk_bytes")
        require(chunkBytes in 1..PHONE_CONTENT_MAX_CHUNK_BYTES) { "invalid chunk_bytes" }
        val chunkCount = frame.long("chunk_count") ?: error("missing chunk_count")
        val expectedChunks = if (size == 0L) 0L else 1L + (size - 1L) / chunkBytes
        require(chunkCount == expectedChunks) { "chunk_count does not match size" }
        committed.remove(contentId)?.let { DirectContentRegistry.remove(it.contentId); it.file.delete() }
        val temporary = File(root, "${UUID.randomUUID()}.part")
        val final = File(root, UUID.randomUUID().toString())
        val received = ReceivedContent(
            contentId,
            authenticatedDeviceId,
            epoch,
            content.string("mime_type") ?: "application/octet-stream",
            content.string("filename"),
            size,
            sha,
            content.long("expires_at_ms"),
            final,
            content.string("source") ?: "companion_blob",
        )
        active[transferId] = Active(
            transferId, received, temporary, FileOutputStream(temporary),
            MessageDigest.getInstance("SHA-256"), chunkBytes, chunkCount,
        )
    }

    private fun commit(frame: JsonValue.Obj, epoch: LinkEpoch) {
        val transferId = frame.string("transfer_id") ?: error("missing transfer_id")
        val transfer = active.remove(transferId) ?: error("commit has no declaration")
        try {
            require(frame.linkEpoch("link_epoch") == epoch && transfer.content.linkEpoch == epoch) { "commit epoch mismatch" }
            require(frame.long("size_bytes") == transfer.content.sizeBytes && frame.string("sha256") == transfer.content.sha256) { "commit metadata mismatch" }
            require(transfer.nextOffset == transfer.content.sizeBytes && transfer.nextIndex == transfer.chunkCount) { "content transfer is incomplete" }
            transfer.output.fd.sync()
            transfer.output.close()
            val actual = transfer.digest.digest().joinToString("") { "%02x".format(it) }
            require(actual == transfer.content.sha256) { "content digest mismatch" }
            require(transfer.temporary.renameTo(transfer.content.file)) { "content commit rename failed" }
            committed[transfer.content.contentId] = transfer.content
            DirectContentRegistry.put(transfer.content)
        } catch (error: Throwable) {
            runCatching { transfer.output.close() }
            transfer.temporary.delete()
            throw error
        }
    }

    private fun abort(transferId: String) {
        active.remove(transferId)?.let {
            runCatching { it.output.close() }
            it.temporary.delete()
        }
    }

    private fun cleanupExpired() {
        val expired = committed.values.filter { it.expiresAtMs?.let { expiry -> nowMs() >= expiry } == true }
        expired.forEach { DirectContentRegistry.remove(it.contentId); it.file.delete() }
        committed.entries.removeAll { it.value in expired }
    }
}
