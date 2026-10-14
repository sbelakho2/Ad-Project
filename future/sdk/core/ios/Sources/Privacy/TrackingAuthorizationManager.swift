import Foundation

#if os(iOS) || os(tvOS)
#if canImport(AppTrackingTransparency)
import AppTrackingTransparency
#endif
#if canImport(AdSupport)
import AdSupport
#endif

/// Centralized helper for ATT authorization status and IDFA availability.
/// Keeps access serialized because adapters may query from background queues.
final class TrackingAuthorizationManager: @unchecked Sendable {
    static let shared = TrackingAuthorizationManager()

    private let lock = NSLock()
    private var cachedStatus: ATTStatus

    private init() {
        cachedStatus = Self.readSystemStatus()
    }

    /// Latest ATT status (or best-effort legacy mapping on iOS < 14).
    func currentStatus() -> ATTStatus {
        lock.lock()
        let status = Self.readSystemStatus()
        cachedStatus = status
        lock.unlock()
        return status
    }

    /// Whether IDFA access is allowed under current ATT settings.
    func canAccessAdvertisingIdentifier() -> Bool {
        #if canImport(AdSupport)
        if #available(iOS 14, tvOS 14, *) {
            return currentStatus() == .authorized
        } else {
            return ASIdentifierManager.shared().isAdvertisingTrackingEnabled
        }
        #else
        return false
        #endif
    }

    /// Whether LAT should be considered on for downstream partners.
    func isLimitAdTrackingEnabled() -> Bool {
        #if canImport(AdSupport)
        if #available(iOS 14, tvOS 14, *) {
            return currentStatus() != .authorized
        } else {
            return !ASIdentifierManager.shared().isAdvertisingTrackingEnabled
        }
        #else
        return true
        #endif
    }

    /// Returns the advertising identifier if ATT permits access.
    func advertisingIdentifier() -> String? {
        guard canAccessAdvertisingIdentifier() else { return nil }
        #if canImport(AdSupport)
        let identifier = ASIdentifierManager.shared().advertisingIdentifier
        let uuidString = identifier.uuidString
        if uuidString == "00000000-0000-0000-0000-000000000000" {
            return nil
        }
        return uuidString
        #else
        return nil
        #endif
    }

    /// Prompts the user for ATT authorization when applicable.
    @MainActor
    func requestAuthorizationIfNeeded() async -> ATTStatus {
        #if canImport(AppTrackingTransparency)
        if #available(iOS 14, tvOS 14, *) {
            let status = ATTrackingManager.trackingAuthorizationStatus
            if status == .notDetermined {
                let resolved = await withCheckedContinuation { continuation in
                    ATTrackingManager.requestTrackingAuthorization { newStatus in
                        continuation.resume(returning: newStatus)
                    }
                }
                let mapped = Self.map(status: resolved)
                updateCache(mapped)
                return mapped
            } else {
                let mapped = Self.map(status: status)
                updateCache(mapped)
                return mapped
            }
        }
        #endif
        let fallback = Self.legacyAuthorizationStatus()
        updateCache(fallback)
        return fallback
    }

    private func updateCache(_ status: ATTStatus) {
        lock.lock()
        cachedStatus = status
        lock.unlock()
    }

    private static func readSystemStatus() -> ATTStatus {
        #if canImport(AppTrackingTransparency)
        if #available(iOS 14, tvOS 14, *) {
            return map(status: ATTrackingManager.trackingAuthorizationStatus)
        }
        #endif
        return legacyAuthorizationStatus()
    }

    private static func legacyAuthorizationStatus() -> ATTStatus {
        #if canImport(AdSupport)
        return ASIdentifierManager.shared().isAdvertisingTrackingEnabled ? .authorized : .denied
        #else
        return .restricted
        #endif
    }

    #if canImport(AppTrackingTransparency)
    @available(iOS 14, tvOS 14, *)
    private static func map(status: ATTrackingManager.AuthorizationStatus) -> ATTStatus {
        switch status {
        case .authorized:
            return .authorized
        case .denied:
            return .denied
        case .restricted:
            return .restricted
        case .notDetermined:
            return .notDetermined
        @unknown default:
            return .restricted
        }
    }
    #else
    private static func map(status _: Int) -> ATTStatus { .restricted }
    #endif
}
#else
/// Minimal stub so macOS SwiftPM builds do not depend on ATT/AdSupport frameworks.
final class TrackingAuthorizationManager: @unchecked Sendable {
    static let shared = TrackingAuthorizationManager()

    private init() {}

    func currentStatus() -> ATTStatus { .restricted }
    func canAccessAdvertisingIdentifier() -> Bool { false }
    func isLimitAdTrackingEnabled() -> Bool { true }
    func advertisingIdentifier() -> String? { nil }

    @MainActor
    func requestAuthorizationIfNeeded() async -> ATTStatus { .restricted }
}
#endif
