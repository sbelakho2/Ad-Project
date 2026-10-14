package com.rivalapexmediation.sdk

import android.app.Application
import android.content.Context
import com.rivalapexmediation.sdk.contract.*
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * Pre-wired BYO adapter conformance for AdMob, AppLovin, IronSource.
 * NOTE: These are test-only fakes and never ship in production artifacts.
 */
@RunWith(RobolectricTestRunner::class)
class PrewiredAdapterConformanceTest {

    private open class BaseFakeAdapter(private val network: String) : AdNetworkAdapterV2 {
        override fun init(config: AdapterConfig, timeoutMs: Int): InitResult = InitResult(true)
        override fun loadInterstitial(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult =
            LoadResult(
                AdHandle(
                    "ih-$placementId",
                    com.rivalapexmediation.sdk.models.AdType.INTERSTITIAL,
                    partnerPlacementId = network
                ),
                ttlMs = 60000
            )
        override fun showInterstitial(handle: AdHandle, viewContext: Any, callbacks: ShowCallbacks) {
            callbacks.onImpression(emptyMap())
            callbacks.onClosed(CloseReason.COMPLETED)
        }
        override fun loadRewarded(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult =
            LoadResult(
                AdHandle(
                    "rh-$placementId",
                    com.rivalapexmediation.sdk.models.AdType.REWARDED,
                    partnerPlacementId = network
                ),
                ttlMs = 60000
            )
        override fun showRewarded(handle: AdHandle, viewContext: Any, callbacks: RewardedCallbacks) {
            callbacks.onImpression(emptyMap())
            callbacks.onRewardVerified("reward", 1.0)
            callbacks.onClosed(CloseReason.COMPLETED)
        }
        override fun loadBanner(placementId: String, size: AdSize, meta: RequestMeta, timeoutMs: Int): LoadResult =
            LoadResult(
                AdHandle(
                    "bn-$placementId",
                    com.rivalapexmediation.sdk.models.AdType.BANNER,
                    partnerPlacementId = network
                ),
                ttlMs = 60000
            )
        override fun attachBanner(handle: AdHandle, bannerHost: Any, callbacks: BannerCallbacks) {
            callbacks.onViewAttached(); callbacks.onViewDetached(CloseReason.DISMISSED)
        }
        override fun isAdReady(handle: AdHandle): Boolean = true
        override fun expiresAt(handle: AdHandle): Long = com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow() + 60000
        override fun invalidate(handle: AdHandle) {}
    }

    private class FakeAdMob : BaseFakeAdapter("admob")
    private class FakeAppLovin : BaseFakeAdapter("applovin")
    private class FakeIronSource : BaseFakeAdapter("ironsource")

    @Test
    fun registry_canRegisterThreeBYOAdapters() {
        val ctx: Context = Application()
        val registry = AdapterRegistry()
        registry.registerRuntimeAdapterFactory("admob") { _ -> FakeAdMob() }
        registry.registerRuntimeAdapterFactory("applovin") { _ -> FakeAppLovin() }
        registry.registerRuntimeAdapterFactory("ironsource") { _ -> FakeIronSource() }
        registry.initialize(ctx)
        val names = registry.getRegisteredNetworks()
        assertTrue(names.contains("admob"))
        assertTrue(names.contains("applovin"))
        assertTrue(names.contains("ironsource"))
    }
}
