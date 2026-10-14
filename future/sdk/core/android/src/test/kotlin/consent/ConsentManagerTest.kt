package com.rivalapexmediation.sdk.consent

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ConsentManagerTest {
    @Test
    fun normalize_trims_and_nulls_empty() {
        val s = ConsentManager.normalize(
            tcf = "  ",
            usp = "\n\t",
            gdprApplies = null,
            coppa = null,
            limitAdTracking = null,
        )
            assertNull(s.tcfString)
        assertNull(s.usPrivacy)
        assertNull(s.gdprApplies)
    }

    @Test
    fun normalize_keeps_values_and_flags() {
        val s = ConsentManager.normalize(
            tcf = "TCF_V2",
            usp = "1YNN",
            gdprApplies = true,
            coppa = false,
            limitAdTracking = true,
        )
            assertEquals("TCF_V2", s.tcfString)
        assertEquals("1YNN", s.usPrivacy)
        assertEquals(true, s.gdprApplies)
        assertEquals(false, s.coppa)
        assertEquals(true, s.limitAdTracking)
    }

    @Test
    fun redact_masks_long_strings() {
        val red = ConsentManager.redact("ABCDEFGHIJKLmnopqrstuvwxyz")
        // Expect first 8, ellipsis, last 4
        assertEquals("ABCDEFGH…wxyz", red)
    }

    @Test
    fun redact_short_strings_become_stars() {
        val red = ConsentManager.redact("shortstr")
        assertEquals("****", red)
    }

    @Test
    fun shouldRefetchIdentifiers_triggersWhenLatTurnsOffWithoutId() {
        val previous = ConsentManager.State(
            limitAdTracking = true,
            identifiers = ConsentManager.IdentifierState(limitAdTracking = true)
        )
        val should = ConsentManager.shouldRefetchIdentifiers(previous, requestedLimitAdTracking = false)
        assertTrue(should)
    }

    @Test
    fun shouldRefetchIdentifiers_skipsWhenIdAlreadyPresent() {
        val previous = ConsentManager.State(
            limitAdTracking = true,
            identifiers = ConsentManager.IdentifierState(
                advertisingId = "gaid",
                limitAdTracking = true
            )
        )
        val should = ConsentManager.shouldRefetchIdentifiers(previous, requestedLimitAdTracking = false)
        assertFalse(should)
    }

    @Test
    fun shouldRefetchIdentifiers_skipsWhenLatAlreadyOff() {
        val previous = ConsentManager.State(
            limitAdTracking = false,
            identifiers = ConsentManager.IdentifierState(limitAdTracking = false)
        )
        val should = ConsentManager.shouldRefetchIdentifiers(previous, requestedLimitAdTracking = false)
        assertFalse(should)
    }

    @Test
    fun toAuctionConsent_includesSandboxAndIdentifiers() {
        val state = ConsentManager.State(
            privacySandboxOptIn = true,
            limitAdTracking = false,
            identifiers = ConsentManager.IdentifierState(
                advertisingId = "gaid-123",
                appSetId = "appset-456",
                source = ConsentManager.IdentifierSource.GAID,
                limitAdTracking = false
            )
        )
        val consent = ConsentManager.toAuctionConsent(state)
        assertTrue(consent.privacySandbox!!)
        assertEquals("gaid-123", consent.advertisingId)
        assertEquals("appset-456", consent.appSetId)
        assertFalse(consent.limitAdTracking!!)
    }

    @Test
    fun toRuntimeConsent_carriesGlobalPrivacyStrings() {
        val state = ConsentManager.State(
            gdprApplies = true,
            tcfString = "tc",
            usPrivacy = "1YNN",
            coppa = true,
            limitAdTracking = true,
            identifiers = ConsentManager.IdentifierState(
                advertisingId = "gaid-1",
                appSetId = "set-2",
                source = ConsentManager.IdentifierSource.GAID,
                limitAdTracking = true
            )
        )

        val runtime = ConsentManager.toRuntimeConsent(state)

        assertEquals("tc", runtime.iabTcfV2)
        assertEquals("1YNN", runtime.iabUsPrivacy)
        assertTrue(runtime.coppa)
        assertTrue(runtime.limitAdTracking)
        assertEquals("gaid-1", runtime.advertisingId)
        assertEquals("set-2", runtime.appSetId)
    }

    @Test
    fun auctionMeta_maps_all_flags_and_strings() {
        val state = ConsentManager.State(
            gdprApplies = false,
            tcfString = "BOJObISOJObISAABAAENAA4AAAAA",
            usPrivacy = "1YNN",
            coppa = true,
            limitAdTracking = false,
            privacySandboxOptIn = true
        )

        val meta = ConsentManager.toAuctionMeta(state)

        assertEquals(mapOf(
            "gdpr_applies" to "0",
            "gdpr_consent" to "BOJObISOJObISAABAAENAA4AAAAA",
            "us_privacy" to "1YNN",
            "coppa" to "1",
            "limit_ad_tracking" to "0",
            "privacy_sandbox" to "1"
        ), meta)
    }

    @Test
    fun debugSummary_redacts_tcf_string_presence_only() {
        val state = ConsentManager.State(
            gdprApplies = true,
            tcfString = "BOJObISOJObISAABAAENAA4AAAAA",
            usPrivacy = "1YNN",
            coppa = false,
            limitAdTracking = true
        )

        val summary = ConsentManager.debugSummary(state)

        assertEquals(true, summary["has_tcf_string"])
        assertEquals("1YNN", summary["us_privacy"])
        assertFalse("raw tcf string must not appear", summary.values.contains(state.tcfString))
    }
}
