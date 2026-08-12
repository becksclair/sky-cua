package com.skycua.phonecompanion.direct

import org.junit.Assert.*
import org.junit.Test
import java.security.MessageDigest
import com.skycua.phonecompanion.json.JsonParser
import java.io.File

class ContentTransferTest {
    @Test fun goldenSmallFirstChunkMatchesFrozenFixture() {
        val bytes = DirectContentChunkCodec.encode("t1", LinkEpoch.of(1), 0, 0, byteArrayOf(1, 2, 3))
        val encoded = bytes.joinToString("") { "%02x".format(it) }
        assertEquals("02743100000000000000010000000000000000000000000000000000000003010203", encoded)
        val fixture = File("../../docs/runtime/fixtures/phone-control-v2-content-chunks.json")
        if (fixture.isFile) assertTrue(fixture.readText().contains(encoded))
    }

    @Test fun unsignedMaxEpochUsesAllOneBitsInBinaryChunkHeader() {
        val bytes = DirectContentChunkCodec.encode(
            "t1",
            LinkEpoch.parseCanonical("18446744073709551615"),
            0,
            0,
            byteArrayOf(1),
        )
        assertEquals("ffffffffffffffff", bytes.copyOfRange(3, 11).joinToString("") { "%02x".format(it) })
        assertEquals(
            LinkEpoch.parseCanonical("18446744073709551615"),
            LinkEpoch.fromBinaryCarrier(java.nio.ByteBuffer.wrap(bytes, 3, 8).long),
        )
    }

    @Test fun senderDeclaresChunksInOrderAndCommitsShaAndLength() {
        val socket = FakeSocket()
        val sender = ContentTransferSender(socket, { "device" }, { LinkEpoch.of(4) }, idFactory = sequenceOf("content", "transfer").iterator()::next)
        val payload = ByteArray(PHONE_CONTENT_MAX_CHUNK_BYTES + 3) { (it and 0xff).toByte() }
        val ref = sender.send(payload, "application/octet-stream", "screenshot")!!
        assertEquals(payload.size.toLong(), ref.sizeBytes)
        assertTrue(setOf("companion_blob", "host_private_artifact", "shared_path", "media_store", "saf", "content_uri", "clipboard", "screenshot", "camera_photo", "camera_video", "camera_preview").contains(ref.source))
        assertEquals(MessageDigest.getInstance("SHA-256").digest(payload).joinToString("") { "%02x".format(it) }, ref.sha256)
        assertEquals(2, socket.binary.size)
        assertTrue(socket.text[0].contains("content_declare")); assertTrue(socket.text[1].contains("content_commit")); assertTrue(socket.text[1].contains("size_bytes")); assertTrue(socket.text[1].contains("sha256"))
        assertEquals(0, sender.activeTransferCount())
    }

    @Test fun disconnectClearsActiveTransferAfterSendFailure() {
        val socket = FakeSocket(failBinary = true)
        val sender = ContentTransferSender(socket, { "device" }, { LinkEpoch.of(1) }, idFactory = { "t" })
        assertNull(sender.send(byteArrayOf(1), "x", "s")); sender.onDisconnected(); assertEquals(0, sender.activeTransferCount())
        assertFalse(socket.text.any { it.contains("content_commit") })
    }

    @Test fun commitFrameCarriesFrozenIdentityLengthAndDigestFields() {
        val socket = FakeSocket()
        val sender = ContentTransferSender(socket, { "d1" }, { LinkEpoch.of(1) }, idFactory = { "t1" })
        sender.send(byteArrayOf(1, 2, 3), "application/octet-stream", "blob", "c1")
        val commit = JsonParser.parseObject(socket.text.last())
        assertEquals("content_commit", commit.string("type")); assertEquals("t1", commit.string("transfer_id")); assertEquals(1L, commit.long("link_epoch")); assertEquals(3L, commit.long("size_bytes")); assertEquals("039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81", commit.string("sha256")); assertNull(commit.string("device_id")); assertNull(commit.string("content_id"))
    }

    @Test fun rejectedDeclareChunkOrCommitNeverReportsRef() {
        listOf(FakeSocket(failTextAt = 0), FakeSocket(failBinary = true), FakeSocket(failTextAt = 1)).forEach { socket ->
            val sender = ContentTransferSender(socket, { "device" }, { LinkEpoch.of(1) }, idFactory = { "t" })
            assertNull(sender.send(byteArrayOf(1, 2, 3), "x", "s")); assertEquals(0, sender.activeTransferCount())
        }
    }

    @Test fun queuedControlInterleavesAfterAtMostOneBulkChunkAndCapacityIsBounded() {
        val socket = FakeSocket(); val sender = ContentTransferSender(socket, { "d" }, { LinkEpoch.of(1) }, idFactory = sequenceOf("c", "t").iterator()::next)
        repeat(16) { assertTrue(sender.enqueueControl { socket.sendText("control$it") }) }
        assertFalse(sender.enqueueControl { true })
        sender.send(ByteArray(PHONE_CONTENT_MAX_CHUNK_BYTES + 1), "x", "s")
        assertTrue(socket.text.indexOfFirst { it == "control0" } >= 0); assertTrue(socket.binary.isNotEmpty())
    }

    private class FakeSocket(private val failBinary: Boolean = false, private val failTextAt: Int? = null) : DirectSocket {
        val text = mutableListOf<String>(); val binary = mutableListOf<ByteArray>()
        override fun connect(endpoint: String, listener: DirectSocket.Listener) = Unit
        override fun sendText(frame: String): Boolean { val index = text.size; text += frame; return index != failTextAt }
        override fun sendBinary(bytes: ByteArray): Boolean { if (failBinary) return false else binary += bytes; return true }
        override fun close() = Unit
    }
}
