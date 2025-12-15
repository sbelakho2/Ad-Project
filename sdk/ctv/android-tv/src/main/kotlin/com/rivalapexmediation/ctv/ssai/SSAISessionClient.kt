package com.rivalapexmediation.ctv.ssai

import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.*
import org.json.JSONObject
import java.io.OutputStreamWriter
import java.net.HttpURLConnection
import java.net.URL
import java.text.SimpleDateFormat
import java.util.*
import kotlin.math.pow

/**
 * SSAI Session Client for Android TV SDK
 *
 * Manages SSAI session lifecycle including creation, tracking, and cleanup.
 */
class SSAISessionClient(
    private val config: Config = Config()
) {
    /**
     * Client configuration
     */
    data class Config(
        /** Request timeout (milliseconds) */
        val requestTimeout: Long = 5000L,
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

    // State
    private var ssaiConfig: SSAIConfig? = null
    private var session: SSAISession? = null
    private var state: SSAISessionState = SSAISessionState.IDLE
    
    private val eventQueue = mutableListOf<SSAITrackingEvent>()
    private val firedEvents = mutableSetOf<String>()
    private var metrics = SSAIMetrics()
    
    private val mainHandler = Handler(Looper.getMainLooper())
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private var batchJob: Job? = null
    
    private val dateFormat = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", Locale.US).apply {
        timeZone = TimeZone.getTimeZone("UTC")
    }

    // Public accessors
    val sessionState: SSAISessionState get() = state
    val currentSession: SSAISession? get() = session
    val currentMetrics: SSAIMetrics get() = metrics
    val isSessionValid: Boolean get() = session?.isValid ?: false

    /**
     * Create a new SSAI session
     */
    fun createSession(
        config: SSAIConfig,
        callback: (Result<SSAISession>) -> Unit
    ) {
        state = SSAISessionState.INITIALIZING
        ssaiConfig = config

        scope.launch {
            try {
                val url = URL("${config.endpoint}/v1/ssai/session")
                val result = fetchWithRetry(url, "POST", buildSessionBody(config), 0)
                
                result.fold(
                    onSuccess = { data ->
                        val sessionResponse = parseSession(data)
                        session = sessionResponse
                        state = SSAISessionState.ACTIVE
                        startBatching()
                        mainHandler.post { callback(Result.success(sessionResponse)) }
                    },
                    onFailure = { error ->
                        state = SSAISessionState.ERROR
                        val ssaiError = when (error) {
                            is SSAIError -> error
                            else -> SSAIError("SESSION_ERROR", error.message ?: "Unknown error", true)
                        }
                        mainHandler.post { callback(Result.failure(ssaiError)) }
                    }
                )
            } catch (e: Exception) {
                state = SSAISessionState.ERROR
                val ssaiError = SSAIError("SESSION_ERROR", e.message ?: "Unknown error", true)
                mainHandler.post { callback(Result.failure(ssaiError)) }
            }
        }
    }

    /**
     * Create session (suspend function)
     */
    suspend fun createSession(config: SSAIConfig): SSAISession = suspendCancellableCoroutine { cont ->
        createSession(config) { result ->
            result.fold(
                onSuccess = { cont.resume(it) {} },
                onFailure = { cont.cancel(it as? Exception ?: Exception(it.message)) }
            )
        }
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
            synchronized(eventQueue) {
                eventQueue.add(event)
                if (eventQueue.size >= config.maxBatchSize) {
                    flushEvents()
                }
            }
        } else {
            sendEvent(event)
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
     * Track progress quartile
     */
    fun trackProgress(podId: String, slotId: String, creativeId: String, mediaTime: Double, progress: Double) {
        val eventType = when {
            progress >= 100 -> SSAITrackingEventType.COMPLETE
            progress >= 75 -> SSAITrackingEventType.THIRD_QUARTILE
            progress >= 50 -> SSAITrackingEventType.MIDPOINT
            progress >= 25 -> SSAITrackingEventType.FIRST_QUARTILE
            progress >= 0 -> SSAITrackingEventType.START
            else -> return
        }

        trackEvent(SSAITrackingEvent(
            type = eventType,
            podId = podId,
            slotId = slotId,
            creativeId = creativeId,
            mediaTime = mediaTime,
            metadata = mapOf("progress" to String.format("%.1f", progress))
        ))
    }

    /**
     * Track error
     */
    fun trackError(podId: String, slotId: String, creativeId: String, errorCode: String, errorMessage: String) {
        trackEvent(SSAITrackingEvent(
            type = SSAITrackingEventType.ERROR,
            podId = podId,
            slotId = slotId,
            creativeId = creativeId,
            mediaTime = 0.0,
            metadata = mapOf("errorCode" to errorCode, "errorMessage" to errorMessage)
        ))
    }

    /**
     * Track skip
     */
    fun trackSkip(podId: String, slotId: String, creativeId: String, mediaTime: Double) {
        trackEvent(SSAITrackingEvent(
            type = SSAITrackingEventType.SKIP,
            podId = podId,
            slotId = slotId,
            creativeId = creativeId,
            mediaTime = mediaTime
        ))
    }

    /**
     * Flush pending events
     */
    fun flushEvents() {
        val events: List<SSAITrackingEvent>
        synchronized(eventQueue) {
            if (eventQueue.isEmpty()) return
            events = eventQueue.take(config.maxBatchSize).toList()
            eventQueue.subList(0, minOf(config.maxBatchSize, eventQueue.size)).clear()
        }
        sendEventBatch(events)
    }

    /**
     * End the session
     */
    fun endSession(callback: ((Boolean) -> Unit)? = null) {
        flushEvents()
        stopBatching()

        val currentSession = session
        val currentConfig = ssaiConfig

        if (currentSession == null || currentConfig == null) {
            callback?.invoke(true)
            return
        }

        scope.launch {
            try {
                val url = URL("${currentConfig.endpoint}/v1/ssai/session/${currentSession.sessionId}/end")
                val connection = url.openConnection() as HttpURLConnection
                connection.requestMethod = "POST"
                connection.connect()
                connection.responseCode // Trigger the request
                connection.disconnect()
            } catch (_: Exception) {
                // Best effort
            }

            session = null
            state = SSAISessionState.IDLE
            firedEvents.clear()
            resetMetrics()
            mainHandler.post { callback?.invoke(true) }
        }
    }

    // Private helpers

    private suspend fun fetchWithRetry(
        url: URL,
        method: String,
        body: String?,
        retryCount: Int
    ): Result<String> {
        return try {
            val connection = url.openConnection() as HttpURLConnection
            connection.requestMethod = method
            connection.connectTimeout = config.requestTimeout.toInt()
            connection.readTimeout = config.requestTimeout.toInt()
            connection.setRequestProperty("Content-Type", "application/json")

            if (body != null && method == "POST") {
                connection.doOutput = true
                OutputStreamWriter(connection.outputStream).use { it.write(body) }
            }

            val responseCode = connection.responseCode
            if (responseCode in 200..299) {
                val response = connection.inputStream.bufferedReader().readText()
                connection.disconnect()
                Result.success(response)
            } else {
                connection.disconnect()
                if (retryCount < config.maxRetries) {
                    delay(config.retryBackoffMs * 2.0.pow(retryCount).toLong())
                    fetchWithRetry(url, method, body, retryCount + 1)
                } else {
                    Result.failure(SSAIError("HTTP_$responseCode", "HTTP error", true))
                }
            }
        } catch (e: Exception) {
            if (retryCount < config.maxRetries) {
                delay(config.retryBackoffMs * 2.0.pow(retryCount).toLong())
                fetchWithRetry(url, method, body, retryCount + 1)
            } else {
                Result.failure(SSAIError("NETWORK_ERROR", e.message ?: "Network error", true))
            }
        }
    }

    private fun buildSessionBody(config: SSAIConfig): String {
        val json = JSONObject().apply {
            put("tenantId", config.tenantId)
            put("contentUrl", config.contentUrl)
            put("format", config.format.value)
            config.capabilities?.let { caps ->
                put("capabilities", JSONObject().apply {
                    caps.maxResolution?.let { res ->
                        put("maxResolution", JSONObject().apply {
                            put("width", res.width)
                            put("height", res.height)
                        })
                    }
                    caps.codecs?.let { put("codecs", it) }
                    put("supportsHdr", caps.supportsHdr)
                    put("supportsDolby", caps.supportsDolby)
                    caps.maxBitrate?.let { put("maxBitrate", it) }
                })
            }
        }
        return json.toString()
    }

    private fun parseSession(json: String): SSAISession {
        val obj = JSONObject(json)
        return SSAISession(
            sessionId = obj.getString("sessionId"),
            manifestUrl = obj.getString("manifestUrl"),
            trackingUrl = obj.getString("trackingUrl"),
            expiresAt = dateFormat.parse(obj.getString("expiresAt")) ?: Date(),
            sessionToken = obj.getString("sessionToken"),
            adBreaks = parseAdBreaks(obj.optJSONArray("adBreaks"))
        )
    }

    private fun parseAdBreaks(jsonArray: org.json.JSONArray?): List<SSAIAdBreak> {
        if (jsonArray == null) return emptyList()
        return (0 until jsonArray.length()).map { i ->
            val obj = jsonArray.getJSONObject(i)
            SSAIAdBreak(
                podId = obj.getString("podId"),
                type = obj.getString("type"),
                position = obj.getInt("position"),
                startTime = obj.getDouble("startTime"),
                duration = obj.getDouble("duration"),
                plannedSlots = obj.getInt("plannedSlots"),
                filledSlots = obj.getInt("filledSlots"),
                ads = parseAds(obj.optJSONArray("ads"))
            )
        }
    }

    private fun parseAds(jsonArray: org.json.JSONArray?): List<SSAIAd> {
        if (jsonArray == null) return emptyList()
        return (0 until jsonArray.length()).map { i ->
            val obj = jsonArray.getJSONObject(i)
            SSAIAd(
                slotId = obj.getString("slotId"),
                creativeId = obj.getString("creativeId"),
                duration = obj.getDouble("duration"),
                startOffset = obj.getDouble("startOffset"),
                skipOffset = if (obj.has("skipOffset")) obj.getDouble("skipOffset") else null,
                vastUrl = obj.optString("vastUrl", null),
                verificationScripts = emptyList()
            )
        }
    }

    private fun startBatching() {
        if (!config.enableBatching) return
        batchJob = scope.launch {
            while (isActive) {
                delay(config.batchIntervalMs)
                flushEvents()
            }
        }
    }

    private fun stopBatching() {
        batchJob?.cancel()
        batchJob = null
    }

    private fun sendEvent(event: SSAITrackingEvent) {
        val session = this.session ?: return
        
        scope.launch {
            try {
                val url = URL("${session.trackingUrl}/event")
                val connection = url.openConnection() as HttpURLConnection
                connection.requestMethod = "POST"
                connection.setRequestProperty("Content-Type", "application/json")
                connection.setRequestProperty("X-Session-Token", session.sessionToken)
                connection.doOutput = true

                val body = eventToJson(event)
                OutputStreamWriter(connection.outputStream).use { it.write(body) }
                connection.responseCode
                connection.disconnect()
            } catch (_: Exception) {
                // Fire and forget
            }
        }
    }

    private fun sendEventBatch(events: List<SSAITrackingEvent>) {
        val session = this.session ?: return
        if (events.isEmpty()) return

        scope.launch {
            try {
                val url = URL("${session.trackingUrl}/events")
                val connection = url.openConnection() as HttpURLConnection
                connection.requestMethod = "POST"
                connection.setRequestProperty("Content-Type", "application/json")
                connection.setRequestProperty("X-Session-Token", session.sessionToken)
                connection.doOutput = true

                val body = JSONObject().apply {
                    put("events", events.map { JSONObject(eventToJson(it)) })
                }.toString()

                OutputStreamWriter(connection.outputStream).use { it.write(body) }
                connection.responseCode
                connection.disconnect()
            } catch (_: Exception) {
                // Fire and forget
            }
        }
    }

    private fun eventToJson(event: SSAITrackingEvent): String {
        return JSONObject().apply {
            put("type", event.type.value)
            put("podId", event.podId)
            put("slotId", event.slotId)
            put("creativeId", event.creativeId)
            put("mediaTime", event.mediaTime)
            put("timestamp", dateFormat.format(event.timestamp))
            if (event.metadata.isNotEmpty()) {
                put("metadata", JSONObject(event.metadata))
            }
        }.toString()
    }

    private fun shouldDedupe(eventType: SSAITrackingEventType): Boolean {
        return eventType in listOf(
            SSAITrackingEventType.IMPRESSION,
            SSAITrackingEventType.START,
            SSAITrackingEventType.FIRST_QUARTILE,
            SSAITrackingEventType.MIDPOINT,
            SSAITrackingEventType.THIRD_QUARTILE,
            SSAITrackingEventType.COMPLETE
        )
    }

    private fun updateMetrics(eventType: SSAITrackingEventType) {
        when (eventType) {
            SSAITrackingEventType.IMPRESSION -> metrics.adCount++
            SSAITrackingEventType.COMPLETE -> metrics.completedAds++
            SSAITrackingEventType.SKIP -> metrics.skippedAds++
            SSAITrackingEventType.ERROR -> metrics.erroredAds++
            else -> {}
        }
    }

    private fun resetMetrics() {
        metrics = SSAIMetrics()
    }
}
