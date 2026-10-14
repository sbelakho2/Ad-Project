package com.rivalapexmediation.sdk

import android.app.Activity
import android.content.Context
import com.rivalapexmediation.sdk.logging.Logger
import com.rivalapexmediation.sdk.models.Ad
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.runtime.AdPresentationCoordinator

/**
 * Simple, stable public API for App Open ads (splash-style).
 * Reuses MediationSDK caching and mirrors Interstitial ergonomics.
 */
object BelAppOpen {
    private const val TAG = "BelAppOpen"
    @Volatile private var lastPlacement: String? = null

    @JvmStatic
    @Suppress("UNUSED_PARAMETER")
    fun load(context: Context, placementId: String, listener: AdLoadCallback) {
        lastPlacement = placementId
        MediationSDK.getInstance().loadAd(placementId, listener)
    }

    /** Attempts to show the last loaded App Open ad. Returns true if shown. */
    @JvmStatic
    fun show(activity: Activity): Boolean {
        val placement = lastPlacement ?: return false
        val sdk = MediationSDK.getInstance()
        val preview = sdk.getCachedAd(placement) ?: return false
        val requestType = if (preview.adType == AdType.APP_OPEN) AdType.APP_OPEN else AdType.INTERSTITIAL
        return AdPresentationCoordinator.begin(activity, placement, requestType) { hostActivity ->
            val ad: Ad = sdk.consumeCachedAd(placement) ?: run {
                Logger.w(TAG, "cached ad missing when attempting to present placement=$placement")
                return@begin false
            }
            if (sdk.renderAd(ad, hostActivity)) {
                return@begin true
            }
            if (ad.adType != AdType.APP_OPEN && ad.adType != AdType.INTERSTITIAL) {
                Logger.d(TAG, "non app-open ad fallback for placement=$placement, type=${ad.adType}")
            }
            try {
                com.rivalapexmediation.sdk.measurement.OmSdkRegistry.controller.startDisplaySession(
                    hostActivity, placement, ad.networkName, "app_open"
                )
            } catch (_: Throwable) {}
            ad.show(hostActivity)
            try { com.rivalapexmediation.sdk.measurement.OmSdkRegistry.controller.endSession(placement) } catch (_: Throwable) {}
            true
        }
    }

    @JvmStatic
    fun isReady(): Boolean {
        val placement = lastPlacement ?: return false
        return MediationSDK.getInstance().isAdReady(placement)
    }
}
