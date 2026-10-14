import Foundation

/// FallbackWaterfall - Implements a waterfall mediation strategy for ad loading.
///
/// When one ad source fails, it automatically falls back to the next source
/// in the waterfall. This ensures maximum fill rate by trying multiple
/// demand partners in priority order.
public final class FallbackWaterfall<T> {
    
    /// Default timeout for source loading
    public let defaultTimeout: TimeInterval
    
    /// Creates a new FallbackWaterfall
    public init(defaultTimeout: TimeInterval = 5.0) {
        self.defaultTimeout = defaultTimeout
    }
    
    /// Represents an ad source in the waterfall
    public struct WaterfallSource {
        public let id: String
        public let priority: Int
        public let timeout: TimeInterval
        public let loader: (@escaping (WaterfallResult<T>) -> Void) -> Void
        
        public init(
            id: String,
            priority: Int,
            timeout: TimeInterval,
            loader: @escaping (@escaping (WaterfallResult<T>) -> Void) -> Void
        ) {
            self.id = id
            self.priority = priority
            self.timeout = timeout
            self.loader = loader
        }
    }
    
    /// Result of attempting to load from a source
    public enum WaterfallResult<T> {
        case success(T)
        case noFill(reason: String)
        case error(Error)
        case timeout
    }
    
    /// Result type for attempt tracking
    public enum AttemptResultType: String {
        case success
        case noFill
        case error
        case timeout
        case skipped
    }
    
    /// Details about each attempt in the waterfall
    public struct AttemptDetail {
        public let sourceId: String
        public let priority: Int
        public let durationMs: Int64
        public let result: AttemptResultType
    }
    
    /// Waterfall execution result with metadata
    public struct ExecutionResult {
        public let result: WaterfallResult<T>
        public let sourceId: String
        public let attemptsCount: Int
        public let totalDurationMs: Int64
        public let attemptDetails: [AttemptDetail]
    }
    
    /// Performance statistics for a source
    public class SourceStats {
        private let lock = NSLock()
        
        private var _successCount: Int = 0
        private var _failureCount: Int = 0
        private var _timeoutCount: Int = 0
        private var _noFillCount: Int = 0
        private var _totalLatencyMs: Int64 = 0
        
        public var successCount: Int {
            lock.lock()
            defer { lock.unlock() }
            return _successCount
        }
        
        public var failureCount: Int {
            lock.lock()
            defer { lock.unlock() }
            return _failureCount
        }
        
        public var timeoutCount: Int {
            lock.lock()
            defer { lock.unlock() }
            return _timeoutCount
        }
        
        public var noFillCount: Int {
            lock.lock()
            defer { lock.unlock() }
            return _noFillCount
        }
        
        public var totalAttempts: Int {
            lock.lock()
            defer { lock.unlock() }
            return _successCount + _failureCount + _timeoutCount + _noFillCount
        }
        
        public var successRate: Float {
            let total = totalAttempts
            guard total > 0 else { return 0 }
            return Float(successCount) / Float(total)
        }
        
        public var averageLatencyMs: Int64 {
            let total = totalAttempts
            guard total > 0 else { return 0 }
            lock.lock()
            defer { lock.unlock() }
            return _totalLatencyMs / Int64(total)
        }
        
        func recordSuccess(latencyMs: Int64) {
            lock.lock()
            _successCount += 1
            _totalLatencyMs += latencyMs
            lock.unlock()
        }
        
        func recordFailure(latencyMs: Int64) {
            lock.lock()
            _failureCount += 1
            _totalLatencyMs += latencyMs
            lock.unlock()
        }
        
        func recordTimeout(latencyMs: Int64) {
            lock.lock()
            _timeoutCount += 1
            _totalLatencyMs += latencyMs
            lock.unlock()
        }
        
        func recordNoFill(latencyMs: Int64) {
            lock.lock()
            _noFillCount += 1
            _totalLatencyMs += latencyMs
            lock.unlock()
        }
    }
    
    private var sourceStats: [String: SourceStats] = [:]
    private let statsLock = NSLock()
    
    /// Executes the waterfall, trying each source in priority order
    public func execute(
        sources: [WaterfallSource],
        completion: @escaping (ExecutionResult) -> Void
    ) {
        let startTime = Date()
        var attemptDetails: [AttemptDetail] = []
        let sortedSources = sources.sorted { $0.priority < $1.priority }
        
        executeNext(
            sources: sortedSources,
            index: 0,
            startTime: startTime,
            attemptDetails: attemptDetails,
            completion: completion
        )
    }
    
    private func executeNext(
        sources: [WaterfallSource],
        index: Int,
        startTime: Date,
        attemptDetails: [AttemptDetail],
        completion: @escaping (ExecutionResult) -> Void
    ) {
        var attemptDetails = attemptDetails
        
        guard index < sources.count else {
            // All sources exhausted
            let totalDuration = Int64(Date().timeIntervalSince(startTime) * 1000)
            completion(ExecutionResult(
                result: .noFill(reason: "All \(sources.count) sources exhausted"),
                sourceId: sources.last?.id ?? "none",
                attemptsCount: attemptDetails.count,
                totalDurationMs: totalDuration,
                attemptDetails: attemptDetails
            ))
            return
        }
        
        let source = sources[index]
        let attemptStart = Date()
        var completed = false
        
        // Set up timeout
        let timeoutWorkItem = DispatchWorkItem { [weak self] in
            guard !completed else { return }
            completed = true
            
            let duration = Int64(Date().timeIntervalSince(attemptStart) * 1000)
            self?.recordStats(sourceId: source.id, result: .timeout, durationMs: duration)
            
            attemptDetails.append(AttemptDetail(
                sourceId: source.id,
                priority: source.priority,
                durationMs: duration,
                result: .timeout
            ))
            
            // Try next source
            self?.executeNext(
                sources: sources,
                index: index + 1,
                startTime: startTime,
                attemptDetails: attemptDetails,
                completion: completion
            )
        }
        
        DispatchQueue.global().asyncAfter(deadline: .now() + source.timeout, execute: timeoutWorkItem)
        
        // Execute loader
        source.loader { [weak self] result in
            guard !completed else { return }
            completed = true
            timeoutWorkItem.cancel()
            
            let duration = Int64(Date().timeIntervalSince(attemptStart) * 1000)
            self?.recordStats(sourceId: source.id, result: result, durationMs: duration)
            
            let attemptResultType: AttemptResultType
            switch result {
            case .success: attemptResultType = .success
            case .noFill: attemptResultType = .noFill
            case .error: attemptResultType = .error
            case .timeout: attemptResultType = .timeout
            }
            
            attemptDetails.append(AttemptDetail(
                sourceId: source.id,
                priority: source.priority,
                durationMs: duration,
                result: attemptResultType
            ))
            
            switch result {
            case .success:
                let totalDuration = Int64(Date().timeIntervalSince(startTime) * 1000)
                completion(ExecutionResult(
                    result: result,
                    sourceId: source.id,
                    attemptsCount: attemptDetails.count,
                    totalDurationMs: totalDuration,
                    attemptDetails: attemptDetails
                ))
                
            case .noFill, .error, .timeout:
                self?.executeNext(
                    sources: sources,
                    index: index + 1,
                    startTime: startTime,
                    attemptDetails: attemptDetails,
                    completion: completion
                )
            }
        }
    }
    
    /// Gets statistics for a specific source
    public func getStats(sourceId: String) -> SourceStats? {
        statsLock.lock()
        defer { statsLock.unlock() }
        return sourceStats[sourceId]
    }
    
    /// Gets statistics for all sources
    public func getAllStats() -> [String: SourceStats] {
        statsLock.lock()
        defer { statsLock.unlock() }
        return sourceStats
    }
    
    /// Clears all recorded statistics
    public func clearStats() {
        statsLock.lock()
        sourceStats.removeAll()
        statsLock.unlock()
    }
    
    /// Creates a source with the given parameters
    public func createSource(
        id: String,
        priority: Int,
        timeout: TimeInterval? = nil,
        loader: @escaping (@escaping (WaterfallResult<T>) -> Void) -> Void
    ) -> WaterfallSource {
        WaterfallSource(
            id: id,
            priority: priority,
            timeout: timeout ?? defaultTimeout,
            loader: loader
        )
    }
    
    private func recordStats(sourceId: String, result: WaterfallResult<T>, durationMs: Int64) {
        statsLock.lock()
        let stats = sourceStats[sourceId] ?? SourceStats()
        sourceStats[sourceId] = stats
        statsLock.unlock()
        
        switch result {
        case .success: stats.recordSuccess(latencyMs: durationMs)
        case .noFill: stats.recordNoFill(latencyMs: durationMs)
        case .error: stats.recordFailure(latencyMs: durationMs)
        case .timeout: stats.recordTimeout(latencyMs: durationMs)
        }
    }
}

/// Builder for creating waterfall configurations
public class WaterfallBuilder<T> {
    private let defaultTimeout: TimeInterval
    private var sources: [FallbackWaterfall<T>.WaterfallSource] = []
    
    public init(defaultTimeout: TimeInterval = 5.0) {
        self.defaultTimeout = defaultTimeout
    }
    
    @discardableResult
    public func addSource(
        id: String,
        priority: Int,
        timeout: TimeInterval? = nil,
        loader: @escaping (@escaping (FallbackWaterfall<T>.WaterfallResult<T>) -> Void) -> Void
    ) -> WaterfallBuilder<T> {
        sources.append(FallbackWaterfall<T>.WaterfallSource(
            id: id,
            priority: priority,
            timeout: timeout ?? defaultTimeout,
            loader: loader
        ))
        return self
    }
    
    public func build() -> [FallbackWaterfall<T>.WaterfallSource] {
        sources
    }
}
