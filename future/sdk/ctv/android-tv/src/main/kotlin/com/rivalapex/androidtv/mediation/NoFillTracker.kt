package com.rivalapex.androidtv.mediation

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import java.util.Calendar
import java.util.concurrent.CopyOnWriteArrayList

/**
 * Reasons for ad no-fill on Android TV
 */
enum class NoFillReason(val value: String) {
    TIMEOUT("timeout"),
    NO_INVENTORY("no_inventory"),
    NETWORK_ERROR("network_error"),
    POLICY_VIOLATION("policy_violation"),
    FREQUENCY_CAP("frequency_cap"),
    GEOGRAPHIC_RESTRICTION("geographic_restriction"),
    BUDGET_EXHAUSTED("budget_exhausted"),
    MALFORMED_RESPONSE("malformed_response"),
    SERVER_ERROR("server_error"),
    CTV_NOT_SUPPORTED("ctv_not_supported"),
    FORMAT_MISMATCH("format_mismatch"),
    UNKNOWN("unknown")
}

/**
 * A single no-fill event for Android TV
 */
data class NoFillEvent(
    val sourceId: String,
    val placementId: String,
    val reason: NoFillReason,
    val timestamp: Long = System.currentTimeMillis(),
    val latencyMs: Long = 0,
    val adFormat: String? = null,
    val metadata: Map<String, String> = emptyMap()
)

/**
 * Statistics for no-fill events
 */
data class NoFillStats(
    val totalNoFills: Int,
    val noFillRate: Double,
    val averageLatencyMs: Double,
    val topReasons: Map<NoFillReason, Int>,
    val topSources: Map<String, Int>,
    val topPlacements: Map<String, Int>,
    val formatBreakdown: Map<String, Int>,
    val hourlyBreakdown: Map<Int, Int>
)

/**
 * Detected no-fill pattern
 */
data class NoFillPattern(
    val type: PatternType,
    val severity: Severity,
    val description: String,
    val detectedAt: Long = System.currentTimeMillis(),
    val affectedSourceId: String? = null,
    val affectedPlacementId: String? = null
) {
    enum class PatternType {
        ELEVATED_RATE,
        SOURCE_SPECIFIC,
        PLACEMENT_SPECIFIC,
        FORMAT_SPECIFIC,
        CONSECUTIVE_FAILURES
    }
    
    enum class Severity {
        LOW, MEDIUM, HIGH, CRITICAL
    }
}

/**
 * Configuration for NoFillTracker
 */
data class NoFillTrackerConfig(
    val maxEventsRetained: Int = 5_000,
    val maxRetentionHours: Int = 12,
    val elevatedRateThreshold: Double = 0.5,
    val patternDetectionEnabled: Boolean = true,
    val consecutiveFailureThreshold: Int = 3
)

/**
 * Pattern listener interface
 */
fun interface NoFillPatternListener {
    fun onPatternDetected(pattern: NoFillPattern)
}

/**
 * Tracks no-fill events for Android TV analytics
 */
object NoFillTracker {
    
    private var config = NoFillTrackerConfig()
    private val events = CopyOnWriteArrayList<NoFillEvent>()
    private val lock = Object()
    
    private val totalNoFills = AtomicInteger(0)
    private val totalLatencyMs = AtomicLong(0)
    
    private val hourlyNoFills = ConcurrentHashMap<Int, AtomicInteger>()
    private val noFillsBySource = ConcurrentHashMap<String, AtomicInteger>()
    private val noFillsByPlacement = ConcurrentHashMap<String, AtomicInteger>()
    private val noFillsByReason = ConcurrentHashMap<NoFillReason, AtomicInteger>()
    private val noFillsByFormat = ConcurrentHashMap<String, AtomicInteger>()
    
    private val consecutiveNoFillsBySource = ConcurrentHashMap<String, AtomicInteger>()
    private val detectedPatterns = CopyOnWriteArrayList<NoFillPattern>()
    private val patternListeners = CopyOnWriteArrayList<NoFillPatternListener>()
    
    /**
     * Record a no-fill event
     */
    fun recordNoFill(
        sourceId: String,
        placementId: String,
        reason: NoFillReason,
        latencyMs: Long = 0,
        adFormat: String? = null,
        metadata: Map<String, String> = emptyMap()
    ) {
        val event = NoFillEvent(
            sourceId = sourceId,
            placementId = placementId,
            reason = reason,
            latencyMs = latencyMs,
            adFormat = adFormat,
            metadata = metadata
        )
        
        synchronized(lock) {
            events.add(event)
            totalNoFills.incrementAndGet()
            totalLatencyMs.addAndGet(latencyMs)
            
            noFillsBySource.getOrPut(sourceId) { AtomicInteger(0) }.incrementAndGet()
            noFillsByPlacement.getOrPut(placementId) { AtomicInteger(0) }.incrementAndGet()
            noFillsByReason.getOrPut(reason) { AtomicInteger(0) }.incrementAndGet()
            
            adFormat?.let { format ->
                noFillsByFormat.getOrPut(format) { AtomicInteger(0) }.incrementAndGet()
            }
            
            val calendar = Calendar.getInstance()
            calendar.timeInMillis = event.timestamp
            val hour = calendar.get(Calendar.HOUR_OF_DAY)
            hourlyNoFills.getOrPut(hour) { AtomicInteger(0) }.incrementAndGet()
            
            consecutiveNoFillsBySource.getOrPut(sourceId) { AtomicInteger(0) }.incrementAndGet()
            
            cleanupIfNeeded()
            
            if (config.patternDetectionEnabled) {
                detectPatternsForEvent(event)
            }
        }
    }
    
    /**
     * Record a successful fill
     */
    fun recordFill(sourceId: String) {
        consecutiveNoFillsBySource[sourceId]?.set(0)
    }
    
    /**
     * Get current statistics
     */
    fun getStats(): NoFillStats {
        val total = totalNoFills.get()
        val avgLatency = if (total > 0) {
            totalLatencyMs.get().toDouble() / total
        } else 0.0
        
        val oneHourAgo = System.currentTimeMillis() - 3_600_000
        val recentCount = events.count { it.timestamp >= oneHourAgo }
        val ratePerMinute = recentCount.toDouble() / 60.0
        
        return NoFillStats(
            totalNoFills = total,
            noFillRate = ratePerMinute,
            averageLatencyMs = avgLatency,
            topReasons = noFillsByReason.mapValues { it.value.get() },
            topSources = noFillsBySource.mapValues { it.value.get() },
            topPlacements = noFillsByPlacement.mapValues { it.value.get() },
            formatBreakdown = noFillsByFormat.mapValues { it.value.get() },
            hourlyBreakdown = hourlyNoFills.mapValues { it.value.get() }
        )
    }
    
    /**
     * Get no-fills by ad format
     */
    fun getNoFillsByFormat(): Map<String, Int> {
        return noFillsByFormat.mapValues { it.value.get() }
    }
    
    /**
     * Get detected patterns
     */
    fun getDetectedPatterns(): List<NoFillPattern> {
        return detectedPatterns.toList()
    }
    
    /**
     * Add pattern listener
     */
    fun addPatternListener(listener: NoFillPatternListener) {
        patternListeners.add(listener)
    }
    
    /**
     * Remove pattern listener
     */
    fun removePatternListener(listener: NoFillPatternListener) {
        patternListeners.remove(listener)
    }
    
    /**
     * Get recent events
     */
    fun getRecentEvents(count: Int = 50): List<NoFillEvent> {
        return events.takeLast(count)
    }
    
    /**
     * Clear all data
     */
    fun clear() {
        synchronized(lock) {
            events.clear()
            totalNoFills.set(0)
            totalLatencyMs.set(0)
            hourlyNoFills.clear()
            noFillsBySource.clear()
            noFillsByPlacement.clear()
            noFillsByReason.clear()
            noFillsByFormat.clear()
            consecutiveNoFillsBySource.clear()
            detectedPatterns.clear()
        }
    }
    
    /**
     * Update configuration
     */
    fun updateConfiguration(newConfig: NoFillTrackerConfig) {
        synchronized(lock) {
            config = newConfig
        }
    }
    
    private fun cleanupIfNeeded() {
        val cutoff = System.currentTimeMillis() - (config.maxRetentionHours * 3_600_000L)
        events.removeAll { it.timestamp < cutoff }
        
        while (events.size > config.maxEventsRetained) {
            events.removeAt(0)
        }
    }
    
    private fun detectPatternsForEvent(event: NoFillEvent) {
        // Check for consecutive failures
        val consecutive = consecutiveNoFillsBySource[event.sourceId]?.get() ?: 0
        if (consecutive >= config.consecutiveFailureThreshold) {
            val severity = when {
                consecutive >= 10 -> NoFillPattern.Severity.CRITICAL
                consecutive >= 5 -> NoFillPattern.Severity.HIGH
                else -> NoFillPattern.Severity.MEDIUM
            }
            val pattern = NoFillPattern(
                type = NoFillPattern.PatternType.CONSECUTIVE_FAILURES,
                severity = severity,
                description = "Source ${event.sourceId} has $consecutive consecutive no-fills",
                affectedSourceId = event.sourceId
            )
            addPattern(pattern)
        }
        
        // Check for format-specific issues
        event.adFormat?.let { format ->
            val formatTotal = noFillsByFormat[format]?.get() ?: 0
            if (formatTotal > 10) {
                val total = maxOf(1, totalNoFills.get())
                val formatRate = formatTotal.toDouble() / total
                if (formatRate > 0.5) {
                    val pattern = NoFillPattern(
                        type = NoFillPattern.PatternType.FORMAT_SPECIFIC,
                        severity = NoFillPattern.Severity.HIGH,
                        description = "Format '$format' accounts for ${(formatRate * 100).toInt()}% of no-fills"
                    )
                    addPattern(pattern)
                }
            }
        }
    }
    
    private fun addPattern(pattern: NoFillPattern) {
        val recentCutoff = System.currentTimeMillis() - 300_000
        val isDuplicate = detectedPatterns.any { existing ->
            existing.type == pattern.type &&
            existing.affectedSourceId == pattern.affectedSourceId &&
            existing.detectedAt > recentCutoff
        }
        
        if (!isDuplicate) {
            detectedPatterns.add(pattern)
            
            val oneDayAgo = System.currentTimeMillis() - 86_400_000
            detectedPatterns.removeAll { it.detectedAt < oneDayAgo }
            
            patternListeners.forEach { it.onPatternDetected(pattern) }
        }
    }
}
