package com.skycua.phonecompanion.ui

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.os.Bundle
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.core.view.doOnLayout
import com.skycua.phonecompanion.overlay.AgentOverlayController
import com.skycua.phonecompanion.overlay.OverlayMath
import com.skycua.phonecompanion.overlay.ViewOverlayHost
import kotlin.math.hypot

/**
 * A self-contained playground for feeling the agent overlay's pointer motion and
 * feedback without the host, the daemon, or ADB. It hosts the same
 * [AgentOverlayController] the AccessibilityService uses — so the glide,
 * rotation, press/bounce, glow ripple, swipe trail, and "no-no" head-shake are
 * identical to production — but attaches the overlay into this Activity
 * ([ViewOverlayHost]) and drives it from local touch:
 *
 *  - tap an empty spot: the pointer sails there and plays the tap feedback,
 *  - double-tap: the pointer does the "no-no" head-shake in place,
 *  - swipe/drag: the pointer sails to the start and slides along, trail following.
 *
 * It dispatches no real input and touches no other app, so it carries no
 * privilege or data risk. Launch from the companion's main screen, or with:
 *
 *   adb shell am start -n com.skycua.phonecompanion/.ui.PointerPlaygroundActivity
 */
class PointerPlaygroundActivity : Activity() {
    private lateinit var controller: AgentOverlayController

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        // Edge-to-edge with the system bars hidden and layout into the cutout, so
        // the screen-edge glow reaches the true display edges and touch
        // coordinates line up 1:1 with the overlay's device-pixel space.
        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.attributes =
            window.attributes.apply {
                layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
            }
        WindowInsetsControllerCompat(window, window.decorView)
            .hide(WindowInsetsCompat.Type.systemBars())

        val container = PlaygroundContainer(this)
        controller = AgentOverlayController(context = this, host = ViewOverlayHost(container))
        container.onTap = { x, y ->
            controller.animateGesture("tap", listOf(OverlayMath.Point(x, y)), 0L)
        }
        container.onDoubleTap = { controller.pokePointer() }
        container.onReplay = { controller.replayEntrance() }
        container.onSwipe = { sx, sy, ex, ey, durationMs ->
            controller.animateGesture(
                "swipe",
                listOf(OverlayMath.Point(sx, sy), OverlayMath.Point(ex, ey)),
                durationMs,
            )
        }
        setContentView(container)
        // Park the pointer and start the breathing glow once the container has a
        // size (the host reads its bounds to size the overlay and centre the
        // cursor). The activation is posted rather than run inside the layout
        // callback so the overlay child it adds gets a normal measure/layout pass
        // — adding a view mid-layout drops its `requestLayout` and leaves it 0×0.
        container.doOnLayout {
            container.post {
                // The post can outlive a fast finish; don't revive the overlay on a
                // dead Activity.
                if (!isDestroyed && !isFinishing) controller.setActive(true)
            }
        }
    }

    override fun onDestroy() {
        controller.destroy()
        super.onDestroy()
    }
}

/**
 * The playground canvas: a themed grid backdrop that hosts the overlay child and
 * intercepts all touch, classifying each gesture into a tap, double-tap, or
 * swipe and forwarding it through the [onTap]/[onDoubleTap]/[onSwipe] callbacks.
 *
 * The backdrop follows the device day/night theme — a dark canvas in dark mode, a
 * white grid in light mode — so the overlay's pink smoke and glow read the way
 * they actually render over the operator's content. The Activity has no
 * `configChanges` override, so toggling the system theme recreates it and the
 * canvas tracks the change.
 */
private class PlaygroundContainer(context: Context) : FrameLayout(context) {
    var onTap: (Float, Float) -> Unit = { _, _ -> }
    var onDoubleTap: () -> Unit = {}
    var onSwipe: (Float, Float, Float, Float, Long) -> Unit = { _, _, _, _, _ -> }
    var onReplay: () -> Unit = {}

    private val density = resources.displayMetrics.density
    private val touchSlop = android.view.ViewConfiguration.get(context).scaledTouchSlop

    private val dark =
        (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES
    private val bgColor = if (dark) Color.rgb(18, 16, 22) else Color.WHITE

    private val gridPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            color = if (dark) Color.rgb(44, 42, 50) else Color.rgb(222, 222, 222)
            strokeWidth = 1f * density
        }
    private val hintPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = if (dark) Color.rgb(120, 116, 128) else Color.rgb(170, 170, 170)
            textAlign = Paint.Align.CENTER
            textSize = HINT_SP * density
        }
    private val cellPx = CELL_DP * density

    // Single-tap-vs-double-tap-vs-swipe classification.
    private val detector =
        GestureDetector(
            context,
            object : GestureDetector.SimpleOnGestureListener() {
                override fun onSingleTapConfirmed(e: MotionEvent): Boolean {
                    onTap(e.x, e.y)
                    return true
                }

                override fun onDoubleTap(e: MotionEvent): Boolean {
                    doubleTapConsumed = true
                    onDoubleTap()
                    return true
                }

                override fun onLongPress(e: MotionEvent) {
                    longPressConsumed = true
                    onReplay()
                }
            },
        )

    private val swipeMinPx = touchSlop * 2
    private var downX = 0f
    private var downY = 0f
    private var downAt = 0L

    // True once the detector has claimed the current stream as (the second tap of)
    // a double-tap, so the swipe classifier doesn't also fire on the same UP.
    private var doubleTapConsumed = false

    // True once the stream fired a long-press (replay entrance), so a hold-then-drag
    // doesn't also classify as a swipe on the same UP.
    private var longPressConsumed = false

    init {
        // Draw the grid backdrop behind the overlay child.
        setWillNotDraw(false)
    }

    // Intercept every touch so the (visual-only) overlay child never sees input;
    // the container alone classifies gestures.
    override fun onInterceptTouchEvent(ev: MotionEvent): Boolean = true

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        // Reset the double-tap guard before the detector runs, so its onDoubleTap
        // (fired during this DOWN) is not clobbered by our own DOWN handling.
        if (event.actionMasked == MotionEvent.ACTION_DOWN) {
            downX = event.x
            downY = event.y
            downAt = event.eventTime
            doubleTapConsumed = false
            longPressConsumed = false
        }
        detector.onTouchEvent(event)
        if (event.actionMasked == MotionEvent.ACTION_UP) {
            // Classify a swipe by net travel from the touch-down: a tap (even one
            // that jitters past the slop and back) stays below the threshold, and a
            // double-tap or long-press is suppressed so it never doubles as a drag.
            val dragged = hypot(event.x - downX, event.y - downY) > swipeMinPx
            if (dragged && !doubleTapConsumed && !longPressConsumed) {
                onSwipe(downX, downY, event.x, event.y, event.eventTime - downAt)
            }
        }
        return true
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawColor(bgColor)
        var x = 0f
        while (x <= width) {
            canvas.drawLine(x, 0f, x, height.toFloat(), gridPaint)
            x += cellPx
        }
        var y = 0f
        while (y <= height) {
            canvas.drawLine(0f, y, width.toFloat(), y, gridPaint)
            y += cellPx
        }
        canvas.drawText(HINT, width / 2f, HINT_TOP_DP * density, hintPaint)
    }

    companion object {
        private const val CELL_DP = 48f
        private const val HINT_SP = 13f
        private const val HINT_TOP_DP = 72f
        private const val HINT =
            "Tap: move   •   Double-tap: no-no   •   Swipe: drag   •   Long-press: replay entrance"
    }
}
