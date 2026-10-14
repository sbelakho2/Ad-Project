import Foundation

// Lightweight telemetry hooks mirror the Android TV metrics recorder.
// MetricsRecorder remains internal and only activates when remote config enables it.

public final class ApexMediation {
    public static let shared = ApexMediation()
    private init() {}

    private var _initialized = false
    private var _config: SDKConfig?
    private var consentMgr: ConsentManager?
    private var auctionClient: AuctionClient?
    private var configManager: ConfigManager?
    private var pacing: [String: Int] = [:]
    private var breakers: [String: AdapterCircuitBreaker] = [:]
    private let clock: ClockProtocol = Clock.shared

    @discardableResult
    public func initialize(config: SDKConfig, completion: ((Bool) -> Void)? = nil) -> Bool {
        guard !_initialized else { completion?(true); return true }
        _config = config
        consentMgr = ConsentManager()
        auctionClient = AuctionClient(config: config)
        configManager = ConfigManager(config: config)
        MetricsRecorder.shared.initialize(config: config)
        configManager?.load()
        _initialized = true
        completion?(true)
        return true
    }

    public var isInitialized: Bool { _initialized }
    public func setConsent(_ consent: ConsentData) { consentMgr?.set(consent) }
    internal var consent: ConsentData { consentMgr?.get() ?? ConsentData() }
    internal var cfg: SDKConfig { _config ?? SDKConfig(appId: "", apiBaseUrl: "https://api.apexmediation.ee/api/v1", apiKey: nil, testMode: false, requestTimeoutMs: 5000) }
    internal var client: AuctionClient { auctionClient ?? AuctionClient(config: cfg) }
    internal func loadGuard(for placementId: String) -> String? {
        if shouldPace(placementId: placementId) { return "pacing" }
        if breaker(for: placementId).isOpen() { return "circuit_open" }
        return configManager?.guardLoad(placementId: placementId)
    }
    internal func showGuard(for placementId: String) -> String? { configManager?.guardShow(placementId: placementId) }
    internal func recordSloSample(success: Bool) { configManager?.recordSloSample(success: success) }
    internal var metricsEnabled: Bool { configManager?.metricsEnabled ?? false }

    // MARK: - Adapter pacing and breaker helpers

    internal func shouldPace(placementId: String, minRetryMs: Int = 2_000) -> Bool {
        let now = Int(clock.monotonicNow() * 1000.0)
        if let last = pacing[placementId], now - last < minRetryMs {
            return true
        }
        return false
    }

    internal func markPaced(placementId: String) {
        pacing[placementId] = Int(clock.monotonicNow() * 1000.0)
    }

    internal func breaker(for placementId: String) -> AdapterCircuitBreaker {
        if let existing = breakers[placementId] { return existing }
        let created = AdapterCircuitBreaker()
        breakers[placementId] = created
        return created
    }

    internal func recordLoadSuccess(placementId: String) {
        breaker(for: placementId).recordSuccess()
    }

    internal func recordLoadFailure(placementId: String, reason: String) {
        let normalized = normalizeReason(reason)
        if normalized == "no_fill" || normalized == "below_floor" {
            markPaced(placementId: placementId)
            breaker(for: placementId).recordSuccess()
            return
        }
        breaker(for: placementId).recordFailure()
    }

    private func normalizeReason(_ reason: String) -> String {
        let lower = reason.lowercased()

        if lower.contains("no_fill") { return "no_fill" }
        if lower.contains("below_floor") { return "below_floor" }
        if lower.contains("timeout") { return "timeout" }
        if lower.contains("network") { return "network_error" }
        if lower.contains("circuit") { return "circuit_open" }

        // Collapse status-specific labels to broad buckets for consistency
        if let code = parseStatusCode(from: lower) {
            if code >= 400 && code < 500 { return "status_4xx" }
            if code >= 500 && code < 600 { return "status_5xx" }
        }

        if lower.contains("status_4xx") { return "status_4xx" }
        if lower.contains("status_5xx") { return "status_5xx" }

        return lower
    }

    private func parseStatusCode(from label: String) -> Int? {
        // Expect patterns like "status_429" or "status_503"
        let parts = label.split(separator: "_")
        guard parts.count >= 2, let code = Int(parts.last ?? "") else { return nil }
        return code
    }
}
