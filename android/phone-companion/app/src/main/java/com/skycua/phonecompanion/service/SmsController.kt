package com.skycua.phonecompanion.service

import android.Manifest
import android.app.AppOpsManager
import android.content.Context
import android.content.pm.PackageManager
import android.database.Cursor
import android.os.Process
import android.provider.Telephony
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.json.JsonParser
import com.skycua.phonecompanion.json.jsonArray
import com.skycua.phonecompanion.json.jsonObject
import com.skycua.phonecompanion.json.longValueExact
import com.skycua.phonecompanion.protocol.MethodApplicationException
import java.util.Base64

/** Observation-only Telephony.Sms reader for the direct phone-control.v2 lane. */
class SmsController(private val context: Context) {
    fun isReadable(): Boolean = permissionError() == null

    fun query(params: JsonValue.Obj): JsonValue.Obj {
        val start = requiredLong(params, "start_ms")
        val end = requiredLong(params, "end_ms")
        val limit = params["limit"]?.let { value ->
            value.longValueExact()
                ?: fail("INVALID_ARGUMENT", "limit must be an integer")
        } ?: DEFAULT_LIMIT
        if (start < 0 || end <= start || limit !in 1..MAX_LIMIT) {
            fail("INVALID_ARGUMENT", "start_ms/end_ms must form a non-empty window and limit must be 1..500")
        }
        val cursor = params.string("cursor")?.let { decodeCursor(it, start, end) }
        permissionError()?.let { fail(it, "READ_SMS is not available to the companion") }

        val (selection, selectionArgs) = smsSelection(start, end, cursor?.date, cursor?.id)

        val result = try {
            context.contentResolver.query(
                Telephony.Sms.CONTENT_URI,
                SMS_PROVIDER_PROJECTION,
                selection,
                selectionArgs,
                "date ASC, _id ASC LIMIT ${limit + 1}",
            ) ?: fail("SMS_PROVIDER_UNAVAILABLE", "Telephony.Sms returned no cursor")
        } catch (e: SecurityException) {
            fail("SMS_PERMISSION_RESTRICTED", e.message ?: "Telephony.Sms access was restricted")
        } catch (e: MethodApplicationException) {
            throw e
        } catch (e: Exception) {
            fail("SMS_QUERY_FAILED", e.message ?: "Telephony.Sms query failed")
        }

        result.use { rows ->
            val records = buildList {
                while (rows.moveToNext()) add(readRecord(rows))
            }
            val hasMore = records.size > limit
            val page = records.take(limit.toInt())
            val next = if (hasMore) {
                val last = page.lastOrNull()
                    ?: fail("SMS_QUERY_FAILED", "provider returned an empty page with more rows")
                val date = last.longValue("date")
                    ?: fail("SMS_QUERY_FAILED", "provider returned a row without date")
                val id = last.longValue("_id")
                    ?: fail("SMS_QUERY_FAILED", "provider returned a row without _id")
                encodeCursor(CursorToken(start, end, date, id))
            } else null
            return jsonObject {
                put("messages", jsonArray(page.map { it.json }))
                put("next_cursor", next)
                put("scan", jsonObject {
                    put("has_more", hasMore)
                    put("exhausted_as_observed", !hasMore)
                    put("snapshot", false)
                    put("observed_at_ms", System.currentTimeMillis())
                })
            }
        }
    }

    private fun readRecord(cursor: Cursor): RawRecord {
        val values = LinkedHashMap<String, JsonValue>()
        for (column in OUTPUT_COLUMNS) {
            val index = cursor.getColumnIndex(column)
            val value = if (index < 0 || cursor.isNull(index)) {
                JsonValue.Null
            } else if (column in LONG_COLUMNS) {
                JsonValue.of(cursor.getLong(index))
            } else {
                JsonValue.of(cursor.getString(index))
            }
            values[column] = value
        }
        return RawRecord(JsonValue.Obj(values))
    }

    private fun permissionError(): String? {
        if (context.checkSelfPermission(Manifest.permission.READ_SMS) == PackageManager.PERMISSION_GRANTED) return null
        val appOps = context.getSystemService(AppOpsManager::class.java)
        val mode = appOps?.unsafeCheckOpNoThrow(
            AppOpsManager.OPSTR_READ_SMS,
            Process.myUid(),
            context.packageName,
        )
        return if (mode == AppOpsManager.MODE_ERRORED) "SMS_PERMISSION_RESTRICTED" else "SMS_PERMISSION_NOT_GRANTED"
    }

    private fun requiredLong(params: JsonValue.Obj, key: String): Long {
        return params.long(key)
            ?: fail("INVALID_ARGUMENT", "$key must be an integer")
    }

    private fun decodeCursor(raw: String, start: Long, end: Long): CursorToken {
        if (raw.isEmpty()) fail("INVALID_CURSOR", "cursor must not be empty")
        val obj = try {
            JsonParser.parseObject(String(Base64.getUrlDecoder().decode(raw), Charsets.UTF_8))
        } catch (_: Exception) {
            fail("INVALID_CURSOR", "cursor is not valid")
        }
        val token = CursorToken(
            start = cursorLong(obj, "start_ms"),
            end = cursorLong(obj, "end_ms"),
            date = cursorLong(obj, "date_ms"),
            id = cursorLong(obj, "id"),
        )
        val version = obj.long("v")
            ?: fail("INVALID_CURSOR", "cursor is missing version")
        if (version != 1L) fail("INVALID_CURSOR", "cursor version is unsupported")
        if (token.start != start || token.end != end) fail("CURSOR_QUERY_MISMATCH", "cursor belongs to a different query window")
        if (token.date < start || token.date >= end || token.id < 0) {
            fail("INVALID_CURSOR", "cursor key is outside the query window")
        }
        return token
    }

    private fun cursorLong(obj: JsonValue.Obj, key: String): Long =
        obj.long(key)
            ?: fail("INVALID_CURSOR", "cursor is missing or invalid $key")

    private fun encodeCursor(token: CursorToken): String {
        val json = com.skycua.phonecompanion.json.JsonWriter.write(jsonObject {
            put("v", 1L)
            put("start_ms", token.start)
            put("end_ms", token.end)
            put("date_ms", token.date)
            put("id", token.id)
        })
        return Base64.getUrlEncoder().withoutPadding().encodeToString(json.toByteArray(Charsets.UTF_8))
    }

    private fun RawRecord.longValue(column: String): Long? =
        json.long(column)

    private data class RawRecord(val json: JsonValue.Obj)
    private data class CursorToken(val start: Long, val end: Long, val date: Long, val id: Long)

    companion object {
        const val DEFAULT_LIMIT = 250L
        const val MAX_LIMIT = 500L
        // Query only the stable identity/ingress subset. OEM restricted SMS
        // views may omit optional AOSP columns (for example `priority`); the
        // wire contract still emits those fields as null below.
        internal val SMS_PROVIDER_PROJECTION = arrayOf(
            "_id", "thread_id", "address", "date", "date_sent", "read", "status", "type", "body",
        )
        private val OUTPUT_COLUMNS = arrayOf(
            "_id", "thread_id", "address", "person", "date", "date_sent", "protocol", "read",
            "status", "type", "reply_path_present", "subject", "body", "service_center", "locked",
            "sub_id", "creator", "seen", "priority", "subscription_id", "error_code", "message_class",
        )
        private val LONG_COLUMNS = setOf(
            "_id", "thread_id", "date", "date_sent", "protocol", "read", "status", "type",
            "reply_path_present", "locked", "sub_id", "seen", "priority", "subscription_id", "error_code",
        )

        private fun fail(code: String, message: String): Nothing =
            throw MethodApplicationException(code, message)
    }
}

internal fun smsSelection(
    start: Long,
    end: Long,
    cursorDate: Long?,
    cursorId: Long?,
): Pair<String, Array<String>> {
    val selectionParts = mutableListOf("date >= ?", "date < ?", "type = ?")
    val selectionArgs = mutableListOf(
        start.toString(),
        end.toString(),
        Telephony.Sms.MESSAGE_TYPE_INBOX.toString(),
    )
    if (cursorDate != null && cursorId != null) {
        selectionParts += "(date > ? OR (date = ? AND _id > ?))"
        selectionArgs += cursorDate.toString()
        selectionArgs += cursorDate.toString()
        selectionArgs += cursorId.toString()
    }
    return selectionParts.joinToString(" AND ") to selectionArgs.toTypedArray()
}
