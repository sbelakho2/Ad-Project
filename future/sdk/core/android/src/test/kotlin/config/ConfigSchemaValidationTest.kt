package com.rivalapexmediation.sdk.config

import android.content.Context
import android.content.SharedPreferences
import com.google.gson.Gson
import com.rivalapexmediation.sdk.SDKConfig
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import org.junit.Assert.assertThrows

/**
 * Validates that ConfigManager rejects malformed remote configs via validateSchema().
 * We run in testMode=true to bypass signature checks but still exercise schema validation.
 */
class ConfigSchemaValidationTest {
    private lateinit var server: MockWebServer

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
    }

    @After
    fun tearDown() {
        server.shutdown()
    }

    private fun mockPrefs(): SharedPreferences {
        val editor = io.mockk.mockk<SharedPreferences.Editor>(relaxed = true)
        val prefs = io.mockk.mockk<SharedPreferences>()
        io.mockk.every { prefs.getString(any(), any()) } returns null
        io.mockk.every { prefs.getLong(any(), any()) } returns 0L
        io.mockk.every { prefs.edit() } returns editor
        return prefs
    }

    private fun mockContext(prefs: SharedPreferences): Context {
        val ctx = io.mockk.mockk<Context>()
        io.mockk.every { ctx.getSharedPreferences("rival_ad_stack_config", Context.MODE_PRIVATE) } returns prefs
        return ctx
    }

    private fun baseConfigJson(placementId: String, timeoutMs: Long, maxWaitMs: Long): String {
        val body = mapOf(
            "configId" to "cfg-bad",
            "version" to 1,
            "placements" to mapOf(
                placementId to mapOf(
                    "placementId" to placementId,
                    "adType" to "INTERSTITIAL",
                    "enabledNetworks" to emptyList<String>(),
                    "timeoutMs" to timeoutMs,
                    "maxWaitMs" to maxWaitMs,
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
            // Signature is ignored in testMode
            "signature" to "test",
            "timestamp" to 1111111111
        )
        return Gson().toJson(body)
    }

    @Test
    fun rejects_timeout_out_of_bounds() {
        // timeoutMs > 30000 should be rejected by validateSchema
        val bad = baseConfigJson(placementId = "pl_toobig", timeoutMs = 60000, maxWaitMs = 1200)
        server.enqueue(MockResponse().setResponseCode(200).setBody(bad))

        val baseUrl = server.url("/").toString().trimEnd('/')
        val sdkCfg = SDKConfig(
            appId = "app-1",
            testMode = true, // bypass signature but not schema
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = baseUrl
        )
        val mgr = ConfigManager(
            context = mockContext(mockPrefs()),
            sdkConfig = sdkCfg
        )

        assertThrows(IllegalStateException::class.java) {
            mgr.loadConfig()
        }
        // Should fail-closed and not populate placements
        val pl = mgr.getPlacementConfig("pl_toobig")
        assertNull(pl)
    }

    @Test
    fun rejects_missing_or_blank_placementId() {
        // placement key provided but placementId blank should be rejected
        val body = mapOf(
            "configId" to "cfg-blank",
            "version" to 1,
            "placements" to mapOf(
                "pl_blank" to mapOf(
                    "placementId" to "", // invalid
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
            "signature" to "test",
            "timestamp" to 1111111111
        )
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(body)))

        val baseUrl = server.url("/").toString().trimEnd('/')
        val sdkCfg = SDKConfig(
            appId = "app-1",
            testMode = true,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = baseUrl
        )
        val mgr = ConfigManager(
            context = mockContext(mockPrefs()),
            sdkConfig = sdkCfg
        )

        assertThrows(IllegalStateException::class.java) {
            mgr.loadConfig()
        }
        val pl = mgr.getPlacementConfig("pl_blank")
        assertNull(pl)
    }
}
