package com.rivalapexmediation.sdk.ssai

import com.google.gson.Gson
import com.google.gson.GsonBuilder
import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.ClockProvider
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.text.SimpleDateFormat
import java.util.*
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * SSAI Session Client for Android Core SDK
 *
 * Manages SSAI session lifecycle including creation, tracking, and cleanup.
 * Thread-safe implementation using concurrent collections and atomic references.
 */
class SSAISessionClient(
    config: Config = Config(),
    httpClient: OkHttpClient? = null,
    private val clock: Clock = ClockProvider.clock
) {
    /**
     * Client configuration
     */
    data class Config(
        /** Request timeout (milliseconds) */
        val requestTimeoutMs: Long = 5000L,
        /** Maximum retry attempts */
        val maxRetries: Int = 2,
        /** Retry backoff base (milliseconds) */
        val retryBackoffMs: Long = 100L,
        /** Enable event batching */
        val enableBatching: Boolean = true,
        /** Batch interval (milliseconds) */
        val batchIntervalMs: Long = 1000L,
        /** Maximum events per batch */
        val maxBatchSize: Int = 10
    )

    private val config: Config = config
    private val gson: Gson = GsonBuilder()
        .setDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSSZ")
        .create()

    private val httpClient: OkHttpClient = (httpClient ?: OkHttpClient()).newBuilder()
        .connectTimeout(config.requestTimeoutMs, TimeUnit.MILLISECONDS)
        .readTimeout(config.requestTimeoutMs, TimeUnit.MILLISECONDS)
        .writeTimeout(config.requestTimeoutMs, TimeUnit.MILLISECONDS)
        .build()

    private val state = AtomicReference(SSAISessionState.IDLE)
    private var ssaiConfig: SSAIConfig? = null
    private var session: SSAISession? = null
    private val eventQueue = ConcurrentHashMap.newKeySet<SSAITrackingEvent>()
    private val firedEvents = ConcurrentHashMap.newKeySet<String>()
    private val metrics = SSAIMetrics()
    
    private val executor = Executors.newSingleThreadScheduledExecutor { r ->
        Thread(r, "SSAI-BatchDispatcher").apply { isDaemon = true }
    }
    private var batchFuture: ScheduledFuture<*>? = null

    // Public accessors
    val sessionState: SSAISessionState get() = state.get()
    val currentSession: SSAISession? get() = session
    val currentMetrics: SSAIMetrics get() = metrics.copy()

    /**
     * Create a new SSAI session
     *
     * @param config SSAI configuration
     * @return Created session
     * @throws SSAIError on failure
     */
    @Throws(SSAIError::class)
    fun createSession(config: SSAIConfig): SSAISession {
        state.set(SSAISessionState.INITIALIZING)
        ssaiConfig = config

        val url = "${config.endpoint.trimEnd('/')}/v1/ssai/session"

        val bodyMap = mutableMapOf<String, Any>(
            "tenantId" to config.tenantId,
            "contentUrl" to config.contentUrl,
            "format" to config.format.value
        )

        config.capabilities?.let { caps ->
            val capsMap = mutableMapOf<String, Any>(
                "supportsHdr" to caps.supportsHdr,
                "supportsDolby" to caps.supportsDolby,
                "deviceType" to caps.deviceType.value
            )
            caps.maxResolution?.let {
                capsMap["maxResolution"] = mapOf("width" to it.width, "height" to it.height)
            }
            caps.codecs?.let { capsMap["codecs"] = it }
            caps.maxBitrate?.let { capsMap["maxBitrate"] = it }
            bodyMap["capabilities"] = capsMap
        }

        config.consent?.let { consent ->
            val consentMap = mutableMapOf<String, Any>("coppa" to consent.coppa)
            consent.gdprApplies?.let { consentMap["gdprApplies"] = it }
            consent.tcfConsent?.let { consentMap["tcfConsent"] = it }
            consent.usPrivacy?.let { consentMap["usPrivacy"] = it }
            consent.gpp?.let { consentMap["gpp"] = it }
            bodyMap["consent"] = consentMap
        }

        val request = Request.Builder()
            .url(url)
            .post(gson.toJson(bodyMap).toRequestBody("application/json".toMediaType()))
            .addHeader("User-Agent", "RivalApexMediation-Android/1.0.0")
            .addHeader("Content-Type", "application/json")
            .build()

        val response = executeWithRetry(request)
        
        val sessionResponse = try {
            gson.fromJson(response, SessionResponse::class.java)
        } catch (e: Exception) {
            state.set(SSAISessionState.ERROR)
            throw SSAIError("DECODE_ERROR", "Failed to decode session response: ${e.message}", fatal = true)
        }

        val createdSession = SSAISession(
            sessionId = sessionResponse.sessionId,
            manifestUrl = sessionResponse.manifestUrl,
            trackingUrl = sessionResponse.trackingUrl,
            expiresAt = sessionResponse.expiresAt,
            sessionToken = sessionResponse.sessionToken,
            contentUrl = config.contentUrl,
            adBreaks = sessionResponse.adBreaks ?: emptyList()
        )

        session = createdSession
        state.set(SSAISessionState.ACTIVE)
        startBatching()

        return createdSession
    }

    /**
     * End the current SSAI session
     */
    fun endSession() {
        flushEvents()
        
        batchFuture?.cancel(false)
        batchFuture = null
        
        session = null
        state.set(SSAISessionState.COMPLETED)
        eventQueue.clear()
        firedEvents.clear()
    }

    /**
     * Track a SSAI event
     */
    fun trackEvent(event: SSAITrackingEvent) {
        if (session == null) {
            if (ssaiConfig?.debug == true) {
                println("[SSAI] Cannot track event without active session")
            }
            return
        }

        // Deduplicate
        val eventKey = "${event.type.value}:${event.slotId}"
        if (shouldDedupe(event.type) && firedEvents.contains(eventKey)) {
            return
        }

        if (config.enableBatching) {
            eventQueue.add(event)
            if (eventQueue.size >= config.maxBatchSize) {
                flushEvents()
            }
        } else {
            sendEvents(listOf(event))
        }

        // Mark as fired
        if (shouldDedupe(event.type)) {
            firedEvents.add(eventKey)
        }

        // Update metrics
        updateMetrics(event.type)
    }

    /**
     * Track impression
     */
    fun trackImpression(podId: String, slotId: String, creativeId: String, mediaTime: Double) {
        trackEvent(SSAITrackingEvent(
            type = SSAITrackingEventType.IMPRESSION,
            podId = podId,
            slotId = slotId,
            creativeId = creativeId,
            mediaTime = mediaTime
        ))
    }

    /**
     * Track quartile progress
     */
    fun trackProgress(
        podId: String,
        slotId: String,
        creativeId: String,
        progress: Double,
        mediaTime: Double
    ) {
        val eventType = when {
            progress < 0.01 -> SSAITrackingEventType.START
            progress in 0.24..0.26 -> SSAITrackingEventType.FIRST_QUARTILE
            progress in 0.49..0.51 -> SSAITrackingEventType.MIDPOINT
            progress in 0.74..0.76 -> SSAITrackingEventType.THIRD_QUARTILE
            progress >= 0.99 -> SSAITrackingEventType.COMPLETE
            else -> return // Don't track intermediate progress
        }

        trackEvent(SSAITrackingEvent(
            type = eventType,
            podId = podId,
            slotId = slotId,
            creativeId = creativeId,
            mediaTime = mediaTime
        ))
    }

    /**
     * Track error
     */
    fun trackError(podId: String, slotId: String, creativeId: String, mediaTime: Double, errorMessage: String) {
        trackEvent(SSAITrackingEvent(
            type = SSAITrackingEventType.ERROR,
            podId = podId,
            slotId = slotId,
            creativeId = creativeId,
            mediaTime = mediaTime,
            metadata = mapOf("error" to errorMessage)
        ))
    }

    // Private methods

    private fun startBatching() {
        if (!config.enableBatching) return

        batchFuture?.cancel(false)
        batchFuture = executor.scheduleAtFixedRate(
            { flushEvents() },
            config.batchIntervalMs,
            config.batchIntervalMs,
            TimeUnit.MILLISECONDS
        )
    }

    private fun flushEvents() {
        if (eventQueue.isEmpty()) return

        val events = eventQueue.toList()
        eventQueue.clear()
        sendEvents(events)
    }

    private fun sendEvents(events: List<SSAITrackingEvent>) {
        val currentSession = session ?: return

        val url = "${currentSession.trackingUrl}/batch"

        val bodyMap = mapOf(
            "sessionId" to currentSession.sessionId,
            "events" to events.map { eventToMap(it) }
        )

        val request = Request.Builder()
            .url(url)
            .post(gson.toJson(bodyMap).toRequestBody("application/json".toMediaType()))
            .addHeader("Content-Type", "application/json")
            .addHeader("Authorization", "Bearer ${currentSession.sessionToken}")
            .build()

        try {
            httpClient.newCall(request).execute().use { response ->
                if (!response.isSuccessful && ssaiConfig?.debug == true) {
                    println("[SSAI] Failed to send tracking events: ${response.code}")
                }
            }
        } catch (e: Exception) {
            if (ssaiConfig?.debug == true) {
                println("[SSAI] Failed to send tracking events: ${e.message}")
            }
        }
    }

    private fun executeWithRetry(request: Request, attempt: Int = 0): String {
        try {
            httpClient.newCall(request).execute().use { response ->
                if (response.isSuccessful) {
                    return response.body?.string()
                        ?: throw SSAIError("EMPTY_RESPONSE", "Empty response body", fatal = true)
                }

                val isFatal = response.code in 400..499
                if (!isFatal && attempt < config.maxRetries) {
                    val delay = config.retryBackoffMs * (1 shl attempt)
                    Thread.sleep(delay)
                    return executeWithRetry(request, attempt + 1)
                }

                state.set(SSAISessionState.ERROR)
                throw SSAIError(
                    "HTTP_${response.code}",
                    "Request failed with status ${response.code}",
                    fatal = isFatal
                )
            }
        } catch (e: SSAIError) {
            throw e
        } catch (e: Exception) {
            if (attempt < config.maxRetries) {
                val delay = config.retryBackoffMs * (1 shl attempt)
                Thread.sleep(delay)
                return executeWithRetry(request, attempt + 1)
            }
            state.set(SSAISessionState.ERROR)
            throw SSAIError("NETWORK_ERROR", "Network request failed: ${e.message}", fatal = false)
        }
    }

    private fun shouldDedupe(eventType: SSAITrackingEventType): Boolean {
        return when (eventType) {
            SSAITrackingEventType.IMPRESSION,
            SSAITrackingEventType.START,
            SSAITrackingEventType.FIRST_QUARTILE,
            SSAITrackingEventType.MIDPOINT,
            SSAITrackingEventType.THIRD_QUARTILE,
            SSAITrackingEventType.COMPLETE -> true
            else -> false
        }
    }

    private fun updateMetrics(eventType: SSAITrackingEventType) {
        synchronized(metrics) {
            when (eventType) {
                SSAITrackingEventType.IMPRESSION -> metrics.adCount++
                SSAITrackingEventType.COMPLETE -> metrics.completedAds++
                SSAITrackingEventType.SKIP -> metrics.skippedAds++
                SSAITrackingEventType.ERROR -> metrics.erroredAds++
                else -> {}
            }
        }
    }

    private fun eventToMap(event: SSAITrackingEvent): Map<String, Any> {
        val dateFormat = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSSZ", Locale.US)
        val map = mutableMapOf<String, Any>(
            "type" to event.type.value,
            "podId" to event.podId,
            "slotId" to event.slotId,
            "creativeId" to event.creativeId,
            "mediaTime" to event.mediaTime,
            "timestamp" to dateFormat.format(event.timestamp)
        )
        if (event.metadata.isNotEmpty()) {
            map["metadata"] = event.metadata
        }
        return map
    }

    // Internal response types
    private data class SessionResponse(
        val sessionId: String,
        val manifestUrl: String,
        val trackingUrl: String,
        val expiresAt: Date,
        val sessionToken: String,
        val adBreaks: List<SSAIAdBreak>?
    )
}
