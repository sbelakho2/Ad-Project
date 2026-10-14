package com.rivalapexmediation.ctv.mediation

import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

class FallbackWaterfallTest {

    private lateinit var waterfall: FallbackWaterfall<String>

    @Before
    fun setUp() {
        waterfall = FallbackWaterfall(defaultTimeout = 1000L)
    }

    @Test
    fun `returns success on first successful source`() = runBlocking {
        val sources = listOf(
            waterfall.createSource("source1", 1, 1000L) {
                FallbackWaterfall.WaterfallResult.Success("ad_data")
            }
        )

        val result = waterfall.execute(sources)

        assertTrue(result.result is FallbackWaterfall.WaterfallResult.Success)
        assertEquals("source1", result.sourceId)
        assertEquals(1, result.attemptsCount)
    }

    @Test
    fun `falls back to next source on no-fill`() = runBlocking {
        val sources = listOf(
            waterfall.createSource("source1", 1, 1000L) {
                FallbackWaterfall.WaterfallResult.NoFill("No inventory")
            },
            waterfall.createSource("source2", 2, 1000L) {
                FallbackWaterfall.WaterfallResult.Success("ad_from_source2")
            }
        )

        val result = waterfall.execute(sources)

        assertTrue(result.result is FallbackWaterfall.WaterfallResult.Success)
        assertEquals("source2", result.sourceId)
        assertEquals(2, result.attemptsCount)
    }

    @Test
    fun `returns no-fill when all sources exhausted`() = runBlocking {
        val sources = listOf(
            waterfall.createSource("source1", 1, 1000L) {
                FallbackWaterfall.WaterfallResult.NoFill("No inventory")
            },
            waterfall.createSource("source2", 2, 1000L) {
                FallbackWaterfall.WaterfallResult.NoFill("Budget exhausted")
            }
        )

        val result = waterfall.execute(sources)

        assertTrue(result.result is FallbackWaterfall.WaterfallResult.NoFill)
        assertEquals(2, result.attemptsCount)
    }

    @Test
    fun `handles timeout correctly`() = runBlocking {
        val sources = listOf(
            waterfall.createSource("slow_source", 1, 100L) {
                delay(500) // Longer than timeout
                FallbackWaterfall.WaterfallResult.Success("should_not_return")
            },
            waterfall.createSource("fast_source", 2, 1000L) {
                FallbackWaterfall.WaterfallResult.Success("from_fast")
            }
        )

        val result = waterfall.execute(sources)

        assertTrue(result.result is FallbackWaterfall.WaterfallResult.Success)
        assertEquals("fast_source", result.sourceId)
        assertEquals(2, result.attemptsCount)
        
        // First attempt should be timeout
        assertEquals(FallbackWaterfall.AttemptResultType.TIMEOUT, result.attemptDetails[0].result)
    }

    @Test
    fun `records attempt details correctly`() = runBlocking {
        val sources = listOf(
            waterfall.createSource("source1", 1, 1000L) {
                FallbackWaterfall.WaterfallResult.NoFill("test")
            },
            waterfall.createSource("source2", 2, 1000L) {
                FallbackWaterfall.WaterfallResult.Error(RuntimeException("test error"))
            },
            waterfall.createSource("source3", 3, 1000L) {
                FallbackWaterfall.WaterfallResult.Success("success!")
            }
        )

        val result = waterfall.execute(sources)

        assertEquals(3, result.attemptDetails.size)
        assertEquals(FallbackWaterfall.AttemptResultType.NO_FILL, result.attemptDetails[0].result)
        assertEquals(FallbackWaterfall.AttemptResultType.ERROR, result.attemptDetails[1].result)
        assertEquals(FallbackWaterfall.AttemptResultType.SUCCESS, result.attemptDetails[2].result)
    }

    @Test
    fun `executes sources in priority order`() = runBlocking {
        val executionOrder = mutableListOf<String>()
        
        val sources = listOf(
            waterfall.createSource("low", 3, 1000L) {
                executionOrder.add("low")
                FallbackWaterfall.WaterfallResult.NoFill("test")
            },
            waterfall.createSource("high", 1, 1000L) {
                executionOrder.add("high")
                FallbackWaterfall.WaterfallResult.NoFill("test")
            },
            waterfall.createSource("medium", 2, 1000L) {
                executionOrder.add("medium")
                FallbackWaterfall.WaterfallResult.NoFill("test")
            }
        )

        waterfall.execute(sources)

        assertEquals(listOf("high", "medium", "low"), executionOrder)
    }

    @Test
    fun `tracks source statistics`() = runBlocking {
        val sources = listOf(
            waterfall.createSource("source1", 1, 1000L) {
                FallbackWaterfall.WaterfallResult.Success("ad")
            }
        )

        waterfall.execute(sources)
        waterfall.execute(sources)

        val stats = waterfall.getStats("source1")
        assertNotNull(stats)
        assertEquals(2, stats!!.successCount.get())
        assertTrue(stats.successRate > 0.9f)
    }

    @Test
    fun `handles empty source list`() = runBlocking {
        val result = waterfall.execute(emptyList())

        assertTrue(result.result is FallbackWaterfall.WaterfallResult.NoFill)
        assertEquals("none", result.sourceId)
        assertEquals(0, result.attemptsCount)
    }

    @Test
    fun `clearStats removes all statistics`() = runBlocking {
        val sources = listOf(
            waterfall.createSource("source1", 1, 1000L) {
                FallbackWaterfall.WaterfallResult.Success("ad")
            }
        )

        waterfall.execute(sources)
        assertNotNull(waterfall.getStats("source1"))

        waterfall.clearStats()
        assertNull(waterfall.getStats("source1"))
    }

    @Test
    fun `getAllStats returns all source statistics`() = runBlocking {
        val sources = listOf(
            waterfall.createSource("source1", 1, 1000L) {
                FallbackWaterfall.WaterfallResult.NoFill("test")
            },
            waterfall.createSource("source2", 2, 1000L) {
                FallbackWaterfall.WaterfallResult.Success("ad")
            }
        )

        waterfall.execute(sources)

        val allStats = waterfall.getAllStats()
        assertEquals(2, allStats.size)
        assertTrue(allStats.containsKey("source1"))
        assertTrue(allStats.containsKey("source2"))
    }
}
