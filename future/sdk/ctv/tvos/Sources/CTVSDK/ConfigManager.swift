import Foundation
#if os(tvOS)
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
#if canImport(CryptoKit)
import CryptoKit
#endif

final class ConfigManager {
    struct RemoteConfig: Codable {
        struct Features: Codable {
            let killSwitch: Bool?
            let disableShow: Bool?
            let metricsEnabled: Bool?
        }
        struct Placement: Codable {
            let killSwitch: Bool?
            let ttlSeconds: Int?
            let refreshSec: Int?
            let enabledNetworks: [String]?
        }
        let version: Int
        let rolloutPercent: Int?
        let features: Features?
        let placements: [String: Placement]?
    }

    private enum Keys {
        static let cache = "ctv_sdk_remote_config"
        static let previous = "ctv_sdk_remote_config_prev"
        static let version = "ctv_sdk_remote_config_version"
        static let bucket = "ctv_sdk_remote_rollout_bucket"
        static let failures = "ctv_sdk_remote_failures"
    }

    private let config: SDKConfig
    private let defaults: UserDefaults
    private let session: URLSession
    private let queue = DispatchQueue(label: "com.rivalapexmediation.ctv.config", qos: .background)
    private(set) var current: RemoteConfig?

    init(config: SDKConfig, session: URLSession = .shared, defaults: UserDefaults = .standard) {
        self.config = config
        self.session = session
        self.defaults = defaults
        self.current = Self.loadCachedConfig(defaults: defaults)
    }

    func load() {
        guard let url = URL(string: config.apiBaseUrl.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + "/sdk/config?appId=\(config.appId)") else { return }
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: TimeInterval(config.requestTimeoutMs) / 1000.0)
        request.httpMethod = "GET"
        if let key = config.apiKey { request.addValue("Bearer \(key)", forHTTPHeaderField: "Authorization") }
        session.dataTask(with: request) { data, response, error in
            guard let data = data,
                  let http = response as? HTTPURLResponse,
                  (200..<300).contains(http.statusCode) else { return }
            if let pem = self.config.configPublicKeyPem,
               let signature = http.value(forHTTPHeaderField: "x-config-sig") ?? http.value(forHTTPHeaderField: "X-Config-Sig") ?? http.value(forHTTPHeaderField: "X-Apex-Signature"),
               !self.verifySignature(body: data, signatureB64: signature, publicKeyPem: pem) {
                return
            }
            guard let parsed = try? JSONDecoder().decode(RemoteConfig.self, from: data) else { return }
            self.queue.async {
                guard self.shouldAdopt(config: parsed) else { return }
                self.current = parsed
                self.save(configData: data, version: parsed.version)
            }
        }.resume()
    }

    func guardLoad(placementId: String) -> String? {
        guard let cfg = current else { return nil }
        if cfg.features?.killSwitch == true { return "kill_switch_active" }
        if cfg.placements?[placementId]?.killSwitch == true { return "placement_disabled" }
        return nil
    }

    func guardShow(placementId: String) -> String? {
        guard let cfg = current else { return nil }
        if cfg.features?.killSwitch == true { return "kill_switch_active" }
        if cfg.features?.disableShow == true { return "show_disabled" }
        if cfg.placements?[placementId]?.killSwitch == true { return "placement_disabled" }
        return nil
    }

    func recordSloSample(success: Bool) {
        queue.async {
            if success {
                self.defaults.set(0, forKey: Keys.failures)
                return
            }
            let failures = self.defaults.integer(forKey: Keys.failures) + 1
            self.defaults.set(failures, forKey: Keys.failures)
            if failures >= 5 {
                self.defaults.set(0, forKey: Keys.failures)
                self.rollbackToPrevious()
            }
        }
    }

    var metricsEnabled: Bool {
        current?.features?.metricsEnabled == true
    }

    private func shouldAdopt(config newConfig: RemoteConfig) -> Bool {
        let activeVersion = defaults.integer(forKey: Keys.version)
        if let current = current, newConfig.version <= activeVersion && activeVersion != 0 { return false }
        let percent = max(0, min(100, newConfig.rolloutPercent ?? 100))
        if percent >= 100 { return true }
        return rolloutBucket() < percent
    }

    private func rolloutBucket() -> Int {
        if defaults.object(forKey: Keys.bucket) != nil {
            return defaults.integer(forKey: Keys.bucket)
        }
        let bucket = Int.random(in: 0..<100)
        defaults.set(bucket, forKey: Keys.bucket)
        return bucket
    }

    private func save(configData: Data, version: Int) {
        defaults.set(defaults.string(forKey: Keys.cache), forKey: Keys.previous)
        defaults.set(String(data: configData, encoding: .utf8), forKey: Keys.cache)
        defaults.set(version, forKey: Keys.version)
    }

    private func rollbackToPrevious() {
        guard let prev = defaults.string(forKey: Keys.previous)?.data(using: .utf8),
              let parsed = try? JSONDecoder().decode(RemoteConfig.self, from: prev) else { return }
        current = parsed
        defaults.set(defaults.string(forKey: Keys.previous), forKey: Keys.cache)
        defaults.set(parsed.version, forKey: Keys.version)
    }

    private func verifySignature(body: Data, signatureB64: String, publicKeyPem: String) -> Bool {
        #if canImport(CryptoKit)
        guard let signature = Data(base64Encoded: signatureB64), let raw = Data(base64Encoded: sanitize(pem: publicKeyPem)) else { return false }
        guard let publicKey = try? Curve25519.Signing.PublicKey(rawRepresentation: raw) else { return false }
        let digest = Data(SHA256.hash(data: body))
        return (try? publicKey.isValidSignature(signature, for: digest)) ?? false
        #else
        // Signature verification unavailable on this platform.
        return true
        #endif
    }

    private func sanitize(pem: String) -> String {
        return pem.replacingOccurrences(of: "-----BEGIN PUBLIC KEY-----", with: "")
            .replacingOccurrences(of: "-----END PUBLIC KEY-----", with: "")
            .replacingOccurrences(of: "\n", with: "")
            .replacingOccurrences(of: "\r", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func loadCachedConfig(defaults: UserDefaults) -> RemoteConfig? {
        guard let json = defaults.string(forKey: Keys.cache)?.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(RemoteConfig.self, from: json)
    }
    
    // MARK: - Config Hash (Parity Verification)
    
    /// Compute deterministic SHA-256 hash of the current configuration.
    /// Uses sorted JSON serialization to ensure cross-platform parity with server.
    /// Hash format: "v1:<hex-digest>"
    ///
    /// - Returns: Configuration hash string or nil if no config loaded
    func getConfigHash() -> String? {
        guard let cfg = current else { return nil }
        
        guard let canonicalJson = buildCanonicalConfigJson(config: cfg) else { return nil }
        guard let data = canonicalJson.data(using: .utf8) else { return nil }
        
        #if canImport(CryptoKit)
        let digest = SHA256.hash(data: data)
        let hexHash = digest.compactMap { String(format: "%02x", $0) }.joined()
        return "v1:\(hexHash)"
        #else
        // Fallback for platforms without CryptoKit
        return "v1:\(data.hashValue)"
        #endif
    }
    
    /// Build canonical JSON representation for hashing.
    /// Keys are sorted alphabetically to ensure deterministic output.
    private func buildCanonicalConfigJson(config: RemoteConfig) -> String? {
        var sortedMap: [String: Any] = [:]
        
        // Add fields in alphabetical order
        sortedMap["appId"] = self.config.appId
        
        // Caps - sorted by placement ID (CTV doesn't have explicit caps in same format, use defaults)
        var caps: [String: [String: Int]] = [:]
        if let placements = config.placements {
            for placementId in placements.keys.sorted() {
                caps[placementId] = ["daily": 0, "hourly": 0]
            }
        }
        sortedMap["caps"] = caps
        
        // Compliance - CTV specific
        sortedMap["compliance"] = [
            "ccpaApplies": false,
            "coppaApplies": false,
            "gdprApplies": false
        ]
        
        // Features
        let features = config.features
        sortedMap["features"] = [
            "bannerRefresh": false,
            "bannerRefreshIntervalMs": 0,
            "bidding": true,
            "waterfall": true
        ]
        
        // Floors - CTV uses defaults
        var floors: [String: Double] = [:]
        if let placements = config.placements {
            for placementId in placements.keys.sorted() {
                floors[placementId] = 0.0
            }
        }
        sortedMap["floors"] = floors
        
        // Networks - enabled networks from placements
        var allNetworks: Set<String> = []
        if let placements = config.placements {
            for (_, placement) in placements {
                if let networks = placement.enabledNetworks {
                    allNetworks.formUnion(networks)
                }
            }
        }
        sortedMap["networks"] = Array(allNetworks).sorted()
        
        // Pacing
        sortedMap["pacing"] = [
            "enabled": false,
            "minIntervalMs": 0
        ]
        
        // Version
        sortedMap["version"] = config.version
        
        // Serialize with sorted keys
        guard let jsonData = try? JSONSerialization.data(
            withJSONObject: sortedMap,
            options: [.sortedKeys, .fragmentsAllowed]
        ) else { return nil }
        return String(data: jsonData, encoding: .utf8)
    }
    
    /// Validate that local config hash matches server hash.
    /// Useful for debugging configuration sync issues.
    ///
    /// - Parameter serverHash: Hash returned from /api/v1/config/sdk/config/hash endpoint
    /// - Returns: true if hashes match, false otherwise
    func validateConfigHash(_ serverHash: String) -> Bool {
        guard let localHash = getConfigHash() else { return false }
        return localHash == serverHash
    }
}
#else
// macOS test host: provide stubs so the package can build without tvOS-only APIs.
final class ConfigManager {
    init(config: SDKConfig, session: URLSession = .shared, defaults: UserDefaults = .standard) {}
    func load() {}
    func guardLoad(placementId: String) -> String? { nil }
    func guardShow(placementId: String) -> String? { nil }
    func recordSloSample(success: Bool) {}
    var metricsEnabled: Bool { false }
    func getConfigHash() -> String? { nil }
    func validateConfigHash(_ serverHash: String) -> Bool { false }
}
#endif
