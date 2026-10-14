package com.rivalapexmediation.sdk.consent

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TCFParserTest {

    private class BitWriter {
        private val bits = StringBuilder()

        fun write(value: Long, count: Int) {
            for (i in count - 1 downTo 0) {
                bits.append(if ((value shr i) and 1L == 1L) '1' else '0')
            }
        }

        fun toBytes(): ByteArray {
            while (bits.length % 8 != 0) bits.append('0')
            return ByteArray(bits.length / 8) { idx ->
                bits.substring(idx * 8, idx * 8 + 8).toInt(2).toByte()
            }
        }
    }

    private fun tcfString(purpose1: Boolean): String {
        val w = BitWriter()
        w.write(2, 6) // version
        w.write(0, 36) // created
        w.write(0, 36) // lastUpdated
        w.write(0, 12) // cmpId
        w.write(0, 12) // cmpVersion
        w.write(0, 6) // consentScreen
        w.write(0, 12) // consentLanguage
        w.write(0, 12) // vendorListVersion
        w.write(2, 6) // tcfPolicyVersion
        w.write(0, 1) // isServiceSpecific
        w.write(0, 1) // useNonStandardStacks
        w.write(0, 12) // specialFeatureOptIns
        w.write(if (purpose1) 1L shl 23 else 0L, 24) // purposesConsent
        w.write(0, 24) // purposesLITransparency
        w.write(0, 1) // purposeOneTreatment
        w.write(0, 12) // publisherCC
        val core = java.util.Base64.getUrlEncoder().withoutPadding().encodeToString(w.toBytes())
        return core
    }

    @Test
    fun parse_reads_purpose1_from_the_core_segment() {
        val consented = TCFParser.parse(tcfString(purpose1 = true), gdprApplies = true)
        assertTrue(consented.purpose1Consent)
        val refused = TCFParser.parse(tcfString(purpose1 = false), gdprApplies = true)
        assertFalse(refused.purpose1Consent)
    }

    @Test
    fun parse_uses_only_the_first_segment() {
        val segmented = tcfString(purpose1 = true) + ".disclosedvendors"
        assertTrue(TCFParser.parse(segmented, gdprApplies = true).purpose1Consent)
    }

    @Test
    fun parse_fails_closed_on_missing_malformed_or_truncated_input() {
        assertFalse(TCFParser.parse(null, gdprApplies = true).purpose1Consent)
        assertFalse(TCFParser.parse("", gdprApplies = true).purpose1Consent)
        assertFalse(TCFParser.parse("!!!not-base64!!!", gdprApplies = true).purpose1Consent)
        val truncated = java.util.Base64.getUrlEncoder()
            .withoutPadding()
            .encodeToString(ByteArray(10))
        assertFalse(TCFParser.parse(truncated, gdprApplies = true).purpose1Consent)
        val empty = TCFParser.parse(null, gdprApplies = false)
        assertFalse(empty.gdprApplies)
    }

    @Test
    fun personalization_gate_matches_the_ios_rules() {
        val consenting = tcfString(purpose1 = true)
        assertTrue(
            ConsentManager.canShowPersonalizedAds(
                ConsentManager.State(gdprApplies = true, tcfString = consenting)
            )
        )
        assertFalse(
            ConsentManager.canShowPersonalizedAds(
                ConsentManager.State(gdprApplies = true, tcfString = tcfString(purpose1 = false))
            )
        )
        assertFalse(
            ConsentManager.canShowPersonalizedAds(
                ConsentManager.State(gdprApplies = true, tcfString = "garbage")
            )
        )
        assertFalse(
            ConsentManager.canShowPersonalizedAds(ConsentManager.State(gdprApplies = true))
        )
        assertFalse(
            ConsentManager.canShowPersonalizedAds(
                ConsentManager.State(gdprApplies = true, tcfString = consenting, coppa = true)
            )
        )
        assertFalse(
            ConsentManager.canShowPersonalizedAds(
                ConsentManager.State(gdprApplies = true, tcfString = consenting, usPrivacy = "1YNN")
            )
        )
        assertTrue(
            ConsentManager.canShowPersonalizedAds(
                ConsentManager.State(gdprApplies = true, tcfString = consenting, usPrivacy = "1NNN")
            )
        )
        assertTrue(ConsentManager.canShowPersonalizedAds(ConsentManager.State()))
    }
}
