package com.rivalapexmediation.ctv.consent

import android.content.Context
import android.content.SharedPreferences

class ConsentManager(context: Context) {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences("ctv_sdk_consent", Context.MODE_PRIVATE)

    @Volatile private var cached: ConsentData = load()

    fun setConsent(data: ConsentData) {
        cached = data
        prefs.edit()
            .putString("tcf", data.tcfString ?: "")
            .putString("usPrivacy", data.usPrivacy ?: "")
            .putBoolean("gdpr", data.gdprApplies == true)
            .putBoolean("coppa", data.coppa == true)
            .apply()
    }

    fun getConsent(): ConsentData = cached

    private fun load(): ConsentData {
        val tcf = prefs.getString("tcf", null)
        val usp = prefs.getString("usPrivacy", null)
        val gdpr = if (prefs.contains("gdpr")) prefs.getBoolean("gdpr", false) else null
        val coppa = if (prefs.contains("coppa")) prefs.getBoolean("coppa", false) else null
        return ConsentData(gdprApplies = gdpr, tcfString = tcf, usPrivacy = usp, coppa = coppa)
    }
}
