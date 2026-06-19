package com.skycua.phonecompanion.overlay

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BlurMaskFilter
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffColorFilter
import android.graphics.RadialGradient
import android.graphics.RectF
import android.graphics.Shader
import android.view.View
import androidx.appcompat.content.res.AppCompatResources
import com.skycua.phonecompanion.R
import kotlin.math.ceil

/**
 * The single full-screen, pass-through overlay drawn by the companion's
 * AccessibilityService. Driven entirely by mutable state set from the
 * controller, it renders the "agent in control" visuals:
 *
 *  - a soft pastel-pink screen-edge glow, with waves of light that emanate from
 *    the edges and travel inward toward the centre (concentric rounded-rect
 *    pulses) over a gently breathing base,
 *  - the agent cursor — the same `cursor-chat` pointer the desktop agent uses,
 *    scaled up and seated in a soft pink halo that breathes,
 *  - a tap ripple (an expanding, fading ring), and
 *  - a swipe/drag trail (a fading polyline plus the moving cursor).
 *
 * The window flags supplied by the controller ([OverlayFlags.passThroughFlags])
 * make the view non-focusable and non-touchable, so it never intercepts input.
 * `onDraw` is allocation-free: every `Paint`, `RectF`, the trail buffer, and the
 * scaled cursor bitmap are built once and reused.
 *
 * All geometry is expressed in dp and converted with the display density, so the
 * glow and cursor read at a consistent physical size across phones rather than
 * vanishing on a high-DPI panel. Coordinates are Android device pixels
 * (post-rotation display pixels); because the overlay is laid out at the
 * top-left covering the whole display, view-local pixels equal device pixels.
 */
class AgentOverlayView(context: Context) : View(context) {
    private val density = resources.displayMetrics.density

    private fun dp(value: Float): Float = value * density

    // --- edge glow + inward waves ---------------------------------------------

    /** Wide, soft base border — the steady "framed in pink" glow. */
    private val glowBasePaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            color = PINK
            strokeWidth = dp(GLOW_BASE_STROKE_DP)
            maskFilter = BlurMaskFilter(dp(GLOW_BASE_BLUR_DP), BlurMaskFilter.Blur.NORMAL)
        }

    /** Tighter inner edge so the border has a soft luminous core. */
    private val glowCorePaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            color = PINK_LIGHT
            strokeWidth = dp(GLOW_CORE_STROKE_DP)
            maskFilter = BlurMaskFilter(dp(GLOW_CORE_BLUR_DP), BlurMaskFilter.Blur.NORMAL)
        }

    /** A single inward-travelling wave; alpha is set per-wave each frame. */
    private val wavePaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            color = PINK_LIGHT
            strokeWidth = dp(WAVE_STROKE_DP)
            maskFilter = BlurMaskFilter(dp(WAVE_BLUR_DP), BlurMaskFilter.Blur.NORMAL)
        }
    private val glowRect = RectF()
    private val waveRect = RectF()

    /** Base glow intensity in [0, 1]; 0 hides the border. Driven by breathing. */
    @Volatile
    private var glowIntensity: Float = 0f

    /** Inward-wave phase in [0, 1); 0 spawns a wave at the edge, 1 at full depth. */
    @Volatile
    private var wavePhase: Float = 0f

    // --- cursor ---------------------------------------------------------------

    // Rasterize the vector pointer at the exact target pixel size. Drawing the
    // vector straight to the final bitmap keeps it crisp; the runtime only ever
    // scales the cursor DOWN (press squash), so it never upsamples a raster.
    private val cursorBitmap: Bitmap =
        run {
            val drawable = AppCompatResources.getDrawable(context, R.drawable.agent_cursor)!!
            val h = dp(CURSOR_HEIGHT_DP).toInt().coerceAtLeast(1)
            val w = (h * drawable.intrinsicWidth / drawable.intrinsicHeight).coerceAtLeast(1)
            Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888).also { bmp ->
                drawable.setBounds(0, 0, w, h)
                drawable.draw(Canvas(bmp))
            }
        }

    // Hotspot within the scaled bitmap (the point the cursor "points at").
    private val cursorHotspotX = cursorBitmap.width * HOTSPOT_FRACTION_X
    private val cursorHotspotY = cursorBitmap.height * HOTSPOT_FRACTION_Y

    private val cursorPaint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG)

    // Soft drop shadow for the raised look, matched to the desktop cursor. A
    // VectorDrawable cannot blur, so the cursor silhouette is pre-rendered once as
    // a blurred, semi-transparent black blob and composited behind the pointer.
    // Offset and blur are expressed as fractions of the 48-unit glyph viewBox so
    // the shadow scales with the cursor size; it is drawn inside the pointer's
    // rotation so it stays glued to the cursor shape.
    private val cursorShadowDx: Float = SHADOW_DX_VB * dp(CURSOR_HEIGHT_DP) / VIEWBOX_HEIGHT
    private val cursorShadowDy: Float = SHADOW_DY_VB * dp(CURSOR_HEIGHT_DP) / VIEWBOX_HEIGHT
    private val cursorShadowPad: Int =
        ceil(SHADOW_BLUR_VB * dp(CURSOR_HEIGHT_DP) / VIEWBOX_HEIGHT * 3f).toInt().coerceAtLeast(1)
    private val cursorShadowBitmap: Bitmap =
        run {
            val blurPx = (SHADOW_BLUR_VB * dp(CURSOR_HEIGHT_DP) / VIEWBOX_HEIGHT).coerceAtLeast(0.5f)
            val shadow =
                Bitmap.createBitmap(
                    cursorBitmap.width + cursorShadowPad * 2,
                    cursorBitmap.height + cursorShadowPad * 2,
                    Bitmap.Config.ARGB_8888,
                )
            val paint =
                Paint(Paint.ANTI_ALIAS_FLAG).apply {
                    colorFilter =
                        PorterDuffColorFilter(android.graphics.Color.BLACK, PorterDuff.Mode.SRC_IN)
                    maskFilter = BlurMaskFilter(blurPx, BlurMaskFilter.Blur.NORMAL)
                    alpha = (SHADOW_ALPHA * 255f).toInt()
                }
            Canvas(shadow).drawBitmap(
                cursorBitmap,
                cursorShadowPad.toFloat(),
                cursorShadowPad.toFloat(),
                paint,
            )
            shadow
        }

    /** Soft pink halo behind the cursor; centred at the origin and translated. */
    private val cursorHaloPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            shader =
                RadialGradient(
                    0f,
                    0f,
                    dp(CURSOR_HALO_RADIUS_DP),
                    intArrayOf(HALO_INNER, HALO_MID, HALO_OUTER),
                    floatArrayOf(0f, 0.55f, 1f),
                    Shader.TileMode.CLAMP,
                )
        }

    @Volatile
    private var cursorVisible: Boolean = false

    @Volatile
    private var cursorX: Float = 0f

    @Volatile
    private var cursorY: Float = 0f

    /** Drawn rotation of the pointer (degrees); noses into the travel heading. */
    @Volatile
    private var cursorRotationDeg: Float = 0f

    /** Drawn scale of the cursor; squashes on press and bounces back to 1.0. */
    @Volatile
    private var cursorScale: Float = 1f

    /** Halo breathing in [0, 1]; scales the halo radius and opacity. */
    @Volatile
    private var cursorPulse: Float = 0.5f

    // --- tap ripple -----------------------------------------------------------

    private val ripplePaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = dp(RIPPLE_STROKE_DP)
            color = PINK_LIGHT
            // Wide, blurred band so the ripple reads as the cursor's glow pulsing
            // outward, not a hard ring outline.
            maskFilter = BlurMaskFilter(dp(RIPPLE_BLUR_DP), BlurMaskFilter.Blur.NORMAL)
        }

    @Volatile
    private var rippleActive: Boolean = false

    @Volatile
    private var rippleX: Float = 0f

    @Volatile
    private var rippleY: Float = 0f

    @Volatile
    private var rippleRadius: Float = 0f

    @Volatile
    private var rippleAlpha: Float = 0f

    // --- swipe / drag trail ---------------------------------------------------

    private val trailPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = dp(TRAIL_STROKE_DP)
            strokeCap = Paint.Cap.ROUND
            color = PINK
        }

    // Pre-sized, mutated-in-place buffer of trail points: [x0, y0, x1, y1, ...].
    private val trailBuffer = FloatArray(TRAIL_MAX_POINTS * 2)

    @Volatile
    private var trailCount: Int = 0

    @Volatile
    private var trailAlpha: Float = 0f

    // --- state setters (called on the main thread by the controller) ---------

    /** Sets the base glow intensity in [0, 1]; values are clamped. */
    fun setGlowIntensity(intensity: Float) {
        glowIntensity = OverlayMath.clamp01(intensity)
        invalidate()
    }

    /** Sets the inward-wave phase in [0, 1). */
    fun setWavePhase(phase: Float) {
        wavePhase = phase - kotlin.math.floor(phase)
        invalidate()
    }

    /** Sets the cursor-halo breathing value in [0, 1]; values are clamped. */
    fun setCursorPulse(pulse: Float) {
        cursorPulse = OverlayMath.clamp01(pulse)
        invalidate()
    }

    /** Sets the agent cursor position and visibility in device pixels. */
    fun setCursor(visible: Boolean, x: Float, y: Float) {
        cursorVisible = visible
        cursorX = x
        cursorY = y
        invalidate()
    }

    /** Sets the drawn pointer rotation in degrees (nose into travel heading). */
    fun setCursorRotation(degrees: Float) {
        cursorRotationDeg = degrees
        invalidate()
    }

    /** Sets the cursor press scale (1.0 = rest); squashes and bounces on a gesture. */
    fun setCursorScale(scale: Float) {
        cursorScale = scale
        invalidate()
    }

    /** Updates the active tap ripple. [alpha] <= 0 retires it. */
    fun setRipple(x: Float, y: Float, radius: Float, alpha: Float) {
        rippleX = x
        rippleY = y
        rippleRadius = radius
        rippleAlpha = OverlayMath.clamp01(alpha)
        rippleActive = rippleAlpha > 0f && radius > 0f
        invalidate()
    }

    /** Clears any active tap ripple. */
    fun clearRipple() {
        rippleActive = false
        rippleAlpha = 0f
        invalidate()
    }

    /**
     * Sets the swipe/drag trail from device-pixel [points] (head last) at the
     * given [alpha]. Points beyond [TRAIL_MAX_POINTS] are dropped from the tail
     * so the head is always retained. Allocation-free: copies into the
     * pre-sized buffer.
     */
    fun setTrail(points: List<OverlayMath.Point>, alpha: Float) {
        val count = minOf(points.size, TRAIL_MAX_POINTS)
        val start = points.size - count
        for (i in 0 until count) {
            val p = points[start + i]
            trailBuffer[i * 2] = p.x
            trailBuffer[i * 2 + 1] = p.y
        }
        trailCount = count
        trailAlpha = OverlayMath.clamp01(alpha)
        invalidate()
    }

    /** Clears the swipe/drag trail. */
    fun clearTrail() {
        trailCount = 0
        trailAlpha = 0f
        invalidate()
    }

    /** Hides every overlay element at once for a clean screenshot. */
    fun clearAll() {
        glowIntensity = 0f
        cursorVisible = false
        cursorScale = 1f
        rippleActive = false
        rippleAlpha = 0f
        trailCount = 0
        trailAlpha = 0f
        invalidate()
    }

    // --- drawing --------------------------------------------------------------

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        drawGlow(canvas)
        drawWaves(canvas)
        drawTrail(canvas)
        drawRipple(canvas)
        drawCursor(canvas)
    }

    private fun roundRectAt(rect: RectF, inset: Float) {
        rect.set(inset, inset, width - inset, height - inset)
    }

    private fun drawGlow(canvas: Canvas) {
        val intensity = glowIntensity
        if (intensity <= 0f) return
        // Hug the very screen edge: the stroke straddles the border so the outer
        // half spills off-screen, leaving a band of glow tight against the edge.
        roundRectAt(glowRect, dp(GLOW_EDGE_INSET_DP))
        val corner = dp(GLOW_CORNER_DP)
        glowBasePaint.alpha = (intensity * MAX_BASE_ALPHA).toInt().coerceIn(0, 255)
        canvas.drawRoundRect(glowRect, corner, corner, glowBasePaint)
        glowCorePaint.alpha = (intensity * MAX_CORE_ALPHA).toInt().coerceIn(0, 255)
        canvas.drawRoundRect(glowRect, corner, corner, glowCorePaint)
    }

    /**
     * Draws [WAVE_COUNT] concentric rounded-rect pulses staggered in phase. Each
     * spawns at the screen edge and contracts inward toward the centre while
     * fading, so the border continuously emits waves of light moving inward.
     */
    private fun drawWaves(canvas: Canvas) {
        if (glowIntensity <= 0f) return
        val edge = dp(GLOW_EDGE_INSET_DP)
        val travel = minOf(width, height) * WAVE_TRAVEL_FRACTION
        val baseCorner = dp(GLOW_CORNER_DP)
        for (i in 0 until WAVE_COUNT) {
            var phase = wavePhase + i.toFloat() / WAVE_COUNT
            phase -= kotlin.math.floor(phase)
            val inset = edge + phase * travel
            // Quick fade-in off the edge, then fade out as it travels inward.
            val fadeIn = OverlayMath.clamp01(phase / WAVE_FADE_IN)
            val fade = fadeIn * (1f - phase)
            val alpha = (fade * WAVE_MAX_ALPHA * glowIntensity).toInt().coerceIn(0, 255)
            if (alpha <= 0) continue
            roundRectAt(waveRect, inset)
            wavePaint.alpha = alpha
            val corner = (baseCorner - phase * travel).coerceAtLeast(dp(8f))
            canvas.drawRoundRect(waveRect, corner, corner, wavePaint)
        }
    }

    private fun drawTrail(canvas: Canvas) {
        val count = trailCount
        if (count < 2 || trailAlpha <= 0f) return
        for (i in 1 until count) {
            val segAlpha = OverlayMath.trailAlpha(i, count, trailAlpha) * MAX_TRAIL_ALPHA
            trailPaint.alpha = segAlpha.toInt().coerceIn(0, 255)
            canvas.drawLine(
                trailBuffer[(i - 1) * 2],
                trailBuffer[(i - 1) * 2 + 1],
                trailBuffer[i * 2],
                trailBuffer[i * 2 + 1],
                trailPaint,
            )
        }
    }

    private fun drawRipple(canvas: Canvas) {
        if (!rippleActive) return
        ripplePaint.alpha = (rippleAlpha * MAX_RIPPLE_ALPHA).toInt().coerceIn(0, 255)
        canvas.drawCircle(rippleX, rippleY, rippleRadius, ripplePaint)
    }

    private fun drawCursor(canvas: Canvas) {
        if (!cursorVisible) return
        canvas.save()
        canvas.translate(cursorX, cursorY)
        // Press squash: scales the whole cursor (halo + pointer) about the hotspot.
        canvas.scale(cursorScale, cursorScale)
        // Breathing halo: the radius and opacity swell and ebb with [cursorPulse].
        val haloScale = HALO_SCALE_MIN + (HALO_SCALE_MAX - HALO_SCALE_MIN) * cursorPulse
        val haloAlpha = HALO_ALPHA_MIN + (HALO_ALPHA_MAX - HALO_ALPHA_MIN) * cursorPulse
        canvas.save()
        canvas.scale(haloScale, haloScale)
        cursorHaloPaint.alpha = (haloAlpha * 255f).toInt().coerceIn(0, 255)
        canvas.drawCircle(0f, 0f, dp(CURSOR_HALO_RADIUS_DP), cursorHaloPaint)
        canvas.restore()
        // Pointer at a fixed size, rotated to face its heading about the hotspot.
        canvas.rotate(cursorRotationDeg)
        // Soft drop shadow beneath the pointer, offset down-right for a raised look.
        canvas.drawBitmap(
            cursorShadowBitmap,
            -cursorHotspotX - cursorShadowPad + cursorShadowDx,
            -cursorHotspotY - cursorShadowPad + cursorShadowDy,
            cursorPaint,
        )
        canvas.drawBitmap(cursorBitmap, -cursorHotspotX, -cursorHotspotY, cursorPaint)
        canvas.restore()
    }

    companion object {
        // Pink agent palette — soft pastel character but a rich, saturated pink.
        val PINK: Int = android.graphics.Color.rgb(255, 96, 172) // intense rose pink
        val PINK_LIGHT: Int = android.graphics.Color.rgb(255, 150, 205) // lighter highlight

        // Cursor halo (radial-gradient stops, centre -> edge).
        val HALO_INNER: Int = android.graphics.Color.argb(205, 255, 118, 188)
        val HALO_MID: Int = android.graphics.Color.argb(90, 255, 118, 188)
        val HALO_OUTER: Int = android.graphics.Color.argb(0, 255, 118, 188)

        // Edge glow geometry (dp). Wide soft blur, but a saturated, present pink.
        const val GLOW_BASE_STROKE_DP: Float = 22f
        const val GLOW_BASE_BLUR_DP: Float = 22f
        const val GLOW_CORE_STROKE_DP: Float = 6f
        const val GLOW_CORE_BLUR_DP: Float = 9f
        const val GLOW_EDGE_INSET_DP: Float = 2f
        const val GLOW_CORNER_DP: Float = 46f
        const val MAX_BASE_ALPHA: Float = 200f
        const val MAX_CORE_ALPHA: Float = 220f

        // Inward-wave geometry.
        const val WAVE_COUNT: Int = 3
        const val WAVE_STROKE_DP: Float = 5f
        const val WAVE_BLUR_DP: Float = 9f
        const val WAVE_TRAVEL_FRACTION: Float = 0.20f // of min(screen) travelled inward
        const val WAVE_FADE_IN: Float = 0.12f // phase fraction spent fading in off the edge
        const val WAVE_MAX_ALPHA: Float = 165f

        // Cursor geometry (dp). The source pointer is 46x48 with its hotspot
        // matching the desktop agent cursor (10/23, 11/24 of the rendered glyph).
        // The rendered height and halo are sized as a unit so the pointer reads at
        // a consistent proportion across phones.
        const val CURSOR_HEIGHT_DP: Float = 35.9375f
        const val CURSOR_HALO_RADIUS_DP: Float = 23.4375f

        // Drop shadow matched to the desktop cursor (c1): offset and blur as
        // fractions of the 48-unit glyph viewBox so the raised look scales with
        // the rendered cursor size.
        const val VIEWBOX_HEIGHT: Float = 48f
        const val SHADOW_DX_VB: Float = 0.5f
        const val SHADOW_DY_VB: Float = 1.3f
        const val SHADOW_BLUR_VB: Float = 1.1f
        const val SHADOW_ALPHA: Float = 0.58f
        const val HOTSPOT_FRACTION_X: Float = 10f / 23f
        const val HOTSPOT_FRACTION_Y: Float = 11f / 24f

        // Cursor-halo breathing band (scale + opacity at pulse 0 -> 1).
        const val HALO_SCALE_MIN: Float = 0.85f
        const val HALO_SCALE_MAX: Float = 1.10f
        const val HALO_ALPHA_MIN: Float = 0.5f
        const val HALO_ALPHA_MAX: Float = 1.0f

        // Ripple geometry (dp): a wide, soft glow band (not a thin line).
        const val RIPPLE_STROKE_DP: Float = 16f
        const val RIPPLE_BLUR_DP: Float = 14f
        const val MAX_RIPPLE_ALPHA: Float = 215f

        // Trail geometry (dp).
        const val TRAIL_STROKE_DP: Float = 6f
        const val MAX_TRAIL_ALPHA: Float = 190f
        const val TRAIL_MAX_POINTS: Int = 24
    }
}
