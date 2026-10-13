package com.rivalapexmediation.ctv.consent

import java.util.Base64

/**
 * IAB TCF v2 parser for CTV.
 *
 * FAIL-CLOSED contract (adversarial case K4): any string that is not a structurally valid
 * TCF v2 core string yields `purpose1Consent = false`. The old heuristic ("has a dot and
 * base64url characters -> consent granted") failed OPEN for meaningless strings such as
 * "AAAA.BBBB.CCCC"; this implementation decodes the actual bitfield and reads the
 * Purpose 1 consent bit, and refuses everything it cannot decode.
 *
 * Structure (TCF v2 spec): three dot-separated base64url segments
 *   segment 1 (core):     6-bit version (must be 2), 36-bit created, 36-bit lastUpdated,
 *                         12-bit cmpId, 12-bit cmpVersion, 6-bit consentScreen, 12-bit
 *                         consentLanguage, 12-bit vendorListVersion, 6-bit tcfPolicyVersion,
 *                         1-bit isServiceSpecific, 1-bit useNonStandardStacks,
 *                         12-bit specialFeatureOptIns, 24-bit purposesConsent (MSB-first:
 *                         purpose 1 first), ...
 *   segment 2 (vendors):  disclosed-vendors bitfield (must be decodable)
 *   segment 3 (publisher): publisher restrictions (must be decodable)
 */
object TCFParser {
    data class Core(
        val gdprApplies: Boolean,
        val purpose1Consent: Boolean,
        val raw: String,
    )

    private const val TCF_V2_VERSION = 2

    @JvmStatic
    fun parse(tcfV2: String?, gdprApplies: Boolean): Core {
        if (tcfV2.isNullOrEmpty()) return Core(gdprApplies, false, "")
        return try {
            val purpose1 = decodePurpose1Consent(tcfV2) ?: return Core(gdprApplies, false, tcfV2)
            Core(gdprApplies, purpose1, tcfV2)
        } catch (_: Throwable) {
            Core(gdprApplies, false, tcfV2)
        }
    }

    /** Returns the Purpose 1 consent bit, or null when the string is not valid TCF v2. */
    private fun decodePurpose1Consent(tcfV2: String): Boolean? {
        val segments = tcfV2.split('.')
        // TCF v2: Core + Disclosed Vendors are REQUIRED; Publisher TC is OPTIONAL.
        if (segments.size < 2 || segments.size > 3) return null

        val core = decodeBase64Url(segments[0]) ?: return null
        // Disclosed vendors segment is required and must be decodable.
        if (decodeBase64Url(segments[1]) == null) return null
        // Optional publisher segment: when present it must be decodable (empty = absent).
        if (segments.size == 3 && segments[2].isNotEmpty() && decodeBase64Url(segments[2]) == null) return null

        val bits = BitReader(core)
        if (bits.read(6) != TCF_V2_VERSION) return null

        // created(36) lastUpdated(36) cmpId(12) cmpVersion(12) consentScreen(6)
        // consentLanguage(12) vendorListVersion(12) tcfPolicyVersion(6) isServiceSpecific(1)
        // useNonStandardStacks(1)
        bits.skip(36 + 36 + 12 + 12 + 6 + 12 + 12 + 6 + 1 + 1)
        bits.skip(12) // specialFeatureOptIns

        val purposesConsent = bits.read(24) // MSB-first: purpose 1 is the top bit
        return (purposesConsent ushr (24 - 1)) and 1 == 1
    }

    /** Decodes a TCF base64url segment (no padding) or returns null when invalid. */
    private fun decodeBase64Url(segment: String): ByteArray? {
        if (segment.isEmpty()) return null
        return try {
            val padded = when (segment.length % 4) {
                2 -> "$segment=="
                3 -> "$segment="
                0 -> segment
                else -> return null // length % 4 == 1 can never be valid base64
            }
            Base64.getUrlDecoder().decode(padded)
        } catch (_: IllegalArgumentException) {
            null
        }
    }

    private class BitReader(private val bytes: ByteArray) {
        private var position = 0

        fun read(count: Int): Int {
            var value = 0
            repeat(count) {
                val byteIndex = position / 8
                val bitIndex = 7 - (position % 8)
                val byte = if (byteIndex < bytes.size) bytes[byteIndex].toInt() else throw IllegalArgumentException("TCF bitfield truncated")
                value = (value shl 1) or ((byte shr bitIndex) and 1)
                position += 1
            }
            return value
        }

        fun skip(count: Int) {
            position += count
            if (position > bytes.size * 8) throw IllegalArgumentException("TCF bitfield truncated")
        }
    }
}
