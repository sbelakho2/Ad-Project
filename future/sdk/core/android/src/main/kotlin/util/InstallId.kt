package com.rivalapexmediation.sdk.util

import android.content.Context
import android.content.SharedPreferences
import java.util.UUID

/**
 * Stable per-install ID provider (non-PII). Stored in app private SharedPreferences.
 * Used for diagnostics and staged rollout bucketing.
 */
object InstallId {
    private const val PREFS = "rival_ad_stack_install"
    private const val KEY = "install_id"

    @JvmStatic
    fun get(context: Context): String {
        val prefs: SharedPreferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        var id = prefs.getString(KEY, null)
        if (id.isNullOrBlank()) {
            id = UUID.randomUUID().toString()
            prefs.edit().putString(KEY, id).apply()
        }
        return id
    }
}
