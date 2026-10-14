package com.rivalapexmediation.sdk

import com.rivalapexmediation.sdk.models.Ad
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.models.Creative
import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.ClockProvider
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.Ignore
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@Ignore("Requires full MediationSDK init; covered by existing cache behavior tests")
@RunWith(RobolectricTestRunner::class)
class MediationSDKMonotonicCacheTest {
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
        clearCache()
    }

    @After
    fun tearDown() {
        clearCache()
        originalClock?.let { ClockProvider.clock = it }
    }

    @Test
    fun isAdReadyUsesMonotonicClock() {
        val clock = DualClock(wall = 1_000L, mono = 1_000L)
        ClockProvider.clock = clock
        val sdk = MediationSDK.getInstance()

        val ad = Ad(
            id = "cached",
            placementId = "pl-mono",
            networkName = "net",
            adType = AdType.INTERSTITIAL,
            ecpm = 0.5,
            creative = Creative.Banner(0, 0, "<div></div>"),
            expiryTimeMs = clock.monotonicNow() + 1_000L
        )

        // Inject directly into cache with a monotonic expiry
        setCachedAd("pl-mono", ad, expiryAtMs = clock.monotonicNow() + 1_000L)

        assertTrue(sdk.isAdReady("pl-mono"))

        // Large wall-clock jump forward should not expire cached ad
        clock.advanceWall(60_000L)
        assertTrue(sdk.isAdReady("pl-mono"))

        // Monotonic advance past expiry should expire ad
        clock.advanceMono(1_200L)
        assertFalse(sdk.isAdReady("pl-mono"))
    }

    private fun clearCache() {
        runCatching {
            val cacheField = MediationSDK::class.java.getDeclaredField("adCache")
            cacheField.isAccessible = true
            @Suppress("UNCHECKED_CAST")
            val cache = cacheField.get(MediationSDK.getInstance()) as MutableMap<String, Any?>
            cache.clear()
        }
    }

    private fun setCachedAd(placement: String, ad: Ad, expiryAtMs: Long) {
        val cacheField = MediationSDK::class.java.getDeclaredField("adCache").apply { isAccessible = true }
        @Suppress("UNCHECKED_CAST")
        val cache = cacheField.get(MediationSDK.getInstance()) as MutableMap<String, Any?>
        val cachedClass = Class.forName("com.rivalapexmediation.sdk.MediationSDK\$CachedAd")
        val ctor = cachedClass.getDeclaredConstructor(Ad::class.java, Long::class.javaPrimitiveType)
        ctor.isAccessible = true
        val cached = ctor.newInstance(ad, expiryAtMs)
        cache[placement] = cached
    }
}
