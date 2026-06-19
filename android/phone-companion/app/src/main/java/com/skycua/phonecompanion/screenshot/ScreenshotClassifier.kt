package com.skycua.phonecompanion.screenshot

import android.accessibilityservice.AccessibilityService
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.Protocol

/**
 * Maps `AccessibilityService.takeScreenshot` failure codes to the structured
 * wire-contract screenshot error codes. Keeping this mapping in one place makes
 * the classification rules unit-testable without invoking the platform service.
 *
 * The integer error constants mirror the platform's
 * `AccessibilityService.ERROR_TAKE_SCREENSHOT_*` values from the compiled SDK,
 * so they stay in sync with the SDK the app targets. Each is a compile-time
 * constant, so the mapping also resolves correctly in plain JVM unit tests.
 */
object ScreenshotClassifier {
    const val ERROR_INTERNAL = AccessibilityService.ERROR_TAKE_SCREENSHOT_INTERNAL_ERROR
    const val ERROR_NO_ACCESSIBILITY_ACCESS =
        AccessibilityService.ERROR_TAKE_SCREENSHOT_NO_ACCESSIBILITY_ACCESS
    const val ERROR_INTERVAL_TIME_SHORT =
        AccessibilityService.ERROR_TAKE_SCREENSHOT_INTERVAL_TIME_SHORT
    const val ERROR_INVALID_DISPLAY =
        AccessibilityService.ERROR_TAKE_SCREENSHOT_INVALID_DISPLAY
    const val ERROR_INVALID_WINDOW =
        AccessibilityService.ERROR_TAKE_SCREENSHOT_INVALID_WINDOW
    const val ERROR_SECURE_WINDOW =
        AccessibilityService.ERROR_TAKE_SCREENSHOT_SECURE_WINDOW

    /** Maps a platform error code to a structured wire-contract error code. */
    fun codeFor(errorCode: Int): String =
        when (errorCode) {
            ERROR_NO_ACCESSIBILITY_ACCESS -> Protocol.ErrorCodes.DISABLED_SERVICE
            ERROR_INTERVAL_TIME_SHORT -> Protocol.ErrorCodes.THROTTLED
            ERROR_INVALID_DISPLAY, ERROR_INVALID_WINDOW -> Protocol.ErrorCodes.UNSUPPORTED_API
            ERROR_SECURE_WINDOW -> Protocol.ErrorCodes.SECURE_WINDOW
            ERROR_INTERNAL -> Protocol.ErrorCodes.TRANSIENT
            else -> Protocol.ErrorCodes.TRANSIENT
        }

    /** Maps a platform error code to a structured [MethodApplicationException]. */
    fun fromErrorCode(errorCode: Int): MethodApplicationException =
        MethodApplicationException(codeFor(errorCode), "screenshot failed (code $errorCode)")
}
