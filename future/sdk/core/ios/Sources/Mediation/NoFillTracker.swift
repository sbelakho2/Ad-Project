import Foundation

// MARK: - NoFillReason

/// Reasons for ad no-fill
public enum NoFillReason: String, Codable {
    case timeout = "timeout"
    case noInventory = "no_inventory"
    case networkError = "network_error"
    case policyViolation = "policy_violation"
    case frequencyCap = "frequency_cap"
    case geographicRestriction = "geographic_restriction"
    case budgetExhausted = "budget_exhausted"
    case malformedResponse = "malformed_response"
    case serverError = "server_error"
    case unknown = "unknown"
}

// MARK: - NoFillEvent

/// A single no-fill event
public struct NoFillEvent: Codable {
    public let sourceId: String
    public let placementId: String
    public let reason: NoFillReason
    public let timestamp: Date
    public let latencyMs: Int64
    public let metadata: [String: String]
    
    public init(
        sourceId: String,
        placementId: String,
        reason: NoFillReason,
        timestamp: Date = Date(),
        latencyMs: Int64 = 0,
        metadata: [String: String] = [:]
    ) {
        self.sourceId = sourceId
        self.placementId = placementId
        self.reason = reason
        self.timestamp = timestamp
        self.latencyMs = latencyMs
        self.metadata = metadata
    }
}

// MARK: - NoFillStats

/// Statistics for no-fill events
public struct NoFillStats {
    public let totalNoFills: Int
    public let noFillRate: Double // Per minute
    public let averageLatencyMs: Double
    public let topReasons: [NoFillReason: Int]
    public let topSources: [String: Int]
    public let topPlacements: [String: Int]
    public let hourlyBreakdown: [Int: Int]
    public let dailyBreakdown: [Int: Int]
}

// MARK: - NoFillPattern

/// Detected no-fill pattern
public struct NoFillPattern {
    public let type: PatternType
    public let severity: Severity
    public let description: String
    public let detectedAt: Date
    public let affectedSourceId: String?
    public let affectedPlacementId: String?
    
    public enum PatternType: String {
        case elevatedRate = "elevated_rate"
        case sourceSpecific = "source_specific"
        case placementSpecific = "placement_specific"
        case timeBasedSpike = "time_based_spike"
        case consecutiveFailures = "consecutive_failures"
    }
    
    public enum Severity: String {
        case low = "low"
        case medium = "medium"
        case high = "high"
        case critical = "critical"
    }
}

// MARK: - NoFillTracker

/// Tracks no-fill events for analytics and pattern detection
public final class NoFillTracker {
    
    // MARK: - Singleton
    
    public static let shared = NoFillTracker()
    
    // MARK: - Configuration
    
    public struct Configuration {
        public var maxEventsRetained: Int = 10_000
        public var maxRetentionHours: Int = 24
        public var elevatedRateThreshold: Double = 0.5 // 50% no-fill rate
        public var patternDetectionEnabled: Bool = true
        public var consecutiveFailureThreshold: Int = 5
        
        public static let `default` = Configuration()
    }
    
    // MARK: - Properties
    
    private var config: Configuration
    private var events: [NoFillEvent] = []
    private let lock = NSLock()
    
    // Aggregated counters
    private var totalNoFills: Int = 0
    private var totalLatencyMs: Int64 = 0
    
    // Hourly and daily breakdowns
    private var hourlyNoFills: [Int: Int] = [:] // Hour of day (0-23) -> count
    private var dailyNoFills: [Int: Int] = [:] // Day of week (1-7) -> count
    
    // Per-source and per-placement counters
    private var noFillsBySource: [String: Int] = [:]
    private var noFillsByPlacement: [String: Int] = [:]
    private var noFillsByReason: [NoFillReason: Int] = [:]
    
    // Pattern detection
    private var consecutiveNoFillsBySource: [String: Int] = [:]
    private var detectedPatterns: [NoFillPattern] = []
    
    // Listeners
    private var patternListeners: [(NoFillPattern) -> Void] = []
    
    // MARK: - Initialization
    
    private init(config: Configuration = .default) {
        self.config = config
    }
    
    // MARK: - Public API
    
    /// Record a no-fill event
    public func recordNoFill(
        sourceId: String,
        placementId: String,
        reason: NoFillReason,
        latencyMs: Int64 = 0,
        metadata: [String: String] = [:]
    ) {
        let event = NoFillEvent(
            sourceId: sourceId,
            placementId: placementId,
            reason: reason,
            latencyMs: latencyMs,
            metadata: metadata
        )
        
        lock.lock()
        defer { lock.unlock() }
        
        // Add event
        events.append(event)
        
        // Update counters
        totalNoFills += 1
        totalLatencyMs += latencyMs
        
        // Update source counter
        noFillsBySource[sourceId, default: 0] += 1
        
        // Update placement counter
        noFillsByPlacement[placementId, default: 0] += 1
        
        // Update reason counter
        noFillsByReason[reason, default: 0] += 1
        
        // Update hourly breakdown
        let calendar = Calendar.current
        let hour = calendar.component(.hour, from: event.timestamp)
        hourlyNoFills[hour, default: 0] += 1
        
        // Update daily breakdown
        let weekday = calendar.component(.weekday, from: event.timestamp)
        dailyNoFills[weekday, default: 0] += 1
        
        // Update consecutive failures
        consecutiveNoFillsBySource[sourceId, default: 0] += 1
        
        // Cleanup old events if needed
        cleanupIfNeeded()
        
        // Detect patterns
        if config.patternDetectionEnabled {
            detectPatternsForEvent(event)
        }
    }
    
    /// Record a successful fill (resets consecutive failure counter)
    public func recordFill(sourceId: String) {
        lock.lock()
        defer { lock.unlock() }
        
        consecutiveNoFillsBySource[sourceId] = 0
    }
    
    /// Get current statistics
    public func getStats() -> NoFillStats {
        lock.lock()
        defer { lock.unlock() }
        
        let avgLatency = totalNoFills > 0 
            ? Double(totalLatencyMs) / Double(totalNoFills) 
            : 0.0
        
        // Calculate rate per minute (based on last hour)
        let oneHourAgo = Date().addingTimeInterval(-3600)
        let recentEvents = events.filter { $0.timestamp >= oneHourAgo }
        let ratePerMinute = Double(recentEvents.count) / 60.0
        
        // Get top reasons
        let topReasons = Dictionary(
            Array(noFillsByReason.sorted { $0.value > $1.value }.prefix(5)),
            uniquingKeysWith: { first, _ in first }
        )
        
        // Get top sources
        let topSources = Dictionary(
            Array(noFillsBySource.sorted { $0.value > $1.value }.prefix(5)),
            uniquingKeysWith: { first, _ in first }
        )
        
        // Get top placements
        let topPlacements = Dictionary(
            Array(noFillsByPlacement.sorted { $0.value > $1.value }.prefix(5)),
            uniquingKeysWith: { first, _ in first }
        )
        
        return NoFillStats(
            totalNoFills: totalNoFills,
            noFillRate: ratePerMinute,
            averageLatencyMs: avgLatency,
            topReasons: topReasons,
            topSources: topSources,
            topPlacements: topPlacements,
            hourlyBreakdown: hourlyNoFills,
            dailyBreakdown: dailyNoFills
        )
    }
    
    /// Get hourly breakdown
    public func getHourlyBreakdown() -> [Int: Int] {
        lock.lock()
        defer { lock.unlock() }
        return hourlyNoFills
    }
    
    /// Get daily breakdown
    public func getDailyBreakdown() -> [Int: Int] {
        lock.lock()
        defer { lock.unlock() }
        return dailyNoFills
    }
    
    /// Get no-fills by source
    public func getNoFillsBySource() -> [String: Int] {
        lock.lock()
        defer { lock.unlock() }
        return noFillsBySource
    }
    
    /// Get no-fills by placement
    public func getNoFillsByPlacement() -> [String: Int] {
        lock.lock()
        defer { lock.unlock() }
        return noFillsByPlacement
    }
    
    /// Get no-fills by reason
    public func getNoFillsByReason() -> [NoFillReason: Int] {
        lock.lock()
        defer { lock.unlock() }
        return noFillsByReason
    }
    
    /// Get events for a specific time range
    public func getEvents(from start: Date, to end: Date) -> [NoFillEvent] {
        lock.lock()
        defer { lock.unlock() }
        
        return events.filter { $0.timestamp >= start && $0.timestamp <= end }
    }
    
    /// Get recent events
    public func getRecentEvents(count: Int = 100) -> [NoFillEvent] {
        lock.lock()
        defer { lock.unlock() }
        
        return Array(events.suffix(count))
    }
    
    /// Get detected patterns
    public func getDetectedPatterns() -> [NoFillPattern] {
        lock.lock()
        defer { lock.unlock() }
        return detectedPatterns
    }
    
    /// Add pattern listener
    public func addPatternListener(_ listener: @escaping (NoFillPattern) -> Void) {
        lock.lock()
        defer { lock.unlock() }
        patternListeners.append(listener)
    }
    
    /// Clear all data
    public func clear() {
        lock.lock()
        defer { lock.unlock() }
        
        events.removeAll()
        totalNoFills = 0
        totalLatencyMs = 0
        hourlyNoFills.removeAll()
        dailyNoFills.removeAll()
        noFillsBySource.removeAll()
        noFillsByPlacement.removeAll()
        noFillsByReason.removeAll()
        consecutiveNoFillsBySource.removeAll()
        detectedPatterns.removeAll()
    }
    
    /// Update configuration
    public func updateConfiguration(_ config: Configuration) {
        lock.lock()
        defer { lock.unlock() }
        self.config = config
    }
    
    // MARK: - Private Methods
    
    private func cleanupIfNeeded() {
        // Remove events exceeding retention limit
        let cutoff = Date().addingTimeInterval(-Double(config.maxRetentionHours) * 3600)
        events.removeAll { $0.timestamp < cutoff }
        
        // Trim to max events
        if events.count > config.maxEventsRetained {
            let excess = events.count - config.maxEventsRetained
            events.removeFirst(excess)
        }
    }
    
    private func detectPatternsForEvent(_ event: NoFillEvent) {
        // Check for consecutive failures
        if let consecutive = consecutiveNoFillsBySource[event.sourceId],
           consecutive >= config.consecutiveFailureThreshold {
            let pattern = NoFillPattern(
                type: .consecutiveFailures,
                severity: consecutive >= 10 ? .high : .medium,
                description: "Source \(event.sourceId) has \(consecutive) consecutive no-fills",
                detectedAt: Date(),
                affectedSourceId: event.sourceId,
                affectedPlacementId: nil
            )
            addPattern(pattern)
        }
        
        // Check for elevated rate on this source
        let sourceTotal = noFillsBySource[event.sourceId] ?? 0
        let totalAttempts = max(1, totalNoFills) // Simplified - in real impl would track fills too
        let sourceRate = Double(sourceTotal) / Double(totalAttempts)
        
        if sourceRate > config.elevatedRateThreshold && sourceTotal > 10 {
            let pattern = NoFillPattern(
                type: .sourceSpecific,
                severity: sourceRate > 0.8 ? .critical : .medium,
                description: "Source \(event.sourceId) has \(Int(sourceRate * 100))% no-fill rate",
                detectedAt: Date(),
                affectedSourceId: event.sourceId,
                affectedPlacementId: nil
            )
            addPattern(pattern)
        }
        
        // Check for placement-specific issues
        let placementTotal = noFillsByPlacement[event.placementId] ?? 0
        if placementTotal > 20 {
            let placementRate = Double(placementTotal) / Double(max(1, totalNoFills))
            if placementRate > 0.3 {
                let pattern = NoFillPattern(
                    type: .placementSpecific,
                    severity: .medium,
                    description: "Placement \(event.placementId) accounts for \(Int(placementRate * 100))% of no-fills",
                    detectedAt: Date(),
                    affectedSourceId: nil,
                    affectedPlacementId: event.placementId
                )
                addPattern(pattern)
            }
        }
    }
    
    private func addPattern(_ pattern: NoFillPattern) {
        // Avoid duplicate patterns within 5 minutes
        let recentCutoff = Date().addingTimeInterval(-300)
        let isDuplicate = detectedPatterns.contains { existing in
            existing.type == pattern.type &&
            existing.affectedSourceId == pattern.affectedSourceId &&
            existing.affectedPlacementId == pattern.affectedPlacementId &&
            existing.detectedAt > recentCutoff
        }
        
        if !isDuplicate {
            detectedPatterns.append(pattern)
            
            // Trim old patterns
            let oneDayAgo = Date().addingTimeInterval(-86400)
            detectedPatterns.removeAll { $0.detectedAt < oneDayAgo }
            
            // Notify listeners
            for listener in patternListeners {
                listener(pattern)
            }
        }
    }
}

// MARK: - NoFillAnalyzer

/// Analyzes no-fill patterns and provides recommendations
public final class NoFillAnalyzer {
    
    public struct Analysis {
        public let summary: String
        public let recommendations: [String]
        public let healthScore: Double // 0-100
        public let problematicSources: [String]
        public let problematicPlacements: [String]
    }
    
    private let tracker: NoFillTracker
    
    public init(tracker: NoFillTracker = .shared) {
        self.tracker = tracker
    }
    
    /// Analyze current no-fill patterns
    public func analyze() -> Analysis {
        let stats = tracker.getStats()
        let patterns = tracker.getDetectedPatterns()
        
        var recommendations: [String] = []
        var problematicSources: [String] = []
        var problematicPlacements: [String] = []
        
        // Analyze top sources
        for (source, count) in stats.topSources {
            let percentage = Double(count) / Double(max(1, stats.totalNoFills)) * 100
            if percentage > 30 {
                problematicSources.append(source)
                recommendations.append("Consider deprioritizing source '\(source)' (accounts for \(Int(percentage))% of no-fills)")
            }
        }
        
        // Analyze top placements
        for (placement, count) in stats.topPlacements {
            let percentage = Double(count) / Double(max(1, stats.totalNoFills)) * 100
            if percentage > 30 {
                problematicPlacements.append(placement)
                recommendations.append("Review placement '\(placement)' configuration (accounts for \(Int(percentage))% of no-fills)")
            }
        }
        
        // Analyze reasons
        if let (topReason, count) = stats.topReasons.max(by: { $0.value < $1.value }) {
            let percentage = Double(count) / Double(max(1, stats.totalNoFills)) * 100
            if percentage > 40 {
                switch topReason {
                case .timeout:
                    recommendations.append("Consider increasing timeout thresholds or improving network conditions")
                case .noInventory:
                    recommendations.append("Explore additional demand sources to improve fill rate")
                case .networkError:
                    recommendations.append("Review network connectivity and retry strategies")
                case .frequencyCap:
                    recommendations.append("Review frequency cap settings - may be too restrictive")
                default:
                    recommendations.append("Investigate high occurrence of '\(topReason.rawValue)' errors")
                }
            }
        }
        
        // Calculate health score (100 = perfect, 0 = critical issues)
        var healthScore: Double = 100.0
        
        // Deduct for high no-fill rate
        let noFillRate = stats.noFillRate
        if noFillRate > 10 {
            healthScore -= min(50, noFillRate * 2)
        }
        
        // Deduct for detected patterns
        let criticalPatterns = patterns.filter { $0.severity == .critical }.count
        let highPatterns = patterns.filter { $0.severity == .high }.count
        healthScore -= Double(criticalPatterns * 15)
        healthScore -= Double(highPatterns * 10)
        healthScore = max(0, min(100, healthScore))
        
        // Generate summary
        let summary: String
        if healthScore >= 80 {
            summary = "No-fill tracking is healthy with \(stats.totalNoFills) events recorded."
        } else if healthScore >= 50 {
            summary = "Moderate no-fill issues detected. \(recommendations.count) recommendations available."
        } else {
            summary = "Critical no-fill issues detected! Immediate attention recommended."
        }
        
        return Analysis(
            summary: summary,
            recommendations: recommendations,
            healthScore: healthScore,
            problematicSources: problematicSources,
            problematicPlacements: problematicPlacements
        )
    }
}
