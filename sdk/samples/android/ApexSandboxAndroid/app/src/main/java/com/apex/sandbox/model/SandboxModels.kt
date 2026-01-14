package com.apex.sandbox.model

data class ConsentState(
    var gdpr: Boolean = false,
    var ccpa: Boolean = false,
    var coppa: Boolean = false,
    var lat: Boolean = true,
    var testMode: Boolean = true,
)

data class Placements(
    val interstitialA: String = "",
    val interstitialB: String = "",
    val rewardedA: String = "",
    val bannerA: String = "",
)

data class SandboxConfig(
    val apiBase: String = "",
    val placements: Placements = Placements(),
    val consent: ConsentState = ConsentState(),
    val adapterWhitelist: List<String>? = null,
    val forceAdapterPipeline: Boolean? = null,
)
