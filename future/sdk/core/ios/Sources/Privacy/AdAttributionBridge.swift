import Foundation

#if os(iOS)
import UIKit

/// Routes attribution events to AdAttributionKit when available, otherwise falls back to SKAdNetwork.
@MainActor
final class AdAttributionBridge {
    static let shared = AdAttributionBridge()
    private init() {}

    /// Record an impression for attribution. Uses SKAdNetwork today; will route to AdAttributionKit when available.
    func recordImpression(for ad: Ad, presentingScene: ApexOverlayScene? = nil) {
        if supportsAdAttributionKit {
            // AdAttributionKit is not yet linked in this SDK build; keep a no-op for forward compatibility.
            // Future: invoke AdAttribution API with source/trigger payload derived from ad.metadata.
        }
        SKAdNetworkCoordinator.shared.recordImpression(for: ad, presentingScene: presentingScene)
    }

    private var supportsAdAttributionKit: Bool {
        if #available(iOS 17.2, *) {
            // Detect presence without linking to avoid build failures when the framework is absent.
            return NSClassFromString("ADAttribution") != nil || NSClassFromString("AdAttribution") != nil
        }
        return false
    }
}
#else
@MainActor
final class AdAttributionBridge {
    static let shared = AdAttributionBridge()
    private init() {}
    func recordImpression(for ad: Ad, presentingScene: ApexOverlayScene? = nil) {}
}
#endif
