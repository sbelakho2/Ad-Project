package com.rivalapexmediation.ctv.ssai

import java.net.URL
import java.util.Date

/**
 * SSAI Types for Android TV SDK
 *
 * Type definitions for Server-Side Ad Insertion functionality.
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
    val maxBitrate: Long? = null
) {
    companion object {
        /** Default capabilities for Android TV */
        val androidTv = SSAIDeviceCapabilities(
            maxResolution = SSAIResolution(3840, 2160),
            codecs = listOf("avc1", "hvc1", "vp9", "mp4a", "opus"),
            supportsHdr = true,
            supportsDolby = true,
            maxBitrate = 25_000_000L
        )
    }
}

/**
 * Video resolution
 */
data class SSAIResolution(
    val width: Int,
    val height: Int
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
    /** Ad breaks included in session */
    val adBreaks: List<SSAIAdBreak> = emptyList()
) {
    /** Check if session is still valid */
    val isValid: Boolean
        get() = Date().before(expiresAt)
}

/**
 * SSAI Ad Break
 */
data class SSAIAdBreak(
    /** Pod identifier */
    val podId: String,
    /** Break type (preroll, midroll, postroll) */
    val type: String,
    /** Position in content (seconds) */
    val position: Int,
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
)

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
    val startOffset: Double,
    /** Skip offset if skippable (seconds) */
    val skipOffset: Double? = null,
    /** Original VAST URL */
    val vastUrl: String? = null,
    /** OM SDK verification scripts */
    val verificationScripts: List<SSAIVerificationScript> = emptyList()
) {
    /** Check if ad is skippable */
    val isSkippable: Boolean
        get() = skipOffset != null
}

/**
 * Verification script for OM SDK
 */
data class SSAIVerificationScript(
    val vendor: String,
    val resourceUrl: String,
    val parameters: String? = null
)

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
    COLLAPSE("collapse")
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
 * SSAI Error
 */
data class SSAIError(
    /** Error code */
    val code: String,
    /** Human-readable error message */
    val errorMessage: String,
    /** Whether this is a fatal error */
    val fatal: Boolean = false
) : Exception("[$code] $errorMessage") {
    override val message: String get() = "[$code] $errorMessage"
}

/**
 * SSAI Session State
 */
enum class SSAISessionState {
    IDLE,
    INITIALIZING,
    ACTIVE,
    REFRESHING,
    ERROR,
    ENDED
}

/**
 * SSAI Metrics
 */
data class SSAIMetrics(
    /** Total ads encountered */
    var adCount: Int = 0,
    /** Ads completed */
    var completedAds: Int = 0,
    /** Ads skipped */
    var skippedAds: Int = 0,
    /** Ads with errors */
    var erroredAds: Int = 0,
    /** Total ad time (seconds) */
    var totalAdTime: Double = 0.0,
    /** Total content time (seconds) */
    var totalContentTime: Double = 0.0
) {
    /** Completion rate as percentage */
    val completionRate: Double
        get() = if (adCount > 0) (completedAds.toDouble() / adCount) * 100 else 0.0
    
    /** Fill rate as percentage */
    val fillRate: Double
        get() = if (adCount > 0) ((adCount - erroredAds).toDouble() / adCount) * 100 else 0.0
}
