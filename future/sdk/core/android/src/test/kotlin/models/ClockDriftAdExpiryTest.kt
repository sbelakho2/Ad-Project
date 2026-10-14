package com.rivalapexmediation.sdk.models

import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.ClockProvider
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class ClockDriftAdExpiryTest {
    private var originalClock: Clock? = null

    private class DualClock(var wall: Long, var mono: Long) : Clock {
        override fun now(): Long = wall
        override fun monotonicNow(): Long = mono
        fun advanceWall(delta: Long) { wall += delta }
        fun advanceMono(delta: Long) { mono += delta }
    }

    @Before
    fun setUp() {
        originalClock = ClockProvider.clock
    }

    @After
    fun tearDown() {
        originalClock?.let { ClockProvider.clock = it }
    }

    @Test
    fun adExpiryFollowsMonotonicClockDespiteWallClockDrift() {
        val clock = DualClock(wall = 1_000L, mono = 1_000L)
        ClockProvider.clock = clock

        val expiry = clock.monotonicNow() + 1_000L
        val ad = Ad(
            id = "drift-ad",
            placementId = "p-drift",
            networkName = "net",
            adType = AdType.INTERSTITIAL,
            ecpm = 0.5,
            creative = Creative.Banner(0, 0, "<div></div>"),
            expiryTimeMs = expiry
        )

        // Large wall-clock jump forward should not expire the ad because expiry uses monotonic clock
        clock.advanceWall(60_000L)
        assertFalse(ad.isExpired())

        // Monotonic time advancing past expiry should expire the ad deterministically
        clock.advanceMono(1_200L)
        assertTrue(ad.isExpired())
    }
}
