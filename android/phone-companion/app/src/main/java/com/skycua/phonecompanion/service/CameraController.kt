package com.skycua.phonecompanion.service

import android.content.Context
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.os.Build
import android.media.CamcorderProfile
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.MethodParamException
import com.skycua.phonecompanion.camera.CameraRuntime
import com.skycua.phonecompanion.camera.CAMERA_V1_MAX_HEIGHT
import com.skycua.phonecompanion.camera.CAMERA_V1_MAX_VIDEO_DURATION_MS
import com.skycua.phonecompanion.camera.CAMERA_V1_MAX_WIDTH
import com.skycua.phonecompanion.camera.isWithinCameraV1Resolution

/** Truthful Camera2 enumeration; capture sessions are activated by CaptureActivity. */
class CameraController(private val context: Context) {
    private val manager = context.getSystemService(CameraManager::class.java)

    fun perform(params: JsonValue.Obj): JsonValue.Obj = when (val operation = params.string("operation")) {
        "enumerate" -> jsonObject { put("cameras", jsonArray(cameraIds().map(::descriptor))) }
        "capabilities" -> {
            val id = params.string("camera_id") ?: bad("camera_id is required")
            jsonObject { put("cameras", jsonArray(listOf(descriptor(id)))) }
        }
        "photo", "video_start", "preview_start" -> {
            ensureCameraPermission()
            val cameraId = params.string("camera_id") ?: bad("camera_id is required")
            if (cameraId !in cameraIds()) throw MethodApplicationException("not_found", "camera '$cameraId' is unavailable")
            val options = params.obj("options") ?: JsonValue.Obj(emptyMap())
            validateCaptureOptions(options)
            CameraRuntime.launch(context, operation, cameraId, options)
        }
        "video_pause" -> CameraRuntime.pause(requireCameraSession(params))
        "video_resume" -> CameraRuntime.resume(requireCameraSession(params))
        "video_stop" -> CameraRuntime.stopVideo(requireCameraSession(params))
        "preview_frame" -> CameraRuntime.previewFrame(requireCameraSession(params))
        "preview_stop" -> CameraRuntime.stopPreview(requireCameraSession(params))
        "controls" -> CameraRuntime.controls(requireCameraSession(params), params.obj("controls") ?: bad("controls are required"))
        null -> bad("camera operation is required")
        else -> bad("unsupported camera operation '$operation'")
    }

    private fun cameraIds(): List<String> = manager?.cameraIdList?.toList().orEmpty()

    private fun descriptor(id: String): JsonValue.Obj {
        val cameraManager = manager ?: throw MethodApplicationException("unsupported_api", "camera service is unavailable")
        if (id !in cameraManager.cameraIdList) throw MethodApplicationException("not_found", "camera '$id' is unavailable")
        val c = cameraManager.getCameraCharacteristics(id)
        val map = c.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        val photoSizes = map?.getOutputSizes(ImageFormat.JPEG).orEmpty()
            .filter { size -> isWithinCameraV1Resolution(size.width, size.height) }
            .map { size ->
            jsonObject { put("width", size.width.toLong()); put("height", size.height.toLong()) }
        }
        val fpsRanges = c.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES).orEmpty().map { range ->
            jsonObject { put("min", range.lower.toLong()); put("max", range.upper.toLong()) }
        }
        val facing = when (c.get(CameraCharacteristics.LENS_FACING)) {
            CameraCharacteristics.LENS_FACING_FRONT -> "front"
            CameraCharacteristics.LENS_FACING_BACK -> "back"
            CameraCharacteristics.LENS_FACING_EXTERNAL -> "external"
            else -> "unknown"
        }
        val capabilities = (c.get(CameraCharacteristics.REQUEST_AVAILABLE_CAPABILITIES) ?: intArrayOf()).toSet()
        val logical = Build.VERSION.SDK_INT >= 28 && CameraCharacteristics.REQUEST_AVAILABLE_CAPABILITIES_LOGICAL_MULTI_CAMERA in capabilities
        val flash = c.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true
        val zoom = if (Build.VERSION.SDK_INT >= 30) c.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE) else null
        val maxDigitalZoom = c.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM)
        val numericId = id.toIntOrNull()
        val videoProfiles = if (numericId == null) emptyList() else listOf(
            CamcorderProfile.QUALITY_2160P,
            CamcorderProfile.QUALITY_1080P,
            CamcorderProfile.QUALITY_720P,
            CamcorderProfile.QUALITY_480P,
        ).mapNotNull { quality ->
            if (!CamcorderProfile.hasProfile(numericId, quality)) return@mapNotNull null
            CamcorderProfile.getAll(id, quality)?.let { profiles ->
                val video = profiles.videoProfiles.firstOrNull() ?: return@let null
                jsonObject {
                    put("size", jsonObject { put("width", video.width.toLong()); put("height", video.height.toLong()) })
                    put("fps", video.frameRate.toLong())
                    put("video_mime_type", video.mediaType)
                    put("audio_mime_types", jsonArray(profiles.audioProfiles.map { JsonValue.Str(it.mediaType) }.distinct()))
                }
            }
        }.filter { profile ->
            profile.obj("size")?.let { size ->
                isWithinCameraV1Resolution(size.int("width") ?: 0, size.int("height") ?: 0)
            } == true
        }.distinctBy { profile -> profile.obj("size")?.let { "${it.int("width")}x${it.int("height")}" } }
        return jsonObject {
            put("camera_id", id); put("facing", facing); put("logical", logical)
            put("physical_camera_ids", jsonArray(if (Build.VERSION.SDK_INT >= 28) c.physicalCameraIds.sorted().map(JsonValue::Str) else emptyList()))
            put("photo_sizes", jsonArray(photoSizes)); put("video_profiles", jsonArray(videoProfiles)); put("fps_ranges", jsonArray(fpsRanges))
            put("flash_modes", jsonArray(if (flash) listOf("off", "on", "auto").map(JsonValue::Str) else listOf(JsonValue.Str("off"))))
            put("hardware_torch", flash)
            if (Build.VERSION.SDK_INT >= 33 && flash) c.get(CameraCharacteristics.FLASH_INFO_STRENGTH_MAXIMUM_LEVEL)?.let { put("max_torch_strength", it.toLong()) }
            put("min_zoom", JsonValue.Num((zoom?.lower ?: 1.0f).toDouble()))
            put("max_zoom", JsonValue.Num((zoom?.upper ?: maxDigitalZoom ?: 1.0f).toDouble()))
            put("vendor_extensions", jsonObject {})
            put("remote_autostart", false)
            put("max_capture_size", jsonObject {
                put("width", CAMERA_V1_MAX_WIDTH.toLong())
                put("height", CAMERA_V1_MAX_HEIGHT.toLong())
            })
            put("max_video_duration_ms", CAMERA_V1_MAX_VIDEO_DURATION_MS)
            put("automatic_media_transfer", false)
        }
    }

    private fun validateCaptureOptions(options: JsonValue.Obj) {
        val size = options.obj("size") ?: return
        val width = size.int("width") ?: bad("options.size.width is required")
        val height = size.int("height") ?: bad("options.size.height is required")
        if (!isWithinCameraV1Resolution(width, height)) {
            bad("capture resolution exceeds the V1 1920x1080 limit")
        }
    }

    private fun ensureCameraPermission() {
        if (context.checkSelfPermission(android.Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            throw MethodApplicationException("permission_required", "camera permission is not granted")
        }
    }

    private fun requireCameraSession(params: JsonValue.Obj): String =
        params.string("camera_session_id") ?: bad("camera_session_id is required")

    private fun bad(message: String): Nothing = throw MethodParamException("bad_request", message)
}
