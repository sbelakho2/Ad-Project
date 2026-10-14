import Foundation

protocol ClockProtocol {
    func monotonicNow() -> TimeInterval
}

/// Lightweight monotonic clock wrapper to avoid wall-clock drift issues.
final class Clock: ClockProtocol {
    static let shared = Clock()
    private init() {}

    /// Returns monotonic uptime in seconds.
    func monotonicNow() -> TimeInterval {
        return ProcessInfo.processInfo.systemUptime
    }
}
