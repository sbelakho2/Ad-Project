#if canImport(UIKit)
import Foundation
import Dispatch
import UIKit

/// Rewarded interstitial facade with shared cache integration and lifecycle listener support.
@MainActor
public enum BelRewardedInterstitial {
    private static var lastPlacement: String?

    /// Load a rewarded interstitial ad for the placement.
    public static func load(placementId: String, listener: BelAdEventListener? = nil) {
        lastPlacement = placementId
        Task { @MainActor in
            do {
                guard let ad = try await MediationSDK.shared.loadAd(placementId: placementId) else {
                    throw SDKError.noFill
                }
                listener?.onAdLoaded(placementId: placementId)
                debugPrint("[BelRewardedInterstitial] Cached ad from \(ad.networkName) for placement \(placementId)")
            } catch {
                listener?.onAdFailedToLoad(placementId: placementId, error: error)
            }
        }
    }

    /// Show the loaded rewarded interstitial using the optional listener callbacks.
    @discardableResult
    public static func show(from viewController: UIViewController, placementId: String? = nil, listener: BelAdEventListener? = nil) -> Bool {
        let targetPlacement = placementId ?? lastPlacement
        guard let placement = targetPlacement else {
            listener?.onAdFailedToShow(placementId: "", error: SDKError.invalidPlacement("missing_placement"))
            return false
        }

        guard MediationSDK.shared.isAdReady(placementId: placement) else {
            listener?.onAdFailedToShow(placementId: placement, error: SDKError.noFill)
            return false
        }

        let presentationToken: AdPresentationCoordinator.Token
        do {
            presentationToken = try AdPresentationCoordinator.shared.beginPresentation(placementId: placement)
        } catch {
            listener?.onAdFailedToShow(placementId: placement, error: error)
            return false
        }

        Task { @MainActor in
            guard let ad = await MediationSDK.shared.claimAd(placementId: placement) else {
                AdPresentationCoordinator.shared.finishPresentation(presentationToken)
                listener?.onAdFailedToShow(placementId: placement, error: SDKError.noFill)
                return
            }
            listener?.onAdShown(placementId: placement)
            AdAttributionBridge.shared.recordImpression(
                for: ad,
                presentingScene: viewController.view.window?.windowScene
            )
            presentDebugRewardedInterstitial(from: viewController, networkName: ad.networkName) { didEarnReward in
                if didEarnReward {
                    listener?.onUserEarnedReward(placementId: placement, reward: BelReward(label: "reward", amount: 1))
                }
                AdPresentationCoordinator.shared.finishPresentation(presentationToken)
                listener?.onAdClosed(placementId: placement)
            }
        }
        return true
    }

    /// Check if a rewarded interstitial is cached and ready.
    public static func isReady(placementId: String? = nil) -> Bool {
        guard let placement = placementId ?? lastPlacement else { return false }
        return MediationSDK.shared.isAdReady(placementId: placement)
    }

    private static func presentDebugRewardedInterstitial(from viewController: UIViewController, networkName: String, completion: @escaping (Bool) -> Void) {
        #if DEBUG
        let alert = UIAlertController(
            title: "Rewarded Interstitial (debug)",
            message: "Watch video to earn reward?\nNetwork: \(networkName)",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "Watch (Reward)", style: .default) { _ in
            completion(true)
        })
        alert.addAction(UIAlertAction(title: "Close (No Reward)", style: .cancel) { _ in
            completion(false)
        })
        viewController.present(alert, animated: true)
        #else
        AdPresentationCoordinator.shared.schedule(after: 1.0) {
            completion(true)
        }
        #endif
    }
}

#endif
