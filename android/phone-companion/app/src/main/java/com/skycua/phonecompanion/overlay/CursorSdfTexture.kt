package com.skycua.phonecompanion.overlay

import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.Shader
import kotlin.math.ceil
import kotlin.math.sqrt

/**
 * Kotlin port of the desktop CPU cursor-texture builder
 * (`crates/sky-cua-overlay-host/src/renderer/cursor_texture.rs`). It synthesizes
 * the agent-pointer texture the upcoming Android AGSL shader samples: a signed
 * distance field of the glyph path (R channel) plus a chamfer-transform smoke
 * anchor (G channel), exactly mirroring what the desktop WGSL shader reads.
 *
 * ## Array layer vs. Bitmap layer
 *
 * The math lives entirely in [buildBuffer], a pure function that operates on
 * plain primitive arrays (it returns a [Buffer] wrapping an `IntArray` of packed
 * ARGB texels, the analogue of the Rust `Vec<u8>`). Nothing in [buildBuffer]
 * touches the Android framework, so it is fully unit-testable on a plain JVM
 * under the module's `unitTests.isReturnDefaultValues = true` (no Robolectric, no
 * real `Bitmap`). The thin [Buffer.toBitmap] / [Buffer.toShader] wrappers are the
 * ONLY part that touches `android.graphics`, so they are kept out of the testable
 * core.
 *
 * ## Channel encoding (matches the desktop, see `cursor_texture.rs`)
 *
 * - **R** = signed distance field. `0.5` lies on the path; the `[0,1]` channel
 *   encodes signed distance `[-SDF_RANGE_TEXELS/2, +SDF_RANGE_TEXELS/2]` texels
 *   (negative inside the fill). The shader reconstructs the black fill + white
 *   outline from this with `fwidth`-based anti-aliasing at the final resolution.
 * - **G** = chamfer smoke anchor. `1.0` on/inside the glyph silhouette, ramping
 *   to `0.0` over the smoke margin band. The shader billows border-style smoke
 *   off the glyph silhouette from this field.
 * - **B** = stepped luminance (CPU-blit fallback gray); `1.0` outside the path,
 *   `0.0` inside, matching the desktop's `glyph_lum`.
 * - **A** = coverage (CPU-blit straight alpha): the fill plus the outline ring.
 *
 * ## Sampled as DATA, not color (#1 fidelity trap)
 *
 * The bitmap carries signed-distance and anchor *values*, not premultiplied
 * colors. [Buffer.toBitmap] therefore builds an un-premultiplied,
 * color-unmanaged bitmap (`setPremultiplied(false)`, sRGB / null color space) so
 * Skia neither premultiplies the channels against alpha nor color-manages them on
 * upload. Premultiplication or a wide-gamut transform would silently corrupt the
 * R/G fields the shader depends on.
 *
 * ## Threading
 *
 * The full texture (footprint of [textureWidth] x [textureHeight] at the 4x
 * supersample) runs an O(n) two-pass chamfer transform and a bbox-restricted
 * per-pixel distance scan — a known desktop startup hotspot. Build it ONCE, off
 * the main thread; callers should invoke [buildBuffer] / [build] from a worker
 * (e.g. a coroutine on `Dispatchers.Default`) and only hop back to the main
 * thread to install the resulting shader.
 */
object CursorSdfTexture {
    // --- footprint / scale / margin constants -------------------------------
    //
    // Mirrored from the Rust `cursor_asset` module in
    // `crates/sky-cua-overlay-host/src/lib.rs` (cited per-constant). The desktop
    // glyph stays AGENT_CURSOR_DESKTOP_*; only the sampled footprint and its
    // hotspot grow by the smoke margin.

    /** `AGENT_CURSOR_SOURCE_WIDTH` (lib.rs cursor_asset): the 46x48 source space. */
    const val SOURCE_WIDTH: Int = 46

    /** `AGENT_CURSOR_SOURCE_HEIGHT` (lib.rs cursor_asset): the 46x48 source space. */
    const val SOURCE_HEIGHT: Int = 48

    /** `AGENT_CURSOR_DESKTOP_WIDTH` (lib.rs cursor_asset): on-screen glyph width. */
    const val DESKTOP_WIDTH: Int = 30

    /** `AGENT_CURSOR_DESKTOP_HEIGHT` (lib.rs cursor_asset): on-screen glyph height. */
    const val DESKTOP_HEIGHT: Int = 31

    /** `AGENT_CURSOR_DESKTOP_HOTSPOT_X` (lib.rs cursor_asset). */
    const val DESKTOP_HOTSPOT_X: Int = 13

    /** `AGENT_CURSOR_DESKTOP_HOTSPOT_Y` (lib.rs cursor_asset). */
    const val DESKTOP_HOTSPOT_Y: Int = 14

    /** `AGENT_CURSOR_SMOKE_MARGIN` (lib.rs cursor_asset): smoke band on every side. */
    const val SMOKE_MARGIN: Int = 30

    /** `AGENT_CURSOR_FOOTPRINT_WIDTH` (lib.rs cursor_asset): glyph + 2*margin. */
    const val FOOTPRINT_WIDTH: Int = DESKTOP_WIDTH + 2 * SMOKE_MARGIN

    /** `AGENT_CURSOR_FOOTPRINT_HEIGHT` (lib.rs cursor_asset): glyph + 2*margin. */
    const val FOOTPRINT_HEIGHT: Int = DESKTOP_HEIGHT + 2 * SMOKE_MARGIN

    /** `AGENT_CURSOR_FOOTPRINT_HOTSPOT_X` (lib.rs cursor_asset): glyph hotspot + margin. */
    const val FOOTPRINT_HOTSPOT_X: Int = DESKTOP_HOTSPOT_X + SMOKE_MARGIN

    /** `AGENT_CURSOR_FOOTPRINT_HOTSPOT_Y` (lib.rs cursor_asset): glyph hotspot + margin. */
    const val FOOTPRINT_HOTSPOT_Y: Int = DESKTOP_HOTSPOT_Y + SMOKE_MARGIN

    /**
     * Supersample factor (`CURSOR_TEXTURE_SCALE` in cursor_texture.rs). The vector
     * cursor is rasterized at this multiple of its on-screen footprint so the GPU
     * minifies a high-resolution texture instead of a blocky blit.
     */
    const val TEXTURE_SCALE: Int = 4

    /**
     * Chaikin corner-rounding iterations applied to the flattened glyph path
     * before it is turned into a distance field (`CURSOR_CORNER_ROUNDING` in
     * cursor_texture.rs). Higher = rounder, softer corners.
     */
    const val CORNER_ROUNDING: Int = 2

    /**
     * Texel span of the glyph SDF packed into R (`SDF_RANGE_TEXELS` in
     * cursor_texture.rs): the `[0,1]` channel encodes `[-R/2, +R/2]` texels.
     * Scales with the texture scale.
     */
    const val SDF_RANGE_TEXELS: Float = 12.0f * TEXTURE_SCALE

    /**
     * White-outline ring half-width as a fraction of the SDF's normalized range,
     * read from the shared generated spec (`cursor_stroke_edge_0_1`) — the same
     * key the WGSL shader reads into `frame.cursor_smoke.x`. Read from
     * [OverlaySpec] rather than re-mirrored so the outline and the smoke seed
     * cannot drift apart. (Desktop reads it from `overlay_spec::shared::effects`.)
     */
    val STROKE_EDGE: Float = OverlaySpec.Shared.Effects.CURSOR_STROKE_EDGE_0_1.toFloat()

    /** Full supersampled texture width: footprint width x [TEXTURE_SCALE]. */
    const val textureWidth: Int = FOOTPRINT_WIDTH * TEXTURE_SCALE

    /** Full supersampled texture height: footprint height x [TEXTURE_SCALE]. */
    const val textureHeight: Int = FOOTPRINT_HEIGHT * TEXTURE_SCALE

    private data class Pt(val x: Float, val y: Float)

    /** A path command over the 46x48 source space (matches Rust `PathCommand`). */
    private sealed interface Cmd {
        data class Move(val p: Pt) : Cmd
        data class Line(val p: Pt) : Cmd
        data class Quad(val control: Pt, val end: Pt) : Cmd
    }

    /**
     * The 14 Move/Line/Quad commands over the 46x48 source space, identical to the
     * Rust `CURSOR_PATH` and the `agent_cursor.xml` `pathData`. Kept here so the
     * SDF builder, the desktop renderer, and the VectorDrawable cannot drift; a
     * guard test parses `agent_cursor.xml` and compares command-by-command.
     */
    private val CURSOR_PATH: List<Cmd> =
        listOf(
            Cmd.Move(Pt(10.0f, 11.0f)),
            Cmd.Quad(Pt(10.5f, 9.5f), Pt(11.99f, 10.03f)),
            Cmd.Line(Pt(37.01f, 18.97f)),
            Cmd.Quad(Pt(38.5f, 19.5f), Pt(38.0f, 21.0f)),
            Cmd.Line(Pt(38.0f, 21.0f)),
            Cmd.Quad(Pt(37.5f, 22.5f), Pt(36.0f, 23.0f)),
            Cmd.Line(Pt(29.77f, 25.08f)),
            Cmd.Quad(Pt(25.5f, 26.5f), Pt(24.08f, 30.77f)),
            Cmd.Line(Pt(22.29f, 36.13f)),
            Cmd.Quad(Pt(21.5f, 38.5f), Pt(19.5f, 37.0f)),
            Cmd.Line(Pt(19.5f, 37.0f)),
            Cmd.Quad(Pt(17.5f, 35.5f), Pt(16.68f, 33.14f)),
            Cmd.Line(Pt(10.02f, 13.99f)),
            Cmd.Quad(Pt(9.5f, 12.5f), Pt(10.0f, 11.0f)),
        )

    /** Anchor vertices for the guard test: the Move/Line/Quad-end points, in order. */
    val pathKeyVertices: List<Pair<Float, Float>> =
        CURSOR_PATH.map { cmd ->
            when (cmd) {
                is Cmd.Move -> cmd.p.x to cmd.p.y
                is Cmd.Line -> cmd.p.x to cmd.p.y
                is Cmd.Quad -> cmd.end.x to cmd.end.y
            }
        }

    /** Control points for each Quad command, in order, for the guard test. */
    val pathQuadControls: List<Pair<Float, Float>> =
        CURSOR_PATH.mapNotNull { cmd ->
            (cmd as? Cmd.Quad)?.let { it.control.x to it.control.y }
        }

    /**
     * A built RGBA buffer (array layer). [pixels] is row-major packed ARGB int per
     * texel, the same layout `Bitmap.createBitmap(IntArray, ...)` consumes. This
     * is the pure, JVM-unit-testable product; the Android-framework wrappers below
     * are the only code that needs a real device.
     */
    class Buffer internal constructor(
        val width: Int,
        val height: Int,
        val pixels: IntArray,
    ) {
        /** Unpacked R channel `[0,1]` at (x, y) — the signed distance field. */
        fun r(x: Int, y: Int): Float = ((pixels[y * width + x] ushr 16) and 0xFF) / 255.0f

        /** Unpacked G channel `[0,1]` at (x, y) — the chamfer smoke anchor. */
        fun g(x: Int, y: Int): Float = ((pixels[y * width + x] ushr 8) and 0xFF) / 255.0f

        /** Unpacked B channel `[0,1]` at (x, y) — stepped luminance (fallback). */
        fun b(x: Int, y: Int): Float = (pixels[y * width + x] and 0xFF) / 255.0f

        /** Unpacked A channel `[0,1]` at (x, y) — coverage (fallback straight alpha). */
        fun a(x: Int, y: Int): Float = ((pixels[y * width + x] ushr 24) and 0xFF) / 255.0f

        /**
         * Wrap this data buffer into an ARGB_8888 [Bitmap] the shader samples as
         * DATA. Built un-premultiplied and color-unmanaged so Skia neither
         * premultiplies the R/G fields against alpha nor color-manages them: the
         * #1 fidelity trap for SDF-in-a-bitmap. Touches the Android framework — NOT
         * covered by JVM unit tests.
         */
        fun toBitmap(): Bitmap {
            val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            // Do not let Skia premultiply or color-manage the channels: they carry
            // signed-distance / anchor values, not colors.
            bitmap.setPremultiplied(false)
            bitmap.setHasAlpha(true)
            bitmap.setPixels(pixels, 0, width, 0, 0, width, height)
            return bitmap
        }

        /**
         * Build a clamped [BitmapShader] over [toBitmap]. CLAMP so sampling past
         * the texture edge keeps the far-margin "no smoke / fully outside" values
         * instead of wrapping. Touches the Android framework — NOT JVM-unit-tested.
         */
        fun toShader(): BitmapShader =
            BitmapShader(toBitmap(), Shader.TileMode.CLAMP, Shader.TileMode.CLAMP)
    }

    /**
     * Convenience: build the array buffer and wrap it into a clamped
     * [BitmapShader] in one call. Run off the main thread (see class docs).
     */
    fun build(): BitmapShader = buildBuffer().toShader()

    /**
     * Pure array-layer entry point. Rasterizes the glyph SDF (R), chamfer smoke
     * anchor (G), stepped luminance (B), and coverage (A) into a packed-ARGB
     * [Buffer]. No Android types touched; safe to unit-test and to call from a
     * worker thread.
     */
    fun buildBuffer(): Buffer {
        val width = textureWidth
        val height = textureHeight
        val pixels = renderVectorCursor(width, height)
        return Buffer(width, height, pixels)
    }

    // --- core raster (port of render_vector_cursor) -------------------------

    private fun renderVectorCursor(width: Int, height: Int): IntArray {
        val margin = (SMOKE_MARGIN * TEXTURE_SCALE).toFloat()
        val glyphW = (DESKTOP_WIDTH * TEXTURE_SCALE).toFloat()
        val glyphH = (DESKTOP_HEIGHT * TEXTURE_SCALE).toFloat()
        val scaleX = glyphW / SOURCE_WIDTH
        val scaleY = glyphH / SOURCE_HEIGHT

        // Flatten the glyph path, then round its corners (Chaikin).
        val path = chaikinRound(flattenedCursorPath(scaleX, scaleY, margin, margin), CORNER_ROUNDING)

        // Outward extent of the white outline ring (texture px), from the shared
        // SDF parameters so it matches what the shader reconstructs.
        val strokeExtent = STROKE_EDGE * SDF_RANGE_TEXELS

        // Glyph raster bounds: the glyph rect plus a pad covering the full SDF
        // range. Outside this box the canvas is pure smoke margin.
        val pad = ceil(SDF_RANGE_TEXELS * 0.5f).toInt() + 2
        val marginI = margin.toInt()
        val gx0 = (marginI - pad).coerceAtLeast(0)
        val gy0 = (marginI - pad).coerceAtLeast(0)
        val gx1 = (marginI + glyphW.toInt() + pad).coerceAtMost(width)
        val gy1 = (marginI + glyphH.toInt() + pad).coerceAtMost(height)

        // R defaults to "far outside" (1.0 = fully transparent) so untouched
        // margin pixels never reconstruct as fill.
        val glyphSdf = FloatArray(width * height) { 1.0f }
        val glyphAlpha = FloatArray(width * height)
        val glyphLum = FloatArray(width * height)

        for (y in gy0 until gy1) {
            for (x in gx0 until gx1) {
                val px = x + 0.5f
                val py = y + 0.5f
                val dist = distanceToPath(px, py, path)
                val signed = if (pointInPolygon(px, py, path)) -dist else dist
                val index = y * width + x
                glyphSdf[index] = (signed / SDF_RANGE_TEXELS + 0.5f).coerceIn(0.0f, 1.0f)
                // Coverage = fill plus the ring out to `strokeExtent`.
                glyphAlpha[index] = (0.5f + strokeExtent - signed).coerceIn(0.0f, 1.0f)
                glyphLum[index] = if (signed > 0.0f) 1.0f else 0.0f
            }
        }

        // G channel: smoke anchor (chamfer distance transform).
        val smokeAnchor = cursorSmokeAnchor(glyphAlpha, width, height, margin)

        // Pack: R = SDF, G = smoke anchor, B = stepped luminance, A = coverage.
        val pixels = IntArray(width * height)
        for (index in pixels.indices) {
            val r = floatToU8(glyphSdf[index] * 255.0f)
            val gCh = floatToU8(smokeAnchor[index] * 255.0f)
            val b = floatToU8(glyphLum[index] * 255.0f)
            val aCh = floatToU8(glyphAlpha[index] * 255.0f)
            pixels[index] = (aCh shl 24) or (r shl 16) or (gCh shl 8) or b
        }
        return pixels
    }

    /**
     * Distance-anchor field for the cursor smoke (G): `1` on/inside the glyph
     * silhouette, ramping to `0` at `reach` pixels outward. Built with a two-pass
     * chamfer distance transform seeded on the glyph coverage — O(width*height)
     * with a tiny constant (the desktop's known startup hotspot). Mirrors
     * `cursor_smoke_anchor` in cursor_texture.rs.
     */
    internal fun cursorSmokeAnchor(
        glyphAlpha: FloatArray,
        width: Int,
        height: Int,
        reach: Float,
    ): FloatArray {
        val w = width
        val h = height
        val inf = Float.MAX_VALUE / 4.0f
        val dist = FloatArray(w * h) { i -> if (glyphAlpha[i] > 0.5f) 0.0f else inf }
        val d1 = 1.0f
        val d2 = sqrt(2.0f)

        // Forward pass: propagate from the top-left neighborhood.
        for (y in 0 until h) {
            for (x in 0 until w) {
                val i = y * w + x
                var v = dist[i]
                if (x > 0) v = minOf(v, dist[i - 1] + d1)
                if (y > 0) {
                    v = minOf(v, dist[i - w] + d1)
                    if (x > 0) v = minOf(v, dist[i - w - 1] + d2)
                    if (x + 1 < w) v = minOf(v, dist[i - w + 1] + d2)
                }
                dist[i] = v
            }
        }
        // Backward pass: propagate from the bottom-right neighborhood.
        for (y in h - 1 downTo 0) {
            for (x in w - 1 downTo 0) {
                val i = y * w + x
                var v = dist[i]
                if (x + 1 < w) v = minOf(v, dist[i + 1] + d1)
                if (y + 1 < h) {
                    v = minOf(v, dist[i + w] + d1)
                    if (x + 1 < w) v = minOf(v, dist[i + w + 1] + d2)
                    if (x > 0) v = minOf(v, dist[i + w - 1] + d2)
                }
                dist[i] = v
            }
        }

        val r = maxOf(reach, 1.0f)
        return FloatArray(w * h) { i -> (1.0f - dist[i] / r).coerceIn(0.0f, 1.0f) }
    }

    /**
     * Chaikin corner-cutting on a CLOSED polygon: each iteration replaces every
     * vertex with two points at 1/4 and 3/4 along its outgoing edge. Mirrors
     * `chaikin_round` in cursor_texture.rs.
     */
    private fun chaikinRound(points: List<Pt>, iterations: Int): List<Pt> {
        var pts = points
        repeat(iterations) {
            val n = pts.size
            if (n < 3) return pts
            val next = ArrayList<Pt>(n * 2)
            for (i in 0 until n) {
                val p = pts[i]
                val q = pts[(i + 1) % n]
                next.add(Pt(0.75f * p.x + 0.25f * q.x, 0.75f * p.y + 0.25f * q.y))
                next.add(Pt(0.25f * p.x + 0.75f * q.x, 0.25f * p.y + 0.75f * q.y))
            }
            pts = next
        }
        return pts
    }

    private fun flattenedCursorPath(
        scaleX: Float,
        scaleY: Float,
        offsetX: Float,
        offsetY: Float,
    ): List<Pt> {
        val points = ArrayList<Pt>()
        var current = Pt(0.0f, 0.0f)
        for (command in CURSOR_PATH) {
            when (command) {
                is Cmd.Move -> {
                    current = scalePoint(command.p, scaleX, scaleY, offsetX, offsetY)
                    points.add(current)
                }
                is Cmd.Line -> {
                    current = scalePoint(command.p, scaleX, scaleY, offsetX, offsetY)
                    points.add(current)
                }
                is Cmd.Quad -> {
                    val start = current
                    val control = scalePoint(command.control, scaleX, scaleY, offsetX, offsetY)
                    val end = scalePoint(command.end, scaleX, scaleY, offsetX, offsetY)
                    for (step in 1..12) {
                        val t = step / 12.0f
                        points.add(quadraticPoint(start, control, end, t))
                    }
                    current = end
                }
            }
        }
        return points
    }

    private fun scalePoint(p: Pt, scaleX: Float, scaleY: Float, offsetX: Float, offsetY: Float): Pt =
        Pt(p.x * scaleX + offsetX, p.y * scaleY + offsetY)

    private fun quadraticPoint(start: Pt, control: Pt, end: Pt, t: Float): Pt {
        val mt = 1.0f - t
        return Pt(
            mt * mt * start.x + 2.0f * mt * t * control.x + t * t * end.x,
            mt * mt * start.y + 2.0f * mt * t * control.y + t * t * end.y,
        )
    }

    private fun pointInPolygon(px: Float, py: Float, polygon: List<Pt>): Boolean {
        var inside = false
        val n = polygon.size
        for (index in 0 until n) {
            val a = polygon[index]
            val b = polygon[(index + 1) % n]
            if ((a.y > py) != (b.y > py) &&
                px < (b.x - a.x) * (py - a.y) / (b.y - a.y) + a.x
            ) {
                inside = !inside
            }
        }
        return inside
    }

    private fun distanceToPath(px: Float, py: Float, path: List<Pt>): Float {
        var best = Float.MAX_VALUE
        val n = path.size
        for (index in 0 until n) {
            best = minOf(best, distanceToSegment(px, py, path[index], path[(index + 1) % n]))
        }
        return best
    }

    private fun distanceToSegment(px: Float, py: Float, a: Pt, b: Pt): Float {
        val abx = b.x - a.x
        val aby = b.y - a.y
        val denom = maxOf(abx * abx + aby * aby, 0.0001f)
        val t = (((px - a.x) * abx + (py - a.y) * aby) / denom).coerceIn(0.0f, 1.0f)
        val x = a.x + abx * t
        val y = a.y + aby * t
        val dx = px - x
        val dy = py - y
        return sqrt(dx * dx + dy * dy)
    }

    private fun floatToU8(value: Float): Int =
        Math.round(value).coerceIn(0, 255)
}
