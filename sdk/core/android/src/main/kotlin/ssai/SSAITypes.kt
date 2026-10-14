package com.rivalapexmediation.sdk.ssai

import java.util.Date

/**
 * SSAI Types for Android Core SDK
 *
 * Type definitions for Server-Side Ad Insertion functionality.
 * Ported from Android TV SDK with mobile-specific enhancements.
 */

/**
 * SSAI Configuration
 */
data class SSAIConfig(
    /** SSAI service endpoint */
    val endpoint: String,
    /** Tenant identifier */
    val tenantId: String,
    /** Content URL to monetize */
    val contentUrl: String,
    /** Content format (hls or dash) */
    val format: SSAIContentFormat = SSAIContentFormat.HLS,
    /** Device capabilities */
    val capabilities: SSAIDeviceCapabilities? = null,
    /** Consent data for ad targeting */
    val consent: SSAIConsentData? = null,
    /** Request timeout (milliseconds) */
    val timeoutMs: Long = 10000L,
    /** Enable debug logging */
    val debug: Boolean = false
)

/**
 * Content format for SSAI
 */
enum class SSAIContentFormat(val value: String) {
    HLS("hls"),
    DASH("dash")
}

/**
 * Device capabilities for ad targeting
 */
data class SSAIDeviceCapabilities(
    /** Maximum video resolution */
    val maxResolution: SSAIResolution? = null,
    /** Supported video codecs */
    val codecs: List<String>? = null,
    /** HDR support */
    val supportsHdr: Boolean = false,
    /** Dolby support */
    val supportsDolby: Boolean = false,
    /** Maximum bitrate in bits/second */
    val maxBitrate: Long? = null,
    /** Device type */
    val deviceType: SSAIDeviceType = SSAIDeviceType.MOBILE
) {
    companion object {
        /** Default capabilities for Android mobile */
        val mobile = SSAIDeviceCapabilities(
            maxResolution = SSAIResolution(1920, 1080),
            codecs = listOf("avc1", "hvc1", "vp9", "mp4a", "opus"),
            supportsHdr = true,
            supportsDolby = false,
            maxBitrate = 15_000_000L,
            deviceType = SSAIDeviceType.MOBILE
        )
        
        /** Default capabilities for Android tablet */
        val tablet = SSAIDeviceCapabilities(
            maxResolution = SSAIResolution(2560, 1600),
            codecs = listOf("avc1", "hvc1", "vp9", "mp4a", "opus"),
            supportsHdr = true,
            supportsDolby = true,
            maxBitrate = 20_000_000L,
            deviceType = SSAIDeviceType.TABLET
        )
    }
}

/**
 * Device type for SSAI targeting
 */
enum class SSAIDeviceType(val value: String) {
    MOBILE("mobile"),
    TABLET("tablet"),
    DESKTOP("desktop"),
    CTV("ctv"),
    STB("stb")
}

/**
 * Video resolution
 */
data class SSAIResolution(
    val width: Int,
    val height: Int
)

/**
 * Consent data for SSAI targeting
 */
data class SSAIConsentData(
    /** GDPR applies */
    val gdprApplies: Boolean? = null,
    /** IAB TCF v2 consent string */
    val tcfConsent: String? = null,
    /** US Privacy / CCPA string */
    val usPrivacy: String? = null,
    /** GPP string */
    val gpp: String? = null,
    /** COPPA applies */
    val coppa: Boolean = false
)

/**
 * SSAI Session
 */
data class SSAISession(
    /** Unique session identifier */
    val sessionId: String,
    /** Rewritten manifest URL with ad insertions */
    val manifestUrl: String,
    /** Tracking endpoint URL */
    val trackingUrl: String,
    /** Session expiration time */
    val expiresAt: Date,
    /** Session token for authentication */
    val sessionToken: String,
    /** Original content URL */
    val contentUrl: String,
    /** Ad breaks included in session */
    val adBreaks: List<SSAIAdBreak> = emptyList()
) {
    /** Check if session is still valid */
    val isValid: Boolean
        get() = Date().before(expiresAt)
}

/**
 * SSAI Session State
 */
enum class SSAISessionState {
    IDLE,
    INITIALIZING,
    ACTIVE,
    BUFFERING,
    PAUSED,
    COMPLETED,
    ERROR
}

/**
 * SSAI Ad Break
 */
data class SSAIAdBreak(
    /** Pod identifier */
    val podId: String,
    /** Break type (preroll, midroll, postroll) */
    val type: SSAIAdBreakType,
    /** Position in content (seconds) */
    val position: Double,
    /** Start time (seconds) */
    val startTime: Double,
    /** Total duration (seconds) */
    val duration: Double,
    /** Planned number of ad slots */
    val plannedSlots: Int,
    /** Actually filled slots */
    val filledSlots: Int,
    /** Individual ads in this break */
    val ads: List<SSAIAd>
) {
    /** Number of ads in this break */
    val adCount: Int get() = ads.size
}

/**
 * Ad break type
 */
enum class SSAIAdBreakType(val value: String) {
    PREROLL("preroll"),
    MIDROLL("midroll"),
    POSTROLL("postroll")
}

/**
 * SSAI Ad
 */
data class SSAIAd(
    /** Slot identifier */
    val slotId: String,
    /** Creative identifier */
    val creativeId: String,
    /** Ad duration (seconds) */
    val duration: Double,
    /** Start offset within pod (seconds) */
    val startOffset: Double = 0.0,
    /** Advertiser name */
    val advertiser: String? = null,
    /** Ad title */
    val title: String? = null,
    /** Skip offset if skippable (seconds) */
    val skipOffset: Double? = null,
    /** Original VAST URL */
    val vastUrl: String? = null,
    /** Click-through URL */
    val clickThroughUrl: String? = null
) {
    /** Check if ad is skippable */
    val isSkippable: Boolean get() = skipOffset != null
}

/**
 * SSAI Tracking Event Types
 */
enum class SSAITrackingEventType(val value: String) {
    IMPRESSION("impression"),
    START("start"),
    FIRST_QUARTILE("firstQuartile"),
    MIDPOINT("midpoint"),
    THIRD_QUARTILE("thirdQuartile"),
    COMPLETE("complete"),
    SKIP("skip"),
    CLICK("click"),
    ERROR("error"),
    PAUSE("pause"),
    RESUME("resume"),
    MUTE("mute"),
    UNMUTE("unmute"),
    FULLSCREEN("fullscreen"),
    EXIT_FULLSCREEN("exitFullscreen")
}

/**
 * SSAI Tracking Event
 */
data class SSAITrackingEvent(
    /** Event type */
    val type: SSAITrackingEventType,
    /** Pod identifier */
    val podId: String,
    /** Slot identifier */
    val slotId: String,
    /** Creative identifier */
    val creativeId: String,
    /** Media playback time (seconds) */
    val mediaTime: Double,
    /** Event timestamp */
    val timestamp: Date = Date(),
    /** Additional metadata */
    val metadata: Map<String, String> = emptyMap()
)

/**
 * SSAI Metrics
 */
data class SSAIMetrics(
    /** Total ad breaks encountered */
    var adBreakCount: Int = 0,
    /** Total ads encountered */
    var adCount: Int = 0,
    /** Total ad time (seconds) */
    var totalAdTime: Double = 0.0,
    /** Completed ads count */
    var completedAds: Int = 0,
    /** Skipped ads count */
    var skippedAds: Int = 0,
    /** Errored ads count */
    var erroredAds: Int = 0,
    /** Fill rate (0-1) */
    var fillRate: Double = 0.0
)

/**
 * SSAI Error
 */
data class SSAIError(
    /** Error code */
    val code: String,
    /** Human-readable error message */
    override val message: String,
    /** Is this a fatal error */
    val fatal: Boolean = false
) : Exception("[$code] $message") {
    companion object {
        val SESSION_EXPIRED = SSAIError("SESSION_EXPIRED", "SSAI session has expired", fatal = true)
        val NETWORK_ERROR = SSAIError("NETWORK_ERROR", "Network request failed", fatal = false)
        val INVALID_CONFIG = SSAIError("INVALID_CONFIG", "Invalid SSAI configuration", fatal = true)
        val MANIFEST_PARSE_ERROR = SSAIError("MANIFEST_PARSE_ERROR", "Failed to parse manifest", fatal = false)
    }
}
