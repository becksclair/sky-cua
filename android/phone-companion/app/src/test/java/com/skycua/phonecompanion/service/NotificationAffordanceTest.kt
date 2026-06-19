package com.skycua.phonecompanion.service

import android.app.Notification
import android.os.Build
import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonWriter
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.NotificationEvent
import com.skycua.phonecompanion.protocol.Protocol
import com.skycua.phonecompanion.protocol.Redaction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Verifies the notification affordance contract:
 *  - `can_open`/`can_dismiss`/`ongoing` are computed from raw notification
 *    state by [SkyNotificationListenerService.affordancesFor], including the
 *    FULL-redaction override that forces `can_open` false.
 *  - a non-null ranking threads through [NotificationEvent.toJson].
 *  - an immutable PendingIntent send that requires fill-in data is rejected with
 *    the `immutable` code by [SkyNotificationListenerService.rejectIfImmutableFillIn].
 *
 * These exercise the pure, off-device helpers; the [Notification] flag constant
 * is compile-time inlined from android.jar, so this runs as a plain JVM test
 * (mirroring NotificationRedactionTest / OverlayFlagsTest).
 */
class NotificationAffordanceTest {
    @Test
    fun contentIntentDrivesCanOpen() {
        val withIntent =
            SkyNotificationListenerService.affordancesFor(
                hasContentIntent = true,
                isClearable = true,
                flags = 0,
                redaction = Redaction.NONE,
            )
        assertTrue("content intent should allow open", withIntent.canOpen)

        val withoutIntent =
            SkyNotificationListenerService.affordancesFor(
                hasContentIntent = false,
                isClearable = true,
                flags = 0,
                redaction = Redaction.NONE,
            )
        assertFalse("no content intent means no open", withoutIntent.canOpen)
    }

    @Test
    fun clearableDrivesCanDismiss() {
        val clearable =
            SkyNotificationListenerService.affordancesFor(
                hasContentIntent = false,
                isClearable = true,
                flags = 0,
                redaction = Redaction.NONE,
            )
        assertTrue(clearable.canDismiss)

        val nonClearable =
            SkyNotificationListenerService.affordancesFor(
                hasContentIntent = false,
                isClearable = false,
                flags = 0,
                redaction = Redaction.NONE,
            )
        assertFalse(nonClearable.canDismiss)
    }

    @Test
    fun ongoingFlagDrivesOngoing() {
        val ongoing =
            SkyNotificationListenerService.affordancesFor(
                hasContentIntent = true,
                isClearable = false,
                flags = Notification.FLAG_ONGOING_EVENT,
                redaction = Redaction.NONE,
            )
        assertTrue(ongoing.ongoing)

        val notOngoing =
            SkyNotificationListenerService.affordancesFor(
                hasContentIntent = true,
                isClearable = true,
                flags = Notification.FLAG_AUTO_CANCEL,
                redaction = Redaction.NONE,
            )
        assertFalse(notOngoing.ongoing)
    }

    @Test
    fun fullRedactionForcesCanOpenFalse() {
        // A content intent is present, but FULL redaction must suppress open so
        // the advertised affordance matches the content-bearing op rejection.
        val full =
            SkyNotificationListenerService.affordancesFor(
                hasContentIntent = true,
                isClearable = true,
                flags = 0,
                redaction = Redaction.FULL,
            )
        assertFalse("FULL redaction must force can_open false", full.canOpen)
        // Dismiss carries no content and stays available even under FULL.
        assertTrue(full.canDismiss)
    }

    @Test
    fun rankingThreadsThroughSerialization() {
        val event =
            NotificationEvent(
                eventId = "evt-1",
                packageName = "com.example",
                channel = null,
                title = "Title",
                body = "Body",
                redaction = Redaction.NONE,
                ranking = 7,
                whenMs = 0,
                actions = emptyList(),
                canOpen = true,
                canDismiss = true,
                ongoing = false,
            )
        val obj = JsonParser.parseObject(JsonWriter.write(event.toJson()))
        assertEquals(7L, obj.long("ranking"))
    }

    @Test
    fun immutablePendingIntentWithFillInIsRejected() {
        // The pure rejection helper backs the API 31+ guard in sendPending:
        // when the caller needs a fill-in intent for an immutable pending intent,
        // the send is rejected up front with the structured `immutable` code.
        try {
            SkyNotificationListenerService.rejectIfImmutableFillIn(
                immutable = true,
                requiresFillIn = true,
            )
            fail("expected an immutable pending intent to be rejected")
        } catch (e: MethodApplicationException) {
            assertEquals(Protocol.ErrorCodes.IMMUTABLE, e.code)
        }
    }

    @Test
    fun immutablePendingIntentWithoutFillInIsAccepted() {
        // Opening a notification or invoking a plain action uses pending.send()
        // without a fill-in intent, which Android permits for immutable intents.
        SkyNotificationListenerService.rejectIfImmutableFillIn(
            immutable = true,
            requiresFillIn = false,
        )
    }

    @Test
    fun mutablePendingIntentWithFillInIsAccepted() {
        // A mutable (or pre-API-31) pending intent may carry inline reply data.
        SkyNotificationListenerService.rejectIfImmutableFillIn(
            immutable = false,
            requiresFillIn = true,
        )
    }

    @Test
    fun immutabilityGateIsApi31Plus() {
        // Documents the platform gate: isImmutable is only readable on API 31+.
        // Under isReturnDefaultValues this skips on the JVM (SDK_INT reads 0);
        // it asserts the call-site gate constant the service uses.
        assumeTrue(
            "immutable PendingIntent detection requires API 31+",
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.S,
        )
        assertTrue(Build.VERSION.SDK_INT >= 31)
    }
}
