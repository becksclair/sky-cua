package com.skycua.phonecompanion.service

import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.NotificationOp
import com.skycua.phonecompanion.protocol.Protocol
import com.skycua.phonecompanion.protocol.Redaction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Test

/**
 * Verifies the redaction contract on two surfaces:
 *  - [SkyNotificationListenerService.rejectIfRedacted]: content-bearing ops on a
 *    fully-redacted notification are rejected with the `redacted` code, while
 *    dismiss (which carries no content) is always permitted.
 *  - [SkyNotificationListenerService.redactContent]: the title/body the snapshot
 *    emits track the visibility-derived redaction so `VISIBILITY_PRIVATE`
 *    (PARTIAL) never leaks the body the OS flagged as sensitive.
 */
class NotificationRedactionTest {
    private fun expectRedacted(op: NotificationOp) {
        try {
            SkyNotificationListenerService.rejectIfRedacted(op, Redaction.FULL)
            fail("expected ${op.wire} on a FULL-redacted notification to be rejected")
        } catch (e: MethodApplicationException) {
            assertEquals(Protocol.ErrorCodes.REDACTED, e.code)
        }
    }

    @Test
    fun fullRedactionRejectsOpen() {
        expectRedacted(NotificationOp.OPEN)
    }

    @Test
    fun fullRedactionRejectsAction() {
        expectRedacted(NotificationOp.ACTION)
    }

    @Test
    fun fullRedactionRejectsReply() {
        expectRedacted(NotificationOp.REPLY)
    }

    @Test
    fun fullRedactionAllowsDismiss() {
        // Dismiss carries no withheld content, so it must not be gated.
        SkyNotificationListenerService.rejectIfRedacted(NotificationOp.DISMISS, Redaction.FULL)
    }

    @Test
    fun partialRedactionAllowsAllOps() {
        // Partial redaction keeps the title and op affordances (only the body is
        // withheld from the snapshot), so ops are not gated.
        for (op in NotificationOp.entries) {
            SkyNotificationListenerService.rejectIfRedacted(op, Redaction.PARTIAL)
        }
    }

    @Test
    fun noRedactionAllowsAllOps() {
        for (op in NotificationOp.entries) {
            SkyNotificationListenerService.rejectIfRedacted(op, Redaction.NONE)
        }
    }

    @Test
    fun noRedactionKeepsTitleAndBody() {
        val content =
            SkyNotificationListenerService.redactContent("Title", "Body", Redaction.NONE)
        assertEquals("Title", content.title)
        assertEquals("Body", content.body)
    }

    @Test
    fun partialRedactionKeepsTitleButWithholdsBody() {
        // VISIBILITY_PRIVATE maps to PARTIAL: the app/sender label survives, but
        // the body the OS flagged as sensitive must not leak off-device.
        val content =
            SkyNotificationListenerService.redactContent("Title", "Body", Redaction.PARTIAL)
        assertEquals("Title", content.title)
        assertNull("partial redaction must withhold the body", content.body)
    }

    @Test
    fun fullRedactionWithholdsTitleAndBody() {
        val content =
            SkyNotificationListenerService.redactContent("Title", "Body", Redaction.FULL)
        assertNull("full redaction must withhold the title", content.title)
        assertNull("full redaction must withhold the body", content.body)
    }
}
