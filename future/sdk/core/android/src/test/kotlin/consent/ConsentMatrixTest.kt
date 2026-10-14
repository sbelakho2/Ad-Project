package com.rivalapexmediation.sdk.consent

import com.google.gson.Gson
import com.rivalapexmediation.sdk.network.AuctionClient
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import java.util.concurrent.TimeUnit

/**
 * Validates that consent combinations are correctly serialized into AuctionClient request metadata.
 * This is a pure JVM test using MockWebServer; no Android device/emulator required.
 */
class ConsentMatrixTest {
    private lateinit var server: MockWebServer
    private lateinit var client: AuctionClient

    private fun opts(timeoutMs: Int = 200) = AuctionClient.InterstitialOptions(
        publisherId = "pub-1",
        placementId = "pl-1",
        floorCpm = 0.0,
        adapters = listOf("admob"),
        metadata = emptyMap(),
        timeoutMs = timeoutMs,
        auctionType = "header_bidding",
    )

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
        val baseUrl = server.url("/").toString().trimEnd('/')
        client = AuctionClient(baseUrl, apiKey = "test-key")
    }

    @After
    fun tearDown() {
        server.shutdown()
    }

    private fun takePayload(): Map<*, *> {
        val recorded = server.takeRequest(1, TimeUnit.SECONDS) ?: error("no request recorded")
        return Gson().fromJson(recorded.body.readUtf8(), Map::class.java)
    }

    @Test
    fun gdprTrue_coppaFalse_latTrue_setsExpectedFlags() {
        // Force a single request by returning 204 (maps to no_fill and throws)
        server.enqueue(MockResponse().setResponseCode(204))
        try {
            client.requestInterstitial(
                opts(),
                AuctionClient.ConsentOptions(
                    gdprApplies = true,
                    tcfString = "TCF",
                    usPrivacy = null,
                    coppa = false,
                    limitAdTracking = true,
                )
            )
            fail("expected no_fill")
        } catch (_: AuctionClient.AuctionException) { /* expected */ }
        val payload = takePayload()
        @Suppress("UNCHECKED_CAST")
        val meta = payload["metadata"] as Map<String, String>
        // gdpr_applies true -> "1"; coppa false -> "0"; LAT true is carried via user_info.limit_ad_tracking
        assertEquals("1", meta["gdpr_applies"])
        assertEquals("0", meta["coppa"])
        @Suppress("UNCHECKED_CAST")
        val user = payload["user_info"] as Map<String, Any?>
        assertEquals(true, user["limit_ad_tracking"]) 
    }

    @Test
    fun gdprFalse_uspString_setsExpectedFlags() {
        server.enqueue(MockResponse().setResponseCode(204))
        try {
            client.requestInterstitial(
                opts(),
                AuctionClient.ConsentOptions(
                    gdprApplies = false,
                    tcfString = "TCF",
                    usPrivacy = "1YNN",
                    coppa = null,
                    limitAdTracking = null,
                )
            )
            fail("expected no_fill")
        } catch (_: AuctionClient.AuctionException) { /* expected */ }
        val payload = takePayload()
        @Suppress("UNCHECKED_CAST")
        val meta = payload["metadata"] as Map<String, String>
        assertEquals("0", meta["gdpr_applies"])
        assertEquals("1YNN", meta["us_privacy"])
    }

    @Test
    fun gdprUnknown_uspMissing_setsOnlyPresentFlags() {
        server.enqueue(MockResponse().setResponseCode(204))
        try {
            client.requestInterstitial(
                opts(),
                AuctionClient.ConsentOptions(
                    gdprApplies = null,
                    tcfString = null,
                    usPrivacy = null,
                    coppa = true,
                    limitAdTracking = false,
                )
            )
            fail("expected no_fill")
        } catch (_: AuctionClient.AuctionException) { /* expected */ }
        val payload = takePayload()
        @Suppress("UNCHECKED_CAST")
        val meta = payload["metadata"] as Map<String, String>
        // gdpr_applies should be absent when null; us_privacy absent; coppa true -> "1"
        if (meta.containsKey("gdpr_applies")) {
            throw AssertionError("gdpr_applies should not be present when null")
        }
        if (meta.containsKey("us_privacy")) {
            throw AssertionError("us_privacy should not be present when null")
        }
        assertEquals("1", meta["coppa"])
        @Suppress("UNCHECKED_CAST")
        val user = payload["user_info"] as Map<String, Any?>
        assertEquals(false, user["limit_ad_tracking"]) 
    }
}
