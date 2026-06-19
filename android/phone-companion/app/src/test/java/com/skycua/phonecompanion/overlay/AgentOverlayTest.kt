package com.skycua.phonecompanion.overlay

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the pure overlay math isolated in [OverlayMath]: duration
 * clamping, breathing/pulse glow curves, ripple progress, arc-length path
 * sampling, trail fade, and the screenshot capture hide/restore state machine.
 *
 * These exercise the logic the controller relies on without inflating a real
 * `View`/`WindowManager`/`ValueAnimator`, mirroring how [OverlayFlagsTest]
 * isolates the window-flag bitmask. Runs as a plain JVM unit test.
 */
class AgentOverlayTest {
    private val eps = 1e-4f

    // --- duration clamping ----------------------------------------------------

    @Test
    fun clampDurationRaisesTinyHintToMinimum() {
        assertEquals(OverlayMath.MIN_GESTURE_DURATION_MS, OverlayMath.clampDurationMs(0))
        assertEquals(OverlayMath.MIN_GESTURE_DURATION_MS, OverlayMath.clampDurationMs(-100))
        assertEquals(OverlayMath.MIN_GESTURE_DURATION_MS, OverlayMath.clampDurationMs(1))
    }

    @Test
    fun clampDurationCapsOversizedHint() {
        assertEquals(
            OverlayMath.MAX_GESTURE_DURATION_MS,
            OverlayMath.clampDurationMs(OverlayMath.MAX_GESTURE_DURATION_MS + 10_000),
        )
    }

    @Test
    fun clampDurationPassesThroughSaneHint() {
        assertEquals(450L, OverlayMath.clampDurationMs(450))
    }

    // --- clamp / easing -------------------------------------------------------

    @Test
    fun clamp01BoundsInput() {
        assertEquals(0f, OverlayMath.clamp01(-0.5f), eps)
        assertEquals(1f, OverlayMath.clamp01(1.5f), eps)
        assertEquals(0.25f, OverlayMath.clamp01(0.25f), eps)
    }

    @Test
    fun easeInOutHitsEndpointsAndMidpoint() {
        assertEquals(0f, OverlayMath.easeInOut(0f), eps)
        assertEquals(1f, OverlayMath.easeInOut(1f), eps)
        // smoothstep(0.5) == 0.5
        assertEquals(0.5f, OverlayMath.easeInOut(0.5f), eps)
        // smoothstep is symmetric around 0.5.
        assertEquals(
            1f - OverlayMath.easeInOut(0.3f),
            OverlayMath.easeInOut(0.7f),
            eps,
        )
    }

    // --- breathing glow -------------------------------------------------------

    @Test
    fun breathingStaysWithinBaselineBand() {
        val period = OverlayMath.BREATHE_PERIOD_MS
        for (i in 0..16) {
            val t = period * i / 16
            val v = OverlayMath.breathingIntensity(t)
            assertTrue("v=$v below floor", v >= OverlayMath.GLOW_BASELINE_MIN - eps)
            assertTrue("v=$v above ceil", v <= OverlayMath.GLOW_BASELINE_MAX + eps)
        }
    }

    @Test
    fun breathingIsPeriodic() {
        val period = OverlayMath.BREATHE_PERIOD_MS
        assertEquals(
            OverlayMath.breathingIntensity(300),
            OverlayMath.breathingIntensity(300 + period),
            eps,
        )
    }

    @Test
    fun breathingPeaksMidCycleAndTroughsAtEdges() {
        val period = OverlayMath.BREATHE_PERIOD_MS
        val trough = OverlayMath.breathingIntensity(0)
        val peak = OverlayMath.breathingIntensity(period / 2)
        assertEquals(OverlayMath.GLOW_BASELINE_MIN, trough, eps)
        assertEquals(OverlayMath.GLOW_BASELINE_MAX, peak, eps)
        assertTrue(peak > trough)
    }

    // --- wave -----------------------------------------------------------------

    @Test
    fun wavePhaseStaysInUnitRangeAndWraps() {
        val period = OverlayMath.WAVE_PERIOD_MS
        for (t in 0..(2 * period) step 97) {
            val p = OverlayMath.wavePhase(t)
            assertTrue("phase=$p below 0", p >= 0f)
            assertTrue("phase=$p reached 1", p < 1f)
        }
        // One full period later is the same phase (the comet has come full circle).
        assertEquals(
            OverlayMath.wavePhase(450),
            OverlayMath.wavePhase(450 + period),
            eps,
        )
    }

    @Test
    fun wavePhaseAdvancesAcrossTheCycle() {
        val period = OverlayMath.WAVE_PERIOD_MS
        assertEquals(0f, OverlayMath.wavePhase(0), eps)
        assertEquals(0.5f, OverlayMath.wavePhase(period / 2), eps)
        assertTrue(OverlayMath.wavePhase(period / 4) < OverlayMath.wavePhase(period / 2))
    }

    @Test
    fun haloBreathingSwellsToPeakMidCycleAndReturns() {
        val period = OverlayMath.HALO_BREATHE_PERIOD_MS
        assertEquals(0f, OverlayMath.breathing01(0, period), eps)
        assertEquals(1f, OverlayMath.breathing01(period / 2, period), eps)
        for (t in 0..period step 53) {
            val v = OverlayMath.breathing01(t, period)
            assertTrue("v=$v in [0,1]", v >= -eps && v <= 1f + eps)
        }
    }

    // --- cursor mover (vehicle steering) -------------------------------------

    private fun vehicle() =
        OverlayMath.Mover2D(
            maxSpeed = 3000f,
            accel = 18000f,
            turnRateRad = Math.toRadians(300.0).toFloat(),
            arriveRadius = 350f,
            homingRadius = 750f,
            homingBoost = 3.5f,
        )

    @Test
    fun cursorMoverFirstStepSnaps() {
        val m = vehicle()
        m.step(50f, 70f, 1f / 60f)
        assertEquals(50f, m.x, eps)
        assertEquals(70f, m.y, eps)
        assertEquals(0f, m.speed, eps)
    }

    @Test
    fun cursorMoverArrivesExactlyWithoutOvershoot() {
        val m = vehicle()
        m.snapTo(0f, 0f)
        var maxX = 0f
        repeat(400) {
            m.step(500f, 0f, 1f / 60f)
            if (m.x > maxX) maxX = m.x
        }
        // A straight move decelerates to a clean stop ON the target — never past it
        // (no spring-back), and lands exactly.
        assertTrue("overshot to $maxX", maxX <= 500f + eps)
        assertEquals(500f, m.x, eps)
        assertEquals(0f, m.speed, eps)
    }

    @Test
    fun cursorMoverCurvesOnDirectionChange() {
        // Build rightward momentum, then retarget straight DOWN from the turn point.
        // The bounded turn rate keeps it moving right while it swings to face down,
        // so the path bows out instead of being a straight vertical line.
        val m = vehicle()
        m.snapTo(0f, 0f)
        repeat(30) { m.step(4000f, 0f, 1f / 60f) }
        val xAtTurn = m.x
        repeat(15) { m.step(xAtTurn, 4000f, 1f / 60f) }
        assertTrue("expected rightward carry past $xAtTurn, got ${m.x}", m.x > xAtTurn + 1f)
        assertTrue("expected downward progress, got ${m.y}", m.y > 1f)
    }

    @Test
    fun cursorMoverHomingIsRequiredToSettleOffAxisApproach() {
        // The launch heading (reset to default = +x) points away from a target to
        // the left, forcing a wide curving approach. Without the near-target
        // turn-rate boost (homing) the turn radius stays larger than the remaining
        // distance, so the cursor orbits the point forever; with it the cursor curls
        // in and settles exactly. This asserts the boost is load-bearing, not merely
        // that the tuned mover happens to converge.
        val tx = 120f
        val ty = 470f

        // No homing (homingRadius/homingBoost default to 0): never settles — it
        // laps the target for the whole budget.
        val noHoming =
            OverlayMath.Mover2D(
                maxSpeed = 3000f,
                accel = 18000f,
                turnRateRad = Math.toRadians(300.0).toFloat(),
                arriveRadius = 350f,
            )
        noHoming.snapTo(500f, 500f)
        var settled = false
        repeat(2000) {
            noHoming.step(tx, ty, 1f / 60f)
            if (noHoming.speed == 0f && kotlin.math.hypot(noHoming.x - tx, noHoming.y - ty) < 2f) settled = true
        }
        assertTrue("without homing the cursor must orbit and never settle", !settled)

        // With homing (the production-shaped vehicle, same params + homing): curls
        // in and settles exactly on the target.
        val withHoming = vehicle()
        withHoming.snapTo(500f, 500f)
        repeat(600) { withHoming.step(tx, ty, 1f / 60f) } // ~10s of sim time at 60fps
        assertEquals("homing must converge: x", tx, withHoming.x, eps)
        assertEquals("homing must converge: y", ty, withHoming.y, eps)
        assertEquals(0f, withHoming.speed, eps)
    }

    @Test
    fun cursorMoverClampsHugeFrameSteps() {
        val m = vehicle()
        m.snapTo(0f, 0f)
        // A 10s "frame" must not fling the cursor; the step is clamped.
        m.step(500f, 0f, 10f)
        assertTrue("x must stay bounded, got ${m.x}", m.x.isFinite() && m.x in 0f..500f)
    }

    @Test
    fun cursorMoverNeverLeavesBounds() {
        val m = vehicle()
        m.setBounds(1000f, 2000f)
        m.snapTo(500f, 500f)
        // Aim far outside the screen; momentum must never carry it off-screen.
        repeat(200) { m.step(9000f, 9000f, 1f / 60f) }
        assertTrue("x off-screen: ${m.x}", m.x in 0f..1000f)
        assertTrue("y off-screen: ${m.y}", m.y in 0f..2000f)
    }

    @Test
    fun approachAngleTakesShortestPathAcrossTheWrap() {
        // 170 -> -170 is +20 the short way (through 180), not -340.
        assertEquals(175f, OverlayMath.approachAngleDeg(170f, -170f, 5f), eps)
    }

    @Test
    fun approachAngleClampsToMaxDelta() {
        assertEquals(10f, OverlayMath.approachAngleDeg(0f, 90f, 10f), eps)
    }

    @Test
    fun wrapRadiansFoldsFullTurnToZero() {
        assertEquals(0f, OverlayMath.wrapRadians((2.0 * Math.PI).toFloat()), 1e-3f)
    }

    @Test
    fun clickScaleSquashesToPressThenBouncesPastRest() {
        // Rests at full size at both ends so the pointer returns to normal.
        assertEquals(1f, OverlayMath.clickScale(0f), eps)
        assertEquals(1f, OverlayMath.clickScale(1f), eps)
        var minScale = 1f
        var maxScale = 1f
        for (i in 0..100) {
            val s = OverlayMath.clickScale(i / 100f)
            if (s < minScale) minScale = s
            if (s > maxScale) maxScale = s
        }
        // Squashes down to about the press scale, then overshoots past rest.
        assertTrue("min=$minScale should reach the press scale", minScale <= OverlayMath.CURSOR_PRESS_SCALE + 0.02f)
        assertTrue("min=$minScale should not undershoot the press scale", minScale >= OverlayMath.CURSOR_PRESS_SCALE - 0.02f)
        assertTrue("max=$maxScale should overshoot past 1.0 (the bounce)", maxScale > 1.02f)
    }

    @Test
    fun noNoWiggleStartsAndEndsLevelAndTiltsBothWays() {
        assertEquals(0f, OverlayMath.noNoWiggleDeg(0f), eps)
        assertEquals(0f, OverlayMath.noNoWiggleDeg(1f), eps)
        var sawLeft = false
        var sawRight = false
        for (i in 0..100) {
            val a = OverlayMath.noNoWiggleDeg(i / 100f)
            assertTrue("amplitude exceeded: $a", kotlin.math.abs(a) <= OverlayMath.NO_NO_WIGGLE_DEG + eps)
            if (a > 1f) sawRight = true
            if (a < -1f) sawLeft = true
        }
        assertTrue("wiggle should tilt both ways", sawLeft && sawRight)
    }

    // --- pulse glow -----------------------------------------------------------

    @Test
    fun pulseStartsBrightAndDecaysToBaseline() {
        val baseline = OverlayMath.GLOW_BASELINE_MAX
        val start = OverlayMath.pulseIntensity(0f, baseline)
        val end = OverlayMath.pulseIntensity(1f, baseline)
        assertEquals(OverlayMath.GLOW_PULSE_PEAK, start, eps)
        assertEquals(baseline, end, eps)
        // Monotonic-ish: mid is between baseline and peak.
        val mid = OverlayMath.pulseIntensity(0.5f, baseline)
        assertTrue(mid in baseline..OverlayMath.GLOW_PULSE_PEAK)
    }

    // --- ripple ---------------------------------------------------------------

    @Test
    fun rippleRadiusExpandsFromZeroToMax() {
        assertEquals(0f, OverlayMath.rippleRadius(0f, 100f), eps)
        assertEquals(100f, OverlayMath.rippleRadius(1f, 100f), eps)
        assertTrue(OverlayMath.rippleRadius(0.5f, 100f) > 0f)
    }

    @Test
    fun rippleAlphaFadesOut() {
        assertEquals(1f, OverlayMath.rippleAlpha(0f), eps)
        assertEquals(0f, OverlayMath.rippleAlpha(1f), eps)
        assertEquals(0.4f, OverlayMath.rippleAlpha(0.6f), eps)
    }

    // --- path sampling --------------------------------------------------------

    @Test
    fun pathLengthSumsSegments() {
        val pts =
            listOf(
                OverlayMath.Point(0f, 0f),
                OverlayMath.Point(3f, 4f), // 5
                OverlayMath.Point(3f, 4f + 10f), // 10
            )
        assertEquals(15f, OverlayMath.pathLength(pts), eps)
    }

    @Test
    fun pathLengthIsZeroForDegeneratePaths() {
        assertEquals(0f, OverlayMath.pathLength(emptyList()), eps)
        assertEquals(0f, OverlayMath.pathLength(listOf(OverlayMath.Point(5f, 5f))), eps)
    }

    @Test
    fun pointAtProgressReturnsEndpoints() {
        val pts = listOf(OverlayMath.Point(0f, 0f), OverlayMath.Point(10f, 0f))
        val start = OverlayMath.pointAtProgress(pts, 0f)
        val end = OverlayMath.pointAtProgress(pts, 1f)
        assertEquals(0f, start.x, eps)
        assertEquals(10f, end.x, eps)
    }

    @Test
    fun pointAtProgressInterpolatesByArcLength() {
        // A bent path where geometric midpoint != index midpoint. First segment
        // length 10, second segment length 30; total 40. At progress 0.5 we are
        // 20 along: 10 into the second segment, i.e. one-third of the way.
        val pts =
            listOf(
                OverlayMath.Point(0f, 0f),
                OverlayMath.Point(10f, 0f), // seg len 10
                OverlayMath.Point(40f, 0f), // seg len 30
            )
        val mid = OverlayMath.pointAtProgress(pts, 0.5f)
        assertEquals(20f, mid.x, eps)
    }

    @Test
    fun pointAtProgressClampsOutOfRange() {
        val pts = listOf(OverlayMath.Point(0f, 0f), OverlayMath.Point(10f, 0f))
        assertEquals(0f, OverlayMath.pointAtProgress(pts, -1f).x, eps)
        assertEquals(10f, OverlayMath.pointAtProgress(pts, 5f).x, eps)
    }

    @Test
    fun pointAtProgressHandlesZeroLengthPath() {
        val pts = listOf(OverlayMath.Point(7f, 7f), OverlayMath.Point(7f, 7f))
        val p = OverlayMath.pointAtProgress(pts, 0.5f)
        assertEquals(7f, p.x, eps)
        assertEquals(7f, p.y, eps)
    }

    // --- trail fade -----------------------------------------------------------

    @Test
    fun trailHeadIsBrightestAndTailFades() {
        // 5 samples; head (index 4) brightest, tail (index 0) darkest.
        val head = OverlayMath.trailAlpha(4, 5)
        val tail = OverlayMath.trailAlpha(0, 5)
        assertEquals(1f, head, eps)
        assertEquals(0f, tail, eps)
        assertTrue(OverlayMath.trailAlpha(2, 5) in 0f..1f)
    }

    @Test
    fun trailAlphaScalesWithHeadAlpha() {
        assertEquals(0.5f, OverlayMath.trailAlpha(4, 5, headAlpha = 0.5f), eps)
        // Single-sample path: clamp head alpha through.
        assertEquals(0.5f, OverlayMath.trailAlpha(0, 1, headAlpha = 0.5f), eps)
    }

    // --- capture state machine ------------------------------------------------

    @Test
    fun captureHideRestoreRoundTripsActiveState() {
        val state = OverlayMath.CaptureState()
        assertTrue(state.hide(currentlyActive = true))
        assertTrue(state.hidden)
        // Restore reports the overlay must come back.
        assertTrue(state.restore())
        assertFalse(state.hidden)
    }

    @Test
    fun captureRestoreReportsNothingWhenInactive() {
        val state = OverlayMath.CaptureState()
        state.hide(currentlyActive = false)
        // Nothing was active, so nothing to restore.
        assertFalse(state.restore())
        assertFalse(state.hidden)
    }

    @Test
    fun captureRestoreWithoutHideIsNoOp() {
        val state = OverlayMath.CaptureState()
        assertFalse(state.restore())
        assertFalse(state.hidden)
    }

    @Test
    fun captureNestedHidePreservesOriginalState() {
        val state = OverlayMath.CaptureState()
        assertTrue(state.hide(currentlyActive = true))
        // A second hide (e.g. concurrent capture) must not flip the saved state
        // to the now-hidden "inactive" reading.
        assertFalse(state.hide(currentlyActive = false))
        assertFalse(state.restore())
        assertTrue(state.hidden)
        assertTrue(state.restore())
        assertFalse(state.hidden)
    }

    // --- rotation / display-change bounds recompute ---------------------------

    @Test
    fun boundsChangedDetectsRotationSwap() {
        // Portrait -> landscape (a rotation swaps width and height): changed.
        assertTrue(OverlayBoundsMath.boundsChanged(1080, 2400, 2400, 1080))
        // Only one dimension changed (e.g. a foldable resize): still changed.
        assertTrue(OverlayBoundsMath.boundsChanged(1080, 2400, 1080, 1800))
    }

    @Test
    fun boundsChangedIgnoresNoOpCallback() {
        // A display callback for an unchanged size must not trigger a relayout.
        assertFalse(OverlayBoundsMath.boundsChanged(1080, 2400, 1080, 2400))
    }

    @Test
    fun clampTargetPullsOffScreenTargetIntoNewBounds() {
        // A portrait-valid y (2300) exceeds a landscape height (1080): pulled to the
        // edge so the cursor no longer chases an off-screen point.
        assertEquals(1080f, OverlayBoundsMath.clampTarget(2300f, 1080), eps)
        // Negative coordinates clamp to the top-left origin.
        assertEquals(0f, OverlayBoundsMath.clampTarget(-50f, 1080), eps)
    }

    @Test
    fun clampTargetLeavesInBoundsTargetUntouched() {
        // A coordinate already inside the new extent passes through unchanged.
        assertEquals(640f, OverlayBoundsMath.clampTarget(640f, 2400), eps)
    }
}
