package com.rivalapexmediation.ctv.render

import com.rivalapexmediation.ctv.metrics.MetricsRecorder
import okhttp3.OkHttpClient
import okhttp3.Request
import java.util.concurrent.TimeUnit

internal object Beacon {
    private val client: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(2000, TimeUnit.MILLISECONDS)
        .readTimeout(2000, TimeUnit.MILLISECONDS)
        .build()

    fun fire(url: String?, eventName: String? = null) {
        if (url.isNullOrBlank()) return
        try {
            val req = Request.Builder().url(url).get().build()
            client.newCall(req).enqueue(object: okhttp3.Callback {
                override fun onFailure(call: okhttp3.Call, e: java.io.IOException) {
                    eventName?.let { MetricsRecorder.recordTracker(it, false) }
                }
                override fun onResponse(call: okhttp3.Call, response: okhttp3.Response) {
                    eventName?.let { MetricsRecorder.recordTracker(it, true) }
                    response.close()
                }
            })
        } catch (_: Throwable) {
            eventName?.let { MetricsRecorder.recordTracker(it, false) }
        }
    }
}
