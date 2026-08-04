package com.skycua.phonecompanion.direct

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import java.util.concurrent.TimeUnit

/** Production outbound WebSocket adapter. It never binds or accepts a phone port. */
class OkHttpDirectSocket(
    private val client: OkHttpClient = OkHttpClient.Builder()
        .pingInterval(20, TimeUnit.SECONDS)
        .retryOnConnectionFailure(false)
        .build(),
) : DirectSocket {
    private var webSocket: WebSocket? = null

    override fun connect(endpoint: String, listener: DirectSocket.Listener) {
        val request = Request.Builder().url(endpoint).build()
        webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) = listener.onOpen()
            override fun onMessage(webSocket: WebSocket, text: String) = listener.onText(text)
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) = listener.onBinary(bytes.toByteArray())
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) = listener.onClosed(t)
            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = listener.onClosed(null)
        })
    }

    override fun sendText(frame: String): Boolean = webSocket?.send(frame) == true
    override fun sendBinary(bytes: ByteArray): Boolean = webSocket?.send(ByteString.of(*bytes)) == true
    override fun close() { webSocket?.close(1000, "closed"); webSocket = null }
}

class OkHttpDirectSocketFactory : DirectSocketFactory {
    override fun create(): DirectSocket = OkHttpDirectSocket()
}
