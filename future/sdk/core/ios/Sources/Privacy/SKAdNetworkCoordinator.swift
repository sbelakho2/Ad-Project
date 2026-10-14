import Foundation

#if os(iOS)
import StoreKit
import UIKit

/// Handles SKAdNetwork registration, conversion updates, and optional SKOverlay surfaces.
@MainActor
final class SKAdNetworkCoordinator {
    static let shared = SKAdNetworkCoordinator()

    private init() {}

    private let workQueue = DispatchQueue(label: "com.apex.skan", qos: .utility)

    /// Call when an ad is shown so we can forward SKAN metadata downstream.
    func recordImpression(for ad: Ad, presentingScene: ApexOverlayScene? = nil) {
        guard #available(iOS 14.0, *) else { return }
        let payload = SKAdMetadata(metadata: ad.metadata)
        guard payload.hasAttributionPayload else { return }
        workQueue.async { [weak self] in
            guard let self else { return }
            Self.registerIfNeeded()
            self.applyConversionUpdates(payload: payload)
            if let overlay = payload.overlayConfig {
                Task { @MainActor in
                    SKOverlayPresenter.shared.presentOverlay(
                        appIdentifier: overlay.appStoreItemId,
                        scene: presentingScene,
                        position: overlay.position,
                        dismissible: overlay.dismissible
                    )
                }
            }
        }
    }

    @available(iOS 14.0, *)
    private static func registerIfNeeded() {
        SKAdNetwork.registerAppForAdNetworkAttribution()
    }

    @available(iOS 14.0, *)
    private func applyConversionUpdates(payload: SKAdMetadata) {
        guard let fineValue = payload.conversionValue ?? payload.derivedFineValue else { return }
        if #available(iOS 16.1, *) {
            let coarseValue = payload.coarseValue?.storeKitValue ?? .low
            SKAdNetwork.updatePostbackConversionValue(fineValue, coarseValue: coarseValue, lockWindow: payload.lockWindow) { error in
                if let error {
                    debugPrint("[SKAdNetwork] Failed to update postback conversion value: \(error.localizedDescription)")
                }
            }
        } else {
            SKAdNetwork.updateConversionValue(fineValue)
        }
    }
}

private struct SKAdMetadata {
    struct OverlayConfig {
        let appStoreItemId: Int
        let position: OverlayPosition
        let dismissible: Bool
    }

    enum CoarseValue: String {
        case low, medium, high

        @available(iOS 16.1, *)
        var storeKitValue: SKAdNetwork.CoarseConversionValue {
            switch self {
            case .low:
                return .low
            case .medium:
                return .medium
            case .high:
                return .high
            }
        }
    }

    let conversionValue: Int?
    let coarseValue: CoarseValue?
    let lockWindow: Bool
    let overlayConfig: OverlayConfig?

    var hasAttributionPayload: Bool {
        conversionValue != nil || coarseValue != nil || overlayConfig != nil
    }

    /// If only coarse value is provided, synthesize a fine value within allowed range.
    var derivedFineValue: Int? {
        if let conversionValue {
            return conversionValue
        }
        guard let coarseValue else { return nil }
        switch coarseValue {
        case .low:
            return 0
        case .medium:
            return 32
        case .high:
            return 63
        }
    }

    init(metadata: [String: String]) {
        conversionValue = metadata["skan_conversion_value"].flatMap { Int($0) }
        coarseValue = metadata["skan_coarse_value"].flatMap { CoarseValue(rawValue: $0.lowercased()) }
        lockWindow = metadata["skan_lock_window"].flatMap { SKAdMetadata.parseBool($0) } ?? false
        if let appId = metadata["skoverlay_app_id"].flatMap({ Int($0) }) {
            let position = OverlayPosition(metadataValue: metadata["skoverlay_position"])
            let dismissible = metadata["skoverlay_dismissible"].flatMap { SKAdMetadata.parseBool($0) } ?? true
            overlayConfig = OverlayConfig(appStoreItemId: appId, position: position, dismissible: dismissible)
        } else {
            overlayConfig = nil
        }
    }

    private static func parseBool(_ value: String) -> Bool? {
        switch value.lowercased() {
        case "1", "true", "yes", "y":
            return true
        case "0", "false", "no", "n":
            return false
        default:
            return nil
        }
    }
}
#else
/// macOS/tvOS build stub so SwiftPM tests can compile without StoreKit SKAdNetwork symbols.
@MainActor
final class SKAdNetworkCoordinator {
    static let shared = SKAdNetworkCoordinator()
    private init() {}

    func recordImpression(for ad: Ad, presentingScene: ApexOverlayScene? = nil) {
        // No-op on platforms without SKAdNetwork support.
    }
}
#endif
