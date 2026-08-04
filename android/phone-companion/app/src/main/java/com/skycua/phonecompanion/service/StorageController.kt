package com.skycua.phonecompanion.service

import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.content.ContentValues
import android.content.Intent
import android.webkit.MimeTypeMap
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.MethodParamException
import java.io.File
import java.security.MessageDigest
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.UUID
import com.skycua.phonecompanion.direct.DirectContentResolver
import com.skycua.phonecompanion.SafGrantActivity

/** Virtual storage roots backed by the Companion sandbox, shared storage, and content URIs. */
class StorageController(
    private val context: Context,
    private val contentResolver: DirectContentResolver? = null,
) {
    fun perform(params: JsonValue.Obj): JsonValue.Obj = when (val operation = params.string("operation")) {
        "roots", "list_saf_roots" -> roots()
        "list" -> list(requireUri(params))
        "stat" -> jsonObject { put("entries", jsonArray(listOf(storageEntry(requireUri(params))))) }
        "metadata" -> jsonObject { put("metadata", jsonObject { put("entry", storageEntry(requireUri(params))); put("vendor_extensions", jsonObject {}) }) }
        "read" -> read(requireUri(params))
        "mkdir" -> mutateUri(requireUri(params)) { file -> file.mkdirs() || file.isDirectory }
        "delete" -> delete(requireUri(params))
        "trash" -> trash(requireUri(params))
        "copy" -> copy(params, move = false)
        "move" -> copy(params, move = true)
        "rename" -> rename(params)
        "hash" -> hash(params)
        "search" -> search(params)
        "write" -> write(params)
        "thumbnail" -> throw MethodApplicationException("unsupported_api", "thumbnail generation is unavailable")
        "add_saf_root" -> {
            context.startActivity(Intent(context, SafGrantActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            throw MethodApplicationException("local_interaction_required", "the Companion opened Android's folder picker; choose a folder once, then retry")
        }
        "remove_saf_root" -> removeSafRoot(params.string("root_id") ?: bad("root_id is required"))
        null -> bad("storage operation is required")
        else -> bad("unsupported storage operation '$operation'")
    }

    private fun roots(): JsonValue.Obj {
        val roots = mutableListOf(
            root("companion_private", "app://private/", "Companion private", "companion_private", true),
        )
        val sharedReady = Environment.isExternalStorageManager()
        roots += root("shared_primary", "shared://primary/", "Shared storage", "shared", sharedReady)
        roots += root("media_images", "media://images/", "Images", "media_store", true)
        roots += root("media_video", "media://video/", "Videos", "media_store", true)
        roots += root("media_audio", "media://audio/", "Audio", "media_store", true)
        roots += root("media_downloads", "media://downloads/", "Downloads", "media_store", true)
        context.contentResolver.persistedUriPermissions.filter { it.isReadPermission }.forEach { permission ->
            roots += root(safRootId(permission.uri), permission.uri.toString(), permission.uri.lastPathSegment ?: "Documents", "saf", permission.isWritePermission)
        }
        return jsonObject { put("roots", jsonArray(roots)); put("entries", jsonArray(emptyList())) }
    }

    private fun root(id: String, uri: String, name: String, kind: String, write: Boolean): JsonValue.Obj =
        jsonObject {
            put("root_id", id); put("uri", uri); put("display_name", name); put("kind", kind)
            put("features", features(read = true, write = write))
        }

    private fun list(uri: String): JsonValue.Obj {
        if (uri.startsWith("media://")) return listMedia(uri)
        if (uri.startsWith("content://")) return listContent(uri)
        val directory = resolveFile(uri)
        if (!directory.isDirectory) throw MethodApplicationException("not_directory", "$uri is not a directory")
        val entries = directory.listFiles()?.sortedBy { it.name.lowercase() }?.map(::entry).orEmpty()
        return jsonObject { put("entries", jsonArray(entries)); put("roots", jsonArray(emptyList())) }
    }

    private fun read(uri: String): JsonValue.Obj {
        val bytes: ByteArray
        val mime: String
        val name: String?
        if (uri.startsWith("content://")) {
            val parsed = Uri.parse(uri)
            bytes = context.contentResolver.openInputStream(parsed)?.use { it.readBytes() }
                ?: throw MethodApplicationException("not_found", "content URI cannot be opened")
            mime = context.contentResolver.getType(parsed) ?: "application/octet-stream"
            name = null
        } else {
            val file = resolveFile(uri)
            if (!file.isFile) throw MethodApplicationException("not_found", "$uri is not a readable file")
            bytes = file.readBytes()
            mime = mime(file)
            name = file.name
        }
        return jsonObject {
            put("_content_base64", android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP))
            put("_content_mime", mime)
            put("_content_source", if (uri.startsWith("content://")) "content_uri" else "shared_path")
            name?.let { put("_content_filename", it) }
            put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList()))
        }
    }

    private fun write(params: JsonValue.Obj): JsonValue.Obj {
        val uri = requireUri(params)
        val reference = params.obj("content") ?: bad("content is required")
        val epoch = reference.long("link_epoch") ?: bad("content link_epoch is required")
        val received = contentResolver?.resolve(reference, epoch)
            ?: throw MethodApplicationException("content_not_found", "content is unavailable, expired, or belongs to another link epoch")
        if (uri.startsWith("content://") || uri.startsWith("media://")) {
            return writeContentUri(uri, received)
        }
        val destination = resolveFile(uri, allowMissing = true)
        if (destination.exists() && destination.isDirectory) {
            throw MethodApplicationException("is_directory", "$uri is a directory")
        }
        destination.parentFile?.mkdirs()
        val temporary = File(destination.parentFile, ".${destination.name}.${UUID.randomUUID()}.part")
        try {
            received.file.inputStream().use { input -> temporary.outputStream().use(input::copyTo) }
            val digest = MessageDigest.getInstance("SHA-256").digest(temporary.readBytes()).joinToString("") { "%02x".format(it) }
            if (temporary.length() != received.sizeBytes || digest != received.sha256) {
                throw MethodApplicationException("content_verification_failed", "content changed before storage commit")
            }
            runCatching {
                Files.move(temporary.toPath(), destination.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            }.getOrElse {
                Files.move(temporary.toPath(), destination.toPath(), StandardCopyOption.REPLACE_EXISTING)
            }
        } finally {
            temporary.delete()
        }
        return jsonObject { put("result_uri", uri); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
    }

    private fun copy(params: JsonValue.Obj, move: Boolean): JsonValue.Obj {
        val sourceUri = params.string("source") ?: bad("source is required")
        val destinationUri = params.string("destination") ?: bad("destination is required")
        val source = resolveFile(sourceUri)
        val destination = resolveFile(destinationUri, allowMissing = true)
        destination.parentFile?.mkdirs()
        if (source.isDirectory) source.copyRecursively(destination, overwrite = false) else source.copyTo(destination, overwrite = false)
        if (move) source.deleteRecursively()
        return jsonObject { put("result_uri", destinationUri); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
    }

    private fun rename(params: JsonValue.Obj): JsonValue.Obj {
        val uri = requireUri(params)
        val name = params.string("name") ?: bad("name is required")
        if (name == "." || name == ".." || name.contains('/') || name.contains('\\')) bad("name must be one path segment")
        val source = resolveFile(uri)
        val destination = File(source.parentFile, name)
        if (!source.renameTo(destination)) throw MethodApplicationException("io_error", "rename failed")
        return jsonObject { put("result_uri", uri.substringBeforeLast('/') + "/" + name); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
    }

    private fun hash(params: JsonValue.Obj): JsonValue.Obj {
        val file = resolveFile(requireUri(params))
        val algorithm = (params.string("algorithm") ?: "sha256").lowercase()
        if (algorithm != "sha256") bad("only sha256 is supported")
        val digest = MessageDigest.getInstance("SHA-256").digest(file.readBytes()).joinToString("") { "%02x".format(it) }
        return jsonObject { put("result_uri", "sha256:$digest"); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
    }

    private fun search(params: JsonValue.Obj): JsonValue.Obj {
        val root = resolveFile(params.string("root") ?: bad("root is required"))
        val query = params.string("query")?.lowercase() ?: bad("query is required")
        val limit = (params.int("limit") ?: 100).coerceIn(1, 1000)
        val matches = root.walkTopDown().filter { it.name.lowercase().contains(query) }.take(limit).map(::entry).toList()
        return jsonObject { put("entries", jsonArray(matches)); put("roots", jsonArray(emptyList())) }
    }

    private fun mediaCollection(uri: String): Uri = when {
        uri.startsWith("media://images/") -> MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        uri.startsWith("media://video/") -> MediaStore.Video.Media.EXTERNAL_CONTENT_URI
        uri.startsWith("media://audio/") -> MediaStore.Audio.Media.EXTERNAL_CONTENT_URI
        uri.startsWith("media://downloads/") -> MediaStore.Downloads.EXTERNAL_CONTENT_URI
        else -> bad("unknown MediaStore root")
    }

    private fun listMedia(uri: String): JsonValue.Obj {
        val collection = mediaCollection(uri)
        val entries = mutableListOf<JsonValue>()
        context.contentResolver.query(
            collection,
            arrayOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.MIME_TYPE, MediaStore.MediaColumns.SIZE, MediaStore.MediaColumns.DATE_MODIFIED),
            null, null, "${MediaStore.MediaColumns.DATE_MODIFIED} DESC",
        )?.use { cursor ->
            val id = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
            val name = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
            val mime = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.MIME_TYPE)
            val size = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
            val modified = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
            while (cursor.moveToNext()) {
                entries += contentEntry(
                    Uri.withAppendedPath(collection, cursor.getLong(id).toString()),
                    cursor.getString(name) ?: "media",
                    cursor.getString(mime),
                    cursor.getLong(size),
                    cursor.getLong(modified) * 1000L,
                )
            }
        }
        return jsonObject { put("entries", jsonArray(entries)); put("roots", jsonArray(emptyList())) }
    }

    private fun listContent(uri: String): JsonValue.Obj {
        val tree = Uri.parse(uri)
        if (!DocumentsContract.isTreeUri(tree)) throw MethodApplicationException("not_directory", "$uri is not a SAF tree")
        val documentId = DocumentsContract.getTreeDocumentId(tree)
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, documentId)
        val entries = mutableListOf<JsonValue>()
        context.contentResolver.query(
            children,
            arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE, DocumentsContract.Document.COLUMN_SIZE, DocumentsContract.Document.COLUMN_LAST_MODIFIED),
            null, null, null,
        )?.use { cursor ->
            val id = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val name = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
            val mime = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_MIME_TYPE)
            val size = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_SIZE)
            val modified = cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
            while (cursor.moveToNext()) {
                val child = DocumentsContract.buildDocumentUriUsingTree(tree, cursor.getString(id))
                entries += contentEntry(child, cursor.getString(name) ?: "document", cursor.getString(mime), cursor.getLong(size), cursor.getLong(modified))
            }
        }
        return jsonObject { put("entries", jsonArray(entries)); put("roots", jsonArray(emptyList())) }
    }

    private fun storageEntry(uri: String): JsonValue.Obj {
        if (!uri.startsWith("content://")) return entry(resolveFile(uri))
        val parsed = Uri.parse(uri)
        context.contentResolver.query(parsed, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                return contentEntry(
                    parsed,
                    cursor.getString(cursor.getColumnIndexOrThrow(OpenableColumns.DISPLAY_NAME)) ?: "content",
                    context.contentResolver.getType(parsed),
                    cursor.getLong(cursor.getColumnIndexOrThrow(OpenableColumns.SIZE)),
                    null,
                )
            }
        }
        throw MethodApplicationException("not_found", "$uri cannot be queried")
    }

    private fun contentEntry(uri: Uri, name: String, mime: String?, size: Long?, modified: Long?): JsonValue.Obj = jsonObject {
        put("uri", uri.toString()); put("name", name)
        put("kind", if (mime == DocumentsContract.Document.MIME_TYPE_DIR) "directory" else "content")
        mime?.takeUnless { it == DocumentsContract.Document.MIME_TYPE_DIR }?.let { put("mime_type", it) }
        size?.let { put("size_bytes", it) }; modified?.let { put("modified_at_ms", it) }
        put("features", features(true, true))
    }

    private fun writeContentUri(uri: String, content: com.skycua.phonecompanion.direct.ReceivedContent): JsonValue.Obj {
        val target = if (uri.startsWith("media://")) {
            val collection = mediaCollection(uri)
            val name = uri.substringAfterLast('/').takeIf { it.isNotBlank() } ?: content.filename ?: content.contentId
            context.contentResolver.insert(collection, ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, name)
                put(MediaStore.MediaColumns.MIME_TYPE, content.mimeType)
            }) ?: throw MethodApplicationException("io_error", "MediaStore insert failed")
        } else Uri.parse(uri)
        try {
            context.contentResolver.openOutputStream(target, "w")?.use { output -> content.file.inputStream().use { it.copyTo(output) } }
                ?: throw MethodApplicationException("io_error", "content destination cannot be opened")
        } catch (error: Throwable) {
            if (uri.startsWith("media://")) context.contentResolver.delete(target, null, null)
            throw error
        }
        return jsonObject { put("result_uri", target.toString()); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
    }

    private fun delete(uri: String): JsonValue.Obj {
        if (uri.startsWith("content://")) {
            val parsed = Uri.parse(uri)
            val deleted = runCatching { DocumentsContract.deleteDocument(context.contentResolver, parsed) }.getOrElse { context.contentResolver.delete(parsed, null, null) > 0 }
            if (!deleted) throw MethodApplicationException("io_error", "content delete failed")
            return jsonObject { put("result_uri", uri); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
        }
        return mutateUri(uri) { file -> if (file.isDirectory) file.deleteRecursively() else file.delete() }
    }

    private fun trash(uri: String): JsonValue.Obj {
        if (!uri.startsWith("content://")) throw MethodApplicationException("unsupported_api", "trash is unavailable for this storage root")
        val changed = context.contentResolver.update(Uri.parse(uri), ContentValues().apply { put(MediaStore.MediaColumns.IS_TRASHED, 1) }, null, null)
        if (changed <= 0) throw MethodApplicationException("local_interaction_required", "this provider requires system confirmation to trash the item")
        return jsonObject { put("result_uri", uri); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
    }

    private fun safRootId(uri: Uri): String = "saf_${uri.toString().hashCode().toUInt().toString(16)}"

    private fun removeSafRoot(rootId: String): JsonValue.Obj {
        val permission = context.contentResolver.persistedUriPermissions.firstOrNull { safRootId(it.uri) == rootId }
            ?: throw MethodApplicationException("unknown_root", "no matching persisted SAF root")
        var flags = 0
        if (permission.isReadPermission) flags = flags or Intent.FLAG_GRANT_READ_URI_PERMISSION
        if (permission.isWritePermission) flags = flags or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
        context.contentResolver.releasePersistableUriPermission(permission.uri, flags)
        return jsonObject { put("result_uri", permission.uri.toString()); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
    }

    private fun mutateUri(uri: String, action: (File) -> Boolean): JsonValue.Obj {
        if (!action(resolveFile(uri, allowMissing = true))) throw MethodApplicationException("io_error", "storage operation failed")
        return jsonObject { put("result_uri", uri); put("entries", jsonArray(emptyList())); put("roots", jsonArray(emptyList())) }
    }

    private fun entry(file: File): JsonValue.Obj = jsonObject {
        put("uri", toVirtualUri(file)); put("name", file.name.ifEmpty { "/" }); put("kind", if (file.isDirectory) "directory" else "file")
        if (file.isFile) { put("mime_type", mime(file)); put("size_bytes", file.length()) }
        put("modified_at_ms", file.lastModified()); put("features", features(true, canWrite(file)))
    }

    private fun features(read: Boolean, write: Boolean): JsonValue.Obj = jsonObject {
        put("read", read); put("write", write); put("random_access", true); put("rename", write); put("delete", write); put("trash", false)
    }

    private fun resolveFile(uri: String, allowMissing: Boolean = false): File {
        val (root, relative) = when {
            uri.startsWith("app://private/") -> context.filesDir to uri.removePrefix("app://private/")
            uri.startsWith("shared://primary/") -> {
                if (!Environment.isExternalStorageManager()) throw MethodApplicationException("permission_required", "all-files access is not enabled")
                Environment.getExternalStorageDirectory() to uri.removePrefix("shared://primary/")
            }
            else -> bad("unsupported or protected storage URI")
        }
        val target = File(root, relative).canonicalFile
        val canonicalRoot = root.canonicalFile
        if (target != canonicalRoot && !target.path.startsWith(canonicalRoot.path + File.separator)) bad("storage URI escapes its root")
        if (!allowMissing && !target.exists()) throw MethodApplicationException("not_found", "$uri does not exist")
        return target
    }

    private fun toVirtualUri(file: File): String {
        val target = file.canonicalFile
        val privateRoot = context.filesDir.canonicalFile
        if (target == privateRoot || target.path.startsWith(privateRoot.path + File.separator)) return "app://private/" + target.relativeTo(privateRoot).invariantSeparatorsPath
        val sharedRoot = Environment.getExternalStorageDirectory().canonicalFile
        return "shared://primary/" + target.relativeTo(sharedRoot).invariantSeparatorsPath
    }

    private fun mime(file: File): String = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "application/octet-stream"
    private fun canWrite(file: File): Boolean = file.canWrite() || file.parentFile?.canWrite() == true
    private fun requireUri(params: JsonValue.Obj): String = params.string("uri") ?: bad("uri is required")
    private fun bad(message: String): Nothing = throw MethodParamException("bad_request", message)
}
