import Foundation

/// Lightweight, centralized logger with runtime level control and secret
/// redaction, mirroring the Android `Logger`. Debug and verbose messages are
/// suppressed by default; `BelAds.setDebugLogging(true)` raises the level.
public enum Logger {

    private final class State: @unchecked Sendable {
        let lock = NSLock()
        var level: LogLevel = .info
    }

    private static let state = State()

    public static var level: LogLevel {
        get {
            state.lock.lock()
            defer { state.lock.unlock() }
            return state.level
        }
        set {
            state.lock.lock()
            state.level = newValue
            state.lock.unlock()
        }
    }

    public static func setLevel(_ level: LogLevel) {
        self.level = level
    }

    static func shouldLog(_ candidate: LogLevel) -> Bool {
        switch level {
        case .verbose: return true
        case .debug: return rank(candidate) >= rank(.debug)
        case .info: return rank(candidate) >= rank(.info)
        case .warning: return rank(candidate) >= rank(.warning)
        case .error: return rank(candidate) >= rank(.error)
        }
    }

    private static func rank(_ level: LogLevel) -> Int {
        switch level {
        case .verbose: return 0
        case .debug: return 1
        case .info: return 2
        case .warning: return 3
        case .error: return 4
        }
    }

    public static func v(_ tag: String, _ message: String) { log(.verbose, tag, message) }
    public static func d(_ tag: String, _ message: String) { log(.debug, tag, message) }
    public static func i(_ tag: String, _ message: String) { log(.info, tag, message) }
    public static func w(_ tag: String, _ message: String) { log(.warning, tag, message) }
    public static func e(_ tag: String, _ message: String) { log(.error, tag, message) }

    private static func log(_ candidate: LogLevel, _ tag: String, _ message: String) {
        guard shouldLog(candidate) else { return }
        print("[\(tag)] \(redact(message))")
    }

    /// Masks api keys and `key=value` secrets; mirrors the Android redactor.
    static func redact(_ message: String) -> String {
        var output = message
        output = replace(
            in: output,
            pattern: "(?i)x-api-key[:=]\\s*([A-Za-z0-9_\\r\\n-]{4,})"
        )
        output = replace(
            in: output,
            pattern: "(?i)(api[_-]?key|secret|token)=([A-Za-z0-9-_.]{6,})"
        )
        return output
    }

    private static func replace(in input: String, pattern: String) -> String {
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return input }
        let range = NSRange(input.startIndex..<input.endIndex, in: input)
        var result = input
        for match in regex.matches(in: input, range: range).reversed() {
            guard let full = Range(match.range, in: input) else { continue }
            let secretRange = match.numberOfRanges > 2 ? match.range(at: 2) : (match.numberOfRanges > 1 ? match.range(at: 1) : match.range)
            guard let secret = Range(secretRange, in: input) else { continue }
            let masked = mask(String(input[secret]))
            result.replaceSubrange(full, with: input[full].replacingOccurrences(of: String(input[secret]), with: masked))
        }
        return result
    }

    private static func mask(_ value: String) -> String {
        value.count <= 6 ? "***" : "\(value.prefix(3))***\(value.suffix(3))"
    }
}
