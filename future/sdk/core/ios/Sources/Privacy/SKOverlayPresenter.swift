import Foundation
#if os(iOS)
import StoreKit
import UIKit
#endif

#if os(iOS)
typealias ApexOverlayScene = UIWindowScene
#else
typealias ApexOverlayScene = AnyObject
#endif

/// Positions we support for presenting SKOverlay surfaces.
enum OverlayPosition: String {
    case bottom
    case bottomRaised

    init(metadataValue: String?) {
        switch metadataValue?.lowercased().replacingOccurrences(of: " ", with: "") {
        case "bottomraised", "raised", "bottom_raised":
            self = .bottomRaised
        default:
            self = .bottom
        }
    }

    #if os(iOS)
    @available(iOS 14.0, *)
    var storeKitPosition: SKOverlay.Position {
        switch self {
        case .bottom:
            return .bottom
        case .bottomRaised:
            return .bottomRaised
        }
    }
    #endif
}

#if os(iOS)
/// Presents SKOverlay instances on the main thread, enforcing UI safety.
@available(iOSApplicationExtension, unavailable)
@MainActor
final class SKOverlayPresenter {
    static let shared = SKOverlayPresenter()
    private init() {}

    func presentOverlay(appIdentifier: Int, scene: ApexOverlayScene?, position: OverlayPosition, dismissible: Bool) {
        guard #available(iOS 14.0, *) else { return }
        let config = SKOverlay.AppConfiguration(appIdentifier: String(appIdentifier), position: position.storeKitPosition)
        config.userDismissible = dismissible
        let overlay = SKOverlay(configuration: config)
        if let targetScene = scene ?? Self.activeScene() {
            overlay.present(in: targetScene)
        }
    }

    private static func activeScene() -> ApexOverlayScene? {
        return UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first { $0.activationState == .foregroundActive }
    }
}
#else
@MainActor
final class SKOverlayPresenter {
    static let shared = SKOverlayPresenter()
    private init() {}

    func presentOverlay(appIdentifier _: Int, scene _: ApexOverlayScene?, position _: OverlayPosition, dismissible _: Bool) {}
}
#endif
