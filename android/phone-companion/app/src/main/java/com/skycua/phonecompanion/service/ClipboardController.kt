package com.skycua.phonecompanion.service

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.PersistableBundle
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.MethodParamException
import java.util.concurrent.atomic.AtomicLong
import com.skycua.phonecompanion.direct.CompanionContentProvider
import com.skycua.phonecompanion.direct.DirectContentResolver

/** Android-native clipboard operations. Binary items remain URI-backed. */
class ClipboardController(
    context: Context,
    private val contentResolver: DirectContentResolver? = null,
) {
    private val clipboard = context.getSystemService(ClipboardManager::class.java)
    private val sequence = AtomicLong(0)

    init {
        clipboard?.addPrimaryClipChangedListener { sequence.incrementAndGet() }
    }

    fun perform(params: JsonValue.Obj): JsonValue.Obj =
        when (val operation = params.string("operation")) {
            "get" -> response(readPayload())
            "set" -> {
                val payload = params.obj("payload") ?: bad("clipboard set requires payload")
                clipboardOrThrow().setPrimaryClip(parseClip(payload))
                response(readPayload())
            }
            "clear" -> {
                clipboardOrThrow().clearPrimaryClip()
                response(null)
            }
            "changes" -> {
                val since = params.long("since_sequence") ?: 0L
                val current = sequence.get()
                jsonObject {
                    put("sequence", current)
                    put("changes", jsonArray(if (current > since) listOfNotNull(readPayload()) else emptyList()))
                }
            }
            null -> bad("clipboard operation is required")
            else -> bad("unsupported clipboard operation '$operation'")
        }

    private fun response(payload: JsonValue.Obj?): JsonValue.Obj =
        jsonObject {
            if (payload != null) put("payload", payload)
            put("sequence", sequence.get())
            put("changes", jsonArray(emptyList()))
        }

    private fun readPayload(): JsonValue.Obj? {
        val clip = clipboardOrThrow().primaryClip ?: return null
        val description = clip.description
        val items = (0 until clip.itemCount).map { index ->
            val item = clip.getItemAt(index)
            jsonObject {
                item.text?.toString()?.let { put("text", it) }
                item.htmlText?.let { put("html", it) }
                item.uri?.toString()?.let { put("uri", it) }
                item.uri?.takeIf { it.authority == CompanionContentProvider.AUTHORITY }
                    ?.lastPathSegment?.let { contentResolver?.describe(it) }?.let { content ->
                        put("content", jsonObject {
                            put("content_id", content.contentId); put("device_id", content.deviceId); put("link_epoch", content.linkEpoch)
                            put("mime_type", content.mimeType); content.filename?.let { put("filename", it) }
                            put("size_bytes", content.sizeBytes); put("sha256", content.sha256); put("source", "host_private_artifact")
                            content.expiresAtMs?.let { put("expires_at_ms", it) }; put("persistence", "temporary")
                        })
                    }
                item.intent?.toUri(Intent.URI_INTENT_SCHEME)?.let { put("intent_uri", it) }
                put("mime_types", jsonArray((0 until description.mimeTypeCount).map { JsonValue.Str(description.getMimeType(it)) }))
            }
        }
        return jsonObject {
            description.label?.toString()?.let { put("label", it) }
            put("items", jsonArray(items))
            put("sensitive", description.extras?.getBoolean("android.content.extra.IS_SENSITIVE", false) ?: false)
            put("changed_at_ms", System.currentTimeMillis())
        }
    }

    private fun parseClip(payload: JsonValue.Obj): ClipData {
        val rawItems = payload.arr("items")?.items ?: bad("clipboard payload requires items")
        if (rawItems.isEmpty()) bad("clipboard payload requires at least one item")
        val items = rawItems.map { raw ->
            val item = raw as? JsonValue.Obj ?: bad("clipboard item must be an object")
            val text = item.string("text")
            val html = item.string("html")
            val uri = item.string("uri")?.let(Uri::parse)
                ?: item.obj("content")?.let { reference ->
                    val epoch = reference.long("link_epoch") ?: bad("content link_epoch is required")
                    val content = contentResolver?.resolve(reference, epoch)
                        ?: throw MethodApplicationException("content_not_found", "clipboard content is unavailable or expired")
                    CompanionContentProvider.uri(content.contentId)
                }
            val intent = item.string("intent_uri")?.let { Intent.parseUri(it, Intent.URI_INTENT_SCHEME) }
            if (text == null && html == null && uri == null && intent == null) {
                bad("clipboard item requires text, html, content, uri, or intent_uri")
            }
            ClipData.Item(text, html, intent, uri)
        }
        val label = payload.string("label") ?: "Sky CUA"
        val mimeTypes = rawItems.flatMap { raw ->
            ((raw as? JsonValue.Obj)?.arr("mime_types")?.items ?: emptyList()).mapNotNull { (it as? JsonValue.Str)?.value }
        }.distinct().ifEmpty {
            rawItems.mapNotNull { (it as? JsonValue.Obj)?.obj("content")?.string("mime_type") }.distinct()
        }.ifEmpty { listOf("text/plain") }.toTypedArray()
        val description = ClipDescription(label, mimeTypes)
        description.extras = PersistableBundle().apply {
            putBoolean("android.content.extra.IS_SENSITIVE", payload.bool("sensitive") ?: false)
        }
        val clip = ClipData(description, items.first())
        items.drop(1).forEach(clip::addItem)
        return clip
    }

    private fun clipboardOrThrow(): ClipboardManager = clipboard ?: throw MethodApplicationException(
        "unsupported_api",
        "clipboard service is unavailable",
    )

    private fun bad(message: String): Nothing = throw MethodParamException("bad_request", message)
}
