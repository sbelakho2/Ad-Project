package com.rivalapexmediation.sdk.dx

import android.content.Context
import android.os.Looper
import android.os.StrictMode
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
 * StrictMode smoke test to ensure SDK public flows do not perform network/disk I/O on the main thread.
 * The test enables StrictMode penalties and exercises a typical init + load (204 no_fill) flow.
 */
@RunWith(RobolectricTestRunner::class)
class StrictModeSmokeTest {
    private lateinit var server: MockWebServer
    private lateinit var appContext: Context

    @Before
    fun setup() {
        server = MockWebServer()
        server.start()
        appContext = ApplicationProvider.getApplicationContext()

        // Enable strict thread policy with network/disk detection and penaltyDeath (will throw in tests).
        StrictMode.setThreadPolicy(
            StrictMode.ThreadPolicy.Builder()
                .detectNetwork()
                .detectDiskReads()
                .detectDiskWrites()
                .penaltyDeath()
                .penaltyLog()
                .build()
        )
        StrictMode.setVmPolicy(
            StrictMode.VmPolicy.Builder()
                .detectLeakedClosableObjects()
                .detectActivityLeaks()
                .penaltyLog()
                .build()
        )
    }

    @After
    fun teardown() {
        try { server.shutdown() } catch (_: Throwable) {}
    }

    private fun configBody(placementId: String = "pl_smoke"): String {
        val body = mapOf(
            "configId" to "cfg-strict",
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
            // Signature ignored in testMode
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    @Test
    fun init_and_load_run_without_mainThreadIO_violations() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Enqueue config, then a 204 (no_fill) for auction
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody()))
        server.enqueue(MockResponse().setResponseCode(204))

        val cfg = SDKConfig(
            appId = "app-1",
            testMode = true, // bypass signature in tests
            logLevel = LogLevel.DEBUG,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = baseUrl,
            strictModePenaltyDeath = true // mirrors CI smoke expectation
        )

        // Initialize SDK (should not perform network/disk on main thread)
        BelAds.initialize(appContext, "app-1", cfg)

        val latch = CountDownLatch(1)
        var errorOnMain = false

        // Exercise a load flow; the SDK should do I/O off the main thread.
        BelInterstitial.load(appContext, "pl_smoke", object : AdLoadCallback {
            override fun onAdLoaded(ad: com.rivalapexmediation.sdk.models.Ad) {
                // Not expected in 204 path
                latch.countDown()
                fail("Expected no_fill, not success")
            }
            override fun onError(error: AdError, message: String) {
                errorOnMain = (Looper.myLooper() == Looper.getMainLooper())
                latch.countDown()
            }
        })

        // Let main thread callbacks run
        Shadows.shadowOf(Looper.getMainLooper()).idle()
        assertTrue("Callback should arrive", latch.await(1, TimeUnit.SECONDS))
        assertTrue("Error callback should be on main thread", errorOnMain)
        // If any network/disk I/O occurred on main, StrictMode.penaltyDeath would have thrown and failed the test.
    }
}
