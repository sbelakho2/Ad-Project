import Foundation

/// Primary SDK entry point matching Android's BelAds API.
/// Provides initialization, configuration, and global SDK settings.
@MainActor
public enum BelAds {
    
    /// SDK version string
    public static var version: String {
        return "1.0.0-ios"
    }
    
    /// Check if SDK is initialized
    public static var isInitialized: Bool {
        return MediationSDK.shared.isInitialized
    }

    /// Current ATT authorization state for UI surfaces/debug screens.
    public static func trackingAuthorizationStatus() -> ATTStatus {
        TrackingAuthorizationManager.shared.currentStatus()
    }

    /// Awaitable ATT authorization request helper for host apps.
    public static func requestTrackingAuthorization() async -> ATTStatus {
        await TrackingAuthorizationManager.shared.requestAuthorizationIfNeeded()
    }

    /// Completion-based ATT authorization helper to match legacy codebases.
    public static func requestTrackingAuthorization(completion: @escaping (ATTStatus) -> Void) {
        Task { @MainActor in
            let status = await TrackingAuthorizationManager.shared.requestAuthorizationIfNeeded()
            completion(status)
        }
    }
    
    /// Initialize the Apex Mediation SDK
    /// - Parameters:
    ///   - appId: Application identifier from Apex console
    ///   - testMode: Enable test mode (bypasses signature verification)
    ///   - completion: Async completion handler
    public static func initialize(
        appId: String,
        testMode: Bool = false,
        completion: @escaping (Result<Void, Error>) -> Void
    ) {
        Task {
            do {
                let config = SDKConfig.default(appId: appId, testMode: testMode)
                try await MediationSDK.shared.initialize(appId: appId, configuration: config)
                completion(.success(()))
            } catch {
                completion(.failure(error))
            }
        }
    }
    
    /// Initialize with custom configuration
    /// - Parameters:
    ///   - appId: Application identifier from Apex console
    ///   - configuration: Custom SDK configuration
    ///   - completion: Async completion handler
    public static func initialize(
        appId: String,
        configuration: SDKConfig,
        completion: @escaping (Result<Void, Error>) -> Void
    ) {
        Task {
            do {
                try await MediationSDK.shared.initialize(appId: appId, configuration: configuration)
                completion(.success(()))
            } catch {
                completion(.failure(error))
            }
        }
    }
    
    /// Set user consent for privacy regulations
    /// - Parameter consent: Consent data with GDPR/CCPA/COPPA flags
    public static func setConsent(_ consent: ConsentData) {
        MediationSDK.shared.setConsent(consent)
    }
    
    /// Get current consent settings
    /// - Returns: Current consent data
    public static func getConsent() -> ConsentData {
        return MediationSDK.shared.currentConsent()
    }
    
    /// Enable or disable test mode
    /// - Parameter enabled: Whether test mode should be enabled
    public static func setTestMode(_ enabled: Bool) {
        // Test mode is set during initialization, but we can log the intent
        if enabled {
            print("[BelAds] Test mode requested. Apply during initialize() call.")
        }
    }
    
    /// Enable or disable debug logging
    /// - Parameter enabled: Whether debug logging should be enabled
    public static func setDebugLogging(_ enabled: Bool) {
        Logger.setLevel(enabled ? .debug : .info)
        UserDefaults.standard.set(enabled, forKey: "apex_debug_logging")
    }
    
    /// Get debug panel information
    /// - Returns: Dictionary with SDK diagnostics
    public static func getDebugInfo() -> [String: Any] {
        let sdk = MediationSDK.shared
        return [
            "sdkVersion": version,
            "isInitialized": isInitialized,
            "testMode": sdk.isTestMode,
            "appId": sdk.currentAppId() ?? "none",
            "configVersion": sdk.remoteConfigVersion ?? 0,
            "adapterCount": sdk.registeredAdapterCount,
            "consent": sdk.consentSummary
        ]
    }
}
