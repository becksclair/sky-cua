package com.skycua.phonecompanion.ui

import android.app.Activity
import android.os.Bundle
import android.os.Build
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.TextView
import androidx.core.view.ViewCompat
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import com.skycua.phonecompanion.R

/**
 * Lane 0-6 interaction playground: deterministic widgets for semantic action
 * smoke verification. Each widget has a stable `viewIdResourceName`
 * (`com.skycua.phonecompanion:id/playground_*`) so the `node_action` lane can
 * target it by viewId without flaky bounds.
 *
 * 7 cards: Click/Long/Context/PressAndHold, Scroll family, Expand/Collapse,
 * Range (SeekBar), TextEdit, Dismiss/Select, Global/Key.
 *
 * Exported so `adb shell am start -n .../.ui.InteractionPlaygroundActivity`
 * and the host smoke can launch it. It shows only local counters and performs
 * no privileged action on launch, so export carries no data risk.
 */
class InteractionPlaygroundActivity : Activity() {

    private var clickCount = 0
    private var longClickCount = 0
    private var contextClickCount = 0
    private var scrollCount = 0
    private var expandCount = 0
    private var collapseCount = 0
    private var progressCount = 0
    private var setTextCount = 0
    private var copyCount = 0
    private var cutCount = 0
    private var pasteCount = 0
    private var dismissCount = 0
    private var selectCount = 0
    private var globalCount = 0

    private var isExpanded = false

    private lateinit var summary: TextView
    private lateinit var eventLog: TextView
    private lateinit var seekBar: SeekBar
    private lateinit var editText: EditText
    private lateinit var expandView: TextView
    private lateinit var scrollView: ScrollView
    private lateinit var dismissView: TextView
    private lateinit var selectView: TextView
    private lateinit var globalStatus: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        applyPlaygroundWindowDecor()
        val views = createInteractionPlaygroundScaffold(
            "Interaction Playground — Lanes 0-6\nClick / Scroll / Expand / Range / Text / Dismiss / Global"
        )
        val scroll = views.scroll
        val container = views.container
        summary = views.summary.apply { text = summaryText() }
        eventLog = views.eventLog

        // ---- Card 1: Click / Long / Context / PressAndHold -----------------
        container.addView(sectionHeader("1. Click — CLICK / LONG_CLICK / CONTEXT_CLICK / PRESS_AND_HOLD"))
        val clickButton = Button(this).apply {
            id = R.id.playground_click_button
            text = "Click me (clickCount=$clickCount)"
            contentDescription = "playground_click_button"
            isClickable = true
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
        // PressAndHold is mapped to same button via accessibility action id 30
        ViewCompat.addAccessibilityAction(
            longClickButton,
            "Press and hold"
        ) { _, _ ->
            longClickCount++
            longClickButton.text = "Long press me (longClickCount=$longClickCount)"
            appendLog("PRESS_AND_HOLD longClickCount=$longClickCount")
            updateSummary()
            true
        }
        container.addView(longClickButton, linearParams())

        // ---- Card 2: Scroll family -----------------------------------------
        container.addView(sectionHeader("2. Scroll — SCROLL_FORWARD/BACKWARD/UP/DOWN/LEFT/RIGHT/PAGE_* / SCROLL_TO_POSITION"))
        scrollView = ScrollView(this).apply {
            id = R.id.playground_scroll_view
            contentDescription = "playground_scroll_view"
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 400)
        }
        val scrollContent = LinearLayout(this).apply {
            id = R.id.playground_scroll_content
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setPadding(16, 16, 16, 16)
        }
        for (i in 1..20) {
            val tv = TextView(this).apply {
                text = "Scroll item $i — drag or SCROLL_FORWARD to reveal"
                textSize = 13f
                setPadding(8, 12, 8, 12)
            }
            scrollContent.addView(tv)
        }
        scrollView.addView(scrollContent)
        // Count scroll via listener
        scrollView.viewTreeObserver.addOnScrollChangedListener {
            // Only count programmatic accessibility scroll, not every pixel
        }
        // Accessibility delegates for scroll actions
        scrollView.accessibilityDelegate = object : View.AccessibilityDelegate() {
            override fun performAccessibilityAction(host: View, action: Int, args: Bundle?): Boolean {
                val handled = when (action) {
                    AccessibilityNodeInfo.ACTION_SCROLL_FORWARD,
                    AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD -> {
                        scrollCount++
                        val targetY = if (action == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD) 800 else 0
                        scrollView.smoothScrollTo(0, targetY)
                        appendLog("SCROLL_${if (action==AccessibilityNodeInfo.ACTION_SCROLL_FORWARD) "FORWARD" else "BACKWARD"} scrollCount=$scrollCount")
                        updateSummary()
                        true
                    }
                    else -> {
                        if (Build.VERSION.SDK_INT >= 23 && action == AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_TO_POSITION.id) {
                            val row = args?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_ROW_INT, 0) ?: 0
                            scrollCount++
                            scrollView.smoothScrollTo(0, row * 80)
                            appendLog("SCROLL_TO_POSITION row=$row scrollCount=$scrollCount")
                            updateSummary()
                            return true
                        }
                        // Modern actions (SCROLL_UP etc) use ids from AccessibilityAction
                        // They arrive as the same int ids; handle generically
                        val name = modernScrollName(action)
                        if (name != null) {
                            scrollCount++
                            appendLog("$name scrollCount=$scrollCount")
                            updateSummary()
                            true
                        } else false
                    }
                }
                return handled || super.performAccessibilityAction(host, action, args)
            }
        }
        // scroll_to_position handled inside delegate below
        container.addView(scrollView, linearParams())
        container.addView(TextView(this).apply {
            text = "scrollCount=$scrollCount — try node_action SCROLL_FORWARD on playground_scroll_view"
            textSize = 11f
            tag = "scroll_hint"
        })

        // ---- Card 3: Expand / Collapse -------------------------------------
        container.addView(sectionHeader("3. Expand — EXPAND / COLLAPSE"))
        expandView = TextView(this).apply {
            id = R.id.playground_expand_view
            text = "Collapsed — tap EXPAND"
            contentDescription = "playground_expand_view"
            textSize = 14f
            gravity = Gravity.CENTER
            setPadding(32, 32, 32, 32)
            setBackgroundColor(0xFFE0E0FF.toInt())
            isClickable = true
            setOnClickListener { toggleExpand() }
        }
        expandView.accessibilityDelegate = object : View.AccessibilityDelegate() {
            override fun performAccessibilityAction(host: View, action: Int, args: Bundle?): Boolean {
                return when (action) {
                    AccessibilityNodeInfo.ACTION_EXPAND -> {
                        if (!isExpanded) toggleExpand()
                        expandCount++
                        appendLog("EXPAND expandCount=$expandCount isExpanded=$isExpanded")
                        updateSummary()
                        true
                    }
                    AccessibilityNodeInfo.ACTION_COLLAPSE -> {
                        if (isExpanded) toggleExpand()
                        collapseCount++
                        appendLog("COLLAPSE collapseCount=$collapseCount isExpanded=$isExpanded")
                        updateSummary()
                        true
                    }
                    else -> super.performAccessibilityAction(host, action, args)
                }
            }
        }
        container.addView(expandView, linearParams())

        // ---- Card 4: Range — SeekBar SET_PROGRESS --------------------------
        container.addView(sectionHeader("4. Range — SET_PROGRESS"))
        seekBar = SeekBar(this).apply {
            id = R.id.playground_seekbar
            contentDescription = "playground_seekbar"
            max = 100
            progress = 42
        }
        seekBar.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(s: SeekBar?, p: Int, fromUser: Boolean) {
                // Count only accessibility-driven changes (fromUser false)
                if (!fromUser) {
                    // already counted in delegate; just log
                }
            }
            override fun onStartTrackingTouch(s: SeekBar?) {}
            override fun onStopTrackingTouch(s: SeekBar?) {}
        })
        seekBar.accessibilityDelegate = object : View.AccessibilityDelegate() {
            override fun performAccessibilityAction(host: View, action: Int, args: Bundle?): Boolean {
                val isSetProgress = if (Build.VERSION.SDK_INT >= 24) {
                    action == AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_PROGRESS.id
                } else false
                if (isSetProgress) {
                    val value = args?.getFloat(AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE, seekBar.progress.toFloat()) ?: 0f
                    seekBar.progress = value.toInt().coerceIn(0, 100)
                    progressCount++
                    appendLog("SET_PROGRESS value=$value progressCount=$progressCount")
                    updateSummary()
                    return true
                }
                return super.performAccessibilityAction(host, action, args)
            }
        }
        container.addView(seekBar, linearParams())
        container.addView(TextView(this).apply {
            text = "SeekBar 0-100 — node_action SET_PROGRESS with args.progress"
            textSize = 11f
        })

        // ---- Card 5: TextEdit — SET_TEXT / COPY / CUT / PASTE / SELECT -----
        container.addView(sectionHeader("5. TextEdit — SET_TEXT / SET_SELECTION / COPY / CUT / PASTE / FOCUS"))
        editText = EditText(this).apply {
            id = R.id.playground_edit_text
            contentDescription = "playground_edit_text"
            hint = "Type or SET_TEXT via node_action"
            setText("Editable playground text")
            textSize = 14f
            setPadding(24, 24, 24, 24)
            setBackgroundColor(0xFFFFF8DC.toInt())
        }
        editText.accessibilityDelegate = object : View.AccessibilityDelegate() {
            override fun performAccessibilityAction(host: View, action: Int, args: Bundle?): Boolean {
                val res = when (action) {
                    AccessibilityNodeInfo.ACTION_SET_TEXT -> {
                        val seq = args?.getCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE) ?: ""
                        editText.setText(seq)
                        setTextCount++
                        appendLog("SET_TEXT text=$seq setTextCount=$setTextCount")
                        updateSummary()
                        true
                    }
                    AccessibilityNodeInfo.ACTION_COPY -> {
                        copyCount++
                        appendLog("COPY copyCount=$copyCount")
                        updateSummary()
                        // Let system also handle copy
                        false
                    }
                    AccessibilityNodeInfo.ACTION_CUT -> {
                        cutCount++
                        appendLog("CUT cutCount=$cutCount")
                        updateSummary()
                        false
                    }
                    AccessibilityNodeInfo.ACTION_PASTE -> {
                        pasteCount++
                        appendLog("PASTE pasteCount=$pasteCount")
                        updateSummary()
                        false
                    }
                    AccessibilityNodeInfo.ACTION_FOCUS, AccessibilityNodeInfo.ACTION_CLEAR_FOCUS,
                    AccessibilityNodeInfo.ACTION_SELECT, AccessibilityNodeInfo.ACTION_CLEAR_SELECTION,
                    AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS, AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS -> {
                        appendLog("FOCUS/SELECT action=$action")
                        updateSummary()
                        false
                    }
                    AccessibilityNodeInfo.ACTION_SET_SELECTION -> {
                        val start = args?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 0) ?: 0
                        val end = args?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, 0) ?: 0
                        appendLog("SET_SELECTION $start..$end")
                        updateSummary()
                        false
                    }
                    else -> {
                        // Movement granularity etc
                        if (action == AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY ||
                            action == AccessibilityNodeInfo.ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY) {
                            appendLog("MOVEMENT_GRANULARITY action=$action")
                            updateSummary()
                            false
                        } else null
                    }
                }
                return if (res == true) true else super.performAccessibilityAction(host, action, args)
            }
        }
        container.addView(editText, linearParams())

        // ---- Card 6: Dismiss / Select --------------------------------------
        container.addView(sectionHeader("6. Dismiss / Select — DISMISS / SELECT / SHOW_ON_SCREEN"))
        dismissView = TextView(this).apply {
            id = R.id.playground_dismiss_view
            text = "Dismissable — DISMISS will hide me"
            contentDescription = "playground_dismiss_view"
            textSize = 13f
            gravity = Gravity.CENTER
            setPadding(24, 32, 24, 32)
            setBackgroundColor(0xFFFFE0E0.toInt())
            isClickable = true
        }
        dismissView.accessibilityDelegate = object : View.AccessibilityDelegate() {
            override fun performAccessibilityAction(host: View, action: Int, args: Bundle?): Boolean {
                return when (action) {
                    AccessibilityNodeInfo.ACTION_DISMISS -> {
                        dismissCount++
                        host.visibility = View.GONE
                        appendLog("DISMISS dismissCount=$dismissCount")
                        updateSummary()
                        true
                    }
                    else -> {
                        if (Build.VERSION.SDK_INT >= 23 && action == AccessibilityNodeInfo.AccessibilityAction.ACTION_SHOW_ON_SCREEN.id) {
                            appendLog("SHOW_ON_SCREEN")
                            updateSummary()
                            return true
                        }
                        super.performAccessibilityAction(host, action, args)
                    }
                }
            }
        }
        container.addView(dismissView, linearParams())

        selectView = TextView(this).apply {
            id = R.id.playground_select_view
            text = "Selectable — SELECT / CLEAR_SELECTION"
            contentDescription = "playground_select_view"
            textSize = 13f
            gravity = Gravity.CENTER
            setPadding(24, 32, 24, 32)
            setBackgroundColor(0xFFE0FFE0.toInt())
            isClickable = true
            isSelected = false
            setOnClickListener { isSelected = !isSelected; updateSelectView() }
        }
        selectView.accessibilityDelegate = object : View.AccessibilityDelegate() {
            override fun performAccessibilityAction(host: View, action: Int, args: Bundle?): Boolean {
                return when (action) {
                    AccessibilityNodeInfo.ACTION_SELECT -> {
                        selectCount++
                        selectView.isSelected = true
                        updateSelectView()
                        appendLog("SELECT selectCount=$selectCount")
                        updateSummary()
                        true
                    }
                    AccessibilityNodeInfo.ACTION_CLEAR_SELECTION -> {
                        selectView.isSelected = false
                        updateSelectView()
                        appendLog("CLEAR_SELECTION")
                        updateSummary()
                        true
                    }
                    else -> super.performAccessibilityAction(host, action, args)
                }
            }
        }
        container.addView(selectView, linearParams())
        container.addView(Button(this).apply {
            text = "Restore dismiss view"
            setOnClickListener {
                dismissView.visibility = View.VISIBLE
                appendLog("RESTORE dismissView")
                updateSummary()
            }
        }, linearParams())

        // ---- Card 7: Global / Key ------------------------------------------
        container.addView(sectionHeader("7. Global / Key — GLOBAL_ACTIONS + KEY_EVENT"))
        globalStatus = TextView(this).apply {
            id = R.id.playground_global_status
            text = "Last global: none — use phone_global_action / phone_key_event tools"
            textSize = 12f
            setPadding(16, 16, 16, 16)
            setBackgroundColor(0xFFF0F0F0.toInt())
        }
        container.addView(globalStatus, linearParams())
        container.addView(TextView(this).apply {
            text = "Globals: BACK/HOME/RECENTS/NOTIFICATIONS/QUICK_SETTINGS/POWER_DIALOG/TOGGLE_SPLIT_SCREEN/LOCK_SCREEN/TAKE_SCREENSHOT etc.\n" +
                    "Keys: KEYCODE_VOLUME_UP etc (ADB fallback if companion denies).\n" +
                    "All report PERMISSION_DENIED visibly on OEM denial, not silent success."
            textSize = 11f
        })

        // ---- Reset + hint ---------------------------------------------------
        val reset = Button(this).apply {
            id = R.id.playground_reset
            text = "Reset counters"
            setOnClickListener {
                clickCount = 0; longClickCount = 0; contextClickCount = 0
                scrollCount = 0; expandCount = 0; collapseCount = 0
                progressCount = 0; setTextCount = 0; copyCount = 0; cutCount = 0; pasteCount = 0
                dismissCount = 0; selectCount = 0; globalCount = 0
                isExpanded = false
                expandView.text = "Collapsed — tap EXPAND"
                seekBar.progress = 42
                editText.setText("Editable playground text")
                dismissView.visibility = View.VISIBLE
                selectView.isSelected = false
                updateSelectView()
                globalStatus.text = "Last global: none"
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
                    "  playground_click_button -> CLICK\n" +
                    "  playground_long_click_button -> LONG_CLICK / CONTEXT_CLICK / PRESS_AND_HOLD\n" +
                    "  playground_scroll_view -> SCROLL_* / PAGE_* / SCROLL_TO_POSITION\n" +
                    "  playground_expand_view -> EXPAND / COLLAPSE\n" +
                    "  playground_seekbar -> SET_PROGRESS (progress:Float)\n" +
                    "  playground_edit_text -> SET_TEXT / SET_SELECTION / COPY / CUT / PASTE / NEXT_AT_GRANULARITY\n" +
                    "  playground_dismiss_view -> DISMISS / SHOW_ON_SCREEN\n" +
                    "  playground_select_view -> SELECT / CLEAR_SELECTION\n" +
                    "  Globals -> phone_global_action, Keys -> phone_key_event\n" +
                    "Use node_action with view_id, args Bundle as needed."
            textSize = 10f
            setPadding(0, 24, 0, 0)
        }
        container.addView(hint)

        setContentView(scroll)
        updateSummary()
    }

    private fun sectionHeader(title: String): TextView = TextView(this).apply {
        text = title
        textSize = 12f
        setPadding(0, 24, 0, 8)
        setTextColor(0xFF333333.toInt())
    }

    private fun linearParams(): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
            topMargin = 12
        }

    private fun toggleExpand() {
        isExpanded = !isExpanded
        expandView.text = if (isExpanded) "Expanded — tap COLLAPSE" else "Collapsed — tap EXPAND"
    }

    private fun updateSelectView() {
        selectView.text = if (selectView.isSelected) "Selected ✓ — CLEAR_SELECTION to unselect" else "Selectable — SELECT / CLEAR_SELECTION"
    }

    private fun modernScrollName(action: Int): String? {
        if (Build.VERSION.SDK_INT < 28) return null
        return when (action) {
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_UP.id -> "SCROLL_UP"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_DOWN.id -> "SCROLL_DOWN"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_LEFT.id -> "SCROLL_LEFT"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_RIGHT.id -> "SCROLL_RIGHT"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_UP.id -> "PAGE_UP"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_DOWN.id -> "PAGE_DOWN"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_LEFT.id -> "PAGE_LEFT"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_PAGE_RIGHT.id -> "PAGE_RIGHT"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_CONTEXT_CLICK.id -> "CONTEXT_CLICK"
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SHOW_ON_SCREEN.id -> "SHOW_ON_SCREEN"
            else -> null
        }
    }

    private fun summaryText(): String {
        val total = 14
        var passed = 0
        if (clickCount > 0) passed++
        if (longClickCount > 0) passed++
        if (contextClickCount > 0) passed++
        if (scrollCount > 0) passed++
        if (expandCount > 0) passed++
        if (collapseCount > 0) passed++
        if (progressCount > 0) passed++
        if (setTextCount > 0) passed++
        if (copyCount > 0 || cutCount > 0 || pasteCount > 0) passed++
        if (dismissCount > 0) passed++
        if (selectCount > 0) passed++
        // global/key counted via event log keyword
        val globalLogged = eventLog?.text?.contains("GLOBAL") == true
        if (globalLogged) passed++
        // pad to show lanes
        return "PASS $passed/$total  click:$clickCount long:$longClickCount ctx:$contextClickCount " +
                "scroll:$scrollCount exp:$expandCount/$collapseCount prog:$progressCount " +
                "text:$setTextCount dis:$dismissCount sel:$selectCount"
    }

    private fun updateSummary() {
        summary.text = summaryText()
    }

    private fun appendLog(line: String) {
        eventLog.append(line + "\n")
    }
}
