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
        // java.net.URI.getHost() returns null for ws://[fe80::1%wlan0]:port (zone not RFC 3986 unless %25)
        // and for some JDKs returns null for link-local with zone. Extract raw host from authority
        // so that `cleartextHostAllowed` can strip %zone/[] itself, matching Rust's SocketAddr zone stripping.
        val rawHost = extractRawHost(endpoint, value)
        require(!rawHost.isNullOrBlank() && endpoint.userInfo == null && endpoint.fragment == null) { "invalid endpoint" }
        if (endpoint.scheme == "ws") require(cleartextHostAllowed(rawHost)) { "cleartext websocket host is not allowed" }
    }

    private fun extractRawHost(uri: URI, rawValue: String): String? {
        uri.host?.let { return it }
        // Fallback: parse authority manually for zone-containing hosts where URI.getHost() is null.
        // rawValue is like ws://[fe80::1%wlan0]:47683/phone/control or ws://10.33.239.222:47683/...
        val authority = uri.rawAuthority ?: uri.authority ?: return null
        // Authority may be userInfo@host:port — we already checked userInfo==null, so just host:port
        // Strip port: host is up to ':' or '/' or '?' or '#', but for IPv6 it's [...]:port
        val withoutUserInfo = authority.substringAfterLast('@')
        if (withoutUserInfo.startsWith("[")) {
            val end = withoutUserInfo.indexOf(']')
            if (end != -1) return withoutUserInfo.substring(1, end)
        }
        // For non-bracketed, host is up to ':' or end
        return withoutUserInfo.substringBefore(':').substringBefore('/').substringBefore('?').substringBefore('#')
    }

    private fun cleartextHostAllowed(rawHost: String): Boolean {
        // Strip IPv6 zone %wlan0 / %rndis0 and brackets.
        val withoutZone = rawHost.substringBefore('%')
        val bare = withoutZone.removePrefix("[").removeSuffix("]")
        val host = bare.lowercase()
        if (host == "localhost" || host == "127.0.0.1" || host == "::1") return true
        if (host.endsWith(".ts.net")) return true
        // Only raw IP literals may be used for cleartext; DNS names (except localhost/.ts.net) require wss.
        val isIpLiteral = host.contains('.') || host.contains(':')
        if (!isIpLiteral) return false
        // No DNS — parse literals only. InetAddress.getByName would block on DNS and is not needed here.
        // IPv4-mapped IPv6 like ::ffff:192.168.1.1 contains '.' — check inner IPv4 first.
        if (host.contains(':') && host.contains('.')) {
            val v4Part = host.substringAfterLast(':')
            val v4Octets = v4Part.split('.').mapNotNull { it.toIntOrNull() }
            if (v4Octets.size == 4 && v4Octets.all { it in 0..255 }) {
                val o0 = v4Octets[0]; val o1 = v4Octets[1]
                if (o0 == 10 || (o0 == 172 && o1 in 16..31) || (o0 == 192 && v4Octets[1] == 168) || (o0 == 169 && o1 == 254) || (o0 == 100 && o1 in 64..127)) return true
                if (o0 == 127) return true
            }
            // Fall through to IPv6 ULA/link-local check below as well.
        }
        // IPv4 literal.
        if (!host.contains(':')) {
            val octets = host.split('.').mapNotNull { it.toIntOrNull() }
            if (octets.size == 4 && octets.all { it in 0..255 }) {
                val o0 = octets[0]; val o1 = octets[1]
                return o0 == 127 || o0 == 10 || (o0 == 172 && o1 in 16..31) || (o0 == 192 && o1 == 168) || (o0 == 169 && o1 == 254) || (o0 == 100 && o1 in 64..127)
            }
            return false
        }
        // IPv6 literal — parse without DNS.
        val bytes = parseIpv6WithoutDns(host) ?: return false
        // Loopback ::1 already handled, but also covers ::ffff:127.0.0.1 etc. which we handled above.
        // fc00::/7 ULA
        val b0 = bytes[0].toInt() and 0xff
        if (b0 == 0xfc || b0 == 0xfd) return true
        // fe80::/10 link-local
        if (b0 == 0xfe && (bytes[1].toInt() and 0xc0) == 0x80) return true
        return false
    }

    // Parse an IPv6 literal (with optional ::) into 16 bytes without any DNS lookup.
    // Returns null if the string is not a valid IPv6 literal. Handles ::ffff: + IPv4 tail already stripped above.
    private fun parseIpv6WithoutDns(host: String): ByteArray? {
        // Reject if it still contains '.' (already handled as mapped) or illegal chars.
        if (host.contains('.')) return null
        // Must be hex digits and ':' only.
        if (host.any { it !in "0123456789abcdefABCDEF:" }) return null
        val parts = host.split("::", limit = 2)
        if (parts.size == 2 && host.indexOf("::") != host.lastIndexOf("::")) return null
        return try {
            if (parts.size == 1) {
                // No :: — exactly 8 hextets.
                val hextets = host.split(':')
                if (hextets.size != 8) return null
                val out = ByteArray(16)
                for (i in hextets.indices) {
                    val v = hextets[i].toIntOrNull(16) ?: return null
                    if (v !in 0..0xffff || hextets[i].isEmpty()) return null
                    out[i * 2] = (v shr 8).toByte()
                    out[i * 2 + 1] = (v and 0xff).toByte()
                }
                out
            } else {
                val left = if (parts[0].isEmpty()) emptyList() else parts[0].split(':')
                val right = if (parts[1].isEmpty()) emptyList() else parts[1].split(':')
                if (left.any { it.isEmpty() } || right.any { it.isEmpty() }) return null
                if (left.size + right.size > 7) return null
                val out = ByteArray(16)
                var idx = 0
                for (h in left) {
                    val v = h.toIntOrNull(16) ?: return null
                    if (v !in 0..0xffff) return null
                    out[idx * 2] = (v shr 8).toByte()
                    out[idx * 2 + 1] = (v and 0xff).toByte()
                    idx++
                }
                val zeros = 8 - left.size - right.size
                idx += zeros
                for (h in right) {
                    val v = h.toIntOrNull(16) ?: return null
                    if (v !in 0..0xffff) return null
                    out[idx * 2] = (v shr 8).toByte()
                    out[idx * 2 + 1] = (v and 0xff).toByte()
                    idx++
                }
                out
            }
        } catch (_: Exception) {
            null
        }
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
