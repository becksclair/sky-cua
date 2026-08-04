package com.skycua.phonecompanion

import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class EnrollmentManifestTest {
    @Test
    fun deepLinksRouteToOneEnrollmentActivityInstance() {
        val manifest = appFile("src/main/AndroidManifest.xml").readText()
        val activity =
            Regex(
                """<activity\s+[^>]*android:name="\.EnrollmentActivity"[^>]*>""",
                setOf(RegexOption.DOT_MATCHES_ALL),
            ).find(manifest)?.value ?: error("EnrollmentActivity declaration not found")

        assertTrue(
            "EnrollmentActivity must use singleTask so later deep links reach onNewIntent",
            activity.contains("android:launchMode=\"singleTask\""),
        )
    }

    private fun appFile(relativePath: String): File =
        sequenceOf(File("."), File("app"))
            .map { File(it, relativePath) }
            .firstOrNull(File::isFile)
            ?: error("could not locate $relativePath from ${File(".").absolutePath}")
}
