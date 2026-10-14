import Foundation

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
 * 1. Copy to your own module/framework
 * 2. Add Meta Audience Network SDK via SPM/CocoaPods
 * 3. Remove the #if DEBUG guard
 * 4. Register via MediationSDK.shared.registerAdapter(...)
 *
 * ============================================================================
 */

#if DEBUG || APEX_SANDBOX_ADAPTERS

import UIKit
import FBAudienceNetwork

/// Meta Audience Network Adapter (SAMPLE)
///
/// ⚠️ REFERENCE IMPLEMENTATION ONLY - NOT FOR PRODUCTION USE
///
/// Implements the V2 adapter contract for integration with the RivalApex mediation SDK.
/// Supports: Banner, Interstitial, Rewarded
/// S2S Bidding: Yes
/// Signal Collection: Yes
public final class MetaAdapterSample: NSObject, AdNetworkAdapterV2 {
    
    public static let networkName = "meta"
    public static let adapterVersion = "1.0.0-sample"
    
    private static var isInitialized = false
    
    // Store loaded ads by handle ID
    private var loadedInterstitials: [String: FBInterstitialAd] = [:]
    private var loadedRewardedAds: [String: FBRewardedVideoAd] = [:]
    private var loadedBanners: [String: FBAdView] = [:]
    private var handleExpiry: [String: Int64] = [:]
    
    private var placementMap: [String: String] = [:]
    private let lock = NSLock()
    
    // MARK: - Initialization
    
    public func initAdapter(config: PartnerAdapterConfig, timeoutMs: Int) -> InitResult {
        if Self.isInitialized {
            return InitResult(success: true, error: nil, partnerMeta: [:])
        }
        
        placementMap = config.placements
        
        // Apply privacy settings
        applyPrivacySettings(config.privacy)
        
        // Initialize Meta SDK
        var initResult: InitResult?
        let semaphore = DispatchSemaphore(value: 0)
        
        DispatchQueue.main.async {
            FBAudienceNetworkAds.initialize(with: nil) { result in
                if result.isSuccess {
                    Self.isInitialized = true
                    initResult = InitResult(success: true, error: nil, partnerMeta: [:])
                } else {
                    initResult = InitResult(
                        success: false,
                        error: AdapterError(
                            code: .config,
                            detail: result.message ?? "Meta SDK initialization failed",
                            vendorCode: nil,
                            recoverable: false
                        ),
                        partnerMeta: [:]
                    )
                }
                semaphore.signal()
            }
        }
        
        let timeout = DispatchTime.now() + .milliseconds(timeoutMs)
        let result = semaphore.wait(timeout: timeout)
        
        if result == .timedOut {
            return InitResult(
                success: false,
                error: AdapterError(code: .timeout, detail: "Meta initialization timed out", vendorCode: nil, recoverable: true),
                partnerMeta: [:]
            )
        }
        
        return initResult ?? InitResult(
            success: false,
            error: AdapterError(code: .error, detail: "Unknown error", vendorCode: nil, recoverable: false),
            partnerMeta: [:]
        )
    }
    
    // MARK: - Interstitial
    
    public func loadInterstitial(placementId: String, meta: RequestMeta, timeoutMs: Int) -> LoadResult {
        let metaPlacementId = placementMap[placementId] ?? placementId
        let handleId = UUID().uuidString
        let ttlMs = 60 * 60 * 1000 // 1 hour
        
        var loadResult: LoadResult?
        var loadError: AdapterError?
        let semaphore = DispatchSemaphore(value: 0)
        
        DispatchQueue.main.async { [weak self] in
            guard let self = self else {
                loadError = AdapterError(code: .error, detail: "Adapter deallocated", vendorCode: nil, recoverable: false)
                semaphore.signal()
                return
            }
            
            let interstitialAd = FBInterstitialAd(placementID: metaPlacementId)
            interstitialAd.delegate = self
            
            // Note: Full implementation would store callbacks and handle delegate methods
            // This is simplified for sample purposes
            
            interstitialAd.load()
            
            // For sample: simulate successful load after delay
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                self.lock.lock()
                self.loadedInterstitials[handleId] = interstitialAd
                self.handleExpiry[handleId] = Int64(Date().timeIntervalSince1970 * 1000) + Int64(ttlMs)
                self.lock.unlock()
                
                loadResult = LoadResult(
                    handle: AdHandle(id: handleId, adType: .interstitial, partnerPlacementId: metaPlacementId),
                    ttlMs: ttlMs,
                    priceMicros: nil,
                    currency: nil,
                    partnerMeta: [:]
                )
                semaphore.signal()
            }
        }
        
        let timeout = DispatchTime.now() + .milliseconds(timeoutMs)
        let result = semaphore.wait(timeout: timeout)
        
        if result == .timedOut {
            throw AdapterError(code: .timeout, detail: "Interstitial load timed out", vendorCode: nil, recoverable: true)
        }
        
        if let error = loadError {
            throw error
        }
        
        return loadResult ?? {
            throw AdapterError(code: .noFill, detail: "No ad loaded", vendorCode: nil, recoverable: true)
        }()
    }
    
    public func showInterstitial(handle: AdHandle, viewController: AnyObject, callbacks: ShowCallbacks) {
        lock.lock()
        let interstitialAd = loadedInterstitials[handle.id]
        lock.unlock()
        
        guard let ad = interstitialAd, ad.isAdValid else {
            callbacks.onError(AdapterError(code: .noAdReady, detail: "Interstitial not ready", vendorCode: nil, recoverable: false))
            return
        }
        
        guard let vc = viewController as? UIViewController else {
            callbacks.onError(AdapterError(code: .config, detail: "viewController must be UIViewController", vendorCode: nil, recoverable: false))
            return
        }
        
        ad.show(fromRootViewController: vc)
    }
    
    // MARK: - Rewarded (stub)
    
    public func loadRewarded(placementId: String, meta: RequestMeta, timeoutMs: Int) -> LoadResult {
        throw AdapterError(code: .unsupported, detail: "See full sample for rewarded implementation", vendorCode: nil, recoverable: false)
    }
    
    public func showRewarded(handle: AdHandle, viewController: AnyObject, callbacks: RewardedCallbacks) {
        callbacks.onError(AdapterError(code: .unsupported, detail: "See full sample for rewarded implementation", vendorCode: nil, recoverable: false))
    }
    
    // MARK: - Banner (stub)
    
    public func loadBanner(placementId: String, size: AdSize, meta: RequestMeta, timeoutMs: Int) -> LoadResult {
        throw AdapterError(code: .unsupported, detail: "See full sample for banner implementation", vendorCode: nil, recoverable: false)
    }
    
    public func attachBanner(handle: AdHandle, hostView: AnyObject, callbacks: BannerCallbacks) {
        callbacks.onError(AdapterError(code: .unsupported, detail: "See full sample for banner implementation", vendorCode: nil, recoverable: false))
    }
    
    // MARK: - Handle Management
    
    public func isAdReady(handle: AdHandle) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        
        switch handle.adType {
        case .interstitial:
            return loadedInterstitials[handle.id]?.isAdValid == true
        case .rewarded:
            return loadedRewardedAds[handle.id]?.isAdValid == true
        case .banner:
            return loadedBanners[handle.id] != nil
        default:
            return false
        }
    }
    
    public func expiresAt(handle: AdHandle) -> Int64 {
        lock.lock()
        defer { lock.unlock() }
        return handleExpiry[handle.id] ?? 0
    }
    
    public func invalidate(handle: AdHandle) {
        lock.lock()
        defer { lock.unlock() }
        
        switch handle.adType {
        case .interstitial:
            loadedInterstitials.removeValue(forKey: handle.id)
        case .rewarded:
            loadedRewardedAds.removeValue(forKey: handle.id)
        case .banner:
            loadedBanners.removeValue(forKey: handle.id)?.removeFromSuperview()
        default:
            break
        }
        handleExpiry.removeValue(forKey: handle.id)
    }
    
    // MARK: - Signal Collection
    
    public func collectBidderToken() -> String? {
        return FBAdSettings.bidderToken
    }
    
    // MARK: - Privacy
    
    private func applyPrivacySettings(_ privacy: ConsentState) {
        if privacy.gdprApplies == true {
            FBAdSettings.setDataProcessingOptions(["LDU"], country: 0, state: 0)
        } else {
            FBAdSettings.setDataProcessingOptions([])
        }
        
        if privacy.coppa {
            FBAdSettings.setMixedAudience(true)
        }
    }
}

// MARK: - FBInterstitialAdDelegate

extension MetaAdapterSample: FBInterstitialAdDelegate {
    public func interstitialAdDidLoad(_ interstitialAd: FBInterstitialAd) {
        // Handle load success
    }
    
    public func interstitialAd(_ interstitialAd: FBInterstitialAd, didFailWithError error: Error) {
        // Handle load failure
    }
    
    public func interstitialAdWillLogImpression(_ interstitialAd: FBInterstitialAd) {
        // Handle impression
    }
    
    public func interstitialAdDidClick(_ interstitialAd: FBInterstitialAd) {
        // Handle click
    }
    
    public func interstitialAdDidClose(_ interstitialAd: FBInterstitialAd) {
        // Handle close
    }
}

#endif // DEBUG || APEX_SANDBOX_ADAPTERS
