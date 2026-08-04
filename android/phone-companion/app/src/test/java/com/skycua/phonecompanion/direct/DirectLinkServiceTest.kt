package com.skycua.phonecompanion.direct

import android.app.Service
import android.app.ForegroundServiceStartNotAllowedException
import android.content.ComponentName
import android.content.ContextWrapper
import android.content.Intent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class DirectLinkServiceTest {
    @Test
    fun serviceRequestsStickyRestart() {
        assertEquals(Service.START_STICKY, DirectLinkService().onStartCommand(null, 0, 0))
    }

    @Test
    fun notificationCopyMatchesLinkLifecycleAndOffersRecoveryAtTerminal() {
        assertEquals("Sky companion connected", DirectLinkNotificationText.forState(LinkState.CONNECTED).title)
        assertEquals(false, DirectLinkNotificationText.forState(LinkState.BACKOFF).recovery)
        assertEquals(true, DirectLinkNotificationText.forState(LinkState.REENROLL_REQUIRED).recovery)
    }

    @Test
    fun stoppedLinkNeedsRetryOnlyWhenDurableIntentExists() {
        assertEquals(true, directLinkNeedsUserRetry(DirectLinkServiceOwner.Availability.STOPPED, true))
        assertEquals(false, directLinkNeedsUserRetry(DirectLinkServiceOwner.Availability.STOPPED, false))
        assertEquals(false, directLinkNeedsUserRetry(DirectLinkServiceOwner.Availability.START_DENIED, true))
    }

    @Test
    fun ownerStartsStoppedBeforeAnyActivator() {
        DirectLinkServiceOwner.resetForTests()
        assertEquals(DirectLinkServiceOwner.Availability.STOPPED, DirectLinkServiceOwner.availability())
    }

    @Test
    fun deniedAccessibilityColdStartIsExposedAndRetryRemainsExplicit() {
        DirectLinkServiceOwner.resetForTests()
        val context = DeniedStartContext()
        assertEquals(false, DirectLinkServiceOwner.acquireAccessibility(context))
        assertEquals(DirectLinkServiceOwner.Availability.START_DENIED, DirectLinkServiceOwner.availability())
        assertEquals(false, DirectLinkServiceOwner.retryAccessibility(context))
    }

    @Test
    fun visibleRetryCanAttemptWithoutAccessibilityLease() {
        DirectLinkServiceOwner.resetForTests()
        assertEquals(false, DirectLinkServiceOwner.retryUserInitiated(DeniedStartContext()))
        assertEquals(DirectLinkServiceOwner.Availability.START_DENIED, DirectLinkServiceOwner.availability())
    }

    @Test
    fun manifestDeclaresRemoteMessagingForegroundService() {
        val manifest = findRepositoryFile("android/phone-companion/app/src/main/AndroidManifest.xml")
            .readText()
        assertTrue(manifest.contains("android.permission.FOREGROUND_SERVICE"))
        assertTrue(manifest.contains("android.permission.FOREGROUND_SERVICE_REMOTE_MESSAGING"))
        assertTrue(manifest.contains("android:foregroundServiceType=\"remoteMessaging\""))
    }

    private fun findRepositoryFile(relativePath: String): File {
        var directory = File(System.getProperty("user.dir") ?: error("user.dir unavailable")).absoluteFile
        while (true) {
            val candidate = File(directory, relativePath)
            if (candidate.isFile) return candidate
            directory = directory.parentFile ?: error("repository file not found: $relativePath")
        }
    }

    private class DeniedStartContext : ContextWrapper(null) {
        override fun startForegroundService(service: Intent): ComponentName? = throw ForegroundServiceStartNotAllowedException("background start denied")
        override fun startService(service: Intent): ComponentName? = throw ForegroundServiceStartNotAllowedException("background start denied")
    }
}
