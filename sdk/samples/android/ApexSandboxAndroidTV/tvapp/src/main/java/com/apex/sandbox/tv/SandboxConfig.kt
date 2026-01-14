package com.apex.sandbox.tv

import android.content.Context
import android.util.Log
import com.google.gson.Gson

data class TvSandboxPlacements(
    val interstitialA: String = "tv_interstitial_a",
    val rewardedA: String = "tv_rewarded_a"
)

data class TvSandboxConfig(
    val appId: String = "sandbox-app-androidtv",
    val placements: TvSandboxPlacements = TvSandboxPlacements(),
    val testMode: Boolean = true,
    val adapterWhitelist: List<String>? = null,
    val forceAdapterPipeline: Boolean? = null
)

object TvSandboxConfigLoader {
    private const val TAG = "ApexSandboxTV"
    private const val FILE = "sandbox_config.json"

    fun load(context: Context): TvSandboxConfig {
        return try {
            context.assets.open(FILE).use { input ->
                val json = input.bufferedReader().readText()
                Gson().fromJson(json, TvSandboxConfig::class.java)
            }
        } catch (e: Exception) {
            Log.w(TAG, "Config load failed, using defaults: ${e.message}")
            TvSandboxConfig()
        }
    }
}
