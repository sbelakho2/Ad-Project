import Foundation
import Dispatch

private enum CacheConfig {
    /// Cushion to absorb network jitter and scheduling delays before expiring cached ads.
    static let expirationGrace: TimeInterval = 0.35
}

private struct AdapterTypeToken: @unchecked Sendable {
    let type: AdNetworkAdapter.Type
}

public final class MediationSDK: @unchecked Sendable {
    public static let shared = MediationSDK()

    private let sdkVersionValue = "1.0.0"
    private var clock: ClockProtocol
    private var runtime: MediationRuntime
    private let snapshotLock = NSLock()
    private var snapshot: SDKSnapshot = .empty

    private init(clock: ClockProtocol = Clock.shared) {
        self.clock = clock
        runtime = MediationRuntime(sdkVersion: sdkVersionValue, clock: clock)
    }
    
    // MARK: - Debug/Diagnostics accessors (read-only)
    public var isInitialized: Bool {
        readSnapshot { $0.isInitialized }
    }

    public func currentAppId() -> String? {
        readSnapshot { $0.appId }
    }

    public func currentPlacementIds() -> [String] {
        readSnapshot { $0.placementIds }
    }
    
    public var sdkVersion: String {
        sdkVersionValue
    }
    
    public var isTestMode: Bool {
        readSnapshot { $0.testMode }
    }
    
    public var remoteConfigVersion: Int? {
        readSnapshot { $0.remoteConfigVersion }
    }

    public var consentSummary: [String: Any] {
        readSnapshot { $0.consentSummary }
    }
    
    public var registeredAdapterCount: Int {
        readSnapshot { $0.registeredAdapters }
    }

    // MARK: - Sandbox/Diagnostics helpers
    /// Returns the list of registered adapter network names (sorted). Requires initialization; if not yet
    /// initialized, returns the SDK's built-in registry names.
    public func adapterNames() async -> [String] {
        await runtime.adapterNames()
    }

    /// In sandbox/test mode, restrict loading to a specific set of adapters.
    /// Pass `nil` to clear the whitelist (restore normal priority behavior).
    public func setSandboxAdapterWhitelist(_ names: [String]?) async {
        await runtime.setSandboxAdapterWhitelist(names)
    }

    /// In sandbox/test mode, force the SDK to use the adapter pipeline instead of the HTTP test-mode loader.
    /// This allows exercising adapter stubs without a live auction service.
    public func setSandboxForceAdapterPipeline(_ enabled: Bool) async {
        await runtime.setSandboxForceAdapterPipeline(enabled)
    }

    #if DEBUG
    /// Testing hook to rebuild runtime with a custom clock.
    func _resetRuntimeForTesting(clock: ClockProtocol = Clock.shared) {
        self.clock = clock
        runtime = MediationRuntime(sdkVersion: sdkVersionValue, clock: clock)
        snapshot = .empty
    }
    #endif

    /// BYO-first: Allow host apps to register their own adapter types at runtime (before initialize()).
    /// This API is available in all build configurations so publishers can bring their own adapters
    /// without the SDK shipping vendor code.
    public func registerAdapter(networkName: String, adapterType: AdNetworkAdapter.Type) async {
        let token = AdapterTypeToken(type: adapterType)
        await runtime.registerAdapter(networkName: networkName, adapterType: token)
    }

    /// Update consent state shared across the SDK.
    public func setConsent(_ consent: ConsentData) {
        ConsentManager.shared.setConsent(consent)
        updateConsentSummarySnapshot()
    }

    /// Returns the latest consent object.
    public func currentConsent() -> ConsentData {
        ConsentManager.shared.getConsent()
    }

    /// Metadata forwarded to network and S2S requests.
    public func consentMetadata() -> [String: Any] {
        ConsentManager.shared.toAdRequestMetadata()
    }

    /// Whether personalized ads are currently allowed.
    public func canShowPersonalizedAds() -> Bool {
        ConsentManager.shared.canShowPersonalizedAds()
    }

    /// Initialize the mediation SDK.
    public func initialize(appId: String, configuration: SDKConfig? = nil) async throws {
        let digest = try await runOffMain { [self] in
            try await runtime.initialize(appId: appId, configuration: configuration)
        }
        updateSnapshot(digest)
    }

    /// Load an ad for the supplied placement.
    public func loadAd(placementId: String) async throws -> Ad? {
        let ad = try await runOffMain { [self] in
            try await runtime.loadAd(placementId: placementId)
        }
        await refreshSnapshot()
        return ad
    }

    /// Check if an ad is cached and ready.
    public func isAdReady(placementId: String) -> Bool {
        readSnapshot { snapshot in
            guard let expirations = snapshot.cacheExpirationsMs[placementId], !expirations.isEmpty else {
                return false
            }
            let nowMs = clock.monotonicMillis()
            let graceMs = Int64(CacheConfig.expirationGrace * 1000)
            return expirations.contains { $0 + graceMs > nowMs }
        }
    }

    /// Claim and remove a cached ad for the placement.
    public func claimAd(placementId: String) async -> Ad? {
        let ad = await runOffMain { [self] in
            await runtime.claimAd(placementId: placementId)
        }
        await refreshSnapshot()
        return ad
    }

    /// Peek at the next cached ad without consuming it.
    public func peekAd(placementId: String) async -> Ad? {
        await runOffMain { [self] in
            await runtime.peekAd(placementId: placementId)
        }
    }

    /// Shutdown the SDK and release resources.
    public func shutdown() {
        let semaphore = DispatchSemaphore(value: 0)
        Task.detached(priority: .userInitiated) { [weak self] in
            defer { semaphore.signal() }
            guard let self else { return }
            let digest = await self.runtime.shutdown()
            self.updateSnapshot(digest)
        }
        semaphore.wait()
    }

    private func refreshSnapshot() async {
        let digest = await runtime.snapshot()
        updateSnapshot(digest)
    }

    /// Ensure heavy/IO work is executed off the main thread to avoid UI stalls.
    private func runOffMain<T: Sendable>(
        _ work: @escaping @Sendable () async -> T
    ) async -> T {
        await Task.detached(priority: .userInitiated) {
            await work()
        }.value
    }

    /// Throwing variant for call sites that propagate errors.
    private func runOffMain<T: Sendable>(
        _ work: @escaping @Sendable () async throws -> T
    ) async throws -> T {
        try await Task.detached(priority: .userInitiated) {
            try await work()
        }.value
    }
    
    private func updateConsentSummarySnapshot() {
        let summary = ConsentManager.shared.getRedactedConsentInfo()
        snapshotLock.lock()
        snapshot = snapshot.updatingConsentSummary(summary)
        snapshotLock.unlock()
    }

    private func updateSnapshot(_ newSnapshot: SDKSnapshot) {
        snapshotLock.lock()
        snapshot = newSnapshot
        snapshotLock.unlock()
    }

    private func readSnapshot<T>(_ transform: (SDKSnapshot) -> T) -> T {
        snapshotLock.lock()
        let value = transform(snapshot)
        snapshotLock.unlock()
        return value
    }
}

// MARK: - Private Runtime Types

private struct SDKSnapshot {
    let appId: String?
    let placementIds: [String]
    let remoteConfigVersion: Int?
    let registeredAdapters: Int
    let testMode: Bool
    let isInitialized: Bool
    let cacheCounts: [String: Int]
    let consentSummary: [String: Any]
    let cacheExpirationsMs: [String: [Int64]]

    static let empty = SDKSnapshot(
        appId: nil,
        placementIds: [],
        remoteConfigVersion: nil,
        registeredAdapters: 0,
        testMode: false,
        isInitialized: false,
        cacheCounts: [:],
        consentSummary: ConsentManager.shared.getRedactedConsentInfo(),
        cacheExpirationsMs: [:]
    )
}

extension SDKSnapshot: @unchecked Sendable {}

private extension SDKSnapshot {
    func updatingConsentSummary(_ summary: [String: Any]) -> SDKSnapshot {
        SDKSnapshot(
            appId: appId,
            placementIds: placementIds,
            remoteConfigVersion: remoteConfigVersion,
            registeredAdapters: registeredAdapters,
            testMode: testMode,
            isInitialized: isInitialized,
            cacheCounts: cacheCounts,
            consentSummary: summary,
            cacheExpirationsMs: cacheExpirationsMs
        )
    }
}

private actor MediationRuntime {
    private let sdkVersion: String
    private let clock: ClockProtocol

    private struct CachedAd {
        let ad: Ad
        let expiresAtMs: Int64
    }

    private var config: SDKConfig?
    private var remoteConfig: SDKRemoteConfig?
    private var configManager: ConfigManager?
    private var telemetry: TelemetryCollector?
    private var adapterRegistry: AdapterRegistry?
    private var signatureVerifier: SignatureVerifier?
    private var testModeLoader: TestModeAdLoader?
    private var adapterCircuitBreakers: [String: AdapterCircuitBreaker] = [:]
    private var pacingMarks: [String: Int64] = [:]
    private var initialized = false
    private var adCache: [String: [CachedAd]] = [:]
    private let consentManager = ConsentManager.shared
    private var sandboxAdapterWhitelist: Set<String>? = nil
    private var sandboxForceAdapterPipeline = false
    private var pendingAdapterRegistrations: [(String, AdapterTypeToken)] = []

    init(sdkVersion: String, clock: ClockProtocol = Clock.shared) {
        self.sdkVersion = sdkVersion
        self.clock = clock
    }

    func initialize(appId: String, configuration: SDKConfig?) async throws -> SDKSnapshot {
        guard !initialized else {
            throw SDKError.alreadyInitialized
        }

        let providedConfig = configuration ?? SDKConfig.default(appId: appId)
        let resolvedConfig = providedConfig.withAppId(appId)

        if resolvedConfig.configSignaturePublicKey != nil || resolvedConfig.testMode {
            signatureVerifier = SignatureVerifier(
                testMode: Self.shouldEnableTestBypass(for: resolvedConfig),
                productionPublicKey: resolvedConfig.configSignaturePublicKey
            )
        } else {
            signatureVerifier = nil
        }

        let manager = ConfigManager(config: resolvedConfig, signatureVerifier: signatureVerifier, clock: clock)
        // In test mode, prefer mocked network (if registered) so tests can supply remote config.
        // Only bypass remote fetch when no mocks are registered and we are explicitly in offline test mode.
        let remote: SDKRemoteConfig
        if resolvedConfig.testMode && !NetworkTestHooks.hasMockProtocols {
            remote = SDKRemoteConfig(
                version: 0,
                placements: [],
                adapters: [:],
                killswitches: [],
                telemetryEnabled: false,
                enableOmSdk: false,
                ctvOmSdk: nil,
                signature: nil
            )
        } else {
            remote = try await manager.loadConfig()
        }

        let registry = AdapterRegistry(sdkVersion: sdkVersion)
        // Apply any adapters registered by the host app before initialization
        if !pendingAdapterRegistrations.isEmpty {
            for (name, type) in pendingAdapterRegistrations {
                registry.registerAdapter(networkName: name, adapterClass: type.type)
            }
            pendingAdapterRegistrations.removeAll()
        }
        configureAdapters(registry: registry, with: remote)

        let telemetryCollector = TelemetryCollector(config: resolvedConfig)
        telemetryCollector.start()
        telemetryCollector.recordInitialization()
        installOmSdkIfAvailable(remote: remote, config: resolvedConfig, telemetry: telemetryCollector)

        if resolvedConfig.testMode {
            guard let loader = TestModeAdLoader(auctionEndpoint: resolvedConfig.auctionEndpoint) else {
                throw SDKError.internalError(message: "Invalid test-mode auction endpoint")
            }
            testModeLoader = loader
        } else {
            testModeLoader = nil
        }

        config = resolvedConfig
        remoteConfig = remote
        configManager = manager
        telemetry = telemetryCollector
        adapterRegistry = registry
        initialized = true

        return snapshot()
    }

    private func installOmSdkIfAvailable(remote: SDKRemoteConfig, config: SDKConfig, telemetry: TelemetryCollector) {
        let flagEnabled = remote.enableOmSdk ?? config.enableOmSdk
        if flagEnabled == false {
            telemetry.recordOmSdkStatus("disabled_byo")
            return
        }

        let available = OmSdkHelper.shared.initializeIfAvailable(partnerName: "ApexMediation", version: sdkVersion)
        telemetry.recordOmSdkStatus(available ? "enabled" : "missing_sdk_byo")
    }

    func loadAd(placementId: String) async throws -> Ad? {
        guard initialized else {
            throw SDKError.notInitialized
        }

        let remote = try await ensureRemoteConfig()
        guard let placement = remote.placements.first(where: { $0.placementId == placementId }) else {
            throw SDKError.invalidPlacement(placementId)
        }

        if config?.testMode == true && sandboxForceAdapterPipeline == false {
            return try await loadAdUsingTestMode(placement: placement)
        }

        guard let registry = adapterRegistry else {
            throw SDKError.internalError(message: "Adapter registry not initialized")
        }

        var adaptersInPriority = placement.adapterPriority
        if let whitelist = sandboxAdapterWhitelist, !whitelist.isEmpty {
            adaptersInPriority = adaptersInPriority.filter { whitelist.contains($0) }
        }
        guard !adaptersInPriority.isEmpty else {
            return nil
        }

        assert(!Thread.isMainThread, "MediationRuntime.loadAd should not execute on the main thread")

        for networkName in adaptersInPriority {
            guard let adapterConfig = remote.adapters[networkName], adapterConfig.enabled else {
                continue
            }

            guard let adapter = registry.getAdapter(networkName: networkName) else {
                continue
            }

            if !adapter.supportsAdType(placement.adType) {
                continue
            }

            let breaker = adapterCircuitBreaker(for: networkName)
            if breaker.isOpen() {
                let retryAfterSeconds = 15.0
                telemetry?.recordError(
                    errorCode: "adapter_circuit_open",
                    error: SDKError.circuitBreakerOpen(retryAfter: retryAfterSeconds),
                    metadata: consentTelemetryMetadata()
                )
                continue
            }

            if shouldPace(networkName: networkName, placementId: placementId, minRetryMs: 2_000) {
                telemetry?.recordError(
                    errorCode: "adapter_paced",
                    error: SDKError.noFill,
                    metadata: consentTelemetryMetadata()
                )
                continue
            }

            ensureAdapterInitialized(networkName: networkName, adapterConfig: adapterConfig)

            let settings = adapterSettingsWithConsent(adapterConfig.settings.mapValues { $0.value })
            let startMs = clock.monotonicMillis()

            do {
                let ad = try await loadAd(from: adapter, placement: placementId, adType: placement.adType, settings: settings)
                storeAd(ad)
                let latency = Int(clock.monotonicMillis() - startMs)
                breaker.recordSuccess()
                telemetry?.recordAdLoad(
                    placement: placementId,
                    adType: placement.adType,
                    networkName: networkName,
                    latency: latency,
                    success: true,
                    metadata: consentTelemetryMetadata()
                )
                return ad
            } catch let adapterError as AdapterError {
                let latency = Int(clock.monotonicMillis() - startMs)
                var metadata = consentTelemetryMetadata() ?? [:]
                metadata["reason"] = adapterError.normalizedReason()
                if let vendor = adapterError.vendorCode { metadata["vendor_code"] = vendor }

                if adapterError.code == .noFill || adapterError.code == .belowFloor {
                    markPace(networkName: networkName, placementId: placementId)
                    breaker.recordSuccess()
                } else {
                    breaker.recordFailure()
                }

                telemetry?.recordAdLoad(
                    placement: placementId,
                    adType: placement.adType,
                    networkName: networkName,
                    latency: latency,
                    success: false,
                    metadata: metadata
                )

                if adapterError.code == .timeout {
                    telemetry?.recordTimeout(
                        placement: placementId,
                        adType: placement.adType,
                        reason: "adapter_timeout",
                        metadata: metadata
                    )
                } else {
                    telemetry?.recordError(errorCode: "adapter_failure", error: adapterError, metadata: metadata)
                }
            } catch {
                let latency = Int(clock.monotonicMillis() - startMs)
                breaker.recordFailure()
                telemetry?.recordAdLoad(
                    placement: placementId,
                    adType: placement.adType,
                    networkName: networkName,
                    latency: latency,
                    success: false,
                    metadata: consentTelemetryMetadata()
                )
                telemetry?.recordError(errorCode: "adapter_failure", error: error, metadata: consentTelemetryMetadata())
            }
        }

        telemetry?.recordAdLoad(
            placement: placementId,
            adType: placement.adType,
            networkName: "none",
            latency: 0,
            success: false,
            metadata: consentTelemetryMetadata()
        )
        return nil
    }

    private func consentTelemetryMetadata() -> [String: String]? {
        let summary = consentManager.getRedactedConsentInfo()
        if summary.isEmpty { return nil }
        return summary.mapValues { String(describing: $0) }
    }

    func shutdown() -> SDKSnapshot {
        guard initialized else {
            return snapshot()
        }

        telemetry?.stop()
        adapterRegistry?.destroy()
        configManager?.shutdown()
        testModeLoader?.shutdown()

        telemetry = nil
        adapterRegistry = nil
        configManager = nil
        remoteConfig = nil
        config = nil
        signatureVerifier = nil
        testModeLoader = nil
        initialized = false
        return snapshot()
    }

    func snapshot() -> SDKSnapshot {
        pruneExpiredCache()
        return SDKSnapshot(
            appId: config?.appId,
            placementIds: remoteConfig?.placements.map { $0.placementId } ?? [],
            remoteConfigVersion: remoteConfig?.version,
            registeredAdapters: adapterRegistry?.registeredCount ?? 0,
            testMode: config?.testMode ?? false,
            isInitialized: initialized,
            cacheCounts: adCache.mapValues { $0.count },
            consentSummary: consentManager.getRedactedConsentInfo(),
            cacheExpirationsMs: adCache.mapValues { $0.map { $0.expiresAtMs } }
        )
    }

    private func ensureRemoteConfig() async throws -> SDKRemoteConfig {
        if let cached = remoteConfig {
            return cached
        }
        guard let manager = configManager else {
            throw SDKError.internalError(message: "Config manager not initialized")
        }
        let refreshed = try await manager.loadConfig()
        remoteConfig = refreshed
        if let registry = adapterRegistry {
            configureAdapters(registry: registry, with: refreshed)
        }
        return refreshed
    }

    private func configureAdapters(registry: AdapterRegistry, with config: SDKRemoteConfig) {
        for (networkName, adapterConfig) in config.adapters where adapterConfig.enabled {
            let settings = adapterSettingsWithConsent(adapterConfig.settings.mapValues { $0.value })
            registry.initializeAdapter(networkName: networkName, config: settings)
        }
    }

    private func ensureAdapterInitialized(networkName: String, adapterConfig: AdapterConfig) {
        guard let registry = adapterRegistry else { return }
        if !registry.isInitialized(networkName: networkName) {
            let settings = adapterSettingsWithConsent(adapterConfig.settings.mapValues { $0.value })
            registry.initializeAdapter(networkName: networkName, config: settings)
        }
    }

    private func loadAd(
        from adapter: AdNetworkAdapter,
        placement: String,
        adType: AdType,
        settings: [String: Any]
    ) async throws -> Ad {
        try await withCheckedThrowingContinuation { continuation in
            adapter.loadAd(placement: placement, adType: adType, config: settings) { result in
                continuation.resume(with: result)
            }
        }
    }

    private func adapterSettingsWithConsent(_ base: [String: Any]) -> [String: Any] {
        var enriched = base
        let trackingManager = TrackingAuthorizationManager.shared
        let attStatus = trackingManager.currentStatus()
        let adapterConsent = consentManager.toAdapterConsentPayload(attStatusProvider: { attStatus })
        if !adapterConsent.isEmpty {
            enriched["apx_consent_state"] = adapterConsent
        }
        let requestMetadata = consentManager.toAdRequestMetadata()
        if !requestMetadata.isEmpty {
            enriched["apx_consent_metadata"] = requestMetadata
        }
        enriched["apx_can_personalize_ads"] = consentManager.canShowPersonalizedAds()
        enriched["apx_att_status"] = attStatus.rawValue
        enriched["apx_limit_ad_tracking"] = trackingManager.isLimitAdTrackingEnabled()
        if let idfa = trackingManager.advertisingIdentifier() {
            enriched["apx_idfa"] = idfa
        }
        if config?.testMode == true {
            enriched["apx_sandbox"] = true
        }
        return enriched
    }

    func claimAd(placementId: String) -> Ad? {
        pruneExpiredCache(for: placementId)
        guard var ads = adCache[placementId], !ads.isEmpty else { return nil }
        let cachedAd = ads.removeFirst()
        if ads.isEmpty {
            adCache.removeValue(forKey: placementId)
        } else {
            adCache[placementId] = ads
        }
        return cachedAd.ad
    }

    func peekAd(placementId: String) -> Ad? {
        pruneExpiredCache(for: placementId)
        return adCache[placementId]?.first?.ad
    }

    private func storeAd(_ ad: Ad) {
        let now = clock.now()
        let ttlSeconds = ad.expiresAt.timeIntervalSince(now)
        let graceMs = Int64(CacheConfig.expirationGrace * 1000)
        let ttlMs = Int64(ttlSeconds * 1000)
        guard ttlMs + graceMs > 0 else { return }

        pruneExpiredCache(for: ad.placement)
        var ads = adCache[ad.placement] ?? []
        let expirationMs = clock.monotonicMillis() + ttlMs
        ads.append(CachedAd(ad: ad, expiresAtMs: expirationMs))
        adCache[ad.placement] = ads
    }

    private func loadAdUsingTestMode(placement: PlacementConfig) async throws -> Ad {
        guard let loader = testModeLoader else {
            throw SDKError.internalError(message: "Test mode loader unavailable")
        }

        let startMs = clock.monotonicMillis()

        do {
            let ad = try await loader.loadAd(for: placement)
            storeAd(ad)
            let latency = Int(clock.monotonicMillis() - startMs)
            telemetry?.recordAdLoad(
                placement: placement.placementId,
                adType: placement.adType,
                networkName: ad.networkName,
                latency: latency,
                success: true
            )
            return ad
        } catch let error as SDKError {
            let latency = Int(clock.monotonicMillis() - startMs)
            telemetry?.recordAdLoad(
                placement: placement.placementId,
                adType: placement.adType,
                networkName: "test_mode",
                latency: latency,
                success: false
            )
            throw error
        } catch {
            let latency = Int(clock.monotonicMillis() - startMs)
            telemetry?.recordAdLoad(
                placement: placement.placementId,
                adType: placement.adType,
                networkName: "test_mode",
                latency: latency,
                success: false
            )
            throw SDKError.internalError(message: error.localizedDescription)
        }
    }

    private func pruneExpiredCache(for placementId: String? = nil) {
        let nowMs = clock.monotonicMillis()
        let graceMs = Int64(CacheConfig.expirationGrace * 1000)
        if let placementId {
            if var ads = adCache[placementId] {
                ads.removeAll { $0.expiresAtMs + graceMs <= nowMs }
                if ads.isEmpty {
                    adCache.removeValue(forKey: placementId)
                } else {
                    adCache[placementId] = ads
                }
            }
            return
        }
        for key in Array(adCache.keys) {
            pruneExpiredCache(for: key)
        }
    }

    private static func shouldEnableTestBypass(for config: SDKConfig) -> Bool {
        guard config.testMode else { return false }
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

    // MARK: - Sandbox/Diagnostics helpers
    func adapterNames() -> [String] {
        if let registry = adapterRegistry {
            return registry.allNetworkNames()
        }
        // When not initialized, create a temporary registry to discover built-ins
        let temp = AdapterRegistry(sdkVersion: sdkVersion)
        return temp.allNetworkNames()
    }

    func setSandboxAdapterWhitelist(_ names: [String]?) {
        if let names, !names.isEmpty {
            sandboxAdapterWhitelist = Set(names)
        } else {
            sandboxAdapterWhitelist = nil
        }
    }

    func setSandboxForceAdapterPipeline(_ enabled: Bool) {
        sandboxForceAdapterPipeline = enabled
    }

    // MARK: - BYO Adapters
    func registerAdapter(networkName: String, adapterType: AdapterTypeToken) {
        if let registry = adapterRegistry {
            registry.registerAdapter(networkName: networkName, adapterClass: adapterType.type)
        } else {
            pendingAdapterRegistrations.append((networkName, adapterType))
        }
    }

    // MARK: - Adapter pacing/circuit helpers

    private func adapterCircuitBreaker(for networkName: String) -> AdapterCircuitBreaker {
        if adapterCircuitBreakers[networkName] == nil {
            adapterCircuitBreakers[networkName] = AdapterCircuitBreaker(clock: clock)
        }
        return adapterCircuitBreakers[networkName]!
    }

    private func shouldPace(networkName: String, placementId: String, minRetryMs: Int64) -> Bool {
        let key = "\(networkName):\(placementId)"
        let now = clock.monotonicMillis()
        if let last = pacingMarks[key], now - last < minRetryMs {
            return true
        }
        return false
    }

    private func markPace(networkName: String, placementId: String) {
        let key = "\(networkName):\(placementId)"
        pacingMarks[key] = clock.monotonicMillis()
    }
}

// MARK: - Section 3.3 Enhanced Error Taxonomy

public enum SDKError: Error, LocalizedError, Equatable {
    case notInitialized
    case alreadyInitialized
    case invalidPlacement(String)
    case noFill
    case frequencyLimited
    case timeout
    case networkError(underlying: String?)
    case internalError(message: String?)
    case presentationInProgress
    
    // Section 3.3: HTTP status code errors for parity with Android
    case status_429(message: String)
    case status_5xx(code: Int, message: String)
    case circuitBreakerOpen(retryAfter: TimeInterval)

    public var errorDescription: String? {
        switch self {
        case .notInitialized:
            return "SDK not initialized. Call initialize() first."
        case .alreadyInitialized:
            return "SDK already initialized."
        case .invalidPlacement(let placement):
            return "Invalid placement: \(placement)"
        case .noFill:
            return "No ad available."
        case .frequencyLimited:
            return "Ad cannot be shown yet due to frequency capping."
        case .timeout:
            return "Request timed out."
        case .networkError(let underlying):
            if let msg = underlying {
                return "Network error: \(msg)"
            }
            return "Network error occurred."
        case .internalError(let message):
            if let msg = message {
                return "Internal error: \(msg)"
            }
            return "Internal error occurred."
        case .presentationInProgress:
            return "Another ad is already being presented."
        case .status_429(let message):
            return "Rate limit exceeded (429): \(message)"
        case .status_5xx(let code, let message):
            return "Server error (\(code)): \(message)"
        case .circuitBreakerOpen(let retryAfter):
            let seconds = Int(ceil(retryAfter))
            return "Circuit breaker open. Retry after \(seconds)s"
        }
    }
    
    /// Map HTTP status codes to SDK errors (Section 3.3)
    public static func fromHTTPStatus(code: Int, message: String? = nil) -> SDKError {
        switch code {
        case 429:
            return .status_429(message: message ?? "Too many requests")
        case 500...599:
            return .status_5xx(code: code, message: message ?? "Server error")
        case 204:
            return .noFill
        default:
            return .networkError(underlying: message ?? "HTTP \(code)")
        }
    }
    
    // Equatable conformance for testing
    public static func == (lhs: SDKError, rhs: SDKError) -> Bool {
        switch (lhs, rhs) {
        case (.notInitialized, .notInitialized),
             (.alreadyInitialized, .alreadyInitialized),
               (.presentationInProgress, .presentationInProgress),
               (.frequencyLimited, .frequencyLimited),
             (.noFill, .noFill),
             (.timeout, .timeout):
            return true
        case (.invalidPlacement(let lp), .invalidPlacement(let rp)):
            return lp == rp
        case (.networkError(let lu), .networkError(let ru)):
            return lu == ru
        case (.internalError(let lm), .internalError(let rm)):
            return lm == rm
        case (.status_429(let lm), .status_429(let rm)):
            return lm == rm
        case (.status_5xx(let lc, let lm), .status_5xx(let rc, let rm)):
            return lc == rc && lm == rm
        case (.circuitBreakerOpen(let la), .circuitBreakerOpen(let ra)):
            return abs(la - ra) < 0.001
        default:
            return false
        }
    }
}
