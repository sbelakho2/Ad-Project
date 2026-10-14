package com.rivalapexmediation.sdk.runtime

import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.ClockProvider
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * Simple per-placement pacing helper used to cap retry cadence after no-fill responses.
 */
class PlacementPacer(
    private val minIntervalMs: Long,
    private val clock: Clock = ClockProvider.clock
) {
    private val nextAllowedByPlacement = ConcurrentHashMap<String, AtomicLong>()

    fun shouldThrottle(placement: String): Boolean {
        val now = clock.monotonicNow()
        val deadline = nextAllowedByPlacement[placement]?.get() ?: 0L
        return now < deadline
    }

    fun remainingMs(placement: String): Long {
        val now = clock.monotonicNow()
        val deadline = nextAllowedByPlacement[placement]?.get() ?: 0L
        return (deadline - now).coerceAtLeast(0L)
    }

    fun markNoFill(placement: String) {
        val delay = minIntervalMs.coerceAtLeast(0L)
        val until = clock.monotonicNow() + delay
        nextAllowedByPlacement.getOrPut(placement) { AtomicLong(0L) }.set(until)
    }

    fun reset(placement: String) {
        nextAllowedByPlacement[placement]?.set(0L)
    }
}
