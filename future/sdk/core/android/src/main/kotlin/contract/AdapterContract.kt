package com.rivalapexmediation.sdk.contract

import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.util.ClockProvider
import java.security.MessageDigest

/**
 * Canonical adapter contract derived from backend Adapters.md spec.
 * These data classes and interfaces are platform-agnostic snapshots that
 * mediate between host SDK controllers and network-specific adapters.
 */

// ----- Data Contracts -----

/**
 * Publisher-supplied vendor credentials injected at runtime.
 * All fields are BYO secrets and must stay on-device.
 */
data class AdapterCredentials(
    val key: String,
    val secret: String? = null,
    val appId: String? = null,
    val accountIds: Map<String, String>? = null
) {
    // Redact secrets by default if ever stringified or logged.
    override fun toString(): String = buildString {
        append("AdapterCredentials(key=")
        append(hash(key))
        append(", secret=")
        append(secret?.let { "***" } ?: "null")
        append(", appId=")
        append(appId?.let { hash(it) } ?: "null")
        append(", accountIds=")
        append(accountIds?.mapValues { (_, v) -> hash(v) })
        append(")")
    }

    private fun hash(value: String): String = try {
        val digest = MessageDigest.getInstance("SHA-256").digest(value.toByteArray())
        digest.take(6).joinToString("") { byte -> "%02x".format(byte) }
    } catch (_: Exception) {
        "hash_err"
    }
}

/** Adapter tuning knobs supplied by the controller per placement. */
data class AdapterOptions(
    val startMuted: Boolean? = null,
    val testMode: Boolean? = null,
    val bidFloorMicros: Long? = null
)

enum class PartnerRegion { US, EU, APAC, CN, GLOBAL }

/** Normalized privacy metadata forwarded to every adapter request. */
data class ConsentState(
    val gdprApplies: Boolean? = null,
    val iabTcfV2: String? = null,
    val iabUsPrivacy: String? = null,
    val coppa: Boolean = false,
    val attStatus: AttStatus = AttStatus.NOT_DETERMINED,
    val limitAdTracking: Boolean = false,
    val privacySandboxOptIn: Boolean? = null,
    val advertisingId: String? = null,
    val appSetId: String? = null
)

enum class AttStatus { AUTHORIZED, DENIED, NOT_DETERMINED, RESTRICTED }

/** Canonical init payload passed to each adapter implementation. */
data class AdapterConfig(
    val partner: String,
    val credentials: AdapterCredentials,
    val placements: Map<String, String>,
    val privacy: ConsentState,
    val region: PartnerRegion? = null,
    val options: AdapterOptions? = null
)

data class DeviceMeta(
    val os: String,
    val osVersion: String,
    val model: String
)

data class UserMeta(
    val ageRestricted: Boolean,
    val consent: ConsentState,
    val advertisingId: String? = null,
    val appSetId: String? = null
)

data class NetworkMeta(
    val ipPrefixed: String,
    val uaNormalized: String,
    val connType: ConnectionType
)

enum class ConnectionType { WIFI, CELL, OTHER }

data class ContextMeta(
    val orientation: Orientation,
    val sessionDepth: Int
)

enum class Orientation { PORTRAIT, LANDSCAPE }

data class AuctionMeta(
    val floorsMicros: Long? = null,
    val sChain: String? = null,
    val sellersJsonOk: Boolean? = null
)

/** Per-request context (device, user, auction) shared with adapters. */
data class RequestMeta(
    val requestId: String,
    val device: DeviceMeta,
    val user: UserMeta,
    val net: NetworkMeta,
    val context: ContextMeta,
    val auction: AuctionMeta
)

/** Successful load response returned by an adapter. */
data class LoadResult(
    val handle: AdHandle,
    val ttlMs: Int,
    val priceMicros: Long? = null,
    val currency: String? = null,
    val partnerMeta: Map<String, Any?> = emptyMap()
)

data class InitResult(
    val success: Boolean,
    val error: AdapterError? = null,
    val partnerMeta: Map<String, Any?> = emptyMap()
)

/** Token representing a single-render creative. Single-use by design. */
data class AdHandle(
    val id: String,
    val adType: AdType,
    val partnerPlacementId: String? = null,
    val createdAtMs: Long = ClockProvider.clock.monotonicNow()
)

data class PaidEvent(
    val valueMicros: Long,
    val currency: String,
    val precision: PaidEventPrecision,
    val partner: String,
    val partnerUnitId: String? = null,
    val lineItemId: String? = null,
    val creativeId: String? = null
)

enum class PaidEventPrecision { PUBLISHER, ESTIMATED }

// ----- Error taxonomy -----

enum class ErrorCode {
    NO_FILL,
    TIMEOUT,
    NETWORK_ERROR,
    STATUS_4XX,
    STATUS_5XX,
    BELOW_FLOOR,
    ERROR,
    CIRCUIT_OPEN,
    CONFIG,
    NO_AD_READY
}

/** Normalized error returned by adapters; maps vendor failures to the taxonomy. */
sealed class AdapterError(open val code: ErrorCode, open val detail: String, open val vendorCode: String? = null) : Exception("[$code] $detail") {
    data class Fatal(override val code: ErrorCode, override val detail: String, override val vendorCode: String? = null): AdapterError(code, detail, vendorCode)
    data class Recoverable(override val code: ErrorCode, override val detail: String, override val vendorCode: String? = null): AdapterError(code, detail, vendorCode)

    /**
     * Normalized reason string for telemetry/debug surfaces.
     * This keeps Console debugger and adapters aligned on a shared taxonomy.
     */
    fun normalizedReason(): String = when (code) {
        ErrorCode.NO_FILL -> "no_fill"
        ErrorCode.TIMEOUT -> "timeout"
        ErrorCode.NETWORK_ERROR -> "network_error"
        ErrorCode.STATUS_4XX -> "status_4xx"
        ErrorCode.STATUS_5XX -> "status_5xx"
        ErrorCode.BELOW_FLOOR -> "below_floor"
        ErrorCode.CIRCUIT_OPEN -> "circuit_open"
        ErrorCode.CONFIG -> "config_error"
        ErrorCode.NO_AD_READY -> "no_ad_ready"
        ErrorCode.ERROR -> "error"
    }
}

// ----- Callback contracts -----

interface ShowCallbacks {
    fun onImpression(meta: Map<String, Any?> = emptyMap())
    fun onPaidEvent(event: PaidEvent)
    fun onClick(meta: Map<String, Any?> = emptyMap())
    fun onClosed(reason: CloseReason)
    fun onError(error: AdapterError)
}

interface RewardedCallbacks : ShowCallbacks {
    fun onRewardVerified(rewardType: String, rewardAmount: Double)
}

interface BannerCallbacks : ShowCallbacks {
    fun onViewAttached()
    fun onViewDetached(reason: CloseReason)
}

enum class CloseReason { COMPLETED, SKIPPED, DISMISSED }

// ----- Core adapter interface (v2) -----

/**
 * Shared adapter surface enforced on Android/iOS/Unity.
 * Implementations must obey threading, timeout, and single-use handle rules.
 */
interface AdNetworkAdapterV2 {
    fun init(config: AdapterConfig, timeoutMs: Int): InitResult
    fun loadInterstitial(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult
    fun showInterstitial(handle: AdHandle, viewContext: Any, callbacks: ShowCallbacks)
    fun loadRewarded(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult
    fun showRewarded(handle: AdHandle, viewContext: Any, callbacks: RewardedCallbacks)
    fun loadBanner(placementId: String, size: AdSize, meta: RequestMeta, timeoutMs: Int): LoadResult
    fun attachBanner(handle: AdHandle, bannerHost: Any, callbacks: BannerCallbacks)
    fun isAdReady(handle: AdHandle): Boolean
    fun expiresAt(handle: AdHandle): Long
    fun invalidate(handle: AdHandle)
}

data class AdSize(val width: Int, val height: Int)