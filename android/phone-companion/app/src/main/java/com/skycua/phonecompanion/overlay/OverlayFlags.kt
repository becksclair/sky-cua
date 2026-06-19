package com.skycua.phonecompanion.overlay

import android.view.WindowManager

/**
 * Computes the window flags for the phone-native cursor overlay. The overlay
 * must be non-focusable and non-touchable so taps pass through it to the
 * underlying app, per the wire contract's `cursor_overlay` semantics.
 *
 * The flag math is isolated here, free of Android view inflation, so it can be
 * unit-tested directly. The constants mirror
 * `WindowManager.LayoutParams.FLAG_*` values and are asserted against the
 * platform constants in instrumentation; the JVM unit test pins the intended
 * bitmask.
 */
object OverlayFlags {
    // Mirrors of WindowManager.LayoutParams flag bits used by the overlay.
    const val FLAG_NOT_FOCUSABLE = WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
    const val FLAG_NOT_TOUCHABLE = WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
    const val FLAG_NOT_TOUCH_MODAL = WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL
    const val FLAG_LAYOUT_IN_SCREEN = WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN
    const val FLAG_LAYOUT_NO_LIMITS = WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS
    const val FLAG_HARDWARE_ACCELERATED = WindowManager.LayoutParams.FLAG_HARDWARE_ACCELERATED

    /**
     * The combined pass-through flag set. NOT_FOCUSABLE keeps the overlay from
     * stealing input focus; NOT_TOUCHABLE makes the window ignore touch events
     * entirely so they reach the app beneath it.
     */
    val passThroughFlags: Int =
        FLAG_NOT_FOCUSABLE or
            FLAG_NOT_TOUCHABLE or
            FLAG_NOT_TOUCH_MODAL or
            FLAG_LAYOUT_IN_SCREEN or
            FLAG_LAYOUT_NO_LIMITS or
            FLAG_HARDWARE_ACCELERATED

    /**
     * Flags for the small "no-no" tap-catcher window that follows the cursor: it
     * IS touchable (so a finger tap on the pointer can be detected) but never takes
     * focus and never bounds touches to itself (NOT_TOUCH_MODAL), so it only
     * catches taps inside its own tiny region and everything else still reaches the
     * app beneath. NOT_TOUCHABLE is intentionally absent.
     */
    val touchableFlags: Int =
        FLAG_NOT_FOCUSABLE or
            FLAG_NOT_TOUCH_MODAL or
            FLAG_LAYOUT_IN_SCREEN or
            FLAG_LAYOUT_NO_LIMITS or
            FLAG_HARDWARE_ACCELERATED

    /** True when the given flag set is both non-focusable and non-touchable. */
    fun isPassThrough(flags: Int): Boolean =
        (flags and FLAG_NOT_FOCUSABLE) != 0 && (flags and FLAG_NOT_TOUCHABLE) != 0
}
