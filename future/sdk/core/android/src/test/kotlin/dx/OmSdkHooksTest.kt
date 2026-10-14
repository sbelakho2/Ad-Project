package com.rivalapexmediation.sdk.dx

import android.app.Activity
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.google.gson.Gson
import com.rivalapexmediation.sdk.*
import com.rivalapexmediation.sdk.measurement.OmSdkController
import com.rivalapexmediation.sdk.measurement.OmSdkRegistry
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner

/**
 * Verifies that facade show() paths invoke OM SDK hooks.
 */
@RunWith(RobolectricTestRunner::class)
class OmSdkHooksTest {
    private lateinit var server: MockWebServer
    private lateinit var appContext: Context

    private data class Call(val type: String, val placement: String, val creativeType: String? = null)

    private fun installOmController(calls: MutableList<Call>) {
        OmSdkRegistry.controller = object : OmSdkController {
            override fun startDisplaySession(activity: Activity, placementId: String, networkName: String, creativeType: String?) {
                calls.add(Call("startDisplay", placementId, creativeType))
            }

            override fun startVideoSession(activity: Activity, placementId: String, networkName: String, durationSec: Int?) {
                calls.add(Call("startVideo", placementId))
            }

            override fun endSession(placementId: String) {
                calls.add(Call("end", placementId))
            }
        }
    }

    @Before
    fun setup() {
        server = MockWebServer()
        server.start()
        appContext = ApplicationProvider.getApplicationContext()
    }

    @After
    fun teardown() {
        try { server.shutdown() } catch (_: Throwable) {}
    }

    private fun startSdk(cfg: SDKConfig) {
        BelAds.initialize(appContext, cfg.appId, cfg)
        MediationSDK.getInstance().setAuctionApiKey("test-key")
    }

    private fun configBody(placementId: String, adType: String = "INTERSTITIAL"): String {
        val body = mapOf(
            "configId" to "cfg-omsdk",
            "version" to 1,
            "placements" to mapOf(
                placementId to mapOf(
                    "placementId" to placementId,
                    "adType" to adType,
                    "enabledNetworks" to emptyList<String>(),
                    "timeoutMs" to 500,
                    "maxWaitMs" to 1000,
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
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    @Test
    fun interstitial_show_invokesOmDisplaySession() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        val placementId = "pl_omsdk_int"
        // Serve config then auction winner
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody(placementId)))
        val winner = mapOf(
            "winner" to mapOf(
                "adapter_name" to "admob",
                "cpm" to 1.2,
                "currency" to "USD",
                "creative_id" to "cr1",
                "ad_markup" to "<div>ad</div>"
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

        // Install a test OM controller to capture calls
        val calls = mutableListOf<Call>()
        installOmController(calls)

        // Load then show
        BelInterstitial.load(appContext, placementId, object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) { }
            override fun onError(error: AdError, message: String) { }
        })
        // Build a Robolectric Activity to pass to show()
        val activity = Robolectric.buildActivity(android.app.Activity::class.java).setup().get()
        // Try to show
        val shown = BelInterstitial.show(activity)
        assertTrue("Expected show() to return true after load", shown)
        // Verify OM hooks called (startDisplay then end)
        // Allow for either 2 calls (start + end) or more depending on implementation; assert order of first two
        assertTrue("OM hooks should have been invoked", calls.isNotEmpty())
        assertEquals("startDisplay", calls.first().type)
        // There should be an end call for the same placement eventually
        val hasEnd = calls.any { it.type == "end" && it.placement == placementId }
        assertTrue("Expected endSession call", hasEnd)
    }

    @Test
    fun rewarded_show_invokesOmVideoSession() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        val placementId = "pl_omsdk_rw"
        // Serve config then auction winner
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody(placementId)))
        val winner = mapOf(
            "winner" to mapOf(
                "adapter_name" to "admob",
                "cpm" to 1.0,
                "currency" to "USD",
                "creative_id" to "cr2",
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

        val calls = mutableListOf<Call>()
        installOmController(calls)

        BelRewarded.load(appContext, placementId, object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) { }
            override fun onError(error: AdError, message: String) { }
        })
        val activity = Robolectric.buildActivity(android.app.Activity::class.java).setup().get()
        val shown = BelRewarded.show(activity)
        assertTrue(shown)
        assertTrue(calls.isNotEmpty())
        // Rewarded should startVideo session at some point
        val hasVideo = calls.any { it.type == "startVideo" && it.placement == placementId }
        assertTrue("Expected startVideoSession call", hasVideo)
        val hasEnd = calls.any { it.type == "end" && it.placement == placementId }
        assertTrue("Expected endSession call", hasEnd)
    }

    @Test
    fun rewardedInterstitial_show_invokesOmVideoSession() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        val placementId = "pl_omsdk_rwi"
        // Serve config then auction winner
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody(placementId)))
        val winner = mapOf(
            "winner" to mapOf(
                "adapter_name" to "admob",
                "cpm" to 0.9,
                "currency" to "USD",
                "creative_id" to "cr_rwi",
                "ad_markup" to "<div>rewarded_interstitial</div>"
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

        val calls = mutableListOf<Call>()
        installOmController(calls)

        BelRewardedInterstitial.load(appContext, placementId, object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) { }
            override fun onError(error: AdError, message: String) { }
        })
        val activity = Robolectric.buildActivity(android.app.Activity::class.java).setup().get()
        val shown = BelRewardedInterstitial.show(activity)
        assertTrue(shown)
        assertTrue(calls.isNotEmpty())
        val hasVideo = calls.any { it.type == "startVideo" && it.placement == placementId }
        assertTrue("Expected startVideoSession call", hasVideo)
        val hasEnd = calls.any { it.type == "end" && it.placement == placementId }
        assertTrue("Expected endSession call", hasEnd)
    }

    @Test
    fun appOpen_show_invokesOmDisplaySession() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        val placementId = "pl_omsdk_appopen"
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody(placementId, adType = "APP_OPEN")))
        val winner = mapOf(
            "winner" to mapOf(
                "adapter_name" to "admob",
                "cpm" to 1.1,
                "currency" to "USD",
                "creative_id" to "cr_appopen",
                "ad_markup" to "<div>appopen</div>"
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

        val calls = mutableListOf<Call>()
        installOmController(calls)

        BelAppOpen.load(appContext, placementId, object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) { }
            override fun onError(error: AdError, message: String) { }
        })
        val activity = Robolectric.buildActivity(android.app.Activity::class.java).setup().get()
        val shown = BelAppOpen.show(activity)
        assertTrue(shown)
        assertTrue("Expected OM hooks", calls.isNotEmpty())
        val start = calls.firstOrNull { it.type == "startDisplay" && it.placement == placementId }
        assertTrue("Expected startDisplaySession", start != null)
        assertEquals("app_open", start?.creativeType)
        val hasEnd = calls.any { it.type == "end" && it.placement == placementId }
        assertTrue("Expected endSession call", hasEnd)
    }
}
