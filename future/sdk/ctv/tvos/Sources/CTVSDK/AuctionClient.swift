import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Generate a correlation ID for distributed tracing
private func generateCorrelationId() -> String {
    return UUID().uuidString
}

public struct AuctionWin: Codable, Equatable {
    public let requestId: String
    public let bidId: String
    public let adapter: String
    public let cpm: Double
    public let currency: String
    public let ttlSeconds: Int
    public let creativeUrl: String
    public let tracking: Tracking
    public struct Tracking: Codable, Equatable {
        public let impression: String
        public let click: String
        public let start: String?
        public let firstQuartile: String?
        public let midpoint: String?
        public let thirdQuartile: String?
        public let complete: String?
        public let pause: String?
        public let resume: String?
        public let mute: String?
        public let unmute: String?
        public let close: String?
    }
}

public final class AuctionClient {
    private let config: SDKConfig
    private let session: URLSession

    public init(config: SDKConfig, session: URLSession = URLSession(configuration: .default)) {
        self.config = config
        self.session = session
    }

    public struct Result {
        public let win: AuctionWin?
        public let error: LoadError?
        public var noFill: Bool { error == .noFill }
    }

    public func requestBid(placementId: String,
                           adFormat: String,
                           floorCpm: Double = 0,
                           consent: ConsentData?,
                           completion: @escaping (Result) -> Void) {
        let requestId = UUID().uuidString
        var body: [String: Any] = [
            "requestId": requestId,
            "placementId": placementId,
            "adFormat": adFormat,
            "floorCpm": floorCpm,
            "device": ["platform": "ios", "osVersion": ProcessInfo.processInfo.operatingSystemVersionString, "model": "AppleTV"],
            "app": ["id": config.appId]
        ]
        if let c = consent {
            var m: [String: Any] = [:]
            if let g = c.gdprApplies { m["gdpr"] = g ? 1 : 0 }
            if let t = c.tcfString, !t.isEmpty { m["gdpr_consent"] = t }
            if let u = c.usPrivacy, !u.isEmpty { m["us_privacy"] = u }
            if let cp = c.coppa { m["coppa"] = cp }
            body["consent"] = m
        }
        let envelope = ["body": body]
        guard let data = try? JSONSerialization.data(withJSONObject: envelope) else {
            completion(Result(win: nil, error: .generic("encode_error"))); return
        }
        let url = URL(string: config.apiBaseUrl.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + "/rtb/bid")!
        var req = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: TimeInterval(config.requestTimeoutMs) / 1000.0)
        req.httpMethod = "POST"
        req.addValue("application/json", forHTTPHeaderField: "Content-Type")
        if let key = config.apiKey { req.addValue("Bearer \(key)", forHTTPHeaderField: "Authorization") }
        // Add correlation ID for distributed tracing
        let correlationId = generateCorrelationId()
        req.addValue(correlationId, forHTTPHeaderField: "X-Correlation-ID")
        req.addValue("ctv-tvos-sdk/1.0.0", forHTTPHeaderField: "X-SDK-Version")
        req.httpBody = data
        let startTime = Date()

        let task = session.dataTask(with: req) { d, r, e in
            func finish(_ win: AuctionWin?, _ error: LoadError?) {
                let success = (error == nil) || (error == .noFill)
                ApexMediation.shared.recordSloSample(success: success)
                let elapsed = Date().timeIntervalSince(startTime) * 1000.0
                MetricsRecorder.shared.recordRequest(durationMs: elapsed, error: error)
                completion(Result(win: win, error: error))
            }
            if let err = e as NSError? {
                let loadError: LoadError = err.domain == NSURLErrorDomain && err.code == NSURLErrorTimedOut ? .timeout : .network
                finish(nil, loadError)
                return
            }
            guard let http = r as? HTTPURLResponse else {
                finish(nil, .generic("no_response"))
                return
            }
            if http.statusCode == 204 {
                finish(nil, .noFill)
                return
            }
            guard http.statusCode >= 200 && http.statusCode < 300 else {
                finish(nil, self.mapStatusError(http.statusCode))
                return
            }
            guard let d = d else {
                finish(nil, .generic("empty"))
                return
            }
            do {
                if let obj = try JSONSerialization.jsonObject(with: d) as? [String: Any], let resp = obj["response"] {
                    let respData = try JSONSerialization.data(withJSONObject: resp)
                    let win = try JSONDecoder().decode(AuctionWin.self, from: respData)
                    finish(win, nil)
                    return
                }
                let win = try JSONDecoder().decode(AuctionWin.self, from: d)
                finish(win, nil)
            } catch {
                finish(nil, .generic("parse_error"))
            }
        }
        task.resume()
    }

    private func mapStatusError(_ code: Int) -> LoadError {
        if code == 408 || code == 504 { return .timeout }
        if code == 429 { return .status(code: code, label: "status_429") }
        if code == 401 { return .status(code: code, label: "status_401") }
        if (400..<500).contains(code) { return .status(code: code, label: "status_4xx") }
        if (500..<600).contains(code) { return .status(code: code, label: "status_5xx") }
        return .status(code: code, label: "status_\(code)")
    }
}
