package com.rivalapexmediation.sdk.runtime

import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HedgeManagerTest {
    @Test
    fun returnsNullWhenNoSamplesRecorded() {
        val manager = HedgeManager()
        assertNull(manager.getHedgeDelayMs("net"))
    }

    @Test
    fun computesApproximateP95Latency() {
        val manager = HedgeManager()
        (1..100).forEach { manager.recordLatency("net", it.toLong()) }

        val delay = manager.getHedgeDelayMs("net")
        assertTrue("expected delay near 95th percentile but was $delay", delay != null && delay >= 90 && delay <= 100)
    }
}
