package com.rivalapexmediation.sdk.telemetry

import android.content.Context
import com.rivalapexmediation.sdk.SDKConfig
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.MockResponse
import io.mockk.mockk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.util.concurrent.TimeUnit

class TelemetryCollectorNetworkTest {
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
    fun `flushes batch with gzip to telemetry endpoint`() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        // Accept any POST
        server.enqueue(MockResponse().setResponseCode(200).setBody("{}"))

        val cfg = SDKConfig(
            appId = "app-1",
            testMode = true,
            telemetryEnabled = true,
            configEndpoint = baseUrl,
            auctionEndpoint = "http://localhost"
        )
        val ctx = mockk<Context>(relaxed = true)
        val tc = TelemetryCollector(ctx, cfg)
        tc.start()

        // Emit exactly 10 events to trigger immediate flush (batchSize = 10)
        repeat(10) { tc.recordInitialization() }

        val req = server.takeRequest(2, TimeUnit.SECONDS)
        requireNotNull(req) { "Expected a telemetry POST" }
        assertEquals("/v1/telemetry", req.path)
        // Verify gzip header
        assertEquals("gzip", req.getHeader("Content-Encoding"))
        // Body should be non-empty compressed JSON
        val bytes = req.body.readByteArray()
        assertTrue("Expected some compressed payload", bytes.isNotEmpty())
    }

    @Test
    fun `auction latency event is emitted with placement auction`() {
        val baseUrl = server.url("/").toString().trimEnd('/')
        server.enqueue(MockResponse().setResponseCode(200).setBody("{}"))

        val cfg = SDKConfig(
            appId = "app-telemetry",
            testMode = true,
            telemetryEnabled = true,
            configEndpoint = baseUrl,
            auctionEndpoint = "http://localhost",
            observabilityEnabled = true,
            observabilitySampleRate = 1.0,
        )
        val ctx = mockk<Context>(relaxed = true)
        val tc = TelemetryCollector(ctx, cfg)
        tc.start()

        // Emit 10 latency events to trigger an immediate flush (batch size = 10)
        repeat(10) {
            tc.recordAuctionClientLatency(outcome = if (it % 2 == 0) "success" else "timeout", latencyMs = 123L + it)
        }

        val req = server.takeRequest(2, TimeUnit.SECONDS)
        requireNotNull(req) { "Expected a telemetry POST" }
        assertEquals("/api/v1/analytics/byo/spans", req.path)
        val body = req.body.readUtf8()
        assertTrue("Expected auction placement telemetry", body.contains("\"placement\":\"auction\""))
        assertTrue("Expected client network tag", body.contains("\"networkName\":\"client\""))
    }
}
