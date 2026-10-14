import Foundation

// MARK: - Adapter Health Status API

/// Tracks initialization status and health metrics for each ad network adapter.
/// Industry standard alignment with AppLovin MAX's InitializationStatus enum.
public final class AdapterHealthManager {
    
    public static let shared = AdapterHealthManager()
    
    private var healthStatus: [String: AdapterHealth] = [:]
    private let statusLock = NSLock()
    private var statusListeners: [AdapterStatusListener] = []
    
    private init() {}
    
    // MARK: - Types
    
    /// Initialization status enum matching AppLovin MAX pattern
    public enum InitializationStatus: String {
        /// Adapter has not been initialized yet
        case notInitialized = "NOT_INITIALIZED"
        
        /// Adapter is currently initializing
        case initializing = "INITIALIZING"
        
        /// Adapter initialized successfully
        case initializedSuccess = "INITIALIZED_SUCCESS"
        
        /// Adapter failed to initialize
        case initializedFailure = "INITIALIZED_FAILURE"
        
        /// Adapter initialization status unknown
        case initializedUnknown = "INITIALIZED_UNKNOWN"
        
        /// Adapter does not require initialization
        case doesNotApply = "DOES_NOT_APPLY"
    }
    
    /// Health status for an individual adapter
    public struct AdapterHealth {
        public let networkName: String
        public let status: InitializationStatus
        public let sdkVersion: String?
        public let adapterVersion: String?
        public let lastStatusChange: Date
        public let errorMessage: String?
        public let errorCode: String?
        public let capabilities: AdapterCapabilities
        public let metrics: AdapterMetrics
        
        public func isReady() -> Bool {
            return status == .initializedSuccess || status == .doesNotApply
        }
        
        public func toDict() -> [String: Any?] {
            return [
                "networkName": networkName,
                "status": status.rawValue,
                "sdkVersion": sdkVersion,
                "adapterVersion": adapterVersion,
                "lastStatusChange": Int(lastStatusChange.timeIntervalSince1970 * 1000),
                "errorMessage": errorMessage,
                "errorCode": errorCode,
                "isReady": isReady(),
                "capabilities": capabilities.toDict(),
                "metrics": metrics.toDict()
            ]
        }
    }
    
    /// Capabilities supported by an adapter
    public struct AdapterCapabilities {
        public let supportsBanner: Bool
        public let supportsInterstitial: Bool
        public let supportsRewarded: Bool
        public let supportsRewardedInterstitial: Bool
        public let supportsNative: Bool
        public let supportsAppOpen: Bool
        public let supportsSignalCollection: Bool
        public let supportsS2SBidding: Bool
        
        public init(
            supportsBanner: Bool = false,
            supportsInterstitial: Bool = false,
            supportsRewarded: Bool = false,
            supportsRewardedInterstitial: Bool = false,
            supportsNative: Bool = false,
            supportsAppOpen: Bool = false,
            supportsSignalCollection: Bool = false,
            supportsS2SBidding: Bool = false
        ) {
            self.supportsBanner = supportsBanner
            self.supportsInterstitial = supportsInterstitial
            self.supportsRewarded = supportsRewarded
            self.supportsRewardedInterstitial = supportsRewardedInterstitial
            self.supportsNative = supportsNative
            self.supportsAppOpen = supportsAppOpen
            self.supportsSignalCollection = supportsSignalCollection
            self.supportsS2SBidding = supportsS2SBidding
        }
        
        public static let empty = AdapterCapabilities()
        
        public func toDict() -> [String: Bool] {
            return [
                "banner": supportsBanner,
                "interstitial": supportsInterstitial,
                "rewarded": supportsRewarded,
                "rewardedInterstitial": supportsRewardedInterstitial,
                "native": supportsNative,
                "appOpen": supportsAppOpen,
                "signalCollection": supportsSignalCollection,
                "s2sBidding": supportsS2SBidding
            ]
        }
    }
    
    /// Runtime metrics for an adapter
    public struct AdapterMetrics {
        public let totalRequests: Int
        public let successfulFills: Int
        public let noFills: Int
        public let errors: Int
        public let timeouts: Int
        public let avgLoadTimeMs: Double
        public let lastRequestTime: Date?
        
        public var fillRate: Double {
            guard totalRequests > 0 else { return 0 }
            return (Double(successfulFills) / Double(totalRequests)) * 100
        }
        
        public init(
            totalRequests: Int = 0,
            successfulFills: Int = 0,
            noFills: Int = 0,
            errors: Int = 0,
            timeouts: Int = 0,
            avgLoadTimeMs: Double = 0,
            lastRequestTime: Date? = nil
        ) {
            self.totalRequests = totalRequests
            self.successfulFills = successfulFills
            self.noFills = noFills
            self.errors = errors
            self.timeouts = timeouts
            self.avgLoadTimeMs = avgLoadTimeMs
            self.lastRequestTime = lastRequestTime
        }
        
        public static let empty = AdapterMetrics()
        
        public func toDict() -> [String: Any?] {
            return [
                "totalRequests": totalRequests,
                "successfulFills": successfulFills,
                "noFills": noFills,
                "errors": errors,
                "timeouts": timeouts,
                "avgLoadTimeMs": avgLoadTimeMs,
                "lastRequestTime": lastRequestTime.map { Int($0.timeIntervalSince1970 * 1000) },
                "fillRate": fillRate
            ]
        }
        
        public func withRequest(success: Bool, loadTimeMs: Double) -> AdapterMetrics {
            let newTotal = totalRequests + 1
            let newFills = success ? successfulFills + 1 : successfulFills
            let newNoFills = success ? noFills : noFills + 1
            let newAvgTime = ((avgLoadTimeMs * Double(totalRequests)) + loadTimeMs) / Double(newTotal)
            return AdapterMetrics(
                totalRequests: newTotal,
                successfulFills: newFills,
                noFills: newNoFills,
                errors: errors,
                timeouts: timeouts,
                avgLoadTimeMs: newAvgTime,
                lastRequestTime: Date()
            )
        }
        
        public func withError() -> AdapterMetrics {
            return AdapterMetrics(
                totalRequests: totalRequests,
                successfulFills: successfulFills,
                noFills: noFills,
                errors: errors + 1,
                timeouts: timeouts,
                avgLoadTimeMs: avgLoadTimeMs,
                lastRequestTime: Date()
            )
        }
        
        public func withTimeout() -> AdapterMetrics {
            return AdapterMetrics(
                totalRequests: totalRequests,
                successfulFills: successfulFills,
                noFills: noFills,
                errors: errors,
                timeouts: timeouts + 1,
                avgLoadTimeMs: avgLoadTimeMs,
                lastRequestTime: Date()
            )
        }
    }
    
    /// Listener protocol for status changes
    public protocol AdapterStatusListener: AnyObject {
        func onAdapterStatusChanged(networkName: String, health: AdapterHealth)
    }
    
    // MARK: - Public API
    
    /// Get current health status for an adapter
    public func getAdapterHealth(networkName: String) -> AdapterHealth? {
        statusLock.lock()
        defer { statusLock.unlock() }
        return healthStatus[networkName.lowercased()]
    }
    
    /// Get health status for all adapters
    public func getAllAdapterHealth() -> [String: AdapterHealth] {
        statusLock.lock()
        defer { statusLock.unlock() }
        return healthStatus
    }
    
    /// Get list of adapters that are ready
    public func getReadyAdapters() -> [String] {
        statusLock.lock()
        defer { statusLock.unlock() }
        return healthStatus.filter { $0.value.isReady() }.map { $0.key }
    }
    
    /// Get list of adapters that failed initialization
    public func getFailedAdapters() -> [AdapterHealth] {
        statusLock.lock()
        defer { statusLock.unlock() }
        return healthStatus.values.filter { $0.status == .initializedFailure }
    }
    
    /// Register a listener for status changes
    public func addStatusListener(_ listener: AdapterStatusListener) {
        statusLock.lock()
        statusListeners.append(listener)
        statusLock.unlock()
    }
    
    /// Remove a status listener
    public func removeStatusListener(_ listener: AdapterStatusListener) {
        statusLock.lock()
        statusListeners.removeAll { $0 === listener }
        statusLock.unlock()
    }
    
    // MARK: - Internal API (for SDK use)
    
    internal func registerAdapter(
        networkName: String,
        adapterVersion: String?,
        capabilities: AdapterCapabilities = .empty
    ) {
        let key = networkName.lowercased()
        statusLock.lock()
        if healthStatus[key] == nil {
            healthStatus[key] = AdapterHealth(
                networkName: networkName,
                status: .notInitialized,
                sdkVersion: nil,
                adapterVersion: adapterVersion,
                lastStatusChange: Date(),
                errorMessage: nil,
                errorCode: nil,
                capabilities: capabilities,
                metrics: .empty
            )
        }
        statusLock.unlock()
    }
    
    internal func updateStatus(
        networkName: String,
        status: InitializationStatus,
        sdkVersion: String? = nil,
        errorMessage: String? = nil,
        errorCode: String? = nil
    ) {
        let key = networkName.lowercased()
        statusLock.lock()
        
        let current = healthStatus[key] ?? AdapterHealth(
            networkName: networkName,
            status: status,
            sdkVersion: sdkVersion,
            adapterVersion: nil,
            lastStatusChange: Date(),
            errorMessage: errorMessage,
            errorCode: errorCode,
            capabilities: .empty,
            metrics: .empty
        )
        
        let updated = AdapterHealth(
            networkName: networkName,
            status: status,
            sdkVersion: sdkVersion ?? current.sdkVersion,
            adapterVersion: current.adapterVersion,
            lastStatusChange: Date(),
            errorMessage: errorMessage,
            errorCode: errorCode,
            capabilities: current.capabilities,
            metrics: current.metrics
        )
        
        healthStatus[key] = updated
        let listeners = statusListeners
        statusLock.unlock()
        
        // Notify listeners
        for listener in listeners {
            listener.onAdapterStatusChanged(networkName: networkName, health: updated)
        }
    }
    
    internal func recordRequest(networkName: String, success: Bool, loadTimeMs: Double) {
        let key = networkName.lowercased()
        statusLock.lock()
        guard var health = healthStatus[key] else {
            statusLock.unlock()
            return
        }
        let updatedMetrics = health.metrics.withRequest(success: success, loadTimeMs: loadTimeMs)
        healthStatus[key] = AdapterHealth(
            networkName: health.networkName,
            status: health.status,
            sdkVersion: health.sdkVersion,
            adapterVersion: health.adapterVersion,
            lastStatusChange: health.lastStatusChange,
            errorMessage: health.errorMessage,
            errorCode: health.errorCode,
            capabilities: health.capabilities,
            metrics: updatedMetrics
        )
        statusLock.unlock()
    }
    
    internal func recordError(networkName: String) {
        let key = networkName.lowercased()
        statusLock.lock()
        guard let health = healthStatus[key] else {
            statusLock.unlock()
            return
        }
        healthStatus[key] = AdapterHealth(
            networkName: health.networkName,
            status: health.status,
            sdkVersion: health.sdkVersion,
            adapterVersion: health.adapterVersion,
            lastStatusChange: health.lastStatusChange,
            errorMessage: health.errorMessage,
            errorCode: health.errorCode,
            capabilities: health.capabilities,
            metrics: health.metrics.withError()
        )
        statusLock.unlock()
    }
    
    internal func recordTimeout(networkName: String) {
        let key = networkName.lowercased()
        statusLock.lock()
        guard let health = healthStatus[key] else {
            statusLock.unlock()
            return
        }
        healthStatus[key] = AdapterHealth(
            networkName: health.networkName,
            status: health.status,
            sdkVersion: health.sdkVersion,
            adapterVersion: health.adapterVersion,
            lastStatusChange: health.lastStatusChange,
            errorMessage: health.errorMessage,
            errorCode: health.errorCode,
            capabilities: health.capabilities,
            metrics: health.metrics.withTimeout()
        )
        statusLock.unlock()
    }
    
    /// Reset all health data (for testing)
    internal func reset() {
        statusLock.lock()
        healthStatus.removeAll()
        statusLock.unlock()
    }
}
