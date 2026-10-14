import Foundation
import Dispatch

// MARK: - Circuit Breaker

public final class AdapterCircuitBreaker {
    private let failureThreshold: Int
    private let timeWindowMs: Int64
    private let recoveryTimeMs: Int64
    private let clock: ClockProtocol
    
    private var failures: [Int64] = []
    private var state: State = .closed
    private var openedAt: Int64 = 0
    private let lock = NSLock()
    
    public enum State { case closed, open, halfOpen }
    
    public init(
        failureThreshold: Int = 3,
        timeWindowMs: Int64 = 30_000,
        recoveryTimeMs: Int64 = 15_000,
        clock: ClockProtocol = Clock.shared
    ) {
        self.failureThreshold = failureThreshold
        self.timeWindowMs = timeWindowMs
        self.recoveryTimeMs = recoveryTimeMs
        self.clock = clock
    }
    
    public func getState() -> State {
        lock.lock()
        defer { lock.unlock() }
        
        let now = clock.monotonicMillis()
        if state == .open && now - openedAt >= recoveryTimeMs {
            state = .halfOpen
        }
        return state
    }
    
    public func recordSuccess() {
        lock.lock()
        defer { lock.unlock() }
        
        if state == .halfOpen {
            state = .closed
            failures.removeAll()
        }
    }
    
    public func recordFailure() {
        lock.lock()
        defer { lock.unlock() }
        
        let now = clock.monotonicMillis()
        failures.append(now)
        
        // Remove old failures
        failures = failures.filter { now - $0 <= timeWindowMs }
        
        if state == .closed && failures.count >= failureThreshold {
            state = .open
            openedAt = now
        } else if state == .halfOpen {
            state = .open
            openedAt = now
        }
    }
    
    public func isOpen() -> Bool {
        return getState() == .open
    }
}

// MARK: - Retry Policy

public struct RetryPolicy {
    public static func shouldRetry(error: AdapterError, attemptCount: Int) -> Bool {
        guard attemptCount < 2 else { return false }
        
        switch error.code {
        case .networkError, .timeout:
            return true
        case .error:
            return error.detail.lowercased().contains("5xx") || 
                   error.detail.lowercased().contains("transient")
        default:
            return false
        }
    }
    
    public static func jitterMs() -> Int64 {
        return Int64.random(in: 10...100)
    }
}

// MARK: - Hedge Manager

public final class HedgeManager {
    private var p95Latencies: [String: MovingPercentile] = [:]
    private let lock = NSLock()
    
    public init() {}
    
    public func recordLatency(key: String, latencyMs: Int64) {
        lock.lock()
        defer { lock.unlock() }
        
        if p95Latencies[key] == nil {
            p95Latencies[key] = MovingPercentile(windowSize: 100)
        }
        p95Latencies[key]?.add(value: latencyMs)
    }
    
    public func getHedgeDelayMs(key: String) -> Int64? {
        lock.lock()
        defer { lock.unlock() }
        
        return p95Latencies[key]?.getPercentile(p: 0.95)
    }
    
    class MovingPercentile {
        private var values: [Int64] = []
        private let windowSize: Int
        
        init(windowSize: Int) {
            self.windowSize = windowSize
        }
        
        func add(value: Int64) {
            values.append(value)
            if values.count > windowSize {
                values.removeFirst()
            }
        }
        
        func getPercentile(p: Double) -> Int64? {
            guard !values.isEmpty else { return nil }
            let sorted = values.sorted()
            let index = min(Int(Double(sorted.count) * p), sorted.count - 1)
            return sorted[index]
        }
    }
}

// MARK: - Timeout Enforcer

public final class AdapterTimeoutEnforcer {
    public init() {}
    
    public func withTimeout<T: Sendable>(timeoutMs: Int, operation: @escaping @Sendable () async throws -> T) async throws -> T {
        return try await withThrowingTaskGroup(of: T.self) { group in
            group.addTask {
                return try await operation()
            }
            
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(timeoutMs) * 1_000_000)
                throw AdapterError(code: .timeout, detail: "Operation exceeded \(timeoutMs)ms", vendorCode: nil, recoverable: true)
            }
            
            guard let result = try await group.next() else {
                throw AdapterError(code: .error, detail: "No result", vendorCode: nil, recoverable: false)
            }
            
            group.cancelAll()
            return result
        }
    }
}

// MARK: - Adapter Runtime Wrapper

public final class AdapterRuntimeWrapper: @unchecked Sendable {
    private let adapter: AdNetworkAdapterV2
    private let partnerId: String
    private var circuitBreakers: [String: AdapterCircuitBreaker] = [:]
    private var pacing: [String: Int64] = [:]
    private let hedgeManager = HedgeManager()
    private let timeoutEnforcer = AdapterTimeoutEnforcer()
    private let lock = NSLock()
    private let clock: ClockProtocol
    
    public init(adapter: AdNetworkAdapterV2, partnerId: String, clock: ClockProtocol = Clock.shared) {
        self.adapter = adapter
        self.partnerId = partnerId
        self.clock = clock
    }
    
    private func getCircuitBreaker(placement: String) -> AdapterCircuitBreaker {
        lock.lock()
        defer { lock.unlock() }
        
        let key = "\(partnerId):\(placement)"
        if circuitBreakers[key] == nil {
            circuitBreakers[key] = AdapterCircuitBreaker(clock: clock)
        }
        return circuitBreakers[key]!
    }

    private func shouldPace(placement: String, minRetryMs: Int64) -> Bool {
        lock.lock(); defer { lock.unlock() }
        let key = "\(partnerId):\(placement)"
        let now = clock.monotonicMillis()
        if let last = pacing[key], now - last < minRetryMs {
            return true
        }
        return false
    }

    private func markPace(placement: String) {
        lock.lock(); defer { lock.unlock() }
        let key = "\(partnerId):\(placement)"
        pacing[key] = clock.monotonicMillis()
    }
    
    public func loadInterstitialWithEnforcement(
        placement: String,
        meta: RequestMeta,
        timeoutMs: Int,
        minNoFillRetryMs: Int64 = 2_000
    ) async throws -> LoadResult {
        let cb = getCircuitBreaker(placement: placement)
        
        // Fast-fail if circuit open
        guard !cb.isOpen() else {
            throw AdapterError(code: .circuitOpen, detail: "Circuit breaker open for \(partnerId):\(placement)", vendorCode: nil, recoverable: true)
        }

        if shouldPace(placement: placement, minRetryMs: minNoFillRetryMs) {
            throw AdapterError(code: .noFill, detail: "pacing_guard", vendorCode: nil, recoverable: true)
        }
        
        let startTimeMs = clock.monotonicMillis()
        var attemptCount = 0
        var lastError: AdapterError?
        
        while attemptCount < 2 {
            attemptCount += 1
            
            do {
                let result = try await timeoutEnforcer.withTimeout(timeoutMs: timeoutMs) {
                    return await Task.detached(priority: .userInitiated) {
                        self.adapter.loadInterstitial(placementId: placement, meta: meta, timeoutMs: timeoutMs)
                    }.value
                }
                
                let latency = clock.monotonicMillis() - startTimeMs
                hedgeManager.recordLatency(key: "\(partnerId):\(placement)", latencyMs: latency)
                cb.recordSuccess()
                return result
                
            } catch let error as AdapterError {
                lastError = error

                let shouldRetryError = RetryPolicy.shouldRetry(error: error, attemptCount: attemptCount)
                if error.code == .noFill || error.code == .belowFloor {
                    markPace(placement: placement)
                }
                guard shouldRetryError else {
                    // Do not punish the circuit breaker for no-fill/below-floor responses.
                    if error.code != .noFill && error.code != .belowFloor {
                        cb.recordFailure()
                    } else {
                        cb.recordSuccess()
                    }
                    throw error
                }
                
                try await Task.sleep(nanoseconds: UInt64(RetryPolicy.jitterMs()) * 1_000_000)
                
            } catch {
                let wrappedError = AdapterError(code: .error, detail: error.localizedDescription, vendorCode: nil, recoverable: true)
                lastError = wrappedError
                
                let shouldRetryError = RetryPolicy.shouldRetry(error: wrappedError, attemptCount: attemptCount)
                guard shouldRetryError else {
                    cb.recordFailure()
                    throw wrappedError
                }
                
                try await Task.sleep(nanoseconds: UInt64(RetryPolicy.jitterMs()) * 1_000_000)
            }
        }
        
        cb.recordFailure()
        throw lastError ?? AdapterError(code: .error, detail: "Unknown failure after retries", vendorCode: nil, recoverable: false)
    }
    
    public func loadWithHedging(
        placement: String,
        meta: RequestMeta,
        timeoutMs: Int
    ) async throws -> LoadResult {
        guard let hedgeDelay = hedgeManager.getHedgeDelayMs(key: "\(partnerId):\(placement)"),
              hedgeDelay < timeoutMs else {
            return try await loadInterstitialWithEnforcement(placement: placement, meta: meta, timeoutMs: timeoutMs)
        }
        
        return try await withThrowingTaskGroup(of: LoadResult.self) { group in
            // Primary request
            group.addTask {
                return try await self.loadInterstitialWithEnforcement(placement: placement, meta: meta, timeoutMs: timeoutMs)
            }
            
            // Hedged request
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(hedgeDelay) * 1_000_000)
                return try await self.loadInterstitialWithEnforcement(placement: placement, meta: meta, timeoutMs: timeoutMs - Int(hedgeDelay))
            }
            
            guard let result = try await group.next() else {
                throw AdapterError(code: .error, detail: "No hedge result", vendorCode: nil, recoverable: false)
            }
            
            group.cancelAll()
            return result
        }
    }
    
    @MainActor
    public func showInterstitialOnMain(handle: AdHandle, viewController: AnyObject, callbacks: ShowCallbacks) {
        adapter.showInterstitial(handle: handle, viewController: viewController, callbacks: callbacks)
    }
}

// MARK: - Thread Guard

public struct ThreadGuard {
    public static func assertMainThread(operation: String) {
        assert(Thread.isMainThread, "\(operation) must be called from main thread")
    }
    
    public static func assertNotMainThread(operation: String) {
        assert(!Thread.isMainThread, "\(operation) must not be called from main thread")
    }
}
