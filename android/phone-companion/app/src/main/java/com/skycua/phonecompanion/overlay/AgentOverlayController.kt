package com.skycua.phonecompanion.overlay

import android.animation.ValueAnimator
import android.annotation.SuppressLint
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.Choreographer
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.animation.LinearInterpolator
import kotlin.math.abs
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Owns the single full-screen [AgentOverlayView] and drives the persistent
 * breathing edge glow, the per-gesture cursor/ripple/trail animations, the
 * static cursor position, and the screenshot hide/restore cycle.
 *
 * Where the view (and the optional "no-no" tap-catcher) are attached is delegated
 * to an [OverlayHost]: the production path hosts them as `WindowManager` overlay
 * windows ([WindowOverlayHost]); the in-app pointer playground hosts the overlay
 * directly in an Activity view tree ([ViewOverlayHost]). All tuned motion and
 * feedback below is host-agnostic and identical across both surfaces.
 *
 * All view and window mutations happen on the main thread. Callers from RPC
 * worker threads use the [runOnMain]/latch pattern (mirroring
 * `SkyAccessibilityService.setCursorOverlay`) so they observe a settled result.
 *
 * The controller never dispatches real input. Gesture animation is purely
 * visual; input is dispatched separately by the gesture RPC.
 */
class AgentOverlayController(
    private val context: Context,
    private val host: OverlayHost,
    private val mainHandler: Handler = Handler(Looper.getMainLooper()),
) {
    private var view: AgentOverlayView? = null

    /** True while the persistent "agent in control" glow is requested. */
    private var glowActive: Boolean = false

    /** True while a cursor is on screen (persists until disconnect). */
    private var cursorActive: Boolean = false

    // The cursor's drawn position chases this target every frame with momentum,
    // giving the "thruster in space" glide: a turn curves the path and the cursor
    // eases to a stop without overshooting. Taps, swipes, parking, and
    // repositioning all just move the target.
    private var cursorTargetX: Float = 0f
    private var cursorTargetY: Float = 0f
    private val density: Float = context.resources.displayMetrics.density
    private val rippleMaxPx: Float = RIPPLE_MAX_DP * density
    private val rippleMinPx: Float = RIPPLE_MIN_DP * density
    private val gestureArrivePx: Float = GESTURE_ARRIVE_DP * density

    // A gesture is held here until the cursor sails to its start point; the click
    // feedback (squash + ripple + bounce) then fires at the pointer's landing
    // point, so the ripple emanates from the pointer's own glow, not a distant target.
    private var pendingGesture: PendingGesture? = null

    private val cursorMover =
        OverlayMath.Mover2D(
            maxSpeed = OverlayMath.CURSOR_MAX_SPEED_DP_S * density,
            accel = OverlayMath.CURSOR_ACCEL_DP_S2 * density,
            turnRateRad = Math.toRadians(OverlayMath.CURSOR_TURN_RATE_DEG_S.toDouble()).toFloat(),
            arriveRadius = OverlayMath.CURSOR_ARRIVE_RADIUS_DP * density,
            homingRadius = OverlayMath.CURSOR_HOMING_RADIUS_DP * density,
            homingBoost = OverlayMath.CURSOR_HOMING_TURN_BOOST,
            defaultHeadingRad = Math.toRadians(OverlayMath.CURSOR_NOSE_DEG.toDouble()).toFloat(),
        )

    /** Drawn pointer rotation (deg); eased toward the heading, reset on arrival. */
    private var cursorRotationDeg: Float = 0f
    private var lastCursorFrameMs: Long = 0L

    // "No-no" easter egg: a tiny touchable catcher follows the cursor; a finger tap
    // on it (only while the agent is idle) makes the pointer click and shake. The
    // playground host catches finger taps itself, so [catcher] stays null there.
    private val catcherHalfPx: Int = (CATCHER_DP * density / 2f).toInt()
    private var catcher: CatcherHandle? = null
    private var catcherTouchable: Boolean = false
    private var lastGestureMs: Long = 0L
    private var wiggling: Boolean = false
    private var wiggleAnimator: ValueAnimator? = null

    /** Last full-display size used for the overlay window, for centring. */
    private var screenWidth: Int = 0
    private var screenHeight: Int = 0

    private var ambientAnimator: ValueAnimator? = null
    private var gestureAnimator: ValueAnimator? = null

    /** Pure capture state-machine; the view pixels are driven from its result. */
    private val captureState = OverlayMath.CaptureState()

    /** The "no-no" easter-egg blip; preloaded once, played on each head-shake. */
    private val noNoSound = NoNoSound(context)

    // --- predicates (read by the health report) ------------------------------

    /** True when the persistent glow is currently requested. */
    fun isGlowActive(): Boolean = glowActive

    /** True when the overlay window is attached. */
    fun isOverlayVisible(): Boolean = view != null

    /** True when the overlay flags are non-focusable and non-touchable. */
    fun isPassThrough(): Boolean = host.isPassThrough()

    // --- persistent glow ------------------------------------------------------

    /**
     * Shows or hides the persistent "agent in control" overlay. When activating,
     * the overlay is attached, a looping ambient animator drives the breathing
     * glow and the travelling wave, and the cursor is parked at screen centre so
     * the pointer is visible immediately. When deactivating (on host idle expiry
     * or disconnect), the animators stop and the window is removed. Idempotent;
     * the host owns the active lease and may relight it without reconnecting.
     *
     * @return whether the overlay is active after the call (always equals
     *   [active] when the view layer is available).
     */
    fun setActive(active: Boolean): Boolean {
        val latch = CountDownLatch(1)
        runOnMain {
            try {
                if (active) {
                    ensureView()
                    glowActive = true
                    startAmbient()
                    // Park the pointer at centre so it is visible immediately, not
                    // only after the first action. Snap (no glide) on first show; it
                    // then persists, and later moves sail with inertia.
                    if (!cursorActive) {
                        aimCursor(screenWidth / 2f, screenHeight / 2f, snap = true)
                    }
                } else {
                    glowActive = false
                    cursorActive = false
                    pendingGesture = null
                    stopAmbient()
                    view?.setGlowIntensity(0f)
                    view?.setCursor(false, 0f, 0f)
                    removeViewIfIdle()
                }
            } finally {
                latch.countDown()
            }
        }
        latch.await(2, TimeUnit.SECONDS)
        return glowActive
    }

    // --- per-gesture animation ------------------------------------------------

    /**
     * Animates the agent cursor for one action. Visual only — never dispatches
     * input. Moves the cursor to the first point; for `tap` triggers an expanding
     * fading ripple; for `swipe`/`drag` animates the cursor along the path
     * drawing a fading trail; pulses the glow brighter for the duration, then
     * returns to the breathing baseline. Fire-and-forget: returns promptly with
     * the view available.
     *
     * @return true when the overlay view is available to animate.
     */
    fun animateGesture(kind: String, points: List<OverlayMath.Point>, durationMs: Long): Boolean {
        if (points.isEmpty()) return false
        val available = CountDownLatch(1)
        var started = false
        runOnMain {
            try {
                ensureView()
                started = view != null
                if (started) {
                    // Cancel any in-flight feedback, aim the cursor at the gesture
                    // start, and hold the gesture: the squash/ripple/bounce fires once
                    // the cursor lands there (see the ambient loop), so the feedback
                    // happens at the pointer, not at a target it has not reached yet.
                    gestureAnimator?.cancel()
                    gestureAnimator = null
                    view?.clearRipple()
                    view?.clearTrail()
                    view?.setCursorScale(1f)
                    aimCursor(points[0].x, points[0].y, snap = false)
                    pendingGesture = PendingGesture(kind, points, durationMs)
                    lastGestureMs = SystemClock.uptimeMillis()
                    startAmbient()
                }
            } finally {
                available.countDown()
            }
        }
        available.await(2, TimeUnit.SECONDS)
        return started
    }

    // --- static cursor (backs cursor_overlay) --------------------------------

    /**
     * Sets the static agent cursor position, or hides it. Backs the legacy
     * `cursor_overlay` RPC. Returns (shown, passThrough).
     */
    fun setCursor(visible: Boolean, x: Int, y: Int): Pair<Boolean, Boolean> {
        val latch = CountDownLatch(1)
        var shown = false
        var passThrough = true
        runOnMain {
            try {
                if (visible) {
                    ensureView()
                    // Sail to the new position (normal pointer movement glides).
                    aimCursor(x.toFloat(), y.toFloat(), snap = false)
                    startAmbient()
                    shown = view != null
                    passThrough = isPassThrough()
                } else {
                    cursorActive = false
                    view?.setCursor(false, 0f, 0f)
                    shown = false
                    passThrough = true
                    removeViewIfIdle()
                }
            } finally {
                latch.countDown()
            }
        }
        latch.await(2, TimeUnit.SECONDS)
        return shown to passThrough
    }

    // --- screenshot hide / restore -------------------------------------------

    /**
     * Hides every overlay pixel (glow, cursor, ripple, trail) so a screenshot
     * taken after this returns is clean, and does not return until the cleared
     * frame has actually composited. Robust when nothing is shown. The prior
     * state is recorded for restoration.
     */
    fun hideForCapture() {
        val latch = CountDownLatch(1)
        runOnMain {
            captureState.hide(currentlyActive = glowActive)
            // Stop every source that could repaint a pixel during the capture
            // window: any in-flight gesture animation, then the breathing loop.
            // The gesture is cancelled first because its end-listener may restart
            // breathing; stopping breathing after guarantees nothing re-arms the
            // glow. Dropping the gesture tail is acceptable for a fire-and-forget
            // flourish and strictly better than a dirty model-facing frame.
            gestureAnimator?.cancel()
            gestureAnimator = null
            stopAmbient()
            val v = view
            if (v == null) {
                latch.countDown()
            } else {
                v.clearAll()
                // clearAll() only invalidates; the cleared frame composites on a
                // later vsync. Release the caller only after two frames so the
                // screenshot it takes next samples the already-composited clean
                // frame, not the still-dirty one.
                afterFrames(CAPTURE_BARRIER_FRAMES) { latch.countDown() }
            }
        }
        // Two vsyncs (~33ms at 60Hz) plus scheduling slack; the timeout is only a
        // backstop so a missed frame callback can never wedge the RPC worker.
        latch.await(2, TimeUnit.SECONDS)
    }

    /**
     * Restores whatever the overlay was showing before [hideForCapture]. Robust
     * when nothing was hidden (no-op).
     */
    fun restoreAfterCapture() {
        val latch = CountDownLatch(1)
        runOnMain {
            try {
                val shouldRestoreGlow = captureState.restore()
                if (shouldRestoreGlow) {
                    glowActive = true
                    ensureView()
                    startAmbient()
                } else if (cursorActive) {
                    ensureView()
                    startAmbient()
                } else {
                    removeViewIfIdle()
                }
            } finally {
                latch.countDown()
            }
        }
        latch.await(2, TimeUnit.SECONDS)
    }

    /** Tears down the overlay and all animators. Safe to call when idle. */
    fun destroy() {
        val latch = CountDownLatch(1)
        runOnMain {
            try {
                glowActive = false
                wiggling = false
                stopAmbient()
                gestureAnimator?.cancel()
                gestureAnimator = null
                wiggleAnimator?.cancel()
                wiggleAnimator = null
                removeView()
                noNoSound.release()
            } finally {
                latch.countDown()
            }
        }
        latch.await(2, TimeUnit.SECONDS)
    }

    // --- internals (main thread) ---------------------------------------------

    private fun ensureView() {
        if (view != null) return
        // Size the overlay to the full display bounds (including the status- and
        // navigation-bar regions and any display cutout) so the glow reaches the
        // true screen edges. The host resolves those bounds.
        val (w, h) = host.displayBounds()
        screenWidth = w
        screenHeight = h
        // Keep the cursor on-screen: momentum can never carry it past the edges.
        cursorMover.setBounds(screenWidth.toFloat(), screenHeight.toFloat())
        val v = AgentOverlayView(context)
        host.attachOverlay(v, screenWidth, screenHeight)
        view = v
        ensureCatcher()
        // Track rotation / external-display changes so the overlay window and the
        // cursor clamp follow the new dimensions instead of staying portrait-sized.
        host.registerDisplayListener { onDisplayChanged() }
    }

    /**
     * Re-resolves the display bounds after a rotation or display change and, when
     * the overlay is attached, resizes the overlay window and re-bounds the cursor
     * to the new dimensions. The glow/cursor active state is untouched — only the
     * window size and the clamp bounds move — and the cursor target is pulled back
     * inside the new bounds so a coordinate valid in the new orientation is no
     * longer clamped against stale (e.g. portrait) limits. Idempotent and a no-op
     * when nothing changed or the overlay is not attached.
     */
    fun onDisplayChanged() {
        runOnMain {
            val v = view ?: return@runOnMain
            val (w, h) = host.displayBounds()
            if (!OverlayBoundsMath.boundsChanged(screenWidth, screenHeight, w, h)) return@runOnMain
            screenWidth = w
            screenHeight = h
            // Resize the overlay window to the new full-display bounds so the edge
            // glow keeps hugging the true screen edges in the new orientation.
            host.resizeOverlay(v, w, h)
            // Re-bound the cursor and pull its target back inside the new bounds; a
            // landscape-valid coordinate must not stay clamped against portrait
            // limits. setBounds() also clamps the drawn position immediately.
            cursorMover.setBounds(w.toFloat(), h.toFloat())
            cursorTargetX = OverlayBoundsMath.clampTarget(cursorTargetX, w)
            cursorTargetY = OverlayBoundsMath.clampTarget(cursorTargetY, h)
        }
    }

    /**
     * Builds the small touchable "no-no" tap-catcher and hands it to the host to
     * attach near the cursor. Hosts that catch finger taps themselves (the
     * playground) return no handle, so the catcher is simply never driven.
     */
    @SuppressLint("ClickableViewAccessibility")
    private fun ensureCatcher() {
        if (catcher != null) return
        val size = catcherHalfPx * 2
        val v = View(context)
        val slop = ViewConfiguration.get(context).scaledTouchSlop
        val tapMs = ViewConfiguration.getLongPressTimeout().toLong()
        var downX = 0f
        var downY = 0f
        var downAt = 0L
        v.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    downX = event.rawX
                    downY = event.rawY
                    downAt = event.eventTime
                    true
                }
                MotionEvent.ACTION_UP -> {
                    val quick = event.eventTime - downAt <= tapMs
                    val still = abs(event.rawX - downX) <= slop && abs(event.rawY - downY) <= slop
                    if (quick && still) onPointerTapped()
                    true
                }
                else -> true
            }
        }
        catcher = host.attachCatcher(v, size, screenWidth / 2, screenHeight / 2)
        catcherTouchable = false
    }

    /**
     * Flips the catcher between touchable (catches a finger tap on the pointer) and
     * pass-through. It is only made touchable when the agent has been fully idle
     * for [CATCHER_IDLE_MS], so it never sits in the path of an agent action.
     */
    private fun setCatcherTouchable(on: Boolean) {
        if (on == catcherTouchable) return
        catcherTouchable = on
        catcher?.setTouchable(on)
    }

    /** Keeps the catcher centred on the drawn cursor. */
    private fun syncCatcher(cx: Float, cy: Float) {
        catcher?.moveTo((cx - catcherHalfPx).toInt(), (cy - catcherHalfPx).toInt())
    }

    private fun removeCatcher() {
        catcher?.detach()
        catcher = null
    }

    private fun removeView() {
        removeCatcher()
        val v = view ?: return
        // Stop tracking display changes before the window goes away so the host's
        // DisplayListener registration cannot outlive the overlay (leak guard).
        host.unregisterDisplayListener()
        host.detachOverlay(v)
        view = null
    }

    /**
     * Removes the overlay window only when nothing needs it anymore: no glow, no
     * running gesture animation, and no in-flight capture awaiting restore.
     */
    private fun removeViewIfIdle() {
        if (glowActive) return
        if (cursorActive) return
        if (gestureAnimator?.isRunning == true) return
        if (captureState.hidden) return
        stopAmbient()
        removeView()
    }

    private fun startAmbient() {
        if (ambientAnimator?.isRunning == true) return
        val start = SystemClock.uptimeMillis()
        lastCursorFrameMs = start
        val animator =
            ValueAnimator.ofFloat(0f, 1f).apply {
                duration = OverlayMath.WAVE_PERIOD_MS
                repeatCount = ValueAnimator.INFINITE
                repeatMode = ValueAnimator.RESTART
                interpolator = LinearInterpolator()
                addUpdateListener {
                    if (!glowActive && !cursorActive && pendingGesture == null) {
                        return@addUpdateListener
                    }
                    val now = SystemClock.uptimeMillis()
                    val elapsed = now - start
                    val dt = (now - lastCursorFrameMs).coerceAtLeast(0L).toFloat() / 1000f
                    lastCursorFrameMs = now
                    val v = view ?: return@addUpdateListener
                    // The inward waves and the cursor-halo breathing run
                    // continuously, including during a gesture; only the base glow
                    // breathing yields to the gesture's brighter pulse.
                    if (glowActive) {
                        v.setWavePhase(OverlayMath.wavePhase(elapsed))
                    }
                    v.setCursorPulse(OverlayMath.breathing01(elapsed))
                    // Glide the drawn cursor toward its target with inertia. The
                    // target is moved by parking, taps, and swipes; the spring does
                    // the sailing.
                    if (cursorActive) {
                        cursorMover.step(cursorTargetX, cursorTargetY, dt)
                        v.setCursor(true, cursorMover.x, cursorMover.y)
                        syncCatcher(cursorMover.x, cursorMover.y)
                        // The catcher catches a finger tap on the pointer only after a
                        // sustained idle, so it never intercepts the agent's own taps.
                        setCatcherTouchable(
                            pendingGesture == null &&
                                gestureAnimator?.isRunning != true &&
                                !wiggling &&
                                (now - lastGestureMs) > CATCHER_IDLE_MS,
                        )
                        // Nose into the travel heading while moving; ease back to the
                        // default (upright) orientation once it arrives and stops. The
                        // "no-no" wiggle owns the rotation while it plays.
                        if (!wiggling) {
                            val speedDp = cursorMover.speed / density
                            val targetRot =
                                if (speedDp > OverlayMath.CURSOR_ROTATE_MIN_SPEED_DP_S) {
                                    Math.toDegrees(cursorMover.headingRad.toDouble()).toFloat() -
                                        OverlayMath.CURSOR_NOSE_DEG
                                } else {
                                    0f
                                }
                            cursorRotationDeg =
                                OverlayMath.approachAngleDeg(
                                    cursorRotationDeg,
                                    targetRot,
                                    OverlayMath.CURSOR_ROTATE_RATE_DEG_S * dt,
                                )
                            v.setCursorRotation(cursorRotationDeg)
                        }
                        // Fire a held gesture's feedback once the cursor has sailed to
                        // and settled on its start point.
                        val pending = pendingGesture
                        if (pending != null && cursorMover.speed <= 0f) {
                            val dx = cursorMover.x - pending.points[0].x
                            val dy = cursorMover.y - pending.points[0].y
                            if (dx * dx + dy * dy <= gestureArrivePx * gestureArrivePx) {
                                pendingGesture = null
                                startGestureFeedback(pending)
                            }
                        }
                    }
                    if (glowActive && gestureAnimator?.isRunning != true) {
                        v.setGlowIntensity(OverlayMath.breathingIntensity(elapsed))
                    }
                }
            }
        ambientAnimator = animator
        animator.start()
    }

    private fun stopAmbient() {
        ambientAnimator?.cancel()
        ambientAnimator = null
    }

    /**
     * Plays the click feedback once the cursor has landed on the gesture start:
     * the pointer squashes and bounces back, its glow ripples at the landing
     * point, and for a swipe the cursor then sails along the path (trail
     * following) while the bounce-out stretches across the whole slide.
     */
    private fun startGestureFeedback(pending: PendingGesture) {
        gestureAnimator?.cancel()
        val v = view ?: return
        val points = pending.points
        val isTap = pending.kind == OverlayGestureKind.TAP || points.size < 2
        // The cursor is already at points[0] (it arrived); a tap stays put while a
        // swipe sails along the path. The press squash + bounce plays over the click
        // feedback for a tap and across the whole slide for a swipe (so the
        // bounce-out lasts as long as the slide). The ripple bursts at the start.
        val animDuration =
            if (isTap) {
                OverlayMath.CLICK_FEEDBACK_MS
            } else {
                // Sail the path slowly and deliberately, regardless of the brief
                // real-gesture duration.
                maxOf(OverlayMath.clampDurationMs(pending.durationMs), OverlayMath.SWIPE_VISUAL_MIN_MS)
            }
        val rippleX = points[0].x
        val rippleY = points[0].y
        val baseline =
            if (glowActive) OverlayMath.GLOW_BASELINE_MAX else OverlayMath.GLOW_BASELINE_MIN

        val animator =
            ValueAnimator.ofFloat(0f, 1f).apply {
                this.duration = animDuration
                interpolator = LinearInterpolator()
                addUpdateListener { anim ->
                    val progress = anim.animatedFraction
                    val current = view ?: return@addUpdateListener
                    // Pointer press squash + bounce, plus an edge-glow pulse.
                    current.setCursorScale(OverlayMath.clickScale(progress))
                    current.setGlowIntensity(OverlayMath.pulseIntensity(progress, baseline))
                    // Glow ripple: a fixed-length burst from the gesture start point.
                    val elapsedMs = progress * animDuration.toFloat()
                    val rippleProgress =
                        (elapsedMs / OverlayMath.RIPPLE_BURST_MS.toFloat()).coerceIn(0f, 1f)
                    if (rippleProgress < 1f) {
                        current.setRipple(
                            rippleX,
                            rippleY,
                            OverlayMath.rippleRadius(rippleProgress, rippleMaxPx, rippleMinPx),
                            OverlayMath.rippleAlpha(rippleProgress),
                        )
                    } else {
                        current.clearRipple()
                    }
                    // Swipe: sail the cursor along the path, the trail following.
                    if (!isTap) {
                        val head = OverlayMath.pointAtProgress(points, progress)
                        aimCursor(head.x, head.y, snap = false)
                        current.setTrail(sampleTrail(points, progress), alpha = 1f)
                    }
                }
            }
        animator.addListener(
            onEnd = {
                // Only clean up when this animator is still the active one; a
                // cancel triggered by a newer gesture must not wipe its state.
                if (gestureAnimator !== animator) return@addListener
                gestureAnimator = null
                lastGestureMs = SystemClock.uptimeMillis()
                val current = view
                current?.clearRipple()
                current?.clearTrail()
                current?.setCursorScale(1f)
                if (glowActive) {
                    // Resume the breathing baseline.
                    startAmbient()
                } else {
                    current?.setGlowIntensity(0f)
                    cursorActive = false
                    current?.setCursor(false, 0f, 0f)
                    removeViewIfIdle()
                }
            },
        )
        gestureAnimator = animator
        animator.start()
    }

    /**
     * Points the cursor at ([x], [y]). When [snap] (first placement or reconnect)
     * the springs jump there with no glide; otherwise the cursor sails to it with
     * inertia. Marks the cursor active so the ambient loop draws it.
     */
    private fun aimCursor(x: Float, y: Float, snap: Boolean) {
        cursorTargetX = x
        cursorTargetY = y
        if (snap) cursorMover.snapTo(x, y)
        cursorActive = true
    }

    /**
     * Public entry for a host that detects the finger tap on the pointer itself
     * (the in-app pointer playground): reacts with the "no-no" head-shake, subject
     * to the same idle guard as the production catcher path.
     */
    fun pokePointer() = onPointerTapped()

    /**
     * A finger tapped the pointer: react — but only while the agent is idle, so the
     * easter egg can never collide with an in-flight action.
     */
    private fun onPointerTapped() {
        runOnMain {
            if (!cursorActive || wiggling) return@runOnMain
            if (pendingGesture != null || gestureAnimator?.isRunning == true) return@runOnMain
            playNoNo()
        }
    }

    /** Plays the click feedback at the cursor plus a quick "no-no" head shake. */
    private fun playNoNo() {
        if (view == null) return
        noNoSound.play()
        startGestureFeedback(
            PendingGesture("tap", listOf(OverlayMath.Point(cursorMover.x, cursorMover.y)), 0L),
        )
        wiggleAnimator?.cancel()
        wiggling = true
        val animator =
            ValueAnimator.ofFloat(0f, 1f).apply {
                duration = OverlayMath.NO_NO_WIGGLE_MS
                interpolator = LinearInterpolator()
                addUpdateListener { anim ->
                    view?.setCursorRotation(OverlayMath.noNoWiggleDeg(anim.animatedFraction))
                }
            }
        animator.addListener(
            onEnd = {
                if (wiggleAnimator !== animator) return@addListener
                wiggling = false
                wiggleAnimator = null
                lastGestureMs = SystemClock.uptimeMillis()
                // The wiggle ends level; keep the eased rotation state in sync so the
                // ambient loop resumes without a jump.
                cursorRotationDeg = 0f
                view?.setCursorRotation(0f)
            },
        )
        wiggleAnimator = animator
        animator.start()
    }

    /**
     * Samples the swept portion of the path (start..head) into a small point
     * list for the trail. The list grows toward the full path as [progress]
     * approaches 1.
     */
    private fun sampleTrail(
        points: List<OverlayMath.Point>,
        progress: Float,
    ): List<OverlayMath.Point> {
        val samples = ArrayList<OverlayMath.Point>(TRAIL_SAMPLES)
        for (i in 0 until TRAIL_SAMPLES) {
            val frac = progress * (i.toFloat() / (TRAIL_SAMPLES - 1).toFloat())
            samples.add(OverlayMath.pointAtProgress(points, frac))
        }
        return samples
    }

    private fun runOnMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            block()
        } else {
            mainHandler.post(block)
        }
    }

    /**
     * Runs [action] on the main thread after [count] vsync frames have elapsed,
     * so a redraw scheduled just before (e.g. [AgentOverlayView.clearAll]) has
     * been composited. Must be called on the main thread (Choreographer requires
     * a Looper); the capture path always is.
     */
    private fun afterFrames(count: Int, action: () -> Unit) {
        if (count <= 0) {
            action()
            return
        }
        Choreographer.getInstance().postFrameCallback {
            afterFrames(count - 1, action)
        }
    }

    private object OverlayGestureKind {
        const val TAP = "tap"
    }

    companion object {
        /**
         * Number of trail samples used when building the swipe polyline. This is
         * an Android-specific rendering sampling count, not a cross-platform
         * visual constant, so it stays local rather than living in [OverlaySpec].
         */
        private const val TRAIL_SAMPLES: Int = 12

        /** Glow-ripple radius range in dp: it emanates from near the cursor halo
         * (min) and expands outward (max). Density-scaled into px at runtime. */
        private const val RIPPLE_MIN_DP: Float = OverlaySpec.Android.Geometry.RIPPLE_MIN_DP.toFloat()
        private const val RIPPLE_MAX_DP: Float = OverlaySpec.Android.Geometry.RIPPLE_MAX_DP.toFloat()

        /** Cursor must be within this distance (dp) of the gesture start to count as
         * arrived, so the click feedback fires at the pointer's landing point. */
        private const val GESTURE_ARRIVE_DP: Float = OverlaySpec.Android.Geometry.GESTURE_ARRIVE_DP.toFloat()

        /** Size (dp) of the touchable "no-no" tap-catcher that follows the cursor. */
        private const val CATCHER_DP: Float = OverlaySpec.Android.Geometry.CATCHER_DP.toFloat()

        /** The agent must be idle this long (ms) before the catcher becomes
         * touchable, so it can never intercept an agent action. */
        private const val CATCHER_IDLE_MS: Long = OverlaySpec.Shared.Timing.CATCHER_IDLE_MS

        /**
         * Frames to wait after clearing the overlay before a capture is allowed to
         * sample the display. One vsync starts the cleared frame; the second
         * guarantees it has been composited.
         */
        private const val CAPTURE_BARRIER_FRAMES: Int = OverlaySpec.Shared.Effects.CAPTURE_BARRIER_FRAMES
    }
}

/**
 * Pure, Android-free helpers for the rotation / display-change bounds recompute,
 * isolated here so the decision logic is unit-testable on a plain JVM (mirroring
 * how [OverlayMath] isolates the animation math). The controller drives the real
 * window/cursor mutations from these results.
 */
internal object OverlayBoundsMath {
    /**
     * True when the display dimensions actually changed, so the controller can skip
     * a redundant `updateViewLayout`/`setBounds` when a display callback fires for a
     * change that did not alter the overlay's size (e.g. a same-orientation event).
     */
    fun boundsChanged(oldWidth: Int, oldHeight: Int, newWidth: Int, newHeight: Int): Boolean =
        oldWidth != newWidth || oldHeight != newHeight

    /**
     * Clamps a cursor target coordinate into `[0, extent]` for the new display
     * extent (width or height). After a rotation the cursor mover is re-bounded to
     * the new dimensions; pulling the target in alongside it keeps the cursor from
     * chasing a now-off-screen point that the old (e.g. portrait) bounds permitted.
     */
    fun clampTarget(value: Float, extent: Int): Float = value.coerceIn(0f, extent.toFloat())
}

/**
 * A gesture held until the cursor sails to its start point, at which moment the
 * click feedback (squash + ripple + bounce, and the swipe slide) plays.
 */
private data class PendingGesture(
    val kind: String,
    val points: List<OverlayMath.Point>,
    val durationMs: Long,
)

/**
 * Minimal `Animator.AnimatorListener` adapter exposing only `onEnd`, so the
 * controller can resume breathing/cleanup without a verbose anonymous object.
 */
private fun ValueAnimator.addListener(onEnd: () -> Unit) {
    addListener(
        object : android.animation.Animator.AnimatorListener {
            override fun onAnimationEnd(animation: android.animation.Animator) = onEnd()

            override fun onAnimationCancel(animation: android.animation.Animator) = onEnd()

            override fun onAnimationStart(animation: android.animation.Animator) {}

            override fun onAnimationRepeat(animation: android.animation.Animator) {}
        },
    )
}
