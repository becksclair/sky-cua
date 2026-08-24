package com.skycua.phonecompanion

import android.app.AlertDialog
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.Bundle
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.annotation.ColorRes
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.skycua.phonecompanion.direct.AndroidCredentialStore
import com.skycua.phonecompanion.direct.HostLinkSnapshotRegistry
import com.skycua.phonecompanion.direct.HostRecord

/** Management UI: lists paired hosts with per-host remove, plus entry to pairing. */
class HostListActivity : AppCompatActivity() {
    private lateinit var listContainer: LinearLayout
    private lateinit var countLabel: TextView
    private val registryListener = { runOnUiThread { if (!isFinishing && !isDestroyed) render() } }

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            clipToPadding = false
            clipChildren = false
        }
        content.addView(buildHeader())
        content.addView(buildIntro(), marginParams(top = 18))
        content.addView(buildCountRow(), marginParams(top = 14))
        listContainer = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        content.addView(listContainer, marginParams(top = 8))
        content.addView(buildActions(), marginParams(top = 18))
        val scroll = ScrollView(this).apply { isFillViewport = true; clipToPadding = false; clipChildren = false; isVerticalScrollBarEnabled = false; overScrollMode = View.OVER_SCROLL_NEVER; addView(content) }
        setContentView(scroll)
        ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            v.setPadding(dp(20) + bars.left, dp(22) + bars.top, dp(20) + bars.right, dp(28) + bars.bottom)
            insets
        }
    }

    override fun onResume() {
        super.onResume()
        HostLinkSnapshotRegistry.addListener(registryListener)
        render()
    }

    override fun onPause() {
        HostLinkSnapshotRegistry.removeListener(registryListener)
        super.onPause()
    }

    private fun buildHeader(): View {
        val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL }
        row.addView(ImageView(this).apply { setImageResource(R.mipmap.ic_launcher); layoutParams = LinearLayout.LayoutParams(dp(54), dp(54)) })
        val titles = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(14), 0, 0, 0) }
        titles.addView(TextView(this).apply { text = getString(R.string.host_list_title); setTextColor(color(R.color.sky_heading)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 22f); typeface = Typeface.create("sans-serif", Typeface.BOLD); letterSpacing = -0.01f })
        titles.addView(TextView(this).apply { text = getString(R.string.host_list_subtitle); setTextColor(color(R.color.enrollment_muted)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f); setPadding(0, dp(2), 0, 0) })
        row.addView(titles, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
        return row
    }

    private fun buildIntro(): LinearLayout = card().apply {
        addView(TextView(this@HostListActivity).apply { text = getString(R.string.host_list_intro); setTextColor(color(R.color.enrollment_muted)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f); setLineSpacing(0f, 1.15f) })
    }

    private fun buildCountRow(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL; setPadding(dp(4), 0, dp(4), 0)
        countLabel = TextView(this@HostListActivity).apply { setTextColor(color(R.color.sky_text)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f); typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL) }
        addView(countLabel, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
    }

    private fun buildActions(): View {
        val c = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(12), 0, dp(12), 0) }
        c.addView(filledButton(getString(R.string.host_add_new)) { startActivity(Intent(this, EnrollmentActivity::class.java)) })
        return c
    }

    private fun render() {
        val store = AndroidCredentialStore(applicationContext)
        val hosts = store.loadAll()
        val snapshots = HostLinkSnapshotRegistry.get().associateBy { it.host.deviceId }
        countLabel.text = if (hosts.isEmpty()) getString(R.string.host_list_empty) else resources.getQuantityString(R.plurals.host_count, hosts.size, hosts.size)
        listContainer.removeAllViews()
        if (hosts.isEmpty()) {
            listContainer.addView(card().apply {
                addView(TextView(this@HostListActivity).apply { text = getString(R.string.host_list_empty); setTextColor(color(R.color.enrollment_muted)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f); gravity = Gravity.CENTER; setPadding(dp(8), dp(12), dp(8), dp(12)) })
            })
            return
        }
        hosts.forEach { rec ->
            val stateName = snapshots[rec.deviceId]?.link?.state?.name
            listContainer.addView(buildHostRow(rec, stateName), marginParams(top = 10))
        }
    }

    private fun buildHostRow(rec: HostRecord, stateName: String?): View {
        val card = card()
        val top = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL }
        val idShort = rec.deviceId.take(8) + "…" + rec.deviceId.takeLast(4)
        val chipKind = when (stateName) {
            "CONNECTED" -> ChipKind.OK
            "CONNECTING", "AUTHENTICATING" -> ChipKind.NEUTRAL
            else -> ChipKind.OFF
        }
        val chipText = when (stateName) {
            "CONNECTED" -> getString(R.string.host_chip_connected)
            "CONNECTING", "AUTHENTICATING" -> getString(R.string.host_chip_connecting)
            "BACKOFF", "DISCONNECTED" -> getString(R.string.host_chip_paused)
            "REENROLL_REQUIRED", "DISABLED" -> getString(R.string.host_chip_attention)
            else -> getString(R.string.chip_present)
        }
        top.addView(sectionLabel(rec.endpoint.takeIf { it.isNotBlank() } ?: getString(R.string.host_status_no_host)), LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
        top.addView(chip(chipText, chipKind))
        card.addView(top)
        card.addView(TextView(this).apply { text = "id $idShort"; setTextColor(color(R.color.enrollment_muted)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f); typeface = Typeface.MONOSPACE; setPadding(0, dp(6), 0, 0) })
        card.addView(TextView(this).apply { text = rec.endpoint; setTextColor(color(R.color.sky_muted)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f); typeface = Typeface.MONOSPACE; setTextIsSelectable(true); setPadding(0, dp(4), 0, 0); maxLines = 3 })
        val remove = destructiveButton(getString(R.string.host_remove)) { confirmRemove(rec) }
        card.addView(remove, marginParams(top = 12))
        return card
    }

    private fun confirmRemove(rec: HostRecord) {
        AlertDialog.Builder(this)
            .setTitle(getString(R.string.host_remove_confirm_title))
            .setMessage(getString(R.string.host_remove_confirm_body, rec.endpoint.ifBlank { rec.deviceId.take(8) }))
            .setPositiveButton(getString(R.string.host_remove_confirm_ok)) { _, _ ->
                AndroidCredentialStore(applicationContext).deleteHost(rec.deviceId)
                render()
            }
            .setNegativeButton(getString(R.string.host_remove_confirm_cancel), null)
            .show()
    }

    // --- ui helpers (mirrored from MainActivity/EnrollmentActivity) ---
    private fun card(): LinearLayout = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; background = rounded(color(R.color.sky_surface), 16f, color(R.color.sky_card_border), 2); setPadding(dp(20), dp(16), dp(20), dp(16)); elevation = dp(1).toFloat(); outlineProvider = android.view.ViewOutlineProvider.BACKGROUND; clipToPadding = false }

    private fun sectionLabel(text: String): TextView = TextView(this).apply { this.text = text; setTextColor(color(R.color.sky_text)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f); typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL); isSingleLine = false; maxLines = 2 }

    private enum class ChipKind { OK, OFF, NEUTRAL }
    private fun chip(text: String, kind: ChipKind): TextView {
        @ColorRes val fg: Int
        @ColorRes val bg: Int
        @ColorRes val bd: Int
        when (kind) {
            ChipKind.OK -> { fg = R.color.sky_ok; bg = R.color.sky_ok_bg; bd = R.color.sky_ok_border }
            ChipKind.OFF -> { fg = R.color.sky_off; bg = R.color.sky_off_bg; bd = R.color.sky_off_border }
            ChipKind.NEUTRAL -> { fg = R.color.sky_muted; bg = R.color.sky_secondary; bd = R.color.sky_neutral_border }
        }
        return TextView(this).apply { this.text = text; isSingleLine = true; gravity = Gravity.CENTER; setTextColor(color(fg)); setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f); typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL); background = rounded(color(bg), 999f, color(bd), 2); setPadding(dp(11), dp(5), dp(11), dp(6)) }
    }

    private fun filledButton(label: String, onClick: () -> Unit): TextView = button(label, color(R.color.sky_on_primary), rounded(color(R.color.sky_primary), 13f), color(R.color.sky_ripple_on_primary), onClick)
    private fun tonalButton(label: String, onClick: () -> Unit): TextView = button(label, color(R.color.sky_on_secondary), rounded(color(R.color.sky_secondary), 13f), color(R.color.sky_ripple_on_secondary), onClick)
    private fun destructiveButton(label: String, onClick: () -> Unit): TextView = button(label, color(R.color.sky_off), rounded(color(R.color.sky_off_bg), 13f, color(R.color.sky_off_border), 2), color(R.color.sky_ripple_on_secondary), onClick)
    private fun button(label: String, textColor: Int, fill: Drawable, ripple: Int, onClick: () -> Unit): TextView = TextView(this).apply { text = label; gravity = Gravity.CENTER; isAllCaps = false; setTextColor(textColor); setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f); typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL); background = RippleDrawable(ColorStateList.valueOf(ripple), fill, null); setPadding(dp(16), dp(13), dp(16), dp(13)); isClickable = true; isFocusable = true; minHeight = dp(48); setOnClickListener { onClick() }; layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT) }
    private fun rounded(fill: Int, radiusDp: Float, stroke: Int? = null, strokeW: Int = 0): GradientDrawable = GradientDrawable().apply { shape = GradientDrawable.RECTANGLE; cornerRadius = radiusDp * resources.displayMetrics.density; setColor(fill); if (stroke != null) setStroke(strokeW, stroke) }
    private fun marginParams(top: Int = 0): LinearLayout.LayoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(top) }
    private fun color(@ColorRes id: Int): Int = ContextCompat.getColor(this, id)
    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()
}
