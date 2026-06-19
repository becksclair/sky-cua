package com.skycua.phonecompanion.rpc

import com.skycua.phonecompanion.protocol.Envelope
import com.skycua.phonecompanion.protocol.RpcDispatcher
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketException
import java.nio.charset.StandardCharsets
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * A localhost-only HTTP/1.1 server implementing `POST /rpc`. It binds the
 * loopback interface only and serves exactly one request per TCP connection
 * (`Connection: close`), matching the wire contract. It depends only on the JDK
 * networking primitives present on Android — no third-party HTTP stack.
 *
 * The server is intentionally minimal: it reads a request line, headers, then a
 * Content-Length-bounded body, hands the body to the [RpcDispatcher], and writes
 * the JSON envelope back. Bodies are capped to defend against hostile clients.
 */
class RpcServer(
    private val port: Int,
    private val dispatcher: RpcDispatcher,
    private val clock: () -> Long = { System.currentTimeMillis() },
) {
    private val running = AtomicBoolean(false)
    private var serverSocket: ServerSocket? = null
    private val workerIds = AtomicInteger()
    private val workers =
        ThreadPoolExecutor(
            0,
            MAX_WORKERS,
            30L,
            TimeUnit.SECONDS,
            ArrayBlockingQueue(WORKER_QUEUE_CAPACITY),
            { r ->
                Thread(r, "sky-rpc-worker-${workerIds.incrementAndGet()}").apply {
                    isDaemon = true
                }
            },
            ThreadPoolExecutor.AbortPolicy(),
        )
    private var acceptThread: Thread? = null

    val isRunning: Boolean
        get() = running.get()

    @Synchronized
    fun start() {
        if (running.get()) return
        val loopback = InetAddress.getByName(LOOPBACK)
        val socket = ServerSocket(port, BACKLOG, loopback)
        socket.reuseAddress = true
        serverSocket = socket
        running.set(true)
        val thread =
            Thread({ acceptLoop(socket) }, "sky-rpc-accept").apply {
                isDaemon = true
            }
        acceptThread = thread
        thread.start()
    }

    @Synchronized
    fun stop() {
        if (!running.getAndSet(false)) return
        try {
            serverSocket?.close()
        } catch (_: IOException) {
            // Closing is best-effort.
        }
        serverSocket = null
        acceptThread = null
    }

    private fun acceptLoop(socket: ServerSocket) {
        while (running.get()) {
            val client =
                try {
                    socket.accept()
                } catch (_: SocketException) {
                    break // Socket closed by stop().
                } catch (_: IOException) {
                    if (!running.get()) break else continue
                }
            try {
                workers.execute { handleConnection(client) }
            } catch (_: RejectedExecutionException) {
                try {
                    client.close()
                } catch (_: IOException) {
                    // Best-effort rejection close.
                }
            }
        }
    }

    private fun handleConnection(client: Socket) {
        client.use { socket ->
            try {
                socket.soTimeout = READ_TIMEOUT_MS
                val input = BufferedInputStream(socket.getInputStream())
                val output = socket.getOutputStream()
                val request = readHttpRequest(input)
                val responseBody = buildResponseBody(request)
                writeHttpResponse(output, 200, "OK", responseBody)
                output.flush()
            } catch (_: HttpRequestException) {
                try {
                    val output = socket.getOutputStream()
                    writeHttpResponse(output, 400, "Bad Request", BAD_REQUEST_BODY)
                    output.flush()
                } catch (_: IOException) {
                    // Best-effort error reply.
                }
            } catch (_: IOException) {
                // Connection-level failure; nothing to send.
            }
        }
    }

    private fun buildResponseBody(request: HttpRequest): ByteArray {
        if (request.method != "POST" || request.path != "/rpc") {
            // Outside the single supported endpoint; report a protocol-level
            // error envelope rather than HTML.
            return Envelope
                .encodeResponse(
                    com.skycua.phonecompanion.protocol.RpcResponse
                        .Failure(0, "bad_request", "only POST /rpc is supported"),
                ).toByteArray(StandardCharsets.UTF_8)
        }
        val response = dispatcher.handleBody(request.body, clock())
        return Envelope.encodeResponse(response).toByteArray(StandardCharsets.UTF_8)
    }

    private fun writeHttpResponse(
        output: OutputStream,
        status: Int,
        reason: String,
        body: ByteArray,
    ) {
        val header =
            buildString {
                append("HTTP/1.1 ").append(status).append(' ').append(reason).append("\r\n")
                append("Content-Type: application/json\r\n")
                append("Content-Length: ").append(body.size).append("\r\n")
                append("Connection: close\r\n")
                append("\r\n")
            }
        output.write(header.toByteArray(StandardCharsets.US_ASCII))
        output.write(body)
    }

    companion object {
        const val LOOPBACK = "127.0.0.1"
        private const val BACKLOG = 8
        private const val READ_TIMEOUT_MS = 15_000
        const val MAX_WORKERS = 8
        const val WORKER_QUEUE_CAPACITY = 16
        const val MAX_BODY_BYTES = 32 * 1024 * 1024 // 32 MiB, matching host cap.
        private const val MAX_HEADER_BYTES = 64 * 1024
        private val BAD_REQUEST_BODY = "{\"ok\":false}".toByteArray(StandardCharsets.UTF_8)

        /**
         * Reads one HTTP/1.1 request: the request line, headers, then a
         * Content-Length-bounded body. Exposed for unit testing the line/header
         * parsing against in-memory streams.
         */
        fun readHttpRequest(input: InputStream): HttpRequest {
            val requestLine = readLine(input) ?: throw HttpRequestException("empty request")
            val parts = requestLine.split(' ')
            if (parts.size < 2) throw HttpRequestException("malformed request line")
            val method = parts[0]
            val path = parts[1].substringBefore('?')

            var contentLength = 0
            var headerBytes = requestLine.length
            while (true) {
                val line = readLine(input) ?: throw HttpRequestException("unterminated headers")
                if (line.isEmpty()) break
                headerBytes += line.length
                if (headerBytes > MAX_HEADER_BYTES) throw HttpRequestException("headers too large")
                val idx = line.indexOf(':')
                if (idx > 0) {
                    val name = line.substring(0, idx).trim().lowercase()
                    val value = line.substring(idx + 1).trim()
                    if (name == "content-length") {
                        contentLength =
                            value.toIntOrNull()
                                ?: throw HttpRequestException("invalid content-length")
                        if (contentLength < 0 || contentLength > MAX_BODY_BYTES) {
                            throw HttpRequestException("content-length out of bounds")
                        }
                    }
                }
            }

            val body = readBody(input, contentLength)
            return HttpRequest(method, path, body)
        }

        private fun readBody(input: InputStream, length: Int): String {
            if (length == 0) return ""
            val buffer = ByteArrayOutputStream(minOf(length, 64 * 1024))
            val chunk = ByteArray(64 * 1024)
            var remaining = length
            while (remaining > 0) {
                val toRead = minOf(remaining, chunk.size)
                val read = input.read(chunk, 0, toRead)
                if (read < 0) throw HttpRequestException("truncated body")
                buffer.write(chunk, 0, read)
                remaining -= read
            }
            return buffer.toString(StandardCharsets.UTF_8.name())
        }

        /** Reads a CRLF- or LF-terminated line, returning null at end of stream. */
        private fun readLine(input: InputStream): String? {
            val sb = StringBuilder()
            var sawAny = false
            while (true) {
                val b = input.read()
                if (b < 0) {
                    return if (sawAny) sb.toString() else null
                }
                sawAny = true
                when (b) {
                    '\n'.code -> return sb.toString()
                    '\r'.code -> {
                        // Peek for the LF; HTTP uses CRLF.
                        continue
                    }
                    else -> {
                        sb.append(b.toChar())
                        if (sb.length > MAX_HEADER_BYTES) {
                            throw HttpRequestException("line too long")
                        }
                    }
                }
            }
        }
    }
}

/** A parsed HTTP request: method, path, and decoded UTF-8 body. */
data class HttpRequest(
    val method: String,
    val path: String,
    val body: String,
)

class HttpRequestException(message: String) : IOException(message)
