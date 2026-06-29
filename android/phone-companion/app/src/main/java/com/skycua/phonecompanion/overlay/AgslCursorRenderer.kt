package com.skycua.phonecompanion.overlay

import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.RuntimeShader
import androidx.annotation.MainThread
import androidx.annotation.WorkerThread
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.sin

/**
 * AGSL [RuntimeShader] port of the desktop WGSL agent-cursor renderer
 * (`crates/sky-cua-overlay-host/src/renderer/shaders.rs`). It draws, in one
 * full-footprint quad, the same three layers the desktop composites:
 *
 *  1. a soft grounding **shadow** sampled from the glyph distance field,
 *  2. the glyph-anchored **smoke aura** (domain-warped fbm in screen space), and
 *  3. the tilted plum/pink **glyph** reconstructed from the SDF.
 *
 * The edge glow, inward waves, ripple, trail, and no-no mark stay on the existing
 * Canvas path (`AgentOverlayView`); this renderer owns only the cursor itself.
 *
 * ## Why a screen-space shader (not a pre-rotated Canvas blit)
 *
 * The smoke fbm must be sampled in screen space so the cloud churns consistently
 * regardless of the cursor's rotation/scale (a Canvas-pre-rotated quad would spin
 * the noise field with the glyph and read as a rigid decal). The rotate/scale/uv
 * transform therefore happens inside [SHADER]: the host draws an axis-aligned
 * quad and the shader maps each fragment back into the cursor's local/uv frame.
 *
 * ## AGSL deviations from the WGSL source (verify these first on-device)
 *
 *  - **No derivatives.** WGSL reconstructs the glyph edge with `fwidth(d)` AA.
 *    AGSL has no `fwidth`/`dFdx`, so [draw] passes an **analytic** AA width
 *    ([computeAaWidth]) derived from the texture->screen texel ratio.
 *  - **No explicit-LOD sampling.** WGSL samples the shadow silhouette at mip LOD
 *    3 for a blurred blob. `RuntimeShader` shaders have no LOD control, so the
 *    shadow samples the base level and relies on `reach`/`falloff` to feather it;
 *    [SHADOW_LOD] rides in as a soft-falloff scalar (see [SHADER]).
 *  - **Pixel-space `eval`.** `uCursorTex.eval(p)` samples in TEXTURE PIXELS, not
 *    normalized `[0,1]`; the shader multiplies uv by the texture dimensions.
 *  - **Premultiplied output.** Skia runtime shaders output premultiplied alpha,
 *    so `premul`/`over` port directly.
 *
 * ## Threading
 *
 * [prepare] builds the [CursorSdfTexture] (an O(n) chamfer transform + distance
 * scan, a known startup hotspot) and must run OFF the main thread. The cheap
 * per-frame [draw] runs on the main thread inside `onDraw`. Call [prepare] once
 * (e.g. from a `Dispatchers.Default` coroutine) before the first [draw]; until it
 * completes [isReady] is false and [draw] is a no-op.
 */
class AgslCursorRenderer {
    private val runtimeShader = RuntimeShader(SHADER)
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { isDither = true }

    // Reused per-frame so draw() stays allocation-free.
    private val clipRect = RectF()

    @Volatile
    private var ready: Boolean = false

    /** On-screen footprint size in px at scale 1.0 (the un-scaled bounding box). */
    private var footprintPxX: Float = 0f
    private var footprintPxY: Float = 0f

    /** Hotspot within the footprint, in the same scale-1 px units. */
    private var hotspotPxX: Float = 0f
    private var hotspotPxY: Float = 0f

    /**
     * Half the diagonal of the footprint at scale 1.0, in px. The per-frame clip
     * box is `cursor +/- (this * scaleMax)` on each axis so the rotated+scaled
     * footprint always fits regardless of rotation.
     */
    private var footprintHalfDiagPx: Float = 0f

    /** True once [prepare] has installed the SDF texture and constant uniforms. */
    fun isReady(): Boolean = ready

    /**
     * Build the SDF/anchor texture, install it as the shader's input, and push the
     * constant (per-install) uniforms: colors, glyph/smoke/shadow params, texture
     * dims, and the footprint metrics used for the local->uv transform.
     *
     * MUST run off the main thread (builds the SDF texture; see class docs). After
     * it returns, hop to the main thread before the first [draw]; [isReady] flips
     * true here so a concurrent [draw] either no-ops or sees a fully prepared
     * shader (the volatile publishes the constant uniforms written before it).
     *
     * @param densityScaledFootprintPxX on-screen footprint WIDTH in px at scale 1.
     * @param densityScaledFootprintPxY on-screen footprint HEIGHT in px at scale 1.
     * @param hotspotPxX hotspot X within the footprint, scale-1 px.
     * @param hotspotPxY hotspot Y within the footprint, scale-1 px.
     */
    @WorkerThread
    fun prepare(
        densityScaledFootprintPxX: Float,
        densityScaledFootprintPxY: Float,
        hotspotPxX: Float,
        hotspotPxY: Float,
    ) {
        val sdfShader: BitmapShader = CursorSdfTexture.build()
        runtimeShader.setInputShader("uCursorTex", sdfShader)

        footprintPxX = densityScaledFootprintPxX
        footprintPxY = densityScaledFootprintPxY
        this.hotspotPxX = hotspotPxX
        this.hotspotPxY = hotspotPxY
        footprintHalfDiagPx =
            0.5f *
                kotlin.math.sqrt(
                    densityScaledFootprintPxX * densityScaledFootprintPxX +
                        densityScaledFootprintPxY * densityScaledFootprintPxY,
                )

        // --- texture dims (pixel-space eval) ---
        runtimeShader.setFloatUniform(
            "uTexSize",
            CursorSdfTexture.textureWidth.toFloat(),
            CursorSdfTexture.textureHeight.toFloat(),
        )

        // --- footprint + hotspot for the local->uv transform (scale-1 px) ---
        runtimeShader.setFloatUniform("uFootprintPx", footprintPxX, footprintPxY)
        runtimeShader.setFloatUniform("uHotspotPx", hotspotPxX, hotspotPxY)

        // --- palette (agent pink + pink-light), rgb + the breathing alpha band ---
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

        // --- glyph fill rgb (LINEAR) + white-edge mix (frame.cursor_glyph) ---
        // Fed in the desktop's native LINEAR space (0.022,0.006,0.038). The shader
        // composes fill->edge in linear and sRGB-encodes the result (matching the
        // desktop's sRGB swapchain), so the fill presents as the same deep plum AND
        // the white-mixed edge presents near-white instead of a saturated pink.
        runtimeShader.setFloatUniform(
            "uGlyph",
            OverlaySpec.Shared.Effects.GLYPH_FILL_RED_0_1.toFloat(),
            OverlaySpec.Shared.Effects.GLYPH_FILL_GREEN_0_1.toFloat(),
            OverlaySpec.Shared.Effects.GLYPH_FILL_BLUE_0_1.toFloat(),
            OverlaySpec.Shared.Effects.GLYPH_EDGE_WHITE_MIX_0_1.toFloat(),
        )

        // --- smoke params (frame.cursor_smoke): x = ring half-width / stroke
        //     edge, yz = smoke anchor uv offset ---
        runtimeShader.setFloatUniform(
            "uSmoke",
            OverlaySpec.Shared.Effects.CURSOR_STROKE_EDGE_0_1.toFloat(),
            OverlaySpec.Shared.Effects.CURSOR_SMOKE_OFFSET_X_UV.toFloat(),
            OverlaySpec.Shared.Effects.CURSOR_SMOKE_OFFSET_Y_UV.toFloat(),
        )

        // --- shadow params (frame.cursor_shadow): reach, falloff, strength, lod ---
        runtimeShader.setFloatUniform(
            "uShadow",
            OverlaySpec.Shared.Effects.CURSOR_SHADOW_REACH_0_1.toFloat(),
            OverlaySpec.Shared.Effects.CURSOR_SHADOW_FALLOFF_0_1.toFloat(),
            OverlaySpec.Shared.Effects.CURSOR_SHADOW_STRENGTH_0_1.toFloat(),
            OverlaySpec.Shared.Effects.CURSOR_SHADOW_LOD.toFloat(),
        )

        // --- smoke fbm cell size: footprint width (scale-1 px) * 0.16, matching
        //     the WGSL `cell = frame.cursor_metrics.x * 0.16`. Screen-space grain,
        //     scale-independent like the desktop (the cloud is screen-locked and
        //     the cursor churns through it). ---
        runtimeShader.setFloatUniform("uSmokeCellBase", max(footprintPxX * 0.16f, 1.0f))

        // --- breathe period (ms) the smoke pulse wraps on; WGSL `cursor_smoke`
        //     uses frame.timing.y = BREATHE_PERIOD_MS (NOT the halo period). ---
        runtimeShader.setFloatUniform(
            "uBreathePeriodMs",
            OverlaySpec.Shared.Timing.BREATHE_PERIOD_MS.toFloat(),
        )

        ready = true
    }

    /**
     * Draw the cursor for one frame. Allocation-free: sets the per-frame uniforms
     * and draws a single axis-aligned quad clipped to the footprint bounding box.
     * No-ops until [prepare] has completed.
     *
     * @param cursorX hotspot X in view px (where the cursor points).
     * @param cursorY hotspot Y in view px.
     * @param rotationDeg drawn glyph rotation (degrees), nose into heading.
     * @param scale drawn press scale (1.0 = rest; squashes below 1 on press).
     * @param elapsedMs free-running ambient clock (ms) for the smoke drift/breathe.
     * @param cloudAlpha global smoke-aura alpha multiplier (frame.halo.w); 0 hides.
     */
    @MainThread
    fun draw(
        canvas: Canvas,
        cursorX: Float,
        cursorY: Float,
        rotationDeg: Float,
        scale: Float,
        elapsedMs: Float,
        cloudAlpha: Float,
    ) {
        if (!ready) return
        val safeScale = max(scale, 1e-3f)

        runtimeShader.setFloatUniform("uCursorPos", cursorX, cursorY)
        runtimeShader.setFloatUniform("uRotationRad", Math.toRadians(rotationDeg.toDouble()).toFloat())
        runtimeShader.setFloatUniform("uScale", safeScale)
        runtimeShader.setFloatUniform("uTimeMs", elapsedMs)
        runtimeShader.setFloatUniform("uCloudAlpha", cloudAlpha)
        // Analytic AA replacement for fwidth(d): how far the SDF-normalized field
        // moves per screen pixel. See computeAaWidth.
        runtimeShader.setFloatUniform("uAaWidth", computeAaWidth(safeScale))

        paint.shader = runtimeShader

        // Square clip big enough to contain the rotated+scaled footprint: half the
        // footprint diagonal (scale 1) times the scale. Press scale only shrinks
        // (<= 1), but allow a small overscan margin so a bounce overshoot (>1)
        // never clips the smoke. SMOKE already lives inside the footprint, so the
        // footprint box bounds the whole effect.
        val half = footprintHalfDiagPx * safeScale * CLIP_OVERSCAN
        clipRect.set(cursorX - half, cursorY - half, cursorX + half, cursorY + half)

        canvas.save()
        canvas.clipRect(clipRect)
        canvas.drawRect(clipRect, paint)
        canvas.restore()
    }

    /**
     * Analytic AA width replacing WGSL `fwidth(d)`.
     *
     * `d` (the SDF channel minus 0.5) is in SDF-normalized units where one texel
     * of distance is `1 / SDF_RANGE_TEXELS`. The texture spans `textureWidth`
     * texels across one uv unit, which maps to `uFootprintPx.x * scale` screen px.
     * So texels-per-screen-pixel is `textureWidth / (footprintPx * scale)`, and:
     *
     *   aa = (1 / SDF_RANGE_TEXELS) * (textureWidth / (footprintPx * scale))
     *
     * which equals `fwidth(d)` for an axis-aligned, uniformly minified sample.
     * Anisotropy from rotation is ignored (a single isotropic AA width); the glyph
     * is small and this reads cleanly. Tunable on-device if the edge is too soft
     * or too crisp.
     */
    private fun computeAaWidth(scale: Float): Float {
        val footprintPx = max(footprintPxX, 1e-3f)
        val texelsPerScreenPx =
            CursorSdfTexture.textureWidth.toFloat() / (footprintPx * scale)
        return max(texelsPerScreenPx / CursorSdfTexture.SDF_RANGE_TEXELS, 1e-5f)
    }

    private fun norm(value: Int): Float = value / 255.0f

    companion object {
        /** Small overscan on the clip box so a bounce overshoot never clips. */
        private const val CLIP_OVERSCAN: Float = 1.08f

        /**
         * AGSL source. Mirrors the WGSL helpers and the shadow/smoke/glyph passes
         * from `shaders.rs` (cited per function). Only the cursor layers are
         * ported; edge glow / waves / trail / ripple / no-no stay on Canvas.
         *
         * Coordinate transform (matches WGSL `cursor_sample`/`cursor_smoke`):
         *   local   = (fragCoord - uCursorPos) / uScale
         *   rotated = rot(local, -uRotationRad)
         *   uv      = (rotated + uHotspotPx) / uFootprintPx
         *   texel   = uv * uTexSize     // AGSL eval is PIXEL-space
         */
        val SHADER: String =
            """
            uniform shader uCursorTex;

            // texture pixel dims (eval is pixel-space, not [0,1]).
            uniform float2 uTexSize;
            // footprint + hotspot for the local->uv transform (scale-1 px).
            uniform float2 uFootprintPx;
            uniform float2 uHotspotPx;
            // palette: rgb + breathing alpha band.
            uniform float4 uPink;
            uniform float4 uPinkLight;
            // glyph fill rgb + white-edge mix.
            uniform float4 uGlyph;
            // smoke: x = ring half-width / stroke edge, yz = anchor uv offset.
            uniform float3 uSmoke;
            // shadow: reach, falloff, strength, lod(soft scalar).
            uniform float4 uShadow;
            // smoke fbm cell base (footprint px * 0.16, scale-1).
            uniform float uSmokeCellBase;
            uniform float uBreathePeriodMs;

            // per-frame:
            uniform float2 uCursorPos;
            uniform float uRotationRad;
            uniform float uScale;
            uniform float uTimeMs;
            uniform float uCloudAlpha;
            uniform float uAaWidth;   // analytic fwidth(d) replacement.

            // ---- helpers (ports of shaders.rs) -----------------------------

            float saturatef(float v) { return clamp(v, 0.0, 1.0); }

            float safeDiv(float n, float d) { return n / max(d, 0.0001); }

            // Linear -> sRGB transfer (component-wise). The glyph is composed in the
            // desktop's working space and sRGB-encoded on output, reproducing its
            // sRGB swapchain so both the plum fill AND the near-white stroke match.
            // Without this the edge (mix of white + sRGB pink-light) stays a
            // saturated pink instead of the desktop's near-white outline.
            float linearToSrgb1(float c) {
                return (c <= 0.0031308) ? (12.92 * c) : (1.055 * pow(c, 1.0 / 2.4) - 0.055);
            }
            float3 linearToSrgb3(float3 c) {
                return float3(linearToSrgb1(c.r), linearToSrgb1(c.g), linearToSrgb1(c.b));
            }

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

            // over (shaders.rs:182).
            half4 over(half4 src, half4 dst) {
                return src + dst * (1.0 - src.a);
            }

            // Map a fragCoord into the cursor uv frame (rotate/scale/translate).
            float2 cursorUv(float2 frag) {
                float2 local = (frag - uCursorPos) / uScale;
                float angle = -uRotationRad;
                float s = sin(angle);
                float c = cos(angle);
                float2 rotated = float2(local.x * c - local.y * s, local.x * s + local.y * c);
                return (rotated + uHotspotPx) / uFootprintPx;
            }

            // Sample the SDF/anchor texture at a uv in [0,1] (eval is pixel-space).
            half4 sampleTex(float2 uv) {
                return uCursorTex.eval(uv * uTexSize);
            }

            // cursor_shadow (shaders.rs:505): soft grounding silhouette, centered,
            // reach/falloff/strength, black. WGSL samples mip LOD 3 for a blurred
            // blob; AGSL has no LOD, so we sample base level and let reach/falloff
            // feather it. uShadow.w (LOD) is carried but unused as a mip; treat it
            // as a soft-falloff hint to verify on-device.
            half4 cursorShadow(float2 frag) {
                float2 uv = cursorUv(frag);
                if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
                    return half4(0.0);
                }
                float ds = sampleTex(uv).r - 0.5;
                float cov = clamp((uShadow.x - ds) / uShadow.y, 0.0, 1.0);
                return premul(float3(0.0), cov * uShadow.z);
            }

            // cursor_smoke (shaders.rs:309): glyph-anchored aura. Domain-warped fbm
            // in screen space; density/rim/body/edge_fade/breathe; pink->pink_light.
            half4 cursorSmoke(float2 frag) {
                float2 uv = cursorUv(frag);
                float2 smokeUv = uv + float2(uSmoke.y, uSmoke.z);
                // Bounds-check before sampling. (The WGSL twin sampled first to keep
                // textureSample in uniform control flow for derivatives; AGSL eval
                // has no such requirement, so the early-out avoids a wasted read.)
                if (smokeUv.x < 0.0 || smokeUv.y < 0.0 || smokeUv.x > 1.0 || smokeUv.y > 1.0) {
                    return half4(0.0);
                }
                float border = sampleTex(smokeUv).g; // 1 on outline -> 0 at reach.
                if (border <= 0.004) {
                    return half4(0.0);
                }
                // Mask off the arrow using the UNSHIFTED glyph coverage. Coverage
                // is in B (not A): the texture's A is forced opaque so Skia's
                // premultiplied bitmap sampling can't corrupt the R/G data fields.
                float glyphA = sampleTex(uv).b;

                // Drifting domain-warped fbm in screen space. Grain scales with the
                // on-screen footprint: cell = base * uScale (base = footprintPx*0.16).
                float t = uTimeMs * 0.00045;
                // Screen-space grain, scale-independent like the desktop
                // (cell = cursor_metrics.x * 0.16); the cloud is screen-locked so
                // the cursor churns through it.
                float cell = max(uSmokeCellBase, 1.0);
                float smoke = smokeFbm(frag / cell, t);
                // --- desktop cursor_smoke (shaders.rs:352), density-band-limited ---
                // The desktop gates the cloud purely on the raw noise `density`,
                // which makes a small cursor swing between two bad extremes: a fast
                // glide / tap-arrival sweeps it through HIGH-noise patches and it
                // blooms into a bright smooth disc, while a parked cursor over a
                // LOW-noise patch collapses to the bare rim. Compress the density
                // into a controlled band [0.30, 0.82] so it stays CONSISTENT — a
                // feathered, moderate cloud whether moving or parked — never bright
                // disc, never collapsed. It still varies with the noise within the
                // band, so the billowing/feathering reads; it just can't hit either
                // extreme.
                float density = mix(0.30, 0.82, smoothstep(mix(0.42, 0.16, border), 0.82, smoke));
                // d_norm: normalized outward distance, 0 on the outline, 1 at reach.
                float dNorm = 1.0 - border;
                // Bright rim hugging the very outline, lightly shimmered by smoke.
                float rim = exp(-dNorm / 0.05) * mix(0.82, 1.0, density);
                // Smoky body: tendrils lick outward where the noise is dense. Gated
                // on the band-limited density, so the [0.30, 0.82] floor/ceiling
                // carries through — a soft cloud always surrounds the glyph and a
                // high-noise patch can't bloom it.
                float dEff = max(dNorm - density * 0.5, 0.0);
                float body = exp(-dEff / 0.18) * density * (0.5 + border);
                // Keep the smoke off the solid arrow body (B holds glyph coverage).
                float outside = smoothstep(0.35, 0.8, 1.0 - glyphA);
                // Feather the OUTER boundary of the cloud to nothing.
                float edgeFade = smoothstep(0.0, 0.34, border);

                float breathe = clamp((breathing(uTimeMs, uBreathePeriodMs) - 0.5) * 1.5 + 0.5, 0.0, 1.0);
                float baseAlpha = mix(uPink.a, uPinkLight.a, breathe);
                // Warm pink -> pink_light tint, exactly as the desktop. These values
                // are composited in LINEAR and sRGB-encoded once in main(), so the
                // low-alpha cloud reads as a LUMINOUS warm haze that the encode lifts
                // and softens — NOT the flat magenta an un-encoded sRGB-space
                // composite produced (the earlier "too saturated" was that bug).
                float3 tint = mix(uPink.rgb, uPinkLight.rgb, max(density, rim * 0.6));
                float alpha = baseAlpha * (rim * 0.72 + body * 0.48) * outside * edgeFade * uCloudAlpha;
                return premul(tint, alpha);
            }

            // cursor_sample (shaders.rs:464): SDF glyph reconstruction. Analytic AA
            // (uAaWidth) replaces fwidth(d). Fill = uGlyph.rgb; edge =
            // mix(white, pink_light, uGlyph.w); white ring half-width = uSmoke.x.
            half4 cursorGlyph(float2 frag) {
                float2 uv = cursorUv(frag);
                if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
                    return half4(0.0);
                }
                float d = sampleTex(uv).r - 0.5;
                float aa = max(uAaWidth, 1e-5);
                float alpha = clamp(0.5 - (d - uSmoke.x) / aa, 0.0, 1.0);
                if (alpha <= 0.0) {
                    return half4(0.0);
                }
                float lum = clamp(0.5 + d / aa, 0.0, 1.0);
                // Compose exactly like the desktop WGSL: mix the authored fill (deep
                // plum) and the raw sRGB pink-light numbers as-is (the desktop never
                // linearizes the palette), and return the result in the shared
                // pre-encode space. main() sRGB-encodes the whole composited cursor
                // once, matching the desktop's swapchain — that lands the fill plum
                // and the stroke near-white.
                float3 fillLin = uGlyph.rgb;
                float3 edgeLin = mix(float3(1.0), uPinkLight.rgb, uGlyph.w);
                float3 colorLin = mix(fillLin, edgeLin, lum);
                return premul(colorLin, alpha);
            }

            // render order (shaders.rs:524, cursor layers only):
            // shadow -> smoke -> glyph. The three layers are composited in LINEAR
            // (premultiplied), like the desktop's pre-swapchain frame, then the
            // straight color is sRGB-encoded ONCE here. Doing the alpha compositing
            // in linear before encoding is what gives the translucent smoke its
            // luminous, billowing read (an un-encoded sRGB-space composite of the
            // same pink flattens to dull magenta).
            half4 main(float2 fragCoord) {
                half4 color = half4(0.0);
                color = over(cursorShadow(fragCoord), color);
                color = over(cursorSmoke(fragCoord), color);
                color = over(cursorGlyph(fragCoord), color);
                float a = float(color.a);
                if (a <= 0.0) {
                    return half4(0.0);
                }
                float3 straight = float3(color.rgb) / a;
                float3 enc = linearToSrgb3(straight);
                return half4(half3(enc * a), half(a));
            }
            """.trimIndent()
    }
}
