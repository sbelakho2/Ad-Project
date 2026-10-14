package com.rivalapexmediation.sdk.metering

import kotlinx.coroutines.*
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.max

/**
 * Usage meter event types
 */
enum class UsageEventType {
    AD_REQUEST,
    AD_IMPRESSION,
    AD_CLICK,
    AD_VIDEO_START,
    AD_VIDEO_COMPLETE,
    AD_REVENUE,
    API_CALL,
    CACHE_HIT,
    CACHE_MISS,
    ERROR
}

/**
 * Usage event data
 */
data class UsageEvent(
    val type: UsageEventType,
    val timestamp: Long = System.currentTimeMillis(),
    val placementId: String? = null,
    val adapterId: String? = null,
    val adFormat: String? = null,
    val revenueAmount: Double? = null,
    val metadata: Map<String, Any>? = null
)

/**
 * Aggregated usage metrics
 */
data class UsageMetrics(
    val adRequests: Long,
    val adImpressions: Long,
    val adClicks: Long,
    val videoStarts: Long,
    val videoCompletes: Long,
    val totalRevenue: Double,
    val apiCalls: Long,
    val cacheHits: Long,
    val cacheMisses: Long,
    val errors: Long,
    val periodStart: Long,
    val periodEnd: Long
)

/**
 * Breakdown by dimension
 */
data class UsageBreakdown(
    val byPlacement: Map<String, UsageMetrics>,
    val byAdapter: Map<String, UsageMetrics>,
    val byAdFormat: Map<String, UsageMetrics>
)

/**
 * Metering configuration
 */
data class MeteringConfig(
    val flushIntervalMs: Long = 60_000,
    val maxEventsBeforeFlush: Int = 1000,
    val enableLocalStorage: Boolean = true,
    val enableRemoteReporting: Boolean = true,
    val samplingRate: Double = 1.0
)

/**
 * Remote reporter interface
 */
interface MeteringReporter {
    suspend fun report(metrics: UsageMetrics, breakdown: UsageBreakdown): Boolean
}

/**
 * Counter with atomic operations
 */
private class AtomicDoubleCounter {
    private val bits = AtomicLong(0)
    
    fun add(value: Double) {
        while (true) {
            val current = bits.get()
            val currentVal = java.lang.Double.longBitsToDouble(current)
            val next = java.lang.Double.doubleToRawLongBits(currentVal + value)
            if (bits.compareAndSet(current, next)) break
        }
    }
    
    fun get(): Double = java.lang.Double.longBitsToDouble(bits.get())
    
    fun reset() {
        bits.set(0)
    }
}

/**
 * Usage metering for tracking billable events and generating reports
 */
class UsageMeter(
    private val config: MeteringConfig = MeteringConfig(),
    private val reporter: MeteringReporter? = null,
    private val scope: CoroutineScope = CoroutineScope(Dispatchers.Default + SupervisorJob())
) {
    // Global counters
    private val adRequests = AtomicLong(0)
    private val adImpressions = AtomicLong(0)
    private val adClicks = AtomicLong(0)
    private val videoStarts = AtomicLong(0)
    private val videoCompletes = AtomicLong(0)
    private val totalRevenue = AtomicDoubleCounter()
    private val apiCalls = AtomicLong(0)
    private val cacheHits = AtomicLong(0)
    private val cacheMisses = AtomicLong(0)
    private val errors = AtomicLong(0)
    
    // Dimensional breakdowns
    private val placementMetrics = ConcurrentHashMap<String, DimensionMetrics>()
    private val adapterMetrics = ConcurrentHashMap<String, DimensionMetrics>()
    private val formatMetrics = ConcurrentHashMap<String, DimensionMetrics>()
    
    // Pending events
    private val pendingEvents = mutableListOf<UsageEvent>()
    private val eventsLock = Object()
    
    // Period tracking
    @Volatile
    private var periodStart = System.currentTimeMillis()
    
    // Flush job
    private var flushJob: Job? = null
    private var isRunning = false
    
    /**
     * Dimension-specific metrics
     */
    private class DimensionMetrics {
        val adRequests = AtomicLong(0)
        val adImpressions = AtomicLong(0)
        val adClicks = AtomicLong(0)
        val videoStarts = AtomicLong(0)
        val videoCompletes = AtomicLong(0)
        val totalRevenue = AtomicDoubleCounter()
        val apiCalls = AtomicLong(0)
        val cacheHits = AtomicLong(0)
        val cacheMisses = AtomicLong(0)
        val errors = AtomicLong(0)
        
        fun toMetrics(periodStart: Long, periodEnd: Long) = UsageMetrics(
            adRequests = adRequests.get(),
            adImpressions = adImpressions.get(),
            adClicks = adClicks.get(),
            videoStarts = videoStarts.get(),
            videoCompletes = videoCompletes.get(),
            totalRevenue = totalRevenue.get(),
            apiCalls = apiCalls.get(),
            cacheHits = cacheHits.get(),
            cacheMisses = cacheMisses.get(),
            errors = errors.get(),
            periodStart = periodStart,
            periodEnd = periodEnd
        )
        
        fun reset() {
            adRequests.set(0)
            adImpressions.set(0)
            adClicks.set(0)
            videoStarts.set(0)
            videoCompletes.set(0)
            totalRevenue.reset()
            apiCalls.set(0)
            cacheHits.set(0)
            cacheMisses.set(0)
            errors.set(0)
        }
    }
    
    /**
     * Start the metering service
     */
    fun start() {
        if (isRunning) return
        isRunning = true
        periodStart = System.currentTimeMillis()
        
        flushJob = scope.launch {
            while (isActive && isRunning) {
                delay(config.flushIntervalMs)
                flush()
            }
        }
    }
    
    /**
     * Stop the metering service
     */
    fun stop() {
        isRunning = false
        flushJob?.cancel()
        flushJob = null
    }
    
    /**
     * Record a usage event
     */
    fun record(event: UsageEvent) {
        // Apply sampling
        if (config.samplingRate < 1.0 && Math.random() > config.samplingRate) {
            return
        }
        
        // Update global counters
        updateCounters(event)
        
        // Update dimensional breakdowns
        event.placementId?.let { placementId ->
            val metrics = placementMetrics.getOrPut(placementId) { DimensionMetrics() }
            updateDimensionMetrics(metrics, event)
        }
        
        event.adapterId?.let { adapterId ->
            val metrics = adapterMetrics.getOrPut(adapterId) { DimensionMetrics() }
            updateDimensionMetrics(metrics, event)
        }
        
        event.adFormat?.let { format ->
            val metrics = formatMetrics.getOrPut(format) { DimensionMetrics() }
            updateDimensionMetrics(metrics, event)
        }
        
        // Queue event for detailed storage
        if (config.enableLocalStorage) {
            synchronized(eventsLock) {
                pendingEvents.add(event)
                if (pendingEvents.size >= config.maxEventsBeforeFlush) {
                    scope.launch { flush() }
                }
            }
        }
    }
    
    /**
     * Update global counters
     */
    private fun updateCounters(event: UsageEvent) {
        when (event.type) {
            UsageEventType.AD_REQUEST -> adRequests.incrementAndGet()
            UsageEventType.AD_IMPRESSION -> adImpressions.incrementAndGet()
            UsageEventType.AD_CLICK -> adClicks.incrementAndGet()
            UsageEventType.AD_VIDEO_START -> videoStarts.incrementAndGet()
            UsageEventType.AD_VIDEO_COMPLETE -> videoCompletes.incrementAndGet()
            UsageEventType.AD_REVENUE -> event.revenueAmount?.let { totalRevenue.add(it) }
            UsageEventType.API_CALL -> apiCalls.incrementAndGet()
            UsageEventType.CACHE_HIT -> cacheHits.incrementAndGet()
            UsageEventType.CACHE_MISS -> cacheMisses.incrementAndGet()
            UsageEventType.ERROR -> errors.incrementAndGet()
        }
    }
    
    /**
     * Update dimension-specific metrics
     */
    private fun updateDimensionMetrics(metrics: DimensionMetrics, event: UsageEvent) {
        when (event.type) {
            UsageEventType.AD_REQUEST -> metrics.adRequests.incrementAndGet()
            UsageEventType.AD_IMPRESSION -> metrics.adImpressions.incrementAndGet()
            UsageEventType.AD_CLICK -> metrics.adClicks.incrementAndGet()
            UsageEventType.AD_VIDEO_START -> metrics.videoStarts.incrementAndGet()
            UsageEventType.AD_VIDEO_COMPLETE -> metrics.videoCompletes.incrementAndGet()
            UsageEventType.AD_REVENUE -> event.revenueAmount?.let { metrics.totalRevenue.add(it) }
            UsageEventType.API_CALL -> metrics.apiCalls.incrementAndGet()
            UsageEventType.CACHE_HIT -> metrics.cacheHits.incrementAndGet()
            UsageEventType.CACHE_MISS -> metrics.cacheMisses.incrementAndGet()
            UsageEventType.ERROR -> metrics.errors.incrementAndGet()
        }
    }
    
    /**
     * Convenience methods for recording events
     */
    fun recordRequest(placementId: String, adapterId: String? = null, adFormat: String? = null) {
        record(UsageEvent(
            type = UsageEventType.AD_REQUEST,
            placementId = placementId,
            adapterId = adapterId,
            adFormat = adFormat
        ))
    }
    
    fun recordImpression(placementId: String, adapterId: String, adFormat: String? = null) {
        record(UsageEvent(
            type = UsageEventType.AD_IMPRESSION,
            placementId = placementId,
            adapterId = adapterId,
            adFormat = adFormat
        ))
    }
    
    fun recordClick(placementId: String, adapterId: String, adFormat: String? = null) {
        record(UsageEvent(
            type = UsageEventType.AD_CLICK,
            placementId = placementId,
            adapterId = adapterId,
            adFormat = adFormat
        ))
    }
    
    fun recordRevenue(placementId: String, adapterId: String, amount: Double) {
        record(UsageEvent(
            type = UsageEventType.AD_REVENUE,
            placementId = placementId,
            adapterId = adapterId,
            revenueAmount = amount
        ))
    }
    
    fun recordVideoStart(placementId: String, adapterId: String) {
        record(UsageEvent(
            type = UsageEventType.AD_VIDEO_START,
            placementId = placementId,
            adapterId = adapterId,
            adFormat = "video"
        ))
    }
    
    fun recordVideoComplete(placementId: String, adapterId: String) {
        record(UsageEvent(
            type = UsageEventType.AD_VIDEO_COMPLETE,
            placementId = placementId,
            adapterId = adapterId,
            adFormat = "video"
        ))
    }
    
    fun recordCacheHit(placementId: String? = null) {
        record(UsageEvent(
            type = UsageEventType.CACHE_HIT,
            placementId = placementId
        ))
    }
    
    fun recordCacheMiss(placementId: String? = null) {
        record(UsageEvent(
            type = UsageEventType.CACHE_MISS,
            placementId = placementId
        ))
    }
    
    fun recordError(placementId: String? = null, adapterId: String? = null) {
        record(UsageEvent(
            type = UsageEventType.ERROR,
            placementId = placementId,
            adapterId = adapterId
        ))
    }
    
    /**
     * Get current metrics snapshot
     */
    fun getMetrics(): UsageMetrics {
        val now = System.currentTimeMillis()
        return UsageMetrics(
            adRequests = adRequests.get(),
            adImpressions = adImpressions.get(),
            adClicks = adClicks.get(),
            videoStarts = videoStarts.get(),
            videoCompletes = videoCompletes.get(),
            totalRevenue = totalRevenue.get(),
            apiCalls = apiCalls.get(),
            cacheHits = cacheHits.get(),
            cacheMisses = cacheMisses.get(),
            errors = errors.get(),
            periodStart = periodStart,
            periodEnd = now
        )
    }
    
    /**
     * Get breakdown by dimensions
     */
    fun getBreakdown(): UsageBreakdown {
        val now = System.currentTimeMillis()
        
        return UsageBreakdown(
            byPlacement = placementMetrics.mapValues { it.value.toMetrics(periodStart, now) },
            byAdapter = adapterMetrics.mapValues { it.value.toMetrics(periodStart, now) },
            byAdFormat = formatMetrics.mapValues { it.value.toMetrics(periodStart, now) }
        )
    }
    
    /**
     * Get click-through rate
     */
    fun getCTR(): Double {
        val impressions = adImpressions.get()
        if (impressions == 0L) return 0.0
        return adClicks.get().toDouble() / impressions.toDouble()
    }
    
    /**
     * Get fill rate
     */
    fun getFillRate(): Double {
        val requests = adRequests.get()
        if (requests == 0L) return 0.0
        return adImpressions.get().toDouble() / requests.toDouble()
    }
    
    /**
     * Get video completion rate
     */
    fun getVideoCompletionRate(): Double {
        val starts = videoStarts.get()
        if (starts == 0L) return 0.0
        return videoCompletes.get().toDouble() / starts.toDouble()
    }
    
    /**
     * Get cache hit rate
     */
    fun getCacheHitRate(): Double {
        val total = cacheHits.get() + cacheMisses.get()
        if (total == 0L) return 0.0
        return cacheHits.get().toDouble() / total.toDouble()
    }
    
    /**
     * Get effective CPM
     */
    fun getEffectiveCPM(): Double {
        val impressions = adImpressions.get()
        if (impressions == 0L) return 0.0
        return (totalRevenue.get() / impressions.toDouble()) * 1000.0
    }
    
    /**
     * Flush metrics to reporter
     */
    suspend fun flush(): Boolean {
        if (!config.enableRemoteReporting || reporter == null) return true
        
        val metrics = getMetrics()
        val breakdown = getBreakdown()
        
        return try {
            val success = reporter.report(metrics, breakdown)
            if (success) {
                reset()
            }
            success
        } catch (e: Exception) {
            false
        }
    }
    
    /**
     * Reset all counters
     */
    fun reset() {
        adRequests.set(0)
        adImpressions.set(0)
        adClicks.set(0)
        videoStarts.set(0)
        videoCompletes.set(0)
        totalRevenue.reset()
        apiCalls.set(0)
        cacheHits.set(0)
        cacheMisses.set(0)
        errors.set(0)
        
        placementMetrics.values.forEach { it.reset() }
        adapterMetrics.values.forEach { it.reset() }
        formatMetrics.values.forEach { it.reset() }
        
        synchronized(eventsLock) {
            pendingEvents.clear()
        }
        
        periodStart = System.currentTimeMillis()
    }
    
    /**
     * Export metrics as JSON
     */
    fun exportAsJSON(): String {
        val metrics = getMetrics()
        val breakdown = getBreakdown()
        
        val sb = StringBuilder()
        sb.append("{\n")
        sb.append("  \"metrics\": {\n")
        sb.append("    \"adRequests\": ${metrics.adRequests},\n")
        sb.append("    \"adImpressions\": ${metrics.adImpressions},\n")
        sb.append("    \"adClicks\": ${metrics.adClicks},\n")
        sb.append("    \"videoStarts\": ${metrics.videoStarts},\n")
        sb.append("    \"videoCompletes\": ${metrics.videoCompletes},\n")
        sb.append("    \"totalRevenue\": ${metrics.totalRevenue},\n")
        sb.append("    \"apiCalls\": ${metrics.apiCalls},\n")
        sb.append("    \"cacheHits\": ${metrics.cacheHits},\n")
        sb.append("    \"cacheMisses\": ${metrics.cacheMisses},\n")
        sb.append("    \"errors\": ${metrics.errors},\n")
        sb.append("    \"periodStart\": ${metrics.periodStart},\n")
        sb.append("    \"periodEnd\": ${metrics.periodEnd}\n")
        sb.append("  },\n")
        sb.append("  \"computed\": {\n")
        sb.append("    \"ctr\": ${getCTR()},\n")
        sb.append("    \"fillRate\": ${getFillRate()},\n")
        sb.append("    \"videoCompletionRate\": ${getVideoCompletionRate()},\n")
        sb.append("    \"cacheHitRate\": ${getCacheHitRate()},\n")
        sb.append("    \"effectiveCPM\": ${getEffectiveCPM()}\n")
        sb.append("  },\n")
        sb.append("  \"breakdown\": {\n")
        sb.append("    \"placementCount\": ${breakdown.byPlacement.size},\n")
        sb.append("    \"adapterCount\": ${breakdown.byAdapter.size},\n")
        sb.append("    \"formatCount\": ${breakdown.byAdFormat.size}\n")
        sb.append("  }\n")
        sb.append("}")
        
        return sb.toString()
    }
}

/**
 * Builder for UsageMeter
 */
class UsageMeterBuilder {
    private var config = MeteringConfig()
    private var reporter: MeteringReporter? = null
    private var scope: CoroutineScope? = null
    
    fun config(config: MeteringConfig) = apply { this.config = config }
    fun reporter(reporter: MeteringReporter) = apply { this.reporter = reporter }
    fun scope(scope: CoroutineScope) = apply { this.scope = scope }
    
    fun flushIntervalMs(interval: Long) = apply {
        config = config.copy(flushIntervalMs = interval)
    }
    
    fun maxEventsBeforeFlush(max: Int) = apply {
        config = config.copy(maxEventsBeforeFlush = max)
    }
    
    fun samplingRate(rate: Double) = apply {
        config = config.copy(samplingRate = rate.coerceIn(0.0, 1.0))
    }
    
    fun enableLocalStorage(enabled: Boolean) = apply {
        config = config.copy(enableLocalStorage = enabled)
    }
    
    fun enableRemoteReporting(enabled: Boolean) = apply {
        config = config.copy(enableRemoteReporting = enabled)
    }
    
    fun build(): UsageMeter {
        return UsageMeter(
            config = config,
            reporter = reporter,
            scope = scope ?: CoroutineScope(Dispatchers.Default + SupervisorJob())
        )
    }
}
