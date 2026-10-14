package network

import android.content.Context
import com.google.gson.Gson
import com.rivalapexmediation.ctv.SDKConfig
import com.rivalapexmediation.ctv.network.AuctionClient
import com.rivalapexmediation.ctv.network.LoadError
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class AuctionClientTests {
    private lateinit var server: MockWebServer
    private lateinit var context: Context

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
        context = RuntimeEnvironment.getApplication()
    }

    @After
    fun tearDown() {
        server.shutdown()
    }

    @Test
    fun `returns no_fill on 204`() {
        server.enqueue(MockResponse().setResponseCode(204))
        val cfg = SDKConfig(appId = "app-1", apiBaseUrl = server.url("/api/v1").toString())
        val client = AuctionClient(context, cfg)
        var err: LoadError? = null
        client.requestBid("p1", "interstitial", 0.0, null) { res ->
            err = res.error
        }
        Thread.sleep(100) // wait async
        assertTrue(err is LoadError.NoFill)
    }

    @Test
    fun `parses 200 success envelope`() {
        val body = mapOf(
            "response" to mapOf(
                "requestId" to "r1",
                "bidId" to "b1",
                "adapter" to "admob",
                "cpm" to 1.23,
                "currency" to "USD",
                "ttlSeconds" to 300,
                "creativeUrl" to "https://cdn.test/video.mp4",
                "tracking" to mapOf("impression" to "http://i", "click" to "http://c"),
                "payload" to emptyMap<String, Any>()
            )
        )
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(body)).setHeader("Content-Type","application/json"))
        val cfg = SDKConfig(appId = "app-1", apiBaseUrl = server.url("/api/v1").toString())
        val client = AuctionClient(context, cfg)
        var ok = false
        var err: LoadError? = null
        client.requestBid("p1", "interstitial", 0.0, null) { res ->
            ok = res.win != null && res.error == null
            err = res.error
        }
        Thread.sleep(100)
        assertTrue(ok)
        assertEquals(null, err)
    }
}
