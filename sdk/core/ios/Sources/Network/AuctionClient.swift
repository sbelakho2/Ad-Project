import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
#if canImport(UIKit)
import UIKit
#endif

/// Client for server-to-server auction requests
public final class AuctionClient {
    public struct RetryConfiguration: Sendable {
        public let maxAttempts: Int
        public let initialBackoff: TimeInterval
        public let maxBackoff: TimeInterval
        public let backoffMultiplier: Double

        public static let standard = RetryConfiguration()

        public init(
            maxAttempts: Int = 3,
            initialBackoff: TimeInterval = 0.5,
            maxBackoff: TimeInterval = 5.0,
            backoffMultiplier: Double = 2.0
        ) {
            self.maxAttempts = max(1, maxAttempts)
            self.initialBackoff = max(0.05, initialBackoff)
            self.maxBackoff = max(initialBackoff, maxBackoff)
            self.backoffMultiplier = max(1.0, backoffMultiplier)
        }
    }

    public struct CircuitBreakerConfiguration: Sendable {
        public let failureThreshold: Int
        public let cooldown: TimeInterval

        public static let standard = CircuitBreakerConfiguration()

        public init(failureThreshold: Int = 3, cooldown: TimeInterval = 30.0) {
            self.failureThreshold = max(1, failureThreshold)
            self.cooldown = max(1.0, cooldown)
        }
    }

    private typealias SleepFunction = @Sendable (UInt64) async throws -> Void

    private let baseURL: String
    private let apiKey: String
    private let timeout: TimeInterval
    private let session: URLSession
    private let retryConfiguration: RetryConfiguration
    private let circuitBreakerConfiguration: CircuitBreakerConfiguration
    private let circuitBreakerState = CircuitBreakerState()
    private let sleep: SleepFunction
    private let dateProvider: () -> Date
    private let clock: ClockProtocol

    private static let retryAfterFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
        return formatter
    }()

    public init(
        baseURL: String,
        apiKey: String,
        timeout: TimeInterval = 5.0,
        session: URLSession? = nil,
        retryConfiguration: RetryConfiguration = .standard,
        circuitBreakerConfiguration: CircuitBreakerConfiguration = .standard,
        sleep: (@Sendable (UInt64) async throws -> Void)? = nil,
        dateProvider: @escaping () -> Date = { Date() },
        clock: ClockProtocol = Clock.shared
    ) {
        self.baseURL = baseURL
        self.apiKey = apiKey
        self.timeout = timeout
        self.retryConfiguration = retryConfiguration
        self.circuitBreakerConfiguration = circuitBreakerConfiguration
        if let customSleep = sleep {
            self.sleep = customSleep
        } else {
            self.sleep = { duration in try await Task.sleep(nanoseconds: duration) }
        }
        self.dateProvider = dateProvider
        self.clock = clock

        if let customSession = session {
            self.session = customSession
        } else {
            let config = URLSessionConfiguration.apexDefault(
                requestTimeout: timeout,
                resourceTimeout: timeout * 2
            )
            self.session = URLSession(configuration: config)
        }
    }
    
    /// Request a bid from the auction service
    public func requestBid(placementId: String, adType: String) async throws -> AuctionResponse {
        if let remaining = await circuitBreakerState.remainingCooldown(
            now: clock.monotonicSeconds(),
            cooldown: circuitBreakerConfiguration.cooldown
        ) {
            throw SDKError.circuitBreakerOpen(retryAfter: remaining)
        }

        var backoff = retryConfiguration.initialBackoff

        for attempt in 0..<retryConfiguration.maxAttempts {
            do {
                let response = try await executeRequest(placementId: placementId, adType: adType)
                await circuitBreakerState.reset()
                return response
            } catch let sdkError as SDKError {
                let shouldRetryAttempt = attempt < (retryConfiguration.maxAttempts - 1) && shouldRetry(error: sdkError)
                if shouldRetryAttempt {
                    let waitSeconds = min(backoff, retryConfiguration.maxBackoff)
                    let duration = toNanoseconds(waitSeconds)
                    try await sleep(duration)
                    backoff = min(backoff * retryConfiguration.backoffMultiplier, retryConfiguration.maxBackoff)
                    continue
                }

                await handlePostFailure(for: sdkError)
                throw sdkError
            }
        }

        throw SDKError.internalError(message: "Unknown auction failure")
    }

    private func executeRequest(placementId: String, adType: String) async throws -> AuctionResponse {
        let request = try buildRequest(placementId: placementId, adType: adType)

        let (data, response): (Data, URLResponse)
        do {
            (data, response) = try await session.apexData(for: request)
        } catch let error as URLError {
            if error.code == .timedOut {
                throw SDKError.timeout
            }
            if error.code == .cancelled {
                throw SDKError.networkError(underlying: "navigation_cancelled")
            }
            throw SDKError.networkError(underlying: error.localizedDescription)
        } catch {
            throw SDKError.internalError(message: error.localizedDescription)
        }
        
        guard let httpResponse = response as? HTTPURLResponse else {
            throw SDKError.networkError(underlying: "Invalid response type")
        }
        
        switch httpResponse.statusCode {
        case 200:
            return try parseResponse(data: data)
            
        case 204:
            throw SDKError.noFill
            
        case 400:
            throw SDKError.invalidPlacement("Bad request")
            
        case 429:
            let message = rateLimitMessage(data: data, response: httpResponse)
            throw SDKError.status_429(message: message)
            
        case 500...599:
            let message = String(data: data, encoding: .utf8) ?? "Server error"
            throw SDKError.status_5xx(code: httpResponse.statusCode, message: message)
            
        default:
            throw SDKError.networkError(underlying: "HTTP \(httpResponse.statusCode)")
        }
    }
    
    private func buildRequest(placementId: String, adType: String) throws -> URLRequest {
        guard let url = URL(string: "\(baseURL)/v1/auction") else {
            throw SDKError.internalError(message: "Invalid auction URL")
        }
        
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(apiKey, forHTTPHeaderField: "X-Api-Key")
        request.setValue(userAgent(), forHTTPHeaderField: "User-Agent")
        
        // Correlation ID for distributed tracing
        let correlationId = UUID().uuidString
        request.setValue(correlationId, forHTTPHeaderField: "X-Correlation-ID")
        request.setValue("ios-sdk/1.0.0", forHTTPHeaderField: "X-SDK-Version")
        
        request.timeoutInterval = timeout
        
        let body = buildRequestBody(placementId: placementId, adType: adType, correlationId: correlationId)
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        
        return request
    }
    
    private func buildRequestBody(placementId: String, adType: String, correlationId: String) -> [String: Any] {
        var body: [String: Any] = [
            "placementId": placementId,
            "adType": adType,
            "device": deviceInfo(),
            "app": appInfo(),
            "meta": [
                "correlationId": correlationId,
                "sdk": [
                    "name": "apex-ios-sdk",
                    "version": "1.0.0"
                ]
            ]
        ]
        
        let consentMetadata = MediationSDK.shared.consentMetadata()
        if !consentMetadata.isEmpty {
            body["consent"] = consentMetadata
        }
        
        return body
    }
    
    private func deviceInfo() -> [String: Any] {
        var info: [String: Any] = [:]
        
        #if canImport(UIKit)
        let device = UIDevice.current
        info["platform"] = "iOS"
        info["osVersion"] = device.systemVersion
        info["model"] = device.model
        info["language"] = Locale.current.languageCode ?? "en"
        
        let screen = UIScreen.main
        info["screenWidth"] = Int(screen.bounds.width * screen.scale)
        info["screenHeight"] = Int(screen.bounds.height * screen.scale)
        info["screenScale"] = screen.scale
        
        let trackingManager = TrackingAuthorizationManager.shared
        info["attStatus"] = trackingManager.currentStatus().rawValue
        info["limitAdTracking"] = trackingManager.isLimitAdTrackingEnabled()
        if MediationSDK.shared.canShowPersonalizedAds(), let idfa = trackingManager.advertisingIdentifier() {
            info["idfa"] = idfa
        }
        #endif
        
        return info
    }
    
    private func appInfo() -> [String: Any] {
        let bundle = Bundle.main
        return [
            "bundleId": bundle.bundleIdentifier ?? "unknown",
            "version": bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "1.0",
            "build": bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "1"
        ]
    }
    
    private func userAgent() -> String {
        let version = "1.0.0"
        #if canImport(UIKit)
        let device = UIDevice.current
        let os = "\(device.systemName)/\(device.systemVersion)"
        return "ApexMediation-iOS/\(version) (\(os))"
        #else
        return "ApexMediation-iOS/\(version)"
        #endif
    }
    
    private func parseResponse(data: Data) throws -> AuctionResponse {
        do {
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase
            return try decoder.decode(AuctionResponse.self, from: data)
        } catch {
            throw SDKError.internalError(message: "Failed to parse auction response: \(error.localizedDescription)")
        }
    }

    private func shouldRetry(error: SDKError) -> Bool {
        switch error {
        case .timeout, .status_5xx:
            return true
        default:
            return false
        }
    }

    private func shouldTripCircuit(for error: SDKError) -> Bool {
        if case .status_5xx = error {
            return true
        }
        return false
    }

    private func handlePostFailure(for error: SDKError) async {
        if shouldTripCircuit(for: error) {
            await circuitBreakerState.recordFailure(
                now: clock.monotonicSeconds(),
                threshold: circuitBreakerConfiguration.failureThreshold,
                cooldown: circuitBreakerConfiguration.cooldown
            )
        } else {
            await circuitBreakerState.reset()
        }
    }

    private func rateLimitMessage(data: Data, response: HTTPURLResponse) -> String {
        let fallback = "Rate limit exceeded"
        let payload = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
        let message = payload?.isEmpty == false ? payload! : fallback
        if let retryAfter = retryAfterInterval(from: response) {
            let seconds = Int(ceil(retryAfter))
            return "\(message) (retry after \(seconds)s)"
        }
        return message
    }

    private func retryAfterInterval(from response: HTTPURLResponse) -> TimeInterval? {
        guard let raw = headerValue("Retry-After", in: response) else { return nil }
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if let seconds = TimeInterval(trimmed) {
            return max(0, seconds)
        }

        if let date = AuctionClient.retryAfterFormatter.date(from: trimmed) {
            return max(0, date.timeIntervalSince(dateProvider()))
        }

        return nil
    }

    private func headerValue(_ name: String, in response: HTTPURLResponse) -> String? {
        for (key, value) in response.allHeaderFields {
            if let keyString = key as? String, keyString.caseInsensitiveCompare(name) == .orderedSame {
                if let stringValue = value as? String {
                    return stringValue
                }
                return "\(value)"
            }
        }
        return nil
    }

    private func toNanoseconds(_ seconds: TimeInterval) -> UInt64 {
        let clamped = max(0, seconds)
        return UInt64(clamped * 1_000_000_000)
    }
}

private actor CircuitBreakerState {
    private var consecutiveFailures = 0
    private var openedAtMs: Int64?

    func reset() {
        consecutiveFailures = 0
        openedAtMs = nil
    }

    func recordFailure(now: TimeInterval, threshold: Int, cooldown: TimeInterval) {
        consecutiveFailures += 1
        guard consecutiveFailures >= threshold else { return }
        openedAtMs = Int64(now * 1000)
    }

    func remainingCooldown(now: TimeInterval, cooldown: TimeInterval) -> TimeInterval? {
        guard let openedAtMs else { return nil }
        let elapsedSeconds = ((now * 1000) - Double(openedAtMs)) / 1000
        if elapsedSeconds >= cooldown {
            self.openedAtMs = nil
            consecutiveFailures = 0
            return nil
        }
        return cooldown - elapsedSeconds
    }
}

/// Auction response model
public struct AuctionResponse: Codable {
    public let adId: String
    public let networkName: String
    public let creativeUrl: String?
    public let ecpm: Double?
    public let ttl: Int?
    public let metadata: [String: String]?
    
    enum CodingKeys: String, CodingKey {
        case adId = "ad_id"
        case networkName = "network_name"
        case creativeUrl = "creative_url"
        case ecpm
        case ttl
        case metadata
    }
}
