package com.skycua.phonecompanion.overlay

/**
 * Pure, view-free math for the agent overlay animations and glow. It is isolated
 * here, free of Android `View`/`WindowManager`/`ValueAnimator` types, so the
 * timing, interpolation, and capture state-machine logic can be unit-tested on a
 * plain JVM, mirroring how [OverlayFlags] isolates its bitmask for testing.
 *
 * All coordinates are Android device pixels (post-rotation display pixels), the
 * same space gestures use. Nothing here allocates per-frame beyond the small
 * result objects the callers reuse.
 */
object OverlayMath {
    /** A device-pixel point used by the overlay path sampling. */
    data class Point(val x: Float, val y: Float)

    /**
     * All tunable constants below are forwarded from the generated [OverlaySpec]
     * so Android, desktop, and tests share one source of truth. Public names are
     * preserved for existing callers and tests; new code may use [OverlaySpec]
     * directly.
     */

    /** Minimum animation duration so a near-zero hint still reads as motion. */
    const val MIN_GESTURE_DURATION_MS: Long = OverlaySpec.Shared.Timing.MIN_GESTURE_DURATION_MS

    /** Maximum animation duration so an absurd hint cannot pin the overlay on. */
    const val MAX_GESTURE_DURATION_MS: Long = OverlaySpec.Shared.Timing.MAX_GESTURE_DURATION_MS

    /** Breathing-glow cycle period for the persistent "agent in control" state. */
    const val BREATHE_PERIOD_MS: Long = OverlaySpec.Shared.Timing.BREATHE_PERIOD_MS

    /** Resting glow intensity floor while breathing (stays strong, never dim). */
    const val GLOW_BASELINE_MIN: Float = OverlaySpec.Shared.Effects.GLOW_BASELINE_MIN_ALPHA_0_1.toFloat()

    /** Glow intensity ceiling while breathing. */
    const val GLOW_BASELINE_MAX: Float = OverlaySpec.Shared.Effects.GLOW_BASELINE_MAX_ALPHA_0_1.toFloat()

    /** Glow intensity while a gesture pulse is at its peak. */
    const val GLOW_PULSE_PEAK: Float = OverlaySpec.Shared.Effects.GLOW_PULSE_PEAK_ALPHA_0_1.toFloat()

    /** How long an inward wave takes to travel from the edge toward the centre. */
    const val WAVE_PERIOD_MS: Long = OverlaySpec.Shared.Timing.WAVE_PERIOD_MS

    /** Cursor-halo breathing period (independent of the glow breathing). */
    const val HALO_BREATHE_PERIOD_MS: Long = OverlaySpec.Shared.Timing.HALO_BREATHE_PERIOD_MS

    /** Cursor glide speed cap (dp/s). */
    const val CURSOR_MAX_SPEED_DP_S: Float = OverlaySpec.Shared.Motion.CURSOR_MAX_SPEED_DP_PER_S.toFloat()

    /** Cursor forward acceleration/deceleration (dp/s^2). */
    const val CURSOR_ACCEL_DP_S2: Float = OverlaySpec.Shared.Motion.CURSOR_ACCEL_DP_PER_S2.toFloat()

    /**
     * Max heading turn rate (deg/s). The cursor steers its nose toward the target
     * at this rate and thrusts forward along it, so it cannot turn instantly — a
     * direction change carries momentum and curves the path. Lower = wider curves.
     */
    const val CURSOR_TURN_RATE_DEG_S: Float = OverlaySpec.Shared.Motion.CURSOR_TURN_RATE_DEG_PER_S.toFloat()

    /**
     * Distance (dp) within which the cursor decelerates to a clean stop at the
     * target — the speed ramps to zero on arrival, so it eases in and never
     * overshoots or springs back.
     */
    const val CURSOR_ARRIVE_RADIUS_DP: Float = OverlaySpec.Shared.Motion.CURSOR_ARRIVE_RADIUS_DP.toFloat()

    /**
     * Distance (dp) within which the cursor's turn rate ramps up so it curls
     * tightly into the target instead of orbiting it. Outside this radius the wide
     * cruise turn rate (and the whole launch/cruise curve) is untouched.
     */
    const val CURSOR_HOMING_RADIUS_DP: Float = OverlaySpec.Shared.Motion.CURSOR_HOMING_RADIUS_DP.toFloat()

    /**
     * Peak extra turn-rate multiplier at the target (the cursor can turn this much
     * harder than the cruise rate as it arrives). Large enough that the turn radius
     * drops well below the remaining distance across the homing zone, so an
     * off-axis approach from any angle spirals in within a fraction of a turn
     * rather than circling the point.
     */
    const val CURSOR_HOMING_TURN_BOOST: Float = OverlaySpec.Shared.Motion.CURSOR_HOMING_TURN_BOOST.toFloat()

    /** Largest integration step honoured, so a long frame cannot fling the cursor. */
    const val CURSOR_MAX_STEP_S: Float = OverlaySpec.Shared.Motion.CURSOR_MAX_STEP_S.toFloat()

    /** Within this distance (px) the cursor settles exactly on the target and stops. */
    const val CURSOR_SETTLE_PX: Float = OverlaySpec.Shared.Motion.CURSOR_SETTLE_PX.toFloat()

    /**
     * The screen heading (degrees) the cursor image's nose points at zero
     * rotation. The `cursor-chat` pointer's tip aims up-and-left, i.e. roughly
     * -135 degrees in screen coordinates (x right, y down). Calibrated visually.
     */
    const val CURSOR_NOSE_DEG: Float = OverlaySpec.Shared.Motion.CURSOR_NOSE_DEG.toFloat()

    /** Above this speed (dp/s) the cursor noses into its heading; below it, resets. */
    const val CURSOR_ROTATE_MIN_SPEED_DP_S: Float = OverlaySpec.Shared.Motion.CURSOR_ROTATE_MIN_SPEED_DP_PER_S.toFloat()

    /** How fast the drawn cursor rotation eases toward its target (deg/s). */
    const val CURSOR_ROTATE_RATE_DEG_S: Float = OverlaySpec.Shared.Motion.CURSOR_ROTATE_RATE_DEG_PER_S.toFloat()

    /** Tap press-feedback duration (ms): squash + ripple + bounce for a tap. */
    const val CLICK_FEEDBACK_MS: Long = OverlaySpec.Shared.Timing.CLICK_FEEDBACK_MS

    /**
     * Glow-ripple burst duration (ms): the ripple expands and fades over this from
     * the gesture start, for both taps and swipes — for a swipe it is independent
     * of (and shorter than) the slide, so the ripple "happens when the gesture
     * begins" while the squash/bounce stretches across the whole slide.
     */
    const val RIPPLE_BURST_MS: Long = OverlaySpec.Shared.Timing.RIPPLE_BURST_MS

    /** Pointer scale at the bottom of the press squash. */
    const val CURSOR_PRESS_SCALE: Float = OverlaySpec.Shared.Effects.CURSOR_PRESS_SCALE_FRACTION.toFloat()

    /**
     * Minimum visual duration (ms) for a swipe slide. The real gesture is brief,
     * but the cursor sails the path slowly and deliberately so the motion reads;
     * the squash bounce-out stretches across this whole duration.
     */
    const val SWIPE_VISUAL_MIN_MS: Long = OverlaySpec.Shared.Timing.SWIPE_VISUAL_MIN_MS

    /** Fraction of the feedback spent squashing down before the bounce-back. */
    private const val PRESS_IN: Float = OverlaySpec.Shared.Effects.PRESS_IN_FRACTION.toFloat()

    /** Bounce damping; lower = bigger overshoot on the way back to 1.0. */
    private const val BOUNCE_DAMP: Float = OverlaySpec.Shared.Effects.BOUNCE_DAMP.toFloat()

    /** Bounce frequency; PI*1.5 lands the bounce exactly at scale 1.0 at the end. */
    private val BOUNCE_OMEGA: Float = (OverlaySpec.Shared.Effects.BOUNCE_OMEGA_PI_FRACTION * Math.PI).toFloat()

    /** "No-no" head-shake duration (ms) when the operator taps the pointer. */
    const val NO_NO_WIGGLE_MS: Long = OverlaySpec.Shared.Timing.NO_NO_WIGGLE_MS

    /** Peak left/right turn (degrees) of the "no-no" head shake. */
    const val NO_NO_WIGGLE_DEG: Float = OverlaySpec.Shared.Effects.NO_NO_WIGGLE_DEG.toFloat()

    /** Number of full left-right turns; 1.5 = turn one way, the other, then back. */
    private const val NO_NO_SHAKES: Float = OverlaySpec.Shared.Effects.NO_NO_SHAKES_FRACTION.toFloat()

    /** Fraction held at full amplitude before easing out to centre at the end. */
    private const val NO_NO_HOLD: Float = OverlaySpec.Shared.Effects.NO_NO_HOLD_FRACTION.toFloat()

    /**
     * "No-no" head-shake rotation (degrees) at normalized [progress]: a deliberate
     * left-right turn of up to ±[amplitudeDeg] that begins and ends at 0. Unlike a
     * decaying jiggle, each turn reaches full amplitude; the motion only eases out
     * to centre over the final stretch, so it reads as a head turning side to side.
     */
    fun noNoWiggleDeg(progress: Float, amplitudeDeg: Float = NO_NO_WIGGLE_DEG): Float {
        val p = clamp01(progress)
        val envelope = if (p < NO_NO_HOLD) 1f else easeInOut((1f - p) / (1f - NO_NO_HOLD))
        return amplitudeDeg * envelope * Math.sin((2.0 * Math.PI * NO_NO_SHAKES * p)).toFloat()
    }

    /** Wraps an angle (radians) to (-PI, PI]. */
    fun wrapRadians(angle: Float): Float {
        val twoPi = (2.0 * Math.PI).toFloat()
        var a = angle % twoPi
        if (a <= -Math.PI) a += twoPi
        if (a > Math.PI) a -= twoPi
        return a
    }

    /**
     * Moves [current] (degrees) toward [target] (degrees) by at most [maxDelta]
     * along the shortest angular path, returning the new angle wrapped to
     * (-180, 180]. Used to ease the cursor's drawn rotation.
     */
    fun approachAngleDeg(current: Float, target: Float, maxDelta: Float): Float {
        var diff = (target - current) % 360f
        if (diff < -180f) diff += 360f
        if (diff > 180f) diff -= 360f
        val step = diff.coerceIn(-maxDelta, maxDelta)
        var result = (current + step) % 360f
        if (result <= -180f) result += 360f
        if (result > 180f) result -= 360f
        return result
    }

    /**
     * Clamps a requested animation duration into the supported window. A
     * non-positive or tiny hint is raised to [MIN_GESTURE_DURATION_MS]; an
     * oversized hint is capped at [MAX_GESTURE_DURATION_MS].
     */
    fun clampDurationMs(requestedMs: Long): Long =
        when {
            requestedMs < MIN_GESTURE_DURATION_MS -> MIN_GESTURE_DURATION_MS
            requestedMs > MAX_GESTURE_DURATION_MS -> MAX_GESTURE_DURATION_MS
            else -> requestedMs
        }

    /** Clamps an arbitrary float to the inclusive [0, 1] range. */
    fun clamp01(value: Float): Float =
        when {
            value < 0f -> 0f
            value > 1f -> 1f
            else -> value
        }

    /**
     * Smooth ease-in/ease-out (smoothstep) over a normalized [0, 1] input,
     * used to shape the breathing curve and pulse falloff without per-frame
     * trig. Inputs outside [0, 1] are clamped first.
     */
    fun easeInOut(t: Float): Float {
        val x = clamp01(t)
        return x * x * (3f - 2f * x)
    }

    /**
     * Breathing glow intensity at [elapsedMs] into a continuous loop. Produces a
     * smooth triangle-eased oscillation between [GLOW_BASELINE_MIN] and
     * [GLOW_BASELINE_MAX] with period [BREATHE_PERIOD_MS]. Deterministic and
     * allocation-free so it can drive `onDraw` and be unit-tested directly.
     */
    fun breathingIntensity(elapsedMs: Long, periodMs: Long = BREATHE_PERIOD_MS): Float {
        if (periodMs <= 0L) return GLOW_BASELINE_MAX
        val phase = Math.floorMod(elapsedMs, periodMs).toFloat() / periodMs.toFloat()
        // Triangle wave 0 -> 1 -> 0 across the period, then eased for a soft turn.
        val triangle = if (phase < 0.5f) phase * 2f else (1f - phase) * 2f
        val eased = easeInOut(triangle)
        return GLOW_BASELINE_MIN + (GLOW_BASELINE_MAX - GLOW_BASELINE_MIN) * eased
    }

    /**
     * Wave phase in [0, 1) at [elapsedMs] into a continuous loop: the normalized
     * depth of an inward wave (0 at the edge, approaching 1 at full travel),
     * wrapping every [WAVE_PERIOD_MS]. Deterministic and allocation-free.
     */
    fun wavePhase(elapsedMs: Long, periodMs: Long = WAVE_PERIOD_MS): Float {
        if (periodMs <= 0L) return 0f
        return Math.floorMod(elapsedMs, periodMs).toFloat() / periodMs.toFloat()
    }

    /**
     * Normalized eased breathing in [0, 1] at [elapsedMs] for a [periodMs] cycle:
     * a smooth 0 -> 1 -> 0 swell. Used to breathe the cursor halo independently of
     * the glow. Deterministic and allocation-free.
     */
    fun breathing01(elapsedMs: Long, periodMs: Long = HALO_BREATHE_PERIOD_MS): Float {
        if (periodMs <= 0L) return 1f
        val phase = Math.floorMod(elapsedMs, periodMs).toFloat() / periodMs.toFloat()
        val triangle = if (phase < 0.5f) phase * 2f else (1f - phase) * 2f
        return easeInOut(triangle)
    }

    /**
     * Glow intensity during a gesture pulse. The glow rises quickly to
     * [GLOW_PULSE_PEAK] then eases back toward the breathing baseline as the
     * pulse completes. [progress] is the pulse's normalized [0, 1] position.
     */
    fun pulseIntensity(progress: Float, baseline: Float): Float {
        val p = clamp01(progress)
        // Fast attack, slow release: peak near the start, decay to baseline.
        val decay = easeInOut(1f - p)
        return baseline + (GLOW_PULSE_PEAK - baseline) * decay
    }

    /**
     * Tap-ripple radius at normalized [progress], expanding from a small inner
     * radius to [maxRadius]. The expansion is eased so the ring slows as it
     * fades, matching the desktop agent-cursor ripple feel.
     */
    fun rippleRadius(progress: Float, maxRadius: Float, minRadius: Float = 0f): Float {
        val eased = easeInOut(clamp01(progress))
        return minRadius + (maxRadius - minRadius) * eased
    }

    /**
     * Tap-ripple alpha at normalized [progress]: fully opaque at the start,
     * linearly fading to transparent as the ring expands. Returned in [0, 1].
     */
    fun rippleAlpha(progress: Float): Float = clamp01(1f - clamp01(progress))

    /**
     * Pointer scale over a click/slide feedback at normalized [progress]: it
     * squashes quickly from 1 down to [CURSOR_PRESS_SCALE], then springs back to 1
     * with a single overshoot bounce (briefly past 1.0 before settling). Used for
     * both the tap (over [CLICK_FEEDBACK_MS]) and the swipe (stretched across the
     * slide duration). Returns 1.0 at both ends, so the pointer rests at full size.
     */
    fun clickScale(progress: Float): Float {
        val p = clamp01(progress)
        val depth = 1f - CURSOR_PRESS_SCALE
        if (p < PRESS_IN) {
            // Fast squash down: 1 -> CURSOR_PRESS_SCALE.
            return 1f - depth * easeInOut(p / PRESS_IN)
        }
        // Spring back up with one overshoot, ending exactly at 1.0.
        val t = (p - PRESS_IN) / (1f - PRESS_IN)
        val envelope = Math.exp((-BOUNCE_DAMP * t).toDouble()).toFloat()
        val wave = envelope * Math.cos((BOUNCE_OMEGA * t).toDouble()).toFloat()
        return 1f - depth * wave
    }

    /**
     * Trail alpha for a sample at index [sampleIndex] of [sampleCount] along a
     * swipe/drag path, where the head (largest index) is brightest and older
     * samples fade. [headAlpha] scales the whole trail (e.g. as the gesture
     * itself fades out at the end). Returned in [0, 1].
     */
    fun trailAlpha(sampleIndex: Int, sampleCount: Int, headAlpha: Float = 1f): Float {
        if (sampleCount <= 1) return clamp01(headAlpha)
        val fraction = sampleIndex.toFloat() / (sampleCount - 1).toFloat()
        return clamp01(headAlpha) * clamp01(fraction)
    }

    /**
     * Total length of a polyline through [points] in device pixels. Returns 0
     * for fewer than two points.
     */
    fun pathLength(points: List<Point>): Float {
        if (points.size < 2) return 0f
        var total = 0f
        for (i in 1 until points.size) {
            total += distance(points[i - 1], points[i])
        }
        return total
    }

    /**
     * Position along the polyline [points] at normalized [progress] in [0, 1],
     * interpolated by arc length so motion is even regardless of how the input
     * points are spaced. Returns the first point for an empty-ish path and the
     * last point at progress >= 1.
     */
    fun pointAtProgress(points: List<Point>, progress: Float): Point {
        if (points.isEmpty()) return Point(0f, 0f)
        if (points.size == 1) return points[0]
        val p = clamp01(progress)
        if (p <= 0f) return points.first()
        if (p >= 1f) return points.last()

        val total = pathLength(points)
        if (total <= 0f) return points.first()

        val target = total * p
        var travelled = 0f
        for (i in 1 until points.size) {
            val seg = distance(points[i - 1], points[i])
            if (seg <= 0f) continue
            if (travelled + seg >= target) {
                val localT = (target - travelled) / seg
                return lerp(points[i - 1], points[i], localT)
            }
            travelled += seg
        }
        return points.last()
    }

    /** Linear interpolation between two points at [t] in [0, 1]. */
    fun lerp(a: Point, b: Point, t: Float): Point {
        val x = clamp01(t)
        return Point(a.x + (b.x - a.x) * x, a.y + (b.y - a.y) * x)
    }

    /** Euclidean distance between two points. */
    fun distance(a: Point, b: Point): Float {
        val dx = b.x - a.x
        val dy = b.y - a.y
        return Math.sqrt((dx * dx + dy * dy).toDouble()).toFloat()
    }

    /**
     * A vehicle-steering "mover" for the agent cursor — a little thruster in space.
     * It keeps a heading and a forward speed: each step it turns its heading toward
     * the target at a bounded rate and thrusts forward, so it cannot pivot
     * instantly. A direction change therefore carries momentum and bends the path
     * into a curve instead of a straight line. The forward speed ramps to zero
     * inside [arriveRadius], so it eases to a clean stop at the target and never
     * overshoots or springs back. Holds its own position/heading/speed;
     * allocation-free per step and unit-testable without any Android types.
     *
     * @param maxSpeed glide-speed cap (px/s).
     * @param accel forward acceleration/deceleration (px/s^2).
     * @param turnRateRad max heading change rate (rad/s); lower = wider curves.
     * @param arriveRadius distance (px) within which speed ramps down to a stop.
     * @param homingRadius distance (px) within which the turn rate ramps up so the
     *   cursor curls tightly into the target; outside it the wide cruise turn rate
     *   is untouched.
     * @param homingBoost peak extra turn-rate multiplier at the target (0 = off).
     *   Without it a wide-curve approach whose turn radius exceeds the remaining
     *   distance can never curve inward and orbits the point forever; ramping the
     *   turn rate up near the target shrinks the turn radius below the distance so
     *   the heading out-turns the bearing sweep and spirals in to settle.
     * @param defaultHeadingRad the heading the cursor resets to when it parks, so
     *   the next move launches along the pointer's resting (diagonal) nose and
     *   curves toward the target rather than firing straight at it.
     */
    class Mover2D(
        private val maxSpeed: Float,
        private val accel: Float,
        private val turnRateRad: Float,
        private val arriveRadius: Float,
        private val homingRadius: Float = 0f,
        private val homingBoost: Float = 0f,
        private val defaultHeadingRad: Float = 0f,
    ) {
        var x: Float = 0f
            private set
        var y: Float = 0f
            private set

        /** Current heading in radians (0 = +x / right, PI/2 = +y / down). */
        var headingRad: Float = 0f
            private set

        /** Current forward speed in px/s. */
        var speed: Float = 0f
            private set

        private var initialized: Boolean = false
        private var maxX: Float = Float.MAX_VALUE
        private var maxY: Float = Float.MAX_VALUE

        /**
         * Constrains the cursor to `[0, width] x [0, height]` so momentum can never
         * carry it off-screen. Clamps the current position immediately too.
         */
        fun setBounds(width: Float, height: Float) {
            maxX = width
            maxY = height
            x = x.coerceIn(0f, maxX)
            y = y.coerceIn(0f, maxY)
        }

        /** Jumps to ([tx], [ty]), stops, and resets heading to the resting nose. */
        fun snapTo(tx: Float, ty: Float) {
            x = tx.coerceIn(0f, maxX)
            y = ty.coerceIn(0f, maxY)
            speed = 0f
            headingRad = defaultHeadingRad
            initialized = true
        }

        /**
         * Advances toward ([tx], [ty]) by [dtSeconds]. The first call snaps. The
         * step is clamped to [CURSOR_MAX_STEP_S] so a stalled frame cannot fling
         * the cursor, and a step that would pass the target lands exactly on it.
         */
        fun step(tx: Float, ty: Float, dtSeconds: Float) {
            if (!initialized) {
                snapTo(tx, ty)
                return
            }
            val dt = dtSeconds.coerceIn(0f, CURSOR_MAX_STEP_S)
            if (dt <= 0f) return
            val dx = tx - x
            val dy = ty - y
            val dist = Math.sqrt((dx * dx + dy * dy).toDouble()).toFloat()
            if (dist <= CURSOR_SETTLE_PX) {
                // Close enough: settle exactly on the target and stop, and reset the
                // heading to the resting nose so the next move launches along the
                // pointer's diagonal and curves toward its target.
                x = tx
                y = ty
                speed = 0f
                headingRad = defaultHeadingRad
                return
            }
            run {
                // Steer the heading toward the target, bounded by the turn rate. The
                // turn rate ramps up within the homing radius so the cursor curls
                // tightly into the target instead of orbiting it: far away the wide
                // cruise rate is untouched, but near the target the shrinking turn
                // radius lets the heading out-turn the bearing sweep and spiral in.
                val homing =
                    if (homingRadius > 0f && dist < homingRadius) {
                        homingBoost * (1f - dist / homingRadius)
                    } else {
                        0f
                    }
                val targetAngle = Math.atan2(dy.toDouble(), dx.toDouble()).toFloat()
                val maxTurn = turnRateRad * (1f + homing) * dt
                val turn = wrapRadians(targetAngle - headingRad).coerceIn(-maxTurn, maxTurn)
                headingRad = wrapRadians(headingRad + turn)
            }
            // Forward speed ramps down on arrival so the cursor decelerates in.
            val desiredSpeed = if (dist < arriveRadius) maxSpeed * (dist / arriveRadius) else maxSpeed
            val ds = (desiredSpeed - speed).coerceIn(-accel * dt, accel * dt)
            speed = (speed + ds).coerceAtLeast(0f)
            val stepLen = speed * dt
            if (stepLen >= dist) {
                // Would reach/pass the target this frame: land exactly on it.
                x = tx
                y = ty
                speed = 0f
                return
            }
            // Integrate, clamped to the screen so momentum never carries it off.
            x = (x + Math.cos(headingRad.toDouble()).toFloat() * stepLen).coerceIn(0f, maxX)
            y = (y + Math.sin(headingRad.toDouble()).toFloat() * stepLen).coerceIn(0f, maxY)
        }
    }

    /**
     * Pure state machine for the screenshot hide/restore cycle. It records what
     * the overlay was showing before a capture so the controller can restore it
     * afterward, and is robust to being asked to hide when nothing is shown and
     * to restore when nothing was hidden (idempotent in both directions).
     *
     * This holds only data (no View/WindowManager), so the controller drives the
     * real pixels while this class is unit-tested directly.
     */
    class CaptureState {
        /** Whether the overlay was visible before the most recent [hide]. */
        var savedActive: Boolean = false
            private set

        /** Whether a hide is currently in effect (awaiting [restore]). */
        var hidden: Boolean = false
            private set

        private var hideDepth: Int = 0

        /**
         * Records the pre-capture visibility and marks the overlay hidden.
         * Depth-counted: a second hide without an intervening restore keeps the
         * originally-saved state and extends the hidden interval until every
         * overlapping capture restores.
         *
         * @param currentlyActive whether the overlay is showing right now.
         * @return true if this call transitioned from shown to hidden.
         */
        fun hide(currentlyActive: Boolean): Boolean {
            if (hideDepth > 0) {
                hideDepth += 1
                return false
            }
            savedActive = currentlyActive
            hideDepth = 1
            hidden = true
            return true
        }

        /**
         * Clears the hidden flag and reports whether the overlay should be shown
         * again. Idempotent: restoring when nothing was hidden returns false and
         * changes nothing. For overlapping captures, only the final restore ends
         * the hidden interval and may restart the overlay.
         *
         * @return true when the overlay was active before hiding and must be
         *   restored; false otherwise.
         */
        fun restore(): Boolean {
            if (hideDepth <= 0) return false
            hideDepth -= 1
            if (hideDepth > 0) return false
            hidden = false
            val shouldRestore = savedActive
            savedActive = false
            return shouldRestore
        }
    }
}
