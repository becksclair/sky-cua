package com.skycua.phonecompanion.direct

import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonWriter
import com.skycua.phonecompanion.json.jsonObject
import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import java.util.Base64
import java.util.UUID
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

const val PHONE_CONTROL_PROTOCOL = "phone-control.v2"

data class AuthHello(val protocol: String, val deviceId: String, val clientNonce: String)
data class AuthChallenge(val protocol: String, val serverNonce: String, val linkEpoch: String, val serverProof: String)
data class AuthProof(val linkEpoch: String, val clientProof: String)
data class AuthAccepted(val protocol: String, val deviceId: String, val linkEpoch: String)
data class EnrollmentCommitted(val protocol: String, val enrollmentId: String, val deviceId: String)

sealed interface ControlFrame {
    data class Challenge(val value: AuthChallenge) : ControlFrame
    data class Accepted(val value: AuthAccepted) : ControlFrame
    data class Error(val code: String) : ControlFrame
    data class Other(val type: String) : ControlFrame
    data class EnrollmentCommitted(val value: com.skycua.phonecompanion.direct.EnrollmentCommitted) : ControlFrame
}

object ControlFrameCodec {
    fun decode(text: String): ControlFrame {
        val obj = JsonParser.parseObject(text)
        return when (val type = obj.string("type") ?: error("missing frame type")) {
            "enrollment_committed" -> ControlFrame.EnrollmentCommitted(EnrollmentCommitted(obj.string("protocol") ?: error("missing protocol"), obj.string("enrollment_id") ?: error("missing enrollment id"), obj.string("device_id") ?: error("missing device id")))
            "auth_challenge" -> ControlFrame.Challenge(AuthChallenge(
                protocol = obj.string("protocol") ?: error("missing protocol"),
                serverNonce = obj.string("server_nonce") ?: error("missing server nonce"),
                linkEpoch = obj.string("link_epoch") ?: error("missing epoch"),
                serverProof = obj.string("server_proof") ?: error("missing server proof"),
            ))
            "auth_ok" -> ControlFrame.Accepted(AuthAccepted(
                protocol = obj.string("protocol") ?: error("missing protocol"),
                deviceId = obj.string("device_id") ?: error("missing device id"),
                linkEpoch = obj.string("link_epoch") ?: error("missing epoch"),
            ))
            "error" -> ControlFrame.Error(obj.string("code") ?: "unknown")
            else -> ControlFrame.Other(type)
        }
    }
}

object AuthCodec {
    fun newNonce(): String = Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32).also { java.security.SecureRandom().nextBytes(it) })

    fun encodeHello(hello: AuthHello): String = JsonWriter.write(jsonObject {
        put("type", "auth_hello"); put("protocol", hello.protocol); put("device_id", hello.deviceId); put("client_nonce", hello.clientNonce)
    })

    fun encodeProof(proof: AuthProof): String = JsonWriter.write(jsonObject {
        put("type", "auth_proof"); put("link_epoch", proof.linkEpoch); put("client_proof", proof.clientProof)
    })

    fun encodeEnrollmentAck(enrollmentId: String, deviceId: String, clientNonce: String, proof: String): String = JsonWriter.write(jsonObject {
        put("type", "enrollment_ack"); put("protocol", PHONE_CONTROL_PROTOCOL); put("enrollment_id", enrollmentId); put("device_id", deviceId); put("client_nonce", clientNonce); put("client_proof", proof)
    })
    fun enrollmentAckProof(secret: ByteArray, enrollmentId: String, deviceId: String, clientNonce: String): String {
        requireCanonicalUuid(deviceId); requireCanonicalNonce(clientNonce)
        return hmac(secret, canonicalFields(PHONE_CONTROL_PROTOCOL, "enrollment_ack", enrollmentId, deviceId, clientNonce))
    }

    fun prove(secret: ByteArray, protocol: String, deviceId: String, serverNonce: String, clientNonce: String, linkEpoch: String, role: String): String {
        require(protocol == PHONE_CONTROL_PROTOCOL)
        requireCanonicalUuid(deviceId); requireCanonicalNonce(serverNonce); requireCanonicalNonce(clientNonce); requireCanonicalEpoch(linkEpoch)
        require(role == "companion" || role == "saga")
        return hmac(secret, canonicalFields(protocol, deviceId, serverNonce, clientNonce, linkEpoch, role))
    }

    fun verifyServerProof(secret: ByteArray, deviceId: String, challenge: AuthChallenge, clientNonce: String): Boolean = try {
        constantTimeEquals(challenge.serverProof, prove(secret, challenge.protocol, deviceId, challenge.serverNonce, clientNonce, challenge.linkEpoch, "saga"))
    } catch (_: IllegalArgumentException) { false }

    fun verifyAuthOk(deviceId: String, challenge: AuthChallenge, accepted: AuthAccepted): Boolean =
        accepted.protocol == PHONE_CONTROL_PROTOCOL && accepted.deviceId == deviceId && accepted.linkEpoch == challenge.linkEpoch

    fun requireCanonicalUuid(value: String) {
        require(UUID.fromString(value).toString() == value) { "non-canonical device id" }
    }
    fun requireCanonicalEpoch(value: String) {
        LinkEpoch.parseCanonical(value)
    }
    fun requireCanonicalNonce(value: String) {
        require(Base64.getUrlDecoder().decode(value).size == 32) { "nonce must decode to 32 bytes" }
        require(Base64.getUrlEncoder().withoutPadding().encodeToString(Base64.getUrlDecoder().decode(value)) == value) { "non-canonical nonce" }
    }

    private fun hmac(secret: ByteArray, material: ByteArray): String = Mac.getInstance("HmacSHA256").run {
        init(SecretKeySpec(secret, "HmacSHA256")); doFinal(material).joinToString("") { "%02x".format(it) }
    }
    private fun constantTimeEquals(a: String, b: String): Boolean = try {
        MessageDigest.isEqual(hexToBytes(a), hexToBytes(b))
    } catch (_: IllegalArgumentException) { false }
    private fun hexToBytes(value: String): ByteArray { require(value.length == 64 && value.all { it in "0123456789abcdef" }); return value.chunked(2).map { it.toInt(16).toByte() }.toByteArray() }
    private fun canonicalFields(vararg fields: String): ByteArray = buildList {
        fields.forEach { field ->
            val bytes = field.toByteArray(StandardCharsets.UTF_8)
            addAll(byteArrayOf((bytes.size ushr 24).toByte(), (bytes.size ushr 16).toByte(), (bytes.size ushr 8).toByte(), bytes.size.toByte()).toList()); addAll(bytes.toList())
        }
    }.toByteArray()
}
