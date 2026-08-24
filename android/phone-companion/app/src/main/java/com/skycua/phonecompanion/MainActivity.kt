package com.skycua.phonecompanion

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.Bundle
import android.provider.Settings
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
import com.skycua.phonecompanion.direct.DirectLinkServiceOwner
import com.skycua.phonecompanion.direct.directLinkNeedsUserRetry
import com.skycua.phonecompanion.protocol.HealthState
import com.skycua.phonecompanion.service.DeviceMethodHandler
import com.skycua.phonecompanion.ui.PointerPlaygroundActivity

/**
 * Operator home screen. Shows companion identity, the live connection/permission
 * state, and shortcuts to the system settings and the pointer playground, styled
 * with the sky pink palette (see res/values/colors.xml). The UI is built
 * programmatically — there are no XML layouts in this module — using rounded
 * [GradientDrawable] backgrounds and [RippleDrawable] for the cards, chips, and
 * buttons.
 */
class MainActivity : AppCompatActivity() {
    private val handler by lazy { DeviceMethodHandler(applicationContext) }

    /** The status card's body; cleared and rebuilt on each [renderStatus]. */
    private lateinit var statusBody: LinearLayout
    private lateinit var versionLine: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val content =
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                clipToPadding = false
                clipChildren = false
            }

        content.addView(buildHeader())
        content.addView(buildStatusCard(), marginParams(top = 22))
        content.addView(buildActions(), marginParams(top = 18))

        val scroll =
            ScrollView(this).apply {
                isFillViewport = true
                clipToPadding = false
                clipChildren = false
                isVerticalScrollBarEnabled = false
                overScrollMode = View.OVER_SCROLL_NEVER
                addView(content)
            }
        setContentView(scroll)

        // Android 15+ (targetSdk 36) draws edge-to-edge, so inset the content by the
        // system bars: the header clears the status bar and the last button clears
        // the navigation bar.
        ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            v.setPadding(dp(20) + bars.left, dp(26) + bars.top, dp(20) + bars.right, dp(28) + bars.bottom)
            insets
        }
    }

    override fun onResume() {
        super.onResume()
        renderStatus()
    }

    // --- screen sections ------------------------------------------------------

    private fun buildHeader(): View {
        val row =
            LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
            }
        val icon =
            ImageView(this).apply {
                setImageResource(R.mipmap.ic_launcher)
                layoutParams = LinearLayout.LayoutParams(dp(60), dp(60))
            }
        val titles =
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(dp(14), 0, 0, 0)
            }
        titles.addView(
            TextView(this).apply {
                text = getString(R.string.app_name)
                setTextColor(color(R.color.sky_heading))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 23f)
                typeface = Typeface.create("sans-serif", Typeface.BOLD)
                letterSpacing = -0.01f
            },
        )
        titles.addView(
            TextView(this).apply {
                text = getString(R.string.companion_subtitle)
                setTextColor(color(R.color.sky_muted))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                setPadding(0, dp(2), 0, 0)
            },
        )
        row.addView(icon)
        row.addView(titles, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
        return row
    }

    private fun buildStatusCard(): View {
        val card = card()
        versionLine =
            TextView(this).apply {
                setTextColor(color(R.color.sky_muted))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
                typeface = Typeface.MONOSPACE
            }
        statusBody =
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
            }
        card.addView(versionLine)
        card.addView(statusBody, marginParams(top = 4))
        return card
    }

    private fun buildActions(): View {
        val container =
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                clipChildren = false
                clipToPadding = false
                // Narrower than the info card above.
                setPadding(dp(BTN_SIDE_INSET_DP), 0, dp(BTN_SIDE_INSET_DP), 0)
            }
        container.addView(
            filledButton(getString(R.string.open_host_management)) {
                startActivity(Intent(this, HostListActivity::class.java))
            },
        )
        container.addView(
            tonalButton(getString(R.string.open_pointer_playground)) {
                startActivity(Intent(this, PointerPlaygroundActivity::class.java))
            },
            marginParams(top = 10),
        )
        container.addView(
            tonalButton(getString(R.string.open_accessibility_settings)) {
                startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            },
            marginParams(top = 10),
        )
        container.addView(
            tonalButton(getString(R.string.open_notification_settings)) {
                startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
            },
            marginParams(top = 10),
        )
        container.addView(
            tonalButton(getString(R.string.refresh_status)) { renderStatus() },
            marginParams(top = 10),
        )
        return container
    }

    // --- status rendering -----------------------------------------------------

    private fun renderStatus() {
        val health: HealthState = handler.health()
        versionLine.text =
            "v${health.version} (${health.versionCode})  ·  ${health.packageName}"

        statusBody.removeAllViews()

        statusBody.addView(sectionLabel(getString(R.string.section_connection)), marginParams(top = 14))
        val linkAvailability = DirectLinkServiceOwner.availability()
        val directDesired = runCatching {
            val store = AndroidCredentialStore(applicationContext)
            val hosts = store.loadAll()
            hosts.isNotEmpty() || hosts.any { it.pendingEnrollment != null } || store.pendingEnrollment() != null
        }.getOrDefault(false)
        val needsUserRetry = directLinkNeedsUserRetry(linkAvailability, directDesired)
        statusBody.addView(divider())
        statusBody.addView(
            statusRow(
                getString(R.string.status_host_link),
                chip(
                    when (linkAvailability) {
                        DirectLinkServiceOwner.Availability.RUNNING -> getString(R.string.chip_running)
                        DirectLinkServiceOwner.Availability.STARTING -> getString(R.string.chip_connecting)
                        DirectLinkServiceOwner.Availability.START_DENIED -> getString(R.string.chip_start_denied)
                        DirectLinkServiceOwner.Availability.TERMINAL -> getString(R.string.chip_attention)
                        DirectLinkServiceOwner.Availability.STOPPED ->
                            if (directDesired) getString(R.string.chip_retry_needed) else getString(R.string.chip_idle)
                    },
                    when {
                        linkAvailability == DirectLinkServiceOwner.Availability.RUNNING -> ChipKind.OK
                        linkAvailability == DirectLinkServiceOwner.Availability.START_DENIED || needsUserRetry -> ChipKind.OFF
                        linkAvailability == DirectLinkServiceOwner.Availability.TERMINAL -> ChipKind.OFF
                        else -> ChipKind.NEUTRAL
                    },
                ),
            ),
        )
        if (linkAvailability == DirectLinkServiceOwner.Availability.START_DENIED || needsUserRetry) {
            statusBody.addView(
                subRow(
                    getString(if (needsUserRetry) R.string.status_host_link_stopped else R.string.status_host_link_denied),
                    false,
                ),
            )
            statusBody.addView(
                tonalButton(getString(R.string.retry_host_connection)) {
                    val started = DirectLinkServiceOwner.retryUserInitiated(applicationContext)
                    renderStatus()
                    if (started) {
                        statusBody.postDelayed(
                            { if (!isFinishing && !isDestroyed) renderStatus() },
                            750,
                        )
                    }
                },
                marginParams(top = 6),
            )
        }

        statusBody.addView(sectionLabel(getString(R.string.section_permissions)), marginParams(top = 18))
        statusBody.addView(
            statusRow(getString(R.string.status_accessibility), enabledChip(health.accessibilityEnabled)),
        )
        statusBody.addView(subRow(getString(R.string.status_gestures), health.canPerformGestures))
        statusBody.addView(subRow(getString(R.string.status_window_content), health.canRetrieveWindowContent))
        statusBody.addView(subRow(getString(R.string.status_screenshot), health.canTakeScreenshot))
        statusBody.addView(subRow(getString(R.string.status_native_overlay), health.nativeOverlay))
        statusBody.addView(subRow(getString(R.string.status_overlay_passthrough), health.nativeOverlayPassThrough))
        statusBody.addView(divider(), marginParams(top = 8))
        statusBody.addView(
            statusRow(getString(R.string.status_notification_listener), enabledChip(health.notificationListenerEnabled)),
        )
        val smsGranted = checkSelfPermission(Manifest.permission.READ_SMS) == PackageManager.PERMISSION_GRANTED
        statusBody.addView(divider(), marginParams(top = 8))
        statusBody.addView(statusRow(getString(R.string.status_sms_read), enabledChip(smsGranted)))
        if (!smsGranted) {
            statusBody.addView(
                tonalButton(getString(R.string.request_sms_permission)) {
                    requestPermissions(arrayOf(Manifest.permission.READ_SMS), SMS_PERMISSION_REQUEST_CODE)
                },
                marginParams(top = 6),
            )
        }
    }

    // --- component builders ---------------------------------------------------

    private fun card(): LinearLayout =
        LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background = rounded(color(R.color.sky_surface), 16f, color(R.color.sky_card_border), HAIRLINE_PX)
            setPadding(dp(20), dp(18), dp(20), dp(20))
            elevation = dp(1).toFloat()
            outlineProvider = android.view.ViewOutlineProvider.BACKGROUND
            clipToPadding = false
        }

    private fun sectionLabel(text: String): TextView =
        TextView(this).apply {
            this.text = text.uppercase()
            setTextColor(color(R.color.sky_muted))
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 11f)
            letterSpacing = 0.14f
            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
            setPadding(0, 0, 0, dp(2))
        }

    private fun statusRow(label: String, chip: View): LinearLayout {
        val row =
            LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(0, dp(10), 0, dp(10))
            }
        val tv =
            TextView(this).apply {
                text = label
                setTextColor(color(R.color.sky_text))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 15.5f)
            }
        row.addView(tv, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
        row.addView(chip)
        return row
    }

    private fun subRow(label: String, ok: Boolean): LinearLayout {
        val row =
            LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(dp(4), dp(5), 0, dp(5))
            }
        val dot =
            View(this).apply {
                background =
                    GradientDrawable().apply {
                        shape = GradientDrawable.OVAL
                        setColor(color(if (ok) R.color.sky_ok else R.color.sky_off))
                    }
                layoutParams = LinearLayout.LayoutParams(dp(7), dp(7)).apply { marginEnd = dp(10) }
            }
        val tv =
            TextView(this).apply {
                text = label
                setTextColor(color(R.color.sky_muted))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 13.5f)
            }
        val state =
            TextView(this).apply {
                text = getString(if (ok) R.string.state_enabled else R.string.state_disabled)
                gravity = Gravity.END
                setTextColor(color(if (ok) R.color.sky_ok else R.color.sky_off))
                setTextSize(TypedValue.COMPLEX_UNIT_SP, 12.5f)
                typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
            }
        row.addView(dot)
        row.addView(tv, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
        // Inset the sub-capability status deeper than the parent badges, mirroring
        // the left indent so the nested items read as nested on both sides.
        row.addView(
            state,
            LinearLayout.LayoutParams(dp(BADGE_WIDTH_DP), WRAP_CONTENT).apply { marginEnd = dp(SUB_STATUS_INSET_DP) },
        )
        return row
    }

    private fun divider(): View =
        View(this).apply {
            setBackgroundColor(color(R.color.sky_border))
            layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, dp(1))
        }

    private enum class ChipKind { OK, OFF, NEUTRAL }

    private fun enabledChip(on: Boolean): TextView =
        if (on) {
            chip(getString(R.string.state_enabled), ChipKind.OK)
        } else {
            chip(getString(R.string.state_disabled), ChipKind.OFF)
        }

    private fun chip(text: String, kind: ChipKind): TextView {
        @ColorRes val fg: Int
        @ColorRes val bg: Int
        @ColorRes val bd: Int
        when (kind) {
            ChipKind.OK -> {
                fg = R.color.sky_ok
                bg = R.color.sky_ok_bg
                bd = R.color.sky_ok_border
            }
            ChipKind.OFF -> {
                fg = R.color.sky_off
                bg = R.color.sky_off_bg
                bd = R.color.sky_off_border
            }
            ChipKind.NEUTRAL -> {
                fg = R.color.sky_muted
                bg = R.color.sky_secondary
                bd = R.color.sky_neutral_border
            }
        }
        return TextView(this).apply {
            this.text = text
            isSingleLine = true
            gravity = Gravity.CENTER
            setTextColor(color(fg))
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 12.5f)
            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
            background = rounded(color(bg), 999f, color(bd), HAIRLINE_PX)
            setPadding(dp(12), dp(5), dp(12), dp(6))
            // Uniform width so every badge lines up, with a small gutter from the
            // card edge.
            layoutParams =
                LinearLayout.LayoutParams(dp(BADGE_WIDTH_DP), WRAP_CONTENT).apply {
                    marginEnd = dp(BADGE_GUTTER_DP)
                }
        }
    }

    private fun filledButton(label: String, onClick: () -> Unit): TextView =
        button(
            label = label,
            textColor = color(R.color.sky_on_primary),
            fill = rounded(color(R.color.sky_primary), 13f),
            ripple = color(R.color.sky_ripple_on_primary),
            onClick = onClick,
        )

    private fun tonalButton(label: String, onClick: () -> Unit): TextView =
        button(
            label = label,
            textColor = color(R.color.sky_on_secondary),
            fill = rounded(color(R.color.sky_secondary), 13f),
            ripple = color(R.color.sky_ripple_on_secondary),
            onClick = onClick,
        )

    private fun button(
        label: String,
        textColor: Int,
        fill: Drawable,
        ripple: Int,
        onClick: () -> Unit,
    ): TextView =
        TextView(this).apply {
            text = label
            gravity = Gravity.CENTER
            isAllCaps = false
            setTextColor(textColor)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 15.5f)
            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
            background = RippleDrawable(ColorStateList.valueOf(ripple), fill, null)
            setPadding(dp(16), dp(BTN_VPAD_DP), dp(16), dp(BTN_VPAD_DP))
            isClickable = true
            isFocusable = true
            setOnClickListener { onClick() }
            layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT)
        }

    // --- helpers --------------------------------------------------------------

    private fun rounded(fill: Int, radiusDp: Float, stroke: Int? = null, strokeW: Int = 0): GradientDrawable =
        GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = radiusDp * resources.displayMetrics.density
            setColor(fill)
            if (stroke != null) setStroke(strokeW, stroke)
        }

    private fun marginParams(top: Int = 0): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(top) }

    private fun color(@ColorRes id: Int): Int = ContextCompat.getColor(this, id)

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    private companion object {
        const val SMS_PERMISSION_REQUEST_CODE = 47685
        /** Fixed width for every status badge so the right column lines up. */
        const val BADGE_WIDTH_DP = 92

        /** Gutter between a parent-row badge and the card's right edge. */
        const val BADGE_GUTTER_DP = 4

        /** Deeper right inset for nested sub-capability statuses. */
        const val SUB_STATUS_INSET_DP = 24

        /** Sharp hairline stroke (px) for the card and badge borders. */
        const val HAIRLINE_PX = 2

        /** Button vertical padding (dp) — keeps the buttons compact. */
        const val BTN_VPAD_DP = 13

        /** Horizontal inset (dp) making the buttons narrower than the info card. */
        const val BTN_SIDE_INSET_DP = 12
    }
}
