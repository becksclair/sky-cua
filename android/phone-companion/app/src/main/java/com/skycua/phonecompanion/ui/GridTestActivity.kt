package com.skycua.phonecompanion.ui

import android.app.Activity
import android.content.Context
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
 * A plain white screen with a light-grey grid, used only as a neutral canvas for
 * exercising the agent overlay (cursor glide, taps, swipes) without driving the
 * operator's real apps. Launch with:
 *
 *   adb shell am start -n com.skycua.phonecompanion/.ui.GridTestActivity
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
        setContentView(GridView(this))
    }
}

/** Draws a white background with a light-grey grid sized in dp. */
private class GridView(context: Context) : View(context) {
    private val density = resources.displayMetrics.density

    private val gridPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            color = Color.rgb(222, 222, 222)
            strokeWidth = 1f * density
        }

    private val cellPx = CELL_DP * density

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawColor(Color.WHITE)
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
