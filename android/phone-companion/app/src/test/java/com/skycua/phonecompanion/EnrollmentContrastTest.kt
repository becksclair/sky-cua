package com.skycua.phonecompanion

import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow

class EnrollmentContrastTest {
    @Test
    fun lightEnrollmentTextRolesMeetNormalTextContrast() {
        assertPaletteContrast(loadColors("values/colors.xml"))
    }

    @Test
    fun darkEnrollmentTextRolesMeetNormalTextContrast() {
        assertPaletteContrast(loadColors("values-night/colors.xml"))
    }

    private fun assertPaletteContrast(colors: Map<String, Int>) {
        val pairs =
            listOf(
                "enrollment_muted" to "sky_bg",
                "enrollment_muted" to "sky_surface",
                "enrollment_muted" to "sky_secondary",
                "enrollment_on_primary" to "sky_primary",
                "sky_on_secondary" to "sky_secondary",
                "sky_heading" to "sky_bg",
                "sky_heading" to "sky_surface",
                "sky_text" to "sky_bg",
                "sky_text" to "sky_surface",
                "sky_ok" to "sky_ok_bg",
                "sky_off" to "sky_off_bg",
            )
        for ((foreground, background) in pairs) {
            val ratio = contrast(colors.getValue(foreground), colors.getValue(background))
            assertTrue(
                "$foreground on $background is ${"%.3f".format(ratio)}:1; expected at least 4.5:1",
                ratio >= 4.5,
            )
        }
    }

    private fun loadColors(relativePath: String): Map<String, Int> {
        val appRoot =
            sequenceOf(File("."), File("app"))
                .firstOrNull { File(it, "src/main/res/$relativePath").isFile }
                ?: error("could not locate Android app resources from ${File(".").absolutePath}")
        val xml = File(appRoot, "src/main/res/$relativePath").readText()
        return COLOR.findAll(xml).associate { match ->
            match.groupValues[1] to parseColor(match.groupValues[2])
        }
    }

    private fun parseColor(value: String): Int {
        val hex = value.removePrefix("#")
        val rgb = if (hex.length == 8) hex.drop(2) else hex
        require(rgb.length == 6) { "expected an opaque RGB/ARGB color, got $value" }
        return rgb.toInt(16)
    }

    private fun contrast(first: Int, second: Int): Double {
        val firstLuminance = luminance(first)
        val secondLuminance = luminance(second)
        return (max(firstLuminance, secondLuminance) + 0.05) /
            (min(firstLuminance, secondLuminance) + 0.05)
    }

    private fun luminance(color: Int): Double =
        0.2126 * linear((color shr 16) and 0xff) +
            0.7152 * linear((color shr 8) and 0xff) +
            0.0722 * linear(color and 0xff)

    private fun linear(component: Int): Double {
        val value = component / 255.0
        return if (value <= 0.04045) value / 12.92 else ((value + 0.055) / 1.055).pow(2.4)
    }

    private companion object {
        val COLOR = Regex("""<color\s+name="([^"]+)">\s*(#[0-9A-Fa-f]{6,8})\s*</color>""")
    }
}
