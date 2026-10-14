package com.rivalapexmediation.ctv.network

import com.rivalapexmediation.ctv.consent.ConsentData

data class AuctionRequest(
    val requestId: String,
    val placementId: String,
    val adFormat: String, // interstitial|rewarded|banner|native (CTV focuses on interstitial/rewarded)
    val floorCpm: Double = 0.0,
    val device: Map<String, Any?> = emptyMap(),
    val user: Map<String, Any?> = emptyMap(),
    val app: Map<String, Any?> = emptyMap(),
    val consent: Map<String, Any?>? = null,
    val signal: Map<String, Any?>? = null,
)

data class AuctionWin(
    val requestId: String,
    val bidId: String,
    val adapter: String,
    val cpm: Double,
    val currency: String,
    val ttlSeconds: Int,
    val creativeUrl: String,
    val tracking: TrackingUrls,
    val payload: Map<String, Any?>?,
)

data class TrackingUrls(
    val impression: String,
    val click: String,
    val start: String? = null,
    val firstQuartile: String? = null,
    val midpoint: String? = null,
    val thirdQuartile: String? = null,
    val complete: String? = null,
    val pause: String? = null,
    val resume: String? = null,
    val mute: String? = null,
    val unmute: String? = null,
    val close: String? = null,
)

data class AuctionResponseEnvelope(
    val success: Boolean? = null,
    val response: AuctionWin? = null,
    val reason: String? = null,
)

fun buildConsentMap(c: ConsentData?): Map<String, Any?>? {
    if (c == null) return null
    val map = mutableMapOf<String, Any?>()
    c.gdprApplies?.let { map["gdpr"] = if (it) 1 else 0 }
    c.tcfString?.let { if (it.isNotEmpty()) map["gdpr_consent"] = it }
    c.usPrivacy?.let { if (it.isNotEmpty()) map["us_privacy"] = it }
    c.coppa?.let { map["coppa"] = it }
    return map
}
