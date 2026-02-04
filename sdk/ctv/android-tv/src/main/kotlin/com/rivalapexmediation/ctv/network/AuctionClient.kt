package com.rivalapexmediation.ctv.network

import android.content.Context
import android.os.Build
import android.os.SystemClock
import com.google.gson.Gson
import com.google.gson.reflect.TypeToken
import com.rivalapexmediation.ctv.ApexMediation
import com.rivalapexmediation.ctv.SDKConfig
import com.rivalapexmediation.ctv.consent.ConsentData
import com.rivalapexmediation.ctv.metrics.MetricsRecorder
import com.rivalapexmediation.ctv.util.Logger
import okhttp3.Call
import okhttp3.Callback
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import java.io.IOException
import java.net.SocketTimeoutException
import java.util.*
import java.util.concurrent.TimeUnit

class AuctionClient(
    private val context: Context,
    private val config: SDKConfig,
    private val gson: Gson = Gson(),
) {
    private val client: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(config.requestTimeoutMs.toLong(), TimeUnit.MILLISECONDS)
        .readTimeout(config.requestTimeoutMs.toLong(), TimeUnit.MILLISECONDS)
        .callTimeout((config.requestTimeoutMs * 2L), TimeUnit.MILLISECONDS)
        .build()

    data class Result(
        val win: AuctionWin?,
        val error: LoadError? = null,
    )

    fun requestBid(
        placementId: String,
        adFormat: String,
        floorCpm: Double,
        consent: ConsentData?,
        callback: (Result) -> Unit,
    ) {
        val requestId = UUID.randomUUID().toString()
        val req = AuctionRequest(
            requestId = requestId,
            placementId = placementId,
            adFormat = adFormat,
            floorCpm = floorCpm,
            device = buildDevice(),
            user = buildUser(),
            app = buildApp(),
            consent = buildConsentMap(consent),
            signal = null,
        )
        val body = gson.toJson(mapOf("body" to req)).toRequestBody("application/json".toMediaType())
        val url = config.apiBaseUrl.trimEnd('/') + "/rtb/bid"
        // Generate correlation ID for distributed tracing
        val correlationId = UUID.randomUUID().toString()
        val builder = Request.Builder().url(url).post(body)
            .header("Content-Type", "application/json")
            .header("X-Correlation-ID", correlationId)
            .header("X-SDK-Version", "ctv-android-tv-sdk/1.0.0")
        config.apiKey?.let { builder.header("Authorization", "Bearer $it") }
        val httpReq = builder.build()

        val start = SystemClock.elapsedRealtime()
        client.newCall(httpReq).enqueue(object: Callback {
            override fun onFailure(call: Call, e: IOException) {
                Logger.w("Auction HTTP error: ${e.message}", e)
                val err = if (e is SocketTimeoutException) LoadError.Timeout else LoadError.Network
                ApexMediation.recordSloSample(false)
                MetricsRecorder.recordRequest(SystemClock.elapsedRealtime() - start, err)
                callback(Result(null, err))
            }
            override fun onResponse(call: Call, response: Response) {
                response.use { res ->
                    try {
                        if (res.code == 204) {
                            ApexMediation.recordSloSample(true)
                            MetricsRecorder.recordRequest(SystemClock.elapsedRealtime() - start, LoadError.NoFill)
                            callback(Result(null, LoadError.NoFill)); return
                        }
                        if (!res.isSuccessful) {
                            // Map simple taxonomy
                            val err = mapStatusError(res.code)
                            ApexMediation.recordSloSample(false)
                            MetricsRecorder.recordRequest(SystemClock.elapsedRealtime() - start, err)
                            callback(Result(null, err)); return
                        }
                        val bodyStr = res.body?.string() ?: "{}"
                        val mapType = object: TypeToken<Map<String, Any>>(){}.type
                        val parsed: Map<String, Any> = try { gson.fromJson(bodyStr, mapType) } catch (_: Exception) { emptyMap() }
                        val winJson = when {
                            parsed.containsKey("response") -> gson.toJsonTree(parsed["response"]).toString()
                            else -> bodyStr
                        }
                        val win = gson.fromJson(winJson, AuctionWin::class.java)
                        val loadError = if (win?.creativeUrl.isNullOrBlank()) LoadError.NoFill else null
                        ApexMediation.recordSloSample(true)
                        MetricsRecorder.recordRequest(SystemClock.elapsedRealtime() - start, loadError)
                        if (loadError == null) {
                            callback(Result(win, null))
                        } else {
                            callback(Result(null, loadError))
                        }
                    } catch (e: Exception) {
                        Logger.w("Auction parse error: ${e.message}", e)
                        ApexMediation.recordSloSample(false)
                        val err = LoadError.Generic("parse_error")
                        MetricsRecorder.recordRequest(SystemClock.elapsedRealtime() - start, err)
                        callback(Result(null, err))
                    }
                }
            }
        })
    }

    private fun buildDevice(): Map<String, Any?> = mapOf(
        "platform" to "android",
        "osVersion" to Build.VERSION.RELEASE,
        "model" to Build.MODEL,
        "tv" to true,
    )

    private fun buildUser(): Map<String, Any?> = emptyMap()

    private fun buildApp(): Map<String, Any?> = mapOf(
        "id" to config.appId,
        "name" to context.applicationInfo.loadLabel(context.packageManager).toString(),
        "bundle" to context.packageName,
        "version" to try { context.packageManager.getPackageInfo(context.packageName, 0).versionName } catch (_: Exception) { null }
    )

    private fun mapStatusError(code: Int): LoadError {
        return when {
            code == 408 || code == 504 -> LoadError.Timeout
            code == 429 -> LoadError.Status("status_429", code)
            code == 401 -> LoadError.Status("status_401", code)
            code in 400..499 -> LoadError.Status("status_4xx", code)
            code in 500..599 -> LoadError.Status("status_5xx", code)
            else -> LoadError.Status("status_${code}", code)
        }
    }
}
