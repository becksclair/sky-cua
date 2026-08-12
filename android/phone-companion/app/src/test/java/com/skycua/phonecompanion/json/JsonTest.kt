package com.skycua.phonecompanion.json

import java.math.BigInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class JsonTest {
    @Test
    fun parsesNestedObjectAndArray() {
        val obj =
            JsonParser.parseObject(
                """{"a":1,"b":"two","c":[true,false,null],"d":{"e":-3.5}}""",
            )
        assertEquals(1L, obj.long("a"))
        assertEquals("two", obj.string("b"))
        val arr = obj.arr("c")!!
        assertEquals(3, arr.items.size)
        assertEquals(JsonValue.Bool(true), arr.items[0])
        assertEquals(JsonValue.Null, arr.items[2])
        assertEquals(-3.5, (obj.obj("d")!!["e"] as JsonValue.Num).value, 0.0001)
    }

    @Test
    fun roundTripsThroughWriter() {
        val original = """{"x":10,"y":"hi","z":[1,2,3]}"""
        val parsed = JsonParser.parseObject(original)
        val written = JsonWriter.write(parsed)
        val reparsed = JsonParser.parseObject(written)
        assertEquals(10L, reparsed.long("x"))
        assertEquals("hi", reparsed.string("y"))
        assertEquals(3, reparsed.arr("z")!!.items.size)
    }

    @Test
    fun escapesAndUnescapesStrings() {
        val value = "line1\nline2\t\"quoted\"\\back"
        val json = JsonWriter.write(jsonObject { put("s", value) })
        val parsed = JsonParser.parseObject(json)
        assertEquals(value, parsed.string("s"))
    }

    @Test
    fun handlesUnicodeEscape() {
        val parsed = JsonParser.parseObject("""{"s":"é"}""")
        assertEquals("é", parsed.string("s"))
    }

    @Test(expected = JsonParseException::class)
    fun rejectsMalformedUnicodeEscapeAsParseException() {
        JsonParser.parseObject("""{"s":"\uZZZZ"}""")
    }

    @Test
    fun integralNumbersWriteWithoutDecimal() {
        val json = JsonWriter.write(jsonObject { put("n", 42L) })
        assertEquals("""{"n":42}""", json)
    }

    @Test
    fun exactIntegersRoundTripAcrossUnsigned64Boundaries() {
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
            val parsed = JsonParser.parseObject("{\"n\":$raw}")
            assertEquals(BigInteger(raw), (parsed["n"] as JsonValue.IntNum).value)
            assertEquals("{\"n\":$raw}", JsonWriter.write(parsed))
        }
    }

    @Test
    fun exactLongAccessorsRejectOverflow() {
        assertEquals(Long.MAX_VALUE, JsonParser.parseObject("{\"n\":9223372036854775807}").long("n"))
        assertNull(JsonParser.parseObject("{\"n\":9223372036854775808}").long("n"))
    }

    @Test(expected = JsonParseException::class)
    fun rejectsTrailingGarbage() {
        JsonParser.parse("""{"a":1} trailing""")
    }

    @Test(expected = JsonParseException::class)
    fun rejectsUnterminatedString() {
        JsonParser.parse(""""unterminated""")
    }

    @Test
    fun numIsIntegralDetectsFractions() {
        assertTrue(JsonValue.Num(4.0).isIntegral)
        assertFalse(JsonValue.Num(4.5).isIntegral)
    }

    @Test
    fun objAccessorsReturnNullOnTypeMismatch() {
        val obj = JsonParser.parseObject("""{"a":"str"}""")
        assertNull(obj.long("a"))
        assertNull(obj.bool("a"))
    }

    @Test
    fun parsesNestingUpToTheDepthCap() {
        // A document nested exactly at the cap must still parse cleanly.
        val depth = JsonParser.MAX_DEPTH
        val nested = "[".repeat(depth) + "]".repeat(depth)
        val parsed = JsonParser.parse(nested)
        // Drill in and confirm the innermost value is an empty array.
        var value: JsonValue = parsed
        repeat(depth - 1) {
            value = (value as JsonValue.Arr).items.single()
        }
        assertTrue((value as JsonValue.Arr).items.isEmpty())
    }

    @Test(expected = JsonParseException::class)
    fun rejectsArrayNestingPastTheDepthCap() {
        // One level beyond the cap. Without the depth guard this would recurse
        // into a StackOverflowError on the worker thread; the guard converts it
        // to the already-caught JsonParseException.
        val depth = JsonParser.MAX_DEPTH + 1
        JsonParser.parse("[".repeat(depth) + "]".repeat(depth))
    }

    @Test(expected = JsonParseException::class)
    fun rejectsObjectNestingPastTheDepthCap() {
        val depth = JsonParser.MAX_DEPTH + 1
        val open = """{"a":""".repeat(depth)
        val close = "}".repeat(depth)
        JsonParser.parse(open + "1" + close)
    }

    @Test
    fun rejectsDeeplyNestedInputWithoutStackOverflow() {
        // Far past the cap, within the protocol body cap, mirrors the pre-auth
        // DoS vector: the parser must reject it as JsonParseException, never
        // surface a StackOverflowError.
        val depth = 50_000
        try {
            JsonParser.parse("[".repeat(depth))
            throw AssertionError("expected JsonParseException for deeply nested input")
        } catch (_: JsonParseException) {
            // Expected: bounded depth rejection.
        }
    }
}
