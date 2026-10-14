package com.apex.sandbox.sdk

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlin.random.Random

sealed class AdResult {
    data class Loaded(val placementId: String) : AdResult()
    data class Shown(val placementId: String) : AdResult()
    data class Error(val placementId: String, val code: String, val message: String) : AdResult()
}

enum class FakeNetwork { A_ALWAYS_FILL, B_RANDOM_NO_FILL, C_SLOW_TIMEOUT }

data class InitOptions(
    val apiBase: String,
    val gdpr: Boolean,
    val ccpa: Boolean,
    val coppa: Boolean,
    val lat: Boolean,
    val testMode: Boolean,
)

class FakeMediationSdk(private val scope: CoroutineScope) {
    private var initialized = false
    private val showMutex = Mutex()

    suspend fun initialize(opts: InitOptions): Result<String> {
        delay(300) // simulate small init delay
        initialized = true
        return Result.success("initialized:${opts.apiBase}")
    }

    suspend fun loadInterstitial(placementId: String, network: FakeNetwork, airplane: Boolean, invalidPlacement: Boolean): AdResult {
        if (!initialized) return AdResult.Error(placementId, "not_initialized", "Call initialize first")
        if (invalidPlacement) return AdResult.Error(placementId, "invalid_placement", "Unknown placement")
        if (airplane) return AdResult.Error(placementId, "network_unreachable", "Airplane mode simulation")
        return simulateNetwork(placementId, network)
    }

    suspend fun showInterstitial(placementId: String): AdResult {
        if (!initialized) return AdResult.Error(placementId, "not_initialized", "Call initialize first")
        return showMutex.withLock {
            delay(200)
            AdResult.Shown(placementId)
        }
    }

    suspend fun loadRewarded(placementId: String, network: FakeNetwork, airplane: Boolean, invalidPlacement: Boolean): AdResult {
        return loadInterstitial(placementId, network, airplane, invalidPlacement)
    }

    suspend fun showRewarded(placementId: String): AdResult {
        return showInterstitial(placementId)
    }

    private suspend fun simulateNetwork(placementId: String, network: FakeNetwork): AdResult {
        return when (network) {
            FakeNetwork.A_ALWAYS_FILL -> {
                delay(250)
                AdResult.Loaded(placementId)
            }
            FakeNetwork.B_RANDOM_NO_FILL -> {
                delay(400)
                if (Random.nextBoolean()) AdResult.Loaded(placementId) else AdResult.Error(placementId, "no_fill", "No ad available")
            }
            FakeNetwork.C_SLOW_TIMEOUT -> {
                delay(3000)
                AdResult.Error(placementId, "timeout", "Request timed out")
            }
        }
    }
}
