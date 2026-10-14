import Foundation

/// Abstraction over time sources to enable monotonic timing and test control.
public protocol ClockProtocol: Sendable {
    /// Wall-clock now.
    func now() -> Date
    /// Wall-clock milliseconds since epoch.
    func nowMillis() -> Int64
    /// Monotonic milliseconds since boot.
    func monotonicMillis() -> Int64
    /// Monotonic seconds since boot.
    func monotonicSeconds() -> TimeInterval
}

/// Default system clock using Foundation wall time and process uptime for monotonic duration.
public final class SystemClock: ClockProtocol {
    public init() {}
    public func now() -> Date { Date() }
    public func nowMillis() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }
    public func monotonicMillis() -> Int64 { Int64(ProcessInfo.processInfo.systemUptime * 1000) }
    public func monotonicSeconds() -> TimeInterval { ProcessInfo.processInfo.systemUptime }
}

/// Shared clock instance, overrideable in tests.
@preconcurrency
public enum Clock {
    public static var shared: ClockProtocol {
        get { store.get() }
        set { store.set(newValue) }
    }

    private static let store = ClockStore()
}

private final class ClockStore: @unchecked Sendable {
    private let lock = NSLock()
    private var clock: ClockProtocol = SystemClock()

    func get() -> ClockProtocol { lock.withLock { clock } }
    func set(_ value: ClockProtocol) { lock.withLock { clock = value } }
}

private extension NSLock {
    func withLock<T>(_ body: () -> T) -> T {
        lock()
        defer { unlock() }
        return body()
    }
}
