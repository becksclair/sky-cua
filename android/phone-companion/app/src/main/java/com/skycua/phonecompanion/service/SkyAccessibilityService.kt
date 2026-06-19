package com.skycua.phonecompanion.service

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Base64
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import com.skycua.phonecompanion.overlay.AgentOverlayController
import com.skycua.phonecompanion.overlay.OverlayMath
import com.skycua.phonecompanion.overlay.WindowOverlayHost
import com.skycua.phonecompanion.protocol.AccessibilityNode
import com.skycua.phonecompanion.protocol.AccessibilityTreeParams
import com.skycua.phonecompanion.protocol.AccessibilityTreeResult
import com.skycua.phonecompanion.protocol.GestureKind
import com.skycua.phonecompanion.protocol.GestureParams
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.OverlayGestureParams
import com.skycua.phonecompanion.protocol.Protocol
import com.skycua.phonecompanion.protocol.ScreenshotParams
import com.skycua.phonecompanion.protocol.ScreenshotResult as ProtoScreenshotResult
import com.skycua.phonecompanion.screenshot.ScreenshotClassifier
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * The companion's AccessibilityService. It owns the phone-native cursor overlay,
 * gesture dispatch, accessibility-tree snapshots, and the accessibility
 * screenshot APIs.
 *
 * Screenshot capability is runtime-gated: `takeScreenshot` requires API 30+.
 * Capture uses the display-wide `takeScreenshot(Display.DEFAULT_DISPLAY, ...)`
 * path on all supported APIs, so a visible cursor overlay is included in the
 * frame; the result reports this via `contains_native_overlay`. An
 * overlay-excluding window capture (`takeScreenshotOfWindow`, API 34+) is not
 * yet implemented. All capability decisions are made from
 * `Build.VERSION.SDK_INT` and the resolved service info, never assumed.
 */
class SkyAccessibilityService : AccessibilityService() {
    private val mainHandler = Handler(Looper.getMainLooper())

    /**
     * The single full-screen agent overlay (glow + cursor + ripple/trail). Lazily
     * created on first use so the controller can resolve the window service from
     * the connected accessibility context.
     */
    private val overlay: AgentOverlayController by lazy {
        AgentOverlayController(
            context = this,
            host =
                WindowOverlayHost(
                    context = this,
                    windowType = WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                ),
            mainHandler = mainHandler,
        )
    }

    override fun onServiceConnected() {
        super.onServiceConnected()
        instanceRef.set(this)
    }

    override fun onDestroy() {
        instanceRef.compareAndSet(this, null)
        overlay.destroy()
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        // The companion is driven by RPC, not event streams; no per-event work.
    }

    override fun onInterrupt() {
        // No-op.
    }

    // --- capability flags -----------------------------------------------------

    fun canPerformGestures(): Boolean =
        capabilityEnabled(
            serviceInfo?.capabilities,
            AccessibilityServiceInfo.CAPABILITY_CAN_PERFORM_GESTURES,
        )

    fun canRetrieveWindowContent(): Boolean =
        capabilityEnabled(
            serviceInfo?.capabilities,
            AccessibilityServiceInfo.CAPABILITY_CAN_RETRIEVE_WINDOW_CONTENT,
        )

    fun canTakeScreenshot(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
            capabilityEnabled(
                serviceInfo?.capabilities,
                AccessibilityServiceInfo.CAPABILITY_CAN_TAKE_SCREENSHOT,
            )

    fun screenshotApiLevel(): Int = Build.VERSION.SDK_INT

    fun overlayPassThrough(): Boolean = overlay.isPassThrough()

    fun overlayVisible(): Boolean = overlay.isOverlayVisible()

    /** True while the persistent "agent in control" edge glow is active. */
    fun overlayGlowActive(): Boolean = overlay.isGlowActive()

    // --- agent overlay --------------------------------------------------------

    /**
     * Shows or hides the static agent cursor at device pixel (x, y). Backs the
     * `cursor_overlay` RPC by driving the single full-screen overlay's cursor
     * dot. Returns whether the overlay is shown and whether it is pass-through.
     */
    fun setCursorOverlay(visible: Boolean, x: Int, y: Int): Pair<Boolean, Boolean> =
        overlay.setCursor(visible, x, y)

    /**
     * Toggles the persistent breathing edge glow that signals the agent holds the
     * phone session. Returns whether the glow is active after the call.
     */
    fun setOverlayActive(active: Boolean): Boolean = overlay.setActive(active)

    /**
     * Animates the agent cursor for one action (visual only — no input is
     * dispatched). Returns whether the overlay was available to animate.
     */
    fun animateOverlayGesture(params: OverlayGestureParams): Boolean {
        val points = params.points.map { OverlayMath.Point(it.x.toFloat(), it.y.toFloat()) }
        return overlay.animateGesture(params.kind, points, params.durationMs)
    }

    // --- gesture dispatch -----------------------------------------------------

    /** Dispatches a tap or swipe gesture; throws on disabled capability or failure. */
    fun dispatchPhoneGesture(params: GestureParams) {
        if (!canPerformGestures()) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.DISABLED_SERVICE,
                "accessibility service cannot perform gestures",
            )
        }
        val path = Path()
        when (params.kind) {
            GestureKind.TAP -> {
                val p = params.points[0]
                path.moveTo(p.x.toFloat(), p.y.toFloat())
                path.lineTo(p.x.toFloat(), p.y.toFloat())
            }
            GestureKind.SWIPE -> {
                val start = params.points[0]
                val end = params.points[1]
                path.moveTo(start.x.toFloat(), start.y.toFloat())
                path.lineTo(end.x.toFloat(), end.y.toFloat())
            }
        }
        val stroke = GestureDescription.StrokeDescription(path, 0, params.durationMs)
        val description = GestureDescription.Builder().addStroke(stroke).build()

        val latch = CountDownLatch(1)
        val outcome = AtomicReference(false)
        val dispatched =
            dispatchGesture(
                description,
                object : GestureResultCallback() {
                    override fun onCompleted(gestureDescription: GestureDescription?) {
                        outcome.set(true)
                        latch.countDown()
                    }

                    override fun onCancelled(gestureDescription: GestureDescription?) {
                        outcome.set(false)
                        latch.countDown()
                    }
                },
                mainHandler,
            )
        if (!dispatched) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.TRANSIENT,
                "gesture dispatch was rejected",
            )
        }
        latch.await(5, TimeUnit.SECONDS)
        if (!outcome.get()) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.TRANSIENT,
                "gesture was cancelled",
            )
        }
    }

    // --- accessibility tree ---------------------------------------------------

    fun captureTree(params: AccessibilityTreeParams): AccessibilityTreeResult {
        if (!canRetrieveWindowContent()) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.DISABLED_SERVICE,
                "accessibility service cannot retrieve window content",
            )
        }
        val root =
            rootInActiveWindow
                ?: return AccessibilityTreeResult(null, null, emptyList(), truncated = false, redacted = false)

        val nodes = ArrayList<AccessibilityNode>()
        val truncated = collectNodes(root, params.maxNodes, nodes)
        val pkg = root.packageName?.toString()
        return AccessibilityTreeResult(
            packageName = pkg,
            activity = currentActivity(pkg),
            nodes = nodes,
            truncated = truncated,
            redacted = false,
        )
    }

    /**
     * Best-effort resolution of the foreground activity's flattened component
     * name (`pkg/activity`). Derived from the active window root node's class
     * name, which is only trustworthy as an activity component when it belongs
     * to the foreground package itself; system widget/layout root classes
     * (e.g. android.widget.FrameLayout) are not activities and must not be
     * fabricated into a component. Returns null gracefully when the package is
     * unknown, the root class is not package-owned, or window content is
     * unavailable. The package remains the primary foreground signal; this
     * never blocks or crashes.
     */
    fun currentActivity(packageName: String? = activeWindowPackage()): String? {
        val pkg = packageName ?: return null
        return try {
            val className = rootInActiveWindow?.className?.toString() ?: return null
            // Only emit a component when the root class is owned by the
            // foreground package and looks like a concrete (non-anonymous)
            // class. Otherwise the signal is unreliable, so leave activity null.
            if (!className.startsWith("$pkg.") || className.contains('$')) return null
            "$pkg/$className"
        } catch (_: Exception) {
            null
        }
    }

    /** The package owning the active window, or null when content is unavailable. */
    private fun activeWindowPackage(): String? =
        try {
            rootInActiveWindow?.packageName?.toString()
        } catch (_: Exception) {
            null
        }

    /** Breadth-first collection up to [max] nodes. Returns true when truncated. */
    private fun collectNodes(
        root: AccessibilityNodeInfo,
        max: Int,
        out: MutableList<AccessibilityNode>,
    ): Boolean {
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        while (queue.isNotEmpty()) {
            if (out.size >= max) return true
            val node = queue.removeFirst()
            val bounds = android.graphics.Rect()
            node.getBoundsInScreen(bounds)
            out.add(
                AccessibilityNode(
                    className = node.className?.toString(),
                    text = node.text?.toString(),
                    contentDesc = node.contentDescription?.toString(),
                    bounds = intArrayOf(bounds.left, bounds.top, bounds.right, bounds.bottom),
                    focusable = node.isFocusable,
                    enabled = node.isEnabled,
                    clickable = node.isClickable,
                ),
            )
            for (i in 0 until node.childCount) {
                node.getChild(i)?.let { queue.add(it) }
            }
        }
        return false
    }

    // --- screenshot -----------------------------------------------------------

    fun takePhoneScreenshot(params: ScreenshotParams): ProtoScreenshotResult {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.UNSUPPORTED_API,
                "accessibility screenshot requires API 30+",
            )
        }
        if (!canTakeScreenshot()) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.DISABLED_SERVICE,
                "accessibility service lacks screenshot capability",
            )
        }

        // When the caller wants a clean, model-facing frame, hide every overlay
        // pixel (glow + cursor + ripple + trail) for the duration of the capture
        // and restore the prior state afterward. The capture path is display-wide
        // (takeScreenshot of DEFAULT_DISPLAY), so a visible overlay would
        // otherwise bleed into the frame.
        val hideOverlay = !params.includeOverlay
        if (hideOverlay) {
            overlay.hideForCapture()
        }
        try {
            return captureScreenshot(includeOverlay = params.includeOverlay)
        } finally {
            if (hideOverlay) {
                overlay.restoreAfterCapture()
            }
        }
    }

    private fun captureScreenshot(includeOverlay: Boolean): ProtoScreenshotResult {
        val latch = CountDownLatch(1)
        val resultRef = AtomicReference<ProtoScreenshotResult>()
        val errorRef = AtomicReference<MethodApplicationException>()

        val executor = mainExecutor
        takeScreenshot(
            android.view.Display.DEFAULT_DISPLAY,
            executor,
            object : TakeScreenshotCallback {
                override fun onSuccess(screenshot: ScreenshotResult) {
                    try {
                        val hardwareBuffer = screenshot.hardwareBuffer
                        val colorSpace = screenshot.colorSpace
                        val bitmap =
                            try {
                                android.graphics.Bitmap.wrapHardwareBuffer(hardwareBuffer, colorSpace)
                                    ?: run {
                                        errorRef.set(
                                            MethodApplicationException(
                                                Protocol.ErrorCodes.TRANSIENT,
                                                "screenshot buffer could not be wrapped",
                                            ),
                                        )
                                        return
                                    }
                            } finally {
                                hardwareBuffer.close()
                            }
                        val software = bitmap.copy(android.graphics.Bitmap.Config.ARGB_8888, false)
                        bitmap.recycle()
                        val out = ByteArrayOutputStream()
                        software.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out)
                        val encoded = Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)
                        resultRef.set(
                            ProtoScreenshotResult(
                                mimeType = "image/png",
                                dataBase64 = encoded,
                                width = software.width,
                                height = software.height,
                                // When the overlay was hidden for this capture
                                // (includeOverlay=false), the frame is clean.
                                // Otherwise it reflects whatever was on screen.
                                containsNativeOverlay = includeOverlay && overlayVisible(),
                            ),
                        )
                        software.recycle()
                    } finally {
                        latch.countDown()
                    }
                }

                override fun onFailure(errorCode: Int) {
                    errorRef.set(ScreenshotClassifier.fromErrorCode(errorCode))
                    latch.countDown()
                }
            },
        )

        if (!latch.await(10, TimeUnit.SECONDS)) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.TRANSIENT,
                "screenshot timed out",
            )
        }
        errorRef.get()?.let { throw it }
        return resultRef.get()
            ?: throw MethodApplicationException(
                Protocol.ErrorCodes.TRANSIENT,
                "screenshot produced no result",
            )
    }

    companion object {
        private val instanceRef = AtomicReference<SkyAccessibilityService?>()

        fun instance(): SkyAccessibilityService? = instanceRef.get()

        fun capabilityEnabled(capabilities: Int?, capability: Int): Boolean =
            ((capabilities ?: 0) and capability) != 0
    }
}

// Type alias to the platform screenshot callback so the screenshot routine
// reads cleanly. The result and gesture-callback types are referenced through
// their inherited nested names (AccessibilityService.ScreenshotResult /
// GestureResultCallback) directly.
private typealias TakeScreenshotCallback = AccessibilityService.TakeScreenshotCallback
