package com.skycua.phonecompanion.setup

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SetupExtrasTest {
    @Test
    fun parsesValidToken() {
        val parsed = SetupExtras.parse("tok-123", 1_718_600_000_000L)
        assertEquals("tok-123", parsed!!.token)
        assertEquals(1_718_600_000_000L, parsed.expiresAtMs)
    }

    @Test
    fun rejectsNullToken() {
        assertNull(SetupExtras.parse(null, 1000))
    }

    @Test
    fun rejectsBlankToken() {
        assertNull(SetupExtras.parse("   ", 1000))
    }

    @Test
    fun rejectsNonPositiveExpiry() {
        assertNull(SetupExtras.parse("tok", 0))
        assertNull(SetupExtras.parse("tok", -5))
    }

    @Test
    fun exposesDocumentedExtraKeys() {
        assertEquals("sky_cua_rpc_token_file", SetupExtras.EXTRA_TOKEN_FILE)
        assertEquals("sky_cua_rpc_token_expires_at_ms", SetupExtras.EXTRA_TOKEN_EXPIRES_AT_MS)
    }
}
