import Foundation

/// FallbackWaterfall for tvOS - Implements waterfall mediation for CTV ad loading.
///
/// Optimized for CTV environments with faster timeouts and streaming-aware behavior.
public final class FallbackWaterfall<T> {
    
    /// Default timeout for source loading (shorter for CTV)
    public let defaultTimeout: TimeInterval
    
    public init(defaultTimeout: TimeInterval = 3.0) {
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
    
    public enum WaterfallResult<T> {
        case success(T)
        case noFill(reason: String)
        case error(Error)
        case timeout
    }
    
    public enum AttemptResultType: String {
        case success, noFill, error, timeout, skipped
    }
    
    public struct AttemptDetail {
        public let sourceId: String
        public let priority: Int
        public let durationMs: Int64
        public let result: AttemptResultType
    }
    
    public struct ExecutionResult {
        public let result: WaterfallResult<T>
        public let sourceId: String
        public let attemptsCount: Int
        public let totalDurationMs: Int64
        public let attemptDetails: [AttemptDetail]
    }
    
    public class SourceStats {
        private let lock = NSLock()
        private var _successCount = 0
        private var _failureCount = 0
        private var _timeoutCount = 0
        private var _noFillCount = 0
        private var _totalLatencyMs: Int64 = 0
        
        public var successCount: Int { lock.withLock { _successCount } }
        public var failureCount: Int { lock.withLock { _failureCount } }
        public var timeoutCount: Int { lock.withLock { _timeoutCount } }
        public var noFillCount: Int { lock.withLock { _noFillCount } }
        
        public var totalAttempts: Int {
            lock.withLock { _successCount + _failureCount + _timeoutCount + _noFillCount }
        }
        
        public var successRate: Float {
            let total = totalAttempts
            guard total > 0 else { return 0 }
            return Float(successCount) / Float(total)
        }
        
        public var averageLatencyMs: Int64 {
            let total = totalAttempts
            guard total > 0 else { return 0 }
            return lock.withLock { _totalLatencyMs / Int64(total) }
        }
        
        func record(_ result: AttemptResultType, latencyMs: Int64) {
            lock.lock()
            _totalLatencyMs += latencyMs
            switch result {
            case .success: _successCount += 1
            case .noFill: _noFillCount += 1
            case .error: _failureCount += 1
            case .timeout: _timeoutCount += 1
            case .skipped: break
            }
            lock.unlock()
        }
    }
    
    private var sourceStats: [String: SourceStats] = [:]
    private let statsLock = NSLock()
    
    /// Execute waterfall for CTV ad break
    public func execute(
        sources: [WaterfallSource],
        completion: @escaping (ExecutionResult) -> Void
    ) {
        let startTime = Date()
        let sortedSources = sources.sorted { $0.priority < $1.priority }
        
        executeNext(
            sources: sortedSources,
            index: 0,
            startTime: startTime,
            attemptDetails: [],
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
            
            self?.executeNext(
                sources: sources,
                index: index + 1,
                startTime: startTime,
                attemptDetails: attemptDetails,
                completion: completion
            )
        }
        
        DispatchQueue.global().asyncAfter(deadline: .now() + source.timeout, execute: timeoutWorkItem)
        
        source.loader { [weak self] result in
            guard !completed else { return }
            completed = true
            timeoutWorkItem.cancel()
            
            let duration = Int64(Date().timeIntervalSince(attemptStart) * 1000)
            
            let attemptResultType: AttemptResultType
            switch result {
            case .success: attemptResultType = .success
            case .noFill: attemptResultType = .noFill
            case .error: attemptResultType = .error
            case .timeout: attemptResultType = .timeout
            }
            
            self?.recordStats(sourceId: source.id, result: attemptResultType, durationMs: duration)
            
            attemptDetails.append(AttemptDetail(
                sourceId: source.id,
                priority: source.priority,
                durationMs: duration,
                result: attemptResultType
            ))
            
            if case .success = result {
                let totalDuration = Int64(Date().timeIntervalSince(startTime) * 1000)
                completion(ExecutionResult(
                    result: result,
                    sourceId: source.id,
                    attemptsCount: attemptDetails.count,
                    totalDurationMs: totalDuration,
                    attemptDetails: attemptDetails
                ))
            } else {
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
    
    public func getStats(sourceId: String) -> SourceStats? {
        statsLock.lock()
        defer { statsLock.unlock() }
        return sourceStats[sourceId]
    }
    
    public func getAllStats() -> [String: SourceStats] {
        statsLock.lock()
        defer { statsLock.unlock() }
        return sourceStats
    }
    
    public func clearStats() {
        statsLock.lock()
        sourceStats.removeAll()
        statsLock.unlock()
    }
    
    public func createSource(
        id: String,
        priority: Int,
        timeout: TimeInterval? = nil,
        loader: @escaping (@escaping (WaterfallResult<T>) -> Void) -> Void
    ) -> WaterfallSource {
        WaterfallSource(id: id, priority: priority, timeout: timeout ?? defaultTimeout, loader: loader)
    }
    
    private func recordStats(sourceId: String, result: AttemptResultType, durationMs: Int64) {
        statsLock.lock()
        let stats = sourceStats[sourceId] ?? SourceStats()
        sourceStats[sourceId] = stats
        statsLock.unlock()
        
        stats.record(result, latencyMs: durationMs)
    }
}

extension NSLock {
    func withLock<T>(_ body: () -> T) -> T {
        lock()
        defer { unlock() }
        return body()
    }
}
