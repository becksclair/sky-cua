package com.skycua.phonecompanion.overlay

import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.JsonValue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Validates the pure overlay math in [OverlayMath] against the shared canonical
 * fixtures in `resources/overlay/agent_overlay_motion_fixtures.json`. This proves
 * Android consumes the same reference samples as the desktop/WGSL side. The
 * canonical fixture is copied into `src/test/resources/overlay/` so it is on the
 * JVM test classpath and loads reliably regardless of Gradle's working directory.
 */
class OverlaySpecFixtureTest {
    private val eps = 1e-4f

    private val rootFixture: JsonValue.Obj by lazy {
        val resource = javaClass.classLoader!!.getResource("overlay/agent_overlay_motion_fixtures.json")
        require(resource != null) { "fixture not found on classpath: overlay/agent_overlay_motion_fixtures.json" }
        JsonParser.parseObject(resource.readText())
    }

    private fun fixtureArray(name: String): List<JsonValue.Obj> {
        val fixtures = rootFixture.obj("fixtures") ?: error("missing fixtures object")
        val arr = fixtures.arr(name) ?: error("missing fixture '$name'")
        return arr.items.map { it as? JsonValue.Obj ?: error("expected object in $name") }
    }

    private fun JsonValue.Obj.num(key: String): Double {
        val value = this[key]
        require(value is JsonValue.Num) { "expected number for $key" }
        return value.value
    }

    private fun JsonValue.Obj.point(key: String): OverlayMath.Point {
        val obj = this.obj(key) ?: error("expected object for $key")
        return OverlayMath.Point(obj.num("x").toFloat(), obj.num("y").toFloat())
    }

    @Test
    fun fixtureFileLoadsAndHasExpectedSchemaVersion() {
        assertEquals(1.0, rootFixture.num("schema_version"), eps.toDouble())
        assertTrue(rootFixture.obj("fixtures")!!.entries.isNotEmpty())
    }

    @Test
    fun breathingIntensityMatchesFixtures() {
        for (case in fixtureArray("breathing_intensity")) {
            val elapsed = case.num("elapsed_ms").toLong()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "breathing at $elapsed ms",
                expected,
                OverlayMath.breathingIntensity(elapsed),
                eps,
            )
        }
    }

    @Test
    fun wavePhaseMatchesFixtures() {
        for (case in fixtureArray("wave_phase")) {
            val elapsed = case.num("elapsed_ms").toLong()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "wave phase at $elapsed ms",
                expected,
                OverlayMath.wavePhase(elapsed),
                eps,
            )
        }
    }

    @Test
    fun haloBreathingMatchesFixtures() {
        for (case in fixtureArray("halo_breathing")) {
            val elapsed = case.num("elapsed_ms").toLong()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "halo breathing at $elapsed ms",
                expected,
                OverlayMath.breathing01(elapsed),
                eps,
            )
        }
    }

    @Test
    fun pulseIntensityMatchesFixtures() {
        for (case in fixtureArray("pulse_intensity")) {
            val progress = case.num("progress").toFloat()
            val baseline = case.num("baseline").toFloat()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "pulse at progress=$progress baseline=$baseline",
                expected,
                OverlayMath.pulseIntensity(progress, baseline),
                eps,
            )
        }
    }

    @Test
    fun rippleRadiusMatchesFixtures() {
        for (case in fixtureArray("ripple_radius")) {
            val progress = case.num("progress").toFloat()
            val minRadius = case.num("min_radius").toFloat()
            val maxRadius = case.num("max_radius").toFloat()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "ripple radius at $progress",
                expected,
                OverlayMath.rippleRadius(progress, maxRadius, minRadius),
                eps,
            )
        }
    }

    @Test
    fun rippleAlphaMatchesFixtures() {
        for (case in fixtureArray("ripple_alpha")) {
            val progress = case.num("progress").toFloat()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "ripple alpha at $progress",
                expected,
                OverlayMath.rippleAlpha(progress),
                eps,
            )
        }
    }

    @Test
    fun clickScaleMatchesFixtures() {
        for (case in fixtureArray("click_scale")) {
            val progress = case.num("progress").toFloat()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "click scale at $progress",
                expected,
                OverlayMath.clickScale(progress),
                eps,
            )
        }
    }

    @Test
    fun trailAlphaMatchesFixtures() {
        for (case in fixtureArray("trail_alpha")) {
            val sampleIndex = case.num("sample_index").toInt()
            val sampleCount = case.num("sample_count").toInt()
            val headAlpha = case.num("head_alpha").toFloat()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "trail alpha index=$sampleIndex count=$sampleCount",
                expected,
                OverlayMath.trailAlpha(sampleIndex, sampleCount, headAlpha),
                eps,
            )
        }
    }

    @Test
    fun noNoWiggleMatchesFixtures() {
        for (case in fixtureArray("no_no_wiggle")) {
            val progress = case.num("progress").toFloat()
            val amplitude = case.num("amplitude_deg").toFloat()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "no-no wiggle at $progress",
                expected,
                OverlayMath.noNoWiggleDeg(progress, amplitude),
                eps,
            )
        }
    }

    @Test
    fun pathSamplingMatchesFixtures() {
        for (case in fixtureArray("path_sampling")) {
            val arr = case.arr("points") ?: error("expected points array")
            val points =
                arr.items.map {
                    val p = it as? JsonValue.Obj ?: error("expected point object")
                    OverlayMath.Point(p.num("x").toFloat(), p.num("y").toFloat())
                }
            val progress = case.num("progress").toFloat()
            val expected = case.point("expected")
            val actual = OverlayMath.pointAtProgress(points, progress)
            assertEquals("path x at $progress", expected.x, actual.x, eps)
            assertEquals("path y at $progress", expected.y, actual.y, eps)
        }
    }
}
