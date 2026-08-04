package com.skycua.phonecompanion.camera

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CameraCaptureContractTest {
    @Test fun acceptsLandscapeAndPortraitFhdButRejectsLargerCapture() {
        assertTrue(isWithinCameraV1Resolution(1920, 1080))
        assertTrue(isWithinCameraV1Resolution(1080, 1920))
        assertFalse(isWithinCameraV1Resolution(3840, 2160))
        assertFalse(isWithinCameraV1Resolution(0, 1080))
    }

    @Test fun videoDurationLimitIsOneMinute() {
        assertTrue(CAMERA_V1_MAX_VIDEO_DURATION_MS == 60_000L)
    }
}
