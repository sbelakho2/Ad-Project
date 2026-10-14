package com.rivalapexmediation.sdk.telemetry

import android.content.Context
import com.rivalapexmediation.sdk.BuildConfig
import com.rivalapexmediation.sdk.SDKConfig
import com.rivalapexmediation.sdk.models.EventType
import com.rivalapexmediation.sdk.models.TelemetryEvent
import com.google.gson.Gson
import com.rivalapexmediation.sdk.util.ClockProvider
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.zip.GZIPOutputStream

/**
 * Telemetry collector with batching and compression
 * 
 * Features:
 * - Event batching (reduces network calls)
 * - GZIP compression
 * - Retry logic with exponential backoff
 * - Background processing (no ANR risk)
 */
class TelemetryCollector(
    private val context: Context,
    private val config: SDKConfig
) {
    private val eventQueue = mutableListOf<TelemetryEvent>()
    private val queueLock = Any()
    
    private val executor = Executors.newSingleThreadScheduledExecutor { r ->
        Thread(r, "RivalApexMediation-Telemetry").apply {
            priority = Thread.MIN_PRIORITY
        }
    }
    
    private val httpClient = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()
    
    private val gson = Gson()
    
    private val batchSize = 10
    private val flushInterval = 30000L // 30 seconds
    private val random = java.util.Random()
    
    @Volatile
    private var isRunning = false
    private val sampleRate: Double get() = config.observabilitySampleRate.coerceIn(0.0, 1.0)

    // --- P1.9: Lightweight in-memory observability metrics (privacy-clean) ---
    // We track per {placement, adapter} rolling latencies and outcome counters in-memory
    // for quick developer diagnostics. Bounded memory via small reservoirs per key.
    private data class Key(val placement: String, val adapter: String)
    private data class Counters(
        var fills: Long = 0,
        var noFills: Long = 0,
        var timeouts: Long = 0,
        var errors: Long = 0,
    )
    private val counters = java.util.concurrent.ConcurrentHashMap<Key, Counters>()
    private val latencyReservoirs = java.util.concurrent.ConcurrentHashMap<Key, ArrayDeque<Long>>()
    private val reservoirMax = 200 // up to 200 recent latencies per key

    private fun getOrCreateCounters(key: Key): Counters {
        var c = counters[key]
        if (c != null) return c
        val created = Counters()
        val prev = counters.putIfAbsent(key, created)
        return prev ?: created
    }

    private fun getOrCreateReservoir(key: Key): ArrayDeque<Long> {
        var dq = latencyReservoirs[key]
        if (dq != null) return dq
        val created = ArrayDeque<Long>()
        val prev = latencyReservoirs.putIfAbsent(key, created)
        return prev ?: created
    }

    private fun recordOutcomeSampled(placement: String, adapter: String, outcome: String, latencyMs: Long?) {
        if (!config.observabilityEnabled) return
        if (!shouldSample()) return
        val key = Key(placement, adapter)
        // Update counters
        val c = getOrCreateCounters(key)
        when (outcome) {
            "fill" -> c.fills++
            "no_fill" -> c.noFills++
            "timeout" -> c.timeouts++
            "error" -> c.errors++
        }
        // Update latency reservoir
        if (latencyMs != null && latencyMs >= 0) {
            val dq = getOrCreateReservoir(key)
            dq.addLast(latencyMs)
            while (dq.size > reservoirMax) dq.removeFirst()
        }
    }

    // Percentiles computed over the local reservoir (best-effort, dev-only diagnostics)
    fun getLocalPercentiles(placement: String, adapter: String): Map<String, Long> {
        val key = Key(placement, adapter)
        val arr = latencyReservoirs[key]?.toLongArray() ?: LongArray(0)
        if (arr.isEmpty()) return emptyMap()
        java.util.Arrays.sort(arr)
        fun pct(p: Double): Long {
            if (arr.isEmpty()) return 0L
            val idx = kotlin.math.min(arr.size - 1, kotlin.math.max(0, kotlin.math.floor(p * (arr.size - 1)).toInt()))
            return arr[idx]
        }
        return mapOf(
            "p50" to pct(0.50),
            "p95" to pct(0.95),
            "p99" to pct(0.99),
        )
    }

    fun getLocalCounters(placement: String, adapter: String): Map<String, Long> {
        val key = Key(placement, adapter)
        val c = counters[key] ?: return emptyMap()
        return mapOf(
            "fills" to c.fills,
            "no_fills" to c.noFills,
            "timeouts" to c.timeouts,
            "errors" to c.errors,
        )
    }
    
    /**
     * Start telemetry collection
     */
    fun start() {
        if (!config.telemetryEnabled) {
            return
        }
        
        isRunning = true
        
        // Schedule periodic flush
        executor.scheduleWithFixedDelay(
            { flushEvents() },
            flushInterval,
            flushInterval,
            TimeUnit.MILLISECONDS
        )
    }
    
    /**
     * Stop telemetry collection
     */
    fun stop() {
        isRunning = false
        
        // Flush remaining events
        flushEvents()
        
        executor.shutdown()
        try {
            executor.awaitTermination(5, TimeUnit.SECONDS)
        } catch (e: InterruptedException) {
            executor.shutdownNow()
        }
    }
    
    /**
     * Record SDK initialization
     */
    fun recordInitialization() {
        recordEvent(TelemetryEvent(eventType = EventType.SDK_INIT))
    }
    
    /**
     * Record ad load event
     */
    fun recordAdLoad(placement: String, latency: Long, success: Boolean) {
        val eventType = if (success) EventType.AD_LOADED else EventType.AD_FAILED
        recordEvent(
            TelemetryEvent(
                eventType = eventType,
                placement = placement,
                latency = latency
            )
        )
    }
    
    /**
     * Record timeout event
     */
    fun recordTimeout(placement: String, reason: String) {
        recordEvent(
            TelemetryEvent(
                eventType = EventType.TIMEOUT,
                placement = placement,
                metadata = mapOf("reason" to reason)
            )
        )
    }

    /** Circuit breaker state transition for an adapter (no secrets). */
    fun recordCircuitState(adapter: String, state: String, failureCount: Int) {
        recordEvent(
            TelemetryEvent(
                eventType = EventType.CIRCUIT_OPEN,
                networkName = adapter,
                metadata = mapOf(
                    "state" to state.take(40),
                    "failure_count" to failureCount
                )
            )
        )
    }
    
    /**
     * Record error event
     */
    fun recordError(errorCode: String, error: Throwable) {
        recordEvent(
            TelemetryEvent(
                eventType = EventType.AD_FAILED,
                errorCode = errorCode,
                errorMessage = error.message,
                metadata = mapOf(
                    "stack_trace" to error.stackTraceToString().take(500)
                )
            )
        )
    }
    
    /**
     * Record ANR detection
     */
    fun recordANR(threadName: String, stackTrace: String) {
        recordEvent(
            TelemetryEvent(
                eventType = EventType.ANR_DETECTED,
                metadata = mapOf(
                    "thread" to threadName,
                    "stack_trace" to stackTrace.take(500)
                )
            )
        )
    }

    /** Lightweight status counter for OM SDK availability/config state. */
    fun recordOmSdkStatus(status: String) {
        recordEvent(
            TelemetryEvent(
                eventType = EventType.OMSDK_STATUS,
                metadata = mapOf("status" to status)
            )
        )
    }

    /**
     * Adapter span start (observability). Uses sampling; no secrets allowed.
     */
    fun recordAdapterSpanStart(traceId: String, placement: String, adapter: String, metadata: Map<String, Any> = emptyMap()) {
        if (!config.observabilityEnabled) return
        if (!shouldSample()) return
        val safeMeta = HashMap<String, Any>()
        safeMeta["trace_id"] = traceId
        safeMeta["phase"] = "start"
        for ((k, v) in metadata) {
            if (k.length <= 40) safeMeta[k] = v
        }
        recordEvent(
            TelemetryEvent(
                eventType = EventType.ADAPTER_SPAN_START,
                placement = placement,
                networkName = adapter,
                metadata = safeMeta
            )
        )
    }

    /**
     * Adapter span finish (observability). Outcome taxonomy: fill | no_fill | timeout | error.
     * Includes latency and normalized error code/message when applicable. Uses sampling.
     */
    fun recordAdapterSpanFinish(
        traceId: String,
        placement: String,
        adapter: String,
        outcome: String,
        latencyMs: Long?,
        errorCode: String? = null,
        errorMessage: String? = null,
        metadata: Map<String, Any> = emptyMap()
    ) {
        if (!config.observabilityEnabled) return
        // Update local metrics using the same sampling gate
        recordOutcomeSampled(placement, adapter, outcome, latencyMs)
        if (!shouldSample()) return
        val safeMeta = HashMap<String, Any>()
        safeMeta["trace_id"] = traceId
        safeMeta["phase"] = "finish"
        safeMeta["outcome"] = outcome
        if (latencyMs != null) safeMeta["latency_ms"] = latencyMs
        // Merge limited caller-provided metadata (must be non-sensitive)
        for ((k, v) in metadata) {
            if (k.length <= 40) safeMeta[k] = v
        }
        recordEvent(
            TelemetryEvent(
                eventType = EventType.ADAPTER_SPAN_FINISH,
                placement = placement,
                networkName = adapter,
                latency = latencyMs,
                errorCode = errorCode,
                errorMessage = errorMessage?.take(200),
                metadata = safeMeta
            )
        )
    }

    /** Auction client network latency recorder (dev/observability). Outcome: success | timeout | network_error | status_XXX | error. */
    fun recordAuctionClientLatency(outcome: String, latencyMs: Long) {
        // Reuse local metrics reservoirs under a dedicated key
        recordOutcomeSampled("auction", "client", outcome, latencyMs)
        if (!config.observabilityEnabled || !shouldSample()) return
        val meta = HashMap<String, Any>()
        meta["phase"] = "finish"
        meta["outcome"] = outcome
        meta["latency_ms"] = latencyMs
        recordEvent(
            TelemetryEvent(
                eventType = EventType.ADAPTER_SPAN_FINISH,
                placement = "auction",
                networkName = "client",
                latency = latencyMs,
                metadata = meta
            )
        )
    }

    /**
     * Record a credential validation success for a given network (BYO ValidationMode).
     * Metadata must not contain secrets; only include key names or booleans.
     */
    fun recordCredentialValidationSuccess(network: String, metadata: Map<String, Any> = emptyMap()) {
        recordEvent(
            TelemetryEvent(
                eventType = EventType.CREDENTIAL_VALIDATION_SUCCESS,
                networkName = network,
                metadata = metadata
            )
        )
    }

    /**
     * Record a credential validation failure for a given network (BYO ValidationMode).
     * Provide a normalized error code and optional message. No secrets allowed in metadata.
     */
    fun recordCredentialValidationFailure(network: String, code: String, message: String?, metadata: Map<String, Any> = emptyMap()) {
        recordEvent(
            TelemetryEvent(
                eventType = EventType.CREDENTIAL_VALIDATION_FAILED,
                networkName = network,
                errorCode = code,
                errorMessage = message,
                metadata = metadata
            )
        )
    }
    
    /**
     * Add event to queue
     */
    private fun recordEvent(event: TelemetryEvent) {
        if (!config.telemetryEnabled || !isRunning) {
            return
        }
        
        val safeEvent = TelemetryRedactor.sanitize(event)
        synchronized(queueLock) {
            eventQueue.add(safeEvent)
            
            // Auto-flush if batch size reached
            if (eventQueue.size >= batchSize) {
                executor.execute { flushEvents() }
            }

            // Apply hard cap to queue size to avoid memory growth
            val maxQ = (if (config.observabilityMaxQueue > 0) config.observabilityMaxQueue else 500).coerceAtLeast(100)
            if (eventQueue.size > maxQ) {
                // Drop oldest beyond cap
                eventQueue.subList(0, eventQueue.size - maxQ).clear()
            }
        }
    }
    
    /**
     * Flush events to backend
     */
    private fun flushEvents() {
        val eventsToSend: List<TelemetryEvent>
        
        synchronized(queueLock) {
            if (eventQueue.isEmpty()) {
                return
            }
            
            eventsToSend = eventQueue.toList()
            eventQueue.clear()
        }

        // Partition into adapter-span events (observability) vs general telemetry
        val spans = eventsToSend.filter { it.eventType == EventType.ADAPTER_SPAN_START || it.eventType == EventType.ADAPTER_SPAN_FINISH }
        val others = eventsToSend - spans.toSet()
        try {
            if (spans.isNotEmpty()) sendSpanEvents(spans)
            if (others.isNotEmpty()) sendEvents(others)
        } catch (e: Exception) {
            // Re-queue events on failure
            synchronized(queueLock) {
                eventQueue.addAll(0, eventsToSend)
                // Limit queue size to prevent memory issues (configurable)
                val maxQ = (if (config.observabilityMaxQueue > 0) config.observabilityMaxQueue else 500).coerceAtLeast(100)
                if (eventQueue.size > maxQ) {
                    eventQueue.subList(0, eventQueue.size - maxQ).clear()
                }
            }
        }
    }
    
    /**
     * Send adapter span events to analytics BYO spans endpoint (no gzip; sampled and sanitized).
     */
    private fun sendSpanEvents(events: List<TelemetryEvent>) {
        val base = config.configEndpoint.trimEnd('/')
        val url = "$base/api/v1/analytics/byo/spans"
        val json = gson.toJson(events)
        val request = Request.Builder()
            .url(url)
            .post(json.toRequestBody("application/json".toMediaType()))
            .addHeader("User-Agent", "RivalApexMediation-Android/${BuildConfig.SDK_VERSION}")
            .addHeader("X-App-Id", config.appId)
            .build()
        val response = httpClient.newCall(request).execute()
        if (!response.isSuccessful) {
            throw Exception("Span telemetry upload failed: ${response.code}")
        }
    }

    /**
     * Send general telemetry events to legacy endpoint (compressed)
     */
    private fun sendEvents(events: List<TelemetryEvent>) {
        val payload = mapOf(
            "app_id" to config.appId,
            "sdk_version" to BuildConfig.SDK_VERSION,
            "platform" to "android",
            "events" to events
        )
        
        val json = gson.toJson(payload)
        val compressed = compressGzip(json.toByteArray())
        
        val request = Request.Builder()
            .url("${config.configEndpoint}/v1/telemetry")
            .post(compressed.toRequestBody("application/json".toMediaType()))
            .addHeader("Content-Encoding", "gzip")
            .addHeader("User-Agent", "RivalApexMediation-Android/${BuildConfig.SDK_VERSION}")
            .build()
        
        val response = httpClient.newCall(request).execute()
        
        if (!response.isSuccessful) {
            throw Exception("Telemetry upload failed: ${response.code}")
        }
    }
    
    /**
     * Compress data with GZIP
     */
    private fun compressGzip(data: ByteArray): ByteArray {
        val outputStream = ByteArrayOutputStream()
        GZIPOutputStream(outputStream).use { gzip ->
            gzip.write(data)
        }
        return outputStream.toByteArray()
    }

    private fun shouldSample(): Boolean {
        val p = sampleRate
        if (p >= 1.0) return true
        if (p <= 0.0) return false
        return random.nextDouble() < p
    }
}

/**
 * Performance metrics tracker
 */
class PerformanceMetrics {
    data class Metric(
        val name: String,
        val value: Double,
        val timestamp: Long = ClockProvider.clock.now()
    )
    
    private val metrics = mutableListOf<Metric>()
    
    fun record(name: String, value: Double) {
        synchronized(metrics) {
            metrics.add(Metric(name, value))
            
            // Keep only last 100 metrics
            if (metrics.size > 100) {
                metrics.removeAt(0)
            }
        }
    }
    
    fun getMetrics(): List<Metric> {
        return synchronized(metrics) {
            metrics.toList()
        }
    }
    
    fun clear() {
        synchronized(metrics) {
            metrics.clear()
        }
    }
}
