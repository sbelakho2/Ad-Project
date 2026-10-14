package com.rivalapexmediation.sdk.interstitial

import com.rivalapexmediation.sdk.AdError
import com.rivalapexmediation.sdk.models.Ad
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.CoroutineStart

/**
 * Interstitial lifecycle controller with strict state machine and double-callback guards.
 *
 * States: Idle -> Loading -> Loaded -> Showing -> Closed -> Idle
 * - Only one in-flight load is allowed at a time.
 * - Loaded ad has TTL/expiry via Ad.isExpired(). If expired on show, treated as not ready.
 * - onLoadSuccess/onLoadError are delivered at most once per request.
 * - Callbacks are dispatched on Main dispatcher to be UI-friendly in apps; tests can override.
 */
class InterstitialController(
    private val mainDispatcher: CoroutineDispatcher = Dispatchers.Main
) {
    enum class State { Idle, Loading, Loaded, Showing, Closed }

    private val stateRef = AtomicReference(State.Idle)
    private val currentAdRef = AtomicReference<Ad?>(null)
    private val inFlightCallbackFired = AtomicBoolean(false)
    private val showGuard = AtomicBoolean(false)

    @Volatile private var loadJob: Job? = null

    data class Callbacks(
        val onLoaded: (Ad) -> Unit,
        val onError: (AdError, String) -> Unit,
        val onShown: ((Ad) -> Unit)? = null,
        val onClosed: ((Ad) -> Unit)? = null
    )

    fun getState(): State = stateRef.get()

    fun isReady(): Boolean {
        val ad = currentAdRef.get() ?: return false
        return !ad.isExpired() && stateRef.get() == State.Loaded
    }

    /**
     * Request a load using the provided suspending loader.
     * If a load is already in progress, this is a no-op and returns false.
     */
    fun load(scope: CoroutineScope, loader: suspend () -> Ad, cb: Callbacks): Boolean {
        synchronized(this) {
            if (stateRef.get() == State.Loading) return false
            stateRef.set(State.Loading)
            inFlightCallbackFired.set(false)
            showGuard.set(false)
        }
        val job = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            try {
                val ad = loader.invoke()
                currentAdRef.set(ad)
                stateRef.set(State.Loaded)
                deliverOnMain { cb.onLoaded(ad) }
            } catch (t: Throwable) {
                currentAdRef.set(null)
                stateRef.set(State.Idle)
                deliverOnMain { cb.onError(mapToAdError(t), t.message ?: "load_failed") }
            }
        }
        loadJob = job
        return true
    }

    /**
     * Attempt to show the ad if ready. Returns true if a show is initiated.
     */
    fun showIfReady(cb: Callbacks): Boolean {
        val ad = currentAdRef.get()
        if (ad == null || ad.isExpired() || stateRef.get() != State.Loaded) return false
        if (!showGuard.compareAndSet(false, true)) return false
        stateRef.set(State.Showing)
        cb.onShown?.invoke(ad)
        // In a real implementation we would render and then move to Closed on callbacks
        stateRef.set(State.Closed)
        cb.onClosed?.invoke(ad)
        // After closed, reset to Idle and clear ad
        currentAdRef.set(null)
        stateRef.set(State.Idle)
        return true
    }

    fun cancelLoad() {
        loadJob?.cancel()
        stateRef.set(State.Idle)
        inFlightCallbackFired.set(true)
        showGuard.set(true)
        currentAdRef.set(null)
    }

    /** Save lightweight state for process-death / rotation handoff. */
    fun saveState(): SavedState {
        val ad = currentAdRef.get()
        return SavedState(state = stateRef.get(), ad = ad?.takeIf { !it.isExpired() })
    }

    /** Restore a previously saved state; drops expired ads and resets duplicate-callback guards. */
    fun restoreState(saved: SavedState?) {
        if (saved == null) return
        val ad = saved.ad?.takeIf { !it.isExpired() }
        currentAdRef.set(ad)
        stateRef.set(if (ad != null) saved.state else State.Idle)
        inFlightCallbackFired.set(ad == null)
        showGuard.set(false)
    }

    /** Rebind callbacks after rotation; replays onLoaded once if still in Loaded state. */
    suspend fun rebindCallbacks(cb: Callbacks) {
        val ad = currentAdRef.get()
        if (ad != null && stateRef.get() == State.Loaded) {
            deliverOnMain { cb.onLoaded(ad) }
        }
    }

    data class SavedState(val state: State, val ad: Ad?)

    private suspend fun deliverOnMain(block: suspend () -> Unit) {
        if (inFlightCallbackFired.compareAndSet(false, true)) {
            withContext(mainDispatcher) { block() }
        }
    }

    private fun mapToAdError(t: Throwable): AdError {
        val msg = t.message ?: ""
        return when {
            msg.contains("timeout", ignoreCase = true) -> AdError.TIMEOUT
            msg.startsWith("status_") -> AdError.INTERNAL_ERROR
            msg.contains("network", ignoreCase = true) -> AdError.NETWORK_ERROR
            else -> AdError.INTERNAL_ERROR
        }
    }
}
