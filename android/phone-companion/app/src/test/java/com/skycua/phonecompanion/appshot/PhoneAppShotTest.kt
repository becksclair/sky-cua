package com.skycua.phonecompanion.appshot

import com.skycua.phonecompanion.json.JsonValue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PhoneAppShotTest {
    @Test
    fun stableCaptureIncludesDisplayCoverageAndTreeIdentity() {
        val source = FakeSource()
        val shot = PhoneAppShotProducer(source, clockMs = { 1234L }, idFactory = { "shot-1" }, sleeper = {}).capture()

        assertEquals("shot-1", shot.appshotId)
        assertEquals(PhoneAppShot.Consistency.STABLE, shot.consistency)
        assertEquals(1, shot.windows.single().nodes.single().id)
        assertEquals(800, shot.display.width)
        assertTrue(shot.coverage.semanticsComplete)
        assertEquals("pkg", (shot.toJson()["foreground"] as JsonValue.Obj).string("package"))
    }

    @Test
    fun changedGenerationRetriesOnceAndReportsFinalChange() {
        val source = FakeSource(changeOnFirstCapture = true)
        val shot = PhoneAppShotProducer(source, idFactory = { "shot" }, sleeper = {}).capture()

        assertEquals(2, source.captureCount)
        assertEquals(PhoneAppShot.Consistency.STABLE, shot.consistency)
        assertEquals(8L, shot.eventSequenceBefore)
        assertEquals(8L, shot.eventSequenceAfter)
    }

    @Test
    fun partialCaptureIsHonestAndSerializerDoesNotDropDiagnostics() {
        val source = FakeSource(partial = true)
        val shot = PhoneAppShotProducer(source, idFactory = { "partial" }, sleeper = {}).capture()
        val json = shot.toJson()

        assertEquals(PhoneAppShot.Consistency.PARTIAL, shot.consistency)
        assertEquals(false, (json["coverage"] as JsonValue.Obj).bool("pixels_complete"))
        assertEquals(1, (json["diagnostics"] as JsonValue.Arr).items.size)
    }

    @Test
    fun deadlineIsPassedToSourceAndNoUnfencedFallbackRuns() {
        val source = DeadlineSource()
        var ticks = 0L
        val shot = PhoneAppShotProducer(
            source = source,
            idFactory = { "timeout" },
            sleeper = {},
            nanoTime = { ticks += 1_000_000_000L; ticks },
        ).capture()

        assertEquals(PhoneAppShot.Consistency.PARTIAL, shot.consistency)
        assertTrue(shot.diagnostics.contains("capture_deadline_exceeded"))
        assertEquals(0, source.captureCount)
    }

    @Test
    fun windowDescriptorsRetainOmissionsAndExactBudgetIsNotMarkedTruncated() {
        val complete = PhoneAppShot.PhoneWindow(
            windowId = 1, displayId = 0, type = 1, bounds = intArrayOf(0, 0, 1, 1),
            active = true, focused = true, title = "complete", packageName = "pkg",
            rootAvailable = true, omissionReason = null, truncated = false, nodes = emptyList(),
        )
        val omitted = complete.copy(
            windowId = 2,
            title = null,
            packageName = null,
            rootAvailable = false,
            omissionReason = "root_unavailable",
        )
        val shot = PhoneAppShot(
            appshotId = "descriptors", capturedAtMs = 1,
            consistency = PhoneAppShot.Consistency.PARTIAL,
            foreground = PhoneAppShot.ForegroundApp(null, null),
            display = PhoneAppShot.DisplayState(0, 1, 1, 0, 2, 1f),
            screenshot = null, windows = listOf(complete, omitted),
            eventSequenceBefore = 1, eventSequenceAfter = 1,
            coverage = PhoneAppShot.Coverage(false, false, false, 0, false, 0, 2, 1, false),
            diagnostics = listOf("window_2_root_unavailable"),
        )
        val windows = shot.toJson()["windows"] as JsonValue.Arr
        assertEquals(2, windows.items.size)
        assertEquals(false, (windows.items[0] as JsonValue.Obj).bool("truncated"))
        assertEquals("root_unavailable", (windows.items[1] as JsonValue.Obj).string("omission_reason"))
    }

    private class FakeSource(
        private val changeOnFirstCapture: Boolean = false,
        private val partial: Boolean = false,
    ) : PhoneAppShotSource {
        var sequence = 7L
        var captureCount = 0

        override fun eventSequence(): Long = sequence

        override fun capture(appshotId: String, capturedAtMs: Long, eventSequence: Long, maxNodes: Int, deadlineNanos: Long): PhoneAppShot {
            captureCount++
            if (changeOnFirstCapture && captureCount == 1) sequence++
            val node = PhoneAppShot.PhoneNode(
                id = 1, parentId = null, childIds = emptyList(), windowId = 2,
                packageName = "pkg", className = "pkg.Root", viewId = "pkg:id/root",
                text = "hello", hintText = null, contentDescription = null,
                bounds = intArrayOf(0, 0, 10, 10), enabled = true, focused = true,
                clickable = true, editable = true, scrollable = false, actions = 1,
                actionList = emptyList(), selected = false, checked = false,
                checkable = false, password = false, stateDescription = null,
                inputType = 0, textSelectionStart = -1, textSelectionEnd = -1,
                collection = null, range = null,
            )
            return PhoneAppShot(
                appshotId,
                capturedAtMs,
                if (partial) PhoneAppShot.Consistency.PARTIAL else PhoneAppShot.Consistency.STABLE,
                PhoneAppShot.ForegroundApp("pkg", "pkg/Main"),
                PhoneAppShot.DisplayState(0, 800, 600, 0, 2, 1f),
                if (partial) null else PhoneAppShot.ScreenshotPayload("image/png", "AA==", 800, 600, false),
                listOf(PhoneAppShot.PhoneWindow(2, 0, 1, intArrayOf(0, 0, 800, 600), true, true, "Main", "pkg", true, null, false, listOf(node))),
                eventSequence,
                sequence,
                PhoneAppShot.Coverage(!partial, !partial, false, 1, true, 1, 1, 1, false),
                if (partial) listOf("screenshot_unavailable") else emptyList(),
            )
        }
    }

    private class DeadlineSource : PhoneAppShotSource {
        var captureCount = 0
        override fun eventSequence(): Long = 4
        override fun capture(appshotId: String, capturedAtMs: Long, eventSequence: Long, maxNodes: Int, deadlineNanos: Long): PhoneAppShot {
            captureCount++
            error("capture must not run after the deadline")
        }
    }
}
