package com.rivalapexmediation.sdk.signalcollection

import android.content.Context
import android.os.Build
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.consent.ConsentManager
import com.rivalapexmediation.sdk.util.ClockProvider
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*

/**
 * Signal Collection API for Server-to-Server (S2S) Header Bidding
 * 
 * This API enables collection of bid tokens and device signals that can be passed
 * to Prebid Server, AppLovin MAX, or other SSPs for server-side auction participation.
 * 
 * Industry Standard Alignment:
 * - AppLovin MAX: collectSignal() / MaxSignalCollectionListener
 * - Google AdMob: RtbAdapter.collectSignals() / SignalCallbacks
 * - Prebid Mobile: fetchDemand() with bidder tokens
 */
object SignalCollector {
    
    /**
     * Callback interface for signal collection results
     */
    interface SignalCollectionListener {
        fun onSignalCollected(signal: BidSignal)
        fun onSignalCollectionFailed(error: SignalError)
    }
    
    /**
     * Parameters for signal collection
     */
    data class SignalCollectionParams(
        val adType: AdType,
        val placementId: String,
        val networkHints: List<String> = emptyList(), // Optional: specific networks to collect for
        val timeoutMs: Long = 3000,
        val customData: Map<String, String> = emptyMap()
    )
    
    /**
     * Collected bid signal containing all data needed for S2S bidding
     */
    data class BidSignal(
        val bidToken: String,
        val signalVersion: String = "1.0",
        val collectedAt: Long = ClockProvider.clock.monotonicNow(),
        val adType: AdType,
        val placementId: String,
        val deviceSignals: DeviceSignals,
        val consentSignals: ConsentSignals,
        val networkSignals: Map<String, NetworkBidToken> = emptyMap(),
        val customData: Map<String, String> = emptyMap()
    ) {
        /**
         * Serialize to JSON string for transmission to bid server
         */
        fun toJson(): String {
            val sb = StringBuilder()
            sb.append("{")
            sb.append("\"bidToken\":\"$bidToken\",")
            sb.append("\"signalVersion\":\"$signalVersion\",")
            sb.append("\"collectedAt\":$collectedAt,")
            sb.append("\"adType\":\"${adType.name}\",")
            sb.append("\"placementId\":\"$placementId\",")
            sb.append("\"deviceSignals\":${deviceSignals.toJson()},")
            sb.append("\"consentSignals\":${consentSignals.toJson()},")
            sb.append("\"networkSignals\":{")
            sb.append(networkSignals.entries.joinToString(",") { (k, v) -> 
                "\"$k\":${v.toJson()}" 
            })
            sb.append("},")
            sb.append("\"customData\":{")
            sb.append(customData.entries.joinToString(",") { (k, v) -> 
                "\"$k\":\"$v\"" 
            })
            sb.append("}}")
            return sb.toString()
        }
        
        /**
         * Get the raw token for passing to SSPs
         */
        fun getRawToken(): String = bidToken
    }
    
    /**
     * Device-level signals for targeting
     */
    data class DeviceSignals(
        val osVersion: String,
        val deviceModel: String,
        val deviceManufacturer: String,
        val screenDensity: Float,
        val screenWidth: Int,
        val screenHeight: Int,
        val language: String,
        val timezone: String,
        val connectionType: String,
        val carrier: String?,
        val isTablet: Boolean,
        val sdkVersion: String
    ) {
        fun toJson(): String {
            return """{"osVersion":"$osVersion","deviceModel":"$deviceModel","manufacturer":"$deviceManufacturer","density":$screenDensity,"width":$screenWidth,"height":$screenHeight,"language":"$language","timezone":"$timezone","connectionType":"$connectionType","carrier":"${carrier ?: ""}","isTablet":$isTablet,"sdkVersion":"$sdkVersion"}"""
        }
    }
    
    /**
     * Consent and privacy signals
     */
    data class ConsentSignals(
        val gdprApplies: Boolean?,
        val tcfConsentString: String?,
        val uspString: String?,
        val gppString: String?,
        val coppa: Boolean?,
        val limitAdTracking: Boolean?
    ) {
        fun toJson(): String {
            return """{"gdprApplies":${gdprApplies ?: "null"},"tcfConsent":"${tcfConsentString ?: ""}","usPrivacy":"${uspString ?: ""}","gpp":"${gppString ?: ""}","coppa":${coppa ?: "null"},"lat":${limitAdTracking ?: "null"}}"""
        }
    }
    
    /**
     * Per-network bid token (for networks that support bidding)
     */
    data class NetworkBidToken(
        val networkName: String,
        val token: String,
        val collectedAt: Long = ClockProvider.clock.monotonicNow(),
        val expiresAt: Long? = null
    ) {
        fun toJson(): String {
            return """{"network":"$networkName","token":"$token","collectedAt":$collectedAt,"expiresAt":${expiresAt ?: "null"}}"""
        }
        
        fun isExpired(): Boolean {
            val exp = expiresAt ?: return false
            return ClockProvider.clock.monotonicNow() > exp
        }
    }
    
    /**
     * Error types for signal collection
     */
    sealed class SignalError(val message: String, val code: String) {
        class Timeout(message: String = "Signal collection timed out") : SignalError(message, "TIMEOUT")
        class NetworkError(message: String = "Network error during signal collection") : SignalError(message, "NETWORK_ERROR")
        class NotInitialized(message: String = "SDK not initialized") : SignalError(message, "NOT_INITIALIZED")
        class InvalidParams(message: String = "Invalid parameters") : SignalError(message, "INVALID_PARAMS")
        class InternalError(message: String = "Internal error") : SignalError(message, "INTERNAL_ERROR")
    }
    
    // Cache for bid tokens with TTL
    private val tokenCache = ConcurrentHashMap<String, CachedToken>()
    private data class CachedToken(val signal: BidSignal, val expiresAt: Long)
    private const val TOKEN_TTL_MS = 300_000L // 5 minutes default TTL
    
    private var sdkVersion: String = "1.0.0"
    private var applicationContext: Context? = null
    
    /**
     * Initialize the signal collector (called by MediationSDK.initialize)
     */
    fun initialize(context: Context, version: String) {
        applicationContext = context.applicationContext
        sdkVersion = version
    }
    
    /**
     * Collect signals for S2S bidding (async with callback)
     * 
     * This is the primary API matching AppLovin MAX's collectSignal() pattern.
     */
    @JvmStatic
    fun collectSignals(
        params: SignalCollectionParams,
        listener: SignalCollectionListener
    ) {
        val context = applicationContext
        if (context == null) {
            listener.onSignalCollectionFailed(SignalError.NotInitialized())
            return
        }
        
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val signal = withTimeout(params.timeoutMs) {
                    collectSignalsInternal(context, params)
                }
                withContext(Dispatchers.Main) {
                    listener.onSignalCollected(signal)
                }
            } catch (e: TimeoutCancellationException) {
                withContext(Dispatchers.Main) {
                    listener.onSignalCollectionFailed(SignalError.Timeout())
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    listener.onSignalCollectionFailed(SignalError.InternalError(e.message ?: "Unknown error"))
                }
            }
        }
    }
    
    /**
     * Collect signals synchronously (for Kotlin coroutines)
     */
    suspend fun collectSignals(params: SignalCollectionParams): BidSignal {
        val context = applicationContext 
            ?: throw IllegalStateException("SDK not initialized")
        return collectSignalsInternal(context, params)
    }
    
    /**
     * Get cached signal if available and not expired
     */
    fun getCachedSignal(placementId: String, adType: AdType): BidSignal? {
        val key = "$placementId:${adType.name}"
        val cached = tokenCache[key] ?: return null
        if (ClockProvider.clock.monotonicNow() > cached.expiresAt) {
            tokenCache.remove(key)
            return null
        }
        return cached.signal
    }
    
    /**
     * Clear all cached signals
     */
    fun clearCache() {
        tokenCache.clear()
    }
    
    private suspend fun collectSignalsInternal(
        context: Context,
        params: SignalCollectionParams
    ): BidSignal = withContext(Dispatchers.IO) {
        // Collect device signals
        val deviceSignals = collectDeviceSignals(context)
        
        // Collect consent signals
        val consentSignals = collectConsentSignals()
        
        // Collect per-network bid tokens if specific networks requested
        val networkSignals = if (params.networkHints.isNotEmpty()) {
            collectNetworkTokens(params.networkHints, params.timeoutMs / 2)
        } else {
            emptyMap()
        }
        
        // Generate the unified bid token
        val bidToken = generateBidToken(
            params.placementId,
            params.adType,
            deviceSignals,
            consentSignals
        )
        
        val signal = BidSignal(
            bidToken = bidToken,
            adType = params.adType,
            placementId = params.placementId,
            deviceSignals = deviceSignals,
            consentSignals = consentSignals,
            networkSignals = networkSignals,
            customData = params.customData
        )
        
        // Cache the signal
        val key = "${params.placementId}:${params.adType.name}"
        tokenCache[key] = CachedToken(
            signal = signal,
            expiresAt = ClockProvider.clock.monotonicNow() + TOKEN_TTL_MS
        )
        
        signal
    }
    
    private fun collectDeviceSignals(context: Context): DeviceSignals {
        val resources = context.resources
        val displayMetrics = resources.displayMetrics
        val config = resources.configuration
        
        return DeviceSignals(
            osVersion = Build.VERSION.RELEASE,
            deviceModel = Build.MODEL,
            deviceManufacturer = Build.MANUFACTURER,
            screenDensity = displayMetrics.density,
            screenWidth = displayMetrics.widthPixels,
            screenHeight = displayMetrics.heightPixels,
            language = config.locales.get(0)?.language ?: "en",
            timezone = java.util.TimeZone.getDefault().id,
            connectionType = detectConnectionType(context),
            carrier = detectCarrier(context),
            isTablet = (config.screenLayout and android.content.res.Configuration.SCREENLAYOUT_SIZE_MASK) >= 
                android.content.res.Configuration.SCREENLAYOUT_SIZE_LARGE,
            sdkVersion = sdkVersion
        )
    }
    
    private fun collectConsentSignals(): ConsentSignals {
        val consentState = ConsentManager.getConsentState()
        return ConsentSignals(
            gdprApplies = consentState.gdprApplies,
            tcfConsentString = consentState.tcfConsentString,
            uspString = consentState.uspString,
            gppString = consentState.gppString,
            coppa = consentState.coppa,
            limitAdTracking = consentState.limitAdTracking
        )
    }
    
    private suspend fun collectNetworkTokens(
        networks: List<String>,
        timeoutMs: Long
    ): Map<String, NetworkBidToken> {
        // This would be extended to call each network's bidding token API
        // For now, return empty map - adapters will register their collectors
        return emptyMap()
    }
    
    private fun generateBidToken(
        placementId: String,
        adType: AdType,
        deviceSignals: DeviceSignals,
        consentSignals: ConsentSignals
    ): String {
        val input = buildString {
            append(placementId)
            append(adType.name)
            append(deviceSignals.osVersion)
            append(deviceSignals.deviceModel)
            append(consentSignals.gdprApplies)
            append(consentSignals.tcfConsentString ?: "")
            append(ClockProvider.clock.monotonicNow())
            append(java.util.UUID.randomUUID().toString())
        }
        
        val md = MessageDigest.getInstance("SHA-256")
        val digest = md.digest(input.toByteArray())
        return "apex_" + digest.joinToString("") { "%02x".format(it) }.take(32)
    }
    
    private fun detectConnectionType(context: Context): String {
        return try {
            val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? android.net.ConnectivityManager
            val network = cm?.activeNetwork
            val caps = cm?.getNetworkCapabilities(network)
            when {
                caps?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) == true -> "wifi"
                caps?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_CELLULAR) == true -> "cellular"
                caps?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_ETHERNET) == true -> "ethernet"
                else -> "unknown"
            }
        } catch (e: Exception) {
            "unknown"
        }
    }
    
    private fun detectCarrier(context: Context): String? {
        return try {
            val tm = context.getSystemService(Context.TELEPHONY_SERVICE) as? android.telephony.TelephonyManager
            tm?.networkOperatorName?.takeIf { it.isNotBlank() }
        } catch (e: Exception) {
            null
        }
    }
}

/**
 * Extension for registering network-specific signal collectors
 */
interface NetworkSignalCollector {
    val networkName: String
    suspend fun collectBidToken(context: Context, adType: AdType): SignalCollector.NetworkBidToken?
}
