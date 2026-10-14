package com.rivalapex.sdk.mediation

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/**
 * NoFillTracker - Tracks no-fill events and patterns for analytics.
 *
 * This class provides:
 * - No-fill event tracking per ad unit and source
 * - Fill rate calculation
 * - No-fill reason categorization
 * - Time-based analytics (hourly/daily patterns)
 * - Real-time fill rate monitoring
 */
object NoFillTracker {
    
    /** No-fill reason categories */
    enum class NoFillReason {
        NO_INVENTORY,          // No ads available from source
        TIMEOUT,               // Request timed out
        NETWORK_ERROR,         // Network connectivity issue
        PARSE_ERROR,           // Response parsing failed
        INVALID_RESPONSE,      // Response was invalid/malformed
        TARGETING_MISMATCH,    // User/context didn't match targeting
        FREQUENCY_CAP,         // User hit frequency cap
        BUDGET_EXHAUSTED,      // Campaign budget exhausted
        GEO_RESTRICTED,        // Geographic restriction
        UNKNOWN                // Unknown or unspecified reason
    }
    
    /** A single no-fill event record */
    data class NoFillEvent(
        val timestamp: Long = System.currentTimeMillis(),
        val adUnitId: String,
        val sourceId: String,
        val reason: NoFillReason,
        val details: String? = null,
        val latencyMs: Long = 0
    )
    
    /** Statistics for a specific ad unit/source combination */
    data class FillStats(
        val requests: AtomicInteger = AtomicInteger(0),
        val fills: AtomicInteger = AtomicInteger(0),
        val noFills: AtomicInteger = AtomicInteger(0),
        val totalLatencyMs: AtomicLong = AtomicLong(0),
        val reasonCounts: ConcurrentHashMap<NoFillReason, AtomicInteger> = ConcurrentHashMap()
    ) {
        val fillRate: Float
            get() = if (requests.get() > 0) fills.get().toFloat() / requests.get() else 0f
        
        val noFillRate: Float
            get() = if (requests.get() > 0) noFills.get().toFloat() / requests.get() else 0f
        
        val averageLatencyMs: Long
            get() = if (requests.get() > 0) totalLatencyMs.get() / requests.get() else 0
        
        fun getReasonCount(reason: NoFillReason): Int =
            reasonCounts[reason]?.get() ?: 0
        
        fun topNoFillReasons(limit: Int = 3): List<Pair<NoFillReason, Int>> =
            reasonCounts.map { it.key to it.value.get() }
                .sortedByDescending { it.second }
                .take(limit)
    }
    
    /** Time bucket for hourly analytics */
    data class HourlyBucket(
        val hour: Int,
        val requests: AtomicInteger = AtomicInteger(0),
        val fills: AtomicInteger = AtomicInteger(0)
    ) {
        val fillRate: Float
            get() = if (requests.get() > 0) fills.get().toFloat() / requests.get() else 0f
    }
    
    private val adUnitStats = ConcurrentHashMap<String, FillStats>()
    private val sourceStats = ConcurrentHashMap<String, FillStats>()
    private val combinedStats = ConcurrentHashMap<String, FillStats>() // "adUnit:source"
    
    private val hourlyBuckets = ConcurrentHashMap<Int, HourlyBucket>()
    private val recentEvents = mutableListOf<NoFillEvent>()
    private const val MAX_RECENT_EVENTS = 100
    
    private val listeners = mutableListOf<(NoFillEvent) -> Unit>()
    
    /**
     * Records a successful ad fill.
     */
    fun recordFill(adUnitId: String, sourceId: String, latencyMs: Long = 0) {
        recordRequest(adUnitId, sourceId, latencyMs, filled = true)
    }
    
    /**
     * Records a no-fill event.
     */
    fun recordNoFill(
        adUnitId: String,
        sourceId: String,
        reason: NoFillReason,
        latencyMs: Long = 0,
        details: String? = null
    ) {
        recordRequest(adUnitId, sourceId, latencyMs, filled = false)
        
        val event = NoFillEvent(
            adUnitId = adUnitId,
            sourceId = sourceId,
            reason = reason,
            details = details,
            latencyMs = latencyMs
        )
        
        // Update reason counts
        getOrCreateStats(adUnitStats, adUnitId).reasonCounts
            .getOrPut(reason) { AtomicInteger(0) }.incrementAndGet()
        getOrCreateStats(sourceStats, sourceId).reasonCounts
            .getOrPut(reason) { AtomicInteger(0) }.incrementAndGet()
        getOrCreateStats(combinedStats, "$adUnitId:$sourceId").reasonCounts
            .getOrPut(reason) { AtomicInteger(0) }.incrementAndGet()
        
        // Store recent event
        synchronized(recentEvents) {
            recentEvents.add(event)
            if (recentEvents.size > MAX_RECENT_EVENTS) {
                recentEvents.removeAt(0)
            }
        }
        
        // Notify listeners
        listeners.forEach { it(event) }
    }
    
    private fun recordRequest(
        adUnitId: String,
        sourceId: String,
        latencyMs: Long,
        filled: Boolean
    ) {
        val currentHour = (System.currentTimeMillis() / 3600000 % 24).toInt()
        
        // Update ad unit stats
        val adStats = getOrCreateStats(adUnitStats, adUnitId)
        adStats.requests.incrementAndGet()
        adStats.totalLatencyMs.addAndGet(latencyMs)
        if (filled) adStats.fills.incrementAndGet() else adStats.noFills.incrementAndGet()
        
        // Update source stats
        val srcStats = getOrCreateStats(sourceStats, sourceId)
        srcStats.requests.incrementAndGet()
        srcStats.totalLatencyMs.addAndGet(latencyMs)
        if (filled) srcStats.fills.incrementAndGet() else srcStats.noFills.incrementAndGet()
        
        // Update combined stats
        val combStats = getOrCreateStats(combinedStats, "$adUnitId:$sourceId")
        combStats.requests.incrementAndGet()
        combStats.totalLatencyMs.addAndGet(latencyMs)
        if (filled) combStats.fills.incrementAndGet() else combStats.noFills.incrementAndGet()
        
        // Update hourly bucket
        val bucket = hourlyBuckets.getOrPut(currentHour) { HourlyBucket(currentHour) }
        bucket.requests.incrementAndGet()
        if (filled) bucket.fills.incrementAndGet()
    }
    
    private fun getOrCreateStats(map: ConcurrentHashMap<String, FillStats>, key: String): FillStats =
        map.getOrPut(key) { FillStats() }
    
    /**
     * Gets fill statistics for an ad unit.
     */
    fun getAdUnitStats(adUnitId: String): FillStats? = adUnitStats[adUnitId]
    
    /**
     * Gets fill statistics for a source.
     */
    fun getSourceStats(sourceId: String): FillStats? = sourceStats[sourceId]
    
    /**
     * Gets fill statistics for an ad unit and source combination.
     */
    fun getCombinedStats(adUnitId: String, sourceId: String): FillStats? =
        combinedStats["$adUnitId:$sourceId"]
    
    /**
     * Gets overall fill rate across all ad units.
     */
    fun getOverallFillRate(): Float {
        val totalRequests = adUnitStats.values.sumOf { it.requests.get() }
        val totalFills = adUnitStats.values.sumOf { it.fills.get() }
        return if (totalRequests > 0) totalFills.toFloat() / totalRequests else 0f
    }
    
    /**
     * Gets fill rate for a specific hour (0-23).
     */
    fun getHourlyFillRate(hour: Int): Float = hourlyBuckets[hour]?.fillRate ?: 0f
    
    /**
     * Gets hourly fill rate pattern for all hours.
     */
    fun getHourlyPattern(): Map<Int, Float> =
        (0..23).associateWith { getHourlyFillRate(it) }
    
    /**
     * Gets the best performing sources by fill rate.
     */
    fun getBestSources(limit: Int = 5): List<Pair<String, Float>> =
        sourceStats.map { it.key to it.value.fillRate }
            .sortedByDescending { it.second }
            .take(limit)
    
    /**
     * Gets the worst performing sources by fill rate.
     */
    fun getWorstSources(limit: Int = 5): List<Pair<String, Float>> =
        sourceStats.map { it.key to it.value.fillRate }
            .sortedBy { it.second }
            .take(limit)
    
    /**
     * Gets recent no-fill events.
     */
    fun getRecentNoFills(limit: Int = 10): List<NoFillEvent> =
        synchronized(recentEvents) {
            recentEvents.takeLast(limit)
        }
    
    /**
     * Gets summary report of fill statistics.
     */
    fun getSummaryReport(): Map<String, Any> = mapOf(
        "overallFillRate" to getOverallFillRate(),
        "totalRequests" to adUnitStats.values.sumOf { it.requests.get() },
        "totalFills" to adUnitStats.values.sumOf { it.fills.get() },
        "totalNoFills" to adUnitStats.values.sumOf { it.noFills.get() },
        "adUnitCount" to adUnitStats.size,
        "sourceCount" to sourceStats.size,
        "bestSources" to getBestSources(3),
        "worstSources" to getWorstSources(3)
    )
    
    /**
     * Adds a listener for no-fill events.
     */
    fun addNoFillListener(listener: (NoFillEvent) -> Unit) {
        listeners.add(listener)
    }
    
    /**
     * Removes a no-fill listener.
     */
    fun removeNoFillListener(listener: (NoFillEvent) -> Unit) {
        listeners.remove(listener)
    }
    
    /**
     * Clears all tracked statistics.
     */
    fun clear() {
        adUnitStats.clear()
        sourceStats.clear()
        combinedStats.clear()
        hourlyBuckets.clear()
        synchronized(recentEvents) {
            recentEvents.clear()
        }
    }
}
