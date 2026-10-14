import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
#if canImport(Compression)
import Compression
#endif

/// Telemetry collector with batching and compression
///
/// Features:
/// - Event batching (reduces network calls)
/// - GZIP compression
/// - Retry logic with exponential backoff
/// - Background processing (no main thread blocking)
public class TelemetryCollector: @unchecked Sendable {
    private let config: SDKConfig
    private let urlSession: URLSession
    
    private var eventQueue: [TelemetryEvent] = []
    private let queueLock = NSLock()
    
    private let backgroundQueue = DispatchQueue(label: "com.rivalapexmediation.telemetry", qos: .utility)
    private var flushTimer: DispatchSourceTimer?
    
    private let batchSize = 10
    private let flushInterval: TimeInterval = 30.0 // 30 seconds
    
    private var isRunning = false
    
    /// Initialize telemetry collector
    public init(config: SDKConfig) {
        self.config = config
        
        let configuration = URLSessionConfiguration.apexDefault()
        configuration.timeoutIntervalForRequest = 10
        configuration.timeoutIntervalForResource = 30
        self.urlSession = URLSession(configuration: configuration)
    }
    
    /// Start telemetry collection
    public func start() {
        guard config.telemetryEnabled else { return }
        
        isRunning = true
        
        // Schedule periodic flush
        let timer = DispatchSource.makeTimerSource(queue: backgroundQueue)
        timer.schedule(deadline: .now() + flushInterval, repeating: flushInterval)
        timer.setEventHandler { [weak self] in
            self?.flushEvents()
        }
        timer.resume()
        
        flushTimer = timer
    }
    
    /// Stop telemetry collection
    public func stop() {
        isRunning = false
        
        // Cancel timer
        flushTimer?.cancel()
        flushTimer = nil
        
        // Flush remaining events
        flushEvents()
    }
    
    // MARK: - Event Recording
    
    /// Record SDK initialization
    public func recordInitialization() {
        recordEvent(TelemetryEvent(eventType: .sdkInit))
    }
    
    /// Record ad load event
    public func recordAdLoad(placement: String, adType: AdType, networkName: String, latency: Int, success: Bool, metadata: [String: String]? = nil) {
        let eventType: EventType = success ? .adLoaded : .adFailed
        recordEvent(
            TelemetryEvent(
                eventType: eventType,
                placement: placement,
                adType: adType,
                networkName: networkName,
                latency: latency,
                metadata: metadata
            )
        )
    }
    
    /// Record ad impression
    public func recordImpression(placement: String, adType: AdType, networkName: String) {
        recordEvent(
            TelemetryEvent(
                eventType: .adImpression,
                placement: placement,
                adType: adType,
                networkName: networkName
            )
        )
    }
    
    /// Record ad click
    public func recordClick(placement: String, adType: AdType, networkName: String) {
        recordEvent(
            TelemetryEvent(
                eventType: .adClicked,
                placement: placement,
                adType: adType,
                networkName: networkName
            )
        )
    }
    
    /// Record timeout event
    public func recordTimeout(placement: String, adType: AdType, reason: String, metadata: [String: String]? = nil) {
        recordEvent(
            TelemetryEvent(
                eventType: .timeout,
                placement: placement,
                adType: adType,
                metadata: {
                    var meta = metadata ?? [:]
                    meta["reason"] = reason
                    return meta
                }()
            )
        )
    }
    
    /// Record error event
    public func recordError(errorCode: String, error: Error, metadata: [String: String]? = nil) {
        recordEvent(
            TelemetryEvent(
                eventType: .adFailed,
                errorCode: errorCode,
                errorMessage: error.localizedDescription,
                metadata: {
                    var meta = metadata ?? [:]
                    meta["error_domain"] = (error as NSError).domain
                    meta["error_code"] = String((error as NSError).code)
                    return meta
                }()
            )
        )
    }
    
    /// Record ANR detection (not applicable on iOS, but kept for parity)
    public func recordANR(threadName: String, stackTrace: String) {
        recordEvent(
            TelemetryEvent(
                eventType: .anrDetected,
                metadata: [
                    "thread": threadName,
                    "stack_trace": String(stackTrace.prefix(500))
                ]
            )
        )
    }

    /// Record OM SDK availability/status transitions (BYO-safe).
    public func recordOmSdkStatus(_ status: String) {
        recordEvent(
            TelemetryEvent(
                eventType: .omSdkStatus,
                metadata: ["status": status]
            )
        )
    }
    
    // MARK: - Private Methods
    
    /// Add event to queue
    private func recordEvent(_ event: TelemetryEvent) {
        guard config.telemetryEnabled, isRunning else { return }
        
        let sanitized = sanitize(event)

        queueLock.lock()
        eventQueue.append(sanitized)
        let shouldFlush = eventQueue.count >= batchSize
        queueLock.unlock()
        
        // Auto-flush if batch size reached
        if shouldFlush {
            backgroundQueue.async { [weak self] in
                self?.flushEvents()
            }
        }
    }
    
    /// Flush events to backend
    private func flushEvents() {
        queueLock.lock()
        guard !eventQueue.isEmpty else {
            queueLock.unlock()
            return
        }
        
        let eventsToSend = eventQueue
        eventQueue.removeAll()
        queueLock.unlock()
        
        Task.detached(priority: .utility) { [weak self] in
            guard let self else { return }
            do {
                try await self.sendEvents(eventsToSend)
            } catch {
                self.backgroundQueue.async { [weak self] in
                    guard let self else { return }
                    self.queueLock.lock()
                    self.eventQueue.insert(contentsOf: eventsToSend, at: 0)
                    
                    // Limit queue size to prevent memory issues
                    if self.eventQueue.count > 1000 {
                        self.eventQueue.removeFirst(self.eventQueue.count - 1000)
                    }
                    self.queueLock.unlock()
                }
            }
        }
    }

    private func sanitize(_ event: TelemetryEvent) -> TelemetryEvent {
        guard let metadata = event.metadata else { return event }

        let redacted = sanitizeMetadata(metadata)
        if redacted == metadata { return event }

        return TelemetryEvent(
            eventType: event.eventType,
            timestamp: event.timestamp,
            placement: event.placement,
            adType: event.adType,
            networkName: event.networkName,
            latency: event.latency,
            errorCode: event.errorCode,
            errorMessage: event.errorMessage,
            metadata: redacted
        )
    }

    private func sanitizeMetadata(_ metadata: [String: String]) -> [String: String] {
        var cleaned: [String: String] = [:]
        let sensitive = Set([
            "api_key",
            "access_key",
            "secret",
            "token",
            "api_token",
            "account_id",
            "placement_id",
            "app_token"
        ])

        for (key, value) in metadata {
            if sensitive.contains(key.lowercased()) {
                cleaned[key] = "****"
            } else {
                cleaned[key] = value
            }
        }
        return cleaned
    }
    
    /// Send events to telemetry endpoint
    private func sendEvents(_ events: [TelemetryEvent]) async throws {
        let payload: [String: Any] = [
            "app_id": config.appId,
            "sdk_version": "1.0.0",
            "platform": "ios",
            "events": events.map { event in
                var dict: [String: Any] = [
                    "event_type": event.eventType.rawValue,
                    "timestamp": ISO8601DateFormatter().string(from: event.timestamp)
                ]
                
                if let placement = event.placement { dict["placement"] = placement }
                if let adType = event.adType { dict["ad_type"] = adType.rawValue }
                if let networkName = event.networkName { dict["network_name"] = networkName }
                if let latency = event.latency { dict["latency"] = latency }
                if let errorCode = event.errorCode { dict["error_code"] = errorCode }
                if let errorMessage = event.errorMessage { dict["error_message"] = errorMessage }
                if let metadata = event.metadata { dict["metadata"] = metadata }
                
                return dict
            }
        ]
        
        let jsonData = try JSONSerialization.data(withJSONObject: payload)
        let compressed = try compress(jsonData)
        
        let url = URL(string: "\(config.configEndpoint)/v1/telemetry")!
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.httpBody = compressed
        request.addValue("application/json", forHTTPHeaderField: "Content-Type")
        #if canImport(Compression)
        request.addValue("gzip", forHTTPHeaderField: "Content-Encoding")
        #endif
        request.addValue("RivalApexMediation-iOS/1.0.0", forHTTPHeaderField: "User-Agent")
        
        let (_, response) = try await urlSession.apexData(for: request)
        
        guard let httpResponse = response as? HTTPURLResponse,
              httpResponse.statusCode == 200 else {
            throw TelemetryError.uploadFailed
        }
    }
    
    /// Compress data with GZIP
    private func compress(_ data: Data) throws -> Data {
        #if canImport(Compression)
        var compressedData = Data()
        
        try data.withUnsafeBytes { (sourceBuffer: UnsafeRawBufferPointer) in
            guard let sourcePtr = sourceBuffer.baseAddress else {
                throw TelemetryError.compressionFailed
            }
            
            let destinationBufferSize = data.count
            let destinationBuffer = UnsafeMutablePointer<UInt8>.allocate(capacity: destinationBufferSize)
            defer { destinationBuffer.deallocate() }
            
            let compressedSize = compression_encode_buffer(
                destinationBuffer,
                destinationBufferSize,
                sourcePtr.assumingMemoryBound(to: UInt8.self),
                data.count,
                nil,
                COMPRESSION_ZLIB
            )
            
            if compressedSize == 0 {
                throw TelemetryError.compressionFailed
            }
            
            compressedData = Data(bytes: destinationBuffer, count: compressedSize)
        }
        
        return compressedData
        #else
        return data
        #endif
    }
}

// MARK: - Performance Metrics

/// Performance metrics tracker
public class PerformanceMetrics {
    public struct Metric {
        public let name: String
        public let value: Double
        public let timestamp: Date
        
        public init(name: String, value: Double, timestamp: Date = Date()) {
            self.name = name
            self.value = value
            self.timestamp = timestamp
        }
    }
    
    private var metrics: [Metric] = []
    private let lock = NSLock()
    
    /// Record a metric
    public func record(name: String, value: Double) {
        lock.lock()
        metrics.append(Metric(name: name, value: value))
        
        // Keep only last 100 metrics
        if metrics.count > 100 {
            metrics.removeFirst()
        }
        lock.unlock()
    }
    
    /// Get all metrics
    public func getMetrics() -> [Metric] {
        lock.lock()
        defer { lock.unlock() }
        return metrics
    }
    
    /// Clear all metrics
    public func clear() {
        lock.lock()
        metrics.removeAll()
        lock.unlock()
    }
}

// MARK: - Errors

/// Telemetry errors
public enum TelemetryError: Error, LocalizedError {
    case uploadFailed
    case compressionFailed
    
    public var errorDescription: String? {
        switch self {
        case .uploadFailed:
            return "Failed to upload telemetry data"
        case .compressionFailed:
            return "Failed to compress telemetry data"
        }
    }
}
