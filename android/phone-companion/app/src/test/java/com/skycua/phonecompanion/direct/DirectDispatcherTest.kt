package com.skycua.phonecompanion.direct

import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.protocol.*
import org.junit.Assert.*
import org.junit.Test
import java.util.Base64
import java.io.File

class DirectDispatcherTest {
    private val device = "00000000-0000-4000-8000-000000000001"
    private fun request(method: String, deviceId: String = device, epoch: Long = 7, expiry: Long = 10_000, params: String = "{}") =
        "{\"type\":\"request\",\"request_id\":\"r1\",\"device_id\":\"$deviceId\",\"link_epoch\":$epoch,\"idempotent\":true,\"expires_at_ms\":$expiry,\"method\":\"$method\",\"params\":$params}"
    private fun dispatcher(handler: FakeHandler = FakeHandler()) = DirectRequestDispatcher.forHandler(handler) to handler

    @Test fun validDispatchAndCompanionStatusAliasReturnResponse() {
        val (d, h) = dispatcher()
        val response = d.dispatch(request("companion.status"), device, "7", 100)
        assertTrue(response.contains("\"type\":\"response\"")); assertTrue(response.contains("\"request_id\":\"r1\"")); assertEquals(1, h.healthCalls)
    }

    @Test fun wrongDeviceEpochAndExpiredRequestsDoNotCallHandler() {
        val (d, h) = dispatcher()
        assertTrue(d.dispatch(request("companion.status", deviceId = "00000000-0000-4000-8000-000000000002"), device, "7", 100).contains("device_mismatch"))
        assertTrue(d.dispatch(request("companion.status", epoch = 8), device, "7", 100).contains("epoch_mismatch"))
        assertTrue(d.dispatch(request("companion.status", expiry = 100), device, "7", 100).contains("expired"))
        assertEquals(0, h.healthCalls)
    }

    @Test fun applicationErrorIsStructured() {
        val (d, _) = dispatcher(FakeHandler(appError = true))
        val response = d.dispatch(request("appshot"), device, "7", 100)
        assertTrue(response.contains("\"type\":\"error\"")); assertTrue(response.contains("disabled_service")); assertTrue(response.contains("\"request_id\":\"r1\""))
    }

    @Test fun unknownMethodIsTruthful() {
        val (d, _) = dispatcher()
        assertTrue(d.dispatch(request("not.a.method"), device, "7", 100).contains("unknown_method"))
    }

    @Test fun malformedRequestIsRejectedBeforeHandler() {
        val (d, h) = dispatcher()
        val response = d.dispatch("{\"type\":\"request\",\"request_id\":\"r1\",\"device_id\":\"$device\"", device, "7", 100)
        assertTrue(response.contains("bad_request")); assertEquals(0, h.healthCalls)
    }

    @Test fun appshotResponseUsesContentRefWithoutBase64() {
        val socket = CaptureSocket()
        val sender = ContentTransferSender(socket, { device }, { 7 }, idFactory = { "transfer" })
        val response = DirectRequestDispatcher.forHandler(FakeHandler(appShotBytes = byteArrayOf(1, 2, 3)), { sender })
            .dispatch(request("appshot"), device, "7", 100)
        assertFalse(response.contains("data_base64")); assertTrue(response.contains("content_ref")); assertTrue(socket.text.any { it.contains("content_declare") }); assertTrue(socket.text.any { it.contains("content_commit") })
    }

    @Test fun directContentDescribeAndReleaseStayBoundToCurrentEpoch() {
        val file = File.createTempFile("direct-content", "test").apply { writeText("hello") }
        val received = ReceivedContent("c1", device, 7, "text/plain", "hello.txt", 5, "00".repeat(32), 20_000, file)
        var released = false
        val resolver = object : DirectContentResolver {
            override fun resolve(reference: JsonValue.Obj, expectedEpoch: Long) = received
            override fun describe(contentId: String) = received.takeIf { contentId == "c1" }
            override fun release(contentId: String, expectedEpoch: Long): Boolean =
                (contentId == "c1" && expectedEpoch == 7L).also { released = it }
        }
        val dispatcher = DirectRequestDispatcher.forHandler(FakeHandler(), contentResolver = resolver)
        val described = dispatcher.dispatch(request("content.describe", params = "{\"content_id\":\"c1\"}"), device, "7", 100)
        assertTrue(described.contains("\"content_id\":\"c1\"")); assertTrue(described.contains("\"size_bytes\":5"))
        val release = dispatcher.dispatch(request("content.release", params = "{\"content_id\":\"c1\"}"), device, "7", 100)
        assertTrue(release.contains("\"released\":true")); assertTrue(released)
        file.delete()
    }

    @Test fun cameraCaptureReturnsLocalReferenceAndTransfersOnlyOnExplicitExport() {
        val file = File.createTempFile("camera-local", ".jpg").apply { writeBytes(byteArrayOf(1, 2, 3)) }
        val socket = CaptureSocket()
        val sender = ContentTransferSender(socket, { device }, { 7 }, idFactory = { "transfer" })
        var local: ReceivedContent? = null
        val resolver = object : DirectContentResolver {
            override fun resolve(reference: JsonValue.Obj, expectedEpoch: Long) = local
            override fun describe(contentId: String) = local?.takeIf { it.contentId == contentId }
            override fun release(contentId: String, expectedEpoch: Long) = false
            override fun registerLocal(
                file: File,
                deviceId: String,
                linkEpoch: Long,
                mimeType: String,
                filename: String?,
                source: String,
            ) = ReceivedContent(
                "camera-content", deviceId, linkEpoch, mimeType, filename, file.length(),
                "00".repeat(32), 20_000, file, source,
            ).also { local = it }
        }
        val dispatcher = DirectRequestDispatcher.forHandler(
            FakeHandler(localContentFile = file),
            { sender },
            resolver,
        )
        val captured = dispatcher.dispatch(
            request("camera", params = "{\"operation\":\"photo\",\"camera_id\":\"0\",\"options\":{}}"),
            device,
            "7",
            100,
        )
        assertTrue(captured.contains("camera-content"))
        assertFalse(captured.contains("_content_path"))
        assertEquals(0, socket.binaryCount)
        assertFalse(socket.text.any { it.contains("content_declare") })

        val exported = dispatcher.dispatch(
            request("content.export", params = "{\"content_id\":\"camera-content\"}"),
            device,
            "7",
            100,
        )
        assertTrue(exported.contains("camera-content"))
        assertTrue(socket.text.any { it.contains("content_declare") })
        assertTrue(socket.binaryCount > 0)
        file.delete()
    }

    private class FakeHandler(
        private val appError: Boolean = false,
        private val appShotBytes: ByteArray? = null,
        private val localContentFile: File? = null,
    ) : MethodHandler {
        var healthCalls = 0
        override fun health() = HealthState("test", 1, "pkg", true, true, true, true, false, false, true, null).also { healthCalls++ }
        override fun capabilities(): CapabilitiesState = error("unused")
        override fun accessibilityTree(params: AccessibilityTreeParams): AccessibilityTreeResult = error("unused")
        override fun screenshot(params: ScreenshotParams): ScreenshotResult = error("unused")
        override fun appShot(params: AppShotParams): JsonValue.Obj = if (appError) throw MethodApplicationException("disabled_service", "disabled") else jsonObject { appShotBytes?.let { put("screenshot", jsonObject { put("mime_type", "image/png"); put("data_base64", Base64.getEncoder().encodeToString(it)); put("width", 1); put("height", 1) }) } ?: put("ok", true) }
        override fun gesture(params: GestureParams): JsonValue.Obj = error("unused")
        override fun cursorOverlay(params: CursorOverlayParams): JsonValue.Obj = error("unused")
        override fun overlayActive(params: OverlayActiveParams): JsonValue.Obj = error("unused")
        override fun overlayGesture(params: OverlayGestureParams): JsonValue.Obj = error("unused")
        override fun notifications(params: NotificationsParams): NotificationsResult = error("unused")
        override fun notificationOp(params: NotificationOpParams): JsonValue.Obj = error("unused")
        override fun currentApp(): CurrentAppResult = error("unused")
        override fun appList(params: AppListParams): AppListResult = error("unused")
        override fun appOp(params: AppOpParams): JsonValue.Obj = error("unused")
        override fun camera(params: JsonValue.Obj): JsonValue.Obj = jsonObject {
            val file = localContentFile ?: error("unused")
            put("_content_path", file.absolutePath)
            put("_content_mime", "image/jpeg")
            put("_content_source", "camera_photo")
            put("_content_filename", file.name)
            put("cameras", com.skycua.phonecompanion.json.jsonArray(emptyList()))
        }
    }

    private class CaptureSocket : DirectSocket {
        val text = mutableListOf<String>()
        var binaryCount = 0
        override fun connect(endpoint: String, listener: DirectSocket.Listener) = Unit
        override fun sendText(frame: String): Boolean { text += frame; return true }
        override fun sendBinary(bytes: ByteArray): Boolean { binaryCount++; return true }
        override fun close() = Unit
    }
}
