import Foundation

// MARK: - NoFillReason

/// Reasons for ad no-fill on CTV
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
    case ctvNotSupported = "ctv_not_supported"
    case formatMismatch = "format_mismatch"
    case unknown = "unknown"
}

// MARK: - NoFillEvent

/// A single no-fill event for CTV
public struct NoFillEvent: Codable {
    public let sourceId: String
    public let placementId: String
    public let reason: NoFillReason
    public let timestamp: Date
    public let latencyMs: Int64
    public let adFormat: String?
    public let metadata: [String: String]
    
    public init(
        sourceId: String,
        placementId: String,
        reason: NoFillReason,
        timestamp: Date = Date(),
        latencyMs: Int64 = 0,
        adFormat: String? = nil,
        metadata: [String: String] = [:]
    ) {
        self.sourceId = sourceId
        self.placementId = placementId
        self.reason = reason
        self.timestamp = timestamp
        self.latencyMs = latencyMs
        self.adFormat = adFormat
        self.metadata = metadata
    }
}

// MARK: - NoFillStats

/// Statistics for no-fill events
public struct NoFillStats {
    public let totalNoFills: Int
    public let noFillRate: Double
    public let averageLatencyMs: Double
    public let topReasons: [NoFillReason: Int]
    public let topSources: [String: Int]
    public let topPlacements: [String: Int]
    public let formatBreakdown: [String: Int]
    public let hourlyBreakdown: [Int: Int]
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
        case formatSpecific = "format_specific"
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

/// Tracks no-fill events for CTV analytics
public final class NoFillTracker {
    
    // MARK: - Singleton
    
    public static let shared = NoFillTracker()
    
    // MARK: - Configuration
    
    public struct Configuration {
        public var maxEventsRetained: Int = 5_000 // Less memory for tvOS
        public var maxRetentionHours: Int = 12 // Shorter retention for CTV
        public var elevatedRateThreshold: Double = 0.5
        public var patternDetectionEnabled: Bool = true
        public var consecutiveFailureThreshold: Int = 3 // More sensitive for CTV
        
        public static let `default` = Configuration()
    }
    
    // MARK: - Properties
    
    private var config: Configuration
    private var events: [NoFillEvent] = []
    private let lock = NSLock()
    
    private var totalNoFills: Int = 0
    private var totalLatencyMs: Int64 = 0
    
    private var hourlyNoFills: [Int: Int] = [:]
    private var noFillsBySource: [String: Int] = [:]
    private var noFillsByPlacement: [String: Int] = [:]
    private var noFillsByReason: [NoFillReason: Int] = [:]
    private var noFillsByFormat: [String: Int] = [:]
    
    private var consecutiveNoFillsBySource: [String: Int] = [:]
    private var detectedPatterns: [NoFillPattern] = []
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
        adFormat: String? = nil,
        metadata: [String: String] = [:]
    ) {
        let event = NoFillEvent(
            sourceId: sourceId,
            placementId: placementId,
            reason: reason,
            latencyMs: latencyMs,
            adFormat: adFormat,
            metadata: metadata
        )
        
        lock.lock()
        defer { lock.unlock() }
        
        events.append(event)
        totalNoFills += 1
        totalLatencyMs += latencyMs
        
        noFillsBySource[sourceId, default: 0] += 1
        noFillsByPlacement[placementId, default: 0] += 1
        noFillsByReason[reason, default: 0] += 1
        
        if let format = adFormat {
            noFillsByFormat[format, default: 0] += 1
        }
        
        let hour = Calendar.current.component(.hour, from: event.timestamp)
        hourlyNoFills[hour, default: 0] += 1
        
        consecutiveNoFillsBySource[sourceId, default: 0] += 1
        
        cleanupIfNeeded()
        
        if config.patternDetectionEnabled {
            detectPatternsForEvent(event)
        }
    }
    
    /// Record a successful fill
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
        
        let oneHourAgo = Date().addingTimeInterval(-3600)
        let recentEvents = events.filter { $0.timestamp >= oneHourAgo }
        let ratePerMinute = Double(recentEvents.count) / 60.0
        
        return NoFillStats(
            totalNoFills: totalNoFills,
            noFillRate: ratePerMinute,
            averageLatencyMs: avgLatency,
            topReasons: noFillsByReason,
            topSources: noFillsBySource,
            topPlacements: noFillsByPlacement,
            formatBreakdown: noFillsByFormat,
            hourlyBreakdown: hourlyNoFills
        )
    }
    
    /// Get no-fills by ad format
    public func getNoFillsByFormat() -> [String: Int] {
        lock.lock()
        defer { lock.unlock() }
        return noFillsByFormat
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
    
    /// Get recent events
    public func getRecentEvents(count: Int = 50) -> [NoFillEvent] {
        lock.lock()
        defer { lock.unlock() }
        return Array(events.suffix(count))
    }
    
    /// Clear all data
    public func clear() {
        lock.lock()
        defer { lock.unlock() }
        
        events.removeAll()
        totalNoFills = 0
        totalLatencyMs = 0
        hourlyNoFills.removeAll()
        noFillsBySource.removeAll()
        noFillsByPlacement.removeAll()
        noFillsByReason.removeAll()
        noFillsByFormat.removeAll()
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
        let cutoff = Date().addingTimeInterval(-Double(config.maxRetentionHours) * 3600)
        events.removeAll { $0.timestamp < cutoff }
        
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
                severity: consecutive >= 10 ? .critical : (consecutive >= 5 ? .high : .medium),
                description: "Source \(event.sourceId) has \(consecutive) consecutive no-fills",
                detectedAt: Date(),
                affectedSourceId: event.sourceId,
                affectedPlacementId: nil
            )
            addPattern(pattern)
        }
        
        // Check for format-specific issues
        if let format = event.adFormat {
            let formatTotal = noFillsByFormat[format] ?? 0
            if formatTotal > 10 {
                let formatRate = Double(formatTotal) / Double(max(1, totalNoFills))
                if formatRate > 0.5 {
                    let pattern = NoFillPattern(
                        type: .formatSpecific,
                        severity: .high,
                        description: "Format '\(format)' accounts for \(Int(formatRate * 100))% of no-fills",
                        detectedAt: Date(),
                        affectedSourceId: nil,
                        affectedPlacementId: nil
                    )
                    addPattern(pattern)
                }
            }
        }
    }
    
    private func addPattern(_ pattern: NoFillPattern) {
        let recentCutoff = Date().addingTimeInterval(-300)
        let isDuplicate = detectedPatterns.contains { existing in
            existing.type == pattern.type &&
            existing.affectedSourceId == pattern.affectedSourceId &&
            existing.detectedAt > recentCutoff
        }
        
        if !isDuplicate {
            detectedPatterns.append(pattern)
            
            let oneDayAgo = Date().addingTimeInterval(-86400)
            detectedPatterns.removeAll { $0.detectedAt < oneDayAgo }
            
            for listener in patternListeners {
                listener(pattern)
            }
        }
    }
}
