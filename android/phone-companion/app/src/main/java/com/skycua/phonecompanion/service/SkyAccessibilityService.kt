package com.skycua.phonecompanion.service

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Base64
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import com.skycua.phonecompanion.overlay.AgentOverlayController
import com.skycua.phonecompanion.direct.DirectLinkServiceOwner
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
import com.skycua.phonecompanion.appshot.PhoneAppShot
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.protocol.MethodParamException
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.atomic.AtomicLong

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
        if (!DirectLinkServiceOwner.acquireAccessibility(applicationContext)) {
            // Android may deny a background foreground-service cold start. The
            // owner exposes START_DENIED to MainActivity, where the operator
            // gets an explicit retry action; this service does not pretend the
            // link is running or retry autonomously.
        }
    }

    override fun onDestroy() {
        instanceRef.compareAndSet(this, null)
        DirectLinkServiceOwner.releaseAccessibility(applicationContext)
        overlay.destroy()
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        accessibilityEventSequenceRef.incrementAndGet()
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

    /** Monotonic fence used to detect UI changes during an AppShot capture. */
    fun accessibilityEventSequence(): Long = accessibilityEventSequenceRef.get()

    fun activePackageName(): String? = activeWindowPackage()

    fun displayState(): PhoneAppShot.DisplayState {
        val display = getSystemService(android.hardware.display.DisplayManager::class.java)
            ?.getDisplay(android.view.Display.DEFAULT_DISPLAY)
        val metrics = android.util.DisplayMetrics()
        display?.getRealMetrics(metrics)
        return PhoneAppShot.DisplayState(
            displayId = display?.displayId ?: android.view.Display.DEFAULT_DISPLAY,
            width = metrics.widthPixels,
            height = metrics.heightPixels,
            rotation = display?.rotation ?: android.view.Surface.ROTATION_0,
            state = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) display?.state ?: android.view.Display.STATE_UNKNOWN else 0,
            density = metrics.density,
        )
    }

    /** Captures every interactive accessibility window with a bounded tree. */
    fun interactiveWindowSnapshots(maxNodes: Int, deadlineNanos: Long = Long.MAX_VALUE): List<PhoneAppShot.PhoneWindow> {
        if (!canRetrieveWindowContent()) return emptyList()
        val windows = windows ?: return emptyList()
        val nextId = AtomicLong(1)
        var remaining = maxNodes
        return windows.map { window ->
            if (System.nanoTime() >= deadlineNanos) {
                return@map PhoneAppShot.PhoneWindow(window.id, windowDisplayId(window), window.type, windowBounds(window), window.isActive, window.isFocused, windowTitle(window), null, false, "deadline_exceeded", false, emptyList())
            }
            if (remaining <= 0) {
                return@map PhoneAppShot.PhoneWindow(window.id, windowDisplayId(window), window.type, windowBounds(window), window.isActive, window.isFocused, windowTitle(window), null, false, "node_budget_exhausted", true, emptyList())
            }
            val root = runCatching { window.root }.getOrNull()
            if (root == null) {
                return@map PhoneAppShot.PhoneWindow(window.id, windowDisplayId(window), window.type, windowBounds(window), window.isActive, window.isFocused, windowTitle(window), null, false, "root_unavailable", false, emptyList())
            }
            val nodes = ArrayList<PhoneAppShot.PhoneNode>()
            val traversal = collectAppShotNodes(root, window.id, null, nextId, remaining, deadlineNanos, nodes)
                ?: NodeTraversal(nodes.firstOrNull()?.id ?: -1L, truncated = true)
            remaining -= nodes.size
            PhoneAppShot.PhoneWindow(
                windowId = window.id,
                displayId = windowDisplayId(window),
                type = window.type,
                bounds = windowBounds(window),
                active = window.isActive,
                focused = window.isFocused,
                title = windowTitle(window),
                packageName = nodes.firstOrNull()?.packageName,
                rootAvailable = true,
                omissionReason = null,
                truncated = traversal.truncated,
                nodes = nodes,
            )
        }
    }

    private fun windowDisplayId(window: android.view.accessibility.AccessibilityWindowInfo): Int =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) window.displayId else android.view.Display.DEFAULT_DISPLAY

    private fun windowTitle(window: android.view.accessibility.AccessibilityWindowInfo): String? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) window.title?.toString() else null

    private fun windowBounds(window: android.view.accessibility.AccessibilityWindowInfo): IntArray {
        val bounds = Rect()
        window.getBoundsInScreen(bounds)
        return intArrayOf(bounds.left, bounds.top, bounds.right, bounds.bottom)
    }

    private data class NodeTraversal(val rootId: Long, val truncated: Boolean)

    private fun collectAppShotNodes(
        node: AccessibilityNodeInfo,
        windowId: Int,
        parentId: Long?,
        nextId: AtomicLong,
        remaining: Int,
        deadlineNanos: Long,
        out: MutableList<PhoneAppShot.PhoneNode>,
    ): NodeTraversal? {
        if (out.size >= remaining || System.nanoTime() >= deadlineNanos) return null
        val id = nextId.getAndIncrement()
        val childIds = ArrayList<Long>()
        val bounds = Rect()
        node.getBoundsInScreen(bounds)
        val result = PhoneAppShot.PhoneNode(
            id = id,
            parentId = parentId,
            childIds = childIds,
            windowId = windowId,
            packageName = node.packageName?.toString(),
            className = node.className?.toString(),
            viewId = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.JELLY_BEAN_MR2) node.viewIdResourceName else null,
            text = node.text?.toString(),
            hintText = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) node.hintText?.toString() else null,
            contentDescription = node.contentDescription?.toString(),
            bounds = intArrayOf(bounds.left, bounds.top, bounds.right, bounds.bottom),
            enabled = node.isEnabled,
            focused = node.isFocused,
            clickable = node.isClickable,
            editable = node.isEditable,
            scrollable = node.isScrollable,
            actions = node.actions,
            actionList = node.actionList.map { PhoneAppShot.NodeAction(it.id, it.label?.toString()) },
            selected = node.isSelected,
            checked = node.isChecked,
            checkable = node.isCheckable,
            password = node.isPassword,
            stateDescription = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) node.stateDescription?.toString() else null,
            inputType = node.inputType,
            textSelectionStart = node.textSelectionStart,
            textSelectionEnd = node.textSelectionEnd,
            collection = node.collectionInfo?.let { PhoneAppShot.CollectionMetadata(it.rowCount, it.columnCount, it.isHierarchical, it.selectionMode) },
            range = node.rangeInfo?.let { PhoneAppShot.RangeMetadata(it.type, it.min, it.max, it.current) },
        )
        out.add(result)
        var truncated = false
        for (i in 0 until node.childCount) {
            if (out.size >= remaining || System.nanoTime() >= deadlineNanos) {
                truncated = true
                break
            }
            val child = node.getChild(i)
            if (child == null) {
                truncated = true
                continue
            }
            val childResult = collectAppShotNodes(child, windowId, id, nextId, remaining, deadlineNanos, out)
            if (childResult != null) {
                childIds.add(childResult.rootId)
                truncated = truncated || childResult.truncated
            } else {
                truncated = true
            }
        }
        return NodeTraversal(id, truncated)
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

    fun takePhoneScreenshot(params: ScreenshotParams, deadlineNanos: Long = Long.MAX_VALUE): ProtoScreenshotResult {
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
            return captureScreenshot(includeOverlay = params.includeOverlay, deadlineNanos = deadlineNanos)
        } finally {
            if (hideOverlay) {
                overlay.restoreAfterCapture()
            }
        }
    }

    private fun captureScreenshot(includeOverlay: Boolean, deadlineNanos: Long): ProtoScreenshotResult {
        if (System.nanoTime() >= deadlineNanos) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.TRANSIENT,
                "screenshot deadline exceeded",
            )
        }
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
                        software.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, out)
                        val encoded = Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)
                        resultRef.set(
                            ProtoScreenshotResult(
                                mimeType = "image/jpeg",
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

        val remainingNanos = deadlineNanos - System.nanoTime()
        if (remainingNanos <= 0L || !latch.await(remainingNanos, TimeUnit.NANOSECONDS)) {
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

    /** Baseline editor control through the focused accessibility node. */
    fun performEditorOperation(params: JsonValue.Obj): JsonValue.Obj {
        val operation = params.string("operation") ?: throw MethodParamException(
            Protocol.ErrorCodes.BAD_REQUEST,
            "editor operation is required",
        )
        val node = editableTarget()
            ?: return editorResult("no_editable_target", null)
        if (operation == "context") return editorResult("applied", node)

        val applied = when (operation) {
            "set_text" -> {
                val text = params.string("text") ?: editorBad("set_text requires text")
                node.performAction(
                    AccessibilityNodeInfo.ACTION_SET_TEXT,
                    Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text) },
                )
            }
            "insert_text" -> {
                val inserted = params.string("text") ?: editorBad("insert_text requires text")
                val current = node.text?.toString().orEmpty()
                val start = node.textSelectionStart.takeIf { it >= 0 } ?: current.length
                val end = node.textSelectionEnd.takeIf { it >= start } ?: start
                val updated = current.substring(0, start.coerceAtMost(current.length)) + inserted +
                    current.substring(end.coerceAtMost(current.length))
                node.performAction(
                    AccessibilityNodeInfo.ACTION_SET_TEXT,
                    Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, updated) },
                )
            }
            "set_selection" -> {
                val start = params.int("start") ?: editorBad("set_selection requires start")
                val end = params.int("end") ?: editorBad("set_selection requires end")
                if (start < 0 || end < start) editorBad("selection must satisfy 0 <= start <= end")
                node.performAction(
                    AccessibilityNodeInfo.ACTION_SET_SELECTION,
                    Bundle().apply {
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, start)
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, end)
                    },
                )
            }
            "select_all" -> node.performAction(
                AccessibilityNodeInfo.ACTION_SET_SELECTION,
                Bundle().apply {
                    putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 0)
                    putInt(
                        AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT,
                        node.text?.length ?: 0,
                    )
                },
            )
            "copy" -> node.performAction(AccessibilityNodeInfo.ACTION_COPY)
            "cut" -> node.performAction(AccessibilityNodeInfo.ACTION_CUT)
            "paste" -> node.performAction(AccessibilityNodeInfo.ACTION_PASTE)
            "insert_content" -> return editorResult("unsupported_mime_type", node)
            else -> editorBad("unsupported editor operation '$operation'")
        }
        return editorResult(if (applied) "applied" else "no_editable_target", node)
    }

    fun performKey(rawKey: String): JsonValue.Obj {
        val action = when (rawKey.uppercase()) {
            "BACK", "KEYCODE_BACK" -> GLOBAL_ACTION_BACK
            "HOME", "KEYCODE_HOME" -> GLOBAL_ACTION_HOME
            "RECENTS", "APP_SWITCH", "KEYCODE_APP_SWITCH" -> GLOBAL_ACTION_RECENTS
            "NOTIFICATIONS", "KEYCODE_NOTIFICATION" -> GLOBAL_ACTION_NOTIFICATIONS
            "QUICK_SETTINGS" -> GLOBAL_ACTION_QUICK_SETTINGS
            "DPAD_UP", "KEYCODE_DPAD_UP" -> GLOBAL_ACTION_DPAD_UP
            "DPAD_DOWN", "KEYCODE_DPAD_DOWN" -> GLOBAL_ACTION_DPAD_DOWN
            "DPAD_LEFT", "KEYCODE_DPAD_LEFT" -> GLOBAL_ACTION_DPAD_LEFT
            "DPAD_RIGHT", "KEYCODE_DPAD_RIGHT" -> GLOBAL_ACTION_DPAD_RIGHT
            "DPAD_CENTER", "KEYCODE_DPAD_CENTER", "ENTER", "KEYCODE_ENTER" -> GLOBAL_ACTION_DPAD_CENTER
            else -> throw MethodApplicationException("unsupported_key", "key '$rawKey' is unavailable without the optional Sky IME")
        }
        if (!performGlobalAction(action)) throw MethodApplicationException(Protocol.ErrorCodes.TRANSIENT, "global key action was rejected")
        return jsonObject { put("dispatched", true); put("provider", "accessibility_global_action") }
    }

    private fun editableTarget(): AccessibilityNodeInfo? {
        val root = runCatching { rootInActiveWindow }.getOrNull() ?: return null
        root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.let { if (it.isEditable) return it }
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        while (queue.isNotEmpty()) {
            val node = queue.removeFirst()
            if (node.isEditable && node.isFocused) return node
            for (index in 0 until node.childCount) node.getChild(index)?.let(queue::add)
        }
        return null
    }

    private fun editorResult(outcome: String, node: AccessibilityNodeInfo?): JsonValue.Obj =
        jsonObject {
            put("outcome", outcome)
            node?.text?.toString()?.let { put("surrounding_text", it) }
            node?.textSelectionStart?.takeIf { it >= 0 }?.let { put("selection_start", it.toLong()) }
            node?.textSelectionEnd?.takeIf { it >= 0 }?.let { put("selection_end", it.toLong()) }
            put("accepted_mime_types", jsonArray(emptyList()))
            put("provider", "accessibility_node")
            put("ime_enhanced", false)
        }

    private fun editorBad(message: String): Nothing =
        throw MethodParamException(Protocol.ErrorCodes.BAD_REQUEST, message)

    companion object {
        private val instanceRef = AtomicReference<SkyAccessibilityService?>()
        private val accessibilityEventSequenceRef = AtomicLong(0)

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
