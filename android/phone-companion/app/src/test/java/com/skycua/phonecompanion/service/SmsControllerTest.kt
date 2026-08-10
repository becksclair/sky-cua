package com.skycua.phonecompanion.service

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class SmsControllerTest {
    @Test
    fun providerProjectionUsesOnlyPortableIngressColumns() {
        assertArrayEquals(
            arrayOf("_id", "thread_id", "address", "date", "date_sent", "read", "status", "type", "body"),
            SmsController.SMS_PROVIDER_PROJECTION,
        )
    }

    @Test
    fun querySelectionPagesOnlyInboundMessages() {
        val (selection, args) = smsSelection(100, 200, 150, 9)

        assertEquals(
            "date >= ? AND date < ? AND type = ? AND (date > ? OR (date = ? AND _id > ?))",
            selection,
        )
        assertArrayEquals(arrayOf("100", "200", "1", "150", "150", "9"), args)
    }
}
