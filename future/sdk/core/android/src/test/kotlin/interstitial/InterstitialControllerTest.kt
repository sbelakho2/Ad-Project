package com.rivalapexmediation.sdk.interstitial

import com.rivalapexmediation.sdk.models.Ad
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.models.Creative
import com.rivalapexmediation.sdk.AdError
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

@OptIn(ExperimentalCoroutinesApi::class)
class InterstitialControllerTest {

    private fun fakeAd(expireInMs: Long = 60_000): Ad {
        val ttl = if (expireInMs > 0) com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow() + expireInMs else null
        return Ad(
            id = "ad-1",
            placementId = "placementA",
            networkName = "test",
            adType = AdType.INTERSTITIAL,
            ecpm = 1.23,
            creative = Creative.Banner(0, 0, "<div/>") ,
            expiryTimeMs = ttl
        )
    }

    @Test
    fun load_success_transitions_to_loaded_and_fires_once() = runTest {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val ctrl = InterstitialController(mainDispatcher = dispatcher)
        val loadedCount = AtomicInteger(0)
        val errorCount = AtomicInteger(0)

        val callbacks = InterstitialController.Callbacks(
            onLoaded = { loadedCount.incrementAndGet() },
            onError = { _: AdError, _: String -> errorCount.incrementAndGet() }
        )

        val started = ctrl.load(this, loader = { fakeAd() }, cb = callbacks)
        assertTrue(started)

        // Allow coroutines to run
        testScheduler.advanceUntilIdle()

        assertEquals(1, loadedCount.get())
        assertEquals(0, errorCount.get())
        assertEquals(InterstitialController.State.Loaded, ctrl.getState())
        assertTrue(ctrl.isReady())
    }

    @Test
    fun load_error_fires_error_once_and_returns_to_idle() = runTest {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val ctrl = InterstitialController(mainDispatcher = dispatcher)
        val loadedCount = AtomicInteger(0)
        val errorCount = AtomicInteger(0)

        val callbacks = InterstitialController.Callbacks(
            onLoaded = { loadedCount.incrementAndGet() },
            onError = { _: AdError, _: String -> errorCount.incrementAndGet() }
        )

        val started = ctrl.load(this, loader = { throw RuntimeException("status_500") }, cb = callbacks)
        assertTrue(started)

        // Allow coroutines to run
        testScheduler.advanceUntilIdle()

        assertEquals(0, loadedCount.get())
        assertEquals(1, errorCount.get())
        assertEquals(InterstitialController.State.Idle, ctrl.getState())
        assertFalse(ctrl.isReady())
    }

    @Test
    fun showIfReady_moves_to_closed_then_idle_and_clears_ad() = runTest {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val ctrl = InterstitialController(mainDispatcher = dispatcher)
        var shownCount = 0
        var closedCount = 0

        val callbacks = InterstitialController.Callbacks(
            onLoaded = { /* no-op */ },
            onError = { _, _ -> },
            onShown = { shownCount++ },
            onClosed = { closedCount++ }
        )

        // Load succeeds
        ctrl.load(this, loader = { fakeAd() }, cb = callbacks)
        testScheduler.advanceUntilIdle()
        assertEquals(InterstitialController.State.Loaded, ctrl.getState())

        // Show
        val didShow = ctrl.showIfReady(callbacks)
        assertTrue(didShow)
        assertEquals(1, shownCount)
        assertEquals(1, closedCount)
        assertEquals(InterstitialController.State.Idle, ctrl.getState())
        assertFalse(ctrl.isReady())
    }

    @Test
    fun show_guard_prevents_double_callbacks() = runTest {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val ctrl = InterstitialController(mainDispatcher = dispatcher)
        val events = mutableListOf<String>()
        val callbacks = InterstitialController.Callbacks(
            onLoaded = { events.add("loaded") },
            onError = { _, _ -> events.add("error") },
            onShown = { events.add("shown") },
            onClosed = { events.add("closed") }
        )

        ctrl.load(this, loader = { fakeAd() }, cb = callbacks)
        testScheduler.advanceUntilIdle()

        assertTrue(ctrl.showIfReady(callbacks))
        assertFalse(ctrl.showIfReady(callbacks))
        assertEquals(listOf("loaded", "shown", "closed"), events)
    }

    @Test
    fun restore_and_rebind_replays_loaded_once_and_remains_ready() = runTest {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val ctrl = InterstitialController(mainDispatcher = dispatcher)
        val events = mutableListOf<String>()
        val callbacks = InterstitialController.Callbacks(
            onLoaded = { events.add("loaded") },
            onError = { _, _ -> events.add("error") },
            onShown = { events.add("shown") },
            onClosed = { events.add("closed") }
        )

        ctrl.load(this, loader = { fakeAd() }, cb = callbacks)
        testScheduler.advanceUntilIdle()
        val saved = ctrl.saveState()

        val restored = InterstitialController(mainDispatcher = dispatcher)
        restored.restoreState(saved)
        restored.rebindCallbacks(callbacks)
        restored.rebindCallbacks(callbacks) // should not duplicate

        assertEquals(listOf("loaded", "loaded"), events)
        assertTrue(restored.isReady())
        assertTrue(restored.showIfReady(callbacks))
    }
}
