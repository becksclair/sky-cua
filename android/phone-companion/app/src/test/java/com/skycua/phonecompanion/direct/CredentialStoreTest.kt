package com.skycua.phonecompanion.direct

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class CredentialStoreTest {
    @Test
    fun gcmEnvelopePacksAndUnpacksKeystoreIv() {
        val nonce = ByteArray(GcmCredentialEnvelope.NONCE_BYTES) { it.toByte() }
        val ciphertext = byteArrayOf(1, 2, 3, 4)
        val packed = GcmCredentialEnvelope.pack(nonce, ciphertext)

        assertArrayEquals(nonce, GcmCredentialEnvelope.nonce(packed))
        assertArrayEquals(ciphertext, GcmCredentialEnvelope.ciphertext(packed))
    }

    @Test
    fun gcmEnvelopeRejectsUnexpectedIvLength() {
        assertThrows(IllegalArgumentException::class.java) {
            GcmCredentialEnvelope.pack(ByteArray(16), byteArrayOf(1))
        }
    }

    @Test
    fun memoryStoreMultiHostAddListAndDelete() {
        val store = MemoryCredentialStore(enableMultiHost = true)
        val secret = ByteArray(32) { it.toByte() }
        val secret2 = ByteArray(32) { (it + 1).toByte() }
        store.saveEnrollment(DeviceCredential("host-1", secret), "ws://host1.example/phone/control", PendingEnrollment("enr-1", 9999))
        store.saveEnrollment(DeviceCredential("host-2", secret2), "ws://host2.example/phone/control", null)
        assertEquals(2, store.loadAll().size)
        assertNotNull(store.loadHost("host-1"))
        assertNotNull(store.loadHost("host-2"))
        val firstEpoch = store.lastAcceptedEpochForHost("host-1")
        assertEquals("0", firstEpoch)
        // Host-scoped epoch isolation
        val scoped1 = MemoryHostScopedCredentialStore(store, "host-1")
        scoped1.saveAcceptedEpoch("5")
        assertEquals("5", store.lastAcceptedEpochForHost("host-1"))
        assertEquals("0", store.lastAcceptedEpochForHost("host-2"))
        // Delete one leaves the other
        store.deleteHost("host-1")
        assertEquals(1, store.loadAll().size)
        assertNull(store.loadHost("host-1"))
        assertNotNull(store.loadHost("host-2"))
        // Delete last clears
        store.deleteHost("host-2")
        assertEquals(0, store.loadAll().size)
        assertNull(store.load())
    }

    @Test
    fun memoryStoreMultiHostRejectsTooMany() {
        val store = MemoryCredentialStore(enableMultiHost = true)
        for (i in 0 until MAX_PAIRED_HOSTS) {
            store.saveEnrollment(DeviceCredential("host-$i", ByteArray(32) { i.toByte() }), "ws://host$i.example/phone/control", null)
        }
        assertEquals(MAX_PAIRED_HOSTS, store.hostCount())
        assertThrows(IllegalArgumentException::class.java) {
            store.saveEnrollment(DeviceCredential("host-overflow", ByteArray(32) { 99.toByte() }), "ws://overflow.example/phone/control", null)
        }
    }

    @Test
    fun memoryStoreMigratesLegacyOnFirstMultiWrite() {
        val store = MemoryCredentialStore(enableMultiHost = false)
        val secret = ByteArray(32) { 7.toByte() }
        store.saveEnrollment(DeviceCredential("legacy", secret), "ws://legacy.example/phone/control", PendingEnrollment("p", 123))
        // Enable multi and add second host — legacy should be preserved as first entry
        store.enableMultiHost = true
        val secret2 = ByteArray(32) { 8.toByte() }
        store.saveEnrollment(DeviceCredential("new-host", secret2), "ws://new.example/phone/control", null)
        assertEquals(2, store.loadAll().size)
        assertNotNull(store.loadHost("legacy"))
        assertNotNull(store.loadHost("new-host"))
    }

    @Test
    fun memoryStoreLegacyOverwriteWhenMultiDisabled() {
        val store = MemoryCredentialStore(enableMultiHost = false)
        store.saveEnrollment(DeviceCredential("host-1", ByteArray(32) { 1.toByte() }), "ws://host1.example/phone/control", null)
        store.saveEnrollment(DeviceCredential("host-2", ByteArray(32) { 2.toByte() }), "ws://host2.example/phone/control", null)
        // With multi disabled, second save overwrites — only one host via loadAll's single fallback
        assertEquals(1, store.loadAll().size)
        assertEquals("host-2", store.load()?.deviceId)
    }
}
