package com.rivalapexmediation.sdk.dx

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.google.gson.Gson
import com.rivalapexmediation.sdk.*
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.ClockProvider

/**
 * Validates caching/state guarantees for interstitial placements: expiry, single-use,
 * sequential loads, and cold vs warm cache transitions.
 */
@RunWith(RobolectricTestRunner::class)
class AdCacheBehaviorTest {
    private lateinit var server: MockWebServer
    private lateinit var appContext: Context

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
        appContext = ApplicationProvider.getApplicationContext()
        resetBelInterstitialState()
    }

    @After
    fun tearDown() {
        try { server.shutdown() } catch (_: Throwable) {}
    }

    @Test
    fun coldWarmAndShowOnce_flowBehavesAsExpected() {
        val placement = "pl_cache_state"
        startSdk(placement)
        server.enqueue(MockResponse().setResponseCode(200).setBody(winnerBody("cr-cache")))

        assertFalse("No placement selected yet", BelInterstitial.isReady())

        loadPlacement(placement)
        assertTrue("Cache should be warm after load", BelInterstitial.isReady())

        val consumed = MediationSDK.getInstance().consumeCachedAd(placement)
        assertNotNull("consumeCachedAd should return the cached ad", consumed)
        assertFalse("Ad should be single-use and removed after consume", BelInterstitial.isReady())
    }

    @Test
    fun cachedAdExpiresAfterRefreshWindow() {
        val placement = "pl_expire"
        startSdk(placement, refreshInterval = 1)
        server.enqueue(MockResponse().setResponseCode(200).setBody(winnerBody("cr-exp")))

        loadPlacement(placement)
        val cached = MediationSDK.getInstance().getCachedAd(placement)
        assertNotNull(cached)
        val ttlMs = cached!!.expiryTimeMs!! - com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow()
        assertTrue("TTL should honor refresh window multiplier", ttlMs in 1_000..10_000)
        forceExpire(placement)
        assertFalse("Expired ads must be pruned from cache", BelInterstitial.isReady())
    }

    @Test
    fun subsequentLoadsReplaceCachedAd() {
        val placement = "pl_sequence"
        startSdk(placement)
        server.enqueue(MockResponse().setResponseCode(200).setBody(winnerBody("cr-first")))
        loadPlacement(placement)
        assertEquals("cr-first", MediationSDK.getInstance().getCachedAd(placement)?.id)

        server.enqueue(MockResponse().setResponseCode(200).setBody(winnerBody("cr-second")))
        loadPlacement(placement)
        val cached = MediationSDK.getInstance().getCachedAd(placement)
        assertNotNull(cached)
        assertEquals("cr-second", cached!!.id)
        assertTrue("Cache should report ready after replacement", BelInterstitial.isReady())
    }

    @Test
    fun cachedAdReadinessIgnoresWallClockDriftAndExpiresOnMonotonic() {
        val originalClock = ClockProvider.clock
        val driftClock = object : Clock {
            var wall: Long = 1_000L
            var mono: Long = 1_000L
            override fun now(): Long = wall
            override fun monotonicNow(): Long = mono
            fun advanceWall(delta: Long) { wall += delta }
            fun advanceMono(delta: Long) { mono += delta }
        }
        ClockProvider.clock = driftClock
        try {
            val placement = "pl_drift"
            startSdk(placement, refreshInterval = 1)
            server.enqueue(MockResponse().setResponseCode(200).setBody(winnerBody("cr-drift")))

            loadPlacement(placement)
            assertTrue("Cache should be ready after load", BelInterstitial.isReady())

            // Large wall-clock jump must not expire cached ad because expiry is monotonic
            driftClock.advanceWall(5 * 60_000L)
            assertTrue("Cache should remain ready after wall-clock jump", BelInterstitial.isReady())

            // Advancing monotonic time past TTL should expire the ad deterministically
            driftClock.advanceMono(10_000L)
            assertFalse("Cache should expire when monotonic time passes TTL", BelInterstitial.isReady())

            // Backward wall-clock jump should not un-expire a cached ad once monotonic time is past TTL
            driftClock.advanceWall(-120_000L)
            assertFalse("Cache should remain expired after backward wall-clock jump", BelInterstitial.isReady())
        } finally {
            ClockProvider.clock = originalClock
        }
    }

    private fun startSdk(placementId: String, refreshInterval: Int? = null) {
        val baseUrl = server.url("/").toString().trimEnd('/')
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody(placementId, refreshInterval)))
        val cfg = SDKConfig(
            appId = "app-1",
            testMode = true,
            logLevel = LogLevel.DEBUG,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = baseUrl,
            sdkMode = SdkMode.HYBRID,
            enableS2SWhenCapable = true
        )
        BelAds.initialize(appContext, cfg.appId, cfg)
        MediationSDK.getInstance().setAuctionApiKey("test-key")
    }

    private fun loadPlacement(placementId: String) {
        val latch = CountDownLatch(1)
        var failure: AssertionError? = null
        BelInterstitial.load(appContext, placementId, object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                latch.countDown()
            }

            override fun onError(error: AdError, message: String) {
                failure = AssertionError("Expected success, got ${'$'}error:$message")
                latch.countDown()
            }
        })
        Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
        assertTrue("Load should complete", latch.await(1, TimeUnit.SECONDS))
        failure?.let { throw it }
    }

    private fun configBody(placementId: String, refreshInterval: Int?): String {
        val placement = mutableMapOf<String, Any?>(
            "placementId" to placementId,
            "adType" to "INTERSTITIAL",
            "enabledNetworks" to emptyList<String>(),
            "timeoutMs" to 500,
            "maxWaitMs" to 800,
            "floorPrice" to 0.0,
            "targeting" to emptyMap<String, Any>()
        )
        placement["refreshInterval"] = refreshInterval
        val body = mapOf(
            "configId" to "cfg-1",
            "version" to 1,
            "placements" to mapOf(placementId to placement),
            "adapters" to emptyMap<String, Any>(),
            "features" to mapOf(
                "telemetryEnabled" to false,
                "crashReportingEnabled" to false,
                "debugLoggingEnabled" to true,
                "experimentalFeaturesEnabled" to false,
                "killSwitch" to false
            ),
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    private fun winnerBody(creativeId: String): String {
        val body = mapOf(
            "winner" to mapOf(
                "adapter_name" to "admob",
                "cpm" to 1.2,
                "currency" to "USD",
                "creative_id" to creativeId,
                "ad_markup" to "<div>${creativeId}</div>"
            )
        )
        return Gson().toJson(body)
    }

    private fun resetBelInterstitialState() {
        runCatching {
            val field = BelInterstitial::class.java.getDeclaredField("lastPlacement")
            field.isAccessible = true
            field.set(null, null)
        }
    }

    private fun forceExpire(placement: String) {
        runCatching {
            val sdk = MediationSDK.getInstance()
            val cacheField = MediationSDK::class.java.getDeclaredField("adCache").apply { isAccessible = true }
            @Suppress("UNCHECKED_CAST")
            val cache = cacheField.get(sdk) as MutableMap<String, Any?>
            val cached = cache[placement] ?: return
            val expiryField = cached::class.java.getDeclaredField("expiryAtMs").apply { isAccessible = true }
            expiryField.setLong(cached, com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow() - 1)
        }
    }
}
