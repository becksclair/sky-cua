package com.skycua.phonecompanion.ui

import android.app.Activity
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.skycua.phonecompanion.R

/**
 * Lane 0 interaction playground: deterministic widgets for semantic action
 * smoke verification. Each widget has a stable `viewIdResourceName`
 * (`com.skycua.phonecompanion:id/playground_*`) so the `node_action` lane can
 * target it by viewId without flaky bounds.
 *
 * Card 1 (Lane 0): click / long_click / context_click / press_and_hold.
 * Future lanes will add scroll / expand / range / text / dismiss cards.
 *
 * Exported so `adb shell am start -n .../.ui.InteractionPlaygroundActivity`
 * and the host smoke can launch it. It shows only local counters and performs
 * no privileged action on launch, so export carries no data risk.
 */
class InteractionPlaygroundActivity : Activity() {

    private var clickCount = 0
    private var longClickCount = 0
    private var contextClickCount = 0

    private lateinit var summary: TextView
    private lateinit var eventLog: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        applyPlaygroundWindowDecor()
        val views = createInteractionPlaygroundScaffold("Interaction Playground — Lane 0\nClick / Long Click / Context Click")
        val scroll = views.scroll
        val container = views.container
        summary = views.summary.apply { text = summaryText() }
        eventLog = views.eventLog

        val clickButton = Button(this).apply {
            id = R.id.playground_click_button
            text = "Click me (clickCount=$clickCount)"
            contentDescription = "playground_click_button"
            isClickable = true
            isLongClickable = false
            setOnClickListener {
                clickCount++
                text = "Click me (clickCount=$clickCount)"
                appendLog("CLICK clickCount=$clickCount")
                updateSummary()
            }
        }
        container.addView(clickButton, linearParams())

        val longClickButton = Button(this).apply {
            id = R.id.playground_long_click_button
            text = "Long press me (longClickCount=$longClickCount)"
            contentDescription = "playground_long_click_button"
            isClickable = true
            isLongClickable = true
            isContextClickable = true
            setOnClickListener {
                // Single tap still counts as click for sanity; long press is separate.
                appendLog("TAP on longClickButton")
                updateSummary()
            }
            setOnLongClickListener {
                longClickCount++
                text = "Long press me (longClickCount=$longClickCount)"
                appendLog("LONG_CLICK longClickCount=$longClickCount")
                updateSummary()
                true
            }
            setOnContextClickListener {
                contextClickCount++
                appendLog("CONTEXT_CLICK contextClickCount=$contextClickCount")
                updateSummary()
                true
            }
        }
        container.addView(longClickButton, linearParams())

        val reset = Button(this).apply {
            id = R.id.playground_reset
            text = "Reset counters"
            setOnClickListener {
                clickCount = 0
                longClickCount = 0
                contextClickCount = 0
                clickButton.text = "Click me (clickCount=$clickCount)"
                longClickButton.text = "Long press me (longClickCount=$longClickCount)"
                eventLog.text = "event log:\n"
                updateSummary()
                appendLog("RESET")
            }
        }
        container.addView(reset, linearParams())

        val hint = TextView(this).apply {
            text = "Targets:\n" +
                    "  com.skycua.phonecompanion:id/playground_click_button -> CLICK\n" +
                    "  com.skycua.phonecompanion:id/playground_long_click_button -> LONG_CLICK\n" +
                    "  (same) -> CONTEXT_CLICK\n" +
                    "Use node_action with view_id or long_press gesture."
            textSize = 11f
            setPadding(0, 24, 0, 0)
        }
        container.addView(hint)

        setContentView(scroll)
        updateSummary()
    }

    private fun linearParams(): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
            topMargin = 16
        }

    private fun summaryText(): String =
        "PASS ${if (clickCount > 0) 1 else 0}/1 click  |  ${if (longClickCount > 0) 1 else 0}/1 long_click  |  ${if (contextClickCount > 0) 1 else 0}/1 context_click"

    private fun updateSummary() {
        summary.text = summaryText()
    }

    private fun appendLog(line: String) {
        eventLog.append(line + "\n")
    }
}
