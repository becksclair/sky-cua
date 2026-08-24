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
data class HostRecord(
    val deviceId: String,
    val deviceSecret: ByteArray,
    val endpoint: String,
    val acceptedEpoch: String = "0",
    val pendingEnrollment: PendingEnrollment? = null,
)

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

/** Extended store that supports multiple simultaneously paired hosts. */
interface MultiHostCredentialStore : CredentialStore {
    fun loadAll(): List<HostRecord>
    fun loadHost(deviceId: String): HostRecord?
    fun deleteHost(deviceId: String)
    fun hostCount(): Int = loadAll().size
}

internal const val MAX_PAIRED_HOSTS = 8

/** Small in-memory implementation used by deterministic JVM tests and fakes. */
class MemoryCredentialStore(
    var enableMultiHost: Boolean = false,
) : MultiHostCredentialStore {
    private var value: DeviceCredential? = null
    private var acceptedEpoch: String = "0"
    var endpoint: String? = null
    var failNextEnrollmentCommit = false
    private var pending: PendingEnrollment? = null
    private val hosts: MutableMap<String, HostRecord> = LinkedHashMap()
    override fun load(): DeviceCredential? {
        if (hosts.isNotEmpty()) return hosts.values.first().let { DeviceCredential(it.deviceId, it.deviceSecret.copyOf()) }
        return value?.copy(deviceSecret = value!!.deviceSecret.copyOf())
    }
    override fun save(credential: DeviceCredential) {
        require(credential.deviceId.isNotBlank() && credential.deviceSecret.isNotEmpty())
        acceptedEpoch = "0"
        value = credential.copy(deviceSecret = credential.deviceSecret.copyOf())
        if (hosts.isEmpty()) return
        val primary = hosts.keys.firstOrNull() ?: return
        if (primary == credential.deviceId) {
            hosts[primary] = hosts[primary]!!.copy(deviceSecret = credential.deviceSecret.copyOf(), acceptedEpoch = "0")
        } else {
            val old = hosts.remove(primary) ?: return
            // If new id already exists as another host, replace it (same-endpoint dedup not needed for save)
            hosts.remove(credential.deviceId)
            hosts[credential.deviceId] = old.copy(deviceId = credential.deviceId, deviceSecret = credential.deviceSecret.copyOf(), acceptedEpoch = "0")
        }
    }
    override fun saveEnrollment(credential: DeviceCredential, endpoint: String, pending: PendingEnrollment?) {
        require(credential.deviceId.isNotBlank() && credential.deviceSecret.size == 32)
        if (failNextEnrollmentCommit) { failNextEnrollmentCommit = false; error("commit failed") }
        if (!enableMultiHost) {
            // Legacy single-host path: overwrite.
            value = credential.copy(deviceSecret = credential.deviceSecret.copyOf())
            this.endpoint = endpoint; this.pending = pending; this.acceptedEpoch = "0"
            hosts.clear()
            DirectLinkReplacementNotifier.notifyCommitted()
            return
        }
        if (hosts.isNotEmpty() || value != null) {
            // Migrate legacy single slot into hosts on first multi-host write.
            if (hosts.isEmpty() && value != null) {
                val legacyEndpoint = this.endpoint ?: ""
                hosts[value!!.deviceId] = HostRecord(value!!.deviceId, value!!.deviceSecret.copyOf(), legacyEndpoint, acceptedEpoch, this.pending)
                value = null; this.pending = null
            }
        }
        // Same-endpoint re-pair replaces prior entry for that endpoint.
        val duplicateIds = if (endpoint.isNotBlank()) hosts.filter { it.key != credential.deviceId && it.value.endpoint == endpoint }.keys.toList() else emptyList()
        duplicateIds.forEach { hosts.remove(it) }
        if (hosts.containsKey(credential.deviceId)) {
            val existing = hosts[credential.deviceId]!!
            hosts[credential.deviceId] = existing.copy(deviceSecret = credential.deviceSecret.copyOf(), endpoint = endpoint, pendingEnrollment = pending)
        } else {
            require(hosts.size < MAX_PAIRED_HOSTS) { "too many paired hosts" }
            hosts[credential.deviceId] = HostRecord(credential.deviceId, credential.deviceSecret.copyOf(), endpoint, "0", pending)
        }
        // Keep legacy compat pointers in sync for callers that use the single-slot API.
        value = credential.copy(deviceSecret = credential.deviceSecret.copyOf())
        this.endpoint = endpoint; this.pending = pending; this.acceptedEpoch = "0"
        DirectLinkReplacementNotifier.notifyCommitted()
    }
    override fun pendingEnrollment(): PendingEnrollment? {
        if (hosts.isNotEmpty()) return hosts.values.firstOrNull()?.pendingEnrollment
        return pending
    }
    override fun clearPendingEnrollment() {
        if (hosts.isNotEmpty()) {
            val first = hosts.keys.firstOrNull() ?: return
            hosts[first] = hosts[first]!!.copy(pendingEnrollment = null)
            val firstPending = hosts[first]!!.pendingEnrollment
            if (firstPending == null) pending = null
            return
        }
        pending = null
    }
    override fun clear() { value = null; endpoint = null; pending = null; acceptedEpoch = "0"; hosts.clear() }
    override fun lastAcceptedEpoch(): String {
        if (hosts.isNotEmpty()) return hosts.values.firstOrNull()?.acceptedEpoch ?: "0"
        return acceptedEpoch
    }
    override fun saveAcceptedEpoch(epoch: String) {
        if (hosts.isNotEmpty()) {
            val first = hosts.keys.firstOrNull() ?: return
            hosts[first] = hosts[first]!!.copy(acceptedEpoch = epoch)
            acceptedEpoch = epoch
            return
        }
        acceptedEpoch = epoch
    }
    override fun loadAll(): List<HostRecord> {
        if (hosts.isNotEmpty()) return hosts.values.map { it.copy(deviceSecret = it.deviceSecret.copyOf()) }
        val single = value ?: return emptyList()
        return listOf(HostRecord(single.deviceId, single.deviceSecret.copyOf(), endpoint ?: "", acceptedEpoch, pending))
    }
    override fun loadHost(deviceId: String): HostRecord? = (hosts[deviceId] ?: run {
        if (value?.deviceId == deviceId) HostRecord(value!!.deviceId, value!!.deviceSecret.copyOf(), endpoint ?: "", acceptedEpoch, pending) else null
    })?.let { it.copy(deviceSecret = it.deviceSecret.copyOf()) }
    override fun deleteHost(deviceId: String) {
        if (hosts.isNotEmpty()) { hosts.remove(deviceId); if (hosts.isEmpty()) { value = null; endpoint = null; pending = null; acceptedEpoch = "0" } else if (value?.deviceId == deviceId) { val next = hosts.values.first(); value = DeviceCredential(next.deviceId, next.deviceSecret.copyOf()); endpoint = next.endpoint; pending = next.pendingEnrollment; acceptedEpoch = next.acceptedEpoch }; return }
        if (value?.deviceId == deviceId) clear()
    }

    /** Direct per-host helpers for the pool without going through the legacy alias. */
    fun pendingEnrollmentForHost(deviceId: String): PendingEnrollment? = hosts[deviceId]?.pendingEnrollment ?: if (value?.deviceId == deviceId) pending else null
    fun clearPendingForHost(deviceId: String) { hosts[deviceId]?.let { hosts[deviceId] = it.copy(pendingEnrollment = null) } }
    fun lastAcceptedEpochForHost(deviceId: String): String = hosts[deviceId]?.acceptedEpoch ?: if (value?.deviceId == deviceId) acceptedEpoch else "0"
    fun saveAcceptedEpochForHost(deviceId: String, epoch: String) {
        hosts[deviceId]?.let { hosts[deviceId] = it.copy(acceptedEpoch = epoch) }
        if (value?.deviceId == deviceId) acceptedEpoch = epoch
    }
}

/** Keystore-backed credential store with multi-host support. Data is kept in credential-protected preferences. */
class AndroidCredentialStore(
    context: Context,
    private val testKey: SecretKey? = null,
) : MultiHostCredentialStore {
    // The application context is credential-protected by definition; Android
    // exposes only the inverse createDeviceProtectedStorageContext() helper.
    private val prefs = context.getSharedPreferences("phone_control_v2", Context.MODE_PRIVATE)
    private val keyAlias = "phone-control-v2-wrap"

    // -- legacy helpers --
    private fun loadLegacy(): DeviceCredential? {
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
        } catch (_: Exception) { null }
    }

    private fun decryptSecret(encoded: String): ByteArray {
        val packed: ByteArray = android.util.Base64.decode(encoded, android.util.Base64.NO_WRAP) as ByteArray
        val nonce = GcmCredentialEnvelope.nonce(packed)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(GCM_TAG_BITS, nonce)) }
        return cipher.doFinal(GcmCredentialEnvelope.ciphertext(packed))
    }

    private fun encryptSecret(secret: ByteArray): String {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        val packed = GcmCredentialEnvelope.pack(cipher.iv, cipher.doFinal(secret))
        return android.util.Base64.encodeToString(packed, android.util.Base64.NO_WRAP)
    }

    private fun hostIds(): MutableSet<String> = prefs.getStringSet("host_ids", null)?.toMutableSet() ?: mutableSetOf()
    private fun hostSecretKey(id: String) = "host_${id}_secret"
    private fun hostEndpointKey(id: String) = "host_${id}_endpoint"
    private fun hostEpochKey(id: String) = "host_${id}_accepted_epoch"
    private fun hostPendingIdKey(id: String) = "host_${id}_pending_id"
    private fun hostPendingExpiryKey(id: String) = "host_${id}_pending_expiry"

    private fun readHostRecord(id: String): HostRecord? {
        val encoded = prefs.getString(hostSecretKey(id), null) ?: return null
        val secret = try { decryptSecret(encoded) } catch (_: Exception) { return null }
        val endpoint = prefs.getString(hostEndpointKey(id), "") ?: ""
        val epoch = prefs.getString(hostEpochKey(id), "0") ?: "0"
        val pendingId = prefs.getString(hostPendingIdKey(id), null)
        val pendingExpiry = prefs.getLong(hostPendingExpiryKey(id), 0L)
        val pending = if (pendingId != null && pendingExpiry > 0) PendingEnrollment(pendingId, pendingExpiry) else null
        return HostRecord(id, secret, endpoint, epoch, pending)
    }

    private fun hasMultiHosts(): Boolean = prefs.contains("host_ids")

    private fun ensureMigrated(): Boolean {
        if (hasMultiHosts()) return false
        val legacy = loadLegacy() ?: return false
        val version = prefs.getLong("committed_version", 0L)
        val prefix = if (version > 0) "record_${version}_" else ""
        val endpoint = prefs.getString(prefix + "endpoint", prefs.getString("endpoint", "")) ?: ""
        val epoch = prefs.getString(if (version > 0) "record_${version}_accepted_epoch" else "accepted_epoch", "0") ?: "0"
        val pendingId = if (version > 0) prefs.getString("record_${version}_pending_id", null) else null
        val pendingExpiry = if (version > 0) prefs.getLong("record_${version}_pending_expiry", 0L) else 0L
        val pending = if (pendingId != null && pendingExpiry > 0) PendingEnrollment(pendingId, pendingExpiry) else null
        val encoded = prefs.getString(prefix + "secret", prefs.getString("secret", null)) ?: return false
        val editor = prefs.edit()
            .putStringSet("host_ids", setOf(legacy.deviceId))
            .putString(hostSecretKey(legacy.deviceId), encoded)
            .putString(hostEndpointKey(legacy.deviceId), endpoint)
            .putString(hostEpochKey(legacy.deviceId), epoch)
        if (pending != null) editor.putString(hostPendingIdKey(legacy.deviceId), pending.enrollmentId).putLong(hostPendingExpiryKey(legacy.deviceId), pending.expiresAtMs)
        // Keep legacy keys for rollback safety; they are ignored once host_ids exists.
        check(commit(editor)) { "unable to migrate legacy host" }
        return true
    }

    override fun load(): DeviceCredential? {
        if (hasMultiHosts()) {
            val ids = hostIds(); if (ids.isEmpty()) return null
            val first = ids.sorted().first()
            val rec = readHostRecord(first) ?: return null
            return DeviceCredential(rec.deviceId, rec.deviceSecret)
        }
        // No multi-host set yet — fall back to legacy single.
        return loadLegacy()
    }

    override fun save(credential: DeviceCredential) {
        // After migration, preserve atomic single-commit semantics via saveEnrollment.
        if (hasMultiHosts()) {
            val ids = hostIds(); if (ids.isEmpty()) { saveEnrollment(credential, ""); return }
            val primary = ids.sorted().first()
            val rec = readHostRecord(primary)
            val endpoint = rec?.endpoint ?: ""
            val pending = rec?.pendingEnrollment
            saveEnrollment(credential, endpoint, pending)
            return
        }
        saveEnrollment(credential, prefs.getString("endpoint", null) ?: "")
    }

    override fun saveEnrollment(credential: DeviceCredential, endpoint: String, pending: PendingEnrollment?) {
        require(credential.deviceId.isNotBlank() && credential.deviceSecret.isNotEmpty())
        require(credential.deviceSecret.size == 32) { "device secret must be 256-bit" }
        if (!hasMultiHosts() && (prefs.contains("committed_version") || prefs.contains("device_id") || prefs.contains("secret"))) {
            ensureMigrated()
        }
        val ids = hostIds()
        // Deduplicate same endpoint: re-pairing an already-paired host should replace, not duplicate.
        val duplicateIds = if (endpoint.isNotBlank()) ids.filter { it != credential.deviceId && prefs.getString(hostEndpointKey(it), null) == endpoint } else emptyList()
        val idsAfterDedup = ids - duplicateIds.toSet()
        val isNew = !idsAfterDedup.contains(credential.deviceId)
        if (isNew) require(idsAfterDedup.size < MAX_PAIRED_HOSTS) { "too many paired hosts" }
        val encoded = encryptSecret(credential.deviceSecret)
        val newIds = if (isNew) (idsAfterDedup + credential.deviceId) else idsAfterDedup
        val version = prefs.getLong("committed_version", 0L) + 1L
        val editor = prefs.edit()
            .putStringSet("host_ids", newIds)
            .putString(hostSecretKey(credential.deviceId), encoded)
            .putString(hostEndpointKey(credential.deviceId), endpoint)
            .putString(hostEpochKey(credential.deviceId), "0")
            .putLong("committed_version", version)
        duplicateIds.forEach { oldId ->
            editor.remove(hostSecretKey(oldId)).remove(hostEndpointKey(oldId)).remove(hostEpochKey(oldId))
                .remove(hostPendingIdKey(oldId)).remove(hostPendingExpiryKey(oldId))
        }
        if (pending != null) editor.putString(hostPendingIdKey(credential.deviceId), pending.enrollmentId).putLong(hostPendingExpiryKey(credential.deviceId), pending.expiresAtMs)
        else editor.remove(hostPendingIdKey(credential.deviceId)).remove(hostPendingExpiryKey(credential.deviceId))
        // Also keep legacy pointer in sync for callers that still read it before migration completes.
        editor.putString("record_${version}_device_id", credential.deviceId)
            .putString("record_${version}_secret", encoded)
            .putString("record_${version}_endpoint", endpoint)
            .putString("record_${version}_accepted_epoch", "0")
        if (pending != null) editor.putString("record_${version}_pending_id", pending.enrollmentId).putLong("record_${version}_pending_expiry", pending.expiresAtMs)
        else editor.remove("record_${version}_pending_id").remove("record_${version}_pending_expiry")
        check(commit(editor)) { "unable to commit phone-control credential" }
        DirectLinkReplacementNotifier.notifyCommitted()
    }

    override fun loadAll(): List<HostRecord> {
        if (hasMultiHosts()) {
            return hostIds().sorted().mapNotNull { readHostRecord(it) }.map { it.copy(deviceSecret = it.deviceSecret.copyOf()) }
        }
        val legacy = loadLegacy() ?: return emptyList()
        val version = prefs.getLong("committed_version", 0L)
        val prefix = if (version > 0) "record_${version}_" else ""
        val endpoint = prefs.getString(prefix + "endpoint", prefs.getString("endpoint", "")) ?: ""
        val epoch = if (version > 0) prefs.getString("record_${version}_accepted_epoch", "0") ?: "0" else prefs.getString("accepted_epoch", "0") ?: "0"
        val pendingId = if (version > 0) prefs.getString("record_${version}_pending_id", null) else null
        val pendingExpiry = if (version > 0) prefs.getLong("record_${version}_pending_expiry", 0L) else 0L
        val pending = if (pendingId != null && pendingExpiry > 0) PendingEnrollment(pendingId, pendingExpiry) else null
        return listOf(HostRecord(legacy.deviceId, legacy.deviceSecret.copyOf(), endpoint, epoch, pending))
    }

    override fun loadHost(deviceId: String): HostRecord? {
        if (hasMultiHosts()) return readHostRecord(deviceId)?.let { it.copy(deviceSecret = it.deviceSecret.copyOf()) }
        val legacy = loadLegacy() ?: return null
        if (legacy.deviceId != deviceId) return null
        val version = prefs.getLong("committed_version", 0L)
        val prefix = if (version > 0) "record_${version}_" else ""
        val endpoint = prefs.getString(prefix + "endpoint", prefs.getString("endpoint", "")) ?: ""
        val epoch = if (version > 0) prefs.getString("record_${version}_accepted_epoch", "0") ?: "0" else prefs.getString("accepted_epoch", "0") ?: "0"
        val pendingId = if (version > 0) prefs.getString("record_${version}_pending_id", null) else null
        val pendingExpiry = if (version > 0) prefs.getLong("record_${version}_pending_expiry", 0L) else 0L
        val pending = if (pendingId != null && pendingExpiry > 0) PendingEnrollment(pendingId, pendingExpiry) else null
        return HostRecord(legacy.deviceId, legacy.deviceSecret.copyOf(), endpoint, epoch, pending)
    }

    override fun deleteHost(deviceId: String) {
        if (!hasMultiHosts()) {
            val legacy = loadLegacy()
            if (legacy?.deviceId == deviceId) clear()
            return
        }
        val ids = hostIds()
        if (!ids.contains(deviceId)) return
        val newIds = ids - deviceId
        val editor = prefs.edit().putStringSet("host_ids", newIds)
            .remove(hostSecretKey(deviceId)).remove(hostEndpointKey(deviceId)).remove(hostEpochKey(deviceId))
            .remove(hostPendingIdKey(deviceId)).remove(hostPendingExpiryKey(deviceId))
        check(commit(editor)) { "unable to delete host" }
        DirectLinkReplacementNotifier.notifyCommitted()
    }

    override fun clear() { check(commit(prefs.edit().clear())) { "unable to clear phone-control credential" } }
    override fun pendingEnrollment(): PendingEnrollment? {
        if (hasMultiHosts()) {
            val first = hostIds().sorted().firstOrNull() ?: return null
            return readHostRecord(first)?.pendingEnrollment
        }
        val v = prefs.getLong("committed_version", 0L); if (v <= 0) return null
        val id = prefs.getString("record_${v}_pending_id", null) ?: return null
        val expiry = prefs.getLong("record_${v}_pending_expiry", 0L)
        return PendingEnrollment(id, expiry).takeIf { expiry > 0 }
    }
    override fun clearPendingEnrollment() {
        if (hasMultiHosts()) {
            val first = hostIds().sorted().firstOrNull() ?: return
            check(commit(prefs.edit().remove(hostPendingIdKey(first)).remove(hostPendingExpiryKey(first))))
            return
        }
        val v = prefs.getLong("committed_version", 0L); if (v > 0) check(commit(prefs.edit().remove("record_${v}_pending_id").remove("record_${v}_pending_expiry")))
    }
    override fun lastAcceptedEpoch(): String {
        if (hasMultiHosts()) {
            val first = hostIds().sorted().firstOrNull() ?: return "0"
            return prefs.getString(hostEpochKey(first), "0") ?: "0"
        }
        val version = prefs.getLong("committed_version", 0L)
        return prefs.getString(if (version > 0) "record_${version}_accepted_epoch" else "accepted_epoch", null) ?: "0"
    }
    override fun saveAcceptedEpoch(epoch: String) {
        if (hasMultiHosts()) {
            val first = hostIds().sorted().firstOrNull() ?: return
            check(commit(prefs.edit().putString(hostEpochKey(first), epoch))) { "unable to persist accepted link epoch" }
            // Keep legacy pointer in sync as well
            val v = prefs.getLong("committed_version", 0L)
            if (v > 0) check(commit(prefs.edit().putString("record_${v}_accepted_epoch", epoch))) {}
            return
        }
        val version = prefs.getLong("committed_version", 0L)
        val key = if (version > 0) "record_${version}_accepted_epoch" else "accepted_epoch"
        check(commit(prefs.edit().putString(key, epoch))) { "unable to persist accepted link epoch" }
    }

    /** Per-host epoch helpers used by the multi-host pool. */
    fun lastAcceptedEpochForHost(deviceId: String): String {
        if (hasMultiHosts()) return prefs.getString(hostEpochKey(deviceId), null) ?: "0"
        val version = prefs.getLong("committed_version", 0L)
        return prefs.getString(if (version > 0) "record_${version}_accepted_epoch" else "accepted_epoch", null) ?: "0"
    }
    fun saveAcceptedEpochForHost(deviceId: String, epoch: String) {
        if (hasMultiHosts()) {
            check(commit(prefs.edit().putString(hostEpochKey(deviceId), epoch))) { "unable to persist accepted link epoch" }
            return
        }
        saveAcceptedEpoch(epoch)
    }
    fun pendingEnrollmentForHost(deviceId: String): PendingEnrollment? {
        if (hasMultiHosts()) return readHostRecord(deviceId)?.pendingEnrollment
        return pendingEnrollment()?.takeIf { load()?.deviceId == deviceId }
    }
    fun clearPendingForHost(deviceId: String) {
        if (hasMultiHosts()) { check(commit(prefs.edit().remove(hostPendingIdKey(deviceId)).remove(hostPendingExpiryKey(deviceId)))); return }
        clearPendingEnrollment()
    }

    private fun commit(editor: android.content.SharedPreferences.Editor): Boolean = editor.commit()

    private fun key(): SecretKey {
        testKey?.let { return it }
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

/** Per-host view over a multi-host store; used by each DirectLinkController. */
internal class HostScopedCredentialStore(
    private val delegate: AndroidCredentialStore,
    private val deviceId: String,
) : CredentialStore {
    override fun load(): DeviceCredential? = delegate.loadHost(deviceId)?.let { DeviceCredential(it.deviceId, it.deviceSecret) }
    override fun save(credential: DeviceCredential) = delegate.saveEnrollment(credential, delegate.loadHost(deviceId)?.endpoint ?: "", delegate.pendingEnrollmentForHost(deviceId))
    override fun saveEnrollment(credential: DeviceCredential, endpoint: String, pending: PendingEnrollment?) = delegate.saveEnrollment(credential, endpoint, pending)
    override fun pendingEnrollment(): PendingEnrollment? = delegate.pendingEnrollmentForHost(deviceId)
    override fun clearPendingEnrollment() = delegate.clearPendingForHost(deviceId)
    override fun clear() = delegate.deleteHost(deviceId)
    override fun lastAcceptedEpoch(): String = delegate.lastAcceptedEpochForHost(deviceId)
    override fun saveAcceptedEpoch(epoch: String) = delegate.saveAcceptedEpochForHost(deviceId, epoch)
}

internal class MemoryHostScopedCredentialStore(
    private val delegate: MemoryCredentialStore,
    private val deviceId: String,
) : CredentialStore {
    override fun load(): DeviceCredential? = delegate.loadHost(deviceId)?.let { DeviceCredential(it.deviceId, it.deviceSecret) }
    override fun save(credential: DeviceCredential) = delegate.saveEnrollment(credential, delegate.loadHost(deviceId)?.endpoint ?: "", delegate.pendingEnrollmentForHost(deviceId))
    override fun saveEnrollment(credential: DeviceCredential, endpoint: String, pending: PendingEnrollment?) = delegate.saveEnrollment(credential, endpoint, pending)
    override fun pendingEnrollment(): PendingEnrollment? = delegate.pendingEnrollmentForHost(deviceId)
    override fun clearPendingEnrollment() = delegate.clearPendingForHost(deviceId)
    override fun clear() = delegate.deleteHost(deviceId)
    override fun lastAcceptedEpoch(): String = delegate.lastAcceptedEpochForHost(deviceId)
    override fun saveAcceptedEpoch(epoch: String) = delegate.saveAcceptedEpochForHost(deviceId, epoch)
}
