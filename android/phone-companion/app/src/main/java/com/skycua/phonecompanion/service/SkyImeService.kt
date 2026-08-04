package com.skycua.phonecompanion.service

import android.content.ClipDescription
import android.content.Intent
import android.graphics.Color
import android.inputmethodservice.InputMethodService
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputContentInfo
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import com.skycua.phonecompanion.direct.CompanionContentProvider
import com.skycua.phonecompanion.direct.DirectContentResolver
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.protocol.MethodApplicationException

/** Optional IME provider for rich content insertion and editor-level actions. */
class SkyImeService : InputMethodService() {
    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    override fun onCreateInputView(): View =
        LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(24, 16, 24, 16)
            setBackgroundColor(0xff111827.toInt())
            addView(TextView(context).apply {
                text = "Sky remote input"
                setTextColor(Color.WHITE)
                textSize = 16f
            }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            addView(Button(context).apply {
                text = "Switch keyboard"
                setOnClickListener { switchToNextInputMethod(false) }
            })
        }

    private fun perform(params: JsonValue.Obj, contentResolver: DirectContentResolver?): JsonValue.Obj {
        val connection = currentInputConnection ?: unavailable()
        return when (params.string("operation")) {
            "context" -> contextResult()
            "insert_text" -> applied(connection.commitText(params.string("text") ?: "", 1))
            "set_text" -> {
                connection.performContextMenuAction(android.R.id.selectAll)
                applied(connection.commitText(params.string("text") ?: "", 1))
            }
            "set_selection" -> applied(connection.setSelection(params.int("start") ?: 0, params.int("end") ?: 0))
            "select_all" -> applied(connection.performContextMenuAction(android.R.id.selectAll))
            "copy" -> applied(connection.performContextMenuAction(android.R.id.copy))
            "cut" -> applied(connection.performContextMenuAction(android.R.id.cut))
            "paste" -> applied(connection.performContextMenuAction(android.R.id.paste))
            "insert_content" -> insertContent(connection, params, contentResolver)
            else -> throw MethodApplicationException("unsupported_api", "unsupported IME editor operation")
        }
    }

    private fun insertContent(
        connection: InputConnection,
        params: JsonValue.Obj,
        resolver: DirectContentResolver?,
    ): JsonValue.Obj {
        val reference = params.obj("content") ?: throw MethodApplicationException("bad_request", "content is required")
        val epoch = reference.long("link_epoch") ?: throw MethodApplicationException("bad_request", "content link_epoch is required")
        val content = resolver?.resolve(reference, epoch)
            ?: throw MethodApplicationException("content_not_found", "editor content is unavailable or expired")
        val accepted = currentInputEditorInfo?.contentMimeTypes?.toList().orEmpty()
        if (accepted.isNotEmpty() && accepted.none { ClipDescription.compareMimeTypes(it, content.mimeType) }) {
            return jsonObject {
                put("outcome", "unsupported_mime_type")
                put("accepted_mime_types", jsonArray(accepted.map(JsonValue::Str)))
            }
        }
        val uri = CompanionContentProvider.uri(content.contentId)
        currentInputEditorInfo?.packageName?.let {
            grantUriPermission(it, uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        val info = InputContentInfo(uri, ClipDescription(content.filename ?: "Sky content", arrayOf(content.mimeType)), null)
        val inserted = connection.commitContent(
            info,
            InputConnection.INPUT_CONTENT_GRANT_READ_URI_PERMISSION,
            Bundle(),
        )
        return jsonObject {
            put("outcome", if (inserted) "inserted_directly" else "unsupported_mime_type")
            put("accepted_mime_types", jsonArray(accepted.map(JsonValue::Str)))
        }
    }

    private fun contextResult(): JsonValue.Obj {
        val connection = currentInputConnection ?: unavailable()
        val selected = connection.getSelectedText(0)?.toString()
        val before = connection.getTextBeforeCursor(2048, 0)?.toString().orEmpty()
        val after = connection.getTextAfterCursor(2048, 0)?.toString().orEmpty()
        return jsonObject {
            put("outcome", "applied")
            put("surrounding_text", before + (selected ?: "") + after)
            put("selection_start", before.length.toLong())
            put("selection_end", (before.length + (selected?.length ?: 0)).toLong())
            put("accepted_mime_types", jsonArray(currentInputEditorInfo?.contentMimeTypes?.map(JsonValue::Str).orEmpty()))
        }
    }

    private fun applied(success: Boolean): JsonValue.Obj {
        if (!success) throw MethodApplicationException("editor_action_failed", "focused editor rejected the operation")
        return jsonObject { put("outcome", "applied"); put("accepted_mime_types", jsonArray(emptyList())) }
    }

    private fun unavailable(): Nothing =
        throw MethodApplicationException("ime_required", "Sky IME is not selected or no editor is focused")

    companion object {
        @Volatile private var instance: SkyImeService? = null
        fun isReady(): Boolean = instance?.currentInputConnection != null
        fun perform(params: JsonValue.Obj, contentResolver: DirectContentResolver?): JsonValue.Obj =
            instance?.perform(params, contentResolver)
                ?: throw MethodApplicationException("ime_required", "Sky IME is not selected")
    }
}
