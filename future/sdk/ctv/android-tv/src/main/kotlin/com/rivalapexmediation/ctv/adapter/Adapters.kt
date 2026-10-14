package com.rivalapexmediation.ctv.adapter

/**
 * Lightweight CTV adapter contracts (metadata-only stubs for sandbox readiness)
 */
interface CtvAdapter {
    val networkName: String
    val version: String
    fun supportsBanner(): Boolean = true
    fun supportsInterstitial(): Boolean = true
    fun supportsRewarded(): Boolean = true
}

class StubCtvAdapter(
    override val networkName: String,
    override val version: String = "1.0.0"
) : CtvAdapter

object CtvAdapterRegistry {
    private val names = listOf(
        "admob",
        "applovin",
        "unity",
        "ironsource",
        "facebook",
        "vungle",
        "chartboost",
        "pangle",
        "mintegral",
        "adcolony",
        "tapjoy",
        "moloco",
        "fyber",
        "smaato",
        "amazon"
    )

    val adapters: Map<String, CtvAdapter> = names.associateWith { StubCtvAdapter(it) }

    fun getSupportedNetworks(): List<String> = names
}
