package com.rivalapexmediation.sdk

import com.rivalapexmediation.sdk.consent.ConsentManager
import com.rivalapexmediation.sdk.consent.ConsentManager.IdentifierState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConsentMappingTest {

    @Test
    fun normalize_trims_strings_and_accepts_nulls() {
        val s = ConsentManager.normalize(tcf = "  BOJObISOJObISAABAAENAA4AAAAA  ", usp = " 1YNN ")
        assertEquals("BOJObISOJObISAABAAENAA4AAAAA", s.tcfString)
        assertEquals("1YNN", s.usPrivacy)
    }

    @Test
    fun toRuntimeConsent_maps_flags_and_identifiers() {
        val state = ConsentManager.State(
            gdprApplies = true,
            tcfString = "BOJObISOJObISAABAAENAA4AAAAA",
            usPrivacy = "1YNN",
            coppa = false,
            limitAdTracking = false,
            privacySandboxOptIn = true,
            identifiers = IdentifierState(advertisingId = "38400000-8cf0-11bd-b23e-10b96e40000d", appSetId = "appset-xyz", limitAdTracking = false)
        )
        val rt = ConsentManager.toRuntimeConsent(state)
        assertEquals("BOJObISOJObISAABAAENAA4AAAAA", rt.iabTcfV2)
        assertEquals("1YNN", rt.iabUsPrivacy)
        assertFalse(rt.coppa)
        assertFalse(rt.limitAdTracking)
        assertTrue(rt.privacySandboxOptIn ?: false)
        assertEquals("38400000-8cf0-11bd-b23e-10b96e40000d", rt.advertisingId)
        assertEquals("appset-xyz", rt.appSetId)
    }

    @Test
    fun shouldRefetchIdentifiers_triggers_when_requesting_to_disable_lat_and_missing_adid() {
        val prev = ConsentManager.State(
            limitAdTracking = true,
            identifiers = IdentifierState(advertisingId = null, appSetId = "appset", limitAdTracking = true)
        )
        assertTrue(ConsentManager.shouldRefetchIdentifiers(prev, requestedLimitAdTracking = false))
        // If we already have an ad id, no refetch
        val withId = prev.copy(identifiers = prev.identifiers.copy(advertisingId = "38400000-8cf0-11bd-b23e-10b96e40000d"))
        assertFalse(ConsentManager.shouldRefetchIdentifiers(withId, requestedLimitAdTracking = false))
    }
}
