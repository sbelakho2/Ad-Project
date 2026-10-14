package com.rivalapexmediation.sdk.adapters.ironsource

import android.app.Activity
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.google.gson.Gson
import com.rivalapexmediation.sdk.contract.AdapterConfig
import com.rivalapexmediation.sdk.contract.AdapterCredentials
import com.rivalapexmediation.sdk.contract.AdapterError
import com.rivalapexmediation.sdk.contract.AdapterOptions
import com.rivalapexmediation.sdk.contract.AuctionMeta
import com.rivalapexmediation.sdk.contract.CloseReason
import com.rivalapexmediation.sdk.contract.ConsentState
import com.rivalapexmediation.sdk.contract.ConnectionType
import com.rivalapexmediation.sdk.contract.ContextMeta
import com.rivalapexmediation.sdk.contract.DeviceMeta
import com.rivalapexmediation.sdk.contract.NetworkMeta
import com.rivalapexmediation.sdk.contract.PaidEvent
import com.rivalapexmediation.sdk.contract.RequestMeta
import com.rivalapexmediation.sdk.contract.ShowCallbacks
import com.rivalapexmediation.sdk.contract.UserMeta
import com.rivalapexmediation.sdk.contract.Orientation
import com.rivalapexmediation.sdk.contract.ErrorCode
import com.rivalapexmediation.sdk.contract.RewardedCallbacks
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import java.util.concurrent.atomic.AtomicInteger

@RunWith(RobolectricTestRunner::class)
class IronSourceAdapterTest {
    private lateinit var server: MockWebServer
    private lateinit var context: Context
    private lateinit var renderer: FakeRenderer
    private lateinit var adapter: IronSourceAdapter

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
        context = ApplicationProvider.getApplicationContext()
        renderer = FakeRenderer()
        adapter = IronSourceAdapter(context, renderer)
        val endpoint = server.url("/mediation").toString()
        val result = adapter.init(testConfig(endpoint), 200)
        assertTrue(result.success)
    }

    @After
    fun tearDown() {
        server.shutdown()
    }

    @Test
    fun loadInterstitial_success_returnsHandle() {
        enqueueAdResponse()
        val meta = requestMeta()
        val result = adapter.loadInterstitial("placement_a", meta, 500)
        assertNotNull(result.handle)
        assertEquals(1_500_000L, result.priceMicros)
        assertEquals("USD", result.currency)
    }

    @Test
    fun showInterstitial_dispatchesCallbacksAndPaidEvent() {
        enqueueAdResponse()
        val meta = requestMeta()
        val loadResult = adapter.loadInterstitial("placement_a", meta, 500)
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val callback = RecordingShowCallbacks()
        adapter.showInterstitial(loadResult.handle, activity, callback)
        assertEquals(1, renderer.interstitialShows.get())
        assertTrue(callback.impression)
        assertTrue(callback.closed)
        assertNotNull(callback.paid)
    }

    @Test
    fun loadInterstitial_includesPrivacyMetadata() {
        enqueueAdResponse()
        val consent = ConsentState(
            iabTcfV2 = "tc_string",
            iabUsPrivacy = "1YNN",
            coppa = true,
            limitAdTracking = false,
            advertisingId = "gaid-123"
        )
        val meta = requestMeta().copy(
            user = UserMeta(
                ageRestricted = true,
                consent = consent,
                advertisingId = "gaid-123",
                appSetId = "set-abc"
            )
        )
        adapter.loadInterstitial("placement_a", meta, 500)
        val recorded = server.takeRequest()
        val payload = Gson().fromJson(recorded.body.readUtf8(), IronSourceBidRequest::class.java)
        assertEquals("tc_string", payload.consent.gdpr)
        assertEquals("1YNN", payload.consent.usPrivacy)
        assertTrue(payload.consent.coppa)
        assertEquals("gaid-123", payload.advertisingId)
        assertFalse(payload.lmt)
    }

    @Test
    fun loadRewarded_includesPrivacyMetadata() {
        enqueueAdResponse()
        val base = requestMeta()
        val meta = base.copy(
            user = base.user.copy(
                advertisingId = "gaid-999",
                consent = base.user.consent.copy(limitAdTracking = false, advertisingId = "gaid-999")
            )
        )
        adapter.loadRewarded("placement_a", meta, 500)
        val recorded = server.takeRequest()
        val payload = Gson().fromJson(recorded.body.readUtf8(), IronSourceBidRequest::class.java)
        assertEquals("gaid-999", payload.advertisingId)
        assertFalse(payload.lmt)
    }

    private fun enqueueAdResponse() {
        val body = mapOf(
            "providerName" to "is_provider",
            "revenue" to 1.5,
            "auctionId" to "auction-1",
            "creativeId" to "creative-1",
            "adMarkup" to "<html><body>test</body></html>",
            "instanceId" to "inst-1",
            "ttl" to 120_000
        )
        server.enqueue(MockResponse().setResponseCode(200).setBody(Gson().toJson(body)))
    }

    private fun testConfig(endpoint: String): AdapterConfig {
        return AdapterConfig(
            partner = "ironsource",
            credentials = AdapterCredentials(
                key = "app_key",
                secret = "secret",
                appId = "com.example.app",
                accountIds = mapOf("endpoint" to endpoint)
            ),
            placements = mapOf("placement_a" to "instance_a"),
            privacy = ConsentState(),
            region = null,
            options = AdapterOptions(testMode = true, bidFloorMicros = 400_000)
        )
    }

    private fun requestMeta(): RequestMeta = RequestMeta(
        requestId = "req-1",
        device = DeviceMeta(os = "android", osVersion = "14", model = "Pixel"),
        user = UserMeta(ageRestricted = false, consent = ConsentState()),
        net = NetworkMeta(ipPrefixed = "1.1.1.0", uaNormalized = "ua", connType = ConnectionType.WIFI),
        context = ContextMeta(orientation = Orientation.PORTRAIT, sessionDepth = 1),
        auction = AuctionMeta(floorsMicros = 400_000, sChain = null, sellersJsonOk = true)
    )

    private class FakeRenderer : IronSourceCreativeRenderer() {
        val interstitialShows = AtomicInteger()
        override fun showInterstitial(activity: Activity, markup: String, paidEvent: PaidEvent?, callbacks: ShowCallbacks) {
            interstitialShows.incrementAndGet()
            callbacks.onImpression()
            paidEvent?.let(callbacks::onPaidEvent)
            callbacks.onClosed(CloseReason.DISMISSED)
        }

        override fun showRewarded(
            activity: Activity,
            markup: String,
            paidEvent: PaidEvent?,
            callbacks: RewardedCallbacks,
            rewardType: String,
            rewardAmount: Double
        ) {
            // Not used in these tests, but override to guard against accidental calls
            throw AssertionError("Rewarded path should not be invoked")
        }
    }

    private class RecordingShowCallbacks : ShowCallbacks {
        var impression = false
        var closed = false
        var paid: PaidEvent? = null
        override fun onImpression(meta: Map<String, Any?>) { impression = true }
        override fun onPaidEvent(event: PaidEvent) { paid = event }
        override fun onClick(meta: Map<String, Any?>) {}
        override fun onClosed(reason: CloseReason) { closed = true }
        override fun onError(error: AdapterError) { throw AssertionError("Unexpected error $error") }
    }
}
