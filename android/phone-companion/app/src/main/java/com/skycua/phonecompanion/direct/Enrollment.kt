package com.skycua.phonecompanion.direct

import android.util.Log
import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonWriter
import com.skycua.phonecompanion.json.jsonObject
import java.net.URI
import java.util.Base64
import java.util.UUID

/** Enrollment payload carried by the phone-control.v2 QR/deep link. */
data class EnrollmentPayload(
    val protocol: String,
    val endpoint: String,
    val enrollmentId: String,
    val bootstrapCredential: String,
    val expiresAtMs: Long,
)

object EnrollmentCodec {
    private const val SCHEME = "skycua"
    private const val HOST = "enroll"

    fun encode(payload: EnrollmentPayload): String {
        require(payload.protocol == "phone-control.v2") { "unsupported protocol" }
        require(payload.endpoint.startsWith("ws://") || payload.endpoint.startsWith("wss://")) {
            "endpoint must be a websocket URL"
        }
        validate(payload)
        require(payload.expiresAtMs > 0)
        return buildString {
            append("$SCHEME://$HOST?")
            append("protocol=").append(payload.protocol.encodeQuery())
            append("&endpoint=").append(payload.endpoint.encodeQuery())
            append("&enrollment_id=").append(payload.enrollmentId.encodeQuery())
            append("&bootstrap_credential=").append(payload.bootstrapCredential.encodeQuery())
            append("&expires_at_ms=").append(payload.expiresAtMs)
        }
    }

    fun decode(value: String, nowMs: Long? = null): EnrollmentPayload {
        val uri = URI(value.trim())
        require(uri.scheme == SCHEME && uri.host == HOST) { "invalid enrollment link" }
        val params = uri.rawQuery.orEmpty().split('&').filter { it.isNotEmpty() }.associate {
            val pieces = it.split('=', limit = 2)
            require(pieces.size == 2) { "malformed enrollment parameter" }
            java.net.URLDecoder.decode(pieces[0], Charsets.UTF_8) to java.net.URLDecoder.decode(pieces[1], Charsets.UTF_8)
        }
        val payload = EnrollmentPayload(
            protocol = params["protocol"] ?: error("missing protocol"),
            endpoint = params["endpoint"] ?: error("missing endpoint"),
            enrollmentId = params["enrollment_id"] ?: error("missing enrollment id"),
            bootstrapCredential = params["bootstrap_credential"] ?: error("missing credential"),
            expiresAtMs = params["expires_at_ms"]?.toLongOrNull() ?: error("invalid expiry"),
        )
        validate(payload)
        if (nowMs != null) require(nowMs < payload.expiresAtMs) { "enrollment expired" }
        return payload
    }

    /** Manual fallback format: endpoint, enrollment id, bootstrap credential, expiry (one per line). */
    fun decodeManual(value: String, nowMs: Long? = null): EnrollmentPayload {
        val fields = value.trim().lines().map(String::trim)
        require(fields.size == 4 && fields.none(String::isEmpty)) { "invalid manual enrollment" }
        val payload = EnrollmentPayload(PHONE_CONTROL_PROTOCOL, fields[0], fields[1], fields[2], fields[3].toLongOrNull() ?: error("invalid expiry"))
        validate(payload, nowMs); return payload
    }

    fun validate(payload: EnrollmentPayload, nowMs: Long? = null) {
        require(payload.protocol == PHONE_CONTROL_PROTOCOL) { "unsupported protocol" }
        EndpointValidator.requireAllowed(payload.endpoint)
        require(UUID.fromString(payload.enrollmentId).toString() == payload.enrollmentId) { "invalid enrollment id" }
        require(payload.expiresAtMs > 0) { "invalid expiry" }
        if (nowMs != null) require(nowMs < payload.expiresAtMs) { "enrollment expired" }
        val decoded = decodeCredential(payload.bootstrapCredential)
        require(decoded.size == 32) { "bootstrap credential must be 256-bit" }
    }

    fun decodeCredential(value: String): ByteArray {
        require(value.isNotEmpty() && value.length <= 64 && value.none { it == '=' || it.isWhitespace() })
        val bytes = Base64.getUrlDecoder().decode(value)
        require(Base64.getUrlEncoder().withoutPadding().encodeToString(bytes) == value) { "non-canonical bootstrap credential" }
        return bytes
    }

    private fun String.encodeQuery(): String = java.net.URLEncoder.encode(this, Charsets.UTF_8)
}

object EndpointValidator {
    fun requireAllowed(value: String) {
        val endpoint = URI(value)
        require(endpoint.scheme == "ws" || endpoint.scheme == "wss") { "endpoint must be a websocket URL" }
        require(!endpoint.host.isNullOrBlank() && endpoint.userInfo == null && endpoint.fragment == null) { "invalid endpoint" }
        if (endpoint.scheme == "ws") require(cleartextHostAllowed(endpoint.host.lowercase())) { "cleartext websocket host is not allowed" }
    }
    private fun cleartextHostAllowed(host: String): Boolean {
        if (host == "localhost" || host == "127.0.0.1" || host == "::1" || host.startsWith("fd7a:115c:a1e0:") || host.endsWith(".ts.net")) return true
        val octets = host.split('.').mapNotNull { it.toIntOrNull() }
        return octets.size == 4 && octets[0] == 100 && octets[1] in 64..127
    }
}

data class EnrollmentResult(val credential: DeviceCredential, val endpoint: String)
enum class EnrollmentUiState { IDLE, REVIEW_ENDPOINT, CONFIRM_REPLACE, REDEEMING, SUCCESS, ERROR }
class EnrollmentUiStateMachine(private val hasExisting: Boolean, private val reviewedFingerprint: String? = null) {
    var state: EnrollmentUiState = EnrollmentUiState.IDLE; private set
    fun review() { state = EnrollmentUiState.REVIEW_ENDPOINT }
    fun confirmEndpoint(currentFingerprint: String? = reviewedFingerprint) { state = if (hasExisting || currentFingerprint != reviewedFingerprint) EnrollmentUiState.CONFIRM_REPLACE else EnrollmentUiState.REDEEMING }
    fun confirmReplacement() { check(state == EnrollmentUiState.CONFIRM_REPLACE); state = EnrollmentUiState.REDEEMING }
    fun recheckBeforeRedeem(currentFingerprint: String?): Boolean {
        if (currentFingerprint != reviewedFingerprint && state == EnrollmentUiState.REDEEMING) { state = EnrollmentUiState.CONFIRM_REPLACE; return false }
        return true
    }
}
enum class EnrollmentFailureKind { INVALID, EXPIRED, NETWORK, PERSISTENCE, REJECTED }
sealed class EnrollmentOutcome {
    data class Success(val result: EnrollmentResult) : EnrollmentOutcome()
    data class Failure(val message: String, val kind: EnrollmentFailureKind = EnrollmentFailureKind.REJECTED) : EnrollmentOutcome()
}

/** One-shot enrollment transaction. Bootstrap material is held only in memory. */
class EnrollmentRedeemer(
    private val sockets: DirectSocketFactory,
    private val store: CredentialStore,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
    private val onPendingSaved: () -> Unit = {},
) {
    private companion object { const val TAG = "SkyEnrollment" }
    @Synchronized fun redeem(payload: EnrollmentPayload, callback: (EnrollmentOutcome) -> Unit) {
        try { EnrollmentCodec.validate(payload, nowMs()) } catch (e: Exception) { callback(EnrollmentOutcome.Failure(e.message ?: "invalid enrollment")); return }
        val socket = sockets.create()
        var finished = false
        var enrolled: DeviceCredential? = null
        fun fail(message: String, kind: EnrollmentFailureKind = EnrollmentFailureKind.REJECTED) { if (finished) return; finished = true; socket.close(); callback(EnrollmentOutcome.Failure(message, kind)) }
        socket.connect(payload.endpoint, object : DirectSocket.Listener {
            override fun onOpen() {
                if (finished) return
                val pending = store.pendingEnrollment()
                val existing = store.load()
                if (pending != null && pending.enrollmentId == payload.enrollmentId && nowMs() >= pending.expiresAtMs) {
                    store.clearPendingEnrollment(); fail("pending enrollment expired", EnrollmentFailureKind.EXPIRED); return
                }
                if (pending?.enrollmentId == payload.enrollmentId && existing != null && nowMs() < pending.expiresAtMs) {
                    enrolled = existing
                    val nonce = AuthCodec.newNonce()
                    val proof = AuthCodec.enrollmentAckProof(existing.deviceSecret, pending.enrollmentId, existing.deviceId, nonce)
                    socket.sendText(AuthCodec.encodeEnrollmentAck(pending.enrollmentId, existing.deviceId, nonce, proof))
                    return
                }
                socket.sendText(JsonWriter.write(jsonObject {
                    put("type", "enrollment_redeem"); put("protocol", payload.protocol)
                    put("enrollment_id", payload.enrollmentId); put("bootstrap_credential", payload.bootstrapCredential)
                }))
            }
            override fun onText(frame: String) {
                if (finished) return
                try {
                    val obj = JsonParser.parseObject(frame)
                    if (obj.string("type") == "enrollment_committed") {
                        val current = enrolled ?: error("commit before enrollment")
                        require(obj.string("protocol") == PHONE_CONTROL_PROTOCOL)
                        require(obj.string("enrollment_id") == payload.enrollmentId)
                        require(obj.string("device_id") == current.deviceId)
                        store.clearPendingEnrollment()
                        finished = true; socket.close(); callback(EnrollmentOutcome.Success(EnrollmentResult(current, payload.endpoint))); return
                    }
                    require(obj.string("type") == "enrollment_ok")
                    require(obj.string("protocol") == PHONE_CONTROL_PROTOCOL)
                    require(obj.string("enrollment_id") == payload.enrollmentId)
                    val id = obj.string("device_id") ?: error("missing device id")
                    AuthCodec.requireCanonicalUuid(id)
                    val secretText = obj.string("device_secret") ?: error("missing device secret")
                    val secret = EnrollmentCodec.decodeCredential(secretText)
                    require(secret.size == 32) { "device secret must be 256-bit" }
                    require(obj.long("enrolled_at_ms")?.let { it > 0 } == true)
                    val pendingExpiry = obj.long("pending_expires_at_ms") ?: error("missing pending expiry")
                    require(pendingExpiry > nowMs())
                    // Persistence is the commit point: direct auth is started by the caller only after this returns.
                    enrolled = DeviceCredential(id, secret)
                    try {
                        store.saveEnrollment(enrolled!!, payload.endpoint, PendingEnrollment(payload.enrollmentId, pendingExpiry))
                    } catch (e: Exception) {
                        Log.e(TAG, "phase=persist exception=${e::class.java.name} message=${e.message}", e)
                        fail(e.message ?: "unable to persist enrollment", EnrollmentFailureKind.PERSISTENCE); return
                    }
                    onPendingSaved()
                    val nonce = AuthCodec.newNonce()
                    val proof = AuthCodec.enrollmentAckProof(secret, payload.enrollmentId, id, nonce)
                    socket.sendText(AuthCodec.encodeEnrollmentAck(payload.enrollmentId, id, nonce, proof))
                } catch (e: Exception) {
                    Log.e(TAG, "phase=validate_or_ack exception=${e::class.java.name} message=${e.message}", e)
                    fail(e.message ?: "enrollment rejected", EnrollmentFailureKind.REJECTED)
                }
            }
            override fun onBinary(bytes: ByteArray) = fail("unexpected enrollment frame")
            override fun onClosed(cause: Throwable?) { if (!finished) fail(cause?.message ?: "enrollment connection closed") }
        })
    }
}
