package com.rivalapexmediation.ctv.mediation

import java.util.*
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/**
 * Reasons for ad no-fill on CTV
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
    UNKNOWN("unknown");

    companion object {
        fun fromString(value: String): NoFillReason {
            return entries.find { it.value == value } ?: UNKNOWN
        }
    }
}

/**
 * A single no-fill event for CTV
 */
data class NoFillEvent(
    val sourceId: String,
    val placementId: String,
    val reason: NoFillReason,
    val timestamp: Date = Date(),
    val latencyMs: Long = 0L,
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
    val detectedAt: Date,
    val affectedSourceId: String? = null,
    val affectedPlacementId: String? = null
) {
    enum class PatternType(val value: String) {
        ELEVATED_RATE("elevated_rate"),
        SOURCE_SPECIFIC("source_specific"),
        PLACEMENT_SPECIFIC("placement_specific"),
        FORMAT_SPECIFIC("format_specific"),
        CONSECUTIVE_FAILURES("consecutive_failures")
    }

    enum class Severity(val value: String) {
        LOW("low"),
        MEDIUM("medium"),
        HIGH("high"),
        CRITICAL("critical")
    }
}

/**
 * Configuration for NoFillTracker
 */
data class NoFillTrackerConfig(
    val maxEventsRetained: Int = 5_000, // Less memory for Android TV
    val maxRetentionHours: Int = 12, // Shorter retention for CTV
    val elevatedRateThreshold: Double = 0.5,
    val patternDetectionEnabled: Boolean = true,
    val consecutiveFailureThreshold: Int = 3 // More sensitive for CTV
) {
    companion object {
        val DEFAULT = NoFillTrackerConfig()
    }
}

/**
 * Tracks no-fill events for CTV analytics
 *
 * Provides:
 * - Event recording and retention
 * - Statistics aggregation
 * - Pattern detection for debugging
 * - Listener-based alerting
 */
object NoFillTracker {

    @Volatile
    private var config = NoFillTrackerConfig.DEFAULT

    private val events = CopyOnWriteArrayList<NoFillEvent>()
    private val totalNoFills = AtomicInteger(0)
    private val totalLatencyMs = AtomicLong(0)

    private val hourlyNoFills = ConcurrentHashMap<Int, AtomicInteger>()
    private val noFillsBySource = ConcurrentHashMap<String, AtomicInteger>()
    private val noFillsByPlacement = ConcurrentHashMap<String, AtomicInteger>()
    private val noFillsByReason = ConcurrentHashMap<NoFillReason, AtomicInteger>()
    private val noFillsByFormat = ConcurrentHashMap<String, AtomicInteger>()

    private val consecutiveNoFillsBySource = ConcurrentHashMap<String, AtomicInteger>()
    private val detectedPatterns = CopyOnWriteArrayList<NoFillPattern>()
    private val patternListeners = CopyOnWriteArrayList<(NoFillPattern) -> Unit>()

    /**
     * Record a no-fill event
     */
    @JvmStatic
    fun recordNoFill(
        sourceId: String,
        placementId: String,
        reason: NoFillReason,
        latencyMs: Long = 0L,
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

        events.add(event)
        totalNoFills.incrementAndGet()
        totalLatencyMs.addAndGet(latencyMs)

        noFillsBySource.getOrPut(sourceId) { AtomicInteger(0) }.incrementAndGet()
        noFillsByPlacement.getOrPut(placementId) { AtomicInteger(0) }.incrementAndGet()
        noFillsByReason.getOrPut(reason) { AtomicInteger(0) }.incrementAndGet()

        adFormat?.let {
            noFillsByFormat.getOrPut(it) { AtomicInteger(0) }.incrementAndGet()
        }

        val calendar = Calendar.getInstance()
        calendar.time = event.timestamp
        val hour = calendar.get(Calendar.HOUR_OF_DAY)
        hourlyNoFills.getOrPut(hour) { AtomicInteger(0) }.incrementAndGet()

        consecutiveNoFillsBySource.getOrPut(sourceId) { AtomicInteger(0) }.incrementAndGet()

        cleanupIfNeeded()

        if (config.patternDetectionEnabled) {
            detectPatternsForEvent(event)
        }
    }

    /**
     * Record a successful fill (resets consecutive no-fill counter)
     */
    @JvmStatic
    fun recordFill(sourceId: String) {
        consecutiveNoFillsBySource[sourceId]?.set(0)
    }

    /**
     * Get current statistics
     */
    @JvmStatic
    fun getStats(): NoFillStats {
        val total = totalNoFills.get()
        val avgLatency = if (total > 0) totalLatencyMs.get().toDouble() / total else 0.0

        val oneHourAgo = Date(System.currentTimeMillis() - 3600_000)
        val recentEvents = events.filter { it.timestamp >= oneHourAgo }
        val ratePerMinute = recentEvents.size.toDouble() / 60.0

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
    @JvmStatic
    fun getNoFillsByFormat(): Map<String, Int> {
        return noFillsByFormat.mapValues { it.value.get() }
    }

    /**
     * Get detected patterns
     */
    @JvmStatic
    fun getDetectedPatterns(): List<NoFillPattern> {
        return detectedPatterns.toList()
    }

    /**
     * Add pattern listener
     */
    @JvmStatic
    fun addPatternListener(listener: (NoFillPattern) -> Unit) {
        patternListeners.add(listener)
    }

    /**
     * Remove pattern listener
     */
    @JvmStatic
    fun removePatternListener(listener: (NoFillPattern) -> Unit) {
        patternListeners.remove(listener)
    }

    /**
     * Get recent events
     */
    @JvmStatic
    fun getRecentEvents(count: Int = 50): List<NoFillEvent> {
        return events.takeLast(count)
    }

    /**
     * Clear all data
     */
    @JvmStatic
    fun clear() {
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

    /**
     * Update configuration
     */
    @JvmStatic
    fun updateConfiguration(newConfig: NoFillTrackerConfig) {
        config = newConfig
    }

    private fun cleanupIfNeeded() {
        val cutoff = Date(System.currentTimeMillis() - config.maxRetentionHours * 3600_000L)
        events.removeIf { it.timestamp.before(cutoff) }

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
                detectedAt = Date(),
                affectedSourceId = event.sourceId
            )
            addPattern(pattern)
        }

        // Check for format-specific issues
        event.adFormat?.let { format ->
            val formatTotal = noFillsByFormat[format]?.get() ?: 0
            if (formatTotal > 10) {
                val total = totalNoFills.get()
                val formatRate = formatTotal.toDouble() / maxOf(1, total)
                if (formatRate > 0.5) {
                    val pattern = NoFillPattern(
                        type = NoFillPattern.PatternType.FORMAT_SPECIFIC,
                        severity = NoFillPattern.Severity.HIGH,
                        description = "Format '$format' accounts for ${(formatRate * 100).toInt()}% of no-fills",
                        detectedAt = Date()
                    )
                    addPattern(pattern)
                }
            }
        }
    }

    private fun addPattern(pattern: NoFillPattern) {
        val recentCutoff = Date(System.currentTimeMillis() - 300_000) // 5 minutes
        val isDuplicate = detectedPatterns.any { existing ->
            existing.type == pattern.type &&
            existing.affectedSourceId == pattern.affectedSourceId &&
            existing.detectedAt.after(recentCutoff)
        }

        if (!isDuplicate) {
            detectedPatterns.add(pattern)

            // Cleanup old patterns
            val oneDayAgo = Date(System.currentTimeMillis() - 86400_000)
            detectedPatterns.removeIf { it.detectedAt.before(oneDayAgo) }

            // Notify listeners
            patternListeners.forEach { it(pattern) }
        }
    }
}
