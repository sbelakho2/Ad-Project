import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

final class MetricsRecorder {
    static let shared = MetricsRecorder()

    private static func makeDefaultSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 5
        configuration.timeoutIntervalForResource = 5
        return URLSession(configuration: configuration)
    }

    private static let defaultToggleProvider: () -> Bool = { ApexMediation.shared.metricsEnabled }

    private let queue = DispatchQueue(label: "com.rivalapexmediation.ctv.metrics", qos: .background)
    private var counters: [String: Int] = [:]
    private var latencies: [Double] = []
    private var flushTimer: DispatchSourceTimer?
    private var initialized = false
    private var config: SDKConfig?
    private let maxLatencySamples = 200
    private var flushInterval: TimeInterval = 30
    private var session: URLSession = MetricsRecorder.makeDefaultSession()
    private var toggleProvider: () -> Bool = MetricsRecorder.defaultToggleProvider

    private init() {}

    func initialize(config: SDKConfig, session: URLSession? = nil, flushInterval: TimeInterval = 30) {
        queue.async {
            self.config = config
            self.session = session ?? MetricsRecorder.makeDefaultSession()
            let envInterval = ProcessInfo.processInfo.environment["CTV_METRICS_FLUSH_SECONDS"].flatMap { Double($0) }
            let resolvedInterval = envInterval ?? flushInterval
            self.flushInterval = max(0.1, resolvedInterval)
            self.initialized = true
        }
    }

    func recordRequest(durationMs: Double, error: LoadError?) {
        queue.async {
            guard self.canCollect else { return }
            self.increment("requests_total")
            if error == nil || error == .noFill {
                self.increment("requests_success")
            } else {
                self.increment("requests_error")
                self.increment("requests_error_\(error?.reason ?? "unknown")")
            }
            self.addLatency(durationMs)
            self.scheduleFlushLocked()
        }
    }

    func recordPlayback(eventName: String) {
        guard !eventName.isEmpty else { return }
        queue.async {
            guard self.canCollect else { return }
            self.increment("playback_\(eventName)")
            self.scheduleFlushLocked()
        }
    }

    func recordTracker(eventName: String, success: Bool) {
        guard !eventName.isEmpty else { return }
        queue.async {
            guard self.canCollect else { return }
            let suffix = success ? "success" : "failure"
            self.increment("tracker_\(eventName)_\(suffix)")
            self.scheduleFlushLocked()
        }
    }

    private var canCollect: Bool {
        initialized && toggleProvider()
    }

    private func increment(_ key: String, delta: Int = 1) {
        counters[key, default: 0] += delta
    }

    private func addLatency(_ duration: Double) {
        guard duration > 0 else { return }
        latencies.append(duration)
        if latencies.count > maxLatencySamples {
            latencies.removeFirst(latencies.count - maxLatencySamples)
        }
    }

    private func scheduleFlushLocked() {
        guard flushTimer == nil else { return }
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + flushInterval)
        timer.setEventHandler { [weak self] in
            self?.flushLocked()
        }
        timer.resume()
        flushTimer = timer
    }

    private func flushLocked() {
        flushTimer?.cancel()
        flushTimer = nil
        guard let cfg = config else {
            counters.removeAll()
            latencies.removeAll()
            return
        }
        guard canCollect else {
            counters.removeAll()
            latencies.removeAll()
            return
        }
        guard let payload = buildPayloadLocked(appId: cfg.appId),
              let url = URL(string: cfg.apiBaseUrl.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + "/sdk/metrics"),
              let body = try? JSONSerialization.data(withJSONObject: payload) else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.addValue("application/json", forHTTPHeaderField: "Content-Type")
        if let key = cfg.apiKey {
            request.addValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        }
        request.httpBody = body
        session.dataTask(with: request).resume()
        counters.removeAll()
        latencies.removeAll()
    }

    private func buildPayloadLocked(appId: String) -> [String: Any]? {
        if counters.isEmpty && latencies.isEmpty { return nil }
        var payload: [String: Any] = [
            "timestamp": Int(Date().timeIntervalSince1970 * 1000),
            "sdk": "tvos",
            "appId": appId,
            "counters": counters
        ]
        if !latencies.isEmpty {
            let sorted = latencies.sorted()
            payload["request_latency_ms"] = [
                "p50": percentile(sorted, fraction: 0.50),
                "p95": percentile(sorted, fraction: 0.95),
                "p99": percentile(sorted, fraction: 0.99)
            ]
        }
        return payload
    }

    private func percentile(_ values: [Double], fraction: Double) -> Int {
        guard !values.isEmpty else { return 0 }
        let index = Int(Double(values.count - 1) * fraction)
        let clamped = max(0, min(values.count - 1, index))
        return Int(values[clamped])
    }

    func overrideToggleProvider(_ provider: (() -> Bool)?) {
        queue.async {
            self.toggleProvider = provider ?? MetricsRecorder.defaultToggleProvider
        }
    }

    func flushNowForTesting() {
        queue.async {
            self.flushLocked()
        }
    }

    func resetForTesting() {
        queue.sync {
            self.flushTimer?.cancel()
            self.flushTimer = nil
            self.counters.removeAll()
            self.latencies.removeAll()
            self.initialized = false
            self.config = nil
            self.flushInterval = 30
            self.session = MetricsRecorder.makeDefaultSession()
            self.toggleProvider = MetricsRecorder.defaultToggleProvider
        }
    }
}
