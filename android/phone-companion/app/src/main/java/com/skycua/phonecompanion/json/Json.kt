package com.skycua.phonecompanion.json

/**
 * A small, dependency-free JSON value model plus parser and serializer.
 *
 * The companion deliberately avoids org.json so the protocol layer is fully
 * unit-testable on a plain JVM (org.json is stubbed to default return values in
 * Android unit tests) and so the localhost RPC server has no heavy dependency.
 *
 * Only the subset of JSON the wire contract needs is supported: objects, arrays,
 * strings, numbers (parsed as Long when integral, otherwise Double), booleans,
 * and null.
 */
sealed class JsonValue {
    object Null : JsonValue()

    data class Bool(val value: Boolean) : JsonValue()

    data class Num(val value: Double) : JsonValue() {
        val isIntegral: Boolean
            get() = value == Math.floor(value) && !value.isInfinite() && !value.isNaN()

        fun toLong(): Long = value.toLong()

        fun toInt(): Int = value.toInt()
    }

    data class Str(val value: String) : JsonValue()

    data class Arr(val items: List<JsonValue>) : JsonValue()

    data class Obj(val entries: Map<String, JsonValue>) : JsonValue() {
        operator fun get(key: String): JsonValue? = entries[key]

        fun string(key: String): String? = (entries[key] as? Str)?.value

        fun bool(key: String): Boolean? = (entries[key] as? Bool)?.value

        fun long(key: String): Long? = (entries[key] as? Num)?.toLong()

        fun int(key: String): Int? = (entries[key] as? Num)?.toInt()

        fun obj(key: String): Obj? = entries[key] as? Obj

        fun arr(key: String): Arr? = entries[key] as? Arr
    }

    companion object {
        fun of(value: String?): JsonValue = if (value == null) Null else Str(value)

        fun of(value: Boolean): JsonValue = Bool(value)

        fun of(value: Long): JsonValue = Num(value.toDouble())

        fun of(value: Int): JsonValue = Num(value.toDouble())
    }
}

/** Builds a [JsonValue.Obj] in declaration order. */
class JsonObjectBuilder {
    private val entries = LinkedHashMap<String, JsonValue>()

    fun put(key: String, value: JsonValue): JsonObjectBuilder {
        entries[key] = value
        return this
    }

    fun put(key: String, value: String?): JsonObjectBuilder = put(key, JsonValue.of(value))

    fun put(key: String, value: Boolean): JsonObjectBuilder = put(key, JsonValue.of(value))

    fun put(key: String, value: Long): JsonObjectBuilder = put(key, JsonValue.of(value))

    fun put(key: String, value: Int): JsonObjectBuilder = put(key, JsonValue.of(value))

    fun putOpt(key: String, value: String?): JsonObjectBuilder {
        if (value != null) entries[key] = JsonValue.Str(value)
        return this
    }

    fun build(): JsonValue.Obj = JsonValue.Obj(entries)
}

fun jsonObject(block: JsonObjectBuilder.() -> Unit): JsonValue.Obj =
    JsonObjectBuilder().apply(block).build()

fun jsonArray(items: List<JsonValue>): JsonValue.Arr = JsonValue.Arr(items)

class JsonParseException(message: String) : Exception(message)

/** Serializes a [JsonValue] to a compact JSON string. */
object JsonWriter {
    fun write(value: JsonValue): String {
        val sb = StringBuilder()
        writeValue(sb, value)
        return sb.toString()
    }

    private fun writeValue(sb: StringBuilder, value: JsonValue) {
        when (value) {
            is JsonValue.Null -> sb.append("null")
            is JsonValue.Bool -> sb.append(if (value.value) "true" else "false")
            is JsonValue.Num -> {
                if (value.isIntegral) {
                    sb.append(value.toLong().toString())
                } else {
                    sb.append(value.value.toString())
                }
            }
            is JsonValue.Str -> writeString(sb, value.value)
            is JsonValue.Arr -> {
                sb.append('[')
                value.items.forEachIndexed { index, item ->
                    if (index > 0) sb.append(',')
                    writeValue(sb, item)
                }
                sb.append(']')
            }
            is JsonValue.Obj -> {
                sb.append('{')
                var first = true
                for ((k, v) in value.entries) {
                    if (!first) sb.append(',')
                    first = false
                    writeString(sb, k)
                    sb.append(':')
                    writeValue(sb, v)
                }
                sb.append('}')
            }
        }
    }

    private fun writeString(sb: StringBuilder, value: String) {
        sb.append('"')
        for (ch in value) {
            when (ch) {
                '"' -> sb.append("\\\"")
                '\\' -> sb.append("\\\\")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                '\b' -> sb.append("\\b")
                '\u000C' -> sb.append("\\f")
                else ->
                    if (ch < ' ') {
                        sb.append("\\u")
                        sb.append("%04x".format(ch.code))
                    } else {
                        sb.append(ch)
                    }
            }
        }
        sb.append('"')
    }
}

/** A recursive-descent JSON parser over a single document string. */
class JsonParser(private val text: String) {
    private var pos = 0

    companion object {
        /**
         * Maximum nesting depth for objects and arrays. The recursive-descent
         * parser would otherwise overflow the worker thread's stack on a deeply
         * nested document, which an unauthenticated localhost client could send
         * within the body cap. Exceeding this throws [JsonParseException] (a
         * caught type) instead of an uncaught StackOverflowError.
         */
        const val MAX_DEPTH = 64

        fun parse(text: String): JsonValue = JsonParser(text).parseDocument()

        fun parseObject(text: String): JsonValue.Obj =
            parse(text) as? JsonValue.Obj
                ?: throw JsonParseException("expected a JSON object at top level")
    }

    private fun parseDocument(): JsonValue {
        skipWhitespace()
        val value = parseValue(0)
        skipWhitespace()
        if (pos != text.length) {
            throw JsonParseException("trailing content after JSON document at $pos")
        }
        return value
    }

    private fun parseValue(depth: Int): JsonValue {
        skipWhitespace()
        if (pos >= text.length) throw JsonParseException("unexpected end of input")
        return when (val c = text[pos]) {
            '{' -> parseObject(depth)
            '[' -> parseArray(depth)
            '"' -> JsonValue.Str(parseString())
            't', 'f' -> parseBool()
            'n' -> parseNull()
            else ->
                if (c == '-' || c in '0'..'9') {
                    parseNumber()
                } else {
                    throw JsonParseException("unexpected character '$c' at $pos")
                }
        }
    }

    private fun enterNesting(depth: Int): Int {
        val next = depth + 1
        if (next > MAX_DEPTH) {
            throw JsonParseException("nesting depth exceeds maximum of $MAX_DEPTH at $pos")
        }
        return next
    }

    private fun parseObject(depth: Int): JsonValue.Obj {
        val childDepth = enterNesting(depth)
        expect('{')
        val entries = LinkedHashMap<String, JsonValue>()
        skipWhitespace()
        if (peek() == '}') {
            pos++
            return JsonValue.Obj(entries)
        }
        while (true) {
            skipWhitespace()
            if (peek() != '"') throw JsonParseException("expected object key at $pos")
            val key = parseString()
            skipWhitespace()
            expect(':')
            val value = parseValue(childDepth)
            entries[key] = value
            skipWhitespace()
            when (val c = next()) {
                ',' -> continue
                '}' -> break
                else -> throw JsonParseException("expected ',' or '}' but got '$c' at ${pos - 1}")
            }
        }
        return JsonValue.Obj(entries)
    }

    private fun parseArray(depth: Int): JsonValue.Arr {
        val childDepth = enterNesting(depth)
        expect('[')
        val items = ArrayList<JsonValue>()
        skipWhitespace()
        if (peek() == ']') {
            pos++
            return JsonValue.Arr(items)
        }
        while (true) {
            items.add(parseValue(childDepth))
            skipWhitespace()
            when (val c = next()) {
                ',' -> continue
                ']' -> break
                else -> throw JsonParseException("expected ',' or ']' but got '$c' at ${pos - 1}")
            }
        }
        return JsonValue.Arr(items)
    }

    private fun parseString(): String {
        expect('"')
        val sb = StringBuilder()
        while (true) {
            if (pos >= text.length) throw JsonParseException("unterminated string")
            val c = text[pos++]
            when (c) {
                '"' -> return sb.toString()
                '\\' -> {
                    if (pos >= text.length) throw JsonParseException("unterminated escape")
                    when (val esc = text[pos++]) {
                        '"' -> sb.append('"')
                        '\\' -> sb.append('\\')
                        '/' -> sb.append('/')
                        'n' -> sb.append('\n')
                        'r' -> sb.append('\r')
                        't' -> sb.append('\t')
                        'b' -> sb.append('\b')
                        'f' -> sb.append('\u000C')
                        'u' -> {
                            if (pos + 4 > text.length) {
                                throw JsonParseException("truncated unicode escape")
                            }
                            val hex = text.substring(pos, pos + 4)
                            pos += 4
                            val codePoint =
                                hex.toIntOrNull(16)
                                    ?: throw JsonParseException("invalid unicode escape '\\u$hex'")
                            sb.append(codePoint.toChar())
                        }
                        else -> throw JsonParseException("invalid escape '\\$esc' at ${pos - 1}")
                    }
                }
                else -> sb.append(c)
            }
        }
    }

    private fun parseNumber(): JsonValue.Num {
        val start = pos
        if (peek() == '-') pos++
        while (pos < text.length && (text[pos] in '0'..'9')) pos++
        if (pos < text.length && text[pos] == '.') {
            pos++
            while (pos < text.length && (text[pos] in '0'..'9')) pos++
        }
        if (pos < text.length && (text[pos] == 'e' || text[pos] == 'E')) {
            pos++
            if (pos < text.length && (text[pos] == '+' || text[pos] == '-')) pos++
            while (pos < text.length && (text[pos] in '0'..'9')) pos++
        }
        val raw = text.substring(start, pos)
        val parsed = raw.toDoubleOrNull() ?: throw JsonParseException("invalid number '$raw'")
        return JsonValue.Num(parsed)
    }

    private fun parseBool(): JsonValue.Bool =
        when {
            text.startsWith("true", pos) -> {
                pos += 4
                JsonValue.Bool(true)
            }
            text.startsWith("false", pos) -> {
                pos += 5
                JsonValue.Bool(false)
            }
            else -> throw JsonParseException("invalid literal at $pos")
        }

    private fun parseNull(): JsonValue.Null {
        if (text.startsWith("null", pos)) {
            pos += 4
            return JsonValue.Null
        }
        throw JsonParseException("invalid literal at $pos")
    }

    private fun skipWhitespace() {
        while (pos < text.length && text[pos].isWhitespace()) pos++
    }

    private fun peek(): Char {
        if (pos >= text.length) throw JsonParseException("unexpected end of input")
        return text[pos]
    }

    private fun next(): Char {
        if (pos >= text.length) throw JsonParseException("unexpected end of input")
        return text[pos++]
    }

    private fun expect(c: Char) {
        val actual = next()
        if (actual != c) throw JsonParseException("expected '$c' but got '$actual' at ${pos - 1}")
    }
}
