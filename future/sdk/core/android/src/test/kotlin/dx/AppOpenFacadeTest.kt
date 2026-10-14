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
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Facade-level tests for BelAppOpen to ensure DX expectations:
 * - Main-thread error callback delivery
 * - Graceful no-fill behavior (isReady=false, show() returns false)
 */
@RunWith(RobolectricTestRunner::class)
class AppOpenFacadeTest {
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

    private fun configBody(placementId: String = "pl_appopen"): String {
        val body = mapOf(
            "configId" to "cfg-ao",
            "version" to 1,
            "placements" to mapOf(
                placementId to mapOf(
                    "placementId" to placementId,
                    "adType" to "APP_OPEN",
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
            // signature ignored in testMode
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    @Test
    fun belAppOpen_load_noFill_isGraceful_and_callbacksOnMain() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Enqueue config, then auction 204 to force no_fill
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody("pl_appopen")))
        server.enqueue(MockResponse().setResponseCode(204))

        val cfg = SDKConfig(
            appId = "app-1",
            testMode = true,
            logLevel = LogLevel.DEBUG,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = baseUrl
        )
        BelAds.initialize(appContext, "app-1", cfg)

        val latch = CountDownLatch(1)
        var onMainThread = false
        var errorCalled = false

        BelAppOpen.load(appContext, "pl_appopen", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                latch.countDown()
                fail("Expected no_fill, not success")
            }
            override fun onError(error: AdError, message: String) {
                onMainThread = (Looper.myLooper() == Looper.getMainLooper())
                errorCalled = true
                latch.countDown()
            }
        })

        // Allow main-thread callbacks to run
        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue(latch.await(1, TimeUnit.SECONDS))
        assertTrue("Error callback should be on main thread", onMainThread)
        assertTrue(errorCalled)
        // No ad should be ready, show returns false
        assertFalse(BelAppOpen.isReady())
        // We cannot provide an Activity easily here; just ensure show would return false without a cached ad
    }
}
