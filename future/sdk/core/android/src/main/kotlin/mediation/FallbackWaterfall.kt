package com.rivalapex.sdk.mediation

import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.ConcurrentHashMap

/**
 * FallbackWaterfall - Implements a waterfall mediation strategy for ad loading.
 *
 * When one ad source fails, it automatically falls back to the next source
 * in the waterfall. This ensures maximum fill rate by trying multiple
 * demand partners in priority order.
 *
 * Features:
 * - Priority-based waterfall ordering
 * - Configurable timeout per source
 * - Automatic failover on error, timeout, or no-fill
 * - Performance tracking per source
 * - Concurrent loading with ordered response handling
 */
class FallbackWaterfall<T>(
    private val defaultTimeout: Long = 5000L
) {
    
    /**
     * Represents an ad source in the waterfall
     */
    data class WaterfallSource<T>(
        val id: String,
        val priority: Int,
        val timeoutMs: Long,
        val loader: suspend () -> WaterfallResult<T>
    )
    
    /**
     * Result of attempting to load from a source
     */
    sealed class WaterfallResult<out T> {
        data class Success<T>(val data: T) : WaterfallResult<T>()
        data class NoFill(val reason: String = "No ad available") : WaterfallResult<Nothing>()
        data class Error(val error: Throwable) : WaterfallResult<Nothing>()
        object Timeout : WaterfallResult<Nothing>()
    }
    
    /**
     * Waterfall execution result with metadata
     */
    data class ExecutionResult<T>(
        val result: WaterfallResult<T>,
        val sourceId: String,
        val attemptsCount: Int,
        val totalDurationMs: Long,
        val attemptDetails: List<AttemptDetail>
    )
    
    /**
     * Details about each attempt in the waterfall
     */
    data class AttemptDetail(
        val sourceId: String,
        val priority: Int,
        val durationMs: Long,
        val result: AttemptResultType
    )
    
    enum class AttemptResultType {
        SUCCESS, NO_FILL, ERROR, TIMEOUT, SKIPPED
    }
    
    /**
     * Performance statistics for a source
     */
    data class SourceStats(
        val successCount: AtomicInteger = AtomicInteger(0),
        val failureCount: AtomicInteger = AtomicInteger(0),
        val timeoutCount: AtomicInteger = AtomicInteger(0),
        val noFillCount: AtomicInteger = AtomicInteger(0),
        val totalLatencyMs: AtomicInteger = AtomicInteger(0)
    ) {
        val totalAttempts: Int
            get() = successCount.get() + failureCount.get() + timeoutCount.get() + noFillCount.get()
        
        val successRate: Float
            get() = if (totalAttempts > 0) successCount.get().toFloat() / totalAttempts else 0f
        
        val averageLatencyMs: Long
            get() = if (totalAttempts > 0) totalLatencyMs.get().toLong() / totalAttempts else 0
    }
    
    private val sourceStats = ConcurrentHashMap<String, SourceStats>()
    
    /**
     * Executes the waterfall, trying each source in priority order until
     * one succeeds or all sources are exhausted.
     *
     * @param sources List of sources to try, ordered by priority
     * @return ExecutionResult containing the final result and metadata
     */
    suspend fun execute(sources: List<WaterfallSource<T>>): ExecutionResult<T> {
        val startTime = System.currentTimeMillis()
        val attemptDetails = mutableListOf<AttemptDetail>()
        val sortedSources = sources.sortedBy { it.priority }
        
        for (source in sortedSources) {
            val attemptStart = System.currentTimeMillis()
            
            val result = try {
                withTimeout(source.timeoutMs) {
                    source.loader()
                }
            } catch (e: TimeoutCancellationException) {
                WaterfallResult.Timeout
            } catch (e: CancellationException) {
                throw e // Rethrow cancellation
            } catch (e: Exception) {
                WaterfallResult.Error(e)
            }
            
            val attemptDuration = System.currentTimeMillis() - attemptStart
            recordStats(source.id, result, attemptDuration)
            
            val attemptResultType = when (result) {
                is WaterfallResult.Success -> AttemptResultType.SUCCESS
                is WaterfallResult.NoFill -> AttemptResultType.NO_FILL
                is WaterfallResult.Error -> AttemptResultType.ERROR
                is WaterfallResult.Timeout -> AttemptResultType.TIMEOUT
            }
            
            attemptDetails.add(
                AttemptDetail(
                    sourceId = source.id,
                    priority = source.priority,
                    durationMs = attemptDuration,
                    result = attemptResultType
                )
            )
            
            if (result is WaterfallResult.Success) {
                return ExecutionResult(
                    result = result,
                    sourceId = source.id,
                    attemptsCount = attemptDetails.size,
                    totalDurationMs = System.currentTimeMillis() - startTime,
                    attemptDetails = attemptDetails
                )
            }
        }
        
        // All sources exhausted
        val totalDuration = System.currentTimeMillis() - startTime
        return ExecutionResult(
            result = WaterfallResult.NoFill("All ${sortedSources.size} sources exhausted"),
            sourceId = sortedSources.lastOrNull()?.id ?: "none",
            attemptsCount = attemptDetails.size,
            totalDurationMs = totalDuration,
            attemptDetails = attemptDetails
        )
    }
    
    /**
     * Executes waterfall with optional parallel preloading of lower-priority sources.
     * This can reduce latency when a higher-priority source returns no-fill quickly.
     */
    suspend fun executeWithParallelPreload(
        sources: List<WaterfallSource<T>>,
        preloadCount: Int = 1
    ): ExecutionResult<T> = coroutineScope {
        val startTime = System.currentTimeMillis()
        val attemptDetails = mutableListOf<AttemptDetail>()
        val sortedSources = sources.sortedBy { it.priority }
        
        for ((index, source) in sortedSources.withIndex()) {
            // Start preloading next sources in background
            val preloadJobs = sortedSources
                .drop(index + 1)
                .take(preloadCount)
                .map { preloadSource ->
                    async(start = CoroutineStart.LAZY) {
                        try {
                            withTimeout(preloadSource.timeoutMs) {
                                preloadSource.loader()
                            }
                        } catch (e: Exception) {
                            WaterfallResult.Error(e)
                        }
                    }
                }
            
            // Execute current source
            val attemptStart = System.currentTimeMillis()
            
            val result = try {
                withTimeout(source.timeoutMs) {
                    source.loader()
                }
            } catch (e: TimeoutCancellationException) {
                WaterfallResult.Timeout
            } catch (e: CancellationException) {
                // Cancel preload jobs on cancellation
                preloadJobs.forEach { it.cancel() }
                throw e
            } catch (e: Exception) {
                WaterfallResult.Error(e)
            }
            
            val attemptDuration = System.currentTimeMillis() - attemptStart
            recordStats(source.id, result, attemptDuration)
            
            val attemptResultType = when (result) {
                is WaterfallResult.Success -> AttemptResultType.SUCCESS
                is WaterfallResult.NoFill -> AttemptResultType.NO_FILL
                is WaterfallResult.Error -> AttemptResultType.ERROR
                is WaterfallResult.Timeout -> AttemptResultType.TIMEOUT
            }
            
            attemptDetails.add(
                AttemptDetail(
                    sourceId = source.id,
                    priority = source.priority,
                    durationMs = attemptDuration,
                    result = attemptResultType
                )
            )
            
            if (result is WaterfallResult.Success) {
                preloadJobs.forEach { it.cancel() }
                return@coroutineScope ExecutionResult(
                    result = result,
                    sourceId = source.id,
                    attemptsCount = attemptDetails.size,
                    totalDurationMs = System.currentTimeMillis() - startTime,
                    attemptDetails = attemptDetails
                )
            }
        }
        
        // All sources exhausted
        val totalDuration = System.currentTimeMillis() - startTime
        ExecutionResult(
            result = WaterfallResult.NoFill("All ${sortedSources.size} sources exhausted"),
            sourceId = sortedSources.lastOrNull()?.id ?: "none",
            attemptsCount = attemptDetails.size,
            totalDurationMs = totalDuration,
            attemptDetails = attemptDetails
        )
    }
    
    /**
     * Gets statistics for a specific source.
     */
    fun getStats(sourceId: String): SourceStats? = sourceStats[sourceId]
    
    /**
     * Gets statistics for all sources.
     */
    fun getAllStats(): Map<String, SourceStats> = sourceStats.toMap()
    
    /**
     * Clears all recorded statistics.
     */
    fun clearStats() {
        sourceStats.clear()
    }
    
    /**
     * Creates a source with the given parameters.
     */
    fun createSource(
        id: String,
        priority: Int,
        timeoutMs: Long = defaultTimeout,
        loader: suspend () -> WaterfallResult<T>
    ) = WaterfallSource(id, priority, timeoutMs, loader)
    
    private fun recordStats(sourceId: String, result: WaterfallResult<T>, durationMs: Long) {
        val stats = sourceStats.getOrPut(sourceId) { SourceStats() }
        stats.totalLatencyMs.addAndGet(durationMs.toInt())
        
        when (result) {
            is WaterfallResult.Success -> stats.successCount.incrementAndGet()
            is WaterfallResult.NoFill -> stats.noFillCount.incrementAndGet()
            is WaterfallResult.Error -> stats.failureCount.incrementAndGet()
            is WaterfallResult.Timeout -> stats.timeoutCount.incrementAndGet()
        }
    }
}

/**
 * Builder for creating a waterfall configuration.
 */
class WaterfallBuilder<T>(private val defaultTimeout: Long = 5000L) {
    private val sources = mutableListOf<FallbackWaterfall.WaterfallSource<T>>()
    
    fun addSource(
        id: String,
        priority: Int,
        timeoutMs: Long = defaultTimeout,
        loader: suspend () -> FallbackWaterfall.WaterfallResult<T>
    ): WaterfallBuilder<T> {
        sources.add(FallbackWaterfall.WaterfallSource(id, priority, timeoutMs, loader))
        return this
    }
    
    fun build(): List<FallbackWaterfall.WaterfallSource<T>> = sources.toList()
}
