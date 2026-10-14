package com.rivalapexmediation.sdk

import android.app.Activity
import android.content.Context
import androidx.annotation.NonNull
import com.rivalapexmediation.sdk.logging.Logger
import com.rivalapexmediation.sdk.models.Ad
import com.rivalapexmediation.sdk.models.AdType
import com.rivalapexmediation.sdk.runtime.AdPresentationCoordinator

/**
 * Simple, stable public API for Rewarded Interstitials.
 * Mirrors BelRewarded/BelInterstitial ergonomics and uses the internal cache.
 */
object BelRewardedInterstitial {
    private const val TAG = "BelRewardedInterstitial"
    @Volatile private var lastPlacement: String? = null

    @JvmStatic
    @Suppress("UNUSED_PARAMETER")
    fun load(@NonNull context: Context, @NonNull placementId: String, @NonNull listener: AdLoadCallback) {
        lastPlacement = placementId
        MediationSDK.getInstance().loadAd(placementId, listener)
    }

    /** Attempts to show the last loaded ad. Returns true if an ad was shown. */
    @JvmStatic
    fun show(@NonNull activity: Activity): Boolean {
        val placement = lastPlacement ?: return false
        val sdk = MediationSDK.getInstance()
        val preview = sdk.getCachedAd(placement) ?: return false
        return AdPresentationCoordinator.begin(activity, placement, preview.adType) { hostActivity ->
            val ad: Ad = sdk.consumeCachedAd(placement) ?: run {
                Logger.w(TAG, "cached ad missing when attempting to present placement=$placement")
                return@begin false
            }
            if (sdk.renderAd(ad, hostActivity)) {
                return@begin true
            }
            try {
                com.rivalapexmediation.sdk.measurement.OmSdkRegistry.controller.startVideoSession(
                    hostActivity, placement, ad.networkName, durationSec = null
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
