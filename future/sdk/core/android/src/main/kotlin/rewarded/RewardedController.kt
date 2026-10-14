package com.rivalapexmediation.sdk.rewarded

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
 * Rewarded lifecycle controller mirroring InterstitialController with a reward callback.
 * States: Idle -> Loading -> Loaded -> Showing -> Closed -> Idle
 */
class RewardedController(
    private val mainDispatcher: CoroutineDispatcher = Dispatchers.Main
) {
    enum class State { Idle, Loading, Loaded, Showing, Closed }

    private val stateRef = AtomicReference(State.Idle)
    private val currentAdRef = AtomicReference<Ad?>(null)
    private val inFlightCallbackFired = AtomicBoolean(false)

    @Volatile private var loadJob: Job? = null

    data class Callbacks(
        val onLoaded: (Ad) -> Unit,
        val onError: (AdError, String) -> Unit,
        val onShown: ((Ad) -> Unit)? = null,
        val onReward: ((Ad) -> Unit)? = null,
        val onClosed: ((Ad) -> Unit)? = null
    )

    fun getState(): State = stateRef.get()

    fun isReady(): Boolean {
        val ad = currentAdRef.get() ?: return false
        return !ad.isExpired() && stateRef.get() == State.Loaded
    }

    fun load(scope: CoroutineScope, loader: suspend () -> Ad, cb: Callbacks): Boolean {
        synchronized(this) {
            if (stateRef.get() == State.Loading) return false
            stateRef.set(State.Loading)
            inFlightCallbackFired.set(false)
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

    fun showIfReady(cb: Callbacks): Boolean {
        val ad = currentAdRef.get()
        if (ad == null || ad.isExpired() || stateRef.get() != State.Loaded) return false
        stateRef.set(State.Showing)
        cb.onShown?.invoke(ad)
        // In a real implementation, reward would be triggered by playback completion callback.
        cb.onReward?.invoke(ad)
        stateRef.set(State.Closed)
        cb.onClosed?.invoke(ad)
        currentAdRef.set(null)
        stateRef.set(State.Idle)
        return true
    }

    fun cancelLoad() {
        loadJob?.cancel()
        stateRef.set(State.Idle)
        inFlightCallbackFired.set(true)
    }

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
