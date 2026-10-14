import Foundation

// MARK: - Usage Event Types

/// Types of billable usage events
public enum UsageEventType: String, Codable {
    case adRequest = "ad_request"
    case adImpression = "ad_impression"
    case adClick = "ad_click"
    case adVideoStart = "ad_video_start"
    case adVideoComplete = "ad_video_complete"
    case adRevenue = "ad_revenue"
    case apiCall = "api_call"
    case cacheHit = "cache_hit"
    case cacheMiss = "cache_miss"
    case error = "error"
}

// MARK: - Usage Event

/// A single usage event for metering
public struct UsageEvent {
    public let type: UsageEventType
    public let timestamp: Date
    public let placementId: String?
    public let adapterId: String?
    public let adFormat: String?
    public let revenueAmount: Double?
    public let metadata: [String: Any]?
    
    public init(
        type: UsageEventType,
        timestamp: Date = Date(),
        placementId: String? = nil,
        adapterId: String? = nil,
        adFormat: String? = nil,
        revenueAmount: Double? = nil,
        metadata: [String: Any]? = nil
    ) {
        self.type = type
        self.timestamp = timestamp
        self.placementId = placementId
        self.adapterId = adapterId
        self.adFormat = adFormat
        self.revenueAmount = revenueAmount
        self.metadata = metadata
    }
}

// MARK: - Usage Metrics

/// Aggregated usage metrics for a period
public struct UsageMetrics: Codable {
    public let adRequests: Int64
    public let adImpressions: Int64
    public let adClicks: Int64
    public let videoStarts: Int64
    public let videoCompletes: Int64
    public let totalRevenue: Double
    public let apiCalls: Int64
    public let cacheHits: Int64
    public let cacheMisses: Int64
    public let errors: Int64
    public let periodStart: Date
    public let periodEnd: Date
    
    public init(
        adRequests: Int64 = 0,
        adImpressions: Int64 = 0,
        adClicks: Int64 = 0,
        videoStarts: Int64 = 0,
        videoCompletes: Int64 = 0,
        totalRevenue: Double = 0,
        apiCalls: Int64 = 0,
        cacheHits: Int64 = 0,
        cacheMisses: Int64 = 0,
        errors: Int64 = 0,
        periodStart: Date = Date(),
        periodEnd: Date = Date()
    ) {
        self.adRequests = adRequests
        self.adImpressions = adImpressions
        self.adClicks = adClicks
        self.videoStarts = videoStarts
        self.videoCompletes = videoCompletes
        self.totalRevenue = totalRevenue
        self.apiCalls = apiCalls
        self.cacheHits = cacheHits
        self.cacheMisses = cacheMisses
        self.errors = errors
        self.periodStart = periodStart
        self.periodEnd = periodEnd
    }
}

// MARK: - Usage Breakdown

/// Metrics broken down by dimension
public struct UsageBreakdown {
    public let byPlacement: [String: UsageMetrics]
    public let byAdapter: [String: UsageMetrics]
    public let byAdFormat: [String: UsageMetrics]
}

// MARK: - Metering Configuration

/// Configuration for the usage meter
public struct MeteringConfig {
    public let flushInterval: TimeInterval
    public let maxEventsBeforeFlush: Int
    public let enableLocalStorage: Bool
    public let enableRemoteReporting: Bool
    public let samplingRate: Double
    
    public init(
        flushInterval: TimeInterval = 60,
        maxEventsBeforeFlush: Int = 1000,
        enableLocalStorage: Bool = true,
        enableRemoteReporting: Bool = true,
        samplingRate: Double = 1.0
    ) {
        self.flushInterval = flushInterval
        self.maxEventsBeforeFlush = maxEventsBeforeFlush
        self.enableLocalStorage = enableLocalStorage
        self.enableRemoteReporting = enableRemoteReporting
        self.samplingRate = min(1.0, max(0.0, samplingRate))
    }
}

// MARK: - Metering Reporter Protocol

/// Protocol for reporting metrics to remote service
public protocol MeteringReporter {
    func report(metrics: UsageMetrics, breakdown: UsageBreakdown) async -> Bool
}

// MARK: - Thread-safe Counter

/// Atomic counter using OSAtomic operations
private class AtomicCounter {
    private var value: Int64 = 0
    private let lock = NSLock()
    
    func increment() {
        lock.lock()
        defer { lock.unlock() }
        value += 1
    }
    
    func add(_ amount: Int64) {
        lock.lock()
        defer { lock.unlock() }
        value += amount
    }
    
    func get() -> Int64 {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
    
    func reset() {
        lock.lock()
        defer { lock.unlock() }
        value = 0
    }
}

/// Atomic double counter
private class AtomicDoubleCounter {
    private var value: Double = 0.0
    private let lock = NSLock()
    
    func add(_ amount: Double) {
        lock.lock()
        defer { lock.unlock() }
        value += amount
    }
    
    func get() -> Double {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
    
    func reset() {
        lock.lock()
        defer { lock.unlock() }
        value = 0.0
    }
}

// MARK: - Dimension Metrics

/// Metrics for a single dimension (placement, adapter, format)
private class DimensionMetrics {
    let adRequests = AtomicCounter()
    let adImpressions = AtomicCounter()
    let adClicks = AtomicCounter()
    let videoStarts = AtomicCounter()
    let videoCompletes = AtomicCounter()
    let totalRevenue = AtomicDoubleCounter()
    let apiCalls = AtomicCounter()
    let cacheHits = AtomicCounter()
    let cacheMisses = AtomicCounter()
    let errors = AtomicCounter()
    
    func toMetrics(periodStart: Date, periodEnd: Date) -> UsageMetrics {
        return UsageMetrics(
            adRequests: adRequests.get(),
            adImpressions: adImpressions.get(),
            adClicks: adClicks.get(),
            videoStarts: videoStarts.get(),
            videoCompletes: videoCompletes.get(),
            totalRevenue: totalRevenue.get(),
            apiCalls: apiCalls.get(),
            cacheHits: cacheHits.get(),
            cacheMisses: cacheMisses.get(),
            errors: errors.get(),
            periodStart: periodStart,
            periodEnd: periodEnd
        )
    }
    
    func reset() {
        adRequests.reset()
        adImpressions.reset()
        adClicks.reset()
        videoStarts.reset()
        videoCompletes.reset()
        totalRevenue.reset()
        apiCalls.reset()
        cacheHits.reset()
        cacheMisses.reset()
        errors.reset()
    }
}

// MARK: - Usage Meter

/// Tracks and reports billable usage events
public actor UsageMeter {
    private let config: MeteringConfig
    private let reporter: MeteringReporter?
    
    // Global counters
    private let adRequests = AtomicCounter()
    private let adImpressions = AtomicCounter()
    private let adClicks = AtomicCounter()
    private let videoStarts = AtomicCounter()
    private let videoCompletes = AtomicCounter()
    private let totalRevenue = AtomicDoubleCounter()
    private let apiCalls = AtomicCounter()
    private let cacheHits = AtomicCounter()
    private let cacheMisses = AtomicCounter()
    private let errors = AtomicCounter()
    
    // Dimensional breakdowns
    private var placementMetrics: [String: DimensionMetrics] = [:]
    private var adapterMetrics: [String: DimensionMetrics] = [:]
    private var formatMetrics: [String: DimensionMetrics] = [:]
    
    // Pending events for batch storage
    private var pendingEvents: [UsageEvent] = []
    
    // Period tracking
    private var periodStart: Date = Date()
    
    // Timer for periodic flush
    private var flushTask: Task<Void, Never>?
    private var isRunning = false
    
    public init(config: MeteringConfig = MeteringConfig(), reporter: MeteringReporter? = nil) {
        self.config = config
        self.reporter = reporter
    }
    
    /// Start the metering service
    public func start() {
        guard !isRunning else { return }
        isRunning = true
        periodStart = Date()
        
        flushTask = Task {
            while !Task.isCancelled && isRunning {
                try? await Task.sleep(nanoseconds: UInt64(config.flushInterval * 1_000_000_000))
                await flush()
            }
        }
    }
    
    /// Stop the metering service
    public func stop() {
        isRunning = false
        flushTask?.cancel()
        flushTask = nil
    }
    
    /// Record a usage event
    public nonisolated func record(_ event: UsageEvent) {
        // Apply sampling
        if config.samplingRate < 1.0 && Double.random(in: 0...1) > config.samplingRate {
            return
        }
        
        Task {
            await recordInternal(event)
        }
    }
    
    private func recordInternal(_ event: UsageEvent) {
        // Update global counters
        updateCounters(event)
        
        // Update dimensional breakdowns
        if let placementId = event.placementId {
            let metrics = getOrCreatePlacementMetrics(placementId)
            updateDimensionMetrics(metrics, event: event)
        }
        
        if let adapterId = event.adapterId {
            let metrics = getOrCreateAdapterMetrics(adapterId)
            updateDimensionMetrics(metrics, event: event)
        }
        
        if let format = event.adFormat {
            let metrics = getOrCreateFormatMetrics(format)
            updateDimensionMetrics(metrics, event: event)
        }
        
        // Queue event for storage
        if config.enableLocalStorage {
            pendingEvents.append(event)
            if pendingEvents.count >= config.maxEventsBeforeFlush {
                Task { await flush() }
            }
        }
    }
    
    private func getOrCreatePlacementMetrics(_ placementId: String) -> DimensionMetrics {
        if let existing = placementMetrics[placementId] {
            return existing
        }
        let metrics = DimensionMetrics()
        placementMetrics[placementId] = metrics
        return metrics
    }
    
    private func getOrCreateAdapterMetrics(_ adapterId: String) -> DimensionMetrics {
        if let existing = adapterMetrics[adapterId] {
            return existing
        }
        let metrics = DimensionMetrics()
        adapterMetrics[adapterId] = metrics
        return metrics
    }
    
    private func getOrCreateFormatMetrics(_ format: String) -> DimensionMetrics {
        if let existing = formatMetrics[format] {
            return existing
        }
        let metrics = DimensionMetrics()
        formatMetrics[format] = metrics
        return metrics
    }
    
    private func updateCounters(_ event: UsageEvent) {
        switch event.type {
        case .adRequest: adRequests.increment()
        case .adImpression: adImpressions.increment()
        case .adClick: adClicks.increment()
        case .adVideoStart: videoStarts.increment()
        case .adVideoComplete: videoCompletes.increment()
        case .adRevenue:
            if let amount = event.revenueAmount { totalRevenue.add(amount) }
        case .apiCall: apiCalls.increment()
        case .cacheHit: cacheHits.increment()
        case .cacheMiss: cacheMisses.increment()
        case .error: errors.increment()
        }
    }
    
    private func updateDimensionMetrics(_ metrics: DimensionMetrics, event: UsageEvent) {
        switch event.type {
        case .adRequest: metrics.adRequests.increment()
        case .adImpression: metrics.adImpressions.increment()
        case .adClick: metrics.adClicks.increment()
        case .adVideoStart: metrics.videoStarts.increment()
        case .adVideoComplete: metrics.videoCompletes.increment()
        case .adRevenue:
            if let amount = event.revenueAmount { metrics.totalRevenue.add(amount) }
        case .apiCall: metrics.apiCalls.increment()
        case .cacheHit: metrics.cacheHits.increment()
        case .cacheMiss: metrics.cacheMisses.increment()
        case .error: metrics.errors.increment()
        }
    }
    
    // MARK: - Convenience Recording Methods
    
    public nonisolated func recordRequest(placementId: String, adapterId: String? = nil, adFormat: String? = nil) {
        record(UsageEvent(type: .adRequest, placementId: placementId, adapterId: adapterId, adFormat: adFormat))
    }
    
    public nonisolated func recordImpression(placementId: String, adapterId: String, adFormat: String? = nil) {
        record(UsageEvent(type: .adImpression, placementId: placementId, adapterId: adapterId, adFormat: adFormat))
    }
    
    public nonisolated func recordClick(placementId: String, adapterId: String, adFormat: String? = nil) {
        record(UsageEvent(type: .adClick, placementId: placementId, adapterId: adapterId, adFormat: adFormat))
    }
    
    public nonisolated func recordRevenue(placementId: String, adapterId: String, amount: Double) {
        record(UsageEvent(type: .adRevenue, placementId: placementId, adapterId: adapterId, revenueAmount: amount))
    }
    
    public nonisolated func recordVideoStart(placementId: String, adapterId: String) {
        record(UsageEvent(type: .adVideoStart, placementId: placementId, adapterId: adapterId, adFormat: "video"))
    }
    
    public nonisolated func recordVideoComplete(placementId: String, adapterId: String) {
        record(UsageEvent(type: .adVideoComplete, placementId: placementId, adapterId: adapterId, adFormat: "video"))
    }
    
    public nonisolated func recordCacheHit(placementId: String? = nil) {
        record(UsageEvent(type: .cacheHit, placementId: placementId))
    }
    
    public nonisolated func recordCacheMiss(placementId: String? = nil) {
        record(UsageEvent(type: .cacheMiss, placementId: placementId))
    }
    
    public nonisolated func recordError(placementId: String? = nil, adapterId: String? = nil) {
        record(UsageEvent(type: .error, placementId: placementId, adapterId: adapterId))
    }
    
    // MARK: - Metrics Retrieval
    
    /// Get current metrics snapshot
    public func getMetrics() -> UsageMetrics {
        let now = Date()
        return UsageMetrics(
            adRequests: adRequests.get(),
            adImpressions: adImpressions.get(),
            adClicks: adClicks.get(),
            videoStarts: videoStarts.get(),
            videoCompletes: videoCompletes.get(),
            totalRevenue: totalRevenue.get(),
            apiCalls: apiCalls.get(),
            cacheHits: cacheHits.get(),
            cacheMisses: cacheMisses.get(),
            errors: errors.get(),
            periodStart: periodStart,
            periodEnd: now
        )
    }
    
    /// Get breakdown by dimensions
    public func getBreakdown() -> UsageBreakdown {
        let now = Date()
        
        return UsageBreakdown(
            byPlacement: placementMetrics.mapValues { $0.toMetrics(periodStart: periodStart, periodEnd: now) },
            byAdapter: adapterMetrics.mapValues { $0.toMetrics(periodStart: periodStart, periodEnd: now) },
            byAdFormat: formatMetrics.mapValues { $0.toMetrics(periodStart: periodStart, periodEnd: now) }
        )
    }
    
    // MARK: - Computed Metrics
    
    /// Get click-through rate
    public func getCTR() -> Double {
        let impressions = adImpressions.get()
        guard impressions > 0 else { return 0 }
        return Double(adClicks.get()) / Double(impressions)
    }
    
    /// Get fill rate
    public func getFillRate() -> Double {
        let requests = adRequests.get()
        guard requests > 0 else { return 0 }
        return Double(adImpressions.get()) / Double(requests)
    }
    
    /// Get video completion rate
    public func getVideoCompletionRate() -> Double {
        let starts = videoStarts.get()
        guard starts > 0 else { return 0 }
        return Double(videoCompletes.get()) / Double(starts)
    }
    
    /// Get cache hit rate
    public func getCacheHitRate() -> Double {
        let total = cacheHits.get() + cacheMisses.get()
        guard total > 0 else { return 0 }
        return Double(cacheHits.get()) / Double(total)
    }
    
    /// Get effective CPM
    public func getEffectiveCPM() -> Double {
        let impressions = adImpressions.get()
        guard impressions > 0 else { return 0 }
        return (totalRevenue.get() / Double(impressions)) * 1000.0
    }
    
    // MARK: - Flush and Reset
    
    /// Flush metrics to reporter
    @discardableResult
    public func flush() async -> Bool {
        guard config.enableRemoteReporting, let reporter = reporter else {
            return true
        }
        
        let metrics = getMetrics()
        let breakdown = getBreakdown()
        
        let success = await reporter.report(metrics: metrics, breakdown: breakdown)
        if success {
            reset()
        }
        return success
    }
    
    /// Reset all counters
    public func reset() {
        adRequests.reset()
        adImpressions.reset()
        adClicks.reset()
        videoStarts.reset()
        videoCompletes.reset()
        totalRevenue.reset()
        apiCalls.reset()
        cacheHits.reset()
        cacheMisses.reset()
        errors.reset()
        
        placementMetrics.values.forEach { $0.reset() }
        adapterMetrics.values.forEach { $0.reset() }
        formatMetrics.values.forEach { $0.reset() }
        
        pendingEvents.removeAll()
        periodStart = Date()
    }
    
    /// Export metrics as JSON
    public func exportAsJSON() throws -> String {
        let metrics = getMetrics()
        
        let export: [String: Any] = [
            "metrics": [
                "adRequests": metrics.adRequests,
                "adImpressions": metrics.adImpressions,
                "adClicks": metrics.adClicks,
                "videoStarts": metrics.videoStarts,
                "videoCompletes": metrics.videoCompletes,
                "totalRevenue": metrics.totalRevenue,
                "apiCalls": metrics.apiCalls,
                "cacheHits": metrics.cacheHits,
                "cacheMisses": metrics.cacheMisses,
                "errors": metrics.errors,
                "periodStart": ISO8601DateFormatter().string(from: metrics.periodStart),
                "periodEnd": ISO8601DateFormatter().string(from: metrics.periodEnd)
            ],
            "computed": [
                "ctr": getCTR(),
                "fillRate": getFillRate(),
                "videoCompletionRate": getVideoCompletionRate(),
                "cacheHitRate": getCacheHitRate(),
                "effectiveCPM": getEffectiveCPM()
            ],
            "breakdown": [
                "placementCount": placementMetrics.count,
                "adapterCount": adapterMetrics.count,
                "formatCount": formatMetrics.count
            ]
        ]
        
        let data = try JSONSerialization.data(withJSONObject: export, options: .prettyPrinted)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

// MARK: - Builder

/// Builder for UsageMeter
public class UsageMeterBuilder {
    private var config = MeteringConfig()
    private var reporter: MeteringReporter?
    
    public init() {}
    
    public func config(_ config: MeteringConfig) -> Self {
        self.config = config
        return self
    }
    
    public func reporter(_ reporter: MeteringReporter) -> Self {
        self.reporter = reporter
        return self
    }
    
    public func flushInterval(_ interval: TimeInterval) -> Self {
        self.config = MeteringConfig(
            flushInterval: interval,
            maxEventsBeforeFlush: config.maxEventsBeforeFlush,
            enableLocalStorage: config.enableLocalStorage,
            enableRemoteReporting: config.enableRemoteReporting,
            samplingRate: config.samplingRate
        )
        return self
    }
    
    public func samplingRate(_ rate: Double) -> Self {
        self.config = MeteringConfig(
            flushInterval: config.flushInterval,
            maxEventsBeforeFlush: config.maxEventsBeforeFlush,
            enableLocalStorage: config.enableLocalStorage,
            enableRemoteReporting: config.enableRemoteReporting,
            samplingRate: rate
        )
        return self
    }
    
    public func build() -> UsageMeter {
        return UsageMeter(config: config, reporter: reporter)
    }
}
