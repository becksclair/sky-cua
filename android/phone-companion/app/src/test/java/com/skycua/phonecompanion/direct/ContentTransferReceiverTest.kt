package com.skycua.phonecompanion.direct

import com.skycua.phonecompanion.json.JsonParser
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class ContentTransferReceiverTest {
    @Test
    fun declaredBytesCommitOnlyAfterLengthAndDigestVerification() {
        val socket = RecordingSocket()
        val sender = ContentTransferSender(
            socket,
            { "device-1" },
            { 7 },
            idFactory = sequenceOf("content-1", "transfer-1").iterator()::next,
        )
        val payload = "hello companion".toByteArray()
        assertNotNull(sender.send(payload, "text/plain", "host_private_artifact"))

        val receiver = ContentTransferReceiver(RuntimeEnvironment.getApplication())
        val declaration = JsonParser.parseObject(socket.text.first())
        receiver.receiveControl(declaration, "device-1", 7)
        socket.binary.forEach { receiver.receiveChunk(it, 7) }
        receiver.receiveControl(JsonParser.parseObject(socket.text.last()), "device-1", 7)

        val received = receiver.resolve(declaration.obj("content")!!, 7)
        assertNotNull(received)
        assertArrayEquals(payload, received!!.file.readBytes())
        receiver.abortEpoch(7)
        assertFalse(received.file.exists())
    }

    private class RecordingSocket : DirectSocket {
        val text = mutableListOf<String>()
        val binary = mutableListOf<ByteArray>()
        override fun connect(endpoint: String, listener: DirectSocket.Listener) = Unit
        override fun sendText(frame: String): Boolean = text.add(frame)
        override fun sendBinary(bytes: ByteArray): Boolean = binary.add(bytes)
        override fun close() = Unit
    }
}
