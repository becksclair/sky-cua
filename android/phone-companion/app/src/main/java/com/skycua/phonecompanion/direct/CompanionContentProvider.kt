package com.skycua.phonecompanion.direct

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns

class CompanionContentProvider : ContentProvider() {
    override fun onCreate(): Boolean = true

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw IllegalArgumentException("Companion content is read-only")
        val content = content(uri) ?: throw java.io.FileNotFoundException(uri.toString())
        return ParcelFileDescriptor.open(content.file, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri): String? = content(uri)?.mimeType

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? {
        val content = content(uri) ?: return null
        val columns = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        return MatrixCursor(columns).apply {
            addRow(columns.map {
                when (it) {
                    OpenableColumns.DISPLAY_NAME -> content.filename ?: content.contentId
                    OpenableColumns.SIZE -> content.sizeBytes
                    else -> null
                }
            })
        }
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = 0
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0

    private fun content(uri: Uri): ReceivedContent? =
        uri.pathSegments.takeIf { it.size == 2 && it[0] == "content" }
            ?.get(1)
            ?.let(DirectContentRegistry::get)

    companion object {
        const val AUTHORITY = "com.skycua.phonecompanion.content"
        fun uri(contentId: String): Uri = Uri.Builder().scheme("content").authority(AUTHORITY)
            .appendPath("content").appendPath(contentId).build()
    }
}
