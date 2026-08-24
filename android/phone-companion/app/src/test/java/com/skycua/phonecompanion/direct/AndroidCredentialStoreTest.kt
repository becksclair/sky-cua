package com.skycua.phonecompanion.direct

import android.content.Context
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class AndroidCredentialStoreTest {
    private lateinit var context: Context
    private lateinit var testKey: SecretKey

    @Before
    fun setUp() {
        context = RuntimeEnvironment.getApplication()
        context.getSharedPreferences("phone_control_v2", Context.MODE_PRIVATE).edit().clear().commit()
        testKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    }

    private fun store(): AndroidCredentialStore = AndroidCredentialStore(context, testKey)

    @Test
    fun freshInstallEnrollPersistsAndLoads() {
        val s = store()
        val secret = ByteArray(32) { it.toByte() }
        s.saveEnrollment(DeviceCredential("fresh-1", secret), "ws://fresh.example/phone/control", PendingEnrollment("enr-1", 9999))
        val all = s.loadAll()
        assertEquals(1, all.size)
        assertEquals("fresh-1", all.first().deviceId)
        assertEquals("ws://fresh.example/phone/control", all.first().endpoint)
        assertNotNull(s.loadHost("fresh-1"))
        assertEquals("enr-1", s.pendingEnrollmentForHost("fresh-1")?.enrollmentId)
    }

    @Test
    fun legacyMigratesOnFirstMultiWriteAndPreservesSecondHost() {
        val prefs = context.getSharedPreferences("phone_control_v2", Context.MODE_PRIVATE)
        val s1 = store()
        val secretLegacy = ByteArray(32) { 7.toByte() }
        s1.saveEnrollment(DeviceCredential("legacy-1", secretLegacy), "ws://legacy.example/phone/control", null)
        prefs.edit().remove("host_ids").commit()
        val legacyLoad = store().load()
        assertNotNull(legacyLoad)
        assertEquals("legacy-1", legacyLoad?.deviceId)
        val s2 = store()
        val secret2 = ByteArray(32) { 8.toByte() }
        s2.saveEnrollment(DeviceCredential("new-2", secret2), "ws://new.example/phone/control", null)
        val all = s2.loadAll()
        assertEquals(2, all.size)
        assertNotNull(s2.loadHost("legacy-1"))
        assertNotNull(s2.loadHost("new-2"))
    }

    @Test
    fun deleteLastHostClearsStore() {
        val s = store()
        val s1 = ByteArray(32) { 1.toByte() }
        val s2 = ByteArray(32) { 2.toByte() }
        s.saveEnrollment(DeviceCredential("h1", s1), "ws://h1.example/phone/control", null)
        s.saveEnrollment(DeviceCredential("h2", s2), "ws://h2.example/phone/control", null)
        assertEquals(2, s.hostCount())
        s.deleteHost("h1")
        assertEquals(1, s.hostCount())
        assertNull(s.loadHost("h1"))
        s.deleteHost("h2")
        assertEquals(0, s.hostCount())
        assertEquals(0, s.loadAll().size)
        assertNull(s.load())
    }

    @Test
    fun corruptedSecretRecordIsTolerated() {
        val s = store()
        val secret = ByteArray(32) { 9.toByte() }
        s.saveEnrollment(DeviceCredential("good", secret), "ws://good.example/phone/control", null)
        val prefs = context.getSharedPreferences("phone_control_v2", Context.MODE_PRIVATE)
        val badId = "bad-host"
        val ids = prefs.getStringSet("host_ids", emptySet())!!.toMutableSet().apply { add(badId) }
        prefs.edit()
            .putStringSet("host_ids", ids)
            .putString("host_${badId}_secret", "!!!not-base64!!!")
            .putString("host_${badId}_endpoint", "ws://bad.example/phone/control")
            .putString("host_${badId}_accepted_epoch", "0")
            .commit()
        val all = store().loadAll()
        assertEquals(1, all.size)
        assertEquals("good", all.first().deviceId)
        assertNull(store().loadHost(badId))
    }

    @Test
    fun duplicateEndpointReplacesPriorHost() {
        val s = store()
        val s1 = ByteArray(32) { 11.toByte() }
        val s2 = ByteArray(32) { 12.toByte() }
        s.saveEnrollment(DeviceCredential("host-a", s1), "ws://dup.example/phone/control", null)
        assertEquals(1, s.hostCount())
        s.saveEnrollment(DeviceCredential("host-b", s2), "ws://dup.example/phone/control", null)
        assertEquals(1, s.hostCount())
        assertNull(s.loadHost("host-a"))
        assertNotNull(s.loadHost("host-b"))
        assertEquals("ws://dup.example/phone/control", s.loadHost("host-b")?.endpoint)
    }

    @Test
    fun emptyHostIdsAfterDeleteLeavesNoOrphans() {
        val s = store()
        val secret = ByteArray(32) { 3.toByte() }
        s.saveEnrollment(DeviceCredential("only", secret), "ws://only.example/phone/control", null)
        s.deleteHost("only")
        val prefs = context.getSharedPreferences("phone_control_v2", Context.MODE_PRIVATE)
        assertTrue(prefs.getStringSet("host_ids", null)?.isEmpty() ?: true)
        assertEquals(0, s.loadAll().size)
    }
}
