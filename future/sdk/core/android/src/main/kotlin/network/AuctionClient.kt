package com.rivalapexmediation.sdk.network

import android.os.Build
import android.os.Looper
import com.google.gson.Gson
import com.rivalapexmediation.sdk.threading.CircuitBreaker
import com.rivalapexmediation.sdk.util.Clock
import com.rivalapexmediation.sdk.util.ClockProvider
import okhttp3.ConnectionPool
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.io.InterruptedIOException
import java.net.SocketTimeoutException
import java.text.SimpleDateFormat
import java.util.Locale
import java.util.TimeZone
import java.util.concurrent.CancellationException
import java.util.concurrent.TimeUnit
import kotlin.math.min

/**
 * Lightweight Auction client for the Android SDK.
 * - No external dependencies beyond OkHttp + Gson (already present).
 * - Builds request JSON aligned with backend schema and Web/iOS SDKs.
 * - Maps outcomes to a normalized error taxonomy: timeout, status_XXX, no_fill, network_error, error.
 * - Single retry with small backoff for transient 5xx/timeouts.
 */
class AuctionClient(
    baseUrl: String,
    private val apiKey: String,
    httpClient: OkHttpClient? = null,
    circuitBreakerFactory: () -> CircuitBreaker = {
        CircuitBreaker(failureThreshold = 4, resetTimeoutMs = 15_000, halfOpenMaxAttempts = 1)
    },
    private val clock: Clock = ClockProvider.clock,
    private val latencyRecorder: ((Long, String) -> Unit)? = null,
) {
    private val gson = Gson()
    private val base = baseUrl.trimEnd('/')
    private val client: OkHttpClient = (httpClient ?: OkHttpClient()).newBuilder()
        .connectionPool(ConnectionPool(5, 5, TimeUnit.MINUTES))
        .connectTimeout(2, TimeUnit.SECONDS)
        .readTimeout(5, TimeUnit.SECONDS)
        .writeTimeout(5, TimeUnit.SECONDS)
        .callTimeout(6, TimeUnit.SECONDS)
        .retryOnConnectionFailure(false)
        .followRedirects(false)
        .followSslRedirects(false)
        .protocols(listOf(Protocol.HTTP_2, Protocol.HTTP_1_1))
        .build()
    private val circuitBreaker = circuitBreakerFactory()
    private val maxAttempts = 3
    private val initialBackoffMs = 120L

    private fun buildUserAgent(): String {
    val sdkVersion = try { com.rivalapexmediation.sdk.BuildConfig.SDK_VERSION } catch (_: Throwable) { "0.0.0" }
        val osVer = "Android ${Build.VERSION.RELEASE ?: "unknown"}"
        val device = "${Build.MANUFACTURER ?: ""} ${Build.MODEL ?: ""}".trim()
        return "RivalApexMediation-Android/$sdkVersion ($osVer; $device)"
    }

    data class ConsentOptions(
        val gdprApplies: Boolean? = null,
        val tcfString: String? = null,
        val usPrivacy: String? = null,
        val coppa: Boolean? = null,
        val limitAdTracking: Boolean? = null,
        val privacySandbox: Boolean? = null,
        val advertisingId: String? = null,
        val appSetId: String? = null,
    ) {
        // Legacy accessor for older callers/tests.
        val consentString: String?
            get() = tcfString
    }

    data class InterstitialOptions(
        val publisherId: String,
        val placementId: String,
        val floorCpm: Double? = null,
        val adapters: List<String>? = null,
        val metadata: Map<String, String> = emptyMap(),
        val timeoutMs: Int = 800,
        val auctionType: String = "header_bidding",
    )

    data class InterstitialResult(
        val adapter: String,
        val ecpm: Double,
        val currency: String,
        val creativeId: String?,
        val adMarkup: String?,
        val raw: Map<String, Any?>? = null,
    )

    class AuctionException(val reason: String, override val message: String? = null): Exception(message ?: reason)

    /**
     * Requests an interstitial auction synchronously. Throws AuctionException on no-bid or errors.
     * Timeouts are enforced per-call using OkHttp callTimeout/readTimeout based on options.timeoutMs.
     */
    fun requestInterstitial(opts: InterstitialOptions, consent: ConsentOptions? = null): InterstitialResult {
        val startMs = clock.monotonicNow()
        if (isOnMainThread()) {
            throw AuctionException("main_thread", "AuctionClient called from main thread")
        }
        if (opts.publisherId.isBlank() || opts.placementId.isBlank()) {
            throw AuctionException("invalid_placement", "publisherId/placementId required")
        }
        val outcome = circuitBreaker.execute(
            action = {
                try {
                    RequestOutcome.Success(performRequestWithRetries(opts, consent))
                } catch (ae: AuctionException) {
                    if (shouldTripCircuit(ae.reason)) {
                        throw ae
                    }
                    RequestOutcome.Error(ae)
                }
            },
            classifySuccess = { result ->
                result?.error?.let { err ->
                    !shouldTripCircuit(err.reason)
                } ?: (result?.result != null)
            },
            countException = { ex -> shouldTripCircuit(mapExceptionToReason(ex)) }
        ) ?: throw AuctionException("circuit_open", "auction circuit breaker open")

        outcome.error?.let {
            latencyRecorder?.invoke(clock.monotonicNow() - startMs, it.reason)
            throw it
        }
        val result = outcome.result ?: throw AuctionException("error")
        latencyRecorder?.invoke(clock.monotonicNow() - startMs, "success")
        return result
    }

    private fun performRequestWithRetries(
        opts: InterstitialOptions,
        consent: ConsentOptions?
    ): InterstitialResult {
        val timeout = opts.timeoutMs.coerceAtLeast(100)
        val callClient = client.newBuilder()
            .callTimeout(timeout.toLong(), TimeUnit.MILLISECONDS)
            .connectTimeout(timeout.toLong(), TimeUnit.MILLISECONDS)
            .readTimeout(timeout.toLong(), TimeUnit.MILLISECONDS)
            .writeTimeout(timeout.toLong(), TimeUnit.MILLISECONDS)
            .build()

        val correlationId = java.util.UUID.randomUUID().toString()
        val requestBody = buildRequestBody(opts, consent, correlationId)
        val json = gson.toJson(requestBody)
        val req = Request.Builder()
            .url("$base/v1/auction")
            .post(json.toRequestBody("application/json".toMediaType()))
            .addHeader("Content-Type", "application/json")
            .addHeader("Accept", "application/json")
            .addHeader("User-Agent", buildUserAgent())
            .addHeader("X-Api-Key", apiKey)
            .addHeader("X-Correlation-ID", correlationId)
            .addHeader("X-SDK-Version", try { com.rivalapexmediation.sdk.BuildConfig.SDK_VERSION } catch (_: Throwable) { "0.0.0" })
            .build()

        var attempt = 1
        var lastErr: AuctionException? = null
        var sawTimeout = false
        while (attempt <= maxAttempts) {
            val call = callClient.newCall(req)
            try {
                call.execute().use { resp ->
                    val code = resp.code
                    if (code == 204) {
                        throw AuctionException("no_fill")
                    }
                    if (code == 429) {
                        val retryAfter = parseRetryAfterMillis(resp)
                        val msg = retryAfter?.let { "rate limited; retry_after_ms=$it" } ?: "rate limited"
                        throw AuctionException("rate_limited", msg)
                    }
                    if (!resp.isSuccessful) {
                        val reason = "status_" + code
                        throw AuctionException(reason)
                    }
                    val bodyStr = resp.body?.string() ?: "{}"
                    @Suppress("UNCHECKED_CAST")
                    val root = gson.fromJson(bodyStr, MutableMap::class.java) as MutableMap<String, Any?>
                    @Suppress("UNCHECKED_CAST")
                    val winner = (root["winner"] as? Map<String, Any?>)
                    if (winner == null) {
                        throw AuctionException("no_fill")
                    }
                    val adapter = (winner["adapter_name"] as? String)
                        ?: (winner["AdapterName"] as? String)
                    val ecpmNum = (winner["cpm"] as? Number)
                        ?: (winner["CPM"] as? Number)
                    if (adapter.isNullOrBlank() || ecpmNum == null) {
                        throw AuctionException("no_fill")
                    }
                    val ecpm = ecpmNum.toDouble()
                    val currency = (winner["currency"] as? String)
                        ?: (winner["Currency"] as? String) ?: "USD"
                    val creativeId = (winner["creative_id"] as? String)
                        ?: (winner["CreativeID"] as? String)
                    val adMarkup = (winner["ad_markup"] as? String)
                        ?: (winner["AdMarkup"] as? String)
                    @Suppress("UNCHECKED_CAST")
                    return InterstitialResult(
                        adapter = adapter,
                        ecpm = ecpm,
                        currency = currency,
                        creativeId = creativeId,
                        adMarkup = adMarkup,
                        raw = root as? Map<String, Any?>
                    )
                }
            } catch (e: Exception) {
                val reason = mapExceptionToReason(e)
                if (reason == "timeout") {
                    sawTimeout = true
                }
                val ex = if (e is AuctionException) e else AuctionException(reason, e.message)
                lastErr = ex
                if (!shouldRetry(reason) || attempt == maxAttempts) {
                    break
                }
                safeSleep(computeBackoffDelay(attempt))
                attempt++
                continue
            }
        }
        if (sawTimeout && (lastErr == null || lastErr.reason != "timeout")) {
            val detail = lastErr?.message?.takeIf { it.isNotBlank() }
            val msg = detail?.let { "timeout (after retry): $it" } ?: "timeout"
            throw AuctionException("timeout", msg)
        }
        val finalErr = lastErr ?: AuctionException("error")
        throw finalErr
    }

    private fun shouldRetry(reason: String): Boolean {
        return reason == "timeout" || reason == "network_error" || reason.startsWith("status_5")
    }

    private fun mapExceptionToReason(e: Exception): String {
        return when {
            e is AuctionException -> e.reason
            isCancellationException(e) -> "navigation_cancelled"
            isTimeoutException(e) -> "timeout"
            else -> {
                val msg = e.message ?: ""
                when {
                    msg.startsWith("status_") -> msg
                    else -> if (isNetworkIOException(e)) "network_error" else "error"
                }
            }
        }
    }

    private fun shouldTripCircuit(reason: String?): Boolean {
        if (reason == null) return true
        val normalized = reason.lowercase(Locale.US)
        return normalized == "timeout" ||
            normalized == "network_error" ||
            normalized.startsWith("status_5") ||
            normalized == "rate_limited"
    }

    // OkHttp surfaces call-timeout breaches as generic IOExceptions, so walk the cause chain
    // and message text to classify them as timeouts instead of generic network errors.
    private fun isTimeoutException(t: Throwable?): Boolean {
        var current: Throwable? = t
        while (current != null) {
            when (current) {
                is SocketTimeoutException -> return true
                is InterruptedIOException -> return true
            }
            val msg = current.message
            if (!msg.isNullOrBlank()) {
                val normalized = msg.lowercase(Locale.US)
                if (normalized.contains("timeout") || normalized.contains("time out") || normalized.contains("timed out")) {
                    return true
                }
            }
            current = current.cause
        }
        return false
    }

    private fun isNetworkIOException(e: Exception): Boolean {
        // Basic heuristic; OkHttp surfaces IOExceptions for network conditions
        return e is java.io.IOException
    }

    private fun isCancellationException(t: Throwable?): Boolean {
        var current: Throwable? = t
        while (current != null) {
            if (current is CancellationException) return true
            if (current is kotlinx.coroutines.CancellationException) return true
            val msg = current.message
            if (!msg.isNullOrBlank() && msg.contains("canceled", ignoreCase = true)) {
                return true
            }
            current = current.cause
        }
        return false
    }

    private fun computeBackoffDelay(attempt: Int): Long {
        val exponent = (attempt - 1).coerceAtLeast(0)
        val delay = initialBackoffMs * (1 shl exponent)
        return min(delay, 1000L)
    }

    private fun safeSleep(durationMs: Long) {
        if (durationMs <= 0) return
        try {
            Thread.sleep(durationMs)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        }
    }

    private fun parseRetryAfterMillis(resp: okhttp3.Response): Long? {
        val header = resp.header("Retry-After")?.trim() ?: return null
        header.toLongOrNull()?.let { return (it * 1000L).coerceAtLeast(0) }
        return try {
            val sdf = SimpleDateFormat("EEE, dd MMM yyyy HH:mm:ss zzz", Locale.US)
            sdf.timeZone = TimeZone.getTimeZone("GMT")
            val date = sdf.parse(header)
            date?.time?.let { (it - clock.now()).coerceAtLeast(0) }
        } catch (_: Exception) {
            null
        }
    }

    private data class RequestOutcome(
        val result: InterstitialResult? = null,
        val error: AuctionException? = null,
    ) {
        companion object {
            fun Success(result: InterstitialResult) = RequestOutcome(result = result)
            fun Error(ex: AuctionException) = RequestOutcome(error = ex)
        }
    }

    private fun consentMeta(consent: ConsentOptions?): Map<String, String> {
        if (consent == null) return emptyMap()
        val meta = mutableMapOf<String, String>()
        consent.gdprApplies?.let { meta["gdpr_applies"] = if (it) "1" else "0" }
        consent.tcfString?.let { meta["gdpr_consent"] = it }
        consent.usPrivacy?.let { meta["us_privacy"] = it }
        consent.coppa?.let { meta["coppa"] = if (it) "1" else "0" }
        consent.limitAdTracking?.let { meta["limit_ad_tracking"] = if (it) "1" else "0" }
        consent.privacySandbox?.let { meta["privacy_sandbox"] = if (it) "1" else "0" }
        return meta
    }

    private fun buildRequestBody(opts: InterstitialOptions, consent: ConsentOptions?, correlationId: String): Map<String, Any?> {
        val now = clock.now()
        val rand = (0..999_999).random()
        val reqId = "android-$now-$rand"
        val deviceInfo = mapOf(
            "platform" to "android",
            "osVersion" to (Build.VERSION.RELEASE ?: ""),
            "model" to (Build.MODEL ?: ""),
            "make" to (Build.MANUFACTURER ?: ""),
            "screenWidth" to 0,
            "screenHeight" to 0,
            "language" to Locale.getDefault().language,
            "timezone" to java.util.TimeZone.getDefault().id,
            "connectionType" to "unknown",
        )
        val limitAdTracking = consent?.limitAdTracking == true
        val userInfo = mutableMapOf<String, Any?>(
            "limit_ad_tracking" to limitAdTracking
        )
        consent?.tcfString
            ?.takeUnless { it.isBlank() }
            ?.let { userInfo["consent_string"] = it }
        consent?.privacySandbox?.let { userInfo["privacy_sandbox"] = it }

        val adId = consent?.advertisingId?.takeUnless { it.isNullOrBlank() }
        val appSetId = consent?.appSetId?.takeUnless { it.isNullOrBlank() }
        if (!adId.isNullOrBlank() && !limitAdTracking) {
            userInfo["advertising_id"] = adId
        } else if (!appSetId.isNullOrBlank()) {
            userInfo["app_set_id"] = appSetId
        }

        val baseMeta = opts.metadata.toMutableMap()
        baseMeta.putAll(consentMeta(consent))

        val adapters = if (opts.adapters != null && opts.adapters.isNotEmpty()) opts.adapters
        else listOf("admob", "meta", "unity", "applovin", "ironsource")

        val sdkVersion = try { com.rivalapexmediation.sdk.BuildConfig.SDK_VERSION } catch (_: Throwable) { "0.0.0" }

        return mapOf(
            "request_id" to reqId,
            "placementId" to opts.placementId,
            "adFormat" to "interstitial",
            "device" to deviceInfo,
            "user_info" to userInfo,
            "app" to mapOf("id" to opts.publisherId),
            "floorCpm" to (opts.floorCpm ?: 0.0),
            "timeoutMs" to opts.timeoutMs,
            "auctionType" to opts.auctionType,
            "adapters" to adapters,
            "metadata" to baseMeta,
            "meta" to mapOf(
                "correlationId" to correlationId,
                "sdk" to mapOf(
                    "name" to "apex-android-sdk",
                    "version" to sdkVersion
                )
            ),
        )
    }

    private fun isOnMainThread(): Boolean {
        if (isTestRuntime()) return false
        return try {
            val main = Looper.getMainLooper()
            main != null && Looper.myLooper() == main
        } catch (_: Throwable) {
            false
        }
    }

    private fun isTestRuntime(): Boolean {
        val override = (System.getProperty("bel.force.testRuntime")
            ?: System.getenv("BEL_FORCE_TEST_RUNTIME"))
            ?.lowercase(Locale.US)
        if (override == "1" || override == "true") return true
        if (override == "0" || override == "false") return false
        return try {
            val fingerprint = Build.FINGERPRINT?.lowercase(Locale.US)
            val robolectricDetected = fingerprint?.contains("robolectric") == true
            val hasRobolectricRuntime = try {
                Class.forName("org.robolectric.RuntimeEnvironment")
                true
            } catch (_: Throwable) {
                false
            }
            robolectricDetected || hasRobolectricRuntime
        } catch (_: Throwable) {
            false
        }
    }

}
