package com.skycua.phonecompanion

import android.app.Activity
import android.os.Bundle
import com.skycua.phonecompanion.service.RpcController
import com.skycua.phonecompanion.setup.SetupExtras
import java.io.File

/**
 * Receives an ADB-launched setup intent from the host, reads the ephemeral RPC
 * session token from the host-pushed token file, installs it into the
 * process-wide token store, and starts the RPC server. The token is never
 * logged or persisted; the activity finishes immediately and shows no UI.
 * The activity is exported only behind `android.permission.DUMP`: ADB shell can
 * launch it by explicit component, while ordinary co-resident apps cannot start
 * it to replace the bearer token.
 */
class SetupActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handleIntent()
        finish()
    }

    private fun handleIntent() {
        val tokenFile = intent?.getStringExtra(SetupExtras.EXTRA_TOKEN_FILE)
        val token = readTokenFile(tokenFile)
        val expiry = intent?.getLongExtra(SetupExtras.EXTRA_TOKEN_EXPIRES_AT_MS, 0L) ?: 0L
        val parsed = SetupExtras.parse(token, expiry) ?: return
        RpcController.tokenStore.install(parsed.token, parsed.expiresAtMs)
        RpcController.ensureStarted(applicationContext)
    }

    private fun readTokenFile(path: String?): String? {
        if (path.isNullOrBlank()) return null
        val file = File(path)
        return try {
            val token = file.readText(Charsets.UTF_8).trim()
            file.delete()
            token
        } catch (_: Exception) {
            null
        }
    }
}
