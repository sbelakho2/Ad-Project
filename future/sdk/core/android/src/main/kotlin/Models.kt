package com.rivalapexmediation.sdk.models

/**
 * Represents an ad creative and its metadata
 */
data class Ad(
    val id: String,
    val placementId: String,
    val networkName: String,
    val adType: AdType,
    val ecpm: Double,
    val creative: Creative,
    val metadata: Map<String, String> = emptyMap(),
    // Optional absolute expiry timestamp for the ad (epoch millis). If null, ad does not expire by time.
    val expiryTimeMs: Long? = null,
    // Creation time (epoch millis) recorded by the SDK; informative only.
    val createdAtMs: Long = com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow()
) {
    @Suppress("UNUSED_PARAMETER")
    fun show(activity: android.app.Activity) {
        // Implementation for showing ad
    }
    
    fun isExpired(): Boolean {
        val exp = expiryTimeMs
        return exp != null && com.rivalapexmediation.sdk.util.ClockProvider.clock.monotonicNow() >= exp
    }
}

/**
 * Ad format types
 */
enum class AdType {
    BANNER,
    INTERSTITIAL,
    REWARDED,
    REWARDED_INTERSTITIAL,
    NATIVE,
    APP_OPEN
}

/**
 * Ad creative content
 */
sealed class Creative {
    data class Banner(
        val width: Int,
        val height: Int,
        val markupHtml: String
    ) : Creative()
    
    data class Video(
        val vastXml: String,
        val duration: Int,
        val skipOffset: Int?
    ) : Creative()
    
    data class Native(
        val title: String,
        val description: String,
        val iconUrl: String?,
        val imageUrl: String?,
        val callToAction: String,
        val clickUrl: String
    ) : Creative()
}

/**
 * Response from ad loading operation
 */
data class AdResponse(
    val ad: Ad?,
    val ecpm: Double,
    val loadTime: Long,
    val networkName: String
) {
    fun isValid(): Boolean {
        return ad != null && !ad.isExpired()
    }
}

/**
 * Placement configuration from backend
 */
data class PlacementConfig(
    val placementId: String,
    val adType: AdType,
    val enabledNetworks: List<String>,
    val timeoutMs: Long = 3000,
    val maxWaitMs: Long = 5000,
    val floorPrice: Double = 0.0,
    val refreshInterval: Int? = null,
    val targeting: Targeting = Targeting()
)

/**
 * Targeting parameters
 */
data class Targeting(
    val age: Int? = null,
    val gender: Gender? = null,
    val keywords: List<String> = emptyList(),
    val customParams: Map<String, String> = emptyMap()
)

enum class Gender {
    MALE,
    FEMALE,
    OTHER,
    UNKNOWN
}

/**
 * Ad adapter interface
 */
interface AdAdapter {
    val name: String
    val version: String
    
    fun isAvailable(): Boolean
    fun loadAd(placement: String, config: PlacementConfig): AdResponse?
    fun destroy()
    /**
     * Optional: Validate adapter configuration/credentials without requesting an ad.
     * Default implementation returns [ValidationResult.unsupported()].
     */
    fun validateConfig(config: Map<String, String>): ValidationResult {
        return ValidationResult.unsupported()
    }
}

/**
 * Telemetry event
 */
data class TelemetryEvent(
    val eventType: EventType,
    val timestamp: Long = com.rivalapexmediation.sdk.util.ClockProvider.clock.now(),
    val placement: String? = null,
    val networkName: String? = null,
    val ecpm: Double? = null,
    val latency: Long? = null,
    val errorCode: String? = null,
    val errorMessage: String? = null,
    val metadata: Map<String, Any> = emptyMap()
)

enum class EventType {
    SDK_INIT,
    AD_REQUEST,
    AD_LOADED,
    AD_SHOWN,
    AD_CLICKED,
    AD_CLOSED,
    AD_FAILED,
    CONFIG_LOADED,
    CONFIG_FAILED,
    TIMEOUT,
    CIRCUIT_OPEN,
    ANR_DETECTED,
    CREDENTIAL_VALIDATION_SUCCESS,
    CREDENTIAL_VALIDATION_FAILED,
    // Observability adapter spans (P1.9)
    ADAPTER_SPAN_START,
    ADAPTER_SPAN_FINISH,
    OMSDK_STATUS,
}

/**
 * Configuration from backend
 */
data class SDKRemoteConfig(
    val configId: String,
    val version: Long,
    val placements: Map<String, PlacementConfig>,
    val adapters: Map<String, AdapterConfig>,
    val features: FeatureFlags,
    val signature: String,
    val timestamp: Long
)

data class AdapterConfig(
    val name: String,
    val enabled: Boolean,
    val priority: Int,
    val params: Map<String, String>,
    val supportsS2S: Boolean = false,
    val requiredCredentialKeys: List<String> = emptyList()
)

data class FeatureFlags(
    val telemetryEnabled: Boolean = true,
    val crashReportingEnabled: Boolean = true,
    val debugLoggingEnabled: Boolean = false,
    val experimentalFeaturesEnabled: Boolean = false,
    val killSwitch: Boolean = false,
    // Developer-only banner to warn about app-ads.txt issues (propagated via remote config).
    // SDK never calls inspector; console/back-end compute and set this flag. Default false.
    val devAppAdsInspectorWarn: Boolean = false,
    // Network TLS pinning (P1.10): enable and provide host->pins mapping (OkHttp sha256/sha1 pins)
    // Example JSON: { "api.apexmediation.ee": ["sha256/AAAAAAAA...", "sha256/BBBBBBBB..."] }
    val netTlsPinningEnabled: Boolean = false,
    val netTlsPinning: Map<String, List<String>> = emptyMap(),
    // OM SDK is BYO; default off unless the host bundles the vendor library.
    val enableOmSdk: Boolean = false,
)

/**
 * Result of credential/config validation for an adapter.
 * Details are automatically sanitized so secrets never leave the device/logs.
 */
data class ValidationResult private constructor(
    val success: Boolean,
    val code: String,
    val message: String?,
    val details: Map<String, Any?>
) {
    companion object {
        private val SECRET_KEY_REGEX = Regex("(key|token|secret|signature|app_id|account|placement)", RegexOption.IGNORE_CASE)
        private const val MAX_DETAIL_ENTRIES = 12

        fun ok(message: String? = null, details: Map<String, Any?> = emptyMap()) =
            ValidationResult(true, "ok", message, sanitize(details))

        fun error(code: String, message: String? = null, details: Map<String, Any?> = emptyMap()) =
            ValidationResult(false, code, message, sanitize(details))

        fun unsupported() = ValidationResult(false, "unsupported", "Adapter does not support validation", emptyMap())

        private fun sanitize(details: Map<String, Any?>): Map<String, Any?> {
            if (details.isEmpty()) return emptyMap()
            val sanitized = linkedMapOf<String, Any?>()
            details.entries.take(MAX_DETAIL_ENTRIES).forEach { (key, value) ->
                val needsRedaction = SECRET_KEY_REGEX.containsMatchIn(key) || (value is String && SECRET_KEY_REGEX.containsMatchIn(value))
                sanitized[key] = if (needsRedaction) "***" else value
            }
            return sanitized
        }
    }
}

/**
 * Callback for batch credential validation results.
 */
interface ValidationCallback {
    fun onComplete(results: Map<String, ValidationResult>)
}
