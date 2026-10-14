package com.rivalapexmediation.sdk.runtime

import com.rivalapexmediation.sdk.util.FixedClock
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PlacementPacerTest {
    @Test
    fun throttlesAfterNoFill_and_recovers_after_interval() {
        val clock = FixedClock(0)
        val pacer = PlacementPacer(minIntervalMs = 1000, clock = clock)

        assertFalse(pacer.shouldThrottle("demo"))

        pacer.markNoFill("demo")
        assertTrue(pacer.shouldThrottle("demo"))
        assertTrue(pacer.remainingMs("demo") in 900..1000)

        clock.advance(999)
        assertTrue(pacer.shouldThrottle("demo"))

        clock.advance(2)
        assertFalse(pacer.shouldThrottle("demo"))

        pacer.markNoFill("demo")
        pacer.reset("demo")
        assertFalse(pacer.shouldThrottle("demo"))
    }
}
