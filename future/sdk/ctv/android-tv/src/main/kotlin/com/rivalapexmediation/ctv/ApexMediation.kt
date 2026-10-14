package com.rivalapexmediation.ctv

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.StrictMode
import com.rivalapexmediation.ctv.consent.ConsentData
import com.rivalapexmediation.ctv.consent.ConsentManager
import com.rivalapexmediation.ctv.ads.AdapterCircuitBreaker
import com.rivalapexmediation.ctv.ads.SystemMonotonicClock
import com.rivalapexmediation.ctv.network.AuctionClient
import com.rivalapexmediation.ctv.util.Logger
import com.rivalapexmediation.ctv.config.ConfigManager
import com.rivalapexmediation.ctv.metrics.MetricsRecorder
import com.rivalapexmediation.ctv.BuildConfig

/**
 * Public entry point for the CTV/OTT SDK.
 */
object ApexMediation {
    @Volatile private var initialized = false
    private lateinit var appContext: Context
    private lateinit var cfg: SDKConfig
    private lateinit var consentMgr: ConsentManager
    private lateinit var auctionClient: AuctionClient
    private val mainHandler = Handler(Looper.getMainLooper())
    private var configMgr: ConfigManager? = null
    private val pacing: MutableMap<String, Long> = mutableMapOf()
    private val breakers: MutableMap<String, AdapterCircuitBreaker> = mutableMapOf()

    @JvmStatic
    fun initialize(context: Context, config: SDKConfig, onComplete: ((Boolean) -> Unit)? = null) {
        if (initialized) { onComplete?.let { postMain { it(true) } }; return }
        appContext = context.applicationContext
        cfg = config
        Logger.debug = config.testMode
        MetricsRecorder.initialize(config)
        if (BuildConfig.DEBUG) {
            StrictMode.setThreadPolicy(
                StrictMode.ThreadPolicy.Builder()
                    .detectNetwork()
                    .detectDiskReads()
                    .detectDiskWrites()
                    .penaltyLog()
                    .build()
            )
        }
        consentMgr = ConsentManager(appContext)
        auctionClient = AuctionClient(appContext, cfg)
        // Best-effort OTA config load (safe to fail)
        try {
            configMgr = ConfigManager(appContext, cfg).also { it.load() }
        } catch (_: Throwable) {}
        initialized = true
        postMain { onComplete?.invoke(true) }
    }

    @JvmStatic
    fun isInitialized(): Boolean = initialized

    @JvmStatic
    fun setConsent(consent: ConsentData) {
        ensureInit()
        consentMgr.setConsent(consent)
    }

    internal fun consent() = consentMgr.getConsent()
    internal fun config() = cfg
    internal fun context() = appContext
    internal fun client() = auctionClient
    internal fun remoteConfig() = configMgr?.get()
    internal fun recordSloSample(success: Boolean) { configMgr?.recordSloSample(success) }
    internal fun metricsEnabled(): Boolean = configMgr?.get()?.features?.metricsEnabled == true

    internal fun loadBlockedReason(placementId: String): String? {
        if (shouldPace(placementId)) return "pacing"
        if (breakerFor(placementId).isOpen()) return "circuit_open"
        val rc = remoteConfig() ?: return null
        if (rc.features.killSwitch) return "kill_switch_active"
        val placement = rc.placements[placementId]
        if (placement?.killSwitch == true) return "placement_disabled"
        return null
    }

    internal fun showBlockedReason(placementId: String): String? {
        val rc = remoteConfig() ?: return null
        if (rc.features.killSwitch) return "kill_switch_active"
        if (rc.features.disableShow) return "show_disabled"
        val placement = rc.placements[placementId]
        if (placement?.killSwitch == true) return "placement_disabled"
        return null
    }

    private fun ensureInit() {
        if (!initialized) throw IllegalStateException("ApexMediation not initialized")
    }

    internal fun postMain(block: () -> Unit) { if (Looper.myLooper() == Looper.getMainLooper()) block() else mainHandler.post(block) }

    internal fun recordLoadSuccess(placementId: String) {
        breakerFor(placementId).recordSuccess()
    }

    internal fun recordLoadFailure(placementId: String, reason: String) {
        val normalized = reason.lowercase()
        if (normalized.contains("no_fill") || normalized.contains("below_floor")) {
            markPaced(placementId)
            breakerFor(placementId).recordSuccess()
        } else {
            breakerFor(placementId).recordFailure()
        }
    }

    private fun shouldPace(placementId: String, minRetryMs: Long = 2_000): Boolean {
        val now = SystemMonotonicClock.now()
        val last = pacing[placementId] ?: return false
        return now - last < minRetryMs
    }

    private fun markPaced(placementId: String) {
        pacing[placementId] = SystemMonotonicClock.now()
    }

    private fun breakerFor(placementId: String): AdapterCircuitBreaker {
        return breakers.getOrPut(placementId) { AdapterCircuitBreaker(clock = SystemMonotonicClock) }
    }
}
