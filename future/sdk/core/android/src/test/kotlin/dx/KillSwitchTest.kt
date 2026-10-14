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
 * Verifies that the SDK respects the remote kill switch and fails fast without doing work.
 */
@RunWith(RobolectricTestRunner::class)
class KillSwitchTest {
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

    private fun configBody(placementId: String = "pl_kill"): String {
        val body = mapOf(
            "configId" to "cfg-kill",
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
                "killSwitch" to true
            ),
            // test signature ignored in testMode
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    @Test
    fun killSwitch_blocksLoads_andReportsOnMainThread() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Enqueue config with killSwitch=true
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody()))

        val cfg = SDKConfig(
            appId = "app-1",
            testMode = true, // bypass signature
            logLevel = LogLevel.DEBUG,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = baseUrl
        )
        BelAds.initialize(appContext, "app-1", cfg)

        val latch = CountDownLatch(1)
        var onMain = false
        var gotError = false
        var errorCode: AdError? = null
        var errorMessage: String? = null

        BelInterstitial.load(appContext, "pl_kill", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                fail("Should not load when killSwitch is active")
            }
            override fun onError(error: AdError, message: String) {
                gotError = true
                errorCode = error
                errorMessage = message
                onMain = (Looper.myLooper() == Looper.getMainLooper())
                latch.countDown()
            }
        })

        // Allow main-thread callbacks to run
        Shadows.shadowOf(Looper.getMainLooper()).idle()

        assertTrue("Expected error callback", latch.await(1, TimeUnit.SECONDS))
        assertTrue("Callback should be on main thread", onMain)
        assertTrue(gotError)
        // Our MediationSDK maps kill switch to INTERNAL_ERROR with reason string
        assertEquals(AdError.INTERNAL_ERROR, errorCode)
        assertEquals("kill_switch_active", errorMessage)
        // No ad should be ready
        assertFalse(BelInterstitial.isReady())
    }
}
