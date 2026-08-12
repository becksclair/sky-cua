package com.skycua.phonecompanion.direct

import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.JsonWriter
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject
import java.util.UUID
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit

private val DIRECT_REQUEST_EXECUTOR = ThreadPoolExecutor(
    2,
    4,
    30,
    TimeUnit.SECONDS,
    ArrayBlockingQueue(32),
) { runnable -> Thread(runnable, "sky-direct-request").apply { isDaemon = true } }

enum class LinkState { DISABLED, REENROLL_REQUIRED, DISCONNECTED, CONNECTING, AUTHENTICATING, CONNECTED, BACKOFF }
enum class LinkEvent { CONNECTED, DISCONNECTED, AUTH_FAILED, REVOKED, NETWORK_AVAILABLE, NETWORK_LOST }

data class LinkSnapshot(val state: LinkState, val deviceId: String?, val linkEpoch: String, val retryAttempt: Int, val nextRetryAtMs: Long?)

/** Transport-neutral socket seam; production wiring can provide OkHttp/WebSocket. */
interface DirectSocket {
    fun connect(endpoint: String, listener: Listener)
    fun sendText(frame: String): Boolean
    fun sendBinary(bytes: ByteArray): Boolean
    fun close()

    interface Listener {
        fun onOpen()
        fun onText(frame: String)
        fun onBinary(bytes: ByteArray)
        fun onClosed(cause: Throwable?)
    }
}

interface DirectSocketFactory { fun create(): DirectSocket }

interface CapabilityEventSink { fun onCapabilitiesChanged(capabilities: Set<String>, linkEpoch: String) }

/** Owns link state and epoch fencing independently from Android service lifetimes. */
class DirectLinkController(
    private val socketFactory: DirectSocketFactory,
    private val credentials: CredentialStore,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
    private val capabilityEvents: CapabilityEventSink? = null,
    private val requestDispatcher: DirectRequestHandler? = null,
    private val contentReceiver: ContentTransferReceiver? = null,
    private val requestExecutor: (Runnable) -> Unit = { DIRECT_REQUEST_EXECUTOR.execute(it) },
) {
    private data class RequestWork(
        val generation: Long,
        val frame: String,
        val dispatcher: DirectRequestHandler,
        val deviceId: String,
        val epoch: LinkEpoch,
        val requestId: String,
        val wireEpoch: LinkEpoch,
    )

    private data class PreviewKey(val generation: Long, val sessionId: String)

    private var socket: DirectSocket? = null
    private var endpoint: String? = null
    private var state = LinkState.DISCONNECTED
    private var retryAttempt = 0
    private var nextRetryAtMs: Long? = null
    private var epoch = LinkEpoch.parseCanonical(credentials.lastAcceptedEpoch())
    private var connectionGeneration = 0L
    private var challenge: AuthChallenge? = null
    private var clientNonce: String? = null
    private var capabilities: Set<String> = emptySet()
    private var authDeadlineMs: Long? = null
    private var pendingAck = false
    private var authenticatedDeviceId: String? = null
    private var contentSender: ContentTransferSender? = null
    private val activePreviews = HashSet<PreviewKey>()
    private val pendingPreviews = HashMap<PreviewKey, RequestWork>()

    @Synchronized fun snapshot(): LinkSnapshot = LinkSnapshot(state, credentials.load()?.deviceId, epoch.toString(), retryAttempt, nextRetryAtMs)

    @Synchronized fun configure(endpoint: String) {
        EndpointValidator.requireAllowed(endpoint)
        this.endpoint = endpoint
    }

    @Synchronized fun connect() {
        val target = endpoint ?: return
        if (state == LinkState.DISABLED || state == LinkState.REENROLL_REQUIRED) return
        if (state == LinkState.CONNECTING || state == LinkState.AUTHENTICATING || state == LinkState.CONNECTED) return
        val retryAt = nextRetryAtMs
        if (state == LinkState.BACKOFF && retryAt != null && nowMs() < retryAt) return
        state = LinkState.CONNECTING
        nextRetryAtMs = null
        val generation = ++connectionGeneration
        val socket = socketFactory.create()
        this.socket = socket
        contentSender = ContentTransferSender(socket, { credentials.load()?.deviceId ?: "" }, { epoch }, onTransportFailure = { invalidateAuth() })
        socket.connect(target, object : DirectSocket.Listener {
            override fun onOpen() = opened(generation)
            override fun onText(frame: String) = receiveText(generation, frame)
            override fun onBinary(bytes: ByteArray) = receiveBinary(generation, bytes)
            override fun onClosed(cause: Throwable?) = disconnected(generation)
    })
    }

    @Synchronized private fun opened(generation: Long) {
        if (generation != connectionGeneration || state != LinkState.CONNECTING) return
        val credential = credentials.load() ?: return failAuth()
        try {
            AuthCodec.requireCanonicalUuid(credential.deviceId)
            val pending = credentials.pendingEnrollment()
            if (pending != null && nowMs() < pending.expiresAtMs) {
                clientNonce = AuthCodec.newNonce()
                val proof = AuthCodec.enrollmentAckProof(credential.deviceSecret, pending.enrollmentId, credential.deviceId, clientNonce!!)
                sendControlFrame(AuthCodec.encodeEnrollmentAck(pending.enrollmentId, credential.deviceId, clientNonce!!, proof))
                pendingAck = true
            } else {
                if (pending != null) { terminalReenroll(); return }
                sendAuthHello(credential)
            }
            authDeadlineMs = nowMs() + 15_000L
            state = LinkState.AUTHENTICATING
        } catch (_: IllegalArgumentException) { failAuth() }
    }

    private fun sendAuthHello(credential: DeviceCredential) {
        clientNonce = AuthCodec.newNonce()
        sendControlFrame(AuthCodec.encodeHello(AuthHello(PHONE_CONTROL_PROTOCOL, credential.deviceId, clientNonce!!)))
    }

    fun receiveText(frame: String) = receiveText(connectionGeneration, frame)

    @Synchronized fun receiveBinary(bytes: ByteArray) = receiveBinary(connectionGeneration, bytes)

    @Synchronized private fun receiveBinary(generation: Long, bytes: ByteArray) {
        if (generation != connectionGeneration || state == LinkState.DISABLED || state == LinkState.REENROLL_REQUIRED) return
        if (state != LinkState.CONNECTED) return invalidateAuth()
        try {
            contentReceiver?.receiveChunk(bytes, epoch) ?: error("binary content receiver is unavailable")
        } catch (_: Throwable) {
            invalidateAuth()
        }
    }

    @Synchronized internal fun contentSender(): ContentTransferSender? = contentSender

    @Synchronized internal fun sendIndependentControl(frame: String): Boolean = sendControlFrame(frame)

    private fun sendControlFrame(frame: String, dependent: Boolean = false): Boolean {
        val sender = contentSender
        if (!dependent && sender != null && sender.activeTransferCount() > 0) {
            val accepted = sender.enqueueControl { socket?.sendText(frame) == true }
            if (!accepted) invalidateAuth()
            return accepted
        }
        val accepted = socket?.sendText(frame) == true
        if (!accepted && state == LinkState.CONNECTED) invalidateAuth()
        return accepted
    }

    private fun receiveText(generation: Long, frame: String) {
        val parsed = runCatching { JsonParser.parseObject(frame) }.getOrNull()
        val type = parsed?.string("type")
        if (type == "content_declare" || type == "content_commit" || type == "content_abort") {
            synchronized(this) {
                if (generation != connectionGeneration || state != LinkState.CONNECTED) return
                val device = authenticatedDeviceId ?: return invalidateAuth()
                try {
                    contentReceiver?.receiveControl(parsed, device, epoch) ?: error("content receiver is unavailable")
                } catch (_: Throwable) {
                    invalidateAuth()
                }
            }
            return
        }
        if (type == "request") { receiveRequest(generation, frame, parsed); return }
        synchronized(this) { receiveTextLocked(generation, frame) }
    }

    private fun receiveRequest(generation: Long, frame: String, parsed: JsonValue.Obj) {
        val snapshot = synchronized(this) {
            if (generation != connectionGeneration || state != LinkState.CONNECTED) return
            Triple(requestDispatcher, authenticatedDeviceId, epoch)
        }
        val dispatcher = snapshot.first ?: return
        val device = snapshot.second ?: return
        val requestId = parsed.string("request_id") ?: return
        val wireEpoch = parsed.linkEpoch("link_epoch") ?: return
        val work = RequestWork(generation, frame, dispatcher, device, snapshot.third, requestId, wireEpoch)
        val previewKey = parsed.takeIf { it.string("method") == "camera" }
            ?.obj("params")
            ?.takeIf { it.string("operation") == "preview_frame" }
            ?.string("camera_session_id")
            ?.let { PreviewKey(generation, it) }
        if (previewKey == null) {
            executeRequest(work, null)
            return
        }
        val (startNow, superseded) = synchronized(this) {
            if (activePreviews.add(previewKey)) true to null
            else false to pendingPreviews.put(previewKey, work)
        }
        if (startNow) {
            executeRequest(work, previewKey)
        } else if (superseded != null) {
            sendResponseIfCurrent(
                superseded,
                JsonWriter.write(jsonObject {
                    put("type", "error")
                    put("request_id", superseded.requestId)
                    put("device_id", superseded.deviceId)
                    put("link_epoch", superseded.wireEpoch)
                    put("code", "preview_superseded")
                    put("message", "a newer camera preview frame was requested")
                }),
            )
        }
    }

    private fun executeRequest(work: RequestWork, previewKey: PreviewKey?) {
        try {
            requestExecutor(Runnable {
                val response = work.dispatcher.dispatch(work.frame, work.deviceId, work.epoch, nowMs())
                sendResponseIfCurrent(work, response)
                if (previewKey != null) finishPreview(previewKey)
            })
        } catch (_: Throwable) {
            if (previewKey != null) finishPreview(previewKey)
            sendResponseIfCurrent(
                work,
                JsonWriter.write(jsonObject {
                    put("type", "error")
                    put("request_id", work.requestId)
                    put("device_id", work.deviceId)
                    put("link_epoch", work.wireEpoch)
                    put("code", "server_busy")
                    put("message", "the companion request queue is full")
                }),
            )
        }
    }

    private fun finishPreview(key: PreviewKey) {
        val next = synchronized(this) {
            pendingPreviews.remove(key).also { if (it == null) activePreviews.remove(key) }
        }
        if (next != null) executeRequest(next, key)
    }

    private fun sendResponseIfCurrent(work: RequestWork, response: String) {
        synchronized(this) {
            if (work.generation == connectionGeneration && state == LinkState.CONNECTED && epoch == work.epoch) {
                sendControlFrame(response, dependent = true)
            }
        }
    }

    private fun receiveTextLocked(generation: Long, frame: String) {
        if (generation != connectionGeneration || state == LinkState.DISABLED || state == LinkState.REENROLL_REQUIRED) return
        if (state == LinkState.AUTHENTICATING && authDeadlineMs?.let { nowMs() >= it } == true) return failAuth()
        val decoded = try { ControlFrameCodec.decode(frame) } catch (_: Exception) { failAuth(); return }
        when (decoded) {
            is ControlFrame.Challenge -> {
                val credential = credentials.load()
                if (state != LinkState.AUTHENTICATING || challenge != null || credential == null || clientNonce == null || decoded.value.protocol != PHONE_CONTROL_PROTOCOL) return failAuth()
                val proposed = try { LinkEpoch.parseCanonical(decoded.value.linkEpoch) } catch (_: IllegalArgumentException) { return failAuth() }
                try { AuthCodec.requireCanonicalNonce(decoded.value.serverNonce) } catch (_: IllegalArgumentException) { return failAuth() }
                if (proposed <= epoch || !AuthCodec.verifyServerProof(credential.deviceSecret, credential.deviceId, decoded.value, clientNonce!!)) return failAuth()
                challenge = decoded.value
                val proof = AuthCodec.prove(credential.deviceSecret, PHONE_CONTROL_PROTOCOL, credential.deviceId, decoded.value.serverNonce, clientNonce!!, decoded.value.linkEpoch, "companion")
                sendControlFrame(AuthCodec.encodeProof(AuthProof(decoded.value.linkEpoch, proof)))
            }
            is ControlFrame.Accepted -> {
                val currentChallenge = challenge ?: return failAuth()
                val credential = credentials.load() ?: return failAuth()
                if (state != LinkState.AUTHENTICATING || !AuthCodec.verifyAuthOk(credential.deviceId, currentChallenge, decoded.value)) return failAuth()
                val acceptedEpoch = try { LinkEpoch.parseCanonical(decoded.value.linkEpoch) } catch (_: IllegalArgumentException) { return failAuth() }
                if (acceptedEpoch <= epoch) return failAuth()
                try { credentials.saveAcceptedEpoch(decoded.value.linkEpoch) } catch (_: Exception) { return failAuth() }
                state = LinkState.CONNECTED
                retryAttempt = 0
                nextRetryAtMs = null
                epoch = acceptedEpoch
                authenticatedDeviceId = credential.deviceId
                authDeadlineMs = null
                challenge = null
                clientNonce = null
                sendCapabilitiesChanged()
            }
            is ControlFrame.Error -> if (pendingAck && (decoded.code.contains("unknown", true) || decoded.code.contains("expired", true))) {
                terminalReenroll()
            } else if (state != LinkState.CONNECTED || decoded.code.startsWith("auth")) failAuth()
            is ControlFrame.EnrollmentCommitted -> if (pendingAck) {
                val pending = credentials.pendingEnrollment() ?: return failAuth()
                val credential = credentials.load() ?: return failAuth()
                if (decoded.value.protocol != PHONE_CONTROL_PROTOCOL || decoded.value.enrollmentId != pending.enrollmentId || decoded.value.deviceId != credential.deviceId) return failAuth()
                credentials.clearPendingEnrollment(); pendingAck = false; sendAuthHello(credential)
            } else if (state != LinkState.CONNECTED) failAuth()
            is ControlFrame.Other -> if (state != LinkState.CONNECTED) failAuth() else if (decoded.type == "capability_changed") capabilityEvents?.onCapabilitiesChanged(capabilities, epoch.toString())
        }
    }

    @Synchronized fun updateCapabilities(values: Set<String>) {
        if (capabilities == values) return
        capabilities = values.toSortedSet()
        if (state == LinkState.CONNECTED) sendCapabilitiesChanged()
        capabilityEvents?.onCapabilitiesChanged(capabilities, epoch.toString())
    }

    private fun sendCapabilitiesChanged() {
        val device = authenticatedDeviceId ?: return
        sendControlFrame(JsonWriter.write(jsonObject {
            put("type", "event")
            put("event_id", UUID.randomUUID().toString())
            put("device_id", device)
            put("link_epoch", epoch)
            put("event", "capability_changed")
            put("payload", jsonObject { put("capabilities", jsonArray(capabilities.sorted().map(JsonValue::Str))) })
        }))
    }

    @Synchronized fun pendingAuthProof(): AuthProof? {
        val current = challenge ?: return null
        val nonce = clientNonce ?: return null
        val secret = credentials.load()?.deviceSecret ?: return null
        return AuthProof(current.linkEpoch, AuthCodec.prove(secret, PHONE_CONTROL_PROTOCOL, credentials.load()!!.deviceId, current.serverNonce, nonce, current.linkEpoch, "companion"))
    }

    @Synchronized fun disconnected() = disconnected(connectionGeneration)

    /** Lifecycle owners call this periodically to enforce handshake timeout. */
    @Synchronized fun tick() {
        if (state == LinkState.AUTHENTICATING && authDeadlineMs?.let { nowMs() >= it } == true) failAuth()
    }

    @Synchronized private fun disconnected(generation: Long) {
        if (generation != connectionGeneration || state == LinkState.DISABLED || state == LinkState.REENROLL_REQUIRED) return
        state = LinkState.BACKOFF
        retryAttempt++
        // Exponential backoff is bounded and deterministic; the owner decides
        // when to call connect again (typically from a lifecycle/network tick).
        nextRetryAtMs = nowMs() + minOf(30_000L, 1000L shl retryAttempt.coerceAtMost(5))
        socket = null
        contentSender?.onDisconnected(); contentSender = null
        clearPreviewQueue()
        runCatching { contentReceiver?.abortEpoch(epoch) }
        authenticatedDeviceId = null
    }

    private fun invalidateAuth() {
        val old = socket
        connectionGeneration++
        socket = null
        contentSender?.onDisconnected(); contentSender = null
        clearPreviewQueue()
        runCatching { contentReceiver?.abortEpoch(epoch) }
        authenticatedDeviceId = null
        old?.close()
        challenge = null
        clientNonce = null
        authDeadlineMs = null
        state = LinkState.BACKOFF
        retryAttempt++
        nextRetryAtMs = nowMs() + minOf(30_000L, 1000L shl retryAttempt.coerceAtMost(5))
    }

    private fun failAuth() { invalidateAuth() }

    private fun terminalReenroll() {
        val old = socket
        connectionGeneration++
        socket = null
        contentSender?.onDisconnected(); contentSender = null
        clearPreviewQueue()
        runCatching { contentReceiver?.abortEpoch(epoch) }
        authenticatedDeviceId = null
        pendingAck = false
        challenge = null
        clientNonce = null
        authDeadlineMs = null
        credentials.clear()
        state = LinkState.REENROLL_REQUIRED
        authenticatedDeviceId = null
        old?.close()
    }

    @Synchronized fun revoke() {
        val old = socket
        connectionGeneration++
        socket = null
        contentSender?.onDisconnected(); contentSender = null
        clearPreviewQueue()
        runCatching { contentReceiver?.abortEpoch(epoch) }
        old?.close()
        challenge = null
        clientNonce = null
        authDeadlineMs = null
        credentials.clear()
        state = LinkState.DISABLED
        epoch = LinkEpoch.ZERO
        authenticatedDeviceId = null
    }

    @Synchronized fun close() {
        val old = socket
        connectionGeneration++
        socket = null
        contentSender?.onDisconnected(); contentSender = null
        clearPreviewQueue()
        runCatching { contentReceiver?.abortEpoch(epoch) }
        old?.close()
        challenge = null
        clientNonce = null
        authDeadlineMs = null
        state = LinkState.DISCONNECTED
        authenticatedDeviceId = null
    }

    /** Replaces the authenticated link after a durable credential/endpoint commit. */
    @Synchronized internal fun reconnectForCredentialReplacement(endpoint: String?) {
        close()
        epoch = LinkEpoch.parseCanonical(credentials.lastAcceptedEpoch())
        retryAttempt = 0
        nextRetryAtMs = null
        pendingAck = false
        endpoint?.let { configure(it); connect() }
    }

    fun isCurrentEpoch(candidate: String): Boolean = candidate == epoch.toString()

    private fun clearPreviewQueue() {
        activePreviews.clear()
        pendingPreviews.clear()
    }
}

/** Lifecycle owner seam: Android services/activities may retain one controller. */
interface DirectLinkOwner {
    fun startDirectLink()
    fun stopDirectLink()
    fun controller(): DirectLinkController
}
