package com.rivalapexmediation.sdk.threading

import com.rivalapexmediation.sdk.util.FixedClock
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CircuitBreakerTest {
    @Test
    fun opensAfterFailureThreshold_andResetsAfterTimeout() {
        val clock = FixedClock(0)
        val cb = CircuitBreaker(failureThreshold = 3, resetTimeoutMs = 50, halfOpenMaxAttempts = 1, clock = clock)
        assertEquals("CLOSED", cb.getState())

        // Cause 3 failures to open the circuit
        repeat(3) {
            try { cb.execute(action = { throw RuntimeException("fail") }) } catch (_: Exception) {}
        }
        assertTrue(cb.isOpen())
        assertEquals("OPEN", cb.getState())

        // While open, calls should be short-circuited and return null
        val fast = cb.execute(action = { 42 })
        assertEquals(null, fast)

        // Wait for reset timeout to enter HALF_OPEN on next attempt
        clock.advance(60)
        // Next call should attempt and succeed, closing the circuit
        val result = cb.execute(action = { 7 })
        // In HALF_OPEN with 1 success needed, should now be closed
        assertEquals(7, result)
        assertFalse(cb.isOpen())
        assertEquals("CLOSED", cb.getState())
    }
}
