package com.skycua.phonecompanion.setup

import com.skycua.phonecompanion.protocol.TokenStore

/**
 * Pure parsing/validation of the setup-intent extras, separated from Android's
 * Intent type so it can be unit-tested. The host delivers the token through:
 *
 * ```
 * adb -s <serial> push <local-token-file> \
 *   /sdcard/Android/data/<package>/cache/sky_cua_rpc_token
 * adb -s <serial> shell am start -n <package>/.SetupActivity \
 *   --es sky_cua_rpc_token_file /sdcard/Android/data/<package>/cache/sky_cua_rpc_token \
 *   --el sky_cua_rpc_token_expires_at_ms <epoch_ms>
 * ```
 */
data class SetupToken(val token: String, val expiresAtMs: Long)

object SetupExtras {
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
