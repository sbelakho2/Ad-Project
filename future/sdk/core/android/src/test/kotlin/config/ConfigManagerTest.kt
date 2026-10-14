package com.rivalapexmediation.sdk.config

import android.content.Context
import android.content.SharedPreferences
import com.rivalapexmediation.sdk.SDKConfig
import io.mockk.every
import io.mockk.mockk
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertNotNull
import org.junit.Before
import org.junit.Test

class ConfigManagerTest {
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

    @Test
    fun `loads remote config in test mode and exposes placement`() {
        // Arrange server response
        val body = """
            {
              "configId":"cfg-1",
              "version":1,
              "placements": {
                "pl1": {
                  "placementId":"pl1",
                  "adType":"INTERSTITIAL",
                  "enabledNetworks": [],
                  "timeoutMs":3000,
                  "maxWaitMs":5000,
                  "floorPrice":0.0,
                  "refreshInterval":null,
                  "targeting": {}
                }
              },
              "adapters": {},
              "features": {"telemetryEnabled": true, "crashReportingEnabled": true, "debugLoggingEnabled": false, "experimentalFeaturesEnabled": false},
              "signature":"test",
              "timestamp": 1234567890
            }
        """.trimIndent()
        server.enqueue(MockResponse().setResponseCode(200).setBody(body))

        val baseUrl = server.url("/").toString().trimEnd('/')
        val cfg = SDKConfig(
            appId = "app-1",
            testMode = true,
            telemetryEnabled = false,
            configEndpoint = baseUrl,
            auctionEndpoint = "http://localhost"
        )

        // Mock SharedPreferences (used for cache)
        val editor = mockk<SharedPreferences.Editor>(relaxed = true)
        val prefs = mockk<SharedPreferences>()
        every { prefs.getString(any(), any()) } returns null
        every { prefs.getLong(any(), any()) } returns 0L
        every { prefs.edit() } returns editor

        val context = mockk<Context>()
        every { context.getSharedPreferences(any(), any()) } returns prefs

        val mgr = ConfigManager(context, cfg)

        // Act
        mgr.loadConfig()

        // Assert
        val pl = mgr.getPlacementConfig("pl1")
        assertNotNull("Expected placement from remote config", pl)
    }
}
