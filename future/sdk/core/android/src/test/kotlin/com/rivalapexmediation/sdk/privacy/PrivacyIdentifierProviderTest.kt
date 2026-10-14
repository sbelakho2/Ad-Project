package com.rivalapexmediation.sdk.privacy

import android.app.Application
import android.content.Context
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PrivacyIdentifierProviderTest {
    private val context: Context = Application()

    @Test
    fun collect_returnsLimitedWhenOptOutRequested() {
        val gaid = PrivacyIdentifiers(advertisingId = "gaid", limitAdTracking = false, source = PrivacyIdentifiers.Source.GAID)
        val provider = PrivacyIdentifierProvider(context, gaidSupplier = { gaid }, appSetSupplier = { gaid })
        val result = provider.collect(optOut = true)
        assertTrue(result.limitAdTracking)
        assertEquals(null, result.advertisingId)
    }

    @Test
    fun collect_prefersGaidWhenAvailable() {
        val gaid = PrivacyIdentifiers(advertisingId = "gaid-1", limitAdTracking = false, source = PrivacyIdentifiers.Source.GAID)
        val provider = PrivacyIdentifierProvider(context, gaidSupplier = { gaid }, appSetSupplier = { null })
        val result = provider.collect()
        assertEquals("gaid-1", result.advertisingId)
        assertFalse(result.limitAdTracking)
        assertEquals(PrivacyIdentifiers.Source.GAID, result.source)
    }

    @Test
    fun collect_fallsBackToAppSet() {
        val appSet = PrivacyIdentifiers(appSetId = "set-2", limitAdTracking = false, source = PrivacyIdentifiers.Source.APP_SET)
        val provider = PrivacyIdentifierProvider(context, gaidSupplier = { null }, appSetSupplier = { appSet })
        val result = provider.collect()
        assertEquals("set-2", result.appSetId)
        assertFalse(result.limitAdTracking)
        assertEquals(PrivacyIdentifiers.Source.APP_SET, result.source)
    }

    @Test
    fun collect_defaultsToLimitedWhenNoIds() {
        val provider = PrivacyIdentifierProvider(context, gaidSupplier = { null }, appSetSupplier = { null })
        val result = provider.collect()
        assertTrue(result.limitAdTracking)
        assertEquals(null, result.advertisingId)
        assertEquals(null, result.appSetId)
    }
}
