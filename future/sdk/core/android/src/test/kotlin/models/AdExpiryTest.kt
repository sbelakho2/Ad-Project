package com.rivalapexmediation.sdk.models

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AdExpiryTest {
    @Test
    fun `ad without expiry is not expired`() {
        val ad = Ad(
            id = "1",
            placementId = "p1",
            networkName = "testnet",
            adType = AdType.INTERSTITIAL,
            ecpm = 1.23,
            creative = Creative.Banner(0, 0, "<div></div>"),
            expiryTimeMs = null
        )
        assertFalse(ad.isExpired())
    }

    @Test
    fun `ad with future expiry is not expired`() {
        val ad = Ad(
            id = "2",
            placementId = "p1",
            networkName = "testnet",
            adType = AdType.INTERSTITIAL,
            ecpm = 1.23,
            creative = Creative.Banner(0, 0, "<div></div>"),
            expiryTimeMs = com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow() + 5_000
        )
        assertFalse(ad.isExpired())
    }

    @Test
    fun `ad with past expiry is expired`() {
        val ad = Ad(
            id = "3",
            placementId = "p1",
            networkName = "testnet",
            adType = AdType.INTERSTITIAL,
            ecpm = 1.23,
            creative = Creative.Banner(0, 0, "<div></div>"),
            expiryTimeMs = com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow() - 1
        )
        assertTrue(ad.isExpired())
    }
}
