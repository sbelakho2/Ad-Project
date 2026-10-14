package com.rivalapexmediation.ctv.ads

import com.rivalapexmediation.ctv.network.AuctionWin
import com.rivalapexmediation.ctv.network.TrackingUrls
import org.junit.After
import org.junit.Before
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

class AdCacheTest {
    private val clock = MutableClock()

    @Before
    fun setUp() {
        AdCache.clock = clock
        AdCache.clear()
    }

    @After
    fun tearDown() {
        AdCache.clear()
        AdCache.clock = SystemMonotonicClock
    }

    @Test
    fun peekSurvivesBackwardDrift() {
        AdCache.put("placement", win(ttlSeconds = 300))

        clock.advanceMinutes(-10)

        val cached = AdCache.peek("placement")
        assertNotNull(cached)
        assertEquals("adapter", cached.adapter)
    }

    @Test
    fun takeExpiresAfterForwardDriftPastTtl() {
        AdCache.put("placement", win(ttlSeconds = 300))

        clock.advanceMinutes(11)

        val cached = AdCache.take("placement")
        assertNull(cached)
    }

    @Test
    fun takeRemovesEntryAfterUse() {
        AdCache.put("placement", win(ttlSeconds = 120))

        val first = AdCache.take("placement")
        val second = AdCache.take("placement")

        assertNotNull(first)
        assertNull(second)
    }

    private fun win(ttlSeconds: Int): AuctionWin = AuctionWin(
        requestId = "req",
        bidId = "bid",
        adapter = "adapter",
        cpm = 1.23,
        currency = "USD",
        ttlSeconds = ttlSeconds,
        creativeUrl = "https://example.com/ad.mp4",
        tracking = TrackingUrls(
            impression = "https://example.com/imp",
            click = "https://example.com/click"
        ),
        payload = null,
    )
}

private class MutableClock(startMs: Long = 0L) : Clock {
    private var nowMs: Long = startMs

    override fun now(): Long = nowMs

    fun advanceMinutes(minutes: Long) {
        nowMs += minutes * 60_000L
    }
}
