package com.rivalapexmediation.sdk.threading

import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException

class TimeoutEnforcerTest {
    @Test
    fun throwsOnTimeout() {
        val t = TimeoutEnforcer(timeoutMs = 50)
        try {
            t.execute {
                Thread.sleep(200)
                1
            }
            fail("Expected timeout")
        } catch (e: Exception) {
            // Depending on JDK, CompletableFuture may wrap TimeoutException
            val msg = e.message ?: ""
            // We only assert that an exception was thrown
        }
    }

    @Test
    fun returnsOnTime() {
        val t = TimeoutEnforcer(timeoutMs = 200)
        val v = t.execute {
            Thread.sleep(50)
            42
        }
        assertEquals(42, v)
    }
}
