package com.skycua.phonecompanion.appshot

import android.accessibilityservice.AccessibilityService
import android.graphics.Rect
import android.hardware.display.DisplayManager
import android.os.Build
import android.util.Base64
import android.view.Display
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import com.skycua.phonecompanion.protocol.ScreenshotParams
import com.skycua.phonecompanion.service.SkyAccessibilityService
import java.util.UUID
import java.util.concurrent.TimeUnit

/** Android-side, host-independent representation of a phone AppShot. */
data class PhoneAppShot(
    val appshotId: String,
    val capturedAtMs: Long,
    val consistency: Consistency,
    val foreground: ForegroundApp,
    val display: DisplayState,
    val screenshot: ScreenshotPayload?,
    val windows: List<PhoneWindow>,
    val eventSequenceBefore: Long,
    val eventSequenceAfter: Long,
    val coverage: Coverage,
    val diagnostics: List<String> = emptyList(),
) {
    enum class Consistency { STABLE, CHANGED_DURING_CAPTURE, PARTIAL }

    data class ForegroundApp(val packageName: String?, val activityName: String?)

    data class DisplayState(
        val displayId: Int,
        val width: Int,
        val height: Int,
        val rotation: Int,
        val state: Int,
        val density: Float,
    )

    data class ScreenshotPayload(
        val mimeType: String,
        val dataBase64: String,
        val width: Int,
        val height: Int,
        val containsNativeOverlay: Boolean,
    )

    data class Coverage(
        val pixelsComplete: Boolean,
        val semanticsComplete: Boolean,
        val projectionTruncated: Boolean,
        val totalSemanticNodes: Int,
        val totalSemanticNodesKnown: Boolean,
        val projectedSemanticNodes: Int,
        val windowCount: Int,
        val projectedWindowCount: Int,
        val secureRegionsRedacted: Boolean,
    )

    data class PhoneWindow(
        val windowId: Int,
        val displayId: Int,
        val type: Int,
        val bounds: IntArray,
        val active: Boolean,
        val focused: Boolean,
        val title: String?,
        val packageName: String?,
        val rootAvailable: Boolean,
        val omissionReason: String?,
        val truncated: Boolean,
        val nodes: List<PhoneNode>,
    )

    data class PhoneNode(
        val id: Long,
        val parentId: Long?,
        val childIds: List<Long>,
        val windowId: Int,
        val packageName: String?,
        val className: String?,
        val viewId: String?,
        val text: String?,
        val hintText: String?,
        val contentDescription: String?,
        val bounds: IntArray,
        val enabled: Boolean,
        val focused: Boolean,
        val clickable: Boolean,
        val editable: Boolean,
        val scrollable: Boolean,
        val actions: Int,
        val actionList: List<NodeAction>,
        val selected: Boolean,
        val checked: Boolean,
        val checkable: Boolean,
        val password: Boolean,
        val stateDescription: String?,
        val inputType: Int,
        val textSelectionStart: Int,
        val textSelectionEnd: Int,
        val collection: CollectionMetadata?,
        val range: RangeMetadata?,
    )

    data class NodeAction(val id: Int, val label: String?)

    data class CollectionMetadata(
        val rowCount: Int,
        val columnCount: Int,
        val hierarchical: Boolean,
        val selectionMode: Int,
    )

    data class RangeMetadata(
        val type: Int,
        val min: Float,
        val max: Float,
        val current: Float,
    )

    companion object {
        fun timeout(appshotId: String, capturedAtMs: Long, eventSequence: Long): PhoneAppShot =
            PhoneAppShot(
                appshotId = appshotId,
                capturedAtMs = capturedAtMs,
                consistency = Consistency.PARTIAL,
                foreground = ForegroundApp(null, null),
                display = DisplayState(Display.DEFAULT_DISPLAY, 0, 0, 0, Display.STATE_UNKNOWN, 0f),
                screenshot = null,
                windows = emptyList(),
                eventSequenceBefore = eventSequence,
                eventSequenceAfter = eventSequence,
                coverage = Coverage(false, false, false, 0, false, 0, 0, 0, false),
                diagnostics = listOf("capture_deadline_exceeded"),
            )
    }
}

/**
 * Capture policy is deliberately independent from the RPC transport. The host
 * can later map this DTO to the shared AppShot envelope and a ContentRef.
 */
class PhoneAppShotProducer(
    private val source: PhoneAppShotSource,
    private val clockMs: () -> Long = { System.currentTimeMillis() },
    private val idFactory: () -> String = { UUID.randomUUID().toString() },
    private val sleeper: (Long) -> Unit = { Thread.sleep(it) },
    private val nanoTime: () -> Long = { System.nanoTime() },
    private val maxNodes: Int = 5_000,
) {
    fun capture(): PhoneAppShot {
        val deadline = nanoTime() + TimeUnit.SECONDS.toNanos(2)
        var last: PhoneAppShot? = null
        repeat(2) { attempt ->
            if (nanoTime() >= deadline) return@repeat
            waitForQuiescence(deadline)
            if (nanoTime() >= deadline) return@repeat
            val before = source.eventSequence()
            val shot = source.capture(idFactory(), clockMs(), before, maxNodes, deadline)
            val after = source.eventSequence()
            val changed = before != after || shot.eventSequenceAfter != after
            val candidate = shot.copy(
                eventSequenceBefore = before,
                eventSequenceAfter = after,
                consistency = when {
                    changed -> PhoneAppShot.Consistency.CHANGED_DURING_CAPTURE
                    !shot.coverage.pixelsComplete || !shot.coverage.semanticsComplete -> PhoneAppShot.Consistency.PARTIAL
                    else -> PhoneAppShot.Consistency.STABLE
                },
            )
            last = candidate
            if (!changed || attempt == 1) return candidate
        }
        return last ?: PhoneAppShot.timeout(idFactory(), clockMs(), source.eventSequence())
    }

    private fun waitForQuiescence(deadline: Long) {
        val start = nanoTime()
        var stableSince = start
        var observed = source.eventSequence()
        while (nanoTime() < deadline) {
            sleeper(10)
            val now = nanoTime()
            val current = source.eventSequence()
            if (current != observed) {
                observed = current
                stableSince = now
            }
            if (now - stableSince >= TimeUnit.MILLISECONDS.toNanos(150)) return
        }
    }
}

interface PhoneAppShotSource {
    fun eventSequence(): Long
    fun capture(appshotId: String, capturedAtMs: Long, eventSequence: Long, maxNodes: Int, deadlineNanos: Long): PhoneAppShot
}

/** Production source backed only by the enabled AccessibilityService. */
class AndroidPhoneAppShotSource(private val service: SkyAccessibilityService) : PhoneAppShotSource {
    override fun eventSequence(): Long = service.accessibilityEventSequence()

    override fun capture(
        appshotId: String,
        capturedAtMs: Long,
        eventSequence: Long,
        maxNodes: Int,
        deadlineNanos: Long,
    ): PhoneAppShot {
        val screenshot = runCatching { service.takePhoneScreenshot(ScreenshotParams(includeOverlay = false), deadlineNanos) }.getOrNull()
        val display = service.displayState()
        val windows = service.interactiveWindowSnapshots(maxNodes, deadlineNanos)
        val foreground = PhoneAppShot.ForegroundApp(service.activePackageName(), service.currentActivity())
        val totalNodes = windows.sumOf { it.nodes.size }
        val truncated = windows.any { it.truncated }
        val pixelsComplete = screenshot != null
        val semanticsComplete = service.canRetrieveWindowContent() && windows.isNotEmpty() &&
            windows.all { it.rootAvailable && it.omissionReason == null && !it.truncated }
        val totalNodesKnown = semanticsComplete
        return PhoneAppShot(
            appshotId = appshotId,
            capturedAtMs = capturedAtMs,
            consistency = if (pixelsComplete && semanticsComplete) PhoneAppShot.Consistency.STABLE else PhoneAppShot.Consistency.PARTIAL,
            foreground = foreground,
            display = display,
            screenshot = screenshot?.let { PhoneAppShot.ScreenshotPayload(it.mimeType, it.dataBase64, it.width, it.height, it.containsNativeOverlay) },
            windows = windows,
            eventSequenceBefore = eventSequence,
            eventSequenceAfter = service.accessibilityEventSequence(),
            coverage = PhoneAppShot.Coverage(pixelsComplete, semanticsComplete, truncated, totalNodes, totalNodesKnown, totalNodes, windows.size, windows.count { it.rootAvailable }, secureRegionsRedacted = false),
            diagnostics = buildList {
                if (!pixelsComplete) add("screenshot_unavailable")
                if (!semanticsComplete) add("accessibility_tree_partial")
                if (truncated) add("accessibility_tree_truncated")
                windows.filter { it.omissionReason != null }.forEach { add("window_${it.windowId}_${it.omissionReason}") }
            },
        )
    }
}

/** Converts the DTO to the companion's existing JSON value model for tests and later RPC integration. */
fun PhoneAppShot.toJson(): com.skycua.phonecompanion.json.JsonValue.Obj = PhoneAppShotJson.encode(this)

object PhoneAppShotJson {
    fun encode(shot: PhoneAppShot): com.skycua.phonecompanion.json.JsonValue.Obj {
        fun obj(block: com.skycua.phonecompanion.json.JsonObjectBuilder.() -> Unit) =
            com.skycua.phonecompanion.json.jsonObject(block)
        fun ints(values: IntArray) = com.skycua.phonecompanion.json.jsonArray(values.map { com.skycua.phonecompanion.json.JsonValue.of(it) })
        fun node(n: PhoneAppShot.PhoneNode) = obj {
            put("id", n.id); n.parentId?.let { put("parent_id", it) }
            put("child_ids", com.skycua.phonecompanion.json.jsonArray(n.childIds.map { com.skycua.phonecompanion.json.JsonValue.of(it) }))
            put("window_id", n.windowId); putOpt("package", n.packageName); putOpt("class", n.className); putOpt("view_id", n.viewId)
            putOpt("text", n.text); putOpt("hint_text", n.hintText); putOpt("content_desc", n.contentDescription); put("bounds", ints(n.bounds))
            put("enabled", n.enabled); put("focused", n.focused); put("clickable", n.clickable); put("editable", n.editable); put("scrollable", n.scrollable); put("actions", n.actions)
            put("action_list", com.skycua.phonecompanion.json.jsonArray(n.actionList.map { obj { put("id", it.id); putOpt("label", it.label) } }))
            put("selected", n.selected); put("checked", n.checked); put("checkable", n.checkable); put("password", n.password); putOpt("state_description", n.stateDescription); put("input_type", n.inputType); put("selection_start", n.textSelectionStart); put("selection_end", n.textSelectionEnd)
            n.collection?.let { put("collection", obj { put("rows", it.rowCount); put("columns", it.columnCount); put("hierarchical", it.hierarchical); put("selection_mode", it.selectionMode) }) }
            n.range?.let { put("range", obj { put("type", it.type); put("min", com.skycua.phonecompanion.json.JsonValue.Num(it.min.toDouble())); put("max", com.skycua.phonecompanion.json.JsonValue.Num(it.max.toDouble())); put("current", com.skycua.phonecompanion.json.JsonValue.Num(it.current.toDouble())) }) }
        }
        return obj {
            put("appshot_id", shot.appshotId); put("captured_at_ms", shot.capturedAtMs); put("consistency", shot.consistency.name.lowercase())
            put("foreground", obj { putOpt("package", shot.foreground.packageName); putOpt("activity", shot.foreground.activityName) })
            put("display", obj { put("id", shot.display.displayId); put("width", shot.display.width); put("height", shot.display.height); put("rotation", shot.display.rotation); put("state", shot.display.state); put("density", com.skycua.phonecompanion.json.JsonValue.Num(shot.display.density.toDouble())) })
            shot.screenshot?.let { put("screenshot", obj { put("mime_type", it.mimeType); put("data_base64", it.dataBase64); put("width", it.width); put("height", it.height); put("contains_native_overlay", it.containsNativeOverlay) }) }
            put("windows", com.skycua.phonecompanion.json.jsonArray(shot.windows.map { w -> obj { put("id", w.windowId); put("display_id", w.displayId); put("type", w.type); put("bounds", ints(w.bounds)); put("active", w.active); put("focused", w.focused); putOpt("title", w.title); putOpt("package", w.packageName); put("root_available", w.rootAvailable); putOpt("omission_reason", w.omissionReason); put("truncated", w.truncated); put("nodes", com.skycua.phonecompanion.json.jsonArray(w.nodes.map(::node))) } }))
            put("event_sequence_before", shot.eventSequenceBefore); put("event_sequence_after", shot.eventSequenceAfter)
            put("coverage", obj { put("pixels_complete", shot.coverage.pixelsComplete); put("semantics_complete", shot.coverage.semanticsComplete); put("projection_truncated", shot.coverage.projectionTruncated); put("total_semantic_nodes", shot.coverage.totalSemanticNodes); put("total_semantic_nodes_known", shot.coverage.totalSemanticNodesKnown); put("projected_semantic_nodes", shot.coverage.projectedSemanticNodes); put("window_count", shot.coverage.windowCount); put("projected_window_count", shot.coverage.projectedWindowCount); put("secure_regions_redacted", shot.coverage.secureRegionsRedacted) })
            put("diagnostics", com.skycua.phonecompanion.json.jsonArray(shot.diagnostics.map { com.skycua.phonecompanion.json.JsonValue.Str(it) }))
        }
    }
}
