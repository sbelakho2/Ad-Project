package com.rivalapexmediation.sample.strictmode

import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import com.google.gson.Gson
import com.rivalapexmediation.sdk.AdError
import com.rivalapexmediation.sdk.AdLoadCallback
import com.rivalapexmediation.sdk.BelAds
import com.rivalapexmediation.sdk.BelInterstitial
import com.rivalapexmediation.sdk.LogLevel
import com.rivalapexmediation.sdk.SDKConfig
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows
import org.robolectric.annotation.Config
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * CI smoke harness executed with StrictMode penalties enabled via [StrictModeSampleApp].
 * Confirms SDK public flows keep blocking I/O off the main thread.
 */
@RunWith(RobolectricTestRunner::class)
@Config(application = StrictModeSampleApp::class, manifest = Config.NONE)
class StrictModeSampleSmokeTest {
    private lateinit var server: MockWebServer

    @Before
    fun setup() {
        server = MockWebServer()
        server.start()
    }

    @After
    fun teardown() {
        try { server.shutdown() } catch (_: Throwable) {}
    }

    private fun configBody(placementId: String = "pl_strict_sample"): String {
        val body = mapOf(
            "configId" to "cfg-strict-sample",
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
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    @Test
    fun sdkFlowsRespectStrictMode() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody()))
        server.enqueue(MockResponse().setResponseCode(204))

        // Boot the sample application (enables StrictMode penalties in onCreate()).
    val app = ApplicationProvider.getApplicationContext<StrictModeSampleApp>()

        val cfg = SDKConfig(
            appId = "sample-app",
            testMode = true,
            logLevel = LogLevel.DEBUG,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = baseUrl,
            strictModePenaltyDeath = true
        )

        BelAds.initialize(app.applicationContext, "sample-app", cfg)

        val latch = CountDownLatch(1)
        var callbackOnMain = false

        BelInterstitial.load(app.applicationContext, "pl_strict_sample", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                latch.countDown()
            }

            override fun onError(error: AdError, message: String) {
                callbackOnMain = (Looper.myLooper() == Looper.getMainLooper())
                latch.countDown()
            }
        })

        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue("Callback should arrive", latch.await(1, TimeUnit.SECONDS))
        assertTrue("Error callback should run on main thread", callbackOnMain)
        // Any main-thread network/disk access would have triggered StrictMode penaltyDeath earlier.
    }

}
