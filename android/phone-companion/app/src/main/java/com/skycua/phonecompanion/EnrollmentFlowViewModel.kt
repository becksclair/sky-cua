package com.skycua.phonecompanion

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import com.skycua.phonecompanion.direct.AndroidCredentialStore
import com.skycua.phonecompanion.direct.DirectLinkServiceOwner
import com.skycua.phonecompanion.direct.EnrollmentCodec
import com.skycua.phonecompanion.direct.EnrollmentOutcome
import com.skycua.phonecompanion.direct.EnrollmentPayload
import com.skycua.phonecompanion.direct.EnrollmentRedeemer
import com.skycua.phonecompanion.direct.EnrollmentUiState
import com.skycua.phonecompanion.direct.EnrollmentUiStateMachine
import com.skycua.phonecompanion.direct.OkHttpDirectSocketFactory
import java.security.MessageDigest

internal enum class EnrollmentScreenPhase { ENTRY, REVIEW, REPLACE, CONNECTING, SUCCESS, ERROR }

internal enum class EnrollmentNotice {
    NONE,
    REVIEW_READY,
    REVIEW_EXISTING,
    REPLACE_WARNING,
    CREDENTIAL_CHANGED,
    CONNECTING,
    FINISHING_CURRENT,
    CONNECTED,
    INVALID_OR_EXPIRED,
    FAILED,
}

internal enum class EnrollmentFailureReason { EXPIRED, UNREACHABLE, SAVE, REJECTED }

/** Public-to-the-Activity state intentionally excludes the bootstrap credential. */
internal data class EnrollmentScreenState(
    val phase: EnrollmentScreenPhase = EnrollmentScreenPhase.ENTRY,
    val endpoint: String? = null,
    val notice: EnrollmentNotice = EnrollmentNotice.NONE,
    val failureReason: EnrollmentFailureReason? = null,
)

internal fun interface EnrollmentUiDispatcher {
    fun dispatch(block: () -> Unit)
}

internal interface EnrollmentTransactionRunner {
    fun redeem(payload: EnrollmentPayload, callback: (EnrollmentOutcome) -> Unit)
}

/**
 * Retains the complete enrollment flow across Activity recreation without SavedState.
 *
 * The pending payload and any deferred deep link remain only in ViewModel memory. Observer
 * dispatch looks up the currently attached Activity at execution time, so transaction callbacks
 * never close over an Activity that may already have been destroyed.
 */
internal class EnrollmentFlowViewModel(
    private val transactionRunner: EnrollmentTransactionRunner,
    private val fingerprintProvider: () -> String?,
    private val onConnected: () -> Unit,
    private val nowMs: () -> Long = System::currentTimeMillis,
    private val dispatcher: EnrollmentUiDispatcher = MainThreadEnrollmentDispatcher(),
) : ViewModel() {
    private val lock = Any()
    private var observerGeneration = 0L
    private var observer: Pair<Long, (EnrollmentScreenState) -> Unit>? = null
    private var pending: EnrollmentPayload? = null
    private var stateMachine: EnrollmentUiStateMachine? = null
    private var deferredLink: String? = null
    private var activeEnrollmentId: String? = null
    private var initialized = false

    var screenState: EnrollmentScreenState = EnrollmentScreenState()
        private set

    fun attach(observer: (EnrollmentScreenState) -> Unit): Long {
        val generation: Long
        synchronized(lock) {
            generation = ++observerGeneration
            this.observer = generation to observer
        }
        dispatchCurrent()
        return generation
    }

    fun detach(generation: Long) {
        synchronized(lock) {
            if (observer?.first == generation) observer = null
        }
    }

    /** Handles the launch Intent exactly once per ViewModel lifetime, including after process death. */
    fun acceptInitialLink(raw: String?) {
        synchronized(lock) {
            if (initialized) return
            initialized = true
        }
        raw?.let(::offerLink)
    }

    /** A later deep link joins this single flow; it cannot launch a concurrent redemption. */
    fun offerLink(raw: String) {
        if (screenState.phase == EnrollmentScreenPhase.CONNECTING) {
            val incomingId = runCatching { parse(raw).enrollmentId }.getOrNull()
            if (incomingId == activeEnrollmentId) return
            deferredLink = raw
            publish(
                screenState.copy(
                    notice = EnrollmentNotice.FINISHING_CURRENT,
                ),
            )
            return
        }
        resetInternal()
        review(raw)
    }

    fun review(raw: String) {
        val parsed =
            try {
                parse(raw)
            } catch (_: Exception) {
                pending = null
                stateMachine = null
                publish(
                    EnrollmentScreenState(
                        phase = EnrollmentScreenPhase.ERROR,
                        notice = EnrollmentNotice.INVALID_OR_EXPIRED,
                    ),
                )
                return
            }
        pending = parsed
        val fingerprint = fingerprintProvider()
        val existing = fingerprint != null
        stateMachine = EnrollmentUiStateMachine(existing, fingerprint).also { it.review() }
        publish(
            EnrollmentScreenState(
                phase = EnrollmentScreenPhase.REVIEW,
                endpoint = parsed.endpoint,
                notice =
                    if (existing) {
                        EnrollmentNotice.REVIEW_EXISTING
                    } else {
                        EnrollmentNotice.REVIEW_READY
                    },
            ),
        )
    }

    fun confirmEndpoint() {
        val machine = stateMachine ?: return
        machine.confirmEndpoint(fingerprintProvider())
        if (machine.state == EnrollmentUiState.CONFIRM_REPLACE) {
            publish(
                screenState.copy(
                    phase = EnrollmentScreenPhase.REPLACE,
                    notice = EnrollmentNotice.REPLACE_WARNING,
                ),
            )
        } else {
            submitPending()
        }
    }

    fun submitPending() {
        val parsed = pending ?: return
        val machine = stateMachine ?: return
        if (!machine.recheckBeforeRedeem(fingerprintProvider())) {
            publish(
                screenState.copy(
                    phase = EnrollmentScreenPhase.REPLACE,
                    notice = EnrollmentNotice.CREDENTIAL_CHANGED,
                ),
            )
            return
        }
        if (machine.state == EnrollmentUiState.CONFIRM_REPLACE) {
            machine.confirmReplacement()
        } else {
            check(machine.state == EnrollmentUiState.REDEEMING) {
                "enrollment is not ready to redeem"
            }
        }
        pending = null
        activeEnrollmentId = parsed.enrollmentId
        publish(
            EnrollmentScreenState(
                phase = EnrollmentScreenPhase.CONNECTING,
                endpoint = parsed.endpoint,
                notice = EnrollmentNotice.CONNECTING,
            ),
        )
        transactionRunner.redeem(parsed, ::completeTransaction)
    }

    fun reset() {
        if (screenState.phase == EnrollmentScreenPhase.CONNECTING) return
        resetInternal()
        publish(EnrollmentScreenState())
    }

    override fun onCleared() {
        synchronized(lock) { observer = null }
        pending = null
        deferredLink = null
        stateMachine = null
        activeEnrollmentId = null
    }

    private fun completeTransaction(outcome: EnrollmentOutcome) {
        dispatcher.dispatch { completeTransactionOnUi(outcome) }
    }

    private fun completeTransactionOnUi(outcome: EnrollmentOutcome) {
        activeEnrollmentId = null
        when (outcome) {
            is EnrollmentOutcome.Success -> {
                onConnected()
                publish(
                    EnrollmentScreenState(
                        phase = EnrollmentScreenPhase.SUCCESS,
                        endpoint = screenState.endpoint,
                        notice = EnrollmentNotice.CONNECTED,
                    ),
                )
            }
            is EnrollmentOutcome.Failure -> {
                publish(
                    EnrollmentScreenState(
                        phase = EnrollmentScreenPhase.ERROR,
                        endpoint = screenState.endpoint,
                        notice = EnrollmentNotice.FAILED,
                        failureReason = classifyFailure(outcome.message),
                    ),
                )
            }
        }
        deferredLink?.also { next ->
            deferredLink = null
            resetInternal()
            review(next)
        }
    }

    private fun resetInternal() {
        pending = null
        stateMachine = null
        if (screenState.phase != EnrollmentScreenPhase.CONNECTING) activeEnrollmentId = null
    }

    private fun parse(raw: String): EnrollmentPayload {
        val text = raw.trim()
        return if (text.startsWith("skycua://")) {
            EnrollmentCodec.decode(text, nowMs())
        } else {
            EnrollmentCodec.decodeManual(text, nowMs())
        }
    }

    private fun classifyFailure(message: String): EnrollmentFailureReason =
        when {
            message.contains("expired", true) -> EnrollmentFailureReason.EXPIRED
            message.contains("closed", true) || message.contains("connect", true) ->
                EnrollmentFailureReason.UNREACHABLE
            message.contains("persist", true) ||
                message.contains("commit", true) ||
                message.contains("disk", true) -> EnrollmentFailureReason.SAVE
            else -> EnrollmentFailureReason.REJECTED
        }

    private fun publish(state: EnrollmentScreenState) {
        synchronized(lock) { screenState = state }
        dispatchCurrent()
    }

    private fun dispatchCurrent() {
        dispatcher.dispatch {
            val target: ((EnrollmentScreenState) -> Unit)?
            val snapshot: EnrollmentScreenState
            synchronized(lock) {
                target = observer?.second
                snapshot = screenState
            }
            target?.invoke(snapshot)
        }
    }

    class Factory(private val context: Context) : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <T : ViewModel> create(modelClass: Class<T>): T {
            require(modelClass.isAssignableFrom(EnrollmentFlowViewModel::class.java))
            val appContext = context.applicationContext
            val starter = { DirectLinkServiceOwner.startByEnrollment(appContext) }
            return EnrollmentFlowViewModel(
                transactionRunner =
                    object : EnrollmentTransactionRunner {
                        override fun redeem(
                            payload: EnrollmentPayload,
                            callback: (EnrollmentOutcome) -> Unit,
                        ) {
                            EnrollmentTransactions.redeem(
                                appContext,
                                payload,
                                onPendingSaved = starter,
                                callback = callback,
                            )
                        }
                    },
                fingerprintProvider = { credentialFingerprint(appContext) },
                onConnected = starter,
            ) as T
        }
    }
}

private class MainThreadEnrollmentDispatcher : EnrollmentUiDispatcher {
    private val handler = Handler(Looper.getMainLooper())

    override fun dispatch(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else handler.post(block)
    }
}

private fun credentialFingerprint(context: Context): String? =
    AndroidCredentialStore(context).load()?.let { credential ->
        MessageDigest.getInstance("SHA-256")
            .digest(credential.deviceSecret)
            .joinToString("") { "%02x".format(it) }
    }

/** Process-wide fence prevents duplicate redemption if a transaction is retried externally. */
private object EnrollmentTransactions {
    private val lock = Any()
    private val active = HashSet<String>()

    fun redeem(
        context: Context,
        payload: EnrollmentPayload,
        onPendingSaved: () -> Unit = {},
        callback: (EnrollmentOutcome) -> Unit,
    ) {
        synchronized(lock) {
            if (!active.add(payload.enrollmentId)) {
                callback(EnrollmentOutcome.Failure("enrollment already in progress"))
                return
            }
        }
        EnrollmentRedeemer(
            OkHttpDirectSocketFactory(),
            AndroidCredentialStore(context.applicationContext),
            onPendingSaved = onPendingSaved,
        ).redeem(payload) { result ->
            synchronized(lock) { active.remove(payload.enrollmentId) }
            callback(result)
        }
    }
}
