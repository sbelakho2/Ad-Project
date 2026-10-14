#if canImport(UIKit)
import Foundation
import Dispatch
import UIKit

/// Public facade for App Open ads, mirroring Android's BelAppOpen API.
/// Displayed when app transitions from background to foreground.
@MainActor
public enum BelAppOpen {
    private static var lastPlacement: String?
    private static var lastShowMonotonic: TimeInterval?
    private static let minimumHoursBetweenShows: TimeInterval = 4 * 3600 // 4 hours
    
    /// Load an app open ad for the placement.
    public static func load(placementId: String, listener: BelAdEventListener? = nil) {
        lastPlacement = placementId
        Task { @MainActor in
            do {
                guard let ad = try await MediationSDK.shared.loadAd(placementId: placementId) else {
                    throw SDKError.noFill
                }
                listener?.onAdLoaded(placementId: placementId)
                debugPrint("[BelAppOpen] Cached app open ad from \(ad.networkName) for placement \(placementId)")
            } catch {
                listener?.onAdFailedToLoad(placementId: placementId, error: error)
            }
        }
    }
    
    /// Show the loaded app open ad using the optional listener callbacks.
    @discardableResult
    public static func show(from viewController: UIViewController, placementId: String? = nil, listener: BelAdEventListener? = nil) -> Bool {
        if let lastShow = lastShowMonotonic {
            let timeSinceLastShow = Clock.shared.monotonicSeconds() - lastShow
            if timeSinceLastShow < minimumHoursBetweenShows {
                listener?.onAdFailedToShow(placementId: placementId ?? lastPlacement ?? "", error: SDKError.frequencyLimited)
                return false
            }
        }
        
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
            lastShowMonotonic = Clock.shared.monotonicSeconds()
            listener?.onAdShown(placementId: placement)
            AdAttributionBridge.shared.recordImpression(
                for: ad,
                presentingScene: viewController.view.window?.windowScene
            )
            presentDebugAppOpen(from: viewController, networkName: ad.networkName) {
                AdPresentationCoordinator.shared.finishPresentation(presentationToken)
                listener?.onAdClosed(placementId: placement)
            }
        }
        return true
    }
    
    /// Check if app open ad is loaded and ready
    public static func isReady(placementId: String? = nil) -> Bool {
        guard let placement = placementId ?? lastPlacement else { return false }
        return MediationSDK.shared.isAdReady(placementId: placement)
    }
    
    /// Reset the show time limit (for testing purposes)
    public static func resetShowTimeLimit() {
        lastShowMonotonic = nil
    }
    
    private static func presentDebugAppOpen(from viewController: UIViewController, networkName: String, completion: @escaping () -> Void) {
        #if DEBUG
        let vc = UIViewController()
        vc.view.backgroundColor = .belDebugBackground
        vc.modalPresentationStyle = .fullScreen
        
        let label = UILabel(frame: .zero)
        label.text = "App Open Ad (debug)\n\(networkName)"
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
            AdPresentationCoordinator.shared.schedule(after: 2.0) { [weak vc] in
                guard let placeholder = vc else {
                    completion()
                    return
                }
                placeholder.dismiss(animated: true, completion: completion)
            }
        }
        #else
        AdPresentationCoordinator.shared.schedule(after: 0.5, perform: completion)
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
