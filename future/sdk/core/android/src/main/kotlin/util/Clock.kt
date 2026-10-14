package com.rivalapexmediation.sdk.util

/**
 * Clock abstraction to guard against wall-clock drift. Prefer [monotonicNow] for durations/TTL.
 */
interface Clock {
    fun now(): Long
    fun monotonicNow(): Long
}

/**
 * Global clock access so components can be drift-tolerant and easily faked in tests.
 */
object ClockProvider {
    @Volatile
    var clock: Clock = SystemClockClock
}

object SystemClockClock : Clock {
    override fun now(): Long = System.currentTimeMillis()
    // Use JVM monotonic time to avoid reliance on android.os.SystemClock in unit tests
    override fun monotonicNow(): Long = System.nanoTime() / 1_000_000L
}

class FixedClock(private var currentMillis: Long) : Clock {
    override fun now(): Long = currentMillis
    override fun monotonicNow(): Long = currentMillis
    fun advance(millis: Long) { currentMillis += millis }
}
