@file:Suppress("ConstantConditionIf")
package com.rivalapexmediation.sdk.adapters.samples.meta

/*
 * ============================================================================
 * SAMPLE ADAPTER - FOR REFERENCE ONLY
 * ============================================================================
 * 
 * This is a SAMPLE implementation demonstrating how to build a Meta Audience
 * Network adapter for the ApexMediation SDK. It is NOT intended for production
 * use and is gated behind DEBUG/sandbox compile flags.
 * 
 * BYO-First Architecture:
 * - Publishers must implement their own adapters
 * - Publishers must add their own vendor SDK dependencies  
 * - This sample is for educational/reference purposes only
 * 
 * To use this as a starting point:
 * 1. Copy to your own module
 * 2. Add Meta Audience Network SDK dependency
 * 3. Remove the #if DEBUG guard
 * 4. Register via MediationSDK.registerRuntimeAdapterFactory(...)
 * 
 * ============================================================================
 */

import android.app.Activity
import android.os.Build

// Gate this entire file to sandbox/debug builds only
private val SANDBOX_ENABLED = BuildConfig.DEBUG || 
    System.getProperty("apex.sandbox.adapters") == "1"

// The following code only compiles/executes in sandbox mode
// In release builds, this class should not be included in the build

#if false
// ============================================================================
// BEGIN SAMPLE IMPLEMENTATION (disabled in release)
// ============================================================================

import com.rivalapexmediation.sdk.contract.*
import com.rivalapexmediation.sdk.models.AdType
import com.facebook.ads.*
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * Meta Audience Network Adapter (SAMPLE)
 * 
 * ⚠️ REFERENCE IMPLEMENTATION ONLY - NOT FOR PRODUCTION USE
 * 
 * Implements the V2 adapter contract for integration with the RivalApex mediation SDK.
 * Supports: Banner, Interstitial, Rewarded
 * S2S Bidding: Yes
 * Signal Collection: Yes
 */
class MetaAdapterSample : AdNetworkAdapterV2 {
    
    companion object {
        const val NETWORK_NAME = "meta"
        const val ADAPTER_VERSION = "1.0.0-sample"
        private var isInitialized = false
    }
    
    // Store loaded ads by handle ID
    private val loadedInterstitials = ConcurrentHashMap<String, InterstitialAd>()
    private val loadedRewardedAds = ConcurrentHashMap<String, RewardedVideoAd>()
    private val loadedBanners = ConcurrentHashMap<String, AdView>()
    private val handleExpiry = ConcurrentHashMap<String, Long>()
    
    private var placementMap: Map<String, String> = emptyMap()
    
    // MARK: - Initialization
    
    override fun init(config: AdapterConfig, timeoutMs: Int): InitResult {
        if (isInitialized) {
            return InitResult(success = true)
        }
        
        return try {
            placementMap = config.placements
            
            // Apply privacy settings
            if (config.privacy.gdprApplies == true) {
                AdSettings.setDataProcessingOptions(arrayOf("LDU"), 1, 1000)
            } else {
                AdSettings.setDataProcessingOptions(arrayOf())
            }
            
            if (config.options?.testMode == true) {
                AdSettings.addTestDevice("DEVICE_ID")
            }
            
            isInitialized = true
            InitResult(success = true)
        } catch (e: Exception) {
            InitResult(
                success = false,
                error = AdapterError.Fatal(
                    code = ErrorCode.CONFIG,
                    detail = e.message ?: "Meta SDK initialization failed"
                )
            )
        }
    }
    
    // MARK: - Interstitial
    
    override fun loadInterstitial(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult {
        val metaPlacementId = placementMap[placementId] ?: placementId
        val handleId = UUID.randomUUID().toString()
        val ttlMs = 60 * 60 * 1000 // 1 hour TTL
        
        var loadResult: LoadResult? = null
        var loadError: AdapterError? = null
        val latch = java.util.concurrent.CountDownLatch(1)
        
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            try {
                val context = android.app.Application.getProcessName() as android.content.Context
                val interstitialAd = InterstitialAd(context, metaPlacementId)
                
                interstitialAd.loadAd(
                    interstitialAd.buildLoadAdConfig()
                        .withAdListener(object : InterstitialAdListener {
                            override fun onAdLoaded(ad: Ad) {
                                loadedInterstitials[handleId] = interstitialAd
                                handleExpiry[handleId] = System.currentTimeMillis() + ttlMs
                                
                                loadResult = LoadResult(
                                    handle = AdHandle(
                                        id = handleId,
                                        adType = AdType.INTERSTITIAL,
                                        partnerPlacementId = metaPlacementId
                                    ),
                                    ttlMs = ttlMs
                                )
                                latch.countDown()
                            }
                            
                            override fun onError(ad: Ad?, error: com.facebook.ads.AdError) {
                                loadError = mapMetaError(error)
                                latch.countDown()
                            }
                            
                            override fun onInterstitialDisplayed(ad: Ad) {}
                            override fun onInterstitialDismissed(ad: Ad) {}
                            override fun onAdClicked(ad: Ad) {}
                            override fun onLoggingImpression(ad: Ad) {}
                        })
                        .build()
                )
            } catch (e: Exception) {
                loadError = AdapterError.Fatal(ErrorCode.ERROR, e.message ?: "Load failed")
                latch.countDown()
            }
        }
        
        val completed = latch.await(timeoutMs.toLong(), java.util.concurrent.TimeUnit.MILLISECONDS)
        
        if (!completed) {
            throw AdapterError.Recoverable(ErrorCode.TIMEOUT, "Interstitial load timed out")
        }
        
        loadError?.let { throw it }
        return loadResult ?: throw AdapterError.Fatal(ErrorCode.NO_FILL, "No ad loaded")
    }
    
    override fun showInterstitial(handle: AdHandle, viewContext: Any, callbacks: ShowCallbacks) {
        val interstitialAd = loadedInterstitials[handle.id]
        
        if (interstitialAd == null || !interstitialAd.isAdLoaded) {
            callbacks.onError(AdapterError.Fatal(ErrorCode.NO_AD_READY, "Interstitial not ready"))
            return
        }
        
        if (viewContext !is Activity) {
            callbacks.onError(AdapterError.Fatal(ErrorCode.CONFIG, "viewContext must be Activity"))
            return
        }
        
        interstitialAd.show()
    }
    
    // MARK: - Rewarded (stub - implement similar to interstitial)
    
    override fun loadRewarded(placementId: String, meta: RequestMeta, timeoutMs: Int): LoadResult {
        throw AdapterError.Fatal(ErrorCode.UNSUPPORTED, "See full sample for rewarded implementation")
    }
    
    override fun showRewarded(handle: AdHandle, viewContext: Any, callbacks: RewardedCallbacks) {
        callbacks.onError(AdapterError.Fatal(ErrorCode.UNSUPPORTED, "See full sample for rewarded implementation"))
    }
    
    // MARK: - Banner (stub - implement similar to interstitial)
    
    override fun loadBanner(placementId: String, size: AdSize, meta: RequestMeta, timeoutMs: Int): LoadResult {
        throw AdapterError.Fatal(ErrorCode.UNSUPPORTED, "See full sample for banner implementation")
    }
    
    override fun attachBanner(handle: AdHandle, bannerHost: Any, callbacks: BannerCallbacks) {
        callbacks.onError(AdapterError.Fatal(ErrorCode.UNSUPPORTED, "See full sample for banner implementation"))
    }
    
    // MARK: - Handle Management
    
    override fun isAdReady(handle: AdHandle): Boolean {
        return when (handle.adType) {
            AdType.INTERSTITIAL -> loadedInterstitials[handle.id]?.isAdLoaded == true
            AdType.REWARDED -> loadedRewardedAds[handle.id]?.isAdLoaded == true
            AdType.BANNER -> loadedBanners[handle.id] != null
            else -> false
        }
    }
    
    override fun expiresAt(handle: AdHandle): Long {
        return handleExpiry[handle.id] ?: 0L
    }
    
    override fun invalidate(handle: AdHandle) {
        when (handle.adType) {
            AdType.INTERSTITIAL -> loadedInterstitials.remove(handle.id)?.destroy()
            AdType.REWARDED -> loadedRewardedAds.remove(handle.id)?.destroy()
            AdType.BANNER -> loadedBanners.remove(handle.id)?.destroy()
            else -> {}
        }
        handleExpiry.remove(handle.id)
    }
    
    // MARK: - Signal Collection (Extension)
    
    fun collectBidderToken(context: android.content.Context): String? {
        return try {
            BidderTokenProvider.getBidderToken(context)
        } catch (e: Exception) {
            null
        }
    }
    
    // MARK: - Error Mapping
    
    private fun mapMetaError(error: com.facebook.ads.AdError): AdapterError {
        return when (error.errorCode) {
            AdError.NO_FILL_ERROR_CODE -> AdapterError.Recoverable(
                ErrorCode.NO_FILL,
                error.errorMessage,
                error.errorCode.toString()
            )
            AdError.NETWORK_ERROR_CODE -> AdapterError.Recoverable(
                ErrorCode.NETWORK_ERROR,
                error.errorMessage,
                error.errorCode.toString()
            )
            else -> AdapterError.Fatal(
                ErrorCode.ERROR,
                error.errorMessage,
                error.errorCode.toString()
            )
        }
    }
}

// ============================================================================
// END SAMPLE IMPLEMENTATION
// ============================================================================
#endif
