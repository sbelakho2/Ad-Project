import Foundation

/// Simple circuit breaker to avoid hammering failing adapters.
final class AdapterCircuitBreaker {
    private enum State { case closed, open, halfOpen }

    private let failureThreshold: Int
    private let timeWindowMs: Int
    private let recoveryTimeMs: Int
    private let clock: ClockProtocol

    private var failures: [Int] = []
    private var state: State = .closed
    private var openedAtMs: Int = 0
    private let lock = NSLock()

    init(failureThreshold: Int = 3, timeWindowMs: Int = 30_000, recoveryTimeMs: Int = 15_000, clock: ClockProtocol = Clock.shared) {
        self.failureThreshold = failureThreshold
        self.timeWindowMs = timeWindowMs
        self.recoveryTimeMs = recoveryTimeMs
        self.clock = clock
    }

    func isOpen() -> Bool {
        lock.lock(); defer { lock.unlock() }
        let now = nowMs()
        if state == .open && now - openedAtMs >= recoveryTimeMs {
            state = .halfOpen
        }
        return state == .open
    }

    func recordSuccess() {
        lock.lock(); defer { lock.unlock() }
        if state == .halfOpen {
            state = .closed
            failures.removeAll()
        } else if state == .closed {
            failures.removeAll()
        }
    }

    func recordFailure() {
        lock.lock(); defer { lock.unlock() }
        let now = nowMs()
        failures.append(now)
        failures = failures.filter { now - $0 <= timeWindowMs }
        if state == .halfOpen {
            state = .open
            openedAtMs = now
        } else if state == .closed && failures.count >= failureThreshold {
            state = .open
            openedAtMs = now
        }
    }

    private func nowMs() -> Int {
        Int(clock.monotonicNow() * 1000.0)
    }
}
