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
    private val callbacks = mutableSetOf<() -> Unit>()
    @Synchronized
    fun register(callback: (() -> Unit)?) {
        synchronized(callbacks) {
            if (callback == null) callbacks.clear()
            else callbacks.add(callback)
        }
    }
    @Synchronized
    fun unregister(callback: () -> Unit) {
        synchronized(callbacks) { callbacks.remove(callback) }
    }
    fun notifyCommitted() {
        val copy: List<() -> Unit>
        synchronized(callbacks) { copy = callbacks.toList() }
        copy.forEach { it.invoke() }
    }
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

/** Optional dedicated owner; it is started only after the first credential unlock. */
class DirectLinkService : Service() {
    private var pool: MultiHostDirectLinkPool? = null
    private var notificationState: LinkState? = null
    private var notificationCount: Int? = null
    private var terminalState = false
    override fun onCreate() {
        super.onCreate()
        DirectLinkServiceOwner.onServiceCreated()
        startVisibleForeground()
        pool = MultiHostDirectLinkPool(
            applicationContext,
            onTerminal = ::onTerminal,
            onState = { snaps -> updateNotification(pool?.aggregatedState(snaps) ?: LinkState.CONNECTING, snaps.size) },
        ).also { it.start() }
        // Initial state push — single decrypt via hostSnapshots
        val snaps = pool?.hostSnapshots() ?: emptyList()
        updateNotification(pool?.aggregatedState(snaps) ?: LinkState.CONNECTING, snaps.size)
    }
    override fun onDestroy() {
        pool?.stop(); pool = null
        DirectLinkServiceOwner.onServiceDestroyed()
        super.onDestroy()
    }
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY
        .also {
            if (terminalState) {
                terminalState = false
                updateNotification(LinkState.CONNECTING)
                pool?.start()
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

    private fun updateNotification(state: LinkState, count: Int? = null) {
        if (notificationState == state && notificationCount == count) return
        notificationState = state; notificationCount = count
        val base = DirectLinkNotificationText.forState(state)
        // Enrich title/body with paired count when known.
        val copy = if (count != null && count > 0 && state != LinkState.REENROLL_REQUIRED && state != LinkState.DISABLED) {
            base.copy(title = "${base.title} · $count paired", body = if (state == LinkState.CONNECTED) "Connected to $count host${if (count == 1) "" else "s"}" else base.body)
        } else base
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
        val store = AndroidCredentialStore(context.applicationContext)
        val pendingRecovery = store.loadAll().any { it.pendingEnrollment != null } || store.pendingEnrollment() != null
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
        val store = AndroidCredentialStore(context.applicationContext)
        val pendingRecovery = store.loadAll().any { it.pendingEnrollment != null } || store.pendingEnrollment() != null
        return accessibilityLeases > 0 || pendingRecovery
    }
    @Synchronized internal fun resetForTests() {
        enrollmentLease = false; accessibilityLeases = 0; serviceActive = false; availability = Availability.STOPPED
    }
}
