package com.rivalapexmediation.sdk.threading

import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.SystemClockClock
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference

/**
 * Circuit Breaker pattern implementation for adapter fault tolerance
 * 
 * States:
 * - CLOSED: Normal operation
 * - OPEN: Too many failures, stop making requests
 * - HALF_OPEN: Testing if service recovered
 * 
 * This prevents cascading failures and protects against misbehaving adapters
 */
class CircuitBreaker(
    private val failureThreshold: Int = 5,
    private val resetTimeoutMs: Long = 60000,
    private val halfOpenMaxAttempts: Int = 3,
    private val clock: Clock = SystemClockClock,
    private val onStateChange: (state: String) -> Unit = {}
) {
    private enum class State {
        CLOSED, OPEN, HALF_OPEN
    }
    
    private val state = AtomicReference(State.CLOSED)
    private val failureCount = AtomicInteger(0)
    private val successCount = AtomicInteger(0)
    private val lastFailureTime = AtomicLong(0)
    
    /**
     * Execute action with circuit breaker protection
     */
    fun <T> execute(
        action: () -> T,
        classifySuccess: (T) -> Boolean = { true },
        countException: (Exception) -> Boolean = { true }
    ): T? {
        when (state.get()) {
            State.OPEN -> {
                if (shouldAttemptReset()) {
                    transitionTo(State.HALF_OPEN)
                    successCount.set(0)
                } else {
                    return null
                }
            }
            State.HALF_OPEN -> {
                if (successCount.get() >= halfOpenMaxAttempts) {
                    transitionTo(State.OPEN)
                    return null
                }
            }
            State.CLOSED -> { /* normal */ }
        }

        return try {
            val result = action()
            if (classifySuccess(result)) {
                onSuccess()
            } else {
                onFailure()
            }
            result
        } catch (e: Exception) {
            if (countException(e)) {
                onFailure()
            }
            throw e
        }
    }
    
    /**
     * Record successful execution
     */
    private fun onSuccess() {
        when (state.get()) {
            State.HALF_OPEN -> {
                val count = successCount.incrementAndGet()
                if (count >= halfOpenMaxAttempts) {
                    // Enough successes, close the circuit
                    transitionTo(State.CLOSED)
                    failureCount.set(0)
                    successCount.set(0)
                }
            }
            State.CLOSED -> {
                // Reset failure count on success
                failureCount.set(0)
            }
            State.OPEN -> {
                // Should not happen
            }
        }
    }
    
    /**
     * Record failed execution
     */
    private fun onFailure() {
        lastFailureTime.set(clock.monotonicNow())
        
        when (state.get()) {
            State.HALF_OPEN -> {
                // Failed during recovery test, back to OPEN
                transitionTo(State.OPEN)
                successCount.set(0)
            }
            State.CLOSED -> {
                val count = failureCount.incrementAndGet()
                if (count >= failureThreshold) {
                    // Too many failures, open the circuit
                    transitionTo(State.OPEN)
                }
            }
            State.OPEN -> {
                // Already open
            }
        }
    }
    
    /**
     * Check if enough time has passed to attempt reset
     */
    private fun shouldAttemptReset(): Boolean {
        val lastFailure = lastFailureTime.get()
        return clock.monotonicNow() - lastFailure >= resetTimeoutMs
    }
    
    /**
     * Check if circuit breaker is open
     */
    fun isOpen(): Boolean {
        return state.get() == State.OPEN
    }
    
    /**
     * Get current state for monitoring
     */
    fun getState(): String = state.get().name
    
    /**
     * Get failure count for monitoring
     */
    fun getFailureCount(): Int = failureCount.get()
    
    /**
     * Manual reset (for testing/debugging)
     */
    fun reset() {
        transitionTo(State.CLOSED)
        failureCount.set(0)
        successCount.set(0)
        lastFailureTime.set(0)
    }

    private fun transitionTo(newState: State) {
        val previous = state.getAndSet(newState)
        if (previous != newState) {
            try { onStateChange(newState.name) } catch (_: Throwable) { /* guard telemetry */ }
        }
    }
}

/**
 * Timeout enforcement wrapper
 */
class TimeoutEnforcer(
    private val timeoutMs: Long
) {
    fun <T> execute(action: () -> T): T {
        val future = java.util.concurrent.CompletableFuture.supplyAsync(action)
        return future.get(timeoutMs, java.util.concurrent.TimeUnit.MILLISECONDS)
    }
}

/**
 * Rate limiter for API calls
 */
class RateLimiter(
    private val maxRequestsPerSecond: Int,
    private val clock: com.rivalapexmediation.sdk.util.Clock = com.rivalapexmediation.sdk.util.ClockProvider.clock
) {
    private val timestamps = mutableListOf<Long>()
    
    @Synchronized
    fun acquire(): Boolean {
        val now = clock.monotonicNow()
        
        // Remove timestamps older than 1 second
        timestamps.removeAll { it < now - 1000 }
        
        if (timestamps.size < maxRequestsPerSecond) {
            timestamps.add(now)
            return true
        }
        
        return false
    }
}
