import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
#if canImport(CryptoKit)
import CryptoKit
#endif

/// Configuration manager with remote fetching and caching
///
/// Features:
/// - Remote config fetching with Ed25519 signature verification
/// - UserDefaults caching with TTL
/// - Async/await concurrency
/// - Automatic retry with exponential backoff
public final class ConfigManager: @unchecked Sendable {
    private let config: SDKConfig
    private let urlSession: URLSession
    private let userDefaults: UserDefaults
    private let signatureVerifier: SignatureVerifier?
    private let clock: ClockProtocol
    
    private let cacheKey = "com.rivalapexmediation.config"
    private let cacheVersionKey = "com.rivalapexmediation.config.version"
    private let cacheTimestampKey = "com.rivalapexmediation.config.timestamp"
    private let cacheMonotonicKey = "com.rivalapexmediation.config.timestamp.mono"
    
    private let cacheTTL: TimeInterval = 3600 // 1 hour
    
    private var cachedConfig: SDKRemoteConfig?
    
    /// Initialize config manager
    public init(config: SDKConfig, signatureVerifier: SignatureVerifier?, clock: ClockProtocol = Clock.shared) {
        self.config = config
        self.signatureVerifier = signatureVerifier
        self.clock = clock
        
        let configuration = URLSessionConfiguration.apexDefault(
            requestTimeout: 10,
            resourceTimeout: 30
        )
        self.urlSession = URLSession(configuration: configuration)
        
        self.userDefaults = UserDefaults.standard
    }
    
    /// Load configuration (cache-first)
    public func loadConfig() async throws -> SDKRemoteConfig {
        // Production builds must have a verifier configured to enforce signatures.
        if !config.testMode && signatureVerifier == nil {
            throw ConfigError.missingSignature
        }
        // Check cache first
        if let cached = loadCachedConfig(), !isCacheExpired() {
            cachedConfig = cached
            return cached
        }
        
        // Fetch from remote
        let remoteConfig = try await fetchRemoteConfig()
        
        // Verify signature when verifier available
        if let verifier = signatureVerifier {
            try verifySignature(config: remoteConfig, verifier: verifier)
        }
        
        // Cache the config
        cacheConfig(remoteConfig)
        
        cachedConfig = remoteConfig
        return remoteConfig
    }
    
    /// Get placement config by ID
    public func getPlacement(id: String) -> PlacementConfig? {
        return cachedConfig?.placements.first { $0.placementId == id }
    }

    /// Convenience accessor used by the mediation layer
    public func getPlacementConfig(_ id: String) -> PlacementConfig? {
        getPlacement(id: id)
    }
    
    /// Check if adapter is enabled
    public func isAdapterEnabled(name: String) -> Bool {
        return cachedConfig?.adapters[name]?.enabled ?? false
    }
    
    /// Get adapter configuration
    public func getAdapterConfig(name: String) -> [String: Any]? {
        return cachedConfig?.adapters[name]?.settings.mapValues { $0.value }
    }
    
    /// Check if kill switch is active
    public func isKillSwitchActive(name: String) -> Bool {
        return cachedConfig?.killswitches.contains(name) ?? false
    }
    
    // MARK: - Remote Fetching
    
    /// Fetch configuration from remote endpoint
    private func fetchRemoteConfig() async throws -> SDKRemoteConfig {
        let url = URL(string: "\(config.configEndpoint)/v1/config/\(config.appId)")!
        
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.addValue("RivalApexMediation-iOS/1.0.0", forHTTPHeaderField: "User-Agent")
        request.addValue("application/json", forHTTPHeaderField: "Accept")
        
        let (data, response) = try await urlSession.apexData(for: request)
        
        guard let httpResponse = response as? HTTPURLResponse else {
            throw ConfigError.invalidResponse
        }
        
        guard httpResponse.statusCode == 200 else {
            throw ConfigError.httpError(statusCode: httpResponse.statusCode)
        }
        
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        
        do {
            return try decoder.decode(SDKRemoteConfig.self, from: data)
        } catch {
            throw ConfigError.decodingError(error)
        }
    }
    
    /// Fetch configuration with retry
    public func fetchWithRetry(maxAttempts: Int = 3) async throws -> SDKRemoteConfig {
        var lastError: Error?
        
        for attempt in 0..<maxAttempts {
            do {
                return try await fetchRemoteConfig()
            } catch {
                lastError = error
                
                if attempt < maxAttempts - 1 {
                    // Exponential backoff: 1s, 2s, 4s
                    let delay = pow(2.0, Double(attempt))
                    try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
                }
            }
        }
        
        throw lastError ?? ConfigError.unknownError
    }
    
    // MARK: - Signature Verification
    
    /// Verify Ed25519 signature
    private func verifySignature(config: SDKRemoteConfig, verifier: SignatureVerifier) throws {
        guard let signature = config.signature, !signature.isEmpty else {
            // Allow unsigned payloads when running local test builds with no production key provisioned
            if self.config.testMode && self.config.configSignaturePublicKey == nil {
                return
            }
            throw ConfigError.missingSignature
        }

        let payload = try createSignaturePayload(config: config)

        do {
            try verifier.verifySignature(message: payload, signatureBase64: signature)
        } catch let error as SignatureError {
            throw ConfigError.signatureVerificationFailed(error)
        } catch {
            throw ConfigError.signatureVerificationFailed(error)
        }
    }
    
    /// Canonical message for signature verification, byte-pinned to the
    /// Android `canonicalSigningMessage` output (lexicographic keys). The
    /// signed envelope must carry config_id and timestamp; legacy envelopes
    /// without them fail closed in production.
    func createSignaturePayload(config: SDKRemoteConfig) throws -> String {
        guard let configId = config.configId, let timestamp = config.timestamp else {
            throw ConfigError.missingSignature
        }
        return SignatureVerifier.createCanonicalMessage(
            configId: configId,
            version: config.version,
            timestamp: timestamp
        )
    }
    
    // MARK: - Caching
    
    /// Cache configuration
    private func cacheConfig(_ config: SDKRemoteConfig) {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        
        if let data = try? encoder.encode(config) {
            userDefaults.set(data, forKey: cacheKey)
            userDefaults.set(config.version, forKey: cacheVersionKey)
            userDefaults.set(clock.now().timeIntervalSince1970, forKey: cacheTimestampKey)
            userDefaults.set(clock.monotonicSeconds(), forKey: cacheMonotonicKey)
        }
    }
    
    /// Load cached configuration
    private func loadCachedConfig() -> SDKRemoteConfig? {
        guard let data = userDefaults.data(forKey: cacheKey) else {
            return nil
        }
        
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        
        return try? decoder.decode(SDKRemoteConfig.self, from: data)
    }
    
    /// Check if cache is expired
    private func isCacheExpired() -> Bool {
        let timestamp = userDefaults.double(forKey: cacheTimestampKey)
        let monotonicStamped = userDefaults.double(forKey: cacheMonotonicKey)
        let nowMonotonic = clock.monotonicSeconds()
        
        if timestamp == 0 {
            return true
        }

        let age: TimeInterval
        if monotonicStamped > 0, nowMonotonic >= monotonicStamped {
            age = nowMonotonic - monotonicStamped
        } else {
            age = clock.now().timeIntervalSince1970 - timestamp
        }
        return age > cacheTTL
    }
    
    /// Get cached version
    public func getCachedVersion() -> Int {
        return userDefaults.integer(forKey: cacheVersionKey)
    }
    
    /// Clear cache
    public func clearCache() {
        userDefaults.removeObject(forKey: cacheKey)
        userDefaults.removeObject(forKey: cacheVersionKey)
        userDefaults.removeObject(forKey: cacheTimestampKey)
        userDefaults.removeObject(forKey: cacheMonotonicKey)
        cachedConfig = nil
    }
    
    /// Release resources and clear cached config
    public func shutdown() {
        urlSession.invalidateAndCancel()
        cachedConfig = nil
    }
    
    // MARK: - Config Hash (Parity Verification)
    
    /// Compute deterministic SHA-256 hash of the current configuration.
    /// Uses sorted JSON serialization to ensure cross-platform parity with server.
    /// Hash format: "v1:<hex-digest>"
    ///
    /// - Returns: Configuration hash string or nil if no config loaded
    public func getConfigHash() -> String? {
        guard let config = cachedConfig else { return nil }
        
        do {
            let canonicalJson = try buildCanonicalConfigJson(config: config)
            guard let data = canonicalJson.data(using: .utf8) else { return nil }
            
            #if canImport(CryptoKit)
            let digest = SHA256.hash(data: data)
            let hexHash = digest.compactMap { String(format: "%02x", $0) }.joined()
            return "v1:\(hexHash)"
            #else
            // Fallback for platforms without CryptoKit
            return "v1:\(data.hashValue)"
            #endif
        } catch {
            return nil
        }
    }
    
    /// Build canonical JSON representation for hashing.
    /// Keys are sorted alphabetically to ensure deterministic output.
    private func buildCanonicalConfigJson(config: SDKRemoteConfig) throws -> String {
        var sortedMap: [String: Any] = [:]
        
        // Add fields in alphabetical order
        sortedMap["appId"] = self.config.appId
        
        // Adapters - sorted alphabetically
        var adaptersMap: [String: Any] = [:]
        for adapterName in config.adapters.keys.sorted() {
            if let adapterConfig = config.adapters[adapterName] {
                adaptersMap[adapterName] = [
                    "enabled": adapterConfig.enabled
                ]
            }
        }
        sortedMap["adapters"] = adaptersMap
        
        // Features
        sortedMap["features"] = [
            "enableOmSdk": config.enableOmSdk ?? false,
            "telemetryEnabled": config.telemetryEnabled
        ]
        
        // Killswitches - sorted
        sortedMap["killswitches"] = config.killswitches.sorted()
        
        // Placements - sorted by placement ID
        let sortedPlacements = config.placements.sorted { $0.placementId < $1.placementId }
        var placementsArray: [[String: Any]] = []
        for placement in sortedPlacements {
            var placementDict: [String: Any] = [
                "adType": placement.adType.rawValue,
                "adapterPriority": placement.adapterPriority,
                "floorCPM": placement.floorCPM,
                "placementId": placement.placementId,
                "timeoutMs": placement.timeoutMs
            ]
            if let refreshInterval = placement.refreshInterval {
                placementDict["refreshInterval"] = refreshInterval
            }
            placementsArray.append(placementDict)
        }
        sortedMap["placements"] = placementsArray
        
        // Version
        sortedMap["version"] = config.version
        
        // Serialize with sorted keys
        let jsonData = try JSONSerialization.data(
            withJSONObject: sortedMap,
            options: [.sortedKeys, .fragmentsAllowed]
        )
        return String(data: jsonData, encoding: .utf8) ?? "{}"
    }
    
    /// Validate that local config hash matches server hash.
    /// Useful for debugging configuration sync issues.
    ///
    /// - Parameter serverHash: Hash returned from /api/v1/config/sdk/config/hash endpoint
    /// - Returns: true if hashes match, false otherwise
    public func validateConfigHash(_ serverHash: String) -> Bool {
        guard let localHash = getConfigHash() else { return false }
        return localHash == serverHash
    }
}

// MARK: - Errors

/// Configuration errors
public enum ConfigError: Error, LocalizedError {
    case invalidResponse
    case httpError(statusCode: Int)
    case decodingError(Error)
    case missingSignature
    case invalidSignature
    case signatureVerificationFailed(Error)
    case unknownError
    
    public var errorDescription: String? {
        switch self {
        case .invalidResponse:
            return "Invalid response from server"
        case .httpError(let statusCode):
            return "HTTP error: \(statusCode)"
        case .decodingError(let error):
            return "Failed to decode config: \(error.localizedDescription)"
        case .missingSignature:
            return "Configuration signature missing"
        case .invalidSignature:
            return "Invalid configuration signature"
        case .signatureVerificationFailed(let error):
            return "Signature verification failed: \(error.localizedDescription)"
        case .unknownError:
            return "Unknown error occurred"
        }
    }
}
