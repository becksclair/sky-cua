package com.skycua.phonecompanion.setup

import com.skycua.phonecompanion.protocol.TokenStore

/**
 * Pure parsing/validation of the setup-intent extras, separated from Android's
 * Intent type so it can be unit-tested. The host delivers the token directly as a
 * string extra:
 *
 * ```
 * adb -s <serial> shell am start -n <package>/.SetupActivity \
 *   --es sky_cua_rpc_token <token> \
 *   --el sky_cua_rpc_token_expires_at_ms <epoch_ms>
 * ```
 *
 * A pushed-file path extra (`sky_cua_rpc_token_file`) is still accepted as a
 * legacy fallback, but on Android 11+ per-app storage mount namespaces make a
 * host-pushed file under `/sdcard/Android/data/<pkg>/` unreadable by the app.
 */
data class SetupToken(val token: String, val expiresAtMs: Long)

object SetupExtras {
    const val EXTRA_TOKEN = TokenStore.EXTRA_TOKEN
    const val EXTRA_TOKEN_FILE = TokenStore.EXTRA_TOKEN_FILE
    const val EXTRA_TOKEN_EXPIRES_AT_MS = TokenStore.EXTRA_TOKEN_EXPIRES_AT_MS

    /**
     * Validates the token string and expiry. Returns null when the token is
     * absent/blank or the expiry is non-positive, so the setup activity can
     * reject malformed bootstraps without installing a credential.
     */
    fun parse(token: String?, expiresAtMs: Long): SetupToken? {
        if (token.isNullOrBlank()) return null
        if (expiresAtMs <= 0) return null
        return SetupToken(token, expiresAtMs)
    }
}
