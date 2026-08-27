package com.skycua.phonecompanion.ui

import android.app.Activity
import android.view.WindowManager
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.view.Gravity
import android.view.ViewGroup
import com.skycua.phonecompanion.R

/**
 * Shared scaffold for playground activities (S-012).
 *
 * Both `PointerPlaygroundActivity` and `InteractionPlaygroundActivity` need
 * the same edge-to-edge window decor (keepScreenOn, cutout, hidden system
 * bars). Extracting it avoids drift and keeps the two playgrounds in sync.
 */
fun Activity.applyPlaygroundWindowDecor() {
    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    WindowCompat.setDecorFitsSystemWindows(window, false)
    window.attributes = window.attributes.apply {
        layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
    }
    WindowInsetsControllerCompat(window, window.decorView)
        .hide(WindowInsetsCompat.Type.systemBars())
}

/**
 * Helper for `InteractionPlaygroundActivity`-style card playgrounds.
 * Returns (scrollView, container, summary, eventLog) tuple.
 */
fun Activity.createInteractionPlaygroundScaffold(
    titleText: String,
): PlaygroundViews {
    val scroll = ScrollView(this).apply {
        id = R.id.playground_scroll_container
        layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
    }
    val container = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        gravity = Gravity.CENTER_HORIZONTAL
        setPadding(32, 48, 32, 48)
        layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
    }
    scroll.addView(container)

    val title = TextView(this).apply {
        text = titleText
        textSize = 16f
        gravity = Gravity.CENTER
        setPadding(0, 0, 0, 32)
    }
    container.addView(title)

    val summary = TextView(this).apply {
        id = R.id.playground_summary
        textSize = 14f
        gravity = Gravity.CENTER
        setPadding(0, 0, 0, 16)
    }
    container.addView(summary)

    val eventLog = TextView(this).apply {
        id = R.id.playground_event_log
        text = "event log:\n"
        textSize = 12f
        setPadding(0, 0, 0, 16)
    }
    container.addView(eventLog)

    return PlaygroundViews(scroll, container, summary, eventLog)
}

data class PlaygroundViews(
    val scroll: ScrollView,
    val container: LinearLayout,
    val summary: TextView,
    val eventLog: TextView,
)

fun LinearLayout.LayoutParams.Companion.playgroundButtonParams(): LinearLayout.LayoutParams =
    LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
        topMargin = 16
    }
