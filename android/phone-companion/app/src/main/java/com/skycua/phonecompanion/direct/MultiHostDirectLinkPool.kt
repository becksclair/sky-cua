package com.skycua.phonecompanion.direct

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.os.Handler
import android.os.Looper
import android.os.UserManager
import android.util.Log
import com.skycua.phonecompanion.service.DeviceMethodHandler

/** Snapshot of all host links for the management UI. */
data class HostLinkSnapshot(val host: HostRecord, val link: LinkSnapshot)

/** In-memory registry for HostList live status without binding to the service. */
object HostLinkSnapshotRegistry {
    @Volatile private var snapshots: List<HostLinkSnapshot> = emptyList()
    private val listeners = mutableSetOf<() -> Unit>()
    fun update(snaps: List<HostLinkSnapshot>) {
        snapshots = snaps
        synchronized(listeners) { listeners.toList().forEach { it() } }
    }
    fun get(): List<HostLinkSnapshot> = snapshots
    fun getForHost(deviceId: String): HostLinkSnapshot? = snapshots.find { it.host.deviceId == deviceId }
    fun addListener(l: () -> Unit) { synchronized(listeners) { listeners.add(l) } }
    fun removeListener(l: () -> Unit) { synchronized(listeners) { listeners.remove(l) } }
}

/** Pool that maintains one DirectLinkController per paired host. */
class MultiHostDirectLinkPool(
    private val context: Context,
    private val socketFactory: DirectSocketFactory = OkHttpDirectSocketFactory(),
    private val storeFactory: () -> AndroidCredentialStore = { AndroidCredentialStore(context.applicationContext) },
    private val contentReceiver: ContentTransferReceiver = ContentTransferReceiver(context.applicationContext),
    private val methodHandler: DeviceMethodHandler = DeviceMethodHandler(context.applicationContext, contentReceiver),
    private val onTerminal: (LinkSnapshot) -> Unit = {},
    private val onState: (List<HostLinkSnapshot>) -> Unit = {},
) {
    private data class Entry(val controller: DirectLinkController, val endpoint: String, var generation: Long = 0L)
    private val handler = Handler(Looper.getMainLooper())
    private val lock = Any()
    @Volatile private var started = false
    private var lifecycleGen = 0L
    private val entries: MutableMap<String, Entry> = mutableMapOf()

    fun isStarted(): Boolean = synchronized(lock) { started }

    fun syncHosts() = syncHosts(storeFactory().loadAll())

    private fun syncHosts(hosts: List<HostRecord>) {
        val store = storeFactory()
        val wanted = hosts.associateBy { it.deviceId }
        synchronized(lock) {
            val toRemove = entries.keys - wanted.keys
            toRemove.forEach { id -> entries.remove(id)?.controller?.close() }
            wanted.forEach { (id, rec) ->
                val existing = entries[id]
                if (existing == null) {
                    val scoped = HostScopedCredentialStore(store, id)
                    val controller = DirectLinkController(
                        object : DirectSocketFactory { override fun create(): DirectSocket = socketFactory.create() },
                        scoped,
                        requestDispatcher = DirectRequestDispatcher.forHandler(methodHandler, { entries[id]?.controller?.contentSender() }, contentReceiver),
                        contentReceiver = contentReceiver,
                    )
                    // Guard against stale blank/migrated endpoints that the single-host owner tolerated.
                    val ok = runCatching { controller.configure(rec.endpoint) }.onFailure { e ->
                        Log.w("DirectLinkPool", "invalid endpoint for $id: ${rec.endpoint}", e)
                    }.isSuccess
                    if (!ok) {
                        controller.close()
                        return@forEach
                    }
                    entries[id] = Entry(controller, rec.endpoint)
                } else if (existing.endpoint != rec.endpoint) {
                    existing.controller.close()
                    val ok = runCatching { existing.controller.configure(rec.endpoint) }.onFailure { e ->
                        Log.w("DirectLinkPool", "invalid endpoint for $id: ${rec.endpoint}", e)
                    }.isSuccess
                    if (ok) {
                        entries[id] = existing.copy(endpoint = rec.endpoint)
                    }
                    // on failure keep old endpoint so next tick can retry if store corrects
                }
            }
        }
        if (started) entries.values.forEach { it.controller.connect() }
        pushStateFromHosts(hosts)
    }

    fun start() {
        synchronized(lock) { started = true; lifecycleGen++ }
        DirectLinkReplacementNotifier.register { syncHosts(); entries.values.forEach { it.controller.connect() } }
        val um = context.getSystemService(UserManager::class.java)
        if (um?.isUserUnlocked != true) return
        val hosts = storeFactory().loadAll()
        syncHosts(hosts)
        if (hosts.isEmpty()) {
            onTerminal(LinkSnapshot(LinkState.REENROLL_REQUIRED, null, "0", 0, null))
            return
        }
        handler.removeCallbacks(reconnectLoop)
        handler.post(reconnectLoop)
        context.getSystemService(ConnectivityManager::class.java)?.registerDefaultNetworkCallback(networkCallback)
    }

    fun stop() {
        synchronized(lock) { started = false; lifecycleGen++ }
        DirectLinkReplacementNotifier.register(null)
        handler.removeCallbacks(reconnectLoop)
        runCatching { context.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(networkCallback) }
        synchronized(lock) { entries.values.forEach { it.controller.close() } }
    }

    fun hostSnapshots(): List<HostLinkSnapshot> = hostSnapshots(storeFactory().loadAll())

    private fun hostSnapshots(hosts: List<HostRecord>): List<HostLinkSnapshot> {
        val recs = hosts.associateBy { it.deviceId }
        return synchronized(lock) { entries.mapNotNull { (id, entry) -> recs[id]?.let { HostLinkSnapshot(it, entry.controller.snapshot()) } } }
    }

    fun aggregatedState(): LinkState = aggregatedState(hostSnapshots())

    fun aggregatedState(snaps: List<HostLinkSnapshot>): LinkState {
        if (snaps.isEmpty()) return LinkState.REENROLL_REQUIRED
        if (snaps.any { it.link.state == LinkState.CONNECTED }) return LinkState.CONNECTED
        if (snaps.any { it.link.state == LinkState.CONNECTING || it.link.state == LinkState.AUTHENTICATING }) return LinkState.CONNECTING
        if (snaps.any { it.link.state == LinkState.BACKOFF || it.link.state == LinkState.DISCONNECTED }) return LinkState.BACKOFF
        return LinkState.REENROLL_REQUIRED
    }

    private fun pushState() = pushStateFromHosts(storeFactory().loadAll())
    private fun pushStateFromHosts(hosts: List<HostRecord>) {
        val snaps = hostSnapshots(hosts)
        HostLinkSnapshotRegistry.update(snaps)
        onState(snaps)
    }
    private fun pushStateFromSnaps(snaps: List<HostLinkSnapshot>) {
        HostLinkSnapshotRegistry.update(snaps)
        onState(snaps)
    }

    fun deleteHost(deviceId: String) {
        synchronized(lock) { entries.remove(deviceId)?.controller?.close() }
        storeFactory().deleteHost(deviceId)
        syncHosts()
    }

    private val reconnectLoop = object : Runnable {
        override fun run() {
            val gen = synchronized(lock) { if (!started) return else lifecycleGen }
            val hosts = storeFactory().loadAll()
            syncHosts(hosts)
            entries.values.forEach { it.controller.tick(); it.controller.updateCapabilities(methodHandler.directCapabilityNames()) }
            val snaps = hostSnapshots(hosts)
            pushStateFromSnaps(snaps)
            if (snaps.isEmpty() && hosts.isEmpty()) {
                handler.removeCallbacks(this)
                runCatching { context.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(networkCallback) }
                onTerminal(LinkSnapshot(LinkState.REENROLL_REQUIRED, null, "0", 0, null))
                return
            }
            snaps.forEach { hostSnap ->
                if (hostSnap.link.state == LinkState.BACKOFF) entries[hostSnap.host.deviceId]?.controller?.connect()
            }
            synchronized(lock) { if (started && lifecycleGen == gen) handler.postDelayed(this, 1_000L + (System.nanoTime().ushr(4) % 1_000L)) }
        }
    }

    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            synchronized(lock) { if (!started) return }
            entries.values.forEach { it.controller.connect() }
        }
    }
}
