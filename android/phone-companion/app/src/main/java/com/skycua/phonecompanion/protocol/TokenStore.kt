package com.skycua.phonecompanion.protocol

import java.security.MessageDigest

/**
 * Holds the ephemeral session token and its absolute expiry delivered by the host
 * through the documented setup intent. The setup activity receives
 * `--es sky_cua_rpc_token_file` plus `--el sky_cua_rpc_token_expires_at_ms`.
 *
 * The token is validated on every RPC call before method dispatch. It is never
 * logged, persisted, or returned in responses. Comparison is constant-time to
 * avoid leaking token bytes through timing.
 */
class TokenStore {
    @Volatile
    private var token: String? = null

    @Volatile
    private var expiresAtMs: Long = 0

    /** Installs a new token/expiry pair, replacing any previous session token. */
    @Synchronized
    fun install(token: String, expiresAtMs: Long) {
        this.token = token
        this.expiresAtMs = expiresAtMs
    }

    /** Clears the stored token. */
    @Synchronized
    fun clear() {
        token = null
        expiresAtMs = 0
    }

    val hasToken: Boolean
        get() = token != null

    val expiry: Long
        get() = expiresAtMs

    /**
     * Returns true only when [candidate] is non-null, matches the installed
     * token, and the current time is before expiry.
     */
    fun isValid(candidate: String?, nowMs: Long): Boolean {
        val current = token ?: return false
        if (candidate == null) return false
        if (nowMs >= expiresAtMs) return false
        return constantTimeEquals(current, candidate)
    }

    companion object {
        /** Setup-intent extra key for the device-local token file path. */
        const val EXTRA_TOKEN_FILE = "sky_cua_rpc_token_file"

        /** Setup-intent extra key for the absolute expiry in epoch milliseconds. */
        const val EXTRA_TOKEN_EXPIRES_AT_MS = "sky_cua_rpc_token_expires_at_ms"

        /**
         * Length-independent constant-time string comparison. Returns false fast
         * only on length mismatch; for equal lengths it always inspects every
         * byte.
         */
        fun constantTimeEquals(a: String, b: String): Boolean {
            val ba = a.toByteArray(Charsets.UTF_8)
            val bb = b.toByteArray(Charsets.UTF_8)
            if (ba.size != bb.size) return false
            return MessageDigest.isEqual(ba, bb)
        }
    }
}
