package com.rivalapexmediation.sdk.consent

/**
 * Minimal IAB TCF v2 parser to extract a subset of consent signals for gating personalization.
 * Reads the core segment only: decodes the base64url payload and extracts Purpose 1
 * ("store and/or access information on a device") from the purposesConsent bitfield.
 * A missing, malformed, or truncated string yields fail-closed defaults.
 */
object TCFParser {
    data class Core(
        val gdprApplies: Boolean,
        val purpose1Consent: Boolean,
        val raw: String
    )

    // version(6) created(36) lastUpdated(36) cmpId(12) cmpVersion(12) consentScreen(6)
    // consentLanguage(12) vendorListVersion(12) tcfPolicyVersion(6) isServiceSpecific(1)
    // useNonStandardStacks(1) specialFeatureOptIns(12) then purposesConsent(24).
    private const val PURPOSES_CONSENT_OFFSET = 152

    /**
     * Parse a base64url encoded TCF v2 string and return the minimal subset.
     * On any decode or length failure, returns purpose1Consent = false and
     * preserves the raw string.
     */
    @JvmStatic
    fun parse(tcfV2: String?, gdprApplies: Boolean): Core {
        if (tcfV2.isNullOrEmpty()) return Core(gdprApplies, false, tcfV2 ?: "")
        val coreSegment = tcfV2.substringBefore('.')
        val bytes = decodeBase64Url(coreSegment) ?: return Core(gdprApplies, false, tcfV2)
        val reader = BitReader(bytes)
        if (!reader.skip(PURPOSES_CONSENT_OFFSET)) return Core(gdprApplies, false, tcfV2)
        val purposesConsent = reader.read(24) ?: return Core(gdprApplies, false, tcfV2)
        // Purpose 1 is bit 0, the most significant bit of the 24-bit field.
        val purpose1 = (purposesConsent shr 23) and 1 == 1
        return Core(gdprApplies, purpose1, tcfV2)
    }

    /** Dependency-free base64url decoder (RFC 4648 section 5, padding optional). */
    private fun decodeBase64Url(encoded: String): ByteArray? {
        val alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
        val out = StringBuilder(encoded.length * 6 / 8 + 1)
        var buffer = 0
        var bits = 0
        for (ch in encoded) {
            if (ch == '=') continue
            val value = alphabet.indexOf(ch)
            if (value < 0) return null
            buffer = (buffer shl 6) or value
            bits += 6
            if (bits >= 8) {
                bits -= 8
                out.append(((buffer shr bits) and 0xFF).toChar())
            }
        }
        return out.toString().toByteArray(Charsets.ISO_8859_1)
    }

    private class BitReader(private val bytes: ByteArray) {
        private var pos = 0
        private val remaining: Int get() = bytes.size * 8 - pos

        fun skip(count: Int): Boolean {
            if (remaining < count) return false
            pos += count
            return true
        }

        fun read(count: Int): Int? {
            if (count > 32 || remaining < count) return null
            var value = 0
            repeat(count) {
                val byte = bytes[pos / 8].toInt() and 0xFF
                value = (value shl 1) or ((byte shr (7 - pos % 8)) and 1)
                pos++
            }
            return value
        }
    }
}
