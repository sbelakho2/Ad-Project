package com.rivalapex.androidtv.mediation

import kotlinx.coroutines.*
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.max
import kotlin.math.min
import kotlin.random.Random

/**
 * Ad source with priority and weight configuration for Android TV
 */
data class WeightedAdSource(
    val id: String,
    val priority: Int,
    val weight: Double = 1.0,
    val timeout: Long = 8000L, // Longer default for CTV
    val enabled: Boolean = true,
    val minBid: Double = 0.0,
    val adFormat: String? = null,
    val metadata: Map<String, String> = emptyMap()
) {
    init {
        require(priority >= 0) { "Priority must be non-negative" }
        require(weight > 0) { "Weight must be positive" }
        require(timeout > 0) { "Timeout must be positive" }
    }
}

/**
 * Result of loading an ad from a source
 */
sealed class AdLoadResult {
    data class Success(val ad: Any, val sourceId: String, val latencyMs: Long, val bid: Double = 0.0) : AdLoadResult()
    data class NoFill(val sourceId: String, val reason: String) : AdLoadResult()
    data class Error(val sourceId: String, val error: Throwable) : AdLoadResult()
    data class Timeout(val sourceId: String) : AdLoadResult()
    
    val isSuccess: Boolean get() = this is Success
}

/**
 * Statistics for a source
 */
data class SourcePerformance(
    val sourceId: String,
    val totalAttempts: Int,
    val successCount: Int,
    val noFillCount: Int,
    val errorCount: Int,
    val timeoutCount: Int,
    val averageLatencyMs: Double,
    val fillRate: Double,
    val averageBid: Double
)

/**
 * Configuration for priority-weighted mediation
 */
data class PriorityWeightedConfig(
    val usePerformanceWeighting: Boolean = true,
    val performanceWindowMs: Long = 3_600_000,
    val minSampleSize: Int = 10,
    val maxConcurrentRequests: Int = 2, // Lower for CTV
    val adaptiveTimeoutsEnabled: Boolean = true
)

/**
 * Ad loader function type
 */
typealias AdLoader = suspend (source: WeightedAdSource) -> AdLoadResult

/**
 * Priority-weighted mediation manager for Android TV
 */
class PriorityWeightedMediation(
    private val config: PriorityWeightedConfig = PriorityWeightedConfig()
) {
    private val sourceStats = ConcurrentHashMap<String, SourceStats>()
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    
    private class SourceStats {
        val attempts = AtomicInteger(0)
        val successes = AtomicInteger(0)
        val noFills = AtomicInteger(0)
        val errors = AtomicInteger(0)
        val timeouts = AtomicInteger(0)
        val totalLatencyMs = AtomicLong(0)
        val totalBidMicros = AtomicLong(0)
    }
    
    /**
     * Execute mediation with priority-weighted selection
     */
    suspend fun execute(
        sources: List<WeightedAdSource>,
        loader: AdLoader
    ): AdLoadResult {
        if (sources.isEmpty()) {
            return AdLoadResult.NoFill("none", "No sources configured")
        }
        
        val enabledSources = sources.filter { it.enabled }
        if (enabledSources.isEmpty()) {
            return AdLoadResult.NoFill("none", "All sources disabled")
        }
        
        val priorityGroups = enabledSources.groupBy { it.priority }.toSortedMap()
        
        for ((_, group) in priorityGroups) {
            val result = executePriorityGroup(group, loader)
            if (result.isSuccess) {
                return result
            }
        }
        
        return AdLoadResult.NoFill("all", "All sources exhausted")
    }
    
    /**
     * Get performance statistics for all sources
     */
    fun getPerformanceStats(): List<SourcePerformance> {
        return sourceStats.map { (sourceId, stats) ->
            val attempts = stats.attempts.get()
            val successes = stats.successes.get()
            
            SourcePerformance(
                sourceId = sourceId,
                totalAttempts = attempts,
                successCount = successes,
                noFillCount = stats.noFills.get(),
                errorCount = stats.errors.get(),
                timeoutCount = stats.timeouts.get(),
                averageLatencyMs = if (successes > 0) stats.totalLatencyMs.get().toDouble() / successes else 0.0,
                fillRate = if (attempts > 0) successes.toDouble() / attempts else 0.0,
                averageBid = if (successes > 0) stats.totalBidMicros.get().toDouble() / (successes * 1_000_000) else 0.0
            )
        }
    }
    
    /**
     * Reset all statistics
     */
    fun resetAllStats() {
        sourceStats.clear()
    }
    
    /**
     * Shutdown
     */
    fun shutdown() {
        scope.cancel()
    }
    
    private suspend fun executePriorityGroup(
        sources: List<WeightedAdSource>,
        loader: AdLoader
    ): AdLoadResult {
        val remainingSources = sources.toMutableList()
        
        while (remainingSources.isNotEmpty()) {
            val selected = selectByWeight(remainingSources)
            remainingSources.remove(selected)
            
            val result = executeWithTimeout(selected, loader)
            recordResult(selected.id, result)
            
            if (result.isSuccess) {
                return result
            }
        }
        
        return AdLoadResult.NoFill("priority_group", "No fill from priority group")
    }
    
    private fun selectByWeight(sources: List<WeightedAdSource>): WeightedAdSource {
        if (sources.size == 1) return sources[0]
        
        val effectiveWeights = sources.map { it to calculateEffectiveWeight(it) }
        val totalWeight = effectiveWeights.sumOf { it.second }
        
        if (totalWeight <= 0) return sources.random()
        
        var random = Random.nextDouble() * totalWeight
        for ((source, weight) in effectiveWeights) {
            random -= weight
            if (random <= 0) return source
        }
        
        return sources.last()
    }
    
    private fun calculateEffectiveWeight(source: WeightedAdSource): Double {
        if (!config.usePerformanceWeighting) return source.weight
        
        val stats = sourceStats[source.id] ?: return source.weight
        if (stats.attempts.get() < config.minSampleSize) return source.weight
        
        val fillRate = stats.successes.get().toDouble() / max(1.0, stats.attempts.get().toDouble())
        return max(0.1, source.weight * (0.5 + fillRate))
    }
    
    private suspend fun executeWithTimeout(
        source: WeightedAdSource,
        loader: AdLoader
    ): AdLoadResult {
        val timeout = if (config.adaptiveTimeoutsEnabled) {
            calculateAdaptiveTimeout(source)
        } else {
            source.timeout
        }
        
        return try {
            withTimeout(timeout) {
                loader(source)
            }
        } catch (e: TimeoutCancellationException) {
            AdLoadResult.Timeout(source.id)
        } catch (e: Exception) {
            AdLoadResult.Error(source.id, e)
        }
    }
    
    private fun calculateAdaptiveTimeout(source: WeightedAdSource): Long {
        val stats = sourceStats[source.id] ?: return source.timeout
        val successes = stats.successes.get()
        if (successes < config.minSampleSize) return source.timeout
        
        val avgLatency = stats.totalLatencyMs.get().toDouble() / successes
        return min(source.timeout, max(2000L, (avgLatency * 2).toLong()))
    }
    
    private fun recordResult(sourceId: String, result: AdLoadResult) {
        val stats = sourceStats.getOrPut(sourceId) { SourceStats() }
        stats.attempts.incrementAndGet()
        
        when (result) {
            is AdLoadResult.Success -> {
                stats.successes.incrementAndGet()
                stats.totalLatencyMs.addAndGet(result.latencyMs)
                stats.totalBidMicros.addAndGet((result.bid * 1_000_000).toLong())
            }
            is AdLoadResult.NoFill -> stats.noFills.incrementAndGet()
            is AdLoadResult.Error -> stats.errors.incrementAndGet()
            is AdLoadResult.Timeout -> stats.timeouts.incrementAndGet()
        }
    }
}
