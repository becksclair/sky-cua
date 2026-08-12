package com.skycua.phonecompanion.direct

import com.skycua.phonecompanion.json.JsonObjectBuilder
import com.skycua.phonecompanion.json.JsonValue
import java.math.BigInteger

/** Exact phone-control.v2 link epoch across JSON and binary transport boundaries. */
class LinkEpoch private constructor(val value: BigInteger) : Comparable<LinkEpoch> {
    override fun compareTo(other: LinkEpoch): Int = value.compareTo(other.value)

    override fun toString(): String = value.toString()

    override fun equals(other: Any?): Boolean = other is LinkEpoch && value == other.value

    override fun hashCode(): Int = value.hashCode()

    fun toJson(): JsonValue.IntNum = JsonValue.IntNum(value)

    /** Carries the unsigned bits through ByteBuffer's signed Long API. */
    fun toBinaryCarrier(): Long = value.toLong()

    companion object {
        private val TWO_TO_64 = BigInteger.ONE.shiftLeft(64)
        private val MAX_VALUE = TWO_TO_64.subtract(BigInteger.ONE)
        val ZERO: LinkEpoch = LinkEpoch(BigInteger.ZERO)

        fun of(value: Long): LinkEpoch {
            require(value >= 0) { "link epoch must be unsigned" }
            return LinkEpoch(BigInteger.valueOf(value))
        }

        fun parseCanonical(value: String): LinkEpoch {
            require(value == "0" || (value.isNotEmpty() && value[0] in '1'..'9' && value.all { it in '0'..'9' })) {
                "epoch must be canonical unsigned decimal"
            }
            return fromBigInteger(BigInteger(value))
        }

        fun fromJson(value: JsonValue?): LinkEpoch? =
            (value as? JsonValue.IntNum)?.value?.let { runCatching { fromBigInteger(it) }.getOrNull() }

        fun fromBinaryCarrier(value: Long): LinkEpoch =
            LinkEpoch(if (value >= 0) BigInteger.valueOf(value) else BigInteger.valueOf(value).add(TWO_TO_64))

        private fun fromBigInteger(value: BigInteger): LinkEpoch {
            require(value.signum() >= 0 && value <= MAX_VALUE) { "epoch exceeds unsigned 64-bit range" }
            return LinkEpoch(value)
        }
    }
}

fun JsonObjectBuilder.put(key: String, value: LinkEpoch): JsonObjectBuilder = put(key, value.toJson())

fun JsonValue.Obj.linkEpoch(key: String): LinkEpoch? = LinkEpoch.fromJson(this[key])
