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
 * Facade-level taxonomy tests to ensure normalized error mapping and main-thread callbacks.
 */
@RunWith(RobolectricTestRunner::class)
class FacadeApisTaxonomyTest {
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

    private fun configBody(placementId: String = "pl_tax"): String {
        val body = mapOf(
            "configId" to "cfg-tax",
            "version" to 1,
            "placements" to mapOf(
                placementId to mapOf(
                    "placementId" to placementId,
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
            // test signature ignored in testMode
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    @Test
    fun http400_mapsToInternalError_status400_and_mainThreadCallback() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Enqueue config, then auction 400
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody("pl_400")))
        server.enqueue(MockResponse().setResponseCode(400).setBody("bad"))

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
        var onMain = false
        var errorCode: AdError? = null
        var errorMessage: String? = null

        BelInterstitial.load(appContext, "pl_400", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                fail("Expected error for HTTP 400")
            }
            override fun onError(error: AdError, message: String) {
                onMain = (Looper.myLooper() == Looper.getMainLooper())
                errorCode = error
                errorMessage = message
                latch.countDown()
            }
        })

        // Let main thread execute callbacks
        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue(latch.await(1, TimeUnit.SECONDS))
        assertTrue("Callback should be on main thread", onMain)
        assertEquals(AdError.INTERNAL_ERROR, errorCode)
        assertEquals("status_400", errorMessage)
    }

    @Test
    fun timeout_mapsToTimeout_and_mainThreadCallback() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Enqueue config, then auction 200 with long delay to trigger timeout
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody("pl_to")))
        server.enqueue(
            MockResponse()
                .setResponseCode(200)
                .setBody("{}")
                .setBodyDelay(2, TimeUnit.SECONDS) // exceed placement timeout
        )

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
        var onMain = false
        var errorCode: AdError? = null
        var errorMessage: String? = null

        BelInterstitial.load(appContext, "pl_to", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                fail("Expected timeout path")
            }
            override fun onError(error: AdError, message: String) {
                onMain = (Looper.myLooper() == Looper.getMainLooper())
                errorCode = error
                errorMessage = message
                latch.countDown()
            }
        })

        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue(latch.await(2, TimeUnit.SECONDS))
        assertTrue("Callback should be on main thread", onMain)
        assertEquals(AdError.TIMEOUT, errorCode)
        assertTrue("Message should mention timeout", (errorMessage ?: "").contains("timeout", ignoreCase = true))
    }
}
