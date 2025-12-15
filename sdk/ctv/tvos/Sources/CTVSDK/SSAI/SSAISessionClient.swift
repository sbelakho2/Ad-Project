import Foundation

/// SSAI Session Client for tvOS SDK
///
/// Manages SSAI session lifecycle including creation, tracking, and cleanup.
public final class SSAISessionClient {
    
    // MARK: - Configuration
    
    public struct Config {
        /// Request timeout (seconds)
        public let requestTimeout: TimeInterval
        
        /// Retry count for failed requests
        public let maxRetries: Int
        
        /// Retry backoff base (seconds)
        public let retryBackoff: TimeInterval
        
        /// Enable tracking batching
        public let enableBatching: Bool
        
        /// Batch interval (seconds)
        public let batchInterval: TimeInterval
        
        /// Max events per batch
        public let maxBatchSize: Int
        
        public init(
            requestTimeout: TimeInterval = 5.0,
            maxRetries: Int = 2,
            retryBackoff: TimeInterval = 0.1,
            enableBatching: Bool = true,
            batchInterval: TimeInterval = 1.0,
            maxBatchSize: Int = 10
        ) {
            self.requestTimeout = requestTimeout
            self.maxRetries = maxRetries
            self.retryBackoff = retryBackoff
            self.enableBatching = enableBatching
            self.batchInterval = batchInterval
            self.maxBatchSize = maxBatchSize
        }
    }
    
    // MARK: - Properties
    
    private let config: Config
    private var ssaiConfig: SSAIConfig?
    private var session: SSAISession?
    private var state: SSAISessionState = .idle
    private var eventQueue: [SSAITrackingEvent] = []
    private var batchTimer: Timer?
    private var firedEvents: Set<String> = []
    private var metrics = SSAIMetrics()
    
    private let urlSession: URLSession
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    
    // MARK: - Initialization
    
    public init(config: Config = Config()) {
        self.config = config
        
        let urlConfig = URLSessionConfiguration.default
        urlConfig.timeoutIntervalForRequest = config.requestTimeout
        urlConfig.waitsForConnectivity = true
        self.urlSession = URLSession(configuration: urlConfig)
        
        encoder.dateEncodingStrategy = .iso8601
        decoder.dateDecodingStrategy = .iso8601
    }
    
    // MARK: - Public Interface
    
    /// Get current session state
    public var sessionState: SSAISessionState { state }
    
    /// Get current session
    public var currentSession: SSAISession? { session }
    
    /// Get metrics
    public var currentMetrics: SSAIMetrics { metrics }
    
    /// Create a new SSAI session
    public func createSession(
        config: SSAIConfig,
        completion: @escaping (Result<SSAISession, SSAIError>) -> Void
    ) {
        state = .initializing
        ssaiConfig = config
        
        let url = config.endpoint.appendingPathComponent("v1/ssai/session")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.addValue("application/json", forHTTPHeaderField: "Content-Type")
        
        let body: [String: Any] = [
            "tenantId": config.tenantId,
            "contentUrl": config.contentUrl.absoluteString,
            "format": config.format.rawValue,
            "capabilities": config.capabilities.map { capabilitiesToDict($0) } ?? [:]
        ]
        
        do {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        } catch {
            let ssaiError = SSAIError(code: "ENCODE_ERROR", message: error.localizedDescription, fatal: true)
            state = .error
            completion(.failure(ssaiError))
            return
        }
        
        fetchWithRetry(request: request, retryCount: 0) { [weak self] result in
            guard let self = self else { return }
            
            switch result {
            case .success(let data):
                do {
                    let sessionResponse = try self.decoder.decode(SSAISession.self, from: data)
                    self.session = sessionResponse
                    self.state = .active
                    self.startBatching()
                    completion(.success(sessionResponse))
                } catch {
                    let ssaiError = SSAIError(code: "DECODE_ERROR", message: error.localizedDescription, fatal: true)
                    self.state = .error
                    completion(.failure(ssaiError))
                }
                
            case .failure(let error):
                self.state = .error
                completion(.failure(error))
            }
        }
    }
    
    /// Create a new SSAI session (async)
    @available(tvOS 13.0, *)
    public func createSession(config: SSAIConfig) async throws -> SSAISession {
        try await withCheckedThrowingContinuation { continuation in
            createSession(config: config) { result in
                switch result {
                case .success(let session):
                    continuation.resume(returning: session)
                case .failure(let error):
                    continuation.resume(throwing: error)
                }
            }
        }
    }
    
    /// Track a SSAI event
    public func trackEvent(_ event: SSAITrackingEvent) {
        guard session != nil else {
            if ssaiConfig?.debug == true {
                print("[SSAI] Cannot track event without active session")
            }
            return
        }
        
        // Deduplicate
        let eventKey = "\(event.type.rawValue):\(event.slotId)"
        if shouldDedupe(event.type) && firedEvents.contains(eventKey) {
            return
        }
        
        if config.enableBatching {
            eventQueue.append(event)
            if eventQueue.count >= config.maxBatchSize {
                flushEvents()
            }
        } else {
            sendEvent(event)
        }
        
        // Mark as fired
        if shouldDedupe(event.type) {
            firedEvents.insert(eventKey)
        }
        
        // Update metrics
        updateMetrics(event.type)
    }
    
    /// Track impression
    public func trackImpression(podId: String, slotId: String, creativeId: String, mediaTime: TimeInterval) {
        trackEvent(SSAITrackingEvent(
            type: .impression,
            podId: podId,
            slotId: slotId,
            creativeId: creativeId,
            mediaTime: mediaTime
        ))
    }
    
    /// Track progress quartile
    public func trackProgress(podId: String, slotId: String, creativeId: String, mediaTime: TimeInterval, progress: Double) {
        let eventType: SSAITrackingEventType
        
        switch progress {
        case 100...:
            eventType = .complete
        case 75..<100:
            eventType = .thirdQuartile
        case 50..<75:
            eventType = .midpoint
        case 25..<50:
            eventType = .firstQuartile
        case 0..<25:
            eventType = .start
        default:
            return
        }
        
        trackEvent(SSAITrackingEvent(
            type: eventType,
            podId: podId,
            slotId: slotId,
            creativeId: creativeId,
            mediaTime: mediaTime,
            metadata: ["progress": String(format: "%.1f", progress)]
        ))
    }
    
    /// Track error
    public func trackError(podId: String, slotId: String, creativeId: String, errorCode: String, errorMessage: String) {
        trackEvent(SSAITrackingEvent(
            type: .error,
            podId: podId,
            slotId: slotId,
            creativeId: creativeId,
            mediaTime: 0,
            metadata: ["errorCode": errorCode, "errorMessage": errorMessage]
        ))
    }
    
    /// Track skip
    public func trackSkip(podId: String, slotId: String, creativeId: String, mediaTime: TimeInterval) {
        trackEvent(SSAITrackingEvent(
            type: .skip,
            podId: podId,
            slotId: slotId,
            creativeId: creativeId,
            mediaTime: mediaTime
        ))
    }
    
    /// Flush pending events
    public func flushEvents() {
        guard !eventQueue.isEmpty else { return }
        
        let events = Array(eventQueue.prefix(config.maxBatchSize))
        eventQueue.removeFirst(min(config.maxBatchSize, eventQueue.count))
        
        sendEventBatch(events)
    }
    
    /// End the session
    public func endSession(completion: ((Bool) -> Void)? = nil) {
        // Flush remaining events
        flushEvents()
        stopBatching()
        
        // Notify server of session end
        guard let session = session, let ssaiConfig = ssaiConfig else {
            completion?(true)
            return
        }
        
        let url = ssaiConfig.endpoint
            .appendingPathComponent("v1/ssai/session")
            .appendingPathComponent(session.sessionId)
            .appendingPathComponent("end")
        
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        
        urlSession.dataTask(with: request) { [weak self] _, _, _ in
            self?.session = nil
            self?.state = .idle
            self?.firedEvents.removeAll()
            self?.resetMetrics()
            completion?(true)
        }.resume()
    }
    
    /// Check if session is valid
    public var isSessionValid: Bool {
        guard let session = session else { return false }
        return Date() < session.expiresAt
    }
    
    // MARK: - Private Methods
    
    private func fetchWithRetry(
        request: URLRequest,
        retryCount: Int,
        completion: @escaping (Result<Data, SSAIError>) -> Void
    ) {
        urlSession.dataTask(with: request) { [weak self] data, response, error in
            guard let self = self else { return }
            
            if let error = error {
                if retryCount < self.config.maxRetries {
                    let delay = self.config.retryBackoff * pow(2.0, Double(retryCount))
                    DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                        self.fetchWithRetry(request: request, retryCount: retryCount + 1, completion: completion)
                    }
                } else {
                    let ssaiError = SSAIError(code: "NETWORK_ERROR", message: error.localizedDescription, fatal: true)
                    DispatchQueue.main.async { completion(.failure(ssaiError)) }
                }
                return
            }
            
            guard let httpResponse = response as? HTTPURLResponse else {
                let ssaiError = SSAIError(code: "INVALID_RESPONSE", message: "No HTTP response", fatal: true)
                DispatchQueue.main.async { completion(.failure(ssaiError)) }
                return
            }
            
            guard (200...299).contains(httpResponse.statusCode) else {
                let ssaiError = SSAIError(code: "HTTP_\(httpResponse.statusCode)", message: "HTTP error", fatal: true)
                DispatchQueue.main.async { completion(.failure(ssaiError)) }
                return
            }
            
            guard let data = data else {
                let ssaiError = SSAIError(code: "NO_DATA", message: "No response data", fatal: true)
                DispatchQueue.main.async { completion(.failure(ssaiError)) }
                return
            }
            
            DispatchQueue.main.async { completion(.success(data)) }
        }.resume()
    }
    
    private func startBatching() {
        guard config.enableBatching else { return }
        
        batchTimer = Timer.scheduledTimer(withTimeInterval: config.batchInterval, repeats: true) { [weak self] _ in
            self?.flushEvents()
        }
    }
    
    private func stopBatching() {
        batchTimer?.invalidate()
        batchTimer = nil
    }
    
    private func sendEvent(_ event: SSAITrackingEvent) {
        guard let session = session else { return }
        
        let url = session.trackingUrl.appendingPathComponent("event")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.addValue("application/json", forHTTPHeaderField: "Content-Type")
        request.addValue(session.sessionToken, forHTTPHeaderField: "X-Session-Token")
        
        do {
            request.httpBody = try encoder.encode(event)
        } catch {
            return
        }
        
        urlSession.dataTask(with: request).resume()
    }
    
    private func sendEventBatch(_ events: [SSAITrackingEvent]) {
        guard let session = session, !events.isEmpty else { return }
        
        let url = session.trackingUrl.appendingPathComponent("events")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.addValue("application/json", forHTTPHeaderField: "Content-Type")
        request.addValue(session.sessionToken, forHTTPHeaderField: "X-Session-Token")
        
        do {
            let body = ["events": events]
            request.httpBody = try encoder.encode(body)
        } catch {
            return
        }
        
        urlSession.dataTask(with: request).resume()
    }
    
    private func shouldDedupe(_ eventType: SSAITrackingEventType) -> Bool {
        [.impression, .start, .firstQuartile, .midpoint, .thirdQuartile, .complete].contains(eventType)
    }
    
    private func updateMetrics(_ eventType: SSAITrackingEventType) {
        switch eventType {
        case .impression:
            metrics.adCount += 1
        case .complete:
            metrics.completedAds += 1
        case .skip:
            metrics.skippedAds += 1
        case .error:
            metrics.erroredAds += 1
        default:
            break
        }
    }
    
    private func resetMetrics() {
        metrics = SSAIMetrics()
    }
    
    private func capabilitiesToDict(_ capabilities: SSAIDeviceCapabilities) -> [String: Any] {
        var dict: [String: Any] = [
            "supportsHdr": capabilities.supportsHdr,
            "supportsDolby": capabilities.supportsDolby
        ]
        
        if let resolution = capabilities.maxResolution {
            dict["maxResolution"] = ["width": resolution.width, "height": resolution.height]
        }
        
        if let codecs = capabilities.codecs {
            dict["codecs"] = codecs
        }
        
        if let bitrate = capabilities.maxBitrate {
            dict["maxBitrate"] = bitrate
        }
        
        return dict
    }
}
