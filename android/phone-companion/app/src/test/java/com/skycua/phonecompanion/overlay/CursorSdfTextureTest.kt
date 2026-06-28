package com.skycua.phonecompanion.overlay

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * JVM unit tests for the pure array layer of [CursorSdfTexture], mirroring the
 * relevant tests in `crates/sky-cua-overlay-host/src/renderer/cursor_texture.rs`.
 * Everything here exercises [CursorSdfTexture.buildBuffer] (plain primitive
 * arrays), so it runs without a real `Bitmap` under
 * `unitTests.isReturnDefaultValues = true`. The Bitmap/BitmapShader wrappers are
 * deliberately untested here (they touch the Android framework).
 */
class CursorSdfTextureTest {
    private val buffer by lazy { CursorSdfTexture.buildBuffer() }

    /**
     * Mirrors `cursor_image_is_supersampled_above_desktop_size`: the texture
     * covers the glyph footprint PLUS the smoke margin, at the 4x supersample.
     */
    @Test
    fun textureIsSupersampledFootprintWithMargin() {
        assertEquals(
            CursorSdfTexture.FOOTPRINT_WIDTH * CursorSdfTexture.TEXTURE_SCALE,
            buffer.width,
        )
        assertEquals(
            CursorSdfTexture.FOOTPRINT_HEIGHT * CursorSdfTexture.TEXTURE_SCALE,
            buffer.height,
        )
        assertTrue(
            "footprint includes a smoke margin beyond the glyph",
            buffer.width > CursorSdfTexture.DESKTOP_WIDTH * CursorSdfTexture.TEXTURE_SCALE,
        )
        assertEquals(
            "pixel buffer matches the supersampled texture dimensions",
            buffer.width * buffer.height,
            buffer.pixels.size,
        )
    }

    /**
     * Mirrors `cursor_image_is_vector_rendered_with_transparent_corners`: the far
     * margin corner is fully transparent, and the footprint hotspot is covered.
     */
    @Test
    fun farCornerIsTransparentAndHotspotIsCovered() {
        assertEquals("the margin corner is transparent", 0.0f, buffer.a(0, 0), 0.0f)

        val hx = CursorSdfTexture.FOOTPRINT_HOTSPOT_X * CursorSdfTexture.TEXTURE_SCALE
        val hy = CursorSdfTexture.FOOTPRINT_HOTSPOT_Y * CursorSdfTexture.TEXTURE_SCALE
        assertTrue("vector cursor should cover the hotspot", buffer.a(hx, hy) > 0.0f)
    }

    /**
     * Mirrors `cursor_smoke_anchor_peaks_at_glyph_and_fades_into_margin`: the G
     * channel saturates inside the glyph and is zero in the far transparent
     * corner of the margin.
     */
    @Test
    fun smokeAnchorPeaksAtGlyphAndFadesIntoMargin() {
        val hx = CursorSdfTexture.FOOTPRINT_HOTSPOT_X * CursorSdfTexture.TEXTURE_SCALE
        val hy = CursorSdfTexture.FOOTPRINT_HOTSPOT_Y * CursorSdfTexture.TEXTURE_SCALE
        assertTrue("anchor saturates inside the glyph", buffer.g(hx, hy) > 200f / 255f)
        assertEquals("anchor is zero in the far margin corner", 0.0f, buffer.g(0, 0), 0.0f)
    }

    /** The SDF reads ~0.5 on the glyph path (the encoded zero-distance contour). */
    @Test
    fun sdfIsApproximatelyHalfOnThePath() {
        // R encodes signed distance: 0.5 lies exactly on the path. As a row scans
        // from outside (R high) across the boundary into the fill (R < 0.5), it
        // MUST cross 0.5; at the crossing a texel sits within one step's worth of
        // the boundary, so R there is within ~1/SDF_RANGE of 0.5. Find a row that
        // has both an interior texel and an exterior texel and verify the crossing.
        val hy = CursorSdfTexture.FOOTPRINT_HOTSPOT_Y * CursorSdfTexture.TEXTURE_SCALE
        val width = buffer.width

        // The minimum R on this row is the deepest interior texel; the texel
        // nearest 0.5 between it and the exterior is the on-path sample.
        var closest = Float.MAX_VALUE
        var sawInterior = false
        var sawExterior = false
        for (x in 0 until width) {
            val r = buffer.r(x, hy)
            if (r < 0.45f) sawInterior = true
            if (r > 0.55f) sawExterior = true
            val diff = kotlin.math.abs(r - 0.5f)
            if (diff < closest) closest = diff
        }
        assertTrue("row crosses the glyph interior", sawInterior)
        assertTrue("row reaches the glyph exterior", sawExterior)
        // One texel maps to 1/SDF_RANGE_TEXELS of normalized distance, so the
        // nearest-to-0.5 sample on a crossing row must land inside that band.
        val band = 1.0f / CursorSdfTexture.SDF_RANGE_TEXELS
        assertTrue(
            "SDF should hit ~0.5 on the path (delta=$closest, band=$band)",
            closest <= band + 1e-3f,
        )
    }

    /**
     * The SDF ramps monotonically away from the path: along a horizontal scan that
     * crosses the glyph, R should decrease toward the interior (more negative
     * signed distance) and increase toward the exterior. We assert it is
     * monotone-increasing as we move outward from an interior texel toward the
     * margin on one side.
     */
    @Test
    fun sdfRampsMonotonicallyOutward() {
        // Pick the row through the footprint hotspot and find an interior texel
        // (R < 0.5, i.e. inside the fill).
        val hy = CursorSdfTexture.FOOTPRINT_HOTSPOT_Y * CursorSdfTexture.TEXTURE_SCALE
        val width = buffer.width

        var interiorX = -1
        for (x in 0 until width) {
            if (buffer.r(x, hy) < 0.45f) {
                interiorX = x
                break
            }
        }
        assertTrue("found an interior (inside-fill) texel on the hotspot row", interiorX >= 0)

        // Walk LEFT (outward toward the margin) until the field saturates at 1.0.
        // Each step must be non-decreasing: signed distance grows monotonically as
        // we leave the fill (within the SDF's resolved range).
        var prev = buffer.r(interiorX, hy)
        var sawIncrease = false
        for (x in interiorX - 1 downTo (interiorX - 30).coerceAtLeast(0)) {
            val cur = buffer.r(x, hy)
            assertTrue(
                "SDF must be non-decreasing moving outward at x=$x (prev=$prev cur=$cur)",
                cur >= prev - 1e-3f,
            )
            if (cur > prev + 1e-3f) sawIncrease = true
            prev = cur
            if (cur >= 0.999f) break
        }
        assertTrue("SDF should visibly ramp upward moving outward", sawIncrease)
    }

    /** The chamfer anchor is 1.0 on the glyph and decreases to ~0 at the margin reach. */
    @Test
    fun chamferAnchorIsOneOnGlyphAndZeroAtReach() {
        val hx = CursorSdfTexture.FOOTPRINT_HOTSPOT_X * CursorSdfTexture.TEXTURE_SCALE
        val hy = CursorSdfTexture.FOOTPRINT_HOTSPOT_Y * CursorSdfTexture.TEXTURE_SCALE
        // On the glyph the anchor is saturated (== 1.0 within u8 quantization).
        assertEquals("anchor is 1.0 on the glyph", 1.0f, buffer.g(hx, hy), 1.5f / 255f)
        // The extreme corner is a full margin away from any glyph coverage, so the
        // anchor has decayed to 0.
        assertEquals("anchor decays to 0 at the far corner", 0.0f, buffer.g(0, 0), 0.0f)
    }

    /**
     * Unit-test the chamfer transform in isolation against a hand-built coverage
     * grid: a single seeded cell anchors at 1.0, its orthogonal neighbor at
     * `1 - 1/reach`, and a far cell decays to 0.
     */
    @Test
    fun chamferTransformDecaysWithDistanceFromSeed() {
        val w = 9
        val h = 9
        val reach = 4.0f
        val cov = FloatArray(w * h)
        // Seed a single covered cell at the center.
        cov[4 * w + 4] = 1.0f
        val anchor = CursorSdfTexture.cursorSmokeAnchor(cov, w, h, reach)

        assertEquals("seed cell is anchored at 1.0", 1.0f, anchor[4 * w + 4], 1e-5f)
        // Orthogonal neighbor: chamfer distance 1 -> 1 - 1/reach.
        assertEquals(
            "orthogonal neighbor decays by one step",
            1.0f - 1.0f / reach,
            anchor[4 * w + 5],
            1e-5f,
        )
        // Far corner is well past the reach -> clamped to 0.
        assertEquals("far corner is past reach", 0.0f, anchor[0], 1e-5f)
    }

    /**
     * Guard against drift between the three copies of the cursor path: the Kotlin
     * [CursorSdfTexture] path, the Rust `CURSOR_PATH`, and the VectorDrawable
     * `agent_cursor.xml`. Parses the drawable's `android:pathData` and asserts the
     * command-by-command geometry matches the Kotlin key vertices and quad
     * controls.
     */
    @Test
    fun cursorPathMatchesAgentCursorDrawable() {
        val xml = findAgentCursorXml().readText()
        val pathData = extractPathData(xml)
        val parsed = parsePathData(pathData)

        // The drawable's command list (M/Q/L/Q/L/Q/L/Q/L/Q/L/Q/L/Q + Z).
        val expectedKeyVertices = CursorSdfTexture.pathKeyVertices
        val expectedQuadControls = CursorSdfTexture.pathQuadControls

        assertEquals(
            "drawable key vertices must match the Kotlin CURSOR_PATH",
            expectedKeyVertices.size,
            parsed.keyVertices.size,
        )
        for (i in expectedKeyVertices.indices) {
            assertEquals("key vertex $i x", expectedKeyVertices[i].first, parsed.keyVertices[i].first, 1e-3f)
            assertEquals("key vertex $i y", expectedKeyVertices[i].second, parsed.keyVertices[i].second, 1e-3f)
        }
        assertEquals(
            "drawable quad controls must match the Kotlin CURSOR_PATH",
            expectedQuadControls.size,
            parsed.quadControls.size,
        )
        for (i in expectedQuadControls.indices) {
            assertEquals("quad control $i x", expectedQuadControls[i].first, parsed.quadControls[i].first, 1e-3f)
            assertEquals("quad control $i y", expectedQuadControls[i].second, parsed.quadControls[i].second, 1e-3f)
        }
    }

    // --- minimal pathData parser (M/L/Q/Z only, absolute) -------------------

    private data class ParsedPath(
        val keyVertices: List<Pair<Float, Float>>,
        val quadControls: List<Pair<Float, Float>>,
    )

    private fun extractPathData(xml: String): String {
        val marker = "android:pathData=\""
        val start = xml.indexOf(marker)
        require(start >= 0) { "android:pathData not found in agent_cursor.xml" }
        val from = start + marker.length
        val end = xml.indexOf('"', from)
        require(end >= 0) { "unterminated android:pathData" }
        return xml.substring(from, end)
    }

    /**
     * Parse the absolute M/L/Q/Z pathData used by `agent_cursor.xml`. Returns the
     * Move/Line/Quad-end key vertices (in command order, matching
     * [CursorSdfTexture.pathKeyVertices]) and the quad control points.
     */
    private fun parsePathData(data: String): ParsedPath {
        val keyVertices = ArrayList<Pair<Float, Float>>()
        val quadControls = ArrayList<Pair<Float, Float>>()
        var i = 0
        val n = data.length
        while (i < n) {
            val c = data[i]
            when (c) {
                'M', 'L' -> {
                    i++
                    val (x, j1) = readFloat(data, i)
                    val j2 = skipSep(data, j1)
                    val (y, j3) = readFloat(data, j2)
                    keyVertices.add(x to y)
                    i = j3
                }
                'Q' -> {
                    i++
                    val (cx, j1) = readFloat(data, i)
                    val (cy, j2) = readFloat(data, skipSep(data, j1))
                    val (ex, j3) = readFloat(data, skipSep(data, j2))
                    val (ey, j4) = readFloat(data, skipSep(data, j3))
                    quadControls.add(cx to cy)
                    keyVertices.add(ex to ey)
                    i = j4
                }
                'Z', 'z' -> i++
                ' ', ',', '\n', '\r', '\t' -> i++
                else -> error("unexpected pathData token '$c' at $i in: $data")
            }
        }
        return ParsedPath(keyVertices, quadControls)
    }

    private fun skipSep(data: String, start: Int): Int {
        var i = start
        while (i < data.length && (data[i] == ' ' || data[i] == ',')) i++
        return i
    }

    private fun readFloat(data: String, start: Int): Pair<Float, Int> {
        var i = start
        while (i < data.length && (data[i] == ' ' || data[i] == ',')) i++
        val from = i
        while (i < data.length && (data[i].isDigit() || data[i] == '.' || data[i] == '-' || data[i] == '+')) i++
        return data.substring(from, i).toFloat() to i
    }

    /**
     * Locate `app/src/main/res/drawable/agent_cursor.xml`. The drawable is not on
     * the JVM test classpath (only `src/test/resources` is), so resolve it by
     * walking up from the test working directory (Gradle runs unit tests with cwd
     * = the module dir) until the known relative path resolves.
     */
    private fun findAgentCursorXml(): File {
        val rel = "src/main/res/drawable/agent_cursor.xml"
        var dir: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (dir != null) {
            val direct = File(dir, rel)
            if (direct.isFile) return direct
            val viaApp = File(dir, "app/$rel")
            if (viaApp.isFile) return viaApp
            dir = dir.parentFile
        }
        error("could not locate agent_cursor.xml from user.dir=${System.getProperty("user.dir")}")
    }
}
