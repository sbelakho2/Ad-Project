package com.rivalapexmediation.sdk.dx

import android.content.Context
import android.widget.FrameLayout
import androidx.test.core.app.ApplicationProvider
import com.google.gson.Gson
import com.rivalapexmediation.sdk.*
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * Verifies BelBanner.attach/detach behavior:
 * - Attaching on main thread renders a test placeholder in test mode when no cached creative exists.
 * - Detach clears the container.
 */
@RunWith(RobolectricTestRunner::class)
class BannerAttachDetachTest {
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
        try { server.shutdown() } catch (_: Throwable) {}
    }

    private fun configBody(placementId: String = "pl_banner"): String {
        val body = mapOf(
            "configId" to "cfg-banner",
            "version" to 1,
            "placements" to mapOf(
                placementId to mapOf(
                    "placementId" to placementId,
                    "adType" to "BANNER",
                    "enabledNetworks" to emptyList<String>(),
                    "timeoutMs" to 300,
                    "maxWaitMs" to 600,
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
    fun attach_rendersPlaceholderInTestMode_then_detach_clears() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Only need config for SDK init
        server.enqueue(MockResponse().setResponseCode(200).setBody(configBody()))

        val cfg = SDKConfig(
            appId = "app-1",
            testMode = true,
            logLevel = LogLevel.DEBUG,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = baseUrl
        )
        BelAds.initialize(appContext, "app-1", cfg)

        val container = FrameLayout(appContext)
        // Attach banner without a cached creative should render placeholder in test mode
        BelBanner.attach(container, placementId = "pl_banner")
        assertTrue("Expected one child after attach()", container.childCount >= 1)
        // Detach should clear views
        BelBanner.detach(container)
        assertEquals(0, container.childCount)
    }
}
