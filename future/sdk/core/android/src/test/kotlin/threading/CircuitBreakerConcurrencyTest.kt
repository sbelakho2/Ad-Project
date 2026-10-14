package com.rivalapexmediation.sdk.threading

import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import org.junit.Assert.assertTrue
import org.junit.Test

class CircuitBreakerConcurrencyTest {
    @Test
    fun fastFailsConcurrentCallsWhenOpen() {
        val breaker = CircuitBreaker(failureThreshold = 1, resetTimeoutMs = 10_000, halfOpenMaxAttempts = 1)
        try {
            breaker.execute(action = { throw IllegalStateException("boom") })
        } catch (_: Exception) {
            // expected – opens the circuit immediately
        }

        val executor = Executors.newFixedThreadPool(4)
        val fastFailures = Collections.synchronizedList(mutableListOf<Boolean>())
        val latch = CountDownLatch(4)

        repeat(4) {
            executor.execute {
                val result = breaker.execute(action = { 42 })
                fastFailures += (result == null)
                latch.countDown()
            }
        }

        latch.await()
        executor.shutdownNow()

        assertTrue(fastFailures.all { it })
    }
}
