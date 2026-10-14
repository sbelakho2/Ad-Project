package com.rivalapexmediation.sdk.network

import android.os.Handler
import android.os.Looper
import com.google.gson.Gson
import com.rivalapexmediation.sdk.threading.CircuitBreaker
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows
import org.robolectric.annotation.Config
import java.net.ConnectException
import java.net.UnknownHostException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [33])
class AuctionClientTest {
    private fun takeRequestOrFail(): okhttp3.mockwebserver.RecordedRequest {
        val req = server.takeRequest(1, java.util.concurrent.TimeUnit.SECONDS)
        return req ?: error("no request recorded")
    }
    private lateinit var server: MockWebServer
    private lateinit var client: AuctionClient

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

    private fun opts(timeoutMs: Int = 800) = AuctionClient.InterstitialOptions(
        publisherId = "pub-1",
        placementId = "pl-1",
        floorCpm = 0.0,
        adapters = listOf("admob"),
        metadata = emptyMap(),
        timeoutMs = timeoutMs,
        auctionType = "header_bidding",
    )

    @Test
    fun success200_parsesWinner() {
        val body = mapOf(
            "winner" to mapOf(
                "adapter_name" to "admob",
                "cpm" to 2.5,
                "currency" to "USD",
                "creative_id" to "cr",
                "ad_markup" to "<div>ad</div>"
            )
        )
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(body)))

        val res = client.requestInterstitial(opts())
        assertEquals("admob", res.adapter)
        assertEquals(2.5, res.ecpm, 0.0001)
        assertEquals("USD", res.currency)
        assertEquals("cr", res.creativeId)
    }

    @Test
    fun noWinnerInBody_mapsToNoFill() {
        val body = mapOf("some" to "field")
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(body)))
        try {
            client.requestInterstitial(opts())
            fail("expected no_fill")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("no_fill", e.reason)
        }
    }

    @Test
    fun status400_noRetry_andMapsReason() {
        server.enqueue(MockResponse().setResponseCode(400).setBody("bad"))
        try {
            client.requestInterstitial(opts())
            fail("expected status_400")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("status_400", e.reason)
        }
        // Only one request should have been made
        assertEquals(1, server.requestCount)
    }

    @Test
    fun retryOn5xx_thenSuccess() {
        // First 500, then 200 OK
        server.enqueue(MockResponse().setResponseCode(500).setBody("oops"))
        val body = mapOf("winner" to mapOf("adapter_name" to "meta", "cpm" to 1.1, "currency" to "USD"))
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(body)))

        val res = client.requestInterstitial(opts())
        assertEquals("meta", res.adapter)
        assertEquals(2, server.requestCount)
    }

    @Test
    fun timeoutMapsToTimeout() {
        // Delay the body well beyond timeout
        server.enqueue(
            MockResponse()
                .setResponseCode(200)
                .setBody("{}")
                .setBodyDelay(2, TimeUnit.SECONDS)
        )
        try {
            client.requestInterstitial(opts(timeoutMs = 50))
            fail("expected timeout")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("timeout", e.reason)
        }
    }

    @Test
    fun rateLimited429_mapsReasonWithoutRetry() {
        server.enqueue(
            MockResponse()
                .setResponseCode(429)
                .setBody("slow down")
                .setHeader("Retry-After", "2")
        )
        try {
            client.requestInterstitial(opts())
            fail("expected rate_limited")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("rate_limited", e.reason)
            assertTrue(e.message?.contains("retry_after_ms") == true)
        }
        assertEquals(1, server.requestCount)
    }

    @Test
    fun circuitBreakerTripsAfterTransientFailures() {
        server.enqueue(MockResponse().setResponseCode(500))
        server.enqueue(MockResponse().setResponseCode(500))
        server.enqueue(MockResponse().setResponseCode(500))
        val breakerClient = AuctionClient(
            server.url("/").toString().trimEnd('/'),
            apiKey = "test-key",
            httpClient = null,
            circuitBreakerFactory = {
                CircuitBreaker(failureThreshold = 1, resetTimeoutMs = 60_000, halfOpenMaxAttempts = 1)
            }
        )
        try {
            breakerClient.requestInterstitial(opts())
            fail("expected status_500 on first call")
        } catch (e: AuctionClient.AuctionException) {
            assertTrue(e.reason.startsWith("status_5"))
        }
        try {
            breakerClient.requestInterstitial(opts())
            fail("expected circuit_open on second call")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("circuit_open", e.reason)
        }
        assertEquals(3, server.requestCount)
    }

    @Test
    fun cancellationMapsToNavigationCancelled() {
        val cancelingClient = AuctionClient(
            server.url("/").toString().trimEnd('/'),
            apiKey = "test-key",
            httpClient = OkHttpClient.Builder()
                .addInterceptor { throw java.io.IOException("Canceled") }
                .build()
        )
        try {
            cancelingClient.requestInterstitial(opts())
            fail("expected navigation_cancelled")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("navigation_cancelled", e.reason)
        }
    }

    @Test
    fun airplaneMode_connectFailure_retriesAndMapsNetworkError() {
        val attempts = AtomicInteger(0)
        val failingClient = AuctionClient(
            baseUrl = "http://localhost",
            apiKey = "test-key",
            httpClient = OkHttpClient.Builder()
                .addInterceptor {
                    attempts.incrementAndGet()
                    throw ConnectException("failed to connect")
                }
                .build()
        )
        try {
            failingClient.requestInterstitial(opts())
            fail("expected network_error")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("network_error", e.reason)
        }
        assertEquals(3, attempts.get())
    }

    @Test
    fun dnsFailure_unknownHost_retriesAndMapsNetworkError() {
        val attempts = AtomicInteger(0)
        val dnsClient = AuctionClient(
            baseUrl = "http://does-not-resolve",
            apiKey = "test-key",
            httpClient = OkHttpClient.Builder()
                .addInterceptor {
                    attempts.incrementAndGet()
                    throw UnknownHostException("no dns")
                }
                .build()
        )
        try {
            dnsClient.requestInterstitial(opts())
            fail("expected network_error")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("network_error", e.reason)
        }
        assertEquals(3, attempts.get())
    }

    @Test
    fun captivePortal_redirect_mapsStatus302WithoutRetry() {
        server.enqueue(
            MockResponse()
                .setResponseCode(302)
                .setHeader("Location", "http://captive.portal/login")
                .setBody("redirect")
        )
        try {
            client.requestInterstitial(opts())
            fail("expected status_302")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("status_302", e.reason)
        }
        assertEquals(1, server.requestCount)
    }

    @Test
    fun networkFlip_disconnectAfterRequest_recoversOnRetry() {
        val winner = mapOf(
            "winner" to mapOf(
                "adapter_name" to "admob",
                "cpm" to 3.2,
                "currency" to "USD",
                "creative_id" to "cr-2",
                "ad_markup" to "<div>ad</div>"
            )
        )
        server.enqueue(
            MockResponse()
                .setResponseCode(200)
                .setBody(Gson().toJson(winner))
        )
        server.enqueue(
            MockResponse()
                .setResponseCode(200)
                .setBody(Gson().toJson(winner))
        )

        val flipCounter = AtomicInteger(0)
        val flakyClient = AuctionClient(
            baseUrl = server.url("/").toString().trimEnd('/'),
            apiKey = "test-key",
            httpClient = OkHttpClient.Builder()
                .addInterceptor { chain ->
                    val attempt = flipCounter.getAndIncrement()
                    if (attempt == 0) {
                        val resp = chain.proceed(chain.request())
                        resp.close()
                        throw java.io.IOException("connection reset mid-response")
                    }
                    chain.proceed(chain.request())
                }
                .build()
        )

        val result: AuctionClient.InterstitialResult = try {
            flakyClient.requestInterstitial(opts())
        } catch (e: AuctionClient.AuctionException) {
            throw AssertionError("expected success after retry, got ${e.reason}; requests=${server.requestCount}")
        }
        assertEquals("admob", result.adapter)
        assertTrue("should have performed at least two attempts", flipCounter.get() >= 2)
        assertEquals(2, server.requestCount)
    }

    @Test
    fun consentSerialization_setsMetadataFlags() {
        // Return a no_fill to avoid success path; we'll inspect recorded request
        server.enqueue(MockResponse().setResponseCode(204))
        try {
            client.requestInterstitial(
                opts(),
                AuctionClient.ConsentOptions(
                    gdprApplies = true,
                    tcfString = "TCF",
                    usPrivacy = "1YNN",
                    coppa = false,
                    limitAdTracking = true,
                    privacySandbox = true,
                    advertisingId = "gaid-will-be-ignored",
                    appSetId = "appset-123"
                )
            )
            fail("expected no_fill")
        } catch (_: AuctionClient.AuctionException) {
            // expected no_fill
        }
        val recorded = server.takeRequest(1, TimeUnit.SECONDS) ?: error("no request")
        val payload = Gson().fromJson(recorded.body.readUtf8(), Map::class.java)
        @Suppress("UNCHECKED_CAST")
        val meta = payload["metadata"] as Map<String, String>
        assertEquals("1", meta["gdpr_applies"]) // true → "1"
        assertEquals("1YNN", meta["us_privacy"])
        assertEquals("0", meta["coppa"]) // false → "0"
        assertEquals("1", meta["privacy_sandbox"])
        @Suppress("UNCHECKED_CAST")
        val userInfo = payload["user_info"] as Map<String, Any?>
        assertEquals(true, userInfo["privacy_sandbox"])
        assertEquals(true, userInfo["limit_ad_tracking"])
        assertEquals("appset-123", userInfo["app_set_id"])
        assertNull(userInfo["advertising_id"])
    }

    @Test
    fun advertisingIdSentWhenLatDisabled() {
        server.enqueue(MockResponse().setResponseCode(204))
        try {
            client.requestInterstitial(
                opts(),
                AuctionClient.ConsentOptions(
                    gdprApplies = false,
                    tcfString = null,
                    usPrivacy = null,
                    coppa = null,
                    limitAdTracking = false,
                    privacySandbox = false,
                    advertisingId = "gaid-789",
                    appSetId = "appset-should-not-send"
                )
            )
            fail("expected no_fill")
        } catch (_: AuctionClient.AuctionException) {
            // expected
        }
        val recorded = takeRequestOrFail()
        val payload = Gson().fromJson(recorded.body.readUtf8(), Map::class.java)
        @Suppress("UNCHECKED_CAST")
        val userInfo = payload["user_info"] as Map<String, Any?>
        assertEquals("gaid-789", userInfo["advertising_id"])
        assertFalse(userInfo["limit_ad_tracking"] as Boolean)
        assertFalse(userInfo.containsKey("app_set_id"))
    }

    @Test
    fun headersContainUserAgentAndApiKey() {
        // Return 204 to force no_fill and stop after one request
        server.enqueue(MockResponse().setResponseCode(204))
        try {
            client.requestInterstitial(opts(), AuctionClient.ConsentOptions())
            fail("expected no_fill")
        } catch (_: AuctionClient.AuctionException) { /* expected */ }
        val recorded = takeRequestOrFail()
        val ua = recorded.getHeader("User-Agent")
        val apiKey = recorded.getHeader("X-Api-Key")
        assertNotNull("User-Agent header must be set", ua)
        assertNotNull("X-Api-Key header must be set", apiKey)
        // Basic UA sanity
        require(ua!!.contains("RivalApexMediation-Android")) { "UA should contain SDK tag" }
    }

    @Test
    fun requestIdFormat_isGeneratedAndLooksReasonable() {
        // Force a single request by returning 204 (no_fill)
        server.enqueue(MockResponse().setResponseCode(204))
        try {
            client.requestInterstitial(opts())
            fail("expected no_fill")
        } catch (_: AuctionClient.AuctionException) { /* expected */ }
        val recorded = server.takeRequest(1, TimeUnit.SECONDS) ?: error("no request")
        val payload = Gson().fromJson(recorded.body.readUtf8(), Map::class.java)
        val reqId = payload["request_id"] as? String ?: error("missing request_id")
        // Expect format: android-<millis>-<rand>
        val regex = Regex("^android-\\d{10,}-\\d{1,6}$")
        assertTrue("request_id should match pattern", regex.containsMatchIn(reqId))
    }

    @Test
    fun malformedWinner_missingAdapterOrCpm_mapsToNoFill() {
        // Winner object present but missing critical fields should map to no_fill
        val bodyMissingAdapter = mapOf("winner" to mapOf(
            // "adapter_name" missing
            "cpm" to 1.0,
            "currency" to "USD",
            "creative_id" to "cr",
        ))
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(bodyMissingAdapter)))
        try {
            client.requestInterstitial(opts())
            fail("expected no_fill due to missing adapter_name")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("no_fill", e.reason)
        }
        // Next: missing cpm
        val bodyMissingCpm = mapOf("winner" to mapOf(
            "adapter_name" to "admob",
            // "cpm" missing
            "currency" to "USD",
            "creative_id" to "cr",
        ))
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(bodyMissingCpm)))
        try {
            client.requestInterstitial(opts())
            fail("expected no_fill due to missing cpm")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("no_fill", e.reason)
        }
    }

    @Test
    fun mainThreadGuard_allowsRobolectricOnMainLooper() {
        // Ensure the test runtime (Robolectric) bypasses the main-thread guard; should surface auction result, not main_thread
        server.enqueue(MockResponse().setResponseCode(204))
        try {
            client.requestInterstitial(opts())
            fail("expected no_fill, not success")
        } catch (e: AuctionClient.AuctionException) {
            assertEquals("no_fill", e.reason)
            assertNotEquals("main_thread", e.reason)
        }
    }

    @Test
    fun mainThreadGuard_blocksWhenForcedProdInTests() {
        val prev = System.getProperty("bel.force.testRuntime")
        System.setProperty("bel.force.testRuntime", "false")
        val latch = java.util.concurrent.CountDownLatch(1)
        val captured = java.util.concurrent.atomic.AtomicReference<AuctionClient.AuctionException?>()
        Handler(Looper.getMainLooper()).post {
            try {
                client.requestInterstitial(opts())
            } catch (e: AuctionClient.AuctionException) {
                captured.set(e)
            } finally {
                latch.countDown()
            }
        }
        Shadows.shadowOf(Looper.getMainLooper()).idle()
        latch.await(1, java.util.concurrent.TimeUnit.SECONDS)
        val ex = captured.get()
        assertNotNull("expected main_thread exception", ex)
        assertEquals("main_thread", ex?.reason)
        try {
            // no-op
        } finally {
            if (prev == null) System.clearProperty("bel.force.testRuntime") else System.setProperty("bel.force.testRuntime", prev)
        }
    }
}