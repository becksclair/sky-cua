package com.skycua.phonecompanion.camera

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.BitmapFactory
import android.media.MediaMetadataRetriever
import android.hardware.camera2.CameraManager
import android.util.Size
import androidx.camera.camera2.interop.Camera2CameraInfo
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.FocusMeteringAction
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.video.FileOutputOptions
import androidx.camera.video.Quality
import androidx.camera.video.QualitySelector
import androidx.camera.video.Recorder
import androidx.camera.video.Recording
import androidx.camera.video.VideoCapture
import androidx.camera.video.VideoRecordEvent
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import com.skycua.phonecompanion.CaptureActivity
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.doubleValue
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.protocol.MethodApplicationException
import java.io.File
import java.util.UUID
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

object CameraRuntime {
    private data class Launch(
        val cameraId: String,
        val operation: String,
        val options: JsonValue.Obj,
        val result: CompletableFuture<JsonValue.Obj>,
    )

    private data class Session(
        val id: String,
        val cameraId: String,
        val activity: CaptureActivity,
        val provider: ProcessCameraProvider,
        val camera: Camera,
        val previewView: PreviewView,
        val imageCapture: ImageCapture?,
        val recording: Recording?,
        val startedAtMs: Long,
        val videoResult: CompletableFuture<JsonValue.Obj>? = null,
        val autoStop: Runnable? = null,
    )

    private val launches = ConcurrentHashMap<String, Launch>()
    private val sessions = ConcurrentHashMap<String, Session>()

    fun launch(context: Context, operation: String, cameraId: String, options: JsonValue.Obj): JsonValue.Obj {
        val requestId = UUID.randomUUID().toString()
        val result = CompletableFuture<JsonValue.Obj>()
        launches[requestId] = Launch(cameraId, operation, options, result)
        try {
            context.startActivity(
                Intent(context, CaptureActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                    .putExtra(CaptureActivity.EXTRA_REQUEST_ID, requestId),
            )
            return result.get(40, TimeUnit.SECONDS)
        } catch (error: Throwable) {
            launches.remove(requestId)
            (error.cause as? MethodApplicationException)?.let { throw it }
            throw MethodApplicationException(
                "visible_activity_required",
                "Android did not allow camera capture to become ready: " +
                    (error.cause?.message ?: error.message ?: "timeout"),
            )
        }
    }

    fun attach(activity: CaptureActivity, previewView: PreviewView, requestId: String) {
        val launch = launches.remove(requestId) ?: run { activity.finish(); return }
        val providerFuture = ProcessCameraProvider.getInstance(activity)
        providerFuture.addListener({
            try {
                val provider = providerFuture.get()
                provider.unbindAll()
                val selector = CameraSelector.Builder().addCameraFilter { infos ->
                    infos.filter { Camera2CameraInfo.from(it).cameraId == launch.cameraId }
                }.build()
                val preview = Preview.Builder().build().also { it.surfaceProvider = previewView.surfaceProvider }
                when (launch.operation) {
                    "photo" -> bindPhoto(activity, previewView, provider, selector, preview, launch)
                    "preview_start" -> bindPreview(activity, previewView, provider, selector, preview, launch)
                    "video_start" -> bindVideo(activity, previewView, provider, selector, preview, launch)
                    else -> error("unsupported camera activation")
                }
            } catch (error: Throwable) {
                launch.result.completeExceptionally(error)
                activity.finish()
            }
        }, ContextCompat.getMainExecutor(activity))
    }

    private fun bindPhoto(
        activity: CaptureActivity,
        previewView: PreviewView,
        provider: ProcessCameraProvider,
        selector: CameraSelector,
        preview: Preview,
        launch: Launch,
    ) {
        CameraCaptureService.start(activity, false)
        val capture = imageCapture(launch.options)
        val camera = provider.bindToLifecycle(activity, selector, preview, capture)
        camera.cameraControl.enableTorch(false)
        val output = File(activity.cacheDir, "camera-${UUID.randomUUID()}.jpg")
        capture.takePicture(
            ImageCapture.OutputFileOptions.Builder(output).build(),
            ContextCompat.getMainExecutor(activity),
            object : ImageCapture.OnImageSavedCallback {
                override fun onImageSaved(result: ImageCapture.OutputFileResults) {
                    runCatching {
                        mediaResult(output, launch.cameraId, "image/jpeg", false, null)
                    }.fold(launch.result::complete, launch.result::completeExceptionally)
                    provider.unbindAll()
                    CameraCaptureService.stop(activity)
                    activity.finish()
                }
                override fun onError(exception: ImageCaptureException) {
                    output.delete()
                    launch.result.completeExceptionally(exception)
                    provider.unbindAll()
                    CameraCaptureService.stop(activity)
                    activity.finish()
                }
            },
        )
    }

    private fun bindPreview(
        activity: CaptureActivity,
        previewView: PreviewView,
        provider: ProcessCameraProvider,
        selector: CameraSelector,
        preview: Preview,
        launch: Launch,
    ) {
        CameraCaptureService.start(activity, false)
        val capture = imageCapture(launch.options)
        val camera = provider.bindToLifecycle(activity, selector, preview, capture)
        val id = UUID.randomUUID().toString()
        sessions[id] = Session(id, launch.cameraId, activity, provider, camera, previewView, capture, null, System.currentTimeMillis())
        activity.showActive("Camera preview active")
        launch.result.complete(sessionResult(id))
    }

    private fun bindVideo(
        activity: CaptureActivity,
        previewView: PreviewView,
        provider: ProcessCameraProvider,
        selector: CameraSelector,
        preview: Preview,
        launch: Launch,
    ) {
        val includeAudio = launch.options.bool("include_audio") == true
        if (includeAudio && activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            provider.unbindAll()
            launch.result.completeExceptionally(IllegalStateException("microphone permission is not granted"))
            activity.finish()
            return
        }
        CameraCaptureService.start(activity, includeAudio)
        val quality = QualitySelector.fromOrderedList(listOf(Quality.FHD, Quality.HD, Quality.SD))
        val video = VideoCapture.withOutput(Recorder.Builder().setQualitySelector(quality).build())
        val camera = provider.bindToLifecycle(activity, selector, preview, video)
        val output = File(activity.cacheDir, "camera-${UUID.randomUUID()}.mp4")
        var pending = video.output.prepareRecording(activity, FileOutputOptions.Builder(output).build())
        if (includeAudio) {
            pending = pending.withAudioEnabled()
        }
        val id = UUID.randomUUID().toString()
        val videoResult = CompletableFuture<JsonValue.Obj>()
        var session: Session? = null
        val recording = pending.start(ContextCompat.getMainExecutor(activity)) { event ->
            when (event) {
                is VideoRecordEvent.Start -> {
                    activity.showActive("Video recording active")
                    launch.result.complete(sessionResult(id))
                }
                is VideoRecordEvent.Finalize -> {
                    val current = sessions[id] ?: session
                    current?.autoStop?.let(activity.window.decorView::removeCallbacks)
                    if (event.hasError()) {
                        output.delete()
                        current?.videoResult?.completeExceptionally(
                            IllegalStateException("video recording failed with code ${event.error}"),
                        )
                    } else if (current != null) {
                        runCatching {
                            mediaResult(
                                output,
                                launch.cameraId,
                                "video/mp4",
                                includeAudio,
                                System.currentTimeMillis() - current.startedAtMs,
                            )
                        }.fold(
                            { current.videoResult?.complete(it) },
                            { current.videoResult?.completeExceptionally(it) },
                        )
                    }
                    provider.unbindAll()
                    CameraCaptureService.stop(activity)
                    activity.finish()
                }
                else -> Unit
            }
        }
        val autoStop = Runnable { sessions[id]?.recording?.stop() }
        session = Session(
            id,
            launch.cameraId,
            activity,
            provider,
            camera,
            previewView,
            null,
            recording,
            System.currentTimeMillis(),
            videoResult = videoResult,
            autoStop = autoStop,
        )
        sessions[id] = session
        activity.window.decorView.postDelayed(autoStop, CAMERA_V1_MAX_VIDEO_DURATION_MS)
    }

    fun pause(id: String): JsonValue.Obj = session(id).let {
        it.recording?.pause() ?: unknown()
        sessionResult(id)
    }

    fun resume(id: String): JsonValue.Obj = session(id).let {
        it.recording?.resume() ?: unknown()
        sessionResult(id)
    }

    fun stopVideo(id: String): JsonValue.Obj {
        val session = session(id)
        val recording = session.recording ?: unknown()
        val result = session.videoResult ?: unknown()
        if (!result.isDone) {
            session.autoStop?.let(session.activity.window.decorView::removeCallbacks)
            recording.stop()
        }
        return try {
            await(result, "video stop")
        } finally {
            sessions.remove(id)
        }
    }

    fun previewFrame(id: String): JsonValue.Obj {
        val session = session(id)
        val capture = session.imageCapture ?: unknown()
        val output = File(session.activity.cacheDir, "preview-${UUID.randomUUID()}.jpg")
        val result = CompletableFuture<JsonValue.Obj>()
        capture.takePicture(
            ImageCapture.OutputFileOptions.Builder(output).build(),
            ContextCompat.getMainExecutor(session.activity),
            object : ImageCapture.OnImageSavedCallback {
                override fun onImageSaved(saved: ImageCapture.OutputFileResults) {
                    runCatching {
                        mediaResult(
                            output,
                            session.cameraId,
                            "image/jpeg",
                            false,
                            null,
                            "camera_preview",
                        )
                    }.fold(result::complete, result::completeExceptionally)
                }
                override fun onError(exception: ImageCaptureException) {
                    output.delete()
                    result.completeExceptionally(exception)
                }
            },
        )
        return await(result, "preview frame")
    }

    fun stopPreview(id: String): JsonValue.Obj {
        val session = sessions.remove(id) ?: unknown()
        val stopped = CompletableFuture<Unit>()
        session.activity.runOnUiThread {
            runCatching {
                session.provider.unbindAll()
                CameraCaptureService.stop(session.activity)
                session.activity.finish()
            }.fold(stopped::complete, stopped::completeExceptionally)
        }
        try {
            stopped.get(5, TimeUnit.SECONDS)
        } catch (error: Throwable) {
            throw MethodApplicationException(
                "camera_stop_failed",
                error.cause?.message ?: error.message ?: "camera preview teardown timed out",
            )
        }
        return sessionResult(id)
    }

    fun controls(id: String, controls: JsonValue.Obj): JsonValue.Obj {
        val session = session(id)
        if (controls.string("stabilization_mode") != null) {
            throw MethodApplicationException("unsupported_api", "runtime stabilization changes are not exposed by this CameraX session")
        }
        controls["zoom"]?.doubleValue()?.toFloat()?.let(session.camera.cameraControl::setZoomRatio)
        val torch = controls.bool("torch_enabled")
        val strength = controls.int("torch_strength")
        if (strength != null) {
            val manager = session.activity.getSystemService(CameraManager::class.java)
            if (torch == false) manager.setTorchMode(session.cameraId, false)
            else manager.turnOnTorchWithStrengthLevel(session.cameraId, strength)
        } else {
            torch?.let(session.camera.cameraControl::enableTorch)
        }
        controls.int("exposure_compensation")?.let(session.camera.cameraControl::setExposureCompensationIndex)
        val x = controls["focus_x"]?.doubleValue()?.toFloat()
        val y = controls["focus_y"]?.doubleValue()?.toFloat()
        if ((x == null) != (y == null)) {
            throw MethodApplicationException("bad_request", "focus_x and focus_y must be provided together")
        }
        if (x != null && y != null) {
            val point = session.previewView.meteringPointFactory.createPoint(
                x.coerceIn(0f, 1f) * session.previewView.width,
                y.coerceIn(0f, 1f) * session.previewView.height,
            )
            session.camera.cameraControl.startFocusAndMetering(FocusMeteringAction.Builder(point).build())
        }
        return sessionResult(id)
    }

    fun activityDestroyed(activity: CaptureActivity) {
        sessions.values.filter { it.activity === activity }.forEach {
            sessions.remove(it.id)
            it.autoStop?.let(activity.window.decorView::removeCallbacks)
            runCatching { it.recording?.stop() }
            runCatching { it.provider.unbindAll() }
            CameraCaptureService.stop(activity)
            it.videoResult?.completeExceptionally(IllegalStateException("capture activity closed"))
        }
    }

    private fun imageCapture(options: JsonValue.Obj): ImageCapture {
        val builder = ImageCapture.Builder().setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY)
        val requested = options.obj("size")
        val target = if (requested == null) {
            Size(CAMERA_V1_MAX_WIDTH, CAMERA_V1_MAX_HEIGHT)
        } else {
            requested.let {
            val width = it.int("width") ?: 0
            val height = it.int("height") ?: 0
            if (!isWithinCameraV1Resolution(width, height)) {
                throw MethodApplicationException(
                    "capture_limit_exceeded",
                    "capture resolution exceeds the V1 1920x1080 limit",
                )
            }
            Size(width, height)
            }
        }
        builder.setResolutionSelector(
            ResolutionSelector.Builder()
                .setResolutionStrategy(
                    ResolutionStrategy(target, ResolutionStrategy.FALLBACK_RULE_CLOSEST_LOWER),
                )
                .build(),
        )
        builder.setFlashMode(
            when (options.string("flash")) {
                "on" -> ImageCapture.FLASH_MODE_ON
                "auto" -> ImageCapture.FLASH_MODE_AUTO
                else -> ImageCapture.FLASH_MODE_OFF
            },
        )
        return builder.build()
    }

    private fun mediaResult(
        file: File,
        cameraId: String,
        mime: String,
        audio: Boolean,
        duration: Long?,
        source: String = if (mime.startsWith("image/")) "camera_photo" else "camera_video",
    ): JsonValue.Obj {
        val (width, height) = if (mime.startsWith("image/")) {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(file.absolutePath, bounds)
            bounds.outWidth to bounds.outHeight
        } else {
            val retriever = MediaMetadataRetriever()
            try {
                retriever.setDataSource(file.absolutePath)
                (retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull() ?: 0) to
                    (retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull() ?: 0)
            } finally {
                retriever.release()
            }
        }
        if (!isWithinCameraV1Resolution(width, height)) {
            file.delete()
            throw MethodApplicationException(
                "capture_limit_exceeded",
                "captured media exceeded the V1 1920x1080 limit",
            )
        }
        return jsonObject {
            put("_content_path", file.absolutePath)
            put("_content_mime", mime)
            put("_content_source", source)
            put("_content_filename", file.name)
            put("cameras", jsonArray(emptyList()))
            put("metadata", jsonObject {
                put("camera_id", cameraId)
                put("size", jsonObject { put("width", width.toLong()); put("height", height.toLong()) })
                put("mime_type", mime)
                duration?.let { put("duration_ms", it) }
                put("audio_included", audio)
            })
        }
    }

    private fun await(future: CompletableFuture<JsonValue.Obj>, label: String): JsonValue.Obj =
        try {
            future.get(40, TimeUnit.SECONDS)
        } catch (error: Throwable) {
            (error.cause as? MethodApplicationException)?.let { throw it }
            throw MethodApplicationException(
                "camera_capture_failed",
                "$label failed: ${error.cause?.message ?: error.message ?: "timeout"}",
            )
        }

    private fun session(id: String): Session = sessions[id] ?: unknown()
    private fun sessionResult(id: String): JsonValue.Obj =
        jsonObject { put("camera_session_id", id); put("cameras", jsonArray(emptyList())) }
    private fun unknown(): Nothing =
        throw MethodApplicationException("unknown_camera_session", "camera session is not active")
}
