package com.skycua.phonecompanion.direct

import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LinkEpochTest {
    @Test
    fun acceptsTheFullUnsigned64JsonDomainExactly() {
        val values = listOf(
            "0",
            "1",
            "9007199254740991",
            "9007199254740992",
            "9007199254740993",
            "9223372036854775807",
            "9223372036854775808",
            "18446744073709551615",
        )
        values.forEach { raw ->
            val parsed = JsonParser.parseObject("{\"link_epoch\":$raw}")
            val epoch = parsed.linkEpoch("link_epoch")
            assertEquals(raw, epoch.toString())
            assertEquals("{\"link_epoch\":$raw}", JsonWriter.write(parsed))
            assertEquals(epoch, LinkEpoch.fromBinaryCarrier(epoch!!.toBinaryCarrier()))
        }
    }

    @Test
    fun rejectsNonIntegerAndOutOfRangeJsonEpochs() {
        listOf(
            "-1",
            "18446744073709551616",
            "1.0",
            "1e0",
            "\"1\"",
        ).forEach { raw ->
            assertNull(JsonParser.parseObject("{\"link_epoch\":$raw}").linkEpoch("link_epoch"))
        }
    }

    @Test(expected = IllegalArgumentException::class)
    fun rejectsNonCanonicalAuthEpoch() {
        LinkEpoch.parseCanonical("01")
    }

    @Test(expected = IllegalArgumentException::class)
    fun rejectsNonAsciiAuthEpochDigits() {
        LinkEpoch.parseCanonical("1٢")
    }
}
