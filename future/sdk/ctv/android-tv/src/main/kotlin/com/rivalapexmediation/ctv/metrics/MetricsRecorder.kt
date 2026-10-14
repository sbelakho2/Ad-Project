package com.rivalapexmediation.ctv.metrics

import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import com.rivalapexmediation.ctv.ApexMediation
import com.rivalapexmediation.ctv.SDKConfig
import com.rivalapexmediation.ctv.network.LoadError
import com.rivalapexmediation.ctv.network.reason
import com.rivalapexmediation.ctv.render.VideoProgressEvent
import com.rivalapexmediation.ctv.util.Logger
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.util.Collections
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap

internal object MetricsRecorder {
    private const val FLUSH_INTERVAL_MS = 30_000L
    private const val MAX_LATENCY_SAMPLES = 200
    private val handler = Handler(Looper.getMainLooper())
    private val counters = ConcurrentHashMap<String, Long>()
    private val latencies = Collections.synchronizedList(mutableListOf<Long>())
    private val client = OkHttpClient.Builder().build()
    private var initialized = false
    private lateinit var cfg: SDKConfig
    private var flushScheduled = false
    private val flushRunnable = Runnable {
        flush()
        flushScheduled = false
    }

    fun initialize(config: SDKConfig) {
        cfg = config
        initialized = true
    }

    fun recordRequest(durationMs: Long, error: LoadError?) {
        if (!shouldCollect()) return
        increment("requests_total")
        val success = error == null || error is LoadError.NoFill
        if (success) increment("requests_success") else increment("requests_error")
        val reasonKey = error?.reason()
        if (!reasonKey.isNullOrBlank()) increment("requests_error_$reasonKey")
        addLatency(durationMs)
        scheduleFlush()
    }

    fun recordPlayback(event: VideoProgressEvent) {
        if (!shouldCollect()) return
        val key = "playback_${event.name.lowercase(Locale.ROOT)}"
        increment(key)
        scheduleFlush()
    }

    fun recordTracker(event: String, success: Boolean) {
        if (!shouldCollect() || event.isEmpty()) return
        val key = "tracker_${event.lowercase(Locale.ROOT)}_${if (success) "success" else "failure"}"
        increment(key)
        scheduleFlush()
    }

    private fun addLatency(durationMs: Long) {
        if (durationMs <= 0) return
        synchronized(latencies) {
            if (latencies.size >= MAX_LATENCY_SAMPLES) {
                latencies.removeAt(0)
            }
            latencies.add(durationMs)
        }
    }

    private fun increment(key: String, delta: Long = 1) {
        counters.merge(key, delta) { old, add -> old + add }
    }

    private fun scheduleFlush() {
        if (flushScheduled) return
        flushScheduled = true
        handler.postDelayed(flushRunnable, FLUSH_INTERVAL_MS)
    }

    private fun flush() {
        if (!shouldCollect()) {
            counters.clear()
            synchronized(latencies) { latencies.clear() }
            return
        }
        val snapshot = buildPayload() ?: return
        val body = snapshot.toString().toRequestBody("application/json".toMediaType())
        val url = cfg.apiBaseUrl.trimEnd('/') + "/sdk/metrics"
        val builder = Request.Builder().url(url).post(body).header("Content-Type", "application/json")
        cfg.apiKey?.let { builder.header("Authorization", "Bearer $it") }
        client.newCall(builder.build()).enqueue(object: okhttp3.Callback {
            override fun onFailure(call: okhttp3.Call, e: java.io.IOException) {
                Logger.w("CTV metrics post failed: ${e.message}", e)
            }
            override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) {
                response.close()
            }
        })
        counters.clear()
        synchronized(latencies) { latencies.clear() }
    }

    private fun buildPayload(): JSONObject? {
        if (counters.isEmpty() && latencies.isEmpty()) return null
        val root = JSONObject()
        val countersJson = JSONObject()
        counters.forEach { (key, value) -> countersJson.put(key, value) }
        root.put("counters", countersJson)
        if (latencies.isNotEmpty()) {
            val sorted = synchronized(latencies) { latencies.sorted() }
            val percentiles = JSONObject()
            percentiles.put("p50", percentile(sorted, 0.50))
            percentiles.put("p95", percentile(sorted, 0.95))
            percentiles.put("p99", percentile(sorted, 0.99))
            root.put("request_latency_ms", percentiles)
        }
        root.put("timestamp", System.currentTimeMillis())
        root.put("sdk", "android_tv")
        root.put("appId", cfg.appId)
        return root
    }

    private fun percentile(values: List<Long>, fraction: Double): Long {
        if (values.isEmpty()) return 0
        val index = ((values.size - 1) * fraction).toInt().coerceIn(0, values.size - 1)
        return values[index]
    }

    private fun shouldCollect(): Boolean = initialized && ApexMediation.metricsEnabled()
}
