package com.rivalapexmediation.sdk

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.rivalapexmediation.sdk.contract.*
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ConformanceSmokeInstrumentedTest {

    class FakeAdapter : AdNetworkAdapterV2 {
        override fun name(): String = "fake"
        override fun init(config: AdapterConfig, timeoutMs: Int): InitResult = InitResult(true)
        override suspend fun loadInterstitial(placement: String, meta: RequestMeta, timeoutMs: Int): LoadResult {
            return LoadResult(success = true, handle = AdHandle("fake-$placement"), ecpm = 1.23)
        }
        override suspend fun loadRewarded(placement: String, meta: RequestMeta, timeoutMs: Int): LoadResult {
            return LoadResult(success = true, handle = AdHandle("fake-$placement"), ecpm = 1.23)
        }
        override fun invalidate(handle: AdHandle) { /* no-op */ }
        override fun showInterstitial(handle: AdHandle, viewContext: Any, callbacks: ShowCallbacks) {
            callbacks.onShown()
            callbacks.onClosed(CloseReason.FINISHED)
        }
        override fun showRewarded(handle: AdHandle, viewContext: Any, callbacks: RewardedCallbacks) {
            callbacks.onShown()
            callbacks.onUserEarnedReward()
            callbacks.onClosed(CloseReason.FINISHED)
        }
    }

    @Test
    fun registryRegistersRuntimeAdapterFactory() {
        val ctx: Context = InstrumentationRegistry.getInstrumentation().targetContext
        val registry = AdapterRegistry()
        registry.registerRuntimeAdapterFactory("fake") { _ -> FakeAdapter() }
        registry.initialize(ctx)

        val networks = registry.getRegisteredNetworks()
        assertTrue("Expected 'fake' to be registered", networks.contains("fake"))
    }
}
