#if canImport(UIKit)
import Foundation
import Dispatch
import UIKit

/// Rewarded facade that bridges to MediationSDK caching and BelAdEventListener callbacks.
@MainActor
public enum BelRewarded {
    private static var lastPlacement: String?

    /// Load a rewarded ad for the placement. Success/failure events flow through the optional listener.
    public static func load(placementId: String, listener: BelAdEventListener? = nil) {
        lastPlacement = placementId
        Task { @MainActor in
            do {
                guard let ad = try await MediationSDK.shared.loadAd(placementId: placementId) else {
                    throw SDKError.noFill
                }
                listener?.onAdLoaded(placementId: placementId)
                debugPrint("[BelRewarded] Cached rewarded ad from \(ad.networkName) for placement \(placementId)")
            } catch {
                listener?.onAdFailedToLoad(placementId: placementId, error: error)
            }
        }
    }

    /// Attempts to show a cached rewarded ad. Returns true if we scheduled a presentation.
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
            presentDebugRewarded(from: viewController, networkName: ad.networkName) { didEarnReward in
                if didEarnReward {
                    listener?.onUserEarnedReward(placementId: placement, reward: BelReward(label: "reward", amount: 1))
                }
                AdPresentationCoordinator.shared.finishPresentation(presentationToken)
                listener?.onAdClosed(placementId: placement)
            }
        }
        return true
    }

    /// Whether an ad is cached for the provided placement (defaults to last loaded placement).
    public static func isReady(placementId: String? = nil) -> Bool {
        guard let placement = placementId ?? lastPlacement else { return false }
        return MediationSDK.shared.isAdReady(placementId: placement)
    }

    private static func presentDebugRewarded(from viewController: UIViewController, networkName: String, completion: @escaping (Bool) -> Void) {
        #if DEBUG
        let alert = UIAlertController(
            title: "Rewarded (debug placeholder)",
            message: "Simulating rewarded show for network: \(networkName)",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "Watch", style: .default) { _ in
            completion(true)
        })
        alert.addAction(UIAlertAction(title: "Close", style: .cancel) { _ in
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
