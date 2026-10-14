import Foundation

// MARK: - WeightedAdSource

/// Ad source with priority and weight configuration
public struct WeightedAdSource {
    public let id: String
    public let priority: Int
    public let weight: Double
    public let timeout: TimeInterval
    public let enabled: Bool
    public let minBid: Double
    public let metadata: [String: String]
    
    public init(
        id: String,
        priority: Int,
        weight: Double = 1.0,
        timeout: TimeInterval = 5.0,
        enabled: Bool = true,
        minBid: Double = 0.0,
        metadata: [String: String] = [:]
    ) {
        precondition(priority >= 0, "Priority must be non-negative")
        precondition(weight > 0, "Weight must be positive")
        precondition(timeout > 0, "Timeout must be positive")
        
        self.id = id
        self.priority = priority
        self.weight = weight
        self.timeout = timeout
        self.enabled = enabled
        self.minBid = minBid
        self.metadata = metadata
    }
}

// MARK: - AdLoadResult

/// Result of loading an ad from a source
public enum AdLoadResult {
    case success(ad: Any, sourceId: String, latencyMs: Int64, bid: Double)
    case noFill(sourceId: String, reason: String)
    case error(sourceId: String, error: Error)
    case timeout(sourceId: String)
    
    public var isSuccess: Bool {
        if case .success = self { return true }
        return false
    }
    
    public var sourceId: String {
        switch self {
        case .success(_, let sourceId, _, _): return sourceId
        case .noFill(let sourceId, _): return sourceId
        case .error(let sourceId, _): return sourceId
        case .timeout(let sourceId): return sourceId
        }
    }
}

// MARK: - SourcePerformance

/// Statistics for a source
public struct SourcePerformance {
    public let sourceId: String
    public let totalAttempts: Int
    public let successCount: Int
    public let noFillCount: Int
    public let errorCount: Int
    public let timeoutCount: Int
    public let averageLatencyMs: Double
    public let fillRate: Double
    public let averageBid: Double
    public let effectiveWeight: Double
}

// MARK: - PriorityWeightedConfig

/// Configuration for priority-weighted mediation
public struct PriorityWeightedConfig {
    public var usePerformanceWeighting: Bool
    public var performanceWindowSeconds: TimeInterval
    public var minSampleSize: Int
    public var weightDecayFactor: Double
    public var maxConcurrentRequests: Int
    public var bidFloorEnabled: Bool
    public var adaptiveTimeoutsEnabled: Bool
    
    public init(
        usePerformanceWeighting: Bool = true,
        performanceWindowSeconds: TimeInterval = 3600,
        minSampleSize: Int = 10,
        weightDecayFactor: Double = 0.1,
        maxConcurrentRequests: Int = 3,
        bidFloorEnabled: Bool = true,
        adaptiveTimeoutsEnabled: Bool = true
    ) {
        self.usePerformanceWeighting = usePerformanceWeighting
        self.performanceWindowSeconds = performanceWindowSeconds
        self.minSampleSize = minSampleSize
        self.weightDecayFactor = weightDecayFactor
        self.maxConcurrentRequests = maxConcurrentRequests
        self.bidFloorEnabled = bidFloorEnabled
        self.adaptiveTimeoutsEnabled = adaptiveTimeoutsEnabled
    }
    
    public static let `default` = PriorityWeightedConfig()
}

// MARK: - AdLoader

/// Ad loader function type
public typealias AdLoader = (WeightedAdSource) async throws -> AdLoadResult

// MARK: - PriorityWeightedMediation

/// Priority-weighted mediation manager
public final class PriorityWeightedMediation {
    
    // MARK: - SourceStats
    
    private class SourceStats {
        var attempts: Int = 0
        var successes: Int = 0
        var noFills: Int = 0
        var errors: Int = 0
        var timeouts: Int = 0
        var totalLatencyMs: Int64 = 0
        var totalBidMicros: Int64 = 0
        var lastSuccessTime: Date?
    }
    
    // MARK: - Properties
    
    private let config: PriorityWeightedConfig
    private var sourceStats: [String: SourceStats] = [:]
    private let lock = NSLock()
    
    // MARK: - Initialization
    
    public init(config: PriorityWeightedConfig = .default) {
        self.config = config
    }
    
    // MARK: - Public API
    
    /// Execute mediation with priority-weighted selection
    public func execute(
        sources: [WeightedAdSource],
        loader: @escaping AdLoader
    ) async -> AdLoadResult {
        guard !sources.isEmpty else {
            return .noFill(sourceId: "none", reason: "No sources configured")
        }
        
        let enabledSources = sources.filter { $0.enabled }
        guard !enabledSources.isEmpty else {
            return .noFill(sourceId: "none", reason: "All sources disabled")
        }
        
        // Group by priority
        let priorityGroups = Dictionary(grouping: enabledSources) { $0.priority }
        let sortedPriorities = priorityGroups.keys.sorted()
        
        // Try each priority group in order
        for priority in sortedPriorities {
            guard let group = priorityGroups[priority] else { continue }
            
            let result = await executePriorityGroup(group, loader: loader)
            if result.isSuccess {
                return result
            }
        }
        
        return .noFill(sourceId: "all", reason: "All sources exhausted")
    }
    
    /// Get performance statistics for all sources
    public func getPerformanceStats() -> [SourcePerformance] {
        lock.lock()
        defer { lock.unlock() }
        
        return sourceStats.map { (sourceId, stats) in
            let avgLatency = stats.successes > 0 
                ? Double(stats.totalLatencyMs) / Double(stats.successes) 
                : 0.0
            let fillRate = stats.attempts > 0 
                ? Double(stats.successes) / Double(stats.attempts) 
                : 0.0
            let avgBid = stats.successes > 0 
                ? Double(stats.totalBidMicros) / Double(stats.successes * 1_000_000) 
                : 0.0
            
            return SourcePerformance(
                sourceId: sourceId,
                totalAttempts: stats.attempts,
                successCount: stats.successes,
                noFillCount: stats.noFills,
                errorCount: stats.errors,
                timeoutCount: stats.timeouts,
                averageLatencyMs: avgLatency,
                fillRate: fillRate,
                averageBid: avgBid,
                effectiveWeight: 1.0
            )
        }
    }
    
    /// Get performance for a specific source
    public func getSourcePerformance(sourceId: String) -> SourcePerformance? {
        lock.lock()
        defer { lock.unlock() }
        
        guard let stats = sourceStats[sourceId] else { return nil }
        
        let avgLatency = stats.successes > 0 
            ? Double(stats.totalLatencyMs) / Double(stats.successes) 
            : 0.0
        let fillRate = stats.attempts > 0 
            ? Double(stats.successes) / Double(stats.attempts) 
            : 0.0
        let avgBid = stats.successes > 0 
            ? Double(stats.totalBidMicros) / Double(stats.successes * 1_000_000) 
            : 0.0
        
        return SourcePerformance(
            sourceId: sourceId,
            totalAttempts: stats.attempts,
            successCount: stats.successes,
            noFillCount: stats.noFills,
            errorCount: stats.errors,
            timeoutCount: stats.timeouts,
            averageLatencyMs: avgLatency,
            fillRate: fillRate,
            averageBid: avgBid,
            effectiveWeight: 1.0
        )
    }
    
    /// Reset statistics for a source
    public func resetStats(sourceId: String) {
        lock.lock()
        defer { lock.unlock() }
        sourceStats.removeValue(forKey: sourceId)
    }
    
    /// Reset all statistics
    public func resetAllStats() {
        lock.lock()
        defer { lock.unlock() }
        sourceStats.removeAll()
    }
    
    // MARK: - Private Methods
    
    private func executePriorityGroup(
        _ sources: [WeightedAdSource],
        loader: @escaping AdLoader
    ) async -> AdLoadResult {
        var remainingSources = sources
        
        while !remainingSources.isEmpty {
            let selected = selectByWeight(remainingSources)
            remainingSources.removeAll { $0.id == selected.id }
            
            let result = await executeWithTimeout(selected, loader: loader)
            recordResult(selected.id, result: result)
            
            if result.isSuccess {
                return result
            }
        }
        
        return .noFill(sourceId: "priority_group", reason: "No fill from priority group")
    }
    
    private func selectByWeight(_ sources: [WeightedAdSource]) -> WeightedAdSource {
        guard sources.count > 1 else {
            return sources[0]
        }
        
        let effectiveWeights = sources.map { source -> (WeightedAdSource, Double) in
            (source, calculateEffectiveWeight(source))
        }
        
        let totalWeight = effectiveWeights.reduce(0) { $0 + $1.1 }
        guard totalWeight > 0 else {
            return sources.randomElement()!
        }
        
        var random = Double.random(in: 0..<totalWeight)
        for (source, weight) in effectiveWeights {
            random -= weight
            if random <= 0 {
                return source
            }
        }
        
        return sources.last!
    }
    
    private func calculateEffectiveWeight(_ source: WeightedAdSource) -> Double {
        let baseWeight = source.weight
        
        guard config.usePerformanceWeighting else {
            return baseWeight
        }
        
        lock.lock()
        let stats = sourceStats[source.id]
        lock.unlock()
        
        guard let stats = stats, stats.attempts >= config.minSampleSize else {
            return baseWeight
        }
        
        let fillRate = Double(stats.successes) / max(1.0, Double(stats.attempts))
        let performanceMultiplier = 0.5 + fillRate
        
        return max(0.1, baseWeight * performanceMultiplier)
    }
    
    private func executeWithTimeout(
        _ source: WeightedAdSource,
        loader: @escaping AdLoader
    ) async -> AdLoadResult {
        let timeout = config.adaptiveTimeoutsEnabled 
            ? calculateAdaptiveTimeout(source) 
            : source.timeout
        
        return await withTaskGroup(of: AdLoadResult?.self) { group in
            group.addTask {
                do {
                    return try await loader(source)
                } catch {
                    return .error(sourceId: source.id, error: error)
                }
            }
            
            group.addTask {
                try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                return nil
            }
            
            if let firstResult = await group.next() {
                group.cancelAll()
                return firstResult ?? .timeout(sourceId: source.id)
            }
            
            return .timeout(sourceId: source.id)
        }
    }
    
    private func calculateAdaptiveTimeout(_ source: WeightedAdSource) -> TimeInterval {
        lock.lock()
        let stats = sourceStats[source.id]
        lock.unlock()
        
        guard let stats = stats, stats.successes >= config.minSampleSize else {
            return source.timeout
        }
        
        let avgLatencyMs = Double(stats.totalLatencyMs) / Double(stats.successes)
        let adaptiveTimeoutMs = avgLatencyMs * 2
        
        return min(source.timeout, max(1.0, adaptiveTimeoutMs / 1000))
    }
    
    private func recordResult(_ sourceId: String, result: AdLoadResult) {
        lock.lock()
        defer { lock.unlock() }
        
        let stats = sourceStats[sourceId] ?? SourceStats()
        stats.attempts += 1
        
        switch result {
        case .success(_, _, let latencyMs, let bid):
            stats.successes += 1
            stats.totalLatencyMs += latencyMs
            stats.totalBidMicros += Int64(bid * 1_000_000)
            stats.lastSuccessTime = Date()
        case .noFill:
            stats.noFills += 1
        case .error:
            stats.errors += 1
        case .timeout:
            stats.timeouts += 1
        }
        
        sourceStats[sourceId] = stats
    }
}

// MARK: - WeightedAdSourceBuilder

/// Builder for creating weighted ad source configurations
public final class WeightedAdSourceBuilder {
    private var id: String = ""
    private var priority: Int = 0
    private var weight: Double = 1.0
    private var timeout: TimeInterval = 5.0
    private var enabled: Bool = true
    private var minBid: Double = 0.0
    private var metadata: [String: String] = [:]
    
    public init() {}
    
    @discardableResult
    public func id(_ id: String) -> Self {
        self.id = id
        return self
    }
    
    @discardableResult
    public func priority(_ priority: Int) -> Self {
        self.priority = priority
        return self
    }
    
    @discardableResult
    public func weight(_ weight: Double) -> Self {
        self.weight = weight
        return self
    }
    
    @discardableResult
    public func timeout(_ timeout: TimeInterval) -> Self {
        self.timeout = timeout
        return self
    }
    
    @discardableResult
    public func enabled(_ enabled: Bool) -> Self {
        self.enabled = enabled
        return self
    }
    
    @discardableResult
    public func minBid(_ minBid: Double) -> Self {
        self.minBid = minBid
        return self
    }
    
    @discardableResult
    public func metadata(_ key: String, _ value: String) -> Self {
        self.metadata[key] = value
        return self
    }
    
    public func build() -> WeightedAdSource {
        precondition(!id.isEmpty, "Source ID is required")
        return WeightedAdSource(
            id: id,
            priority: priority,
            weight: weight,
            timeout: timeout,
            enabled: enabled,
            minBid: minBid,
            metadata: metadata
        )
    }
}
