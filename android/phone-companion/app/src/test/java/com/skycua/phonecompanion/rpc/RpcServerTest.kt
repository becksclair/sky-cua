package com.skycua.phonecompanion.rpc

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import java.io.ByteArrayInputStream
import java.nio.charset.StandardCharsets

class RpcServerTest {
    private fun stream(text: String) =
        ByteArrayInputStream(text.toByteArray(StandardCharsets.UTF_8))

    @Test
    fun parsesPostRpcWithBody() {
        val body = """{"id":1,"m":1}""" // 14 bytes
        val raw =
            "POST /rpc HTTP/1.1\r\n" +
                "Host: 127.0.0.1\r\n" +
                "Content-Type: application/json\r\n" +
                "Content-Length: ${body.length}\r\n" +
                "\r\n" +
                body
        val request = RpcServer.readHttpRequest(stream(raw))
        assertEquals("POST", request.method)
        assertEquals("/rpc", request.path)
        assertEquals(body, request.body)
    }

    @Test
    fun stripsQueryFromPath() {
        val raw =
            "GET /rpc?x=1 HTTP/1.1\r\n" +
                "Content-Length: 0\r\n" +
                "\r\n"
        val request = RpcServer.readHttpRequest(stream(raw))
        assertEquals("/rpc", request.path)
    }

    @Test
    fun emptyBodyWhenNoContentLength() {
        val raw = "POST /rpc HTTP/1.1\r\nHost: x\r\n\r\n"
        val request = RpcServer.readHttpRequest(stream(raw))
        assertEquals("", request.body)
    }

    @Test
    fun rejectsOversizeContentLength() {
        val tooBig = RpcServer.MAX_BODY_BYTES.toLong() + 1
        val raw = "POST /rpc HTTP/1.1\r\nContent-Length: $tooBig\r\n\r\n"
        assertThrows(HttpRequestException::class.java) {
            RpcServer.readHttpRequest(stream(raw))
        }
    }

    @Test
    fun rejectsInvalidContentLength() {
        val raw = "POST /rpc HTTP/1.1\r\nContent-Length: notanumber\r\n\r\n"
        assertThrows(HttpRequestException::class.java) {
            RpcServer.readHttpRequest(stream(raw))
        }
    }

    @Test
    fun rejectsTruncatedBody() {
        val raw = "POST /rpc HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort"
        assertThrows(HttpRequestException::class.java) {
            RpcServer.readHttpRequest(stream(raw))
        }
    }

    @Test
    fun emptyStreamThrows() {
        assertThrows(HttpRequestException::class.java) {
            RpcServer.readHttpRequest(stream(""))
        }
    }
}
