package com.skycua.phonecompanion.direct

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.os.Build
import android.os.IBinder
import android.os.UserManager
import android.os.Handler
import android.os.Looper
import android.net.ConnectivityManager
import android.net.Network
import androidx.core.content.ContextCompat
import com.skycua.phonecompanion.EnrollmentActivity
import com.skycua.phonecompanion.MainActivity
import com.skycua.phonecompanion.service.DeviceMethodHandler

internal object DirectLinkReplacementNotifier {
    @Volatile private var callback: (() -> Unit)? = null
    fun register(callback: (() -> Unit)?) { this.callback = callback }
    fun notifyCommitted() { callback?.invoke() }
}

/** Persisted non-secret endpoint settings for the outbound link. */
object DirectLinkSettings {
    private const val PREFS = "phone_control_v2"
    private const val ENDPOINT = "endpoint"
    fun setEndpoint(context: Context, endpoint: String) {
        EndpointValidator.requireAllowed(endpoint)
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(ENDPOINT, endpoint).apply()
    }
    fun endpoint(context: Context): String? {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val version = prefs.getLong("committed_version", 0L)
        val value = prefs.getString(if (version > 0) "record_${version}_endpoint" else ENDPOINT, null)
        return value?.also { runCatching { EndpointValidator.requireAllowed(it) }.getOrNull() ?: return null }
    }
}

internal data class DirectLinkNotificationCopy(val title: String, val body: String, val recovery: Boolean)

internal object DirectLinkNotificationText {
    fun forState(state: LinkState): DirectLinkNotificationCopy = when (state) {
        LinkState.CONNECTED -> DirectLinkNotificationCopy("Sky companion connected", "Host connection is active", false)
        LinkState.CONNECTING, LinkState.AUTHENTICATING -> DirectLinkNotificationCopy("Connecting to Sky host", "Establishing the host connection", false)
        LinkState.BACKOFF, LinkState.DISCONNECTED -> DirectLinkNotificationCopy("Sky host connection paused", "Waiting to reconnect", false)
        LinkState.REENROLL_REQUIRED, LinkState.DISABLED -> DirectLinkNotificationCopy("Sky companion needs attention", "Connect a new host to continue", true)
    }
}

internal fun directLinkNeedsUserRetry(availability: DirectLinkServiceOwner.Availability, desired: Boolean): Boolean =
    desired && availability == DirectLinkServiceOwner.Availability.STOPPED

/** Process-safe lifecycle owner used by the accessibility service or explicit service start. */
class AndroidDirectLinkOwner(
    private val context: Context,
    private val onTerminal: (LinkSnapshot) -> Unit = {},
    private val onState: (LinkSnapshot) -> Unit = {},
) : DirectLinkOwner {
    private val credentials = AndroidCredentialStore(context.applicationContext)
    private val contentReceiver = ContentTransferReceiver(context.applicationContext)
    private val methodHandler = DeviceMethodHandler(context.applicationContext, contentReceiver)
    private val controller: DirectLinkController by lazy {
        DirectLinkController(
            OkHttpDirectSocketFactory(),
            credentials,
            requestDispatcher = DirectRequestDispatcher.forHandler(
                methodHandler,
                { controller.contentSender() },
                contentReceiver,
            ),
            contentReceiver = contentReceiver,
        )
    }
    private val handler = Handler(Looper.getMainLooper())
    private val lifecycleLock = Any()
    private var started = false
    private var lifecycleGeneration = 0L
    private var terminalNotified = false
    private var configuredDeviceId: String? = null
    private var configuredEndpoint: String? = null
    init { DirectLinkReplacementNotifier.register { restartForCredentialReplacement() } }
    private fun restartForCredentialReplacement() {
        synchronized(lifecycleLock) {
            if (!started) return
            configuredDeviceId = credentials.load()?.deviceId
            configuredEndpoint = DirectLinkSettings.endpoint(context)
            controller.reconnectForCredentialReplacement(configuredEndpoint)
        }
    }
    private fun haltForTerminal(snapshot: LinkSnapshot) {
        val notify = synchronized(lifecycleLock) {
            if (!started || terminalNotified) return
            terminalNotified = true
            started = false
            lifecycleGeneration++
            handler.removeCallbacks(reconnect)
            true
        }
        if (notify) {
            runCatching { context.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(networkCallback) }
            // Terminal credential loss must fence and close the transport before
            // handing control back to the service; no socket may survive solely
            // because an accessibility lease is still held.
            controller.close()
            onTerminal(snapshot)
        }
    }
    private val reconnect = object : Runnable {
        override fun run() {
            val generation = synchronized(lifecycleLock) { if (!started) return else lifecycleGeneration }
            controller.tick()
            val currentDeviceId = credentials.load()?.deviceId
            val currentEndpoint = DirectLinkSettings.endpoint(context)
            if (configuredDeviceId != currentDeviceId || configuredEndpoint != currentEndpoint) {
                configuredDeviceId = currentDeviceId
                configuredEndpoint = currentEndpoint
                controller.close()
                currentEndpoint?.let { controller.configure(it); controller.connect() }
            }
            val snapshot = controller.snapshot()
            onState(snapshot)
            if (snapshot.state == LinkState.REENROLL_REQUIRED || snapshot.state == LinkState.DISABLED ||
                (credentials.load() == null && credentials.pendingEnrollment() == null)
            ) {
                haltForTerminal(snapshot.copy(state = if (snapshot.state == LinkState.DISABLED) LinkState.DISABLED else LinkState.REENROLL_REQUIRED))
                return
            }
            if (snapshot.state == LinkState.BACKOFF) {
                synchronized(lifecycleLock) { if (started && lifecycleGeneration == generation) controller.connect() }
            }
            synchronized(lifecycleLock) {
                if (started && lifecycleGeneration == generation) handler.postDelayed(this, 1_000L + (System.nanoTime().ushr(4) % 1_000L))
            }
        }
    }
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            val generation = synchronized(lifecycleLock) { if (!started) return else lifecycleGeneration }
            synchronized(lifecycleLock) { if (started && lifecycleGeneration == generation) controller.connect() }
        }
    }
    override fun startDirectLink() {
        synchronized(lifecycleLock) { started = true; terminalNotified = false; lifecycleGeneration++ }
        val userManager = context.getSystemService(UserManager::class.java)
        if (userManager?.isUserUnlocked != true) return
        if (credentials.load() == null && credentials.pendingEnrollment() == null) {
            haltForTerminal(controller.snapshot().copy(state = LinkState.REENROLL_REQUIRED))
            return
        }
        configuredDeviceId = credentials.load()?.deviceId
        configuredEndpoint = DirectLinkSettings.endpoint(context)
        controller.updateCapabilities(methodHandler.directCapabilityNames())
        configuredEndpoint?.let { controller.configure(it); controller.connect() }
        handler.removeCallbacks(reconnect); handler.post(reconnect)
        context.getSystemService(ConnectivityManager::class.java)?.registerDefaultNetworkCallback(networkCallback)
    }
    override fun stopDirectLink() {
        synchronized(lifecycleLock) { started = false; lifecycleGeneration++ }
        DirectLinkReplacementNotifier.register(null)
        handler.removeCallbacks(reconnect)
        runCatching { context.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(networkCallback) }
        controller.close()
    }
    override fun controller(): DirectLinkController = controller
}

/** Optional dedicated owner; it is started only after the first credential unlock. */
class DirectLinkService : Service() {
    private var owner: AndroidDirectLinkOwner? = null
    private var notificationState: LinkState? = null
    private var terminalState = false
    override fun onCreate() {
        super.onCreate()
        DirectLinkServiceOwner.onServiceCreated()
        startVisibleForeground()
        owner = AndroidDirectLinkOwner(applicationContext, ::onTerminal, { updateNotification(it.state) }).also { it.startDirectLink() }
    }
    override fun onDestroy() {
        owner?.stopDirectLink(); owner = null
        DirectLinkServiceOwner.onServiceDestroyed()
        super.onDestroy()
    }
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY
        .also {
            if (terminalState) {
                terminalState = false
                updateNotification(LinkState.CONNECTING)
                owner?.startDirectLink()
            }
        }

    private fun onTerminal(snapshot: LinkSnapshot) {
        terminalState = true
        updateNotification(snapshot.state)
        if (!DirectLinkServiceOwner.onServiceTerminal(applicationContext)) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun startVisibleForeground() {
        val notifications = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            notifications.createNotificationChannel(
                NotificationChannel(
                    NOTIFICATION_CHANNEL_ID,
                    "Sky host connection",
                    NotificationManager.IMPORTANCE_LOW,
                ).apply {
                    description = "Keeps the Sky host connection available"
                    setShowBadge(false)
                },
            )
        }
        updateNotification(LinkState.CONNECTING)
    }

    private fun updateNotification(state: LinkState) {
        if (notificationState == state) return
        notificationState = state
        val copy = DirectLinkNotificationText.forState(state)
        val openIntent = PendingIntent.getActivity(
            this, 1, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(com.skycua.phonecompanion.R.mipmap.ic_launcher)
            .setContentTitle(copy.title)
            .setContentText(copy.body)
            .setContentIntent(openIntent)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setOngoing(true)
            .setShowWhen(false)
            .apply {
                if (copy.recovery) {
                    val recoveryIntent = PendingIntent.getActivity(
                        this@DirectLinkService, 2, Intent(this@DirectLinkService, EnrollmentActivity::class.java),
                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                    )
                    addAction(Notification.Action.Builder(
                        Icon.createWithResource(this@DirectLinkService, com.skycua.phonecompanion.R.mipmap.ic_launcher),
                        "Connect new host", recoveryIntent,
                    ).build())
                }
            }
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private companion object {
        const val NOTIFICATION_CHANNEL_ID = "sky_host_connection"
        const val NOTIFICATION_ID = 47684
    }
}

object DirectLinkServiceOwner {
    enum class Availability { STOPPED, STARTING, RUNNING, START_DENIED, TERMINAL }
    @Volatile private var enrollmentLease = false
    @Volatile private var serviceActive = false
    @Volatile private var availability = Availability.STOPPED
    private var accessibilityLeases = 0
    @Synchronized private fun start(context: Context): Boolean {
        if (serviceActive && availability != Availability.TERMINAL) return true
        return try {
            ContextCompat.startForegroundService(context, Intent(context, DirectLinkService::class.java))
            availability = Availability.STARTING
            true
        } catch (_: android.app.ForegroundServiceStartNotAllowedException) {
            availability = Availability.START_DENIED
            false
        } catch (_: IllegalStateException) {
            availability = Availability.START_DENIED
            false
        }
    }
    @Synchronized fun startByEnrollment(context: Context) {
        startByEnrollmentResult(context)
    }
    @Synchronized fun startByEnrollmentResult(context: Context): Boolean {
        if (enrollmentLease) return true
        if (!start(context)) return false
        enrollmentLease = true
        return true
    }
    @Synchronized fun acquireAccessibility(context: Context): Boolean {
        accessibilityLeases++
        if (enrollmentLease || serviceActive) return true
        return start(context)
    }
    /** Retry a denied accessibility cold start only after an explicit user action. */
    @Synchronized fun retryAccessibility(context: Context): Boolean {
        if (accessibilityLeases == 0) return false
        return start(context)
    }
    /** User-initiated retry from the visible operator screen. */
    @Synchronized fun retryUserInitiated(context: Context): Boolean {
        if (accessibilityLeases > 0) return start(context)
        return startByEnrollmentResult(context)
    }
    @Synchronized fun releaseAccessibility(context: Context) {
        if (accessibilityLeases == 0) return
        accessibilityLeases--
        val pendingRecovery = AndroidCredentialStore(context.applicationContext).pendingEnrollment() != null
        if (accessibilityLeases == 0 && !enrollmentLease && !pendingRecovery) context.stopService(Intent(context, DirectLinkService::class.java))
    }
    @Synchronized fun stopEnrollment(context: Context) { if (!enrollmentLease) return; enrollmentLease = false; if (accessibilityLeases == 0) context.stopService(Intent(context, DirectLinkService::class.java)) }
    @Synchronized fun availability(): Availability = availability
    @Synchronized internal fun onServiceCreated() { serviceActive = true; availability = Availability.RUNNING }
    @Synchronized internal fun onServiceDestroyed() { serviceActive = false; availability = Availability.STOPPED }
    @Synchronized internal fun onServiceTerminal(context: Context): Boolean {
        enrollmentLease = false
        accessibilityLeases = 0
        availability = Availability.TERMINAL
        val pendingRecovery = AndroidCredentialStore(context.applicationContext).pendingEnrollment() != null
        return accessibilityLeases > 0 || pendingRecovery
    }
    @Synchronized internal fun resetForTests() {
        enrollmentLease = false; accessibilityLeases = 0; serviceActive = false; availability = Availability.STOPPED
    }
}
