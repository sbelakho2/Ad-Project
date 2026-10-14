#if canImport(UIKit)
import Foundation
import UIKit
#if canImport(AppTrackingTransparency)
import AppTrackingTransparency
#endif

/// Enriched in-app debug panel for iOS mirroring Android's DebugPanel.
/// Safe to ship; shows comprehensive SDK state with redacted consent info.
/// Section 3.1 enhancement: SDK version, test mode, config version, consent snapshot, adapter count.
@MainActor
public enum DebugPanel {
    /// Redacted consent snapshot for debugging - PII is masked
    public struct ConsentSnapshot: Codable {
        let gdprApplies: Bool?
        let ccpaApplicable: Bool?
        let attStatus: String
        
        /// Redacted for privacy - only shows flags, not actual consent strings
        var redactedDescription: String {
            let gdpr = gdprApplies.map { $0 ? "YES" : "NO" } ?? "UNKNOWN"
            let ccpa = ccpaApplicable.map { $0 ? "YES" : "NO" } ?? "UNKNOWN"
            return "GDPR=\(gdpr), CCPA=\(ccpa), ATT=\(attStatus)"
        }
    }
    
    /// Returns true when the app is allowed to show the debug panel.
    private static func allowlisted() -> Bool {
        #if DEBUG
        return true
        #else
        let bundleId = Bundle.main.bundleIdentifier ?? ""
        if let list = Bundle.main.object(forInfoDictionaryKey: "ApexMediationDebugAllowlist") as? [String] {
            return list.contains(bundleId)
        }
        return false
        #endif
    }

    public static func show(from viewController: UIViewController) {
        guard allowlisted() else { return }
        let sdk = MediationSDK.shared
        
        // Basic info
        let appId = sdk.currentAppId() ?? "N/A"
        let placements = sdk.currentPlacementIds().joined(separator: ", ")
        let placementsDisplay = placements.isEmpty ? "N/A" : placements
        
        // SDK version (Section 3.1 requirement)
        let sdkVersion = sdk.sdkVersion
        
        // Test mode indicator (Section 3.1 requirement)
        let testMode = sdk.isTestMode ? "ON ⚠️" : "OFF"
        
        // Config version (Section 3.1 requirement)
        let configVersion = sdk.remoteConfigVersion.map(String.init) ?? "N/A"
        
        // Redacted consent snapshot (Section 3.1 requirement)
        let consentInfo = buildConsentSnapshot()
        
        // Adapter count (Section 3.1 requirement)
        let adapterCount = sdk.registeredAdapterCount
        
        // Build enriched message
        let message = """
        Apex Mediation — Debug Panel
        
        SDK Version: \(sdkVersion)
        App ID: \(appId)
        Test Mode: \(testMode)
        Config Version: \(configVersion)
        
        Placements: \(placementsDisplay)
        Adapters: \(adapterCount) registered
        
        Consent: \(consentInfo.redactedDescription)
        
        Note: Consent values are redacted for privacy.
        """
        
        let alert = UIAlertController(title: "Mediation Debugger", message: message, preferredStyle: .alert)
        #if !os(tvOS)
        alert.addAction(UIAlertAction(title: "Copy", style: .default, handler: { _ in
            UIPasteboard.general.string = message
        }))
        #endif
        alert.addAction(UIAlertAction(title: "Close", style: .cancel, handler: nil))
        viewController.present(alert, animated: true)
    }
    
    /// Build consent snapshot with ATT status and privacy flags
    private static func buildConsentSnapshot() -> ConsentSnapshot {
        var attStatus = "UNKNOWN"
        
        #if os(tvOS)
        attStatus = "UNAVAILABLE_TVOS"
        #elseif canImport(AppTrackingTransparency)
        if #available(iOS 14, *) {
            switch ATTrackingManager.trackingAuthorizationStatus {
            case .authorized:
                attStatus = "AUTHORIZED"
            case .denied:
                attStatus = "DENIED"
            case .restricted:
                attStatus = "RESTRICTED"
            case .notDetermined:
                attStatus = "NOT_DETERMINED"
            @unknown default:
                attStatus = "UNKNOWN"
            }
        }
        #endif
        
        let consent = ConsentManager.shared.getConsent()
        return ConsentSnapshot(
            gdprApplies: consent.gdprApplies,
            ccpaApplicable: consent.usPrivacyString != nil || consent.ccpaOptOut,
            attStatus: attStatus
        )
    }
}

#endif
