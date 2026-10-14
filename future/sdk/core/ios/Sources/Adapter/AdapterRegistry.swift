import Foundation

// MARK: - Adapter Protocol

/// Base adapter protocol
///
/// All ad network adapters must implement this protocol
public protocol AdNetworkAdapter {
    init()
    /// Adapter metadata
    var networkName: String { get }
    var version: String { get }
    var minSDKVersion: String { get }
    
    /// Initialize adapter with app-level configuration
    func initialize(config: [String: Any]) throws
    
    /// Load an ad
    func loadAd(
        placement: String,
        adType: AdType,
        config: [String: Any],
        completion: @escaping @Sendable (Result<Ad, AdapterRegistryError>) -> Void
    )
    
    /// Check if adapter supports specific ad type
    func supportsAdType(_ adType: AdType) -> Bool
    
    /// Clean up adapter resources
    func destroy()
}

// MARK: - Adapter Registry

/// Adapter registry with dynamic loading
///
/// Features:
/// - Dynamic framework loading
/// - Version compatibility checking
/// - Lazy initialization
/// - Thread-safe adapter management
public final class AdapterRegistry {
    private var adapters: [String: AdapterWrapper] = [:]
    private var adapterClasses: [String: AdNetworkAdapter.Type] = [:]
    private let lock = NSLock()
    
    private let sdkVersion: String
    
    /// Adapter wrapper with metadata
    private struct AdapterWrapper {
        let adapter: AdNetworkAdapter
        var isInitialized: Bool
        var initError: String?
    }
    
    /// Initialize adapter registry
    public init(sdkVersion: String = "1.0.0") {
        self.sdkVersion = sdkVersion
        registerBuiltInAdapters()
    }
    
    // MARK: - Adapter Discovery
    
    /// Register built-in adapters that ship with the SDK target
    private func registerBuiltInAdapters() {
        #if DEBUG || APEX_SANDBOX_ADAPTERS
        let builtIns: [String: AdNetworkAdapter.Type] = [
            "admob": AdMobAdapter.self,
            "applovin": AppLovinAdapter.self,
            "unity": UnityAdsAdapter.self,
            "ironsource": IronSourceAdapter.self,
            "facebook": FacebookAdapter.self,
            "vungle": VungleAdapter.self,
            "chartboost": ChartboostAdapter.self,
            "pangle": PangleAdapter.self,
            "mintegral": MintegralAdapter.self,
            "adcolony": AdColonyAdapter.self,
            "tapjoy": TapjoyAdapter.self,
            "moloco": MolocoAdapter.self,
            "fyber": FyberAdapter.self,
            "smaato": SmaatoAdapter.self,
            "amazon": AmazonAdapter.self
        ]
        adapterClasses.merge(builtIns) { _, new in new }
        #else
        // In production builds, we do not ship any built-in third-party adapters.
        // Publishers can register their own adapters via MediationSDK.registerAdapter(...)
        #endif
    }
    
    /// Register custom adapter
    public func registerAdapter(networkName: String, adapterClass: AdNetworkAdapter.Type) {
        lock.lock()
        adapterClasses[networkName] = adapterClass
        lock.unlock()
    }

    /// Returns all registered adapter network names (sorted)
    public func allNetworkNames() -> [String] {
        lock.lock()
        let names = Array(adapterClasses.keys)
        lock.unlock()
        return names.sorted()
    }
    
    /// Section 3.1: Get count of registered adapters for debug panel
    public var registeredCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return adapterClasses.count
    }
    
    /// Initialization diagnostics
    public struct InitializationStatus: Codable, Equatable {
        public let networkName: String
        public let registered: Bool
        public let initialized: Bool
        public let version: String?
        public let minSDKVersion: String?
        public let error: String?
    }
    
    /// Returns a list of initialization statuses for all known adapters
    public func getInitializationReport() -> [InitializationStatus] {
        lock.lock()
        defer { lock.unlock() }
        let known = Set(adapterClasses.keys)
        let union = Array(known.union(Set(adapters.keys))).sorted()
        return union.map { name in
            let wrapper = adapters[name]
            let cls = adapterClasses[name]
            let adapter: AdNetworkAdapter? = {
                if let w = wrapper { return w.adapter }
                if let c = cls { return c.init() }
                return nil
            }()
            return InitializationStatus(
                networkName: name,
                registered: cls != nil,
                initialized: wrapper?.isInitialized == true,
                version: adapter?.version,
                minSDKVersion: adapter?.minSDKVersion,
                error: wrapper?.initError
            )
        }
    }
    
    // MARK: - Adapter Management
    
    /// Get adapter instance (lazy initialization)
    public func getAdapter(networkName: String) -> AdNetworkAdapter? {
        lock.lock()
        defer { lock.unlock() }
        
        // Check if already initialized
        if let wrapper = adapters[networkName], wrapper.isInitialized {
            return wrapper.adapter
        }
        
        // Load adapter class
        guard let adapterClass = adapterClasses[networkName] else {
            return nil
        }
        
        // Create adapter instance
        let adapter = adapterClass.init()
        
        // Check version compatibility
        if !isCompatible(minVersion: adapter.minSDKVersion) {
            adapters[networkName] = AdapterWrapper(
                adapter: adapter,
                isInitialized: false,
                initError: "Incompatible SDK version"
            )
            return nil
        }
        
        adapters[networkName] = AdapterWrapper(
            adapter: adapter,
            isInitialized: false,
            initError: nil
        )
        
        return adapter
    }
    
    /// Initialize adapter
    public func initializeAdapter(networkName: String, config: [String: Any]) {
        lock.lock()
        defer { lock.unlock() }
        
        guard var wrapper = adapters[networkName] else { return }
        
        if wrapper.isInitialized {
            return
        }
        
        do {
            try wrapper.adapter.initialize(config: config)
            wrapper.isInitialized = true
            wrapper.initError = nil
            adapters[networkName] = wrapper
        } catch {
            wrapper.isInitialized = false
            wrapper.initError = error.localizedDescription
            adapters[networkName] = wrapper
        }
    }
    
    /// Check if adapter is initialized
    public func isInitialized(networkName: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return adapters[networkName]?.isInitialized ?? false
    }
    
    /// Get all available adapters
    public func getAvailableAdapters() -> [String] {
        lock.lock()
        defer { lock.unlock() }
        return Array(adapterClasses.keys)
    }
    
    /// Get all initialized adapters
    public func getInitializedAdapters() -> [String] {
        lock.lock()
        defer { lock.unlock() }
        return adapters.filter { $0.value.isInitialized }.map { $0.key }
    }
    
    /// Get adapter initialization error
    public func getAdapterError(networkName: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        return adapters[networkName]?.initError
    }
    
    // MARK: - Version Compatibility
    
    /// Check version compatibility
    private func isCompatible(minVersion: String) -> Bool {
        return compareVersions(sdkVersion, minVersion) >= 0
    }
    
    /// Compare semantic versions
    private func compareVersions(_ version1: String, _ version2: String) -> Int {
        let parts1 = version1.split(separator: ".").compactMap { Int($0) }
        let parts2 = version2.split(separator: ".").compactMap { Int($0) }
        
        let maxLength = max(parts1.count, parts2.count)
        
        for i in 0..<maxLength {
            let part1 = i < parts1.count ? parts1[i] : 0
            let part2 = i < parts2.count ? parts2[i] : 0
            
            if part1 != part2 {
                return part1 < part2 ? -1 : 1
            }
        }
        
        return 0
    }
    
    /// Clean up all adapters
    public func destroy() {
        lock.lock()
        defer { lock.unlock() }
        
        for (_, wrapper) in adapters {
            wrapper.adapter.destroy()
        }
        
        adapters.removeAll()
        adapterClasses.removeAll()
    }
}

// MARK: - Adapter Error

/// Adapter errors
public enum AdapterRegistryError: Error, LocalizedError {
    case notInitialized
    case loadFailed(String)
    case unsupportedAdType
    case timeout
    case networkError
    
    public var errorDescription: String? {
        switch self {
        case .notInitialized:
            return "Adapter not initialized"
        case .loadFailed(let message):
            return "Ad load failed: \(message)"
        case .unsupportedAdType:
            return "Unsupported ad type"
        case .timeout:
            return "Ad load timeout"
        case .networkError:
            return "Network error"
        }
    }
}

// Example adapters have been moved under StubAdapters.swift and are compiled only in DEBUG or when
// APEX_SANDBOX_ADAPTERS is defined. The core AdapterRegistry does not include vendor adapters in Release builds.
// See StubAdapters.swift for DEBUG-only example implementations.
