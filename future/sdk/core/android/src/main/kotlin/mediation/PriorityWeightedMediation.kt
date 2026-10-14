package com.rivalapex.mediation

import kotlinx.coroutines.*
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.max
import kotlin.math.min
import kotlin.random.Random

/**
 * Ad source with priority and weight configuration
 */
data class WeightedAdSource(
    val id: String,
    val priority: Int,
    val weight: Double = 1.0,
    val timeout: Long = 5000L,
    val enabled: Boolean = true,
    val minBid: Double = 0.0,
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
    val averageBid: Double,
    val effectiveWeight: Double
)

/**
 * Configuration for priority-weighted mediation
 */
data class PriorityWeightedConfig(
    val usePerformanceWeighting: Boolean = true,
    val performanceWindowMs: Long = 3_600_000, // 1 hour
    val minSampleSize: Int = 10,
    val weightDecayFactor: Double = 0.1,
    val maxConcurrentRequests: Int = 3,
    val bidFloorEnabled: Boolean = true,
    val adaptiveTimeoutsEnabled: Boolean = true
)

/**
 * Ad loader function type
 */
typealias AdLoader = suspend (source: WeightedAdSource) -> AdLoadResult

/**
 * Priority-weighted mediation manager
 *
 * Implements intelligent ad source selection based on:
 * - Configured priority (lower = higher priority)
 * - Weighted random selection within same priority tier
 * - Performance-based weight adjustments
 * - Fill rate optimization
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
        val totalBid = AtomicLong(0) // Stored as micros
        var lastSuccessTime = 0L
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
        
        // Group by priority
        val priorityGroups = enabledSources
            .groupBy { it.priority }
            .toSortedMap()
        
        // Try each priority group in order
        for ((_, group) in priorityGroups) {
            val result = executePriorityGroup(group, loader)
            if (result is AdLoadResult.Success) {
                return result
            }
        }
        
        return AdLoadResult.NoFill("all", "All sources exhausted")
    }
    
    /**
     * Execute mediation within a single priority group using weighted selection
     */
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
            
            if (result is AdLoadResult.Success) {
                return result
            }
        }
        
        return AdLoadResult.NoFill("priority_group", "No fill from priority group")
    }
    
    /**
     * Select a source using weighted random selection
     */
    private fun selectByWeight(sources: List<WeightedAdSource>): WeightedAdSource {
        if (sources.size == 1) {
            return sources[0]
        }
        
        val effectiveWeights = sources.map { source ->
            source to calculateEffectiveWeight(source)
        }
        
        val totalWeight = effectiveWeights.sumOf { it.second }
        if (totalWeight <= 0) {
            return sources.random()
        }
        
        var random = Random.nextDouble() * totalWeight
        for ((source, weight) in effectiveWeights) {
            random -= weight
            if (random <= 0) {
                return source
            }
        }
        
        return sources.last()
    }
    
    /**
     * Calculate effective weight based on configuration and performance
     */
    private fun calculateEffectiveWeight(source: WeightedAdSource): Double {
        val baseWeight = source.weight
        
        if (!config.usePerformanceWeighting) {
            return baseWeight
        }
        
        val stats = sourceStats[source.id] ?: return baseWeight
        
        // Need minimum samples
        if (stats.attempts.get() < config.minSampleSize) {
            return baseWeight
        }
        
        // Calculate fill rate
        val attempts = stats.attempts.get().toDouble()
        val successes = stats.successes.get().toDouble()
        val fillRate = successes / max(1.0, attempts)
        
        // Weight adjustment: boost high performers, penalize low performers
        // Using a sigmoid-like function centered around 50% fill rate
        val performanceMultiplier = 0.5 + fillRate
        
        return max(0.1, baseWeight * performanceMultiplier)
    }
    
    /**
     * Execute ad loading with timeout
     */
    private suspend fun executeWithTimeout(
        source: WeightedAdSource,
        loader: AdLoader
    ): AdLoadResult {
        val timeout = if (config.adaptiveTimeoutsEnabled) {
            calculateAdaptiveTimeout(source)
        } else {
            source.timeout
        }
        
        val startTime = System.currentTimeMillis()
        
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
    
    /**
     * Calculate adaptive timeout based on historical latency
     */
    private fun calculateAdaptiveTimeout(source: WeightedAdSource): Long {
        val stats = sourceStats[source.id] ?: return source.timeout
        
        val attempts = stats.attempts.get()
        if (attempts < config.minSampleSize) {
            return source.timeout
        }
        
        val avgLatency = stats.totalLatencyMs.get().toDouble() / attempts
        
        // Timeout = 2x average latency, bounded by original timeout
        return min(source.timeout, (avgLatency * 2).toLong().coerceAtLeast(1000L))
    }
    
    /**
     * Record result for statistics
     */
    private fun recordResult(sourceId: String, result: AdLoadResult) {
        val stats = sourceStats.getOrPut(sourceId) { SourceStats() }
        stats.attempts.incrementAndGet()
        
        when (result) {
            is AdLoadResult.Success -> {
                stats.successes.incrementAndGet()
                stats.totalLatencyMs.addAndGet(result.latencyMs)
                stats.totalBid.addAndGet((result.bid * 1_000_000).toLong())
                stats.lastSuccessTime = System.currentTimeMillis()
            }
            is AdLoadResult.NoFill -> {
                stats.noFills.incrementAndGet()
            }
            is AdLoadResult.Error -> {
                stats.errors.incrementAndGet()
            }
            is AdLoadResult.Timeout -> {
                stats.timeouts.incrementAndGet()
            }
        }
    }
    
    /**
     * Get performance statistics for all sources
     */
    fun getPerformanceStats(): List<SourcePerformance> {
        return sourceStats.map { (sourceId, stats) ->
            val attempts = stats.attempts.get()
            val successes = stats.successes.get()
            val noFills = stats.noFills.get()
            val errors = stats.errors.get()
            val timeouts = stats.timeouts.get()
            
            val avgLatency = if (successes > 0) {
                stats.totalLatencyMs.get().toDouble() / successes
            } else 0.0
            
            val fillRate = if (attempts > 0) {
                successes.toDouble() / attempts
            } else 0.0
            
            val avgBid = if (successes > 0) {
                stats.totalBid.get().toDouble() / (successes * 1_000_000)
            } else 0.0
            
            SourcePerformance(
                sourceId = sourceId,
                totalAttempts = attempts,
                successCount = successes,
                noFillCount = noFills,
                errorCount = errors,
                timeoutCount = timeouts,
                averageLatencyMs = avgLatency,
                fillRate = fillRate,
                averageBid = avgBid,
                effectiveWeight = 1.0 // Would need source reference
            )
        }
    }
    
    /**
     * Get performance for a specific source
     */
    fun getSourcePerformance(sourceId: String): SourcePerformance? {
        val stats = sourceStats[sourceId] ?: return null
        val attempts = stats.attempts.get()
        val successes = stats.successes.get()
        
        return SourcePerformance(
            sourceId = sourceId,
            totalAttempts = attempts,
            successCount = successes,
            noFillCount = stats.noFills.get(),
            errorCount = stats.errors.get(),
            timeoutCount = stats.timeouts.get(),
            averageLatencyMs = if (successes > 0) stats.totalLatencyMs.get().toDouble() / successes else 0.0,
            fillRate = if (attempts > 0) successes.toDouble() / attempts else 0.0,
            averageBid = if (successes > 0) stats.totalBid.get().toDouble() / (successes * 1_000_000) else 0.0,
            effectiveWeight = 1.0
        )
    }
    
    /**
     * Reset statistics for a source
     */
    fun resetStats(sourceId: String) {
        sourceStats.remove(sourceId)
    }
    
    /**
     * Reset all statistics
     */
    fun resetAllStats() {
        sourceStats.clear()
    }
    
    /**
     * Cleanup resources
     */
    fun shutdown() {
        scope.cancel()
    }
}

/**
 * Builder for creating weighted ad source configurations
 */
class WeightedAdSourceBuilder {
    private var id: String = ""
    private var priority: Int = 0
    private var weight: Double = 1.0
    private var timeout: Long = 5000L
    private var enabled: Boolean = true
    private var minBid: Double = 0.0
    private var metadata: MutableMap<String, String> = mutableMapOf()
    
    fun id(id: String) = apply { this.id = id }
    fun priority(priority: Int) = apply { this.priority = priority }
    fun weight(weight: Double) = apply { this.weight = weight }
    fun timeout(timeout: Long) = apply { this.timeout = timeout }
    fun enabled(enabled: Boolean) = apply { this.enabled = enabled }
    fun minBid(minBid: Double) = apply { this.minBid = minBid }
    fun metadata(key: String, value: String) = apply { this.metadata[key] = value }
    
    fun build(): WeightedAdSource {
        require(id.isNotEmpty()) { "Source ID is required" }
        return WeightedAdSource(
            id = id,
            priority = priority,
            weight = weight,
            timeout = timeout,
            enabled = enabled,
            minBid = minBid,
            metadata = metadata.toMap()
        )
    }
}

/**
 * Extension function for fluent source creation
 */
fun weightedAdSource(block: WeightedAdSourceBuilder.() -> Unit): WeightedAdSource {
    return WeightedAdSourceBuilder().apply(block).build()
}
