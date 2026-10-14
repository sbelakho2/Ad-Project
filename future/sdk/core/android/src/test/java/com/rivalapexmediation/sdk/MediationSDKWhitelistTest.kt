package com.rivalapexmediation.sdk

import org.junit.Assert.assertEquals
import org.junit.Test

class MediationSDKWhitelistTest {

    @Test
    fun filter_returns_input_when_no_whitelist() {
        val input = listOf("admob", "AppLovin", "ironSource")
        val out = MediationSDK.filterByWhitelist(input, null)
        assertEquals(input, out)
    }

    @Test
    fun filter_applies_case_insensitive_match_and_preserves_order() {
        val input = listOf("admob", "AppLovin", "ironSource", "Vungle")
        val wl = setOf("applovin", "VUNGLE")
        val out = MediationSDK.filterByWhitelist(input, wl)
        assertEquals(listOf("AppLovin", "Vungle"), out)
    }

    @Test
    fun filter_empty_whitelist_behaves_like_null() {
        val input = listOf("admob")
        val out = MediationSDK.filterByWhitelist(input, emptySet())
        assertEquals(input, out)
    }
}
