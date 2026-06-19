package com.skycua.phonecompanion.screenshot

import com.skycua.phonecompanion.protocol.Protocol
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Verifies that platform screenshot error codes map to the structured wire
 * contract codes (secure_window/unsupported_api/disabled_service/oem_policy/
 * throttled/transient). The platform constants are compile-time inlined.
 */
class ScreenshotClassifierTest {
    @Test
    fun secureWindowMapsToSecureWindow() {
        assertEquals(
            Protocol.ErrorCodes.SECURE_WINDOW,
            ScreenshotClassifier.codeFor(ScreenshotClassifier.ERROR_SECURE_WINDOW),
        )
    }

    @Test
    fun noAccessibilityAccessMapsToDisabledService() {
        assertEquals(
            Protocol.ErrorCodes.DISABLED_SERVICE,
            ScreenshotClassifier.codeFor(ScreenshotClassifier.ERROR_NO_ACCESSIBILITY_ACCESS),
        )
    }

    @Test
    fun intervalTooShortMapsToThrottled() {
        assertEquals(
            Protocol.ErrorCodes.THROTTLED,
            ScreenshotClassifier.codeFor(ScreenshotClassifier.ERROR_INTERVAL_TIME_SHORT),
        )
    }

    @Test
    fun invalidDisplayMapsToUnsupportedApi() {
        assertEquals(
            Protocol.ErrorCodes.UNSUPPORTED_API,
            ScreenshotClassifier.codeFor(ScreenshotClassifier.ERROR_INVALID_DISPLAY),
        )
    }

    @Test
    fun invalidWindowMapsToUnsupportedApi() {
        assertEquals(
            Protocol.ErrorCodes.UNSUPPORTED_API,
            ScreenshotClassifier.codeFor(ScreenshotClassifier.ERROR_INVALID_WINDOW),
        )
    }

    @Test
    fun internalMapsToTransient() {
        assertEquals(
            Protocol.ErrorCodes.TRANSIENT,
            ScreenshotClassifier.codeFor(ScreenshotClassifier.ERROR_INTERNAL),
        )
    }

    @Test
    fun unknownCodeMapsToTransient() {
        assertEquals(Protocol.ErrorCodes.TRANSIENT, ScreenshotClassifier.codeFor(999))
    }

    @Test
    fun fromErrorCodeCarriesStructuredException() {
        val ex = ScreenshotClassifier.fromErrorCode(ScreenshotClassifier.ERROR_SECURE_WINDOW)
        assertEquals(Protocol.ErrorCodes.SECURE_WINDOW, ex.code)
    }
}
