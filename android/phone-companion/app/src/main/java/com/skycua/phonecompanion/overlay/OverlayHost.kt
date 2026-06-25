package com.skycua.phonecompanion.overlay

import android.content.Context
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout

/**
 * Where the [AgentOverlayView] and its optional "no-no" tap-catcher live. The
 * animation engine in [AgentOverlayController] is host-agnostic; only the
 * windowing and finger-tap plumbing differ:
 *
 *  - [WindowOverlayHost] (production) attaches a pass-through `WindowManager`
 *    overlay window from the AccessibilityService plus a tiny catcher window that
 *    follows the idle cursor for the "no-no" tap feedback.
 *  - [ViewOverlayHost] (the in-app pointer playground) attaches the overlay into
 *    an Activity's view tree and catches finger taps itself, so it needs no
 *    catcher window.
 *
 * Isolating these two operations keeps the controller's tuned motion/feedback
 * code identical across both surfaces.
 */
interface OverlayHost {
    /** Full drawable display bounds in device px, as `(width, height)`. */
    fun displayBounds(): Pair<Int, Int>

    /**
     * Attaches the full-screen overlay [view] sized to ([width], [height]) px.
     * Precondition: the caller must not attach a view that is already attached
     * (the controller gates this with its single-view invariant).
     */
    fun attachOverlay(view: AgentOverlayView, width: Int, height: Int)

    /**
     * Resizes the already-attached overlay [view] to ([width], [height]) px after a
     * rotation or display change so the edge glow keeps hugging the true screen
     * edges. Safe to call repeatedly; the controller only calls it when the
     * dimensions actually changed.
     */
    fun resizeOverlay(view: AgentOverlayView, width: Int, height: Int)

    /** Detaches the overlay [view] if attached. Safe when already detached. */
    fun detachOverlay(view: AgentOverlayView)

    /**
     * Starts delivering rotation / display-change notifications to [onChange] (on
     * the main thread) so the controller can re-resolve [displayBounds] and resize.
     * Called once when the overlay is attached. Hosts whose surface relayouts
     * itself (the playground's Activity view tree) may no-op. Idempotent.
     */
    fun registerDisplayListener(onChange: () -> Unit)

    /**
     * Stops delivering display-change notifications. Called when the overlay is
     * detached so the registration cannot outlive the overlay. Safe when no
     * listener is registered.
     */
    fun unregisterDisplayListener()

    /**
     * True when the overlay window is non-focusable and non-touchable
     * (pass-through).
     */
    fun isPassThrough(): Boolean

    /**
     * Attaches the touchable catcher [view] (side [size] px) centred at
     * ([centerX], [centerY]) so a finger tap on the pointer can be detected.
     * Returns null when the host does not support catchers. The catcher can still
     * be flipped pass-through through [CatcherHandle.setTouchable] while the
     * overlay itself remains non-touchable.
     */
    fun attachCatcher(view: View, size: Int, centerX: Int, centerY: Int): CatcherHandle?
}

/** Lifecycle handle for the "no-no" tap-catcher, owned by the host. */
interface CatcherHandle {
    /** Flips the catcher between touchable and pass-through. */
    fun setTouchable(on: Boolean)

    /** Moves the catcher's top-left corner to ([x], [y]) px. */
    fun moveTo(x: Int, y: Int)

    /** Detaches the catcher. Safe when already detached. */
    fun detach()
}

/**
 * Hosts the overlay as `WindowManager` overlay windows of [windowType] (the
 * production AccessibilityService path). The overlay window is sized to the full
 * display bounds — including the status/navigation-bar regions and any display
 * cutout — so the edge glow reaches the true screen edges.
 */
class WindowOverlayHost(
    private val context: Context,
    private val windowType: Int,
) : OverlayHost {
    private val windowManager: WindowManager
        get() = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager

    private val displayManager: DisplayManager
        get() = context.getSystemService(Context.DISPLAY_SERVICE) as DisplayManager

    private var overlayParams: WindowManager.LayoutParams? = null
    private var displayListener: DisplayManager.DisplayListener? = null

    override fun displayBounds(): Pair<Int, Int> {
        val bounds = windowManager.currentWindowMetrics.bounds
        return bounds.width() to bounds.height()
    }

    override fun attachOverlay(view: AgentOverlayView, width: Int, height: Int) {
        val lp =
            WindowManager.LayoutParams(
                width,
                height,
                windowType,
                OverlayFlags.passThroughFlags,
                PixelFormat.TRANSLUCENT,
            ).apply {
                gravity = Gravity.TOP or Gravity.START
                x = 0
                y = 0
                layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
            }
        windowManager.addView(view, lp)
        overlayParams = lp
    }

    override fun resizeOverlay(view: AgentOverlayView, width: Int, height: Int) {
        val lp = overlayParams ?: return
        if (lp.width == width && lp.height == height) return
        lp.width = width
        lp.height = height
        windowManager.updateViewLayout(view, lp)
    }

    override fun detachOverlay(view: AgentOverlayView) {
        try {
            windowManager.removeView(view)
        } catch (_: IllegalArgumentException) {
            // Already detached.
        }
        overlayParams = null
    }

    override fun registerDisplayListener(onChange: () -> Unit) {
        if (displayListener != null) return
        val listener =
            object : DisplayManager.DisplayListener {
                override fun onDisplayChanged(displayId: Int) {
                    // The default display's rotation/size changed (or an external
                    // display did): let the controller re-resolve and resize.
                    onChange()
                }

                override fun onDisplayAdded(displayId: Int) {}

                override fun onDisplayRemoved(displayId: Int) {}
            }
        // Deliver callbacks on the main thread, where all view/window mutation runs.
        displayManager.registerDisplayListener(listener, Handler(Looper.getMainLooper()))
        displayListener = listener
    }

    override fun unregisterDisplayListener() {
        val listener = displayListener ?: return
        displayManager.unregisterDisplayListener(listener)
        displayListener = null
    }

    override fun isPassThrough(): Boolean =
        overlayParams?.let { OverlayFlags.isPassThrough(it.flags) } ?: true

    override fun attachCatcher(view: View, size: Int, centerX: Int, centerY: Int): CatcherHandle? {
        val lp =
            WindowManager.LayoutParams(
                size,
                size,
                windowType,
                OverlayFlags.passThroughFlags,
                PixelFormat.TRANSLUCENT,
            ).apply {
                gravity = Gravity.TOP or Gravity.START
                x = centerX - size / 2
                y = centerY - size / 2
                layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
            }
        windowManager.addView(view, lp)
        return WindowCatcherHandle(windowManager, view, lp)
    }

    private class WindowCatcherHandle(
        private val windowManager: WindowManager,
        private val view: View,
        private val lp: WindowManager.LayoutParams,
    ) : CatcherHandle {
        private var attached = true

        override fun setTouchable(on: Boolean) {
            if (!attached) return
            val flags = if (on) OverlayFlags.touchableFlags else OverlayFlags.passThroughFlags
            if (lp.flags == flags) return
            lp.flags = flags
            update()
        }

        override fun moveTo(x: Int, y: Int) {
            if (!attached) return
            if (lp.x == x && lp.y == y) return
            lp.x = x
            lp.y = y
            update()
        }

        override fun detach() {
            if (!attached) return
            attached = false
            try {
                windowManager.removeView(view)
            } catch (_: IllegalArgumentException) {
                // Already detached.
            }
        }

        private fun update() {
            try {
                windowManager.updateViewLayout(view, lp)
            } catch (_: IllegalArgumentException) {
                attached = false
            }
        }
    }
}

/**
 * Hosts the overlay as a child of an Activity [container] (the in-app pointer
 * playground). The container intercepts touch and feeds gesture intents to the
 * controller directly, so there is no pass-through window and no catcher: a
 * finger tap on the pointer is detected by the container, not a catcher window.
 */
class ViewOverlayHost(
    private val container: FrameLayout,
) : OverlayHost {
    override fun displayBounds(): Pair<Int, Int> = container.width to container.height

    override fun attachOverlay(view: AgentOverlayView, width: Int, height: Int) {
        container.addView(
            view,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT,
            ),
        )
    }

    // The overlay child is MATCH_PARENT, so it tracks the container's size through
    // the Activity view tree on rotation; no explicit window resize is needed.
    override fun resizeOverlay(view: AgentOverlayView, width: Int, height: Int) = Unit

    override fun detachOverlay(view: AgentOverlayView) {
        container.removeView(view)
    }

    // The overlay child is non-touchable and the container handles input, so the
    // "non-focusable, non-touchable" pass-through invariant holds trivially.
    override fun isPassThrough(): Boolean = true

    // The playground catches finger taps itself; it drives no catcher window.
    override fun attachCatcher(view: View, size: Int, centerX: Int, centerY: Int): CatcherHandle? = null

    // The Activity view tree relayouts the container on configuration changes, so
    // the playground needs no DisplayManager listener.
    override fun registerDisplayListener(onChange: () -> Unit) = Unit

    override fun unregisterDisplayListener() = Unit
}
