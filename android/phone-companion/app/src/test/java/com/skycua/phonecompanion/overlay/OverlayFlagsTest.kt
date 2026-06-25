package com.skycua.phonecompanion.overlay

import android.view.WindowManager
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Verifies the overlay window is non-focusable and non-touchable so taps pass
 * through to the underlying app. The flag constants are compile-time inlined
 * from android.jar, so this runs as a plain JVM unit test.
 */
class OverlayFlagsTest {
    @Test
    fun passThroughFlagsAreNonFocusableAndNonTouchable() {
        val flags = OverlayFlags.passThroughFlags
        assertTrue(
            "overlay must be non-focusable",
            (flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE) != 0,
        )
        assertTrue(
            "overlay must be non-touchable",
            (flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE) != 0,
        )
    }

    @Test
    fun touchableFlagsLetCatcherReceiveTapsWithoutFocus() {
        val flags = OverlayFlags.touchableFlags
        assertTrue(
            "catcher must be non-focusable",
            (flags and WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE) != 0,
        )
        assertTrue(
            "catcher must allow outside touches through",
            (flags and WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL) != 0,
        )
        assertFalse(
            "catcher must be touchable",
            (flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE) != 0,
        )
    }

    @Test
    fun isPassThroughRequiresBothFlags() {
        assertTrue(OverlayFlags.isPassThrough(OverlayFlags.passThroughFlags))
        assertFalse(
            OverlayFlags.isPassThrough(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE),
        )
        assertFalse(
            OverlayFlags.isPassThrough(WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE),
        )
        assertFalse(OverlayFlags.isPassThrough(0))
    }
}
