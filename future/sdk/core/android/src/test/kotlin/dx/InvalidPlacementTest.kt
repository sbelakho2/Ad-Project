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
 * Verifies INVALID_PLACEMENT mapping and main-thread delivery when the placement is unknown.
 */
@RunWith(RobolectricTestRunner::class)
class InvalidPlacementTest {
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

    private fun configBody(withPlacementId: String = "pl_known"): String {
        val body = mapOf(
            "configId" to "cfg-invalid-pl",
            "version" to 1,
            "placements" to mapOf(
                withPlacementId to mapOf(
                    "placementId" to withPlacementId,
                    "adType" to "INTERSTITIAL",
                    "enabledNetworks" to emptyList<String>(),
                    "timeoutMs" to 200,
                    "maxWaitMs" to 400,
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
    fun load_withUnknownPlacement_returnsInvalidPlacement_onMainThread() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Serve a config that contains only pl_known; we'll request pl_unknown
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody("pl_known")))

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
        var onMain = false
        var errCode: AdError? = null
        var errorMessage: String? = null

        BelInterstitial.load(appContext, "pl_unknown", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                fail("Expected INVALID_PLACEMENT error")
            }
            override fun onError(error: AdError, message: String) {
                onMain = (Looper.myLooper() == Looper.getMainLooper())
                errCode = error
                errorMessage = message
                latch.countDown()
            }
        })

        // Let main-thread callbacks run
        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue(latch.await(1, TimeUnit.SECONDS))
        assertTrue("Callback should be on main thread", onMain)
        assertEquals(AdError.INVALID_PLACEMENT, errCode)
        assertTrue((errorMessage ?: "").contains("Unknown placement"))
        assertFalse(BelInterstitial.isReady())
    }
}
