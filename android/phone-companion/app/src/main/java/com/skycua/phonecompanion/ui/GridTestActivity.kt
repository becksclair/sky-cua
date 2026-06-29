package com.skycua.phonecompanion.ui

import android.app.Activity
import android.content.Context
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.os.Bundle
import android.view.View
import android.view.WindowManager
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

/**
 * A neutral grid canvas for exercising the agent overlay (cursor glide, taps,
 * swipes) without driving the operator's real apps. Launch with:
 *
 *   adb shell am start -n com.skycua.phonecompanion/.ui.GridTestActivity
 *
 * By default it follows the device day/night theme — a dark canvas in dark mode,
 * a white grid in light mode — so the overlay can be felt the way it actually
 * renders over the operator's content. Toggling the system theme recreates the
 * activity (no `configChanges` override), so the canvas tracks the change live.
 *
 * It keeps the screen on and fills the whole display, with the system bars hidden,
 * so the grid is a truly edge-to-edge neutral canvas — the screen-edge glow is
 * graded against grid, not against status/navigation-bar chrome. It has no
 * controls and performs no actions.
 */
class GridTestActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView)
            .hide(WindowInsetsCompat.Type.systemBars())
        setContentView(GridView(this, resolveDark()))
    }

    // The activity is already top-most during overlay tests, so a re-launch with a
    // different `dark` extra lands here instead of onCreate. Adopt the new intent
    // and swap the content view directly (recreate() would restore the original
    // launch intent and ignore the new extra), so we can flip the canvas under a
    // live overlay.
    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        setContentView(GridView(this, resolveDark()))
    }

    // Follow the device day/night theme by default. `--ez dark <bool>` still
    // force-overrides it, for capture harnesses that need a fixed canvas
    // regardless of the device's current mode.
    private fun resolveDark(): Boolean =
        if (intent.hasExtra("dark")) {
            intent.getBooleanExtra("dark", true)
        } else {
            val night = resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK
            night == Configuration.UI_MODE_NIGHT_YES
        }
}

/** Draws a white (or dark) background with a faint grid sized in dp. */
private class GridView(context: Context, private val dark: Boolean) : View(context) {
    private val density = resources.displayMetrics.density

    private val bgColor = if (dark) Color.rgb(18, 16, 22) else Color.WHITE
    private val gridPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            color = if (dark) Color.rgb(44, 42, 50) else Color.rgb(222, 222, 222)
            strokeWidth = 1f * density
        }

    private val cellPx = CELL_DP * density

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
    }

    companion object {
        private const val CELL_DP = 48f
    }
}
