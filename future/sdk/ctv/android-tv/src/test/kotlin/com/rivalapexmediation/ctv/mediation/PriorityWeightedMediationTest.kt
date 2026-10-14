package com.rivalapexmediation.ctv.mediation

import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

class PriorityWeightedMediationTest {

    private lateinit var mediation: PriorityWeightedMediation

    @Before
    fun setUp() {
        mediation = PriorityWeightedMediation()
    }

    @After
    fun tearDown() {
        mediation.resetAllStats()
    }

    @Test
    fun `returns noFill when no sources provided`() = runBlocking {
        val result = mediation.execute(emptyList()) { 
            AdLoadResult.Success(Any(), it.id, 100, 1.0) 
        }
        
        assertTrue(result is AdLoadResult.NoFill)
        assertEquals("none", (result as AdLoadResult.NoFill).sourceId)
    }

    @Test
    fun `returns noFill when all sources disabled`() = runBlocking {
        val sources = listOf(
            WeightedAdSource("source1", priority = 1, enabled = false),
            WeightedAdSource("source2", priority = 2, enabled = false)
        )
        
        val result = mediation.execute(sources) { 
            AdLoadResult.Success(Any(), it.id, 100, 1.0) 
        }
        
        assertTrue(result is AdLoadResult.NoFill)
    }

    @Test
    fun `executes sources in priority order`() = runBlocking {
        val executionOrder = mutableListOf<String>()
        val sources = listOf(
            WeightedAdSource("low", priority = 3),
            WeightedAdSource("high", priority = 1),
            WeightedAdSource("medium", priority = 2)
        )
        
        mediation.execute(sources) { source ->
            executionOrder.add(source.id)
            AdLoadResult.NoFill(source.id, "test")
        }
        
        assertEquals(listOf("high", "medium", "low"), executionOrder)
    }

    @Test
    fun `stops on first success`() = runBlocking {
        val executionOrder = mutableListOf<String>()
        val sources = listOf(
            WeightedAdSource("first", priority = 1),
            WeightedAdSource("second", priority = 2),
            WeightedAdSource("third", priority = 3)
        )
        
        val result = mediation.execute(sources) { source ->
            executionOrder.add(source.id)
            if (source.id == "second") {
                AdLoadResult.Success("ad", source.id, 100, 1.0)
            } else {
                AdLoadResult.NoFill(source.id, "no fill")
            }
        }
        
        assertTrue(result is AdLoadResult.Success)
        assertEquals(listOf("first", "second"), executionOrder)
    }

    @Test
    fun `handles timeout correctly`() = runBlocking {
        val sources = listOf(
            WeightedAdSource("slow", priority = 1, timeoutMs = 100)
        )
        
        val result = mediation.execute(sources) { source ->
            try {
                delay(500) // Longer than timeout
                AdLoadResult.Success("ad", source.id, 100, 1.0)
            } catch (e: kotlinx.coroutines.CancellationException) {
                // Re-throw to let withTimeout handle it properly
                throw e
            }
        }
        
        // When all sources timeout, we get NoFill (all sources exhausted)
        assertTrue("Expected NoFill (all exhausted) but got: $result", result is AdLoadResult.NoFill)
        
        // Verify timeout was recorded in stats
        val stats = mediation.getPerformanceStats()
        val sourceStats = stats.find { it.sourceId == "slow" }
        assertNotNull(sourceStats)
        assertEquals(1, sourceStats!!.timeoutCount)
    }

    @Test
    fun `records performance statistics`() = runBlocking {
        val sources = listOf(
            WeightedAdSource("source1", priority = 1)
        )
        
        // Execute with success
        mediation.execute(sources) { 
            AdLoadResult.Success("ad", it.id, 150, 2.5) 
        }
        
        val stats = mediation.getPerformanceStats()
        assertEquals(1, stats.size)
        
        val source1Stats = stats.find { it.sourceId == "source1" }
        assertNotNull(source1Stats)
        assertEquals(1, source1Stats!!.totalAttempts)
        assertEquals(1, source1Stats.successCount)
        assertEquals(150.0, source1Stats.averageLatencyMs, 0.01)
        assertEquals(1.0, source1Stats.fillRate, 0.01)
    }

    @Test
    fun `uses weighted selection within same priority`() = runBlocking {
        // Create sources with same priority but different weights
        val sources = listOf(
            WeightedAdSource("heavy", priority = 1, weight = 100.0),
            WeightedAdSource("light", priority = 1, weight = 0.001)
        )
        
        // Run multiple times and count selections
        val selections = mutableMapOf<String, Int>()
        repeat(50) {
            mediation.execute(sources) { source ->
                selections[source.id] = selections.getOrDefault(source.id, 0) + 1
                AdLoadResult.NoFill(source.id, "test")
            }
        }
        
        // Heavy weight source should be selected much more often
        val heavyCount = selections["heavy"] ?: 0
        assertTrue("Heavy source should be selected more often", heavyCount > 30)
    }

    @Test
    fun `handles errors from loader`() = runBlocking {
        val sources = listOf(
            WeightedAdSource("error", priority = 1)
        )
        
        val result = mediation.execute(sources) { 
            throw RuntimeException("Test error")
        }
        
        // When all sources error, we get NoFill (all sources exhausted)
        assertTrue("Expected NoFill (all exhausted) but got: $result", result is AdLoadResult.NoFill)
        
        // Verify error was recorded in stats
        val stats = mediation.getPerformanceStats()
        val sourceStats = stats.find { it.sourceId == "error" }
        assertNotNull(sourceStats)
        assertEquals(1, sourceStats!!.errorCount)
    }
}
