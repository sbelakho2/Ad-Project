package com.rivalapexmediation.sdk

import android.app.Application
import android.content.Context
import com.rivalapexmediation.sdk.contract.*
import com.rivalapexmediation.sdk.models.AdType
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class AdapterV2ConformanceTest {

    private class OrderTrackingCallbacks : RewardedCallbacks {
        val events = mutableListOf<String>()
        override fun onImpression(meta: Map<String, Any?>) { events.add("impression") }
        override fun onPaidEvent(event: PaidEvent) { /* ignored */ }
        override fun onClick(meta: Map<String, Any?>) { /* ignored */ }
        override fun onClosed(reason: CloseReason) { events.add("closed:${reason.name}") }
        override fun onError(error: AdapterError) { events.add("error:${error.code.name}") }
        override fun onRewardVerified(rewardType: String, rewardAmount: Double) {
            events.add("reward:$rewardType:${rewardAmount.toInt()}")
        }
    }

    private class FakeAdapter : AdNetworkAdapterV2 {
        override fun init(config: AdapterConfig, timeoutMs: Int): InitResult = InitResult(success = true)
        override fun loadInterstitial(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult =
            LoadResult(
                handle = AdHandle("ih-$placementId", AdType.INTERSTITIAL),
                ttlMs = 60_000,
                priceMicros = 2_340_000
            )

        override fun showInterstitial(handle: AdHandle, viewContext: Any, callbacks: ShowCallbacks) {
            callbacks.onImpression(emptyMap())
            callbacks.onClosed(CloseReason.COMPLETED)
        }

        override fun loadRewarded(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult =
            LoadResult(
                handle = AdHandle("rh-$placementId", AdType.REWARDED),
                ttlMs = 60_000,
                priceMicros = 3_210_000
            )

        override fun showRewarded(handle: AdHandle, viewContext: Any, callbacks: RewardedCallbacks) {
            callbacks.onImpression(emptyMap())
            callbacks.onRewardVerified("coins", 1.0)
            callbacks.onClosed(CloseReason.COMPLETED)
        }

        override fun loadBanner(placementId: String, size: AdSize, meta: RequestMeta, timeoutMs: Int): LoadResult =
            LoadResult(
                handle = AdHandle("bn-$placementId", AdType.BANNER),
                ttlMs = 60_000
            )

        override fun attachBanner(handle: AdHandle, bannerHost: Any, callbacks: BannerCallbacks) {
            callbacks.onViewAttached()
            callbacks.onViewDetached(CloseReason.DISMISSED)
        }

        override fun isAdReady(handle: AdHandle): Boolean = true

        override fun expiresAt(handle: AdHandle): Long = com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow() + 60_000

        override fun invalidate(handle: AdHandle) { /* no-op */ }
    }

    @Test
    fun runtimeRegistry_registersAndInitializesAdapter() {
        val ctx: Context = Application()
        val registry = AdapterRegistry()
        registry.registerRuntimeAdapterFactory("fake") { _ -> FakeAdapter() }
        registry.initialize(ctx)
        val names = registry.getRegisteredNetworks()
        assertTrue(names.contains("fake"))
    }

    @Test
    fun orderOfCallbacks_isExactlyOnce() {
        val cb = OrderTrackingCallbacks()
        val fake = FakeAdapter()
        // Interstitial
        fake.showInterstitial(AdHandle("x", AdType.INTERSTITIAL), Any(), cb)
        assertEquals(listOf("impression", "closed:COMPLETED"), cb.events)
        cb.events.clear()
        // Rewarded
        fake.showRewarded(AdHandle("y", AdType.REWARDED), Any(), cb)
        assertEquals(listOf("impression", "reward:coins:1", "closed:COMPLETED"), cb.events)
    }
}
