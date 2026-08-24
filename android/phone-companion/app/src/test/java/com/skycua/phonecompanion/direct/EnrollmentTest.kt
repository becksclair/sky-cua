package com.skycua.phonecompanion.direct

import org.junit.Assert.*
import org.junit.Test
import java.util.Base64

class EnrollmentTest {
    private val id = "00000000-0000-4000-8000-000000000001"
    private val bootstrap = Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32) { 7 })
    private val secret = Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32) { 9 })
    private fun payload(expiry: Long = 2_000L) = EnrollmentPayload(PHONE_CONTROL_PROTOCOL, "ws://127.0.0.1:47683/phone/control", id, bootstrap, expiry)

    @Test fun rejectsExpiredAndNonCanonicalBootstrap() {
        expectIllegal { EnrollmentCodec.decode(EnrollmentCodec.encode(payload(10)), 10) }
        expectIllegal { EnrollmentCodec.validate(payload().copy(bootstrapCredential = bootstrap + "=")) }
    }

    @Test fun acceptsManualFourLineFallbackWithoutRenderingCredential() {
        val parsed = EnrollmentCodec.decodeManual("ws://127.0.0.1:47683/phone/control\n$id\n$bootstrap\n2000", 1000)
        assertEquals(id, parsed.enrollmentId); assertEquals(bootstrap, parsed.bootstrapCredential)
    }

    @Test fun cleartextWebSocketIsLimitedToLoopbackOrTailscale() {
        EnrollmentCodec.validate(payload())
        expectIllegal { EnrollmentCodec.validate(payload().copy(endpoint = "ws://public.example/control")) }
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://node.tailnet.ts.net/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "wss://public.example/control"))
        // Private LAN + tether now allowed for ws:// (same as Rust is_private_ip)
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://192.168.1.10:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://192.168.42.10:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://10.0.0.5:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://172.16.5.5:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://169.254.1.1:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://[fd12:3456::1]:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://[fe80::1]:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://[fe80::1%wlan0]:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://[fe80::1%rndis0]:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "ws://100.64.1.1:47683/phone/control"))
        expectIllegal { EnrollmentCodec.validate(payload().copy(endpoint = "ws://203.0.113.5:47683/phone/control")) }
        expectIllegal { EnrollmentCodec.validate(payload().copy(endpoint = "ws://172.15.0.1:47683/phone/control")) }
        expectIllegal { EnrollmentCodec.validate(payload().copy(endpoint = "ws://example.com:47683/phone/control")) }
        // wss allows public (TLS)
        EnrollmentCodec.validate(payload().copy(endpoint = "wss://203.0.113.5:47683/phone/control"))
        EnrollmentCodec.validate(payload().copy(endpoint = "wss://example.com:47683/phone/control"))
    }

    @Test fun redeemsExactlyOnceAndPersistsOnlyMatchingResult() {
        val socket = FakeSocket(); val store = MemoryCredentialStore(); val outcomes = mutableListOf<EnrollmentOutcome>()
        EnrollmentRedeemer(FakeFactory(socket), store, { 1_000L }).redeem(payload(), outcomes::add)
        socket.listener!!.onOpen(); assertTrue(socket.sent.single().contains("enrollment_redeem"))
        socket.listener!!.onText("{\"type\":\"enrollment_ok\",\"protocol\":\"phone-control.v2\",\"enrollment_id\":\"$id\",\"device_id\":\"$id\",\"device_secret\":\"$secret\",\"enrolled_at_ms\":1000,\"pending_expires_at_ms\":2000}")
        socket.listener!!.onText("{\"type\":\"enrollment_committed\",\"protocol\":\"phone-control.v2\",\"enrollment_id\":\"$id\",\"device_id\":\"$id\"}")
        assertTrue(outcomes.single() is EnrollmentOutcome.Success); assertEquals(id, store.load()!!.deviceId); assertTrue(socket.closed)
        assertEquals(payload().endpoint, store.endpoint)
        socket.listener!!.onText("{\"type\":\"enrollment_ok\",\"protocol\":\"phone-control.v2\",\"enrollment_id\":\"$id\",\"device_id\":\"$id\",\"device_secret\":\"$secret\",\"enrolled_at_ms\":1000,\"pending_expires_at_ms\":2000}")
        assertEquals(1, outcomes.size)
    }

    @Test fun rejectsReplayWrongIdentityAndNetworkFailureWithoutCredential() {
        val socket = FakeSocket(); val store = MemoryCredentialStore(); val outcomes = mutableListOf<EnrollmentOutcome>()
        EnrollmentRedeemer(FakeFactory(socket), store, { 1_000L }).redeem(payload(), outcomes::add)
        socket.listener!!.onOpen(); socket.listener!!.onText("{\"type\":\"enrollment_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"bad\",\"device_secret\":\"$secret\",\"enrolled_at_ms\":1000,\"pending_expires_at_ms\":2000}")
        assertTrue(outcomes.single() is EnrollmentOutcome.Failure); assertNull(store.load())
        val socket2 = FakeSocket(); val outcomes2 = mutableListOf<EnrollmentOutcome>()
        EnrollmentRedeemer(FakeFactory(socket2), store, { 1_000L }).redeem(payload(), outcomes2::add)
        socket2.listener!!.onClosed(java.io.IOException("offline")); assertTrue(outcomes2.single() is EnrollmentOutcome.Failure); assertNull(store.load())
    }

    @Test fun persistenceFailureClosesAndDoesNotReportSuccess() {
        val socket = FakeSocket(); val outcomes = mutableListOf<EnrollmentOutcome>()
        val failing = object : CredentialStore {
            override fun load(): DeviceCredential? = null
            override fun save(credential: DeviceCredential) = error("disk full")
            override fun saveEnrollment(credential: DeviceCredential, endpoint: String, pending: PendingEnrollment?) = error("disk full")
            override fun clear() = Unit
        }
        EnrollmentRedeemer(FakeFactory(socket), failing, { 1_000L }).redeem(payload(), outcomes::add)
        socket.listener!!.onOpen(); socket.listener!!.onText("{\"type\":\"enrollment_ok\",\"protocol\":\"phone-control.v2\",\"enrollment_id\":\"$id\",\"device_id\":\"$id\",\"device_secret\":\"$secret\",\"enrolled_at_ms\":1000,\"pending_expires_at_ms\":2000}")
        assertTrue(outcomes.single() is EnrollmentOutcome.Failure); assertEquals(EnrollmentFailureKind.PERSISTENCE, (outcomes.single() as EnrollmentOutcome.Failure).kind); assertTrue(socket.closed)
    }

    @Test fun pendingRestartRetriesAckWithoutRedeemBootstrap() {
        val store = MemoryCredentialStore().also { it.saveEnrollment(DeviceCredential(id, ByteArray(32) { 9 }), payload().endpoint, PendingEnrollment(id, 2000)) }
        val socket = FakeSocket(); EnrollmentRedeemer(FakeFactory(socket), store, { 1000L }).redeem(payload(), {})
        socket.listener!!.onOpen()
        assertEquals(1, socket.sent.count { it.contains("enrollment_ack") }); assertEquals(0, socket.sent.count { it.contains("bootstrap_credential") })
    }

    @Test fun ackUsesClientProofWireField() {
        val frame = AuthCodec.encodeEnrollmentAck(id, id, AuthCodec.newNonce(), "a".repeat(64))
        assertTrue(frame.contains("\"client_proof\"")); assertFalse(frame.contains("\"proof\""))
    }

    @Test fun missingEnrollmentIdNeverPersistsOrAcknowledges() {
        val socket = FakeSocket(); val store = MemoryCredentialStore(); val outcomes = mutableListOf<EnrollmentOutcome>()
        EnrollmentRedeemer(FakeFactory(socket), store, { 1_000L }).redeem(payload(), outcomes::add)
        socket.listener!!.onOpen(); socket.listener!!.onText("{\"type\":\"enrollment_ok\",\"protocol\":\"phone-control.v2\",\"device_id\":\"$id\",\"device_secret\":\"$secret\",\"enrolled_at_ms\":1000,\"pending_expires_at_ms\":2000}")
        assertTrue(outcomes.single() is EnrollmentOutcome.Failure); assertNull(store.load()); assertEquals(1, socket.sent.count { it.contains("enrollment_redeem") })
    }

    private class FakeFactory(private val socket: FakeSocket) : DirectSocketFactory { override fun create() = socket }
    private fun expectIllegal(block: () -> Unit) {
        try { block(); fail("expected invalid enrollment") } catch (_: IllegalArgumentException) { }
    }
    private class FakeSocket : DirectSocket {
        var listener: DirectSocket.Listener? = null; var closed = false; val sent = mutableListOf<String>()
        override fun connect(endpoint: String, listener: DirectSocket.Listener) { this.listener = listener }
        override fun sendText(frame: String): Boolean { sent += frame; return true }
        override fun sendBinary(bytes: ByteArray): Boolean = true
        override fun close() { closed = true }
    }
}
