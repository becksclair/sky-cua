package com.skycua.phonecompanion.direct

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

data class DeviceCredential(val deviceId: String, val deviceSecret: ByteArray)
data class PendingEnrollment(val enrollmentId: String, val expiresAtMs: Long)

/** Stable storage envelope for the Keystore-generated AES-GCM IV and payload. */
internal object GcmCredentialEnvelope {
    const val NONCE_BYTES = 12

    fun pack(nonce: ByteArray, ciphertext: ByteArray): ByteArray {
        require(nonce.size == NONCE_BYTES) { "unexpected AES-GCM IV length" }
        return nonce + ciphertext
    }

    fun nonce(packed: ByteArray): ByteArray {
        require(packed.size > NONCE_BYTES)
        return packed.copyOfRange(0, NONCE_BYTES)
    }

    fun ciphertext(packed: ByteArray): ByteArray {
        require(packed.size > NONCE_BYTES)
        return packed.copyOfRange(NONCE_BYTES, packed.size)
    }
}

interface CredentialStore {
    fun load(): DeviceCredential?
    fun save(credential: DeviceCredential)
    fun saveEnrollment(credential: DeviceCredential, endpoint: String, pending: PendingEnrollment? = null) = save(credential)
    fun pendingEnrollment(): PendingEnrollment? = null
    fun clearPendingEnrollment() {}
    fun clear()
    fun lastAcceptedEpoch(): String = "0"
    fun saveAcceptedEpoch(epoch: String) {}
}

/** Small in-memory implementation used by deterministic JVM tests and fakes. */
class MemoryCredentialStore : CredentialStore {
    private var value: DeviceCredential? = null
    private var acceptedEpoch: String = "0"
    var endpoint: String? = null
    var failNextEnrollmentCommit = false
    private var pending: PendingEnrollment? = null
    override fun load(): DeviceCredential? = value?.copy(deviceSecret = value!!.deviceSecret.copyOf())
    override fun save(credential: DeviceCredential) {
        require(credential.deviceId.isNotBlank() && credential.deviceSecret.isNotEmpty())
        acceptedEpoch = "0"
        value = credential.copy(deviceSecret = credential.deviceSecret.copyOf())
    }
    override fun saveEnrollment(credential: DeviceCredential, endpoint: String, pending: PendingEnrollment?) {
        require(credential.deviceId.isNotBlank() && credential.deviceSecret.size == 32)
        if (failNextEnrollmentCommit) { failNextEnrollmentCommit = false; error("commit failed") }
        save(credential); this.endpoint = endpoint; this.pending = pending
        DirectLinkReplacementNotifier.notifyCommitted()
    }
    override fun pendingEnrollment(): PendingEnrollment? = pending
    override fun clearPendingEnrollment() { pending = null }
    override fun clear() { value = null; endpoint = null; pending = null; acceptedEpoch = "0" }
    override fun lastAcceptedEpoch(): String = acceptedEpoch
    override fun saveAcceptedEpoch(epoch: String) { acceptedEpoch = epoch }
}

/** Keystore-backed credential store. Data is kept in credential-protected preferences. */
class AndroidCredentialStore(context: Context) : CredentialStore {
    // The application context is credential-protected by definition; Android
    // exposes only the inverse createDeviceProtectedStorageContext() helper.
    private val prefs = context.getSharedPreferences("phone_control_v2", Context.MODE_PRIVATE)
    private val keyAlias = "phone-control-v2-wrap"

    override fun load(): DeviceCredential? {
        val version = prefs.getLong("committed_version", 0L)
        val prefix = if (version > 0) "record_${version}_" else ""
        val deviceId = prefs.getString(prefix + "device_id", prefs.getString("device_id", null)) ?: return null
        val encoded = prefs.getString(prefix + "secret", prefs.getString("secret", null)) ?: return null
        return try {
            val packed: ByteArray = android.util.Base64.decode(encoded, android.util.Base64.NO_WRAP) as ByteArray
            val nonce = GcmCredentialEnvelope.nonce(packed)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
                init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(GCM_TAG_BITS, nonce))
            }
            DeviceCredential(deviceId, cipher.doFinal(GcmCredentialEnvelope.ciphertext(packed)))
        } catch (_: Exception) {
            null
        }
    }

    override fun save(credential: DeviceCredential) {
        saveEnrollment(credential, prefs.getString("endpoint", null) ?: "")
    }

    override fun saveEnrollment(credential: DeviceCredential, endpoint: String, pending: PendingEnrollment?) {
        require(credential.deviceId.isNotBlank() && credential.deviceSecret.isNotEmpty())
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
            // Android Keystore owns IV generation for GCM. Supplying an IV here
            // is rejected by some API 36 providers (and would weaken the
            // randomized-encryption contract if callers reused one).
            init(Cipher.ENCRYPT_MODE, key())
        }
        val packed = GcmCredentialEnvelope.pack(cipher.iv, cipher.doFinal(credential.deviceSecret))
        val version = prefs.getLong("committed_version", 0L) + 1L
        val staged = prefs.edit()
            .putString("record_${version}_device_id", credential.deviceId)
            .putString("record_${version}_secret", android.util.Base64.encodeToString(packed, android.util.Base64.NO_WRAP))
            .putString("record_${version}_endpoint", endpoint)
            .putString("record_${version}_accepted_epoch", "0")
            .putString("record_${version}_pending_id", pending?.enrollmentId)
            .putLong("record_${version}_pending_expiry", pending?.expiresAtMs ?: 0L)
        check(commit(staged)) { "unable to stage phone-control credential" }
        // Pointer swap is the commit point. If it fails, the previous pointer and record remain active.
        check(commit(prefs.edit().putLong("committed_version", version))) { "unable to commit phone-control credential" }
        DirectLinkReplacementNotifier.notifyCommitted()
    }

    override fun clear() { check(commit(prefs.edit().clear())) { "unable to clear phone-control credential" } }
    override fun pendingEnrollment(): PendingEnrollment? {
        val v = prefs.getLong("committed_version", 0L); if (v <= 0) return null
        val id = prefs.getString("record_${v}_pending_id", null) ?: return null
        val expiry = prefs.getLong("record_${v}_pending_expiry", 0L)
        return PendingEnrollment(id, expiry).takeIf { expiry > 0 }
    }
    override fun clearPendingEnrollment() {
        val v = prefs.getLong("committed_version", 0L); if (v > 0) check(commit(prefs.edit().remove("record_${v}_pending_id").remove("record_${v}_pending_expiry")))
    }
    override fun lastAcceptedEpoch(): String {
        val version = prefs.getLong("committed_version", 0L)
        return prefs.getString(if (version > 0) "record_${version}_accepted_epoch" else "accepted_epoch", null) ?: "0"
    }
    override fun saveAcceptedEpoch(epoch: String) {
        val version = prefs.getLong("committed_version", 0L)
        val key = if (version > 0) "record_${version}_accepted_epoch" else "accepted_epoch"
        check(commit(prefs.edit().putString(key, epoch))) { "unable to persist accepted link epoch" }
    }

    private fun commit(editor: android.content.SharedPreferences.Editor): Boolean = editor.commit()

    private fun key(): SecretKey {
        val keys = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (keys.getKey(keyAlias, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        generator.init(KeyGenParameterSpec.Builder(keyAlias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setUserAuthenticationRequired(false)
            .build())
        return generator.generateKey()
    }

    private companion object {
        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val GCM_TAG_BITS = 128
    }
}
