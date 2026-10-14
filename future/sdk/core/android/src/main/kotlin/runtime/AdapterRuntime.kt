package com.rivalapexmediation.sdk.runtime

import android.os.Handler
import android.os.Looper
import com.rivalapexmediation.sdk.contract.*
import com.rivalapexmediation.sdk.util.ClockProvider
import kotlinx.coroutines.*
import java.util.concurrent.*
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import kotlin.random.Random

/**
 * Enforces runtime rules per Adapters.md section 2:
 * - Threading & latency budgets
 * - Circuit breaker
 * - Retry with jitter
 * - Hedging
 */

// MARK: - Circuit Breaker

class CircuitBreaker(
    private val failureThreshold: Int = 3,
    private val timeWindowMs: Long = 30_000,
    private val recoveryTimeMs: Long = 15_000
) {
    private val failures = ConcurrentLinkedQueue<Long>()
    @Volatile private var state = State.CLOSED
    @Volatile private var openedAt: Long = 0
    
    enum class State { CLOSED, OPEN, HALF_OPEN }
    
    fun getState(): State {
        val now = ClockProvider.clock.monotonicNow()
        if (state == State.OPEN && now - openedAt >= recoveryTimeMs) {
            synchronized(this) {
                val refreshedNow = ClockProvider.clock.monotonicNow()
                if (state == State.OPEN && refreshedNow - openedAt >= recoveryTimeMs) {
                    state = State.HALF_OPEN
                }
            }
        }
        return state
    }
    
    fun recordSuccess() {
        synchronized(this) {
            if (state == State.HALF_OPEN) {
                state = State.CLOSED
                failures.clear()
            }
        }
    }
    
    fun recordFailure() {
        val now = ClockProvider.clock.monotonicNow()
        failures.add(now)
        
        // Remove old failures outside window
        while (failures.peek()?.let { now - it > timeWindowMs } == true) {
            failures.poll()
        }
        
        synchronized(this) {
            if (state == State.CLOSED && failures.size >= failureThreshold) {
                state = State.OPEN
                openedAt = now
            } else if (state == State.HALF_OPEN) {
                state = State.OPEN
                openedAt = now
            }
        }
    }
    
    fun isOpen(): Boolean = getState() == State.OPEN
}

// MARK: - Timeout Enforcement

class TimeoutEnforcer(private val scope: CoroutineScope) {
    
    suspend fun <T> withTimeout(timeoutMs: Int, block: suspend () -> T): Result<T> {
        return try {
            withTimeout(timeoutMs.toLong()) {
                Result.success(block())
            }
        } catch (e: TimeoutCancellationException) {
            Result.failure(AdapterError.Recoverable(ErrorCode.TIMEOUT, "Operation exceeded ${timeoutMs}ms"))
        } catch (e: Exception) {
            Result.failure(e)
        }
    }
}

// MARK: - Retry Logic

object RetryPolicy {
    fun shouldRetry(error: AdapterError, attemptCount: Int): Boolean {
        if (attemptCount >= 2) return false // Max 1 retry = 2 total attempts
        
        return when (error.code) {
            ErrorCode.NETWORK_ERROR, ErrorCode.TIMEOUT -> true
            ErrorCode.ERROR -> error.detail.contains("5xx", ignoreCase = true) || 
                              error.detail.contains("transient", ignoreCase = true)
            else -> false
        }
    }
    
    fun jitterMs(): Long = Random.nextLong(10, 101) // 10-100ms jitter
}

// MARK: - Hedging

class HedgeManager {
    private val p95Latencies = ConcurrentHashMap<String, MovingPercentile>()
    
    fun recordLatency(key: String, latencyMs: Long) {
        p95Latencies.computeIfAbsent(key) { MovingPercentile(windowSize = 100) }
            .add(latencyMs)
    }
    
    fun getHedgeDelayMs(key: String): Long? {
        val percentile = p95Latencies[key] ?: return null
        return percentile.getPercentile(0.95)?.toLong()
    }
    
    class MovingPercentile(private val windowSize: Int) {
        private val values = ConcurrentLinkedQueue<Long>()
        
        fun add(value: Long) {
            values.add(value)
            while (values.size > windowSize) {
                values.poll()
            }
        }
        
        fun getPercentile(p: Double): Double? {
            if (values.isEmpty()) return null
            val sorted = values.sorted()
            val index = (sorted.size * p).toInt().coerceIn(0, sorted.size - 1)
            return sorted[index].toDouble()
        }
    }
}

// MARK: - Adapter Runtime Wrapper

class AdapterRuntimeWrapper(
    private val adapter: AdNetworkAdapterV2,
    private val partnerId: String,
    private val scope: CoroutineScope = CoroutineScope(Dispatchers.Default + SupervisorJob())
) {
    private val circuitBreakers = ConcurrentHashMap<String, CircuitBreaker>()
    private val hedgeManager = HedgeManager()
    private val timeoutEnforcer = TimeoutEnforcer(scope)
    private val mainHandler = Handler(Looper.getMainLooper())
    
    private fun getCircuitBreaker(placement: String): CircuitBreaker {
        return circuitBreakers.computeIfAbsent("$partnerId:$placement") { CircuitBreaker() }
    }
    
    /**
     * Load with full runtime enforcement: timeout, retry, circuit breaker, hedging
     */
    suspend fun loadInterstitialWithEnforcement(
        placement: String,
        meta: RequestMeta,
        timeoutMs: Int
    ): LoadResult {
        val cb = getCircuitBreaker(placement)
        
        // Fast-fail if circuit is open
        if (cb.isOpen()) {
            throw AdapterError.Fatal(ErrorCode.CIRCUIT_OPEN, "Circuit breaker open for $partnerId:$placement")
        }
        
        val startTime = ClockProvider.clock.monotonicNow()
        var attemptCount = 0
        var lastError: AdapterError? = null
        
        // Primary attempt with retry
        while (attemptCount < 2) {
            attemptCount++
            
            try {
                val result = timeoutEnforcer.withTimeout(timeoutMs) {
                    // Ensure no blocking on main thread
                    withContext(Dispatchers.IO) {
                        adapter.loadInterstitial(placement, meta, timeoutMs)
                    }
                }
                
                result.onSuccess { loadResult ->
                    val latency = ClockProvider.clock.monotonicNow() - startTime
                    hedgeManager.recordLatency("$partnerId:$placement", latency)
                    cb.recordSuccess()
                    return loadResult
                }
                
                result.onFailure { throwable ->
                    val error = when (throwable) {
                        is AdapterError -> throwable
                        else -> AdapterError.Recoverable(ErrorCode.ERROR, throwable.message ?: "Unknown error")
                    }
                    lastError = error
                    
                    if (!RetryPolicy.shouldRetry(error, attemptCount)) {
                        throw error
                    }
                    
                    delay(RetryPolicy.jitterMs())
                }
            } catch (e: AdapterError) {
                lastError = e
                if (!RetryPolicy.shouldRetry(e, attemptCount)) {
                    cb.recordFailure()
                    throw e
                }
                delay(RetryPolicy.jitterMs())
            }
        }
        
        cb.recordFailure()
        throw lastError ?: AdapterError.Fatal(ErrorCode.ERROR, "Unknown failure after retries")
    }
    
    /**
     * Load with hedging: start second request at p95 latency
     */
    suspend fun loadWithHedging(
        placement: String,
        meta: RequestMeta,
        timeoutMs: Int
    ): LoadResult = coroutineScope {
        val hedgeDelay = hedgeManager.getHedgeDelayMs("$partnerId:$placement")
        
        if (hedgeDelay == null || hedgeDelay >= timeoutMs) {
            // No hedging data or would exceed timeout
            return@coroutineScope loadInterstitialWithEnforcement(placement, meta, timeoutMs)
        }
        
        // Start primary
        val primary = async { loadInterstitialWithEnforcement(placement, meta, timeoutMs) }
        
        // Start hedge after delay
        val hedge = async {
            delay(hedgeDelay)
            loadInterstitialWithEnforcement(placement, meta, timeoutMs - hedgeDelay.toInt())
        }
        
        // Return first success (race)
        try {
            withTimeout(timeoutMs.toLong()) {
                val primaryResult = primary.await()
                hedge.cancel()
                primaryResult
            }
        } catch (e: TimeoutCancellationException) {
            hedge.await()
        }
    }
    
    /**
     * Show on main thread with guards
     */
    fun showInterstitialOnMain(handle: AdHandle, viewContext: Any, callbacks: ShowCallbacks) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            adapter.showInterstitial(handle, viewContext, callbacks)
        } else {
            mainHandler.post {
                adapter.showInterstitial(handle, viewContext, callbacks)
            }
        }
    }

    fun showRewardedOnMain(handle: AdHandle, viewContext: Any, callbacks: RewardedCallbacks) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            adapter.showRewarded(handle, viewContext, callbacks)
        } else {
            mainHandler.post {
                adapter.showRewarded(handle, viewContext, callbacks)
            }
        }
    }
}

// MARK: - Thread Guard (Debug)

object ThreadGuard {
    fun assertMainThread(operation: String) {
        if (BuildConfig.DEBUG && Looper.myLooper() != Looper.getMainLooper()) {
            throw IllegalStateException("$operation must be called from main thread")
        }
    }
    
    fun assertNotMainThread(operation: String) {
        if (BuildConfig.DEBUG && Looper.myLooper() == Looper.getMainLooper()) {
            throw IllegalStateException("$operation must not be called from main thread")
        }
    }
}

// Stub for BuildConfig
private object BuildConfig {
    const val DEBUG = true
}
