package com.rivalapexmediation.sdk.privacy

import android.annotation.SuppressLint
import android.content.Context
import android.os.Build

/** Snapshot of Android Privacy Sandbox / ad-servicing state sliders. */
data class PrivacySandboxSnapshot(
    val sandboxEnabled: Boolean?,
    val adIdEnabled: Boolean?
)

/**
 * Best-effort reader for the new Android Privacy Sandbox settings exposed via AdServices.
 * Falls back silently when the reflection targets are unavailable (pre-Android 13 or missing libs).
 */
internal class PrivacySandboxStateProvider(private val context: Context) {

    fun snapshot(): PrivacySandboxSnapshot {
        val state = readAdServicesState()
        val sandbox = readBooleanFlag(state, listOf(
            "isPrivacySandboxEnabled",
            "isAdServicesEnabled",
            "isTopicsEnabled",
            "isFledgeCustomAudienceEnabled"
        ))
        val adIdEnabled = readBooleanFlag(state, listOf("isAdIdEnabled"))
        return PrivacySandboxSnapshot(sandboxEnabled = sandbox, adIdEnabled = adIdEnabled)
    }

    @SuppressLint("ClassVerificationFailure", "SoonBlockedPrivateApi")
    private fun readAdServicesState(): Any? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return null
        }
        return try {
            val managerClass = Class.forName("android.adservices.common.AdServicesStateManager")
            val getInstance = managerClass.getDeclaredMethod("getInstance", Context::class.java)
            val manager = getInstance.invoke(null, context)
            val getState = managerClass.getDeclaredMethod("getAdServicesState")
            getState.invoke(manager)
        } catch (_: Throwable) {
            null
        }
    }

    private fun readBooleanFlag(state: Any?, methodNames: List<String>): Boolean? {
        if (state == null) return null
        return try {
            val stateClass = state.javaClass
            methodNames.forEach { name ->
                val method = stateClass.methods.firstOrNull { it.name == name } ?: return@forEach
                val value = method.invoke(state)
                if (value is Boolean) {
                    return value
                }
            }
            null
        } catch (_: Throwable) {
            null
        }
    }
}
