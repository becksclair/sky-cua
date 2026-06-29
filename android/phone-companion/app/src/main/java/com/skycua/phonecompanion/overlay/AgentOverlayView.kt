package com.skycua.phonecompanion.overlay

import android.content.Context
import android.graphics.BlurMaskFilter
import android.graphics.Canvas
import android.graphics.Paint
import android.view.View

/**
 * The single full-screen, pass-through overlay drawn by the companion's
 * AccessibilityService. Driven entirely by mutable state set from the
 * controller, it renders the "agent in control" visuals:
 *
 *  - the desktop's smoky fbm screen-edge glow ([AgslEdgeGlowRenderer]),
 *  - the agent cursor — the desktop's tilted plum/pink glyph with its grounding
 *    shadow and glyph-anchored smoke aura ([AgslCursorRenderer]),
 *  - a tap ripple (an expanding, fading ring), and
 *  - a swipe/drag trail (a fading polyline plus the moving cursor).
 *
 * The window flags supplied by the controller ([OverlayFlags.passThroughFlags])
 * make the view non-focusable and non-touchable, so it never intercepts input.
 * `onDraw` is allocation-free: every `Paint`, the trail buffer, and the AGSL
 * renderers' uniforms are built/reused without per-frame allocation.
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

    // --- edge glow ------------------------------------------------------------

    /**
     * AGSL edge-glow renderer (the desktop's smoky fbm border). Cheap to
     * [AgslEdgeGlowRenderer.prepare] (no texture), so it is prepared on the main
     * thread from [onAttachedToWindow].
     */
    private val agslEdgeGlowRenderer = AgslEdgeGlowRenderer()

    /** Base glow intensity in [0, 1]; 0 hides the border. Driven by breathing. */
    @Volatile
    private var glowIntensity: Float = 0f

    // --- cursor ---------------------------------------------------------------

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

    /**
     * Free-running ambient clock (ms) fed to the AGSL cursor renderer for the
     * smoke aura's drift and breathing. Mirrors the desktop `frame.timing.x`; set
     * from the controller's ambient loop. The aura computes its own breathing
     * curve from this clock, so there is no separate "pulse" value.
     */
    @Volatile
    private var cursorElapsedMs: Float = 0f

    /**
     * Global smoke-aura alpha multiplier (desktop `frame.halo.w`); 1.0 = full.
     * The smoke aura is the cursor's "halo", so this fades it in/out without
     * touching the aura's internal breathing.
     */
    @Volatile
    private var cursorCloudAlpha: Float = 1f

    /**
     * AGSL cursor renderer (shadow + smoke aura + glyph) — the sole cursor draw
     * path. It becomes drawable once [prepare] has built its SDF texture off-thread;
     * until then [drawCursor] no-ops, so the cursor only appears once it is ready.
     */
    private val agslCursorRenderer = AgslCursorRenderer()

    // On-screen footprint (glyph + smoke margin) and its hotspot, in scale-1 px.
    // The glyph renders at CURSOR_HEIGHT_DP tall; the smoke footprint is the glyph
    // grown by the margin on every side (FOOTPRINT/DESKTOP ratio from the SDF
    // builder), so the footprint scales with the on-screen glyph size. The hotspot
    // (the point the cursor "points at") sits margin-in from the footprint corner.
    private val footprintPxY: Float =
        dp(CURSOR_HEIGHT_DP) *
            CursorSdfTexture.FOOTPRINT_HEIGHT / CursorSdfTexture.DESKTOP_HEIGHT
    private val footprintPxX: Float =
        footprintPxY *
            CursorSdfTexture.FOOTPRINT_WIDTH / CursorSdfTexture.FOOTPRINT_HEIGHT
    private val footprintHotspotPxX: Float =
        footprintPxX *
            CursorSdfTexture.FOOTPRINT_HOTSPOT_X / CursorSdfTexture.FOOTPRINT_WIDTH
    private val footprintHotspotPxY: Float =
        footprintPxY *
            CursorSdfTexture.FOOTPRINT_HOTSPOT_Y / CursorSdfTexture.FOOTPRINT_HEIGHT

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

    /**
     * Sets the free-running ambient clock (ms) the AGSL cursor renderer uses for
     * the smoke aura drift/breathing. Fed from the controller's ambient loop so the
     * aura animates every frame.
     */
    fun setCursorElapsedMs(elapsedMs: Float) {
        cursorElapsedMs = elapsedMs
        invalidate()
    }

    /** Sets the smoke-aura global alpha multiplier in [0, 1]; values are clamped. */
    fun setCursorCloudAlpha(alpha: Float) {
        cursorCloudAlpha = OverlayMath.clamp01(alpha)
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

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        // The edge-glow renderer has no texture to build, so prepare it inline on
        // the main thread (installs the palette / px-per-mm / breathe-period
        // uniforms). Idempotent, so a re-attach is harmless.
        agslEdgeGlowRenderer.prepare(context)
        // Build the AGSL cursor's SDF texture off the main thread (an O(n) chamfer
        // transform + distance scan; see CursorSdfTexture). Once prepared, request
        // a redraw so the GPU cursor appears. Guarded so a re-attach does not
        // rebuild an already-ready renderer.
        if (!agslCursorRenderer.isReady()) {
            Thread(
                {
                    agslCursorRenderer.prepare(
                        densityScaledFootprintPxX = footprintPxX,
                        densityScaledFootprintPxY = footprintPxY,
                        hotspotPxX = footprintHotspotPxX,
                        hotspotPxY = footprintHotspotPxY,
                    )
                    postInvalidate()
                },
                "agsl-cursor-prepare",
            ).start()
        }
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        // Inward waves retired: the desktop folded that motion into the fbm edge
        // glow (shaders.rs `inward_waves` is a no-op), so the discrete square-ish
        // ripple-in rings are gone here too.
        drawGlow(canvas)
        drawTrail(canvas)
        drawRipple(canvas)
        drawCursor(canvas)
    }

    /**
     * Draws the desktop's smoky fbm edge glow via [agslEdgeGlowRenderer]. The gate
     * is BINARY (in-control 1/0): the desktop folds
     * its own `breathing()` into the glow's base alpha, so feeding the controller's
     * breathing band here too would pulse it twice. [glowIntensity] is >0 exactly
     * while in control, so `> 0 -> 1` reproduces the desktop's `frame.flags.w` gate
     * and lets the shader own the breath. [cursorElapsedMs] is the same ambient
     * clock the cursor smoke drifts on, so the border and the cursor aura churn on
     * one clock.
     */
    private fun drawGlow(canvas: Canvas) {
        agslEdgeGlowRenderer.draw(
            canvas = canvas,
            widthPx = width.toFloat(),
            heightPx = height.toFloat(),
            elapsedMs = cursorElapsedMs,
            gate = if (glowIntensity > 0f) 1f else 0f,
        )
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

    /**
     * Draws the agent cursor via the AGSL renderer: the soft grounding shadow, the
     * glyph-anchored smoke aura, and the tilted plum/pink glyph — all in one
     * screen-space pass so the smoke fbm churns in screen space rather than spinning
     * with the glyph. Rotation, press scale, position, and the ambient clock ride in
     * as uniforms; the Canvas is NOT pre-rotated or pre-scaled.
     *
     * While the renderer is still building its SDF texture off-thread it no-ops, so
     * the cursor simply appears once ready.
     */
    private fun drawCursor(canvas: Canvas) {
        if (!cursorVisible) return
        agslCursorRenderer.draw(
            canvas = canvas,
            cursorX = cursorX,
            cursorY = cursorY,
            rotationDeg = cursorRotationDeg,
            scale = cursorScale,
            elapsedMs = cursorElapsedMs,
            cloudAlpha = cursorCloudAlpha,
        )
    }

    companion object {
        /**
         * All visual constants below are forwarded from the generated [OverlaySpec]
         * so Android, desktop, and tests share one source of truth. Public names
         * are preserved for existing callers.
         */

        // Pink agent palette — soft pastel character but a rich, saturated pink.
        val PINK: Int = android.graphics.Color.rgb(
            OverlaySpec.Shared.Colors.AGENT_PINK_RED_0_255,
            OverlaySpec.Shared.Colors.AGENT_PINK_GREEN_0_255,
            OverlaySpec.Shared.Colors.AGENT_PINK_BLUE_0_255,
        )
        val PINK_LIGHT: Int = android.graphics.Color.rgb(
            OverlaySpec.Shared.Colors.AGENT_PINK_LIGHT_RED_0_255,
            OverlaySpec.Shared.Colors.AGENT_PINK_LIGHT_GREEN_0_255,
            OverlaySpec.Shared.Colors.AGENT_PINK_LIGHT_BLUE_0_255,
        )

        // Cursor glyph height (dp). The AGSL cursor footprint (glyph + smoke
        // margin) scales from this; the SDF builder owns the glyph/shadow/aura
        // geometry, so only the overall size lives here.
        const val CURSOR_HEIGHT_DP: Float = OverlaySpec.Android.Geometry.CURSOR_HEIGHT_DP.toFloat()

        // Ripple geometry (dp): a wide, soft glow band (not a thin line).
        const val RIPPLE_STROKE_DP: Float = OverlaySpec.Android.Geometry.RIPPLE_STROKE_DP.toFloat()
        const val RIPPLE_BLUR_DP: Float = OverlaySpec.Android.Geometry.RIPPLE_BLUR_DP.toFloat()
        const val MAX_RIPPLE_ALPHA: Float = OverlaySpec.Android.Rendering.MAX_RIPPLE_ALPHA_0_255.toFloat()

        // Trail geometry (dp).
        const val TRAIL_STROKE_DP: Float = OverlaySpec.Android.Geometry.TRAIL_STROKE_DP.toFloat()
        const val MAX_TRAIL_ALPHA: Float = OverlaySpec.Android.Rendering.MAX_TRAIL_ALPHA_0_255.toFloat()
        const val TRAIL_MAX_POINTS: Int = OverlaySpec.Android.Rendering.TRAIL_MAX_POINTS
    }
}
