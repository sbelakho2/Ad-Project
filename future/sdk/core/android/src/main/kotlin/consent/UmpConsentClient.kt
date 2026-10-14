package com.rivalapexmediation.sdk.consent

import android.app.Activity
import android.content.Context
import com.google.android.ump.ConsentInformation
import com.google.android.ump.ConsentRequestParameters
import com.google.android.ump.UserMessagingPlatform

/**
 * Thin wrapper around Google's UMP SDK to keep the public API dependency-free.
 * Host apps can invoke this to request/update consent, while the SDK persists the normalized state.
 */
class UmpConsentClient(private val activity: Activity) {

    data class Params(
        val tagForUnderAgeOfConsent: Boolean = false
    )

    interface Callback {
        fun onConsentUpdated(state: ConsentManager.State)
        fun onConsentUpdateFailed(throwable: Throwable)
    }

    fun request(params: Params = Params(), callback: Callback) {
        val requestParams = ConsentRequestParameters.Builder().apply {
            if (params.tagForUnderAgeOfConsent) {
                setTagForUnderAgeOfConsent(true)
            }
        }.build()
        val consentInfo = UserMessagingPlatform.getConsentInformation(activity)
        consentInfo.requestConsentInfoUpdate(
            activity,
            requestParams,
            { handleConsentInfo(consentInfo, callback) },
            { error -> callback.onConsentUpdateFailed(IllegalStateException(error.message ?: "ump_error")) }
        )
    }

    private fun handleConsentInfo(
        consentInfo: ConsentInformation,
        callback: Callback
    ) {
        if (consentInfo.isConsentFormAvailable) {
            UserMessagingPlatform.loadConsentForm(
                activity,
                { form ->
                    form.show(activity) {
                        emitSnapshot(consentInfo, callback)
                    }
                },
                { error -> callback.onConsentUpdateFailed(IllegalStateException(error.message ?: "ump_form_error")) }
            )
        } else {
            emitSnapshot(consentInfo, callback)
        }
    }

    private fun emitSnapshot(
        consentInfo: ConsentInformation,
        callback: Callback
    ) {
        try {
            val updated = snapshotState(activity.applicationContext, consentInfo)
            callback.onConsentUpdated(updated)
        } catch (t: Throwable) {
            callback.onConsentUpdateFailed(t)
        }
    }

    private fun snapshotState(
        context: Context,
        consentInfo: ConsentInformation
    ): ConsentManager.State {
        val stored = runCatching { ConsentManager.fromIabStorage(context) }.getOrDefault(ConsentManager.State())
        val gdprApplies = when (consentInfo.consentStatus) {
            ConsentInformation.ConsentStatus.REQUIRED -> true
            ConsentInformation.ConsentStatus.OBTAINED -> stored.gdprApplies ?: true
            ConsentInformation.ConsentStatus.NOT_REQUIRED -> false
            else -> stored.gdprApplies
        }
        val limitAdTracking = stored.limitAdTracking
        val consentString = readConsentString(consentInfo) ?: stored.tcfString
        return stored.copy(
            gdprApplies = gdprApplies,
            tcfString = consentString,
            limitAdTracking = limitAdTracking
        )
    }

    private fun readConsentString(consentInfo: ConsentInformation): String? {
        return try {
            val method = consentInfo.javaClass.methods.firstOrNull { it.name == "getConsentString" }
                ?: return null
            method.invoke(consentInfo) as? String
        } catch (_: Throwable) {
            null
        }
    }
}
