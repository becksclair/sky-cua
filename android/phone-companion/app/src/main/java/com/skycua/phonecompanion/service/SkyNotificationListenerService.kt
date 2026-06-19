package com.skycua.phonecompanion.service

import android.app.Notification
import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.app.RemoteInput
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.NotificationAction
import com.skycua.phonecompanion.protocol.NotificationEvent
import com.skycua.phonecompanion.protocol.NotificationOp
import com.skycua.phonecompanion.protocol.NotificationOpParams
import com.skycua.phonecompanion.protocol.NotificationsParams
import com.skycua.phonecompanion.protocol.NotificationsResult
import com.skycua.phonecompanion.protocol.Protocol
import com.skycua.phonecompanion.protocol.Redaction
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicReference

/**
 * Receives notification posted/removed events and exposes a bounded, redaction-
 * aware view of recent notifications. Notification operations
 * (open/dismiss/action/reply) require explicit event/action ids from a fresh
 * observation and return structured unavailable errors when the underlying
 * PendingIntent or RemoteInput is missing or no longer valid.
 *
 * Notification content is never persisted; it lives only in an in-memory ring
 * keyed by ephemeral event ids.
 */
class SkyNotificationListenerService : NotificationListenerService() {
    private val active = ConcurrentHashMap<String, Tracked>()
    private var connected = false

    override fun onListenerConnected() {
        super.onListenerConnected()
        connected = true
        instanceRef.set(this)
        try {
            activeNotifications?.forEach { track(it) }
        } catch (_: SecurityException) {
            // Listener not yet fully bound; events will arrive via callbacks.
        }
    }

    override fun onListenerDisconnected() {
        connected = false
        instanceRef.compareAndSet(this, null)
        active.clear()
        super.onListenerDisconnected()
    }

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        sbn ?: return
        track(sbn)
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        sbn ?: return
        active.remove(eventIdFor(sbn))
    }

    val listenerEnabled: Boolean
        get() = connected

    private fun track(sbn: StatusBarNotification) {
        active[eventIdFor(sbn)] = Tracked(sbn)
    }

    /** Returns a bounded snapshot of recent notifications. */
    fun snapshot(params: NotificationsParams): NotificationsResult {
        val all = active.values.sortedByDescending { it.sbn.postTime }
        val limited = all.take(params.max)
        val events = limited.map { it.toEvent() }
        return NotificationsResult(
            listenerEnabled = connected,
            events = events,
            truncated = all.size > limited.size,
        )
    }

    /** Performs an open/dismiss/action/reply against an explicit event id. */
    fun performOp(params: NotificationOpParams) {
        val tracked =
            active[params.eventId]
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.GONE,
                    "notification ${params.eventId} is no longer present",
                )
        // Content-bearing ops on a fully-redacted notification must not act:
        // the title/body were nulled in the snapshot, so acting on it would
        // surface content the redaction policy withheld. Dismiss is allowed.
        rejectIfRedacted(params.op, redactionFor(tracked.sbn.notification))
        when (params.op) {
            NotificationOp.OPEN -> open(tracked)
            NotificationOp.DISMISS -> cancelNotification(tracked.sbn.key)
            NotificationOp.ACTION -> invokeAction(tracked, params.actionId!!, replyText = null)
            NotificationOp.REPLY ->
                invokeAction(tracked, params.actionId!!, replyText = params.replyText!!)
        }
    }

    private fun open(tracked: Tracked) {
        val contentIntent =
            tracked.sbn.notification.contentIntent
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.PENDING_INTENT_MISSING,
                    "notification has no content intent",
                )
        sendPending(contentIntent, fillIn = null)
    }

    private fun invokeAction(tracked: Tracked, actionId: String, replyText: String?) {
        val actions =
            tracked.sbn.notification.actions
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.GONE,
                    "notification has no actions",
                )
        val index =
            actionId.removePrefix(ACTION_PREFIX).toIntOrNull()
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.GONE,
                    "invalid action id $actionId",
                )
        if (index < 0 || index >= actions.size) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.GONE,
                "action $actionId no longer exists",
            )
        }
        val action = actions[index]
        val pending =
            action.actionIntent
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.PENDING_INTENT_MISSING,
                    "action has no pending intent",
                )

        if (replyText != null) {
            val remoteInputs = action.remoteInputs
            if (remoteInputs.isNullOrEmpty()) {
                throw MethodApplicationException(
                    Protocol.ErrorCodes.REPLY_UNAVAILABLE,
                    "action does not accept inline replies",
                )
            }
            val fillIn = Intent()
            val results = Bundle()
            remoteInputs.forEach { ri -> results.putCharSequence(ri.resultKey, replyText) }
            RemoteInput.addResultsToIntent(remoteInputs, fillIn, results)
            sendPending(pending, fillIn)
        } else {
            sendPending(pending, fillIn = null)
        }
    }

    private fun sendPending(pending: PendingIntent, fillIn: Intent?) {
        // An immutable PendingIntent cannot accept a fill-in or be re-targeted;
        // an inline reply against it would silently drop our payload. Plain
        // open/action sends have no fill-in and remain valid even when immutable.
        // Detectable only on API 31+; older platforms expose no isImmutable.
        val immutable = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && pending.isImmutable
        rejectIfImmutableFillIn(immutable, fillIn != null)
        try {
            if (fillIn != null) {
                pending.send(this, 0, fillIn)
            } else {
                pending.send()
            }
        } catch (_: PendingIntent.CanceledException) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.CANCELED,
                "pending intent was canceled",
            )
        }
    }

    private fun Tracked.toEvent(): NotificationEvent {
        val n = sbn.notification
        val extras = n.extras
        val title = extras?.getCharSequence(Notification.EXTRA_TITLE)?.toString()
        val body = extras?.getCharSequence(Notification.EXTRA_TEXT)?.toString()
        val channel =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) n.channelId else null
        val redaction = redactionFor(n)
        val full = redaction == Redaction.FULL
        // On FULL redaction the snapshot withholds content, so the advertised
        // affordances must match: no content-open, no actions (content-bearing
        // ops are rejected by rejectIfRedacted). Dismiss still carries no
        // content and stays available.
        val actions = if (full) emptyList() else actionsFor(n)
        val affordances =
            affordancesFor(
                hasContentIntent = n.contentIntent != null,
                isClearable = sbn.isClearable,
                flags = n.flags,
                redaction = redaction,
            )
        val redacted = redactContent(title, body, redaction)
        return NotificationEvent(
            eventId = eventIdFor(sbn),
            packageName = sbn.packageName,
            channel = channel,
            title = redacted.title,
            body = redacted.body,
            redaction = redaction,
            ranking = rankingFor(sbn.key),
            whenMs = sbn.postTime,
            actions = actions,
            canOpen = affordances.canOpen,
            canDismiss = affordances.canDismiss,
            ongoing = affordances.ongoing,
        )
    }

    /**
     * Reads the system rank for [key] from the current ranking map. Defensive
     * against an absent key, an unbound listener, or platforms that withhold the
     * ranking: any of those yield null rather than a fabricated rank.
     */
    private fun rankingFor(key: String?): Int? {
        key ?: return null
        return try {
            val ranking = NotificationListenerService.Ranking()
            if (currentRanking?.getRanking(key, ranking) == true) ranking.rank else null
        } catch (_: Exception) {
            null
        }
    }

    private fun redactionFor(n: Notification): Redaction =
        when (n.visibility) {
            Notification.VISIBILITY_SECRET -> Redaction.FULL
            Notification.VISIBILITY_PRIVATE -> Redaction.PARTIAL
            else -> Redaction.NONE
        }

    private fun actionsFor(n: Notification): List<NotificationAction> {
        val actions = n.actions ?: return emptyList()
        return actions.mapIndexed { index, action ->
            val hasReply = !action.remoteInputs.isNullOrEmpty()
            NotificationAction(
                actionId = "$ACTION_PREFIX$index",
                title = action.title?.toString() ?: "action $index",
                isReply = hasReply,
            )
        }
    }

    private fun eventIdFor(sbn: StatusBarNotification): String = "evt-${sbn.key}"

    private class Tracked(val sbn: StatusBarNotification)

    /** Advertised affordances for a notification, derived from its flags/state. */
    data class Affordances(
        val canOpen: Boolean,
        val canDismiss: Boolean,
        val ongoing: Boolean,
    )

    /** The title/body actually emitted in the snapshot after redaction. */
    data class RedactedContent(
        val title: String?,
        val body: String?,
    )

    companion object {
        private const val ACTION_PREFIX = "action-"

        private val instanceRef = AtomicReference<SkyNotificationListenerService?>()

        fun instance(): SkyNotificationListenerService? = instanceRef.get()

        /**
         * Computes the affordance booleans the snapshot advertises from the raw
         * notification state. Pure and off-device testable.
         *
         * - `can_open`: the notification carries a content intent. Forced false
         *   under [Redaction.FULL] because content-open is rejected by
         *   [rejectIfRedacted] and the snapshot withholds the content.
         * - `can_dismiss`: the notification is clearable (`sbn.isClearable`).
         * - `ongoing`: the [Notification.FLAG_ONGOING_EVENT] flag is set.
         */
        fun affordancesFor(
            hasContentIntent: Boolean,
            isClearable: Boolean,
            flags: Int,
            redaction: Redaction,
        ): Affordances {
            val ongoing = (flags and Notification.FLAG_ONGOING_EVENT) != 0
            val canOpen = hasContentIntent && redaction != Redaction.FULL
            return Affordances(
                canOpen = canOpen,
                canDismiss = isClearable,
                ongoing = ongoing,
            )
        }

        /**
         * Applies the redaction policy to the title/body the snapshot emits.
         * Pure and off-device testable so the leak boundary is unit-tested
         * without an Android [Notification].
         *
         * - [Redaction.NONE]: both fields pass through unchanged.
         * - [Redaction.PARTIAL]: the title (app/sender label) is kept, but the
         *   body is withheld, mirroring Android's private-lockscreen behavior
         *   where `VISIBILITY_PRIVATE` hides content the OS flagged as sensitive
         *   on untrusted surfaces.
         * - [Redaction.FULL]: both title and body are withheld.
         */
        fun redactContent(
            title: String?,
            body: String?,
            redaction: Redaction,
        ): RedactedContent =
            when (redaction) {
                Redaction.NONE -> RedactedContent(title = title, body = body)
                Redaction.PARTIAL -> RedactedContent(title = title, body = null)
                Redaction.FULL -> RedactedContent(title = null, body = null)
            }

        /**
         * Rejects an immutable PendingIntent only when a fill-in intent is needed
         * (inline reply / RemoteInput). Plain open/action sends use
         * `PendingIntent.send()` without fill-in, so immutable intents are still
         * valid there. Pure and off-device testable; the platform
         * `isImmutable`/API-level check stays at the call site.
         *
         * @throws MethodApplicationException with [Protocol.ErrorCodes.IMMUTABLE]
         *   when [immutable] and [requiresFillIn] are both true.
         */
        fun rejectIfImmutableFillIn(immutable: Boolean, requiresFillIn: Boolean) {
            if (immutable && requiresFillIn) {
                throw MethodApplicationException(
                    Protocol.ErrorCodes.IMMUTABLE,
                    "pending intent is immutable and cannot be invoked with a fill-in",
                )
            }
        }

        /**
         * Enforces the `redacted` contract: a fully-redacted notification has its
         * title/body withheld from the snapshot, so content-bearing ops
         * (open/action/reply) must be rejected rather than acting on hidden
         * content. Dismiss carries no content and stays allowed regardless of
         * redaction. Pure and side-effect free so it is unit-testable off-device.
         *
         * @throws MethodApplicationException with [Protocol.ErrorCodes.REDACTED]
         *   when a content-bearing op targets a [Redaction.FULL] notification.
         */
        fun rejectIfRedacted(op: NotificationOp, redaction: Redaction) {
            if (redaction != Redaction.FULL) return
            when (op) {
                NotificationOp.OPEN, NotificationOp.ACTION, NotificationOp.REPLY ->
                    throw MethodApplicationException(
                        Protocol.ErrorCodes.REDACTED,
                        "notification content is fully redacted; ${op.wire} is not permitted",
                    )
                NotificationOp.DISMISS -> Unit
            }
        }
    }
}
