package com.rivalapexmediation.sdk.adapters.ironsource

/**
 * Minimal wire payload models for ironSource LevelPlay demand calls.
 * They deliberately track only the fields the SDK can reliably populate.
 */
internal data class IronSourceBidRequest(
    val appKey: String,
    val instanceId: String,
    val adUnit: String,
    val bundleId: String,
    val platform: String,
    val osVersion: String,
    val deviceModel: String,
    val ip: String,
    val lmt: Boolean,
    val userAgent: String,
    val sessionDepth: Int,
    val requestId: String,
    val floorMicros: Long?,
    val test: Boolean,
    val consent: IronSourceConsent,
    val country: String?,
    val advertisingId: String? = null
)

internal data class IronSourceConsent(
    val gdpr: String?,
    val usPrivacy: String?,
    val coppa: Boolean
)

internal data class IronSourceBidResponse(
    val providerName: String? = null,
    val revenue: Double? = null,
    val auctionId: String? = null,
    val creativeId: String? = null,
    val adMarkup: String? = null,
    val instanceId: String? = null,
    val ttl: Long? = null
)
