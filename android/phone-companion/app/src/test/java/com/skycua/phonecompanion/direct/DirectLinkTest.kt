package com.skycua.phonecompanion.direct

import org.junit.Assert.*
import org.junit.Test
import java.util.Base64
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

class DirectLinkTest {
    private val deviceId = "00000000-0000-4000-8000-000000000001"
    private val serverNonce = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8"
    private val clientNonce = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
    private val secret = "fixture-secret".toByteArray()

    @Test fun canonicalAuthProofBindsFixtureFields() {
        val saga = AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, clientNonce, "4", "saga")
        assertEquals("64b9f650ea1ac59efb27c08eeac37ce778214104ebf4b23e2cfcf4b86fa53827", saga)
        assertEquals("545f1d44c6928b6928378dada16f70ce11f87d06156a747dd4bf38171f18ab18", AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, clientNonce, "4", "companion"))
        val challenge = AuthChallenge(PHONE_CONTROL_PROTOCOL, serverNonce, "4", saga)
        assertTrue(AuthCodec.verifyServerProof(secret, deviceId, challenge, clientNonce))
        assertFalse(AuthCodec.verifyServerProof(secret, deviceId, challenge.copy(linkEpoch = "5"), clientNonce))
        assertFalse(AuthCodec.verifyServerProof(secret, deviceId, challenge, clientNonce.dropLast(1) + "A"))
    }

    @Test fun controllerSendsHelloAndReachesConnectedOnlyAfterAuthOk() {
        val socket = FakeSocket()
        val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)) }
        val controller = DirectLinkController(FakeFactory(socket), store).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        socket.listener!!.onOpen()
        assertEquals(LinkState.AUTHENTICATING, controller.snapshot().state)
        val hello = socket.sentText.single()
        val clientNonce = Regex("\\\"client_nonce\\\":\\\"([^\\\"]+)\\\"").find(hello)!!.groupValues[1]
        val serverProof = AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, clientNonce, "7", "saga")
        socket.listener!!.onText("{\"type\":\"auth_challenge\",\"protocol\":\"phone-control.v2\",\"server_nonce\":\"$serverNonce\",\"link_epoch\":\"7\",\"server_proof\":\"$serverProof\"}")
        assertEquals(LinkState.AUTHENTICATING, controller.snapshot().state)
        socket.listener!!.onText("{\"type\":\"auth_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$deviceId\",\"link_epoch\":\"7\"}")
        assertEquals(LinkState.CONNECTED, controller.snapshot().state)
        assertEquals("7", store.lastAcceptedEpoch())
    }

    @Test fun authenticatedLinkPublishesCurrentCapabilitiesAtItsEpoch() {
        val socket = FakeSocket()
        val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)) }
        val controller = DirectLinkController(FakeFactory(socket), store)
        controller.updateCapabilities(setOf("storage", "screenshot"))
        controller.configure("ws://127.0.0.1:1"); controller.connect(); socket.listener!!.onOpen()
        val nonce = Regex("\\\"client_nonce\\\":\\\"([^\\\"]+)\\\"").find(socket.sentText.single())!!.groupValues[1]
        val proof = AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, nonce, "7", "saga")
        socket.listener!!.onText("{\"type\":\"auth_challenge\",\"protocol\":\"phone-control.v2\",\"server_nonce\":\"$serverNonce\",\"link_epoch\":\"7\",\"server_proof\":\"$proof\"}")
        socket.listener!!.onText("{\"type\":\"auth_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$deviceId\",\"link_epoch\":\"7\"}")
        val event = socket.sentText.last()
        assertTrue(event.contains("\"event\":\"capability_changed\""))
        assertTrue(event.contains("\"link_epoch\":7"))
        assertTrue(event.contains("screenshot")); assertTrue(event.contains("storage"))
    }

    @Test fun rejectsStaleEpochBadProofAndPreAuthBinary() {
        val socket = FakeSocket(); val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)); it.saveAcceptedEpoch("7") }
        val controller = DirectLinkController(FakeFactory(socket), store, nowMs = { 0 }).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        socket.listener!!.onOpen(); socket.listener!!.onBinary(byteArrayOf(1)); assertEquals(LinkState.BACKOFF, controller.snapshot().state)
    }

    @Test fun authOkBeforeChallengeIsRejected() {
        val socket = FakeSocket(); val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)) }
        val controller = DirectLinkController(FakeFactory(socket), store, nowMs = { 0 }).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        socket.listener!!.onOpen()
        socket.listener!!.onText("{\"type\":\"auth_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$deviceId\",\"link_epoch\":\"1\"}")
        assertEquals(LinkState.BACKOFF, controller.snapshot().state)
    }

    @Test fun staleBinaryFromPriorGenerationCannotFenceNewConnection() {
        val socket = FakeSocket(); val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)) }
        val controller = DirectLinkController(FakeFactory(socket), store, nowMs = { 0 }).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        val old = socket.listener!!
        controller.close(); controller.connect()
        assertEquals(LinkState.CONNECTING, controller.snapshot().state)
        old.onBinary(byteArrayOf(1))
        assertEquals(LinkState.CONNECTING, controller.snapshot().state)
    }

    @Test fun acceptsUnsignedMaxEpochAndBindsItToCredential() {
        val socket = FakeSocket(); val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)) }
        val controller = DirectLinkController(FakeFactory(socket), store).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        socket.listener!!.onOpen()
        val nonce = Regex("\\\"client_nonce\\\":\\\"([^\\\"]+)\\\"").find(socket.sentText.single())!!.groupValues[1]
        val epoch = "18446744073709551615"
        val proof = AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, nonce, epoch, "saga")
        socket.listener!!.onText("{\"type\":\"auth_challenge\",\"protocol\":\"phone-control.v2\",\"server_nonce\":\"$serverNonce\",\"link_epoch\":\"$epoch\",\"server_proof\":\"$proof\"}")
        socket.listener!!.onText("{\"type\":\"auth_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$deviceId\",\"link_epoch\":\"$epoch\"}")
        assertEquals(LinkState.CONNECTED, controller.snapshot().state)
        assertEquals(epoch, store.lastAcceptedEpoch())
        assertTrue(socket.sentText.any { it.contains("\"link_epoch\":$epoch") })
    }

    @Test fun credentialReplacementResetsAcceptedEpoch() {
        val store = MemoryCredentialStore()
        store.save(DeviceCredential(deviceId, secret)); store.saveAcceptedEpoch("99")
        store.save(DeviceCredential("00000000-0000-4000-8000-000000000002", ByteArray(32) { 1 }))
        assertEquals("0", store.lastAcceptedEpoch())
    }

    @Test fun credentialReplacementClosesOldLinkAndNextHelloUsesNewDevice() {
        val socket = FakeSocket(); val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)) }
        val controller = DirectLinkController(FakeFactory(socket), store).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        socket.listener!!.onOpen()
        assertTrue(socket.sentText.last().contains(deviceId))
        val oldNonce = Regex("\\\"client_nonce\\\":\\\"([^\\\"]+)\\\"").find(socket.sentText.last())!!.groupValues[1]
        val oldProof = AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, oldNonce, "7", "saga")
        socket.listener!!.onText("{\"type\":\"auth_challenge\",\"protocol\":\"phone-control.v2\",\"server_nonce\":\"$serverNonce\",\"link_epoch\":\"7\",\"server_proof\":\"$oldProof\"}")
        socket.listener!!.onText("{\"type\":\"auth_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$deviceId\",\"link_epoch\":\"7\"}")
        assertEquals("7", controller.snapshot().linkEpoch)
        val replacement = "00000000-0000-4000-8000-000000000002"
        val replacementSecret = ByteArray(32) { 2 }
        DirectLinkReplacementNotifier.register {
            controller.reconnectForCredentialReplacement("ws://127.0.0.1:2")
        }
        store.saveEnrollment(DeviceCredential(replacement, replacementSecret), "ws://127.0.0.1:2")
        assertTrue(socket.closed)
        socket.listener!!.onOpen()
        assertTrue(socket.sentText.last().contains(replacement))
        assertEquals("0", controller.snapshot().linkEpoch)
        val replacementNonce = Regex("\\\"client_nonce\\\":\\\"([^\\\"]+)\\\"").find(socket.sentText.last())!!.groupValues[1]
        val replacementProof = AuthCodec.prove(replacementSecret, PHONE_CONTROL_PROTOCOL, replacement, serverNonce, replacementNonce, "1", "saga")
        socket.listener!!.onText("{\"type\":\"auth_challenge\",\"protocol\":\"phone-control.v2\",\"server_nonce\":\"$serverNonce\",\"link_epoch\":\"1\",\"server_proof\":\"$replacementProof\"}")
        assertTrue(socket.sentText.last().contains("\"link_epoch\":\"1\""))
        DirectLinkReplacementNotifier.register(null)
    }

    @Test fun controllerIndependentControlUsesBulkPriorityPath() {
        val socket = FakeSocket(); val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)) }
        val controller = DirectLinkController(FakeFactory(socket), store).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        socket.listener!!.onOpen()
        val nonce = Regex("\\\"client_nonce\\\":\\\"([^\\\"]+)\\\"").find(socket.sentText.single())!!.groupValues[1]
        val proof = AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, nonce, "7", "saga")
        socket.listener!!.onText("{\"type\":\"auth_challenge\",\"protocol\":\"phone-control.v2\",\"server_nonce\":\"$serverNonce\",\"link_epoch\":\"7\",\"server_proof\":\"$proof\"}")
        socket.listener!!.onText("{\"type\":\"auth_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$deviceId\",\"link_epoch\":\"7\"}")
        val firstChunk = CountDownLatch(1); val resume = CountDownLatch(1); var chunks = 0
        socket.onBinary = { chunks++; if (chunks == 1) { firstChunk.countDown(); assertTrue(resume.await(2, TimeUnit.SECONDS)) } }
        val sender = controller.contentSender()!!
        val transfer = Thread { sender.send(ByteArray(PHONE_CONTENT_MAX_CHUNK_BYTES + 1), "x", "s") }.also { it.start() }
        assertTrue(firstChunk.await(2, TimeUnit.SECONDS)); assertTrue(controller.sendIndependentControl("independent")); resume.countDown(); transfer.join(2_000)
        assertTrue(socket.sentText.indexOf("independent") >= 0)
    }

    @Test fun persistenceFailureNeverEntersConnected() {
        val socket = FakeSocket(); val store = FailingEpochStore(DeviceCredential(deviceId, secret))
        val controller = DirectLinkController(FakeFactory(socket), store).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        socket.listener!!.onOpen()
        val nonce = Regex("\\\"client_nonce\\\":\\\"([^\\\"]+)\\\"").find(socket.sentText.single())!!.groupValues[1]
        val proof = AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, nonce, "8", "saga")
        socket.listener!!.onText("{\"type\":\"auth_challenge\",\"protocol\":\"phone-control.v2\",\"server_nonce\":\"$serverNonce\",\"link_epoch\":\"8\",\"server_proof\":\"$proof\"}")
        socket.listener!!.onText("{\"type\":\"auth_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$deviceId\",\"link_epoch\":\"8\"}")
        assertEquals(LinkState.BACKOFF, controller.snapshot().state)
    }

    @Test fun slowPreviewDoesNotBlockControlAndPendingPreviewIsLatestFrameWins() {
        val socket = FakeSocket()
        val store = MemoryCredentialStore().also { it.save(DeviceCredential(deviceId, secret)) }
        val firstPreviewStarted = CountDownLatch(1)
        val releaseFirstPreview = CountDownLatch(1)
        val controlHandled = CountDownLatch(1)
        val newestPreviewHandled = CountDownLatch(1)
        val previewCalls = AtomicInteger()
        val dispatcher = DirectRequestHandler { frame, _, _, _ ->
            val requestId = Regex("\\\"request_id\\\":\\\"([^\\\"]+)\\\"").find(frame)!!.groupValues[1]
            if (frame.contains("\"operation\":\"preview_frame\"")) {
                if (previewCalls.incrementAndGet() == 1) {
                    firstPreviewStarted.countDown()
                    assertTrue(releaseFirstPreview.await(2, TimeUnit.SECONDS))
                } else {
                    newestPreviewHandled.countDown()
                }
            } else {
                controlHandled.countDown()
            }
            "{\"type\":\"response\",\"request_id\":\"$requestId\",\"device_id\":\"$deviceId\",\"link_epoch\":7,\"result\":{}}"
        }
        val executor = Executors.newFixedThreadPool(2)
        try {
            val controller = DirectLinkController(
                FakeFactory(socket),
                store,
                requestDispatcher = dispatcher,
                requestExecutor = { executor.execute(it) },
            ).also { it.configure("ws://127.0.0.1:1"); it.connect() }
            socket.listener!!.onOpen()
            val nonce = Regex("\\\"client_nonce\\\":\\\"([^\\\"]+)\\\"").find(socket.sentText.single())!!.groupValues[1]
            val proof = AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, deviceId, serverNonce, nonce, "7", "saga")
            socket.listener!!.onText("{\"type\":\"auth_challenge\",\"protocol\":\"phone-control.v2\",\"server_nonce\":\"$serverNonce\",\"link_epoch\":\"7\",\"server_proof\":\"$proof\"}")
            socket.listener!!.onText("{\"type\":\"auth_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$deviceId\",\"link_epoch\":\"7\"}")
            fun request(id: String, method: String, params: String) =
                "{\"type\":\"request\",\"request_id\":\"$id\",\"device_id\":\"$deviceId\",\"link_epoch\":7,\"idempotent\":true,\"expires_at_ms\":9999999999999,\"method\":\"$method\",\"params\":$params}"
            socket.listener!!.onText(request("preview-1", "camera", "{\"operation\":\"preview_frame\",\"camera_session_id\":\"camera-1\"}"))
            assertTrue(firstPreviewStarted.await(2, TimeUnit.SECONDS))
            socket.listener!!.onText(request("preview-2", "camera", "{\"operation\":\"preview_frame\",\"camera_session_id\":\"camera-1\"}"))
            socket.listener!!.onText(request("preview-3", "camera", "{\"operation\":\"preview_frame\",\"camera_session_id\":\"camera-1\"}"))
            socket.listener!!.onText(request("control", "companion.status", "{}"))
            assertTrue(controlHandled.await(2, TimeUnit.SECONDS))
            assertTrue(socket.sentText.any { it.contains("preview-2") && it.contains("preview_superseded") })
            releaseFirstPreview.countDown()
            assertTrue(newestPreviewHandled.await(2, TimeUnit.SECONDS))
            assertEquals(2, previewCalls.get())
        } finally {
            releaseFirstPreview.countDown()
            executor.shutdownNow()
        }
    }

    @Test fun expiredPendingTerminalStateSurvivesSocketClose() {
        val socket = FakeSocket(); val store = MemoryCredentialStore().also {
            it.saveEnrollment(DeviceCredential(deviceId, ByteArray(32) { 1 }), "ws://127.0.0.1:1", PendingEnrollment("00000000-0000-4000-8000-000000000002", 10))
        }
        val controller = DirectLinkController(FakeFactory(socket), store, nowMs = { 100 }).also { it.configure("ws://127.0.0.1:1"); it.connect() }
        socket.listener!!.onOpen(); socket.listener!!.onClosed(null)
        assertEquals(LinkState.REENROLL_REQUIRED, controller.snapshot().state); assertNull(store.load()); assertNull(store.pendingEnrollment()); assertTrue(socket.closed)
    }

    private class FakeFactory(private val socket: FakeSocket) : DirectSocketFactory { override fun create() = socket }
    private class FakeSocket : DirectSocket {
        var listener: DirectSocket.Listener? = null
        var closed = false
        var onBinary: ((ByteArray) -> Unit)? = null
        val sentText: MutableList<String> = Collections.synchronizedList(mutableListOf())
        override fun connect(endpoint: String, listener: DirectSocket.Listener) { this.listener = listener }
        override fun sendText(frame: String): Boolean { sentText += frame; return true }
        override fun sendBinary(bytes: ByteArray): Boolean { onBinary?.invoke(bytes); return true }
        override fun close() { closed = true }
    }
    private class FailingEpochStore(private var value: DeviceCredential) : CredentialStore {
        override fun load() = value
        override fun save(credential: DeviceCredential) { value = credential }
        override fun clear() {}
        override fun saveAcceptedEpoch(epoch: String) { error("disk full") }
    }
}
