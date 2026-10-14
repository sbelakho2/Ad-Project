import Foundation

/// SSAI Session Client for iOS Core SDK
///
/// Manages SSAI session lifecycle including creation, tracking, and cleanup.
/// Thread-safe implementation using locks for state management.
public final class SSAISessionClient: @unchecked Sendable {
    
    // MARK: - Configuration
    
    public struct Config: Sendable {
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
    private let lock = NSLock()
    
    private var _ssaiConfig: SSAIConfig?
    private var _session: SSAISession?
    private var _state: SSAISessionState = .idle
    private var _eventQueue: [SSAITrackingEvent] = []
    private var _firedEvents: Set<String> = []
    private var _metrics = SSAIMetrics()
    
    private var batchTimer: DispatchSourceTimer?
    private let batchQueue = DispatchQueue(label: "com.rivalapexmediation.ssai.batch")
    
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
    
    deinit {
        batchTimer?.cancel()
    }
    
    // MARK: - Thread-Safe Accessors
    
    /// Get current session state
    public var sessionState: SSAISessionState {
        lock.lock()
        defer { lock.unlock() }
        return _state
    }
    
    /// Get current session
    public var currentSession: SSAISession? {
        lock.lock()
        defer { lock.unlock() }
        return _session
    }
    
    /// Get metrics
    public var currentMetrics: SSAIMetrics {
        lock.lock()
        defer { lock.unlock() }
        return _metrics
    }
    
    private var ssaiConfig: SSAIConfig? {
        lock.lock()
        defer { lock.unlock() }
        return _ssaiConfig
    }
    
    // MARK: - Public Interface
    
    /// Create a new SSAI session
    public func createSession(config: SSAIConfig) async throws -> SSAISession {
        lock.lock()
        _state = .initializing
        _ssaiConfig = config
        lock.unlock()
        
        let url = config.endpoint.appendingPathComponent("v1/ssai/session")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.addValue("application/json", forHTTPHeaderField: "Content-Type")
        request.addValue("RivalApexMediation-iOS/1.0.0", forHTTPHeaderField: "User-Agent")
        
        var body: [String: Any] = [
            "tenantId": config.tenantId,
            "contentUrl": config.contentUrl.absoluteString,
            "format": config.format.rawValue
        ]
        
        if let capabilities = config.capabilities {
            body["capabilities"] = capabilitiesToDict(capabilities)
        }
        
        if let consent = config.consent {
            body["consent"] = consentToDict(consent)
        }
        
        do {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        } catch {
            lock.lock()
            _state = .error
            lock.unlock()
            throw SSAIError(code: "ENCODE_ERROR", message: error.localizedDescription, fatal: true)
        }
        
        do {
            let data = try await fetchWithRetry(request: request)
            let sessionResponse = try decoder.decode(SSAISession.self, from: data)
            
            lock.lock()
            _session = sessionResponse
            _state = .active
            lock.unlock()
            
            startBatching()
            return sessionResponse
        } catch let error as SSAIError {
            lock.lock()
            _state = .error
            lock.unlock()
            throw error
        } catch {
            lock.lock()
            _state = .error
            lock.unlock()
            throw SSAIError(code: "SESSION_ERROR", message: error.localizedDescription, fatal: true)
        }
    }
    
    /// End the current SSAI session
    public func endSession() async {
        // Flush remaining events
        await flushEvents()
        
        lock.lock()
        _session = nil
        _state = .completed
        _eventQueue.removeAll()
        _firedEvents.removeAll()
        lock.unlock()
        
        batchTimer?.cancel()
        batchTimer = nil
    }
    
    /// Track a SSAI event
    public func trackEvent(_ event: SSAITrackingEvent) {
        lock.lock()
        guard _session != nil else {
            lock.unlock()
            return
        }
        
        // Deduplicate
        let eventKey = "\(event.type.rawValue):\(event.slotId)"
        if shouldDedupe(event.type) && _firedEvents.contains(eventKey) {
            lock.unlock()
            return
        }
        
        if config.enableBatching {
            _eventQueue.append(event)
            let shouldFlush = _eventQueue.count >= config.maxBatchSize
            lock.unlock()
            
            if shouldFlush {
                Task { await flushEvents() }
            }
        } else {
            lock.unlock()
            Task { await sendEvent(event) }
        }
        
        // Mark as fired
        lock.lock()
        if shouldDedupe(event.type) {
            _firedEvents.insert(eventKey)
        }
        updateMetrics(event.type)
        lock.unlock()
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
    
    /// Track quartile progress
    public func trackProgress(
        podId: String,
        slotId: String,
        creativeId: String,
        progress: Double,
        mediaTime: TimeInterval
    ) {
        let eventType: SSAITrackingEventType
        
        switch progress {
        case 0..<0.01:
            eventType = .start
        case 0.24..<0.26:
            eventType = .firstQuartile
        case 0.49..<0.51:
            eventType = .midpoint
        case 0.74..<0.76:
            eventType = .thirdQuartile
        case 0.99...1.0:
            eventType = .complete
        default:
            return // Don't track intermediate progress
        }
        
        trackEvent(SSAITrackingEvent(
            type: eventType,
            podId: podId,
            slotId: slotId,
            creativeId: creativeId,
            mediaTime: mediaTime
        ))
    }
    
    /// Track error
    public func trackError(podId: String, slotId: String, creativeId: String, mediaTime: TimeInterval, errorMessage: String) {
        trackEvent(SSAITrackingEvent(
            type: .error,
            podId: podId,
            slotId: slotId,
            creativeId: creativeId,
            mediaTime: mediaTime,
            metadata: ["error": errorMessage]
        ))
    }
    
    // MARK: - Private Methods
    
    private func startBatching() {
        guard config.enableBatching else { return }
        
        batchTimer?.cancel()
        
        let timer = DispatchSource.makeTimerSource(queue: batchQueue)
        timer.schedule(deadline: .now() + config.batchInterval, repeating: config.batchInterval)
        timer.setEventHandler { [weak self] in
            Task { await self?.flushEvents() }
        }
        timer.resume()
        batchTimer = timer
    }
    
    private func flushEvents() async {
        lock.lock()
        guard !_eventQueue.isEmpty else {
            lock.unlock()
            return
        }
        
        let events = _eventQueue
        _eventQueue.removeAll()
        lock.unlock()
        
        await sendEvents(events)
    }
    
    private func sendEvent(_ event: SSAITrackingEvent) async {
        await sendEvents([event])
    }
    
    private func sendEvents(_ events: [SSAITrackingEvent]) async {
        guard let session = currentSession, let config = ssaiConfig else { return }
        
        let url = session.trackingUrl.appendingPathComponent("batch")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.addValue("application/json", forHTTPHeaderField: "Content-Type")
        request.addValue("Bearer \(session.sessionToken)", forHTTPHeaderField: "Authorization")
        
        let body: [String: Any] = [
            "sessionId": session.sessionId,
            "events": events.map { eventToDict($0) }
        ]
        
        do {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
            _ = try await urlSession.data(for: request)
        } catch {
            if config.debug {
                print("[SSAI] Failed to send tracking events: \(error)")
            }
        }
    }
    
    private func fetchWithRetry(request: URLRequest, attempt: Int = 0) async throws -> Data {
        do {
            let (data, response) = try await urlSession.data(for: request)
            
            guard let httpResponse = response as? HTTPURLResponse else {
                throw SSAIError(code: "INVALID_RESPONSE", message: "Non-HTTP response received", fatal: true)
            }
            
            guard (200...299).contains(httpResponse.statusCode) else {
                throw SSAIError(
                    code: "HTTP_\(httpResponse.statusCode)",
                    message: "Request failed with status \(httpResponse.statusCode)",
                    fatal: httpResponse.statusCode >= 400 && httpResponse.statusCode < 500
                )
            }
            
            return data
        } catch let error as SSAIError {
            if !error.fatal && attempt < config.maxRetries {
                let delay = config.retryBackoff * pow(2.0, Double(attempt))
                try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
                return try await fetchWithRetry(request: request, attempt: attempt + 1)
            }
            throw error
        } catch {
            if attempt < config.maxRetries {
                let delay = config.retryBackoff * pow(2.0, Double(attempt))
                try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
                return try await fetchWithRetry(request: request, attempt: attempt + 1)
            }
            throw SSAIError(code: "NETWORK_ERROR", message: error.localizedDescription, fatal: false)
        }
    }
    
    private func shouldDedupe(_ eventType: SSAITrackingEventType) -> Bool {
        switch eventType {
        case .impression, .start, .firstQuartile, .midpoint, .thirdQuartile, .complete:
            return true
        default:
            return false
        }
    }
    
    private func updateMetrics(_ eventType: SSAITrackingEventType) {
        // Must be called with lock held
        switch eventType {
        case .impression:
            _metrics.adCount += 1
        case .complete:
            _metrics.completedAds += 1
        case .skip:
            _metrics.skippedAds += 1
        case .error:
            _metrics.erroredAds += 1
        default:
            break
        }
    }
    
    private func capabilitiesToDict(_ capabilities: SSAIDeviceCapabilities) -> [String: Any] {
        var dict: [String: Any] = [
            "supportsHdr": capabilities.supportsHdr,
            "supportsDolby": capabilities.supportsDolby,
            "deviceType": capabilities.deviceType.rawValue
        ]
        
        if let resolution = capabilities.maxResolution {
            dict["maxResolution"] = ["width": resolution.width, "height": resolution.height]
        }
        if let codecs = capabilities.codecs {
            dict["codecs"] = codecs
        }
        if let maxBitrate = capabilities.maxBitrate {
            dict["maxBitrate"] = maxBitrate
        }
        
        return dict
    }
    
    private func consentToDict(_ consent: SSAIConsentData) -> [String: Any] {
        var dict: [String: Any] = [
            "coppa": consent.coppa
        ]
        
        if let gdprApplies = consent.gdprApplies {
            dict["gdprApplies"] = gdprApplies
        }
        if let tcfConsent = consent.tcfConsent {
            dict["tcfConsent"] = tcfConsent
        }
        if let usPrivacy = consent.usPrivacy {
            dict["usPrivacy"] = usPrivacy
        }
        if let gpp = consent.gpp {
            dict["gpp"] = gpp
        }
        
        return dict
    }
    
    private func eventToDict(_ event: SSAITrackingEvent) -> [String: Any] {
        var dict: [String: Any] = [
            "type": event.type.rawValue,
            "podId": event.podId,
            "slotId": event.slotId,
            "creativeId": event.creativeId,
            "mediaTime": event.mediaTime,
            "timestamp": ISO8601DateFormatter().string(from: event.timestamp)
        ]
        
        if let metadata = event.metadata {
            dict["metadata"] = metadata
        }
        
        return dict
    }
}
