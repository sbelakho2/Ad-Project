import Foundation
#if canImport(CryptoKit)
import CryptoKit
#endif

/// SDK Configuration hash computation for cross-platform parity verification.
/// Uses SHA-256 with deterministic JSON serialization.
/// Hash format: "v1:<hex-digest>"
///
/// This implementation matches the Web SDK's configHash.ts algorithm exactly
/// to ensure cross-platform configuration parity.
public struct ConfigHash {
    
    /// Configuration structure matching Web SDK's SDKConfig interface
    public struct SDKHashableConfig: Codable, Equatable {
        public let appId: String
        public let version: Int
        public let floors: [String: Double]
        public let caps: [String: CapConfig]
        public let pacing: PacingConfig
        public let networks: [String]
        public let features: FeaturesConfig
        public let compliance: ComplianceConfig
        
        public struct CapConfig: Codable, Equatable {
            public let daily: Int
            public let hourly: Int
            
            public init(daily: Int, hourly: Int) {
                self.daily = daily
                self.hourly = hourly
            }
        }
        
        public struct PacingConfig: Codable, Equatable {
            public let enabled: Bool
            public let minIntervalMs: Int
            
            public init(enabled: Bool, minIntervalMs: Int) {
                self.enabled = enabled
                self.minIntervalMs = minIntervalMs
            }
        }
        
        public struct FeaturesConfig: Codable, Equatable {
            public let bidding: Bool
            public let waterfall: Bool
            public let bannerRefresh: Bool
            public let bannerRefreshIntervalMs: Int
            
            public init(bidding: Bool, waterfall: Bool, bannerRefresh: Bool, bannerRefreshIntervalMs: Int) {
                self.bidding = bidding
                self.waterfall = waterfall
                self.bannerRefresh = bannerRefresh
                self.bannerRefreshIntervalMs = bannerRefreshIntervalMs
            }
        }
        
        public struct ComplianceConfig: Codable, Equatable {
            public let gdprApplies: Bool
            public let ccpaApplies: Bool
            public let coppaApplies: Bool
            
            public init(gdprApplies: Bool, ccpaApplies: Bool, coppaApplies: Bool) {
                self.gdprApplies = gdprApplies
                self.ccpaApplies = ccpaApplies
                self.coppaApplies = coppaApplies
            }
        }
        
        public init(
            appId: String,
            version: Int,
            floors: [String: Double],
            caps: [String: CapConfig],
            pacing: PacingConfig,
            networks: [String],
            features: FeaturesConfig,
            compliance: ComplianceConfig
        ) {
            self.appId = appId
            self.version = version
            self.floors = floors
            self.caps = caps
            self.pacing = pacing
            self.networks = networks
            self.features = features
            self.compliance = compliance
        }
    }
    
    // MARK: - Public API
    
    /// Compute deterministic SHA-256 hash of SDK configuration.
    /// Uses sorted JSON serialization to ensure cross-platform parity with server.
    /// Hash format: "v1:<hex-digest>"
    ///
    /// - Parameter config: SDK configuration object
    /// - Returns: Configuration hash string
    public static func computeHash(_ config: SDKHashableConfig) -> String {
        let canonicalJson = buildCanonicalConfigJson(config)
        let hash = sha256(canonicalJson)
        return "v1:\(hash)"
    }
    
    /// Validate that local config hash matches server hash.
    /// Useful for debugging configuration sync issues.
    ///
    /// - Parameters:
    ///   - config: Local SDK configuration
    ///   - serverHash: Hash returned from server
    /// - Returns: true if hashes match, false otherwise
    public static func validateHash(_ config: SDKHashableConfig, serverHash: String) -> Bool {
        let localHash = computeHash(config)
        return localHash == serverHash
    }
    
    /// Create a default config from SDKConfig for hash computation
    /// when server config is not available.
    public static func createDefaultConfig(appId: String) -> SDKHashableConfig {
        return SDKHashableConfig(
            appId: appId,
            version: 1,
            floors: [:],
            caps: [:],
            pacing: SDKHashableConfig.PacingConfig(enabled: false, minIntervalMs: 0),
            networks: [],
            features: SDKHashableConfig.FeaturesConfig(
                bidding: true,
                waterfall: true,
                bannerRefresh: false,
                bannerRefreshIntervalMs: 0
            ),
            compliance: SDKHashableConfig.ComplianceConfig(
                gdprApplies: false,
                ccpaApplies: false,
                coppaApplies: false
            )
        )
    }
    
    // MARK: - Private Implementation
    
    /// Build canonical JSON string from config with sorted keys.
    /// This ensures deterministic output for hash computation.
    /// Matches Web SDK's buildCanonicalConfigJson() exactly.
    private static func buildCanonicalConfigJson(_ config: SDKHashableConfig) -> String {
        // Build sorted dictionary manually to match Web SDK's output exactly
        var jsonParts: [String] = []
        
        // appId
        jsonParts.append("\"appId\":\"\(escapeJson(config.appId))\"")
        
        // caps - sorted by placement ID
        let sortedCapKeys = config.caps.keys.sorted()
        var capsJson = "{"
        for (index, key) in sortedCapKeys.enumerated() {
            if let cap = config.caps[key] {
                capsJson += "\"\(escapeJson(key))\":{\"daily\":\(cap.daily),\"hourly\":\(cap.hourly)}"
                if index < sortedCapKeys.count - 1 {
                    capsJson += ","
                }
            }
        }
        capsJson += "}"
        jsonParts.append("\"caps\":\(capsJson)")
        
        // compliance - alphabetically sorted keys
        let complianceJson = "{\"ccpaApplies\":\(config.compliance.ccpaApplies),\"coppaApplies\":\(config.compliance.coppaApplies),\"gdprApplies\":\(config.compliance.gdprApplies)}"
        jsonParts.append("\"compliance\":\(complianceJson)")
        
        // features - alphabetically sorted keys
        let featuresJson = "{\"bannerRefresh\":\(config.features.bannerRefresh),\"bannerRefreshIntervalMs\":\(config.features.bannerRefreshIntervalMs),\"bidding\":\(config.features.bidding),\"waterfall\":\(config.features.waterfall)}"
        jsonParts.append("\"features\":\(featuresJson)")
        
        // floors - sorted by placement ID
        let sortedFloorKeys = config.floors.keys.sorted()
        var floorsJson = "{"
        for (index, key) in sortedFloorKeys.enumerated() {
            if let floor = config.floors[key] {
                floorsJson += "\"\(escapeJson(key))\":\(floor)"
                if index < sortedFloorKeys.count - 1 {
                    floorsJson += ","
                }
            }
        }
        floorsJson += "}"
        jsonParts.append("\"floors\":\(floorsJson)")
        
        // networks - sorted alphabetically
        let sortedNetworks = config.networks.sorted()
        let networksJson = "[" + sortedNetworks.map { "\"\(escapeJson($0))\"" }.joined(separator: ",") + "]"
        jsonParts.append("\"networks\":\(networksJson)")
        
        // pacing
        let pacingJson = "{\"enabled\":\(config.pacing.enabled),\"minIntervalMs\":\(config.pacing.minIntervalMs)}"
        jsonParts.append("\"pacing\":\(pacingJson)")
        
        // version
        jsonParts.append("\"version\":\(config.version)")
        
        return "{" + jsonParts.joined(separator: ",") + "}"
    }
    
    /// Escape JSON string characters
    private static func escapeJson(_ string: String) -> String {
        var result = ""
        for char in string {
            switch char {
            case "\"": result += "\\\""
            case "\\": result += "\\\\"
            case "\n": result += "\\n"
            case "\r": result += "\\r"
            case "\t": result += "\\t"
            default: result.append(char)
            }
        }
        return result
    }
    
    /// Compute SHA-256 hash and return hex string
    private static func sha256(_ message: String) -> String {
        let data = Data(message.utf8)
        
        #if canImport(CryptoKit)
        if #available(iOS 13.0, macOS 10.15, tvOS 13.0, watchOS 6.0, *) {
            let digest = SHA256.hash(data: data)
            return digest.map { String(format: "%02x", $0) }.joined()
        }
        #endif
        
        // Fallback using CommonCrypto
        return sha256CommonCrypto(data)
    }
    
    /// SHA-256 fallback using CommonCrypto
    private static func sha256CommonCrypto(_ data: Data) -> String {
        var hash = [UInt8](repeating: 0, count: 32)
        data.withUnsafeBytes { ptr in
            _ = CC_SHA256(ptr.baseAddress, CC_LONG(data.count), &hash)
        }
        return hash.map { String(format: "%02x", $0) }.joined()
    }
}

// CommonCrypto import for fallback
#if canImport(CommonCrypto)
import CommonCrypto
#endif
