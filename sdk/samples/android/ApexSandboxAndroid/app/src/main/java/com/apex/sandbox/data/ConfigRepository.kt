package com.apex.sandbox.data

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONObject
import com.apex.sandbox.model.*

class ConfigRepository(private val context: Context) {
    private val prefs: SharedPreferences = context.getSharedPreferences("sandbox_prefs", Context.MODE_PRIVATE)

    fun loadConfig(): SandboxConfig {
        val json = context.assets.open("sandbox_config.json").bufferedReader().use { it.readText() }
        val root = JSONObject(json)
        val apiBase = root.optString("apiBase", "")
        val placementsObj = root.optJSONObject("placements") ?: JSONObject()
        val consentObj = root.optJSONObject("consent") ?: JSONObject()
        val placements = Placements(
            interstitialA = placementsObj.optString("interstitialA", ""),
            interstitialB = placementsObj.optString("interstitialB", ""),
            rewardedA = placementsObj.optString("rewardedA", ""),
            bannerA = placementsObj.optString("bannerA", ""),
        )
        val consent = ConsentState(
            gdpr = consentObj.optBoolean("gdpr", false),
            ccpa = consentObj.optBoolean("ccpa", false),
            coppa = consentObj.optBoolean("coppa", false),
            lat = consentObj.optBoolean("lat", true),
            testMode = prefs.getBoolean(KEY_TEST_MODE, true)
        )
        return SandboxConfig(apiBase = apiBase, placements = placements, consent = consent)
    }

    fun saveConsent(consent: ConsentState) {
        prefs.edit()
            .putBoolean(KEY_GDPR, consent.gdpr)
            .putBoolean(KEY_CCPA, consent.ccpa)
            .putBoolean(KEY_COPPA, consent.coppa)
            .putBoolean(KEY_LAT, consent.lat)
            .putBoolean(KEY_TEST_MODE, consent.testMode)
            .apply()
    }

    fun loadConsentDefaults(consent: ConsentState): ConsentState {
        return consent.copy(
            gdpr = prefs.getBoolean(KEY_GDPR, consent.gdpr),
            ccpa = prefs.getBoolean(KEY_CCPA, consent.ccpa),
            coppa = prefs.getBoolean(KEY_COPPA, consent.coppa),
            lat = prefs.getBoolean(KEY_LAT, consent.lat),
            testMode = prefs.getBoolean(KEY_TEST_MODE, consent.testMode),
        )
    }

    companion object {
        private const val KEY_GDPR = "gdpr"
        private const val KEY_CCPA = "ccpa"
        private const val KEY_COPPA = "coppa"
        private const val KEY_LAT = "lat"
        private const val KEY_TEST_MODE = "testMode"
    }
}
