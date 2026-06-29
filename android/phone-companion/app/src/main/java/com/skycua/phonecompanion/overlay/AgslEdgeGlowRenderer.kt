package com.skycua.phonecompanion.overlay

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.RuntimeShader
import androidx.annotation.MainThread
import kotlin.math.min

/**
 * AGSL [RuntimeShader] port of the desktop WGSL `edge_glow` pass
 * (`crates/sky-cua-overlay-host/src/renderer/shaders.rs:236`). It replaces the
 * phone overlay's old plain pink rounded-rect border (blurred stroke) with the
 * desktop's **smoky fbm edge glow**: a drifting, domain-warped turbulent field
 * that crawls around the screen edge like a cast spell, over a breathing pink
 * base, fading from a bright rim at the very edge to thin haze further inward.
 *
 * This renderer owns ONLY the edge glow. The inward waves, tap ripple, swipe
 * trail, and the agent cursor (its own AGSL renderer) stay where they are.
 *
 * ## Faithful port of the desktop math
 *
 * The shader mirrors `edge_glow` line-for-line (cited inline). The only
 * coordinate change is the edge-distance source: WGSL uses
 * `frame.surface_size_px`; here `uResolution` carries the view-space screen
 * size, so `edgeDistance(frag) = min(min(frag.x, W-frag.x), min(frag.y, H-frag.y))`.
 *
 * Real-world sizing matches the desktop: `px_per_mm` is the panel's logical px
 * per physical mm (`displayMetrics.xdpi / 25.4`), so the 25mm band depth and the
 * 0.8mm rim read at the same physical size as the desktop overlay.
 *
 * ## The in-control / breathing gate
 *
 * The desktop gates the glow on `frame.flags.w` (in-control) and folds its own
 * `breathing()` into `base_alpha`. On Android the controller already drives a
 * breathing **intensity** in [0.55, 0.92] (the same band as `base_alpha`) and
 * hands it to [draw] as `gate`. The shader multiplies its final alpha by
 * `uGlowGate`, so when the controller sets intensity 0 (not in control) the glow
 * vanishes, and otherwise it tracks the controller's breathing. The shader's own
 * internal `breathing()` still runs (faithful to the desktop); see the report's
 * "verify on-device" note about the resulting compound breathing.
 *
 * ## Perf: draw only the edge band
 *
 * `edge_glow` returns 0 for any pixel deeper than `reach` (and `contain` ramps it
 * to 0 well before that), so the entire screen interior is wasted shader work. We
 * clip the draw to a `reach`-deep frame (an EVEN_ODD outer-minus-inner rect path)
 * and draw a single rect through it. The frame [Path] is rebuilt only when the
 * screen size changes; per-frame [draw] is allocation-free.
 *
 * minSdk is 33, so [RuntimeShader] is available unconditionally.
 */
class AgslEdgeGlowRenderer {
    private val runtimeShader = RuntimeShader(SHADER)
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { isDither = true }

    // Reused per-frame so draw() stays allocation-free. The frame Path is rebuilt
    // only when the screen size changes (tracked by framePathWidth/Height).
    private val framePath = Path()
    private val outerRect = RectF()
    private val innerRect = RectF()
    private var framePathWidth: Float = -1f
    private var framePathHeight: Float = -1f

    /** Logical px per physical mm; set by [prepare] from the display metrics. */
    private var pxPerMm: Float = 1f

    private var prepared: Boolean = false

    /**
     * Install the constant (per-install) uniforms: the pink palette, the physical
     * px-per-mm scale, and the breathing period. Cheap (no texture), so it can run
     * on the main thread from `init`/`onAttachedToWindow`.
     */
    @MainThread
    fun prepare(context: Context) {
        // Logical px per physical mm, matching the desktop `surface_size_px.z`.
        // xdpi is dots per inch along X; / 25.4 converts inches -> mm. Floored so a
        // bogus/zero xdpi can never collapse the band to nothing.
        pxPerMm = max(context.resources.displayMetrics.xdpi / 25.4f, 0.5f)
        runtimeShader.setFloatUniform("uPxPerMm", pxPerMm)

        // Palette: agent pink + pink-light, rgb + the breathing alpha band. The
        // alphas are GLOW_BASELINE_MIN/MAX, exactly as the desktop populates
        // color_agent_pink.a / color_agent_pink_light.a (buffers.rs:183).
        runtimeShader.setFloatUniform(
            "uPink",
            norm(OverlaySpec.Shared.Colors.AGENT_PINK_RED_0_255),
            norm(OverlaySpec.Shared.Colors.AGENT_PINK_GREEN_0_255),
            norm(OverlaySpec.Shared.Colors.AGENT_PINK_BLUE_0_255),
            OverlaySpec.Shared.Effects.GLOW_BASELINE_MIN_ALPHA_0_1.toFloat(),
        )
        runtimeShader.setFloatUniform(
            "uPinkLight",
            norm(OverlaySpec.Shared.Colors.AGENT_PINK_LIGHT_RED_0_255),
            norm(OverlaySpec.Shared.Colors.AGENT_PINK_LIGHT_GREEN_0_255),
            norm(OverlaySpec.Shared.Colors.AGENT_PINK_LIGHT_BLUE_0_255),
            OverlaySpec.Shared.Effects.GLOW_BASELINE_MAX_ALPHA_0_1.toFloat(),
        )

        // Breathe period (ms) the internal breathing() wraps on; matches the
        // desktop edge glow's frame.timing.y = BREATHE_PERIOD_MS.
        runtimeShader.setFloatUniform(
            "uBreathePeriodMs",
            OverlaySpec.Shared.Timing.BREATHE_PERIOD_MS.toFloat(),
        )

        prepared = true
    }

    /**
     * Draw the edge glow for one frame. Allocation-free: sets the per-frame
     * uniforms and draws a single rect clipped to the `reach`-deep edge frame.
     * No-ops until [prepare] has run or when [gate] is non-positive.
     *
     * @param canvas the overlay canvas.
     * @param widthPx view width in px.
     * @param heightPx view height in px.
     * @param elapsedMs free-running ambient clock (ms) for the smoke drift / breathe.
     * @param gate breathing intensity in [0, 1] (controller's 0.55-0.92 band, 0 =
     *   not in control); multiplies the final alpha so the glow breathes/gates.
     */
    @MainThread
    fun draw(
        canvas: Canvas,
        widthPx: Float,
        heightPx: Float,
        elapsedMs: Float,
        gate: Float,
    ) {
        if (!prepared) return
        if (gate <= 0f) return
        if (widthPx <= 0f || heightPx <= 0f) return

        // Band depth, identical to the shader's `reach` (and to the desktop):
        // 25mm physical, capped to 8% of the smaller screen dimension.
        // Larger, softer band than the desktop default so the smoke reads big and
        // billowy on the phone (must match the shader's `reach` for the clip frame).
        val reach = min(42f * pxPerMm, 0.16f * min(widthPx, heightPx))

        runtimeShader.setFloatUniform("uResolution", widthPx, heightPx)
        runtimeShader.setFloatUniform("uTimeMs", elapsedMs)
        runtimeShader.setFloatUniform("uGlowGate", gate.coerceIn(0f, 1f))

        paint.shader = runtimeShader

        rebuildFramePathIfNeeded(widthPx, heightPx, reach)

        canvas.save()
        canvas.clipPath(framePath)
        canvas.drawRect(0f, 0f, widthPx, heightPx, paint)
        canvas.restore()
    }

    /**
     * Rebuild the EVEN_ODD outer-minus-inner frame [Path] only when the screen size
     * changes. The interior is `contain`ed to 0 by the shader anyway, so clipping
     * to the `reach`-deep frame is a pure perf win with no visual change. If `reach`
     * is large enough that the inner rect would invert (tiny screens), we skip the
     * inset and clip to the whole screen.
     */
    private fun rebuildFramePathIfNeeded(widthPx: Float, heightPx: Float, reach: Float) {
        if (widthPx == framePathWidth && heightPx == framePathHeight) return
        framePathWidth = widthPx
        framePathHeight = heightPx

        framePath.rewind()
        framePath.fillType = Path.FillType.EVEN_ODD
        outerRect.set(0f, 0f, widthPx, heightPx)
        framePath.addRect(outerRect, Path.Direction.CW)
        // Inner cutout only if it stays a valid (non-inverted) rect.
        if (reach * 2f < widthPx && reach * 2f < heightPx) {
            innerRect.set(reach, reach, widthPx - reach, heightPx - reach)
            framePath.addRect(innerRect, Path.Direction.CW)
        }
    }

    private fun norm(value: Int): Float = value / 255.0f

    private fun max(a: Float, b: Float): Float = if (a >= b) a else b

    companion object {
        /**
         * AGSL source. A faithful port of the desktop `edge_glow` pass and its fbm
         * helper stack (`shaders.rs`, cited per function). Helpers (hash21 /
         * value_noise / fbm / smoke_fbm / breathing / premul / saturate / safeDiv /
         * easeInOut) are byte-identical to the proven block in
         * [AgslCursorRenderer.SHADER]; only `edgeGlow` / `edgeDistance` / `main` are
         * new here.
         *
         * Edge distance (matches WGSL `edge_distance` with the screen size carried
         * in `uResolution` instead of `frame.surface_size_px`):
         *   edgeDistance(frag) = min(min(frag.x, W-frag.x), min(frag.y, H-frag.y))
         */
        val SHADER: String =
            """
            // palette: rgb + breathing alpha band (a = GLOW_BASELINE_MIN/MAX).
            uniform float4 uPink;
            uniform float4 uPinkLight;
            // logical px per physical mm (matches desktop surface_size_px.z).
            uniform float uPxPerMm;
            uniform float uBreathePeriodMs;

            // per-frame:
            uniform float2 uResolution;  // view size (w, h) px.
            uniform float uTimeMs;       // ambient clock; frame.timing.x.
            uniform float uGlowGate;     // breathing intensity 0..1; 0 hides.

            // ---- helpers (ports of shaders.rs) -----------------------------

            float saturatef(float v) { return clamp(v, 0.0, 1.0); }

            float safeDiv(float n, float d) { return n / max(d, 0.0001); }

            float easeInOut(float v) {
                float t = saturatef(v);
                return t * t * (3.0 - 2.0 * t);
            }

            // breathing (shaders.rs:84): eased triangle 0->1->0 across the period.
            float breathing(float elapsedMs, float periodMs) {
                float phase = fract(safeDiv(elapsedMs, periodMs));
                float triangle = (phase < 0.5) ? phase * 2.0 : (1.0 - phase) * 2.0;
                return easeInOut(triangle);
            }

            // hash21 (shaders.rs:196).
            float hash21(float2 p) {
                float3 p3 = fract(float3(p.x, p.y, p.x) * 0.1031);
                p3 += dot(p3, float3(p3.y, p3.z, p3.x) + 33.33);
                return fract((p3.x + p3.y) * p3.z);
            }

            // value_noise (shaders.rs:202): smootherstep interpolation.
            float valueNoise(float2 p) {
                float2 i = floor(p);
                float2 f = fract(p);
                float2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
                float a = hash21(i);
                float b = hash21(i + float2(1.0, 0.0));
                float c = hash21(i + float2(0.0, 1.0));
                float d = hash21(i + float2(1.0, 1.0));
                return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
            }

            // fbm (shaders.rs:214): 4 octaves, amplitude 0.5.
            float fbm(float2 p) {
                float value = 0.0;
                float amplitude = 0.5;
                float2 freq = p;
                for (int i = 0; i < 4; i++) {
                    value += amplitude * valueNoise(freq);
                    freq *= 2.0;
                    amplitude *= 0.5;
                }
                return value;
            }

            // smoke_fbm (shaders.rs:230): domain-warped, drift (t, -0.55t).
            float smokeFbm(float2 q, float t) {
                float2 drift = float2(t, -t * 0.55);
                float2 warp = float2(fbm(q + drift + 19.3), fbm(q - drift * 0.8 + 4.1));
                return fbm(q + warp * 1.25 + drift * 0.5);
            }

            // premul (shaders.rs:177).
            half4 premul(float3 color, float alpha) {
                float a = saturatef(alpha);
                return half4(half3(color * a), half(a));
            }

            // edge_distance (shaders.rs:186): min distance to any screen edge.
            float edgeDistance(float2 pixel) {
                return min(
                    min(pixel.x, uResolution.x - pixel.x),
                    min(pixel.y, uResolution.y - pixel.y)
                );
            }

            // edge_glow (shaders.rs:236): the full smoky border. Faithful port; the
            // in-control gate (frame.flags.w) is supplied per-frame as uGlowGate,
            // which also carries the controller's breathing intensity and multiplies
            // the final alpha.
            half4 edgeGlow(float2 pixel) {
                float distance = edgeDistance(pixel);
                float pxPerMm = max(uPxPerMm, 0.5);
                float minDim = min(uResolution.x, uResolution.y);
                // Larger, softer band than the desktop (25mm / 0.08) so the smoke
                // reads big and billowy on the small phone. MUST match the Kotlin
                // `reach` that builds the clip frame.
                float reach = min(42.0 * pxPerMm, 0.16 * minDim);
                if (distance > reach) {
                    return half4(0.0); // fully contained to the border.
                }

                float t = uTimeMs * 0.00017;
                float cell = max(reach * 0.55, 1.0);
                float2 q = pixel / cell;
                float smoke = smokeFbm(q, t);
                float border = exp(-distance / max(reach * 0.50, 1.0));
                float density = smoothstep(mix(0.34, 0.08, border), 0.88, smoke);

                float rim = exp(-distance / (0.8 * pxPerMm)) * mix(0.82, 1.0, density);

                // Slower body falloff so the tendrils lick deeper inward (bigger,
                // softer cloud) instead of hugging the very edge.
                float dEff = max(distance - density * reach * 0.5, 0.0);
                float body = exp(-dEff / max(reach * 0.30, 1.0)) * density * (1.0 + 1.3 * border);
                float contain = 1.0 - smoothstep(reach * 0.58, reach * 0.94, distance);

                float breathe = breathing(uTimeMs, uBreathePeriodMs);
                float baseAlpha = mix(uPink.a, uPinkLight.a, breathe);
                // Desaturate slightly toward a dusty grey-mauve so the border reads
                // like the desktop's soft smoke, not a saturated pink band.
                float3 pinkTint = mix(uPink.rgb, uPinkLight.rgb, max(density, rim * 0.6));
                float3 tint = mix(pinkTint, float3(0.60, 0.55, 0.59), 0.22);
                // Slightly more translucent than the desktop so it reads as soft
                // haze, not a saturated band.
                float alpha = baseAlpha * (rim * 0.72 + body * 0.50) * contain;
                // Controller breathing-intensity / in-control gate (see class docs).
                alpha *= uGlowGate;
                return premul(tint, alpha);
            }

            half4 main(float2 fragCoord) {
                return edgeGlow(fragCoord);
            }
            """.trimIndent()
    }
}
