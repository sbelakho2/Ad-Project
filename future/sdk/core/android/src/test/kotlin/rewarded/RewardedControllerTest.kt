package com.rivalapexmediation.sdk.rewarded

import com.rivalapexmediation.sdk.models.Ad
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.models.Creative
import com.rivalapexmediation.sdk.AdError
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class RewardedControllerTest {

    private fun fakeAd(expireInMs: Long = 60_000): Ad {
        val ttl = if (expireInMs > 0) com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow() + expireInMs else null
        return Ad(
            id = "ad-r1",
            placementId = "placementR",
            networkName = "test",
            adType = AdType.REWARDED,
            ecpm = 2.34,
            creative = Creative.Banner(0, 0, "<div/>") ,
            expiryTimeMs = ttl
        )
    }

    @Test
    fun load_success_transitions_to_loaded_and_fires_once() = runTest {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val ctrl = RewardedController(mainDispatcher = dispatcher)
        var loaded = 0
        var errors = 0
        val callbacks = RewardedController.Callbacks(
            onLoaded = { loaded++ },
            onError = { _: AdError, _: String -> errors++ }
        )
        val started = ctrl.load(this, loader = { fakeAd() }, cb = callbacks)
        assertTrue(started)
        testScheduler.advanceUntilIdle()
        assertEquals(1, loaded)
        assertEquals(0, errors)
        assertEquals(RewardedController.State.Loaded, ctrl.getState())
        assertTrue(ctrl.isReady())
    }

    @Test
    fun load_error_fires_error_once_and_returns_to_idle() = runTest {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val ctrl = RewardedController(mainDispatcher = dispatcher)
        var loaded = 0
        var errors = 0
        val callbacks = RewardedController.Callbacks(
            onLoaded = { loaded++ },
            onError = { _: AdError, _: String -> errors++ }
        )
        val started = ctrl.load(this, loader = { throw RuntimeException("status_500") }, cb = callbacks)
        assertTrue(started)
        testScheduler.advanceUntilIdle()
        assertEquals(0, loaded)
        assertEquals(1, errors)
        assertEquals(RewardedController.State.Idle, ctrl.getState())
        assertFalse(ctrl.isReady())
    }

    @Test
    fun showIfReady_triggers_reward_and_closes_then_idle() = runTest {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val ctrl = RewardedController(mainDispatcher = dispatcher)
        var shown = 0
        var rewarded = 0
        var closed = 0
        val callbacks = RewardedController.Callbacks(
            onLoaded = { },
            onError = { _, _ -> },
            onShown = { shown++ },
            onReward = { rewarded++ },
            onClosed = { closed++ }
        )
        ctrl.load(this, loader = { fakeAd() }, cb = callbacks)
        testScheduler.advanceUntilIdle()
        assertEquals(RewardedController.State.Loaded, ctrl.getState())
        val didShow = ctrl.showIfReady(callbacks)
        assertTrue(didShow)
        assertEquals(1, shown)
        assertEquals(1, rewarded)
        assertEquals(1, closed)
        assertEquals(RewardedController.State.Idle, ctrl.getState())
        assertFalse(ctrl.isReady())
    }
}
