package com.skycua.phonecompanion.service

import android.content.Context
import com.skycua.phonecompanion.BuildConfig
import com.skycua.phonecompanion.protocol.RpcDispatcher
import com.skycua.phonecompanion.protocol.TokenStore
import com.skycua.phonecompanion.rpc.RpcServer

/**
 * Process-wide singleton owning the [TokenStore] and the [RpcServer]. The setup
 * activity installs the token here; the server validates it on every call. The
 * server binds loopback only and is reachable solely through host-managed ADB
 * forwarding.
 */
object RpcController {
    val tokenStore = TokenStore()

    @Volatile
    private var server: RpcServer? = null

    /** Starts the RPC server if not already running. Safe to call repeatedly. */
    @Synchronized
    fun ensureStarted(context: Context) {
        if (server?.isRunning == true) return
        val handler = DeviceMethodHandler(context.applicationContext)
        val dispatcher = RpcDispatcher(handler, tokenStore)
        val srv = RpcServer(BuildConfig.RPC_PORT, dispatcher)
        srv.start()
        server = srv
    }

    @Synchronized
    fun stop() {
        server?.stop()
        server = null
    }

    val isRunning: Boolean
        get() = server?.isRunning == true
}
