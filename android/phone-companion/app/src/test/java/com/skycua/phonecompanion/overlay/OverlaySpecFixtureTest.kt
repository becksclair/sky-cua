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

    private fun JsonValue.Obj.points(key: String): List<OverlayMath.Point> {
        val arr = this.arr(key) ?: error("expected array for $key")
        return arr.items.map {
            val p = it as? JsonValue.Obj ?: error("expected point object in $key")
            OverlayMath.Point(p.num("x").toFloat(), p.num("y").toFloat())
        }
    }

    private fun tolerance(name: String): Float {
        val map = rootFixture.obj("tolerance") ?: error("missing tolerance map")
        return map.num(name).toFloat()
    }

    /**
     * Builds the production cursor mover exactly as [AgentOverlayController] does
     * at density 1, so trajectory fixtures replay against the shipping constants.
     */
    private fun productionMover(): OverlayMath.Mover2D {
        val density = 1f
        return OverlayMath.Mover2D(
            maxSpeed = OverlayMath.CURSOR_MAX_SPEED_DP_S * density,
            accel = OverlayMath.CURSOR_ACCEL_DP_S2 * density,
            turnRateRad = Math.toRadians(OverlayMath.CURSOR_TURN_RATE_DEG_S.toDouble()).toFloat(),
            arriveRadius = OverlayMath.CURSOR_ARRIVE_RADIUS_DP * density,
            homingRadius = OverlayMath.CURSOR_HOMING_RADIUS_DP * density,
            homingBoost = OverlayMath.CURSOR_HOMING_TURN_BOOST,
            defaultHeadingRad = Math.toRadians(OverlayMath.CURSOR_NOSE_DEG.toDouble()).toFloat(),
        )
    }

    /** Headings compare across the wrap: |wrapRadians(expected - actual)| <= tol. */
    private fun assertHeading(message: String, expected: Float, actual: Float, tol: Float) {
        val diff = Math.abs(OverlayMath.wrapRadians(expected - actual))
        assertTrue("$message: |wrapRadians($expected - $actual)| = $diff > $tol", diff <= tol)
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

    /**
     * Replays each generated mover trajectory against the production
     * [OverlayMath.Mover2D]: optional bounds, optional snap start, then stepping
     * toward each segment target for its step count at the case dt. Mid-flight
     * samples compare at `tolerance.mover`; samples at or past `settled_step`
     * (and the steady state itself) at `tolerance.default`. Trajectory inputs
     * are exact float32 decimals, so the replay is bit-exact.
     */
    @Test
    fun moverTrajectoryMatchesFixtures() {
        val moverTol = tolerance("mover")
        val defaultTol = tolerance("default")
        val cases = fixtureArray("mover_trajectory")
        assertTrue("mover_trajectory family must not be empty", cases.isNotEmpty())
        for (case in cases) {
            val name = case.string("name") ?: error("mover_trajectory case missing name")
            val mover = productionMover()
            case.obj("bounds")?.let { bounds ->
                // The desktop port takes a rect; the Kotlin reference clamps each
                // axis to [0, max], so generated bounds always have zero minimums.
                assertEquals("$name bounds min_x", 0.0, bounds.num("min_x"), 0.0)
                assertEquals("$name bounds min_y", 0.0, bounds.num("min_y"), 0.0)
                mover.setBounds(bounds.num("max_x").toFloat(), bounds.num("max_y").toFloat())
            }
            case.obj("start")?.let { start ->
                mover.snapTo(start.num("x").toFloat(), start.num("y").toFloat())
            }
            val dt = case.num("dt").toFloat()
            val segments =
                (case.arr("segments") ?: error("$name missing segments")).items.map {
                    it as? JsonValue.Obj ?: error("expected segment object in $name")
                }
            val samplesByStep =
                (case.arr("samples") ?: error("$name missing samples")).items.associate {
                    val sample = it as? JsonValue.Obj ?: error("expected sample object in $name")
                    sample.num("step").toInt() to sample
                }
            val settledStep = (case["settled_step"] as? JsonValue.Num)?.toInt()
            val finalTarget = segments.last().point("target")
            var step = 0
            for (segment in segments) {
                val target = segment.point("target")
                repeat(segment.num("steps").toInt()) {
                    mover.step(target.x, target.y, dt)
                    step += 1
                    samplesByStep[step]?.let { sample ->
                        val tol = if (settledStep != null && step >= settledStep) defaultTol else moverTol
                        assertEquals("$name x at step $step", sample.num("x").toFloat(), mover.x, tol)
                        assertEquals("$name y at step $step", sample.num("y").toFloat(), mover.y, tol)
                        assertHeading(
                            "$name heading at step $step",
                            sample.num("heading_rad").toFloat(),
                            mover.headingRad,
                            tol,
                        )
                        assertEquals("$name speed at step $step", sample.num("speed").toFloat(), mover.speed, tol)
                    }
                    if (settledStep != null && step >= settledStep) {
                        // Settled means landed: from settled_step on, the mover
                        // holds the final target exactly with zero speed.
                        assertEquals("$name settled x at step $step", finalTarget.x, mover.x, 0f)
                        assertEquals("$name settled y at step $step", finalTarget.y, mover.y, 0f)
                        assertEquals("$name settled speed at step $step", 0f, mover.speed, 0f)
                    }
                }
            }
            val unreplayed = samplesByStep.keys.filter { it > step }
            assertTrue("$name has samples beyond the replayed steps: $unreplayed", unreplayed.isEmpty())
        }
    }

    @Test
    fun approachAngleMatchesFixtures() {
        for (case in fixtureArray("approach_angle")) {
            val current = case.num("current").toFloat()
            val target = case.num("target").toFloat()
            val maxDelta = case.num("max_delta").toFloat()
            val expected = case.num("expected").toFloat()
            assertEquals(
                "approach angle current=$current target=$target maxDelta=$maxDelta",
                expected,
                OverlayMath.approachAngleDeg(current, target, maxDelta),
                eps,
            )
        }
    }

    @Test
    fun wrapRadiansMatchesFixtures() {
        for (case in fixtureArray("wrap_radians")) {
            val value = case.num("value").toFloat()
            val expected = case.num("expected").toFloat()
            assertHeading("wrap radians of $value", expected, OverlayMath.wrapRadians(value), eps)
        }
    }

    /**
     * Mirrors [AgentOverlayController.sampleTrail] (private): the trail is
     * [OverlaySpec.Shared.Effects.TRAIL_SAMPLES] arc-length samples of the ideal
     * polyline from the path start to the swept head at `progress`.
     */
    @Test
    fun trailResampleMatchesFixtures() {
        val sampleCount = OverlaySpec.Shared.Effects.TRAIL_SAMPLES
        for (case in fixtureArray("trail_resample")) {
            val points = case.points("points")
            val progress = case.num("progress").toFloat()
            assertEquals("trail_resample sample_count must match the spec", sampleCount, case.num("sample_count").toInt())
            val expected = case.points("expected")
            assertEquals("trail_resample expected sample list size", sampleCount, expected.size)
            for (i in 0 until sampleCount) {
                val frac = progress * (i.toFloat() / (sampleCount - 1).toFloat())
                val actual = OverlayMath.pointAtProgress(points, frac)
                assertEquals("trail sample $i x at progress $progress", expected[i].x, actual.x, eps)
                assertEquals("trail sample $i y at progress $progress", expected[i].y, actual.y, eps)
            }
        }
    }
}
