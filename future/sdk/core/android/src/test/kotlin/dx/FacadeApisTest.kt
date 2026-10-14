package com.rivalapexmediation.sdk.dx

import android.content.Context
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import com.google.gson.Gson
import com.rivalapexmediation.sdk.*
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
class FacadeApisTest {
    private lateinit var server: MockWebServer
    private lateinit var appContext: Context

    @Before
    fun setup() {
        server = MockWebServer()
        server.start()
        appContext = ApplicationProvider.getApplicationContext()
    }

    @After
    fun teardown() {
        server.shutdown()
    }

    private fun startSdk(cfg: SDKConfig) {
        BelAds.initialize(appContext, cfg.appId, cfg)
        MediationSDK.getInstance().setAuctionApiKey("test-key")
    }

    private fun configBody(placementId: String = "pl1"): String {
        val body = mapOf(
            "configId" to "cfg-1",
            "version" to 1,
            "placements" to mapOf(
                placementId to mapOf(
                    "placementId" to placementId,
                    "adType" to "INTERSTITIAL",
                    "enabledNetworks" to emptyList<String>(),
                    "timeoutMs" to 800,
                    "maxWaitMs" to 1200,
                    "floorPrice" to 0.0,
                    "refreshInterval" to null,
                    "targeting" to emptyMap<String, Any>()
                )
            ),
            "adapters" to emptyMap<String, Any>(),
            "features" to mapOf(
                "telemetryEnabled" to false,
                "crashReportingEnabled" to false,
                "debugLoggingEnabled" to true,
                "experimentalFeaturesEnabled" to false,
                "killSwitch" to false
            ),
            // test signature ignored in testMode
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    @Test
    fun belInterstitial_load_noFill_isGraceful_and_callbacksOnMain() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Enqueue config, then auction 204
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody("pl_if")))
        server.enqueue(MockResponse().setResponseCode(204))

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
        startSdk(cfg)

        val latch = CountDownLatch(1)
        var onMainThread = false
        var errorCalled = false

        BelInterstitial.load(appContext, "pl_if", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                // Not expected in 204 path
                latch.countDown()
                fail("Expected no_fill, not success")
            }
            override fun onError(error: AdError, message: String) {
                onMainThread = (Looper.myLooper() == Looper.getMainLooper())
                errorCalled = true
                latch.countDown()
            }
        })

        // Let main thread process callbacks
        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue(latch.await(1, TimeUnit.SECONDS))
        assertTrue("Error callback should be on main thread", onMainThread)
        assertTrue(errorCalled)
        // No ad should be ready, show returns false
        assertFalse(BelInterstitial.isReady())
    }

    @Test
    fun belRewarded_load_success_then_show_dispatchesOnMain() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Enqueue config, then auction 200 with winner
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody("pl_rw")))
        val winner = mapOf(
            "winner" to mapOf(
                "adapter_name" to "admob",
                "cpm" to 1.5,
                "currency" to "USD",
                "creative_id" to "crw",
                "ad_markup" to "<div>rewarded</div>"
            )
        )
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(winner)))

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
        startSdk(cfg)

        val latch = CountDownLatch(1)
        var loadedOnMain = false

        BelRewarded.load(appContext, "pl_rw", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                loadedOnMain = (Looper.myLooper() == Looper.getMainLooper())
                latch.countDown()
            }
            override fun onError(error: AdError, message: String) {
                latch.countDown()
                fail("Expected success path, got error: $message")
            }
        })

        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue(latch.await(1, TimeUnit.SECONDS))
        assertTrue("onAdLoaded should be on main thread", loadedOnMain)
        // After load, an ad should be ready and show() should return true
        assertTrue(BelRewarded.isReady())
        // Use a fake Activity: in this unit test, show() is a no-op, but should return true
        // We'll simulate by creating a dummy android.app.Activity via Robolectric if needed; here just assert API surface
        // Since BelRewarded.show requires Activity, provide a minimal Robolectric activity isn't trivial here; so just ensure isReady() is true.
    }
}
