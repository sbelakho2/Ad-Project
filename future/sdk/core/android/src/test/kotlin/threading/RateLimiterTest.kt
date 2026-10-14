package com.rivalapexmediation.sdk.threading

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RateLimiterTest {
    @Test
    fun enforcesMaxRequestsPerSecond() {
        val rl = RateLimiter(maxRequestsPerSecond = 5)
        // First 5 should pass
        repeat(5) {
            assertTrue("request $it should pass", rl.acquire())
        }
        // Next should be throttled
        assertFalse(rl.acquire())
        // Sleep to new second window
        Thread.sleep(1050)
        assertTrue(rl.acquire())
    }
}
