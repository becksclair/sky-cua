package com.skycua.phonecompanion.protocol

import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EnvelopeTest {
    @Test
    fun parsesWellFormedRequest() {
        val body =
            """{"protocol_version":1,"token":"abc","id":7,"method":"screenshot","params":{"include_overlay":true}}"""
        val request = Envelope.parseRequest(body)
        assertEquals(1L, request.protocolVersion)
        assertEquals("abc", request.token)
        assertEquals(7L, request.id)
        assertEquals("screenshot", request.method)
        assertEquals(true, request.params.bool("include_overlay"))
    }

    @Test
    fun defaultsParamsToEmptyObject() {
        val body = """{"protocol_version":1,"token":"abc","id":1,"method":"health"}"""
        val request = Envelope.parseRequest(body)
        assertTrue(request.params.entries.isEmpty())
    }

    @Test
    fun encodesSuccessEnvelope() {
        val response = RpcResponse.Success(7, jsonObject { put("dispatched", true) })
        val json = Envelope.encodeResponse(response)
        val obj = JsonParser.parseObject(json)
        assertEquals(1L, obj.long("protocol_version"))
        assertEquals(true, obj.bool("ok"))
        assertEquals(7L, obj.long("id"))
        assertEquals(true, obj.obj("result")!!.bool("dispatched"))
    }

    @Test
    fun encodesErrorEnvelope() {
        val response = RpcResponse.Failure(7, Protocol.ErrorCodes.SECURE_WINDOW, "secure")
        val json = Envelope.encodeResponse(response)
        val obj = JsonParser.parseObject(json)
        assertEquals(false, obj.bool("ok"))
        assertEquals("secure_window", obj.obj("error")!!.string("code"))
        assertEquals("secure", obj.obj("error")!!.string("message"))
    }

    @Test
    fun missingIdIsBadRequest() {
        val ex =
            runCatching {
                Envelope.parseRequest("""{"protocol_version":1,"method":"health"}""")
            }.exceptionOrNull()
        assertTrue(ex is EnvelopeException)
        assertEquals(Protocol.ErrorCodes.BAD_REQUEST, (ex as EnvelopeException).code)
    }

    @Test
    fun nonIntegralIdIsBadRequest() {
        val ex =
            runCatching {
                Envelope.parseRequest(
                    """{"protocol_version":1,"id":1.5,"method":"health"}""",
                )
            }.exceptionOrNull()
        assertTrue(ex is EnvelopeException)
    }

    @Test
    fun malformedJsonIsBadRequest() {
        val ex =
            runCatching { Envelope.parseRequest("not json") }.exceptionOrNull()
        assertTrue(ex is EnvelopeException)
        assertEquals(Protocol.ErrorCodes.BAD_REQUEST, (ex as EnvelopeException).code)
    }

    @Test
    fun missingProtocolVersionCarriesId() {
        val ex =
            runCatching {
                Envelope.parseRequest("""{"id":9,"method":"health"}""")
            }.exceptionOrNull() as EnvelopeException
        assertEquals(9L, ex.id)
    }
}
