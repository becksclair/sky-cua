package com.skycua.phonecompanion.camera

/** V1 captures stay bounded and phone-local until an explicit content export. */
const val CAMERA_V1_MAX_WIDTH = 1920
const val CAMERA_V1_MAX_HEIGHT = 1080
const val CAMERA_V1_MAX_VIDEO_DURATION_MS = 60_000L

fun isWithinCameraV1Resolution(width: Int, height: Int): Boolean =
    width > 0 && height > 0 &&
        ((width <= CAMERA_V1_MAX_WIDTH && height <= CAMERA_V1_MAX_HEIGHT) ||
            (width <= CAMERA_V1_MAX_HEIGHT && height <= CAMERA_V1_MAX_WIDTH))
