package com.skycua.phonecompanion

import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.Bundle
import android.text.InputType
import android.text.method.PasswordTransformationMethod
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.view.WindowManager
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.TextView
import androidx.annotation.ColorRes
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.lifecycle.ViewModelProvider

/** Handles skycua://enroll links and manual paste without exposing bootstrap material. */
class EnrollmentActivity : AppCompatActivity() {
    private lateinit var entryCard: LinearLayout
    private lateinit var input: EditText
    private lateinit var endpointCard: LinearLayout
    private lateinit var endpointValue: TextView
    private lateinit var statusCard: LinearLayout
    private lateinit var statusChip: TextView
    private lateinit var statusTitle: TextView
    private lateinit var statusBody: TextView
    private lateinit var progress: ProgressBar
    private lateinit var confirm: TextView
    private lateinit var secondary: TextView
    private lateinit var flow: EnrollmentFlowViewModel
    private var observerGeneration: Long? = null
    private var currentState = EnrollmentScreenState()

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        flow =
            ViewModelProvider(
                this,
                EnrollmentFlowViewModel.Factory(applicationContext),
            )[EnrollmentFlowViewModel::class.java]
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)

        val content =
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                clipToPadding = false
                clipChildren = false
            }

        content.addView(buildHeader())
        content.addView(buildIntro(), marginParams(top = 22))
        entryCard = buildEntryCard()
        content.addView(entryCard, marginParams(top = 14))
        endpointCard = buildEndpointCard().also { it.visibility = View.GONE }
        content.addView(endpointCard, marginParams(top = 14))
        statusCard = buildStatusCard().also { it.visibility = View.GONE }
        content.addView(statusCard, marginParams(top = 14))
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

        ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
            view.setPadding(
                dp(20) + bars.left,
                dp(24) + bars.top,
                dp(20) + bars.right,
                dp(28) + maxOf(bars.bottom, ime.bottom),
            )
            insets
        }

        render(flow.screenState)
        flow.acceptInitialLink(intent?.dataString)
    }

    override fun onStart() {
        super.onStart()
        observerGeneration = flow.attach(::render)
    }

    override fun onStop() {
        observerGeneration?.let(flow::detach)
        observerGeneration = null
        super.onStop()
    }

    override fun onNewIntent(intent: Intent?) {
        super.onNewIntent(intent)
        setIntent(intent)
        intent?.dataString?.let(flow::offerLink)
    }

    private fun buildHeader(): View {
        val row =
            LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
            }
        row.addView(
            ImageView(this).apply {
                setImageResource(R.mipmap.ic_launcher)
                contentDescription = null
            },
            LinearLayout.LayoutParams(dp(58), dp(58)),
        )
        row.addView(
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(dp(14), 0, 0, 0)
                addView(
                    TextView(this@EnrollmentActivity).apply {
                        text = getString(R.string.app_name)
                        setTextColor(color(R.color.sky_heading))
                        setTextSize(TypedValue.COMPLEX_UNIT_SP, 22f)
                        typeface = Typeface.create("sans-serif", Typeface.BOLD)
                        letterSpacing = -0.01f
                    },
                )
                addView(
                    TextView(this@EnrollmentActivity).apply {
                        text = getString(R.string.enrollment_header_subtitle)
                        setTextColor(color(R.color.enrollment_muted))
                        setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                        setPadding(0, dp(2), 0, 0)
                    },
                )
            },
            LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f),
        )
        return row
    }

    private fun buildIntro(): LinearLayout =
        card().apply {
            addView(sectionLabel(getString(R.string.enrollment_section_pairing)))
            addView(
                TextView(this@EnrollmentActivity).apply {
                    text = getString(R.string.enrollment_title)
                    setTextColor(color(R.color.sky_heading))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 23f)
                    typeface = Typeface.create("sans-serif", Typeface.BOLD)
                    letterSpacing = -0.01f
                    setPadding(0, dp(8), 0, 0)
                },
            )
            addView(
                TextView(this@EnrollmentActivity).apply {
                    text = getString(R.string.enrollment_intro)
                    setTextColor(color(R.color.enrollment_muted))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 14.5f)
                    setLineSpacing(0f, 1.16f)
                    setPadding(0, dp(7), 0, 0)
                },
            )
            addView(
                LinearLayout(this@EnrollmentActivity).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                    setPadding(0, dp(16), 0, 0)
                    addView(statusDot(R.color.sky_ok))
                    addView(
                        TextView(this@EnrollmentActivity).apply {
                            text = getString(R.string.enrollment_security_note)
                            setTextColor(color(R.color.sky_text))
                            setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                            setPadding(dp(10), 0, 0, 0)
                        },
                        LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f),
                    )
                },
            )
        }

    private fun buildEntryCard(): LinearLayout =
        card().apply {
            val inputId = View.generateViewId()
            addView(sectionLabel(getString(R.string.enrollment_section_manual)))
            addView(
                TextView(this@EnrollmentActivity).apply {
                    text = getString(R.string.enrollment_manual_title)
                    setTextColor(color(R.color.sky_text))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
                    typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
                    setPadding(0, dp(9), 0, 0)
                    labelFor = inputId
                },
            )
            addView(
                TextView(this@EnrollmentActivity).apply {
                    text = getString(R.string.enrollment_manual_help)
                    setTextColor(color(R.color.enrollment_muted))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
                    setLineSpacing(0f, 1.12f)
                    setPadding(0, dp(4), 0, 0)
                },
            )
            input =
                EditText(this@EnrollmentActivity).apply {
                    id = inputId
                    hint = getString(R.string.enrollment_input_hint)
                    setHintTextColor(color(R.color.enrollment_muted))
                    setTextColor(color(R.color.sky_text))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
                    gravity = Gravity.TOP or Gravity.START
                    minLines = 3
                    maxLines = 5
                    inputType =
                        InputType.TYPE_CLASS_TEXT or
                            InputType.TYPE_TEXT_VARIATION_PASSWORD or
                            InputType.TYPE_TEXT_FLAG_MULTI_LINE or
                            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                    transformationMethod = PasswordTransformationMethod.getInstance()
                    importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
                    // Enrollment material is deliberately absent from both Activity SavedState
                    // and the parent view hierarchy. The retained flow owns parsed state in memory.
                    isSaveEnabled = false
                    isSaveFromParentEnabled = false
                    tag = ENROLLMENT_INPUT_TAG
                    background =
                        rounded(
                            color(R.color.sky_bg),
                            12f,
                            color(R.color.sky_card_border),
                            HAIRLINE_PX,
                        )
                    setPadding(dp(14), dp(12), dp(14), dp(12))
                    contentDescription = getString(R.string.enrollment_input_accessibility)
                }
            addView(input, marginParams(top = 12))
            addView(
                TextView(this@EnrollmentActivity).apply {
                    text = getString(R.string.enrollment_input_privacy)
                    setTextColor(color(R.color.enrollment_muted))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 12f)
                    setPadding(0, dp(8), 0, 0)
                },
            )
        }

    private fun buildEndpointCard(): LinearLayout =
        card().apply {
            addView(
                LinearLayout(this@EnrollmentActivity).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                    addView(
                        sectionLabel(getString(R.string.enrollment_section_destination)),
                        LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f),
                    )
                    addView(chip(getString(R.string.enrollment_chip_endpoint), ChipKind.NEUTRAL))
                },
            )
            addView(
                TextView(this@EnrollmentActivity).apply {
                    text = getString(R.string.enrollment_endpoint_label)
                    setTextColor(color(R.color.sky_text))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
                    typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
                    setPadding(0, dp(13), 0, 0)
                },
            )
            endpointValue =
                TextView(this@EnrollmentActivity).apply {
                    setTextColor(color(R.color.sky_heading))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
                    typeface = Typeface.MONOSPACE
                    setTextIsSelectable(true)
                    setLineSpacing(0f, 1.12f)
                    background = rounded(color(R.color.sky_bg), 11f)
                    setPadding(dp(13), dp(11), dp(13), dp(11))
                }
            addView(endpointValue, marginParams(top = 8))
            addView(
                TextView(this@EnrollmentActivity).apply {
                    text = getString(R.string.enrollment_endpoint_help)
                    setTextColor(color(R.color.enrollment_muted))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 12.5f)
                    setLineSpacing(0f, 1.12f)
                    setPadding(0, dp(10), 0, 0)
                },
            )
        }

    private fun buildStatusCard(): LinearLayout =
        card().apply {
            addView(
                LinearLayout(this@EnrollmentActivity).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                    statusTitle =
                        TextView(this@EnrollmentActivity).apply {
                            setTextColor(color(R.color.sky_heading))
                            setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
                            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
                        }
                    statusChip = chip("", ChipKind.NEUTRAL)
                    addView(statusTitle, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
                    addView(statusChip)
                },
            )
            statusBody =
                TextView(this@EnrollmentActivity).apply {
                    setTextColor(color(R.color.enrollment_muted))
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 13.5f)
                    setLineSpacing(0f, 1.15f)
                    setPadding(0, dp(8), 0, 0)
                    accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
                }
            addView(statusBody)
            progress =
                ProgressBar(
                    this@EnrollmentActivity,
                    null,
                    android.R.attr.progressBarStyleHorizontal,
                ).apply {
                    isIndeterminate = true
                    indeterminateTintList = ColorStateList.valueOf(color(R.color.sky_primary))
                    visibility = View.GONE
                    contentDescription = getString(R.string.enrollment_connecting_progress)
                }
            addView(progress, marginParams(top = 12))
        }

    private fun buildActions(): LinearLayout =
        LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(12), 0, dp(12), 0)
            confirm =
                button(
                    label = getString(R.string.enrollment_review_action),
                    textColor = color(R.color.enrollment_on_primary),
                    fill = rounded(color(R.color.sky_primary), 13f),
                    ripple = color(R.color.sky_ripple_on_primary),
                ) {
                    when (currentState.phase) {
                        EnrollmentScreenPhase.ENTRY -> {
                            val raw = input.text.toString()
                            input.setText("")
                            flow.review(raw)
                        }
                        EnrollmentScreenPhase.REVIEW -> flow.confirmEndpoint()
                        EnrollmentScreenPhase.REPLACE -> flow.submitPending()
                        EnrollmentScreenPhase.SUCCESS -> finish()
                        EnrollmentScreenPhase.ERROR -> resetToEntry(focusInput = true)
                        EnrollmentScreenPhase.CONNECTING -> Unit
                    }
                }
            addView(confirm)
            secondary =
                button(
                    label = getString(R.string.enrollment_not_now),
                    textColor = color(R.color.sky_on_secondary),
                    fill = rounded(color(R.color.sky_secondary), 13f),
                    ripple = color(R.color.sky_ripple_on_secondary),
                ) {
                    when (currentState.phase) {
                        EnrollmentScreenPhase.ENTRY,
                        EnrollmentScreenPhase.SUCCESS,
                        -> finish()
                        EnrollmentScreenPhase.REVIEW,
                        EnrollmentScreenPhase.REPLACE,
                        EnrollmentScreenPhase.ERROR,
                        -> resetToEntry(focusInput = true)
                        EnrollmentScreenPhase.CONNECTING -> Unit
                    }
                }
            addView(secondary, marginParams(top = 10))
        }

    private fun render(state: EnrollmentScreenState) {
        currentState = state
        val phase = state.phase
        val hasEndpoint = state.endpoint != null && phase != EnrollmentScreenPhase.ENTRY
        entryCard.visibility = if (phase == EnrollmentScreenPhase.ENTRY) View.VISIBLE else View.GONE
        endpointCard.visibility = if (hasEndpoint) View.VISIBLE else View.GONE
        if (hasEndpoint) endpointValue.text = state.endpoint

        statusCard.visibility = if (phase == EnrollmentScreenPhase.ENTRY) View.GONE else View.VISIBLE
        progress.visibility = if (phase == EnrollmentScreenPhase.CONNECTING) View.VISIBLE else View.GONE
        confirm.isEnabled = phase != EnrollmentScreenPhase.CONNECTING
        confirm.alpha = if (confirm.isEnabled) 1f else 0.55f
        secondary.isEnabled = phase != EnrollmentScreenPhase.CONNECTING
        secondary.visibility =
            if (phase == EnrollmentScreenPhase.CONNECTING || phase == EnrollmentScreenPhase.SUCCESS) {
                View.GONE
            } else {
                View.VISIBLE
            }

        when (phase) {
            EnrollmentScreenPhase.ENTRY -> {
                confirm.text = getString(R.string.enrollment_review_action)
                secondary.text = getString(R.string.enrollment_not_now)
            }
            EnrollmentScreenPhase.REVIEW -> {
                setStatus(
                    getString(R.string.enrollment_review_title),
                    getString(R.string.enrollment_chip_review),
                    ChipKind.NEUTRAL,
                    messageFor(state),
                )
                confirm.text = getString(R.string.enrollment_confirm_endpoint)
                secondary.text = getString(R.string.enrollment_use_another)
            }
            EnrollmentScreenPhase.REPLACE -> {
                setStatus(
                    getString(R.string.enrollment_replace_title),
                    getString(R.string.enrollment_chip_attention),
                    ChipKind.OFF,
                    messageFor(state),
                )
                confirm.text = getString(R.string.enrollment_replace_action)
                secondary.text = getString(R.string.enrollment_keep_existing)
            }
            EnrollmentScreenPhase.CONNECTING -> {
                setStatus(
                    getString(R.string.enrollment_connecting_title),
                    getString(R.string.enrollment_chip_connecting),
                    ChipKind.NEUTRAL,
                    messageFor(state),
                )
                confirm.text = getString(R.string.enrollment_connecting_action)
            }
            EnrollmentScreenPhase.SUCCESS -> {
                setStatus(
                    getString(R.string.enrollment_connected_title),
                    getString(R.string.enrollment_chip_connected),
                    ChipKind.OK,
                    messageFor(state),
                )
                confirm.text = getString(R.string.enrollment_done)
                secondary.text = getString(R.string.enrollment_close)
            }
            EnrollmentScreenPhase.ERROR -> {
                setStatus(
                    getString(R.string.enrollment_problem_title),
                    getString(R.string.enrollment_chip_not_connected),
                    ChipKind.OFF,
                    messageFor(state),
                )
                confirm.text = getString(R.string.enrollment_enter_another)
                secondary.text = getString(R.string.enrollment_cancel)
            }
        }
    }

    private fun setStatus(
        title: String,
        chipText: String,
        kind: ChipKind,
        body: String,
    ) {
        statusTitle.text = title
        statusBody.text = body
        applyChip(statusChip, chipText, kind)
    }

    private fun resetToEntry(focusInput: Boolean = false) {
        input.setText("")
        flow.reset()
        if (focusInput) {
            input.requestFocus()
            input.post {
                (getSystemService(INPUT_METHOD_SERVICE) as? android.view.inputmethod.InputMethodManager)
                    ?.showSoftInput(input, android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT)
            }
        }
    }

    private fun messageFor(state: EnrollmentScreenState): String =
        when (state.notice) {
            EnrollmentNotice.NONE -> ""
            EnrollmentNotice.REVIEW_READY -> getString(R.string.enrollment_review_ready)
            EnrollmentNotice.REVIEW_EXISTING -> getString(R.string.enrollment_review_existing)
            EnrollmentNotice.REPLACE_WARNING -> getString(R.string.enrollment_replace_warning)
            EnrollmentNotice.CREDENTIAL_CHANGED -> getString(R.string.enrollment_credential_changed)
            EnrollmentNotice.CONNECTING -> getString(R.string.enrollment_connecting_body)
            EnrollmentNotice.FINISHING_CURRENT -> getString(R.string.enrollment_finishing_current)
            EnrollmentNotice.CONNECTED -> getString(R.string.enrollment_connected_body)
            EnrollmentNotice.INVALID_OR_EXPIRED -> getString(R.string.enrollment_invalid_or_expired)
            EnrollmentNotice.FAILED ->
                getString(
                    R.string.enrollment_failure_body,
                    when (state.failureReason) {
                        EnrollmentFailureReason.EXPIRED -> getString(R.string.enrollment_failure_expired)
                        EnrollmentFailureReason.UNREACHABLE -> getString(R.string.enrollment_failure_unreachable)
                        EnrollmentFailureReason.SAVE -> getString(R.string.enrollment_failure_save)
                        EnrollmentFailureReason.REJECTED,
                        null,
                        -> getString(R.string.enrollment_failure_rejected)
                    },
                )
        }

    private fun card(): LinearLayout =
        LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background =
                rounded(
                    color(R.color.sky_surface),
                    16f,
                    color(R.color.sky_card_border),
                    HAIRLINE_PX,
                )
            setPadding(dp(20), dp(18), dp(20), dp(20))
            elevation = dp(1).toFloat()
            outlineProvider = android.view.ViewOutlineProvider.BACKGROUND
            clipToPadding = false
        }

    private fun sectionLabel(text: String): TextView =
        TextView(this).apply {
            this.text = text.uppercase()
            setTextColor(color(R.color.enrollment_muted))
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 11f)
            letterSpacing = 0.14f
            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        }

    private enum class ChipKind { OK, OFF, NEUTRAL }

    private fun chip(text: String, kind: ChipKind): TextView =
        TextView(this).apply { applyChip(this, text, kind) }

    private fun applyChip(view: TextView, text: String, kind: ChipKind) {
        @ColorRes val foreground: Int
        @ColorRes val background: Int
        @ColorRes val border: Int
        when (kind) {
            ChipKind.OK -> {
                foreground = R.color.sky_ok
                background = R.color.sky_ok_bg
                border = R.color.sky_ok_border
            }
            ChipKind.OFF -> {
                foreground = R.color.sky_off
                background = R.color.sky_off_bg
                border = R.color.sky_off_border
            }
            ChipKind.NEUTRAL -> {
                foreground = R.color.enrollment_muted
                background = R.color.sky_secondary
                border = R.color.sky_neutral_border
            }
        }
        view.apply {
            this.text = text
            isSingleLine = true
            gravity = Gravity.CENTER
            setTextColor(color(foreground))
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 11.5f)
            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
            this.background = rounded(color(background), 999f, color(border), HAIRLINE_PX)
            setPadding(dp(11), dp(5), dp(11), dp(6))
        }
    }

    private fun statusDot(@ColorRes colorId: Int): View =
        View(this).apply {
            background =
                GradientDrawable().apply {
                    shape = GradientDrawable.OVAL
                    setColor(color(colorId))
                }
            layoutParams = LinearLayout.LayoutParams(dp(8), dp(8))
        }

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
            setPadding(dp(16), dp(13), dp(16), dp(13))
            isClickable = true
            isFocusable = true
            minHeight = dp(48)
            setOnClickListener { onClick() }
            layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT)
        }

    private fun rounded(
        fill: Int,
        radiusDp: Float,
        stroke: Int? = null,
        strokeWidth: Int = 0,
    ): GradientDrawable =
        GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = radiusDp * resources.displayMetrics.density
            setColor(fill)
            if (stroke != null) setStroke(strokeWidth, stroke)
        }

    private fun marginParams(top: Int = 0): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(top) }

    private fun color(@ColorRes id: Int): Int = ContextCompat.getColor(this, id)

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    private companion object {
        const val HAIRLINE_PX = 2
    }
}

internal const val ENROLLMENT_INPUT_TAG = "private-enrollment-input"
