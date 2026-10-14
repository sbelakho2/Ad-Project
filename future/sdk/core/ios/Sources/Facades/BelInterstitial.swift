#if canImport(UIKit)
import Foundation
import Dispatch
import UIKit

/// Public facade for interstitial ads with lifecycle callbacks routed through BelAdEventListener.
@MainActor
public enum BelInterstitial {
    private static var lastPlacement: String?

    /// Load an interstitial for the placement. The result is cached inside MediationSDK.
    public static func load(placementId: String, listener: BelAdEventListener? = nil) {
        lastPlacement = placementId
        Task { @MainActor in
            do {
                guard let ad = try await MediationSDK.shared.loadAd(placementId: placementId) else {
                    throw SDKError.noFill
                }
                listener?.onAdLoaded(placementId: placementId)
                debugPrint("[BelInterstitial] Cached interstitial from \(ad.networkName) for placement \(placementId)")
            } catch {
                listener?.onAdFailedToLoad(placementId: placementId, error: error)
            }
        }
    }

    /// Show a cached interstitial. Returns true if a cached ad was consumed.
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
            presentDebugPlaceholder(from: viewController, networkName: ad.networkName) {
                AdPresentationCoordinator.shared.finishPresentation(presentationToken)
                listener?.onAdClosed(placementId: placement)
            }
        }
        return true
    }

    /// Whether an ad is cached for the placement (defaults to last loaded placement when omitted).
    public static func isReady(placementId: String? = nil) -> Bool {
        guard let placement = placementId ?? lastPlacement else { return false }
        return MediationSDK.shared.isAdReady(placementId: placement)
    }

    private static func presentDebugPlaceholder(from viewController: UIViewController, networkName: String, completion: @escaping () -> Void) {
        #if DEBUG
        let vc = UIViewController()
        vc.view.backgroundColor = .belDebugBackground
        let label = UILabel(frame: .zero)
        label.text = "Showing interstitial placeholder — network=\(networkName)"
        label.textAlignment = .center
        label.numberOfLines = 0
        label.translatesAutoresizingMaskIntoConstraints = false
        vc.view.addSubview(label)
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: vc.view.centerXAnchor),
            label.centerYAnchor.constraint(equalTo: vc.view.centerYAnchor),
            label.leadingAnchor.constraint(greaterThanOrEqualTo: vc.view.leadingAnchor, constant: 16),
            label.trailingAnchor.constraint(lessThanOrEqualTo: vc.view.trailingAnchor, constant: -16)
        ])
        viewController.present(vc, animated: true) {
            AdPresentationCoordinator.shared.schedule(after: 0.5) { [weak vc] in
                guard let placeholder = vc else {
                    completion()
                    return
                }
                placeholder.dismiss(animated: true, completion: completion)
            }
        }
        #else
        completion()
        #endif
    }
}

#endif

#if canImport(UIKit)
private extension UIColor {
    /// Provides a debug background color that works on tvOS and iOS.
    static var belDebugBackground: UIColor {
        #if os(tvOS)
        return UIColor(red: 0.08, green: 0.08, blue: 0.1, alpha: 1.0)
        #else
        if #available(iOS 13.0, *) {
            return .systemBackground
        }
        return .white
        #endif
    }
}
#endif
