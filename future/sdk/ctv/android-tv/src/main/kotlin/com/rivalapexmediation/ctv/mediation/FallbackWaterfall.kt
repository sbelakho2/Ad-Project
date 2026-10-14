package com.rivalapexmediation.ctv.mediation

import kotlinx.coroutines.*
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.ConcurrentHashMap

/**
 * FallbackWaterfall for Android TV - Implements waterfall mediation for CTV ad loading.
 *
 * Optimized for CTV environments with faster timeouts and streaming-aware behavior.
 */
class FallbackWaterfall<T>(
    private val defaultTimeout: Long = 3000L
) {
    
    data class WaterfallSource<T>(
        val id: String,
        val priority: Int,
        val timeoutMs: Long,
        val loader: suspend () -> WaterfallResult<T>
    )
    
    sealed class WaterfallResult<out T> {
        data class Success<T>(val data: T) : WaterfallResult<T>()
        data class NoFill(val reason: String = "No ad available") : WaterfallResult<Nothing>()
        data class Error(val error: Throwable) : WaterfallResult<Nothing>()
        object Timeout : WaterfallResult<Nothing>()
    }
    
    data class ExecutionResult<T>(
        val result: WaterfallResult<T>,
        val sourceId: String,
        val attemptsCount: Int,
        val totalDurationMs: Long,
        val attemptDetails: List<AttemptDetail>
    )
    
    data class AttemptDetail(
        val sourceId: String,
        val priority: Int,
        val durationMs: Long,
        val result: AttemptResultType
    )
    
    enum class AttemptResultType {
        SUCCESS, NO_FILL, ERROR, TIMEOUT, SKIPPED
    }
    
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
     * Execute waterfall for CTV ad break
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
                AttemptDetail(source.id, source.priority, attemptDuration, attemptResultType)
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
        
        return ExecutionResult(
            result = WaterfallResult.NoFill("All ${sortedSources.size} sources exhausted"),
            sourceId = sortedSources.lastOrNull()?.id ?: "none",
            attemptsCount = attemptDetails.size,
            totalDurationMs = System.currentTimeMillis() - startTime,
            attemptDetails = attemptDetails
        )
    }
    
    fun getStats(sourceId: String): SourceStats? = sourceStats[sourceId]
    
    fun getAllStats(): Map<String, SourceStats> = sourceStats.toMap()
    
    fun clearStats() {
        sourceStats.clear()
    }
    
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
