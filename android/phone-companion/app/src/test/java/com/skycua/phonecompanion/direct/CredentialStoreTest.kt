package com.skycua.phonecompanion.direct

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertThrows
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
}
