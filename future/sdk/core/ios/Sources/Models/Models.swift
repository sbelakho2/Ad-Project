import Foundation

// MARK: - Ad Models

/// Ad creative types
public enum AdType: String, Codable, Sendable {
    case banner = "banner"
    case interstitial = "interstitial"
    case rewarded = "rewarded"
    case rewardedInterstitial = "rewarded_interstitial"
    case native = "native"
    case appOpen = "app_open"
}

/// Creative content
public enum Creative: Codable, Sendable {
    case banner(imageURL: String, clickURL: String, width: Int, height: Int)
    case video(videoURL: String, clickURL: String, duration: Int)
    case native(title: String, description: String, iconURL: String, imageURL: String, clickURL: String, ctaText: String)
    
    private enum CodingKeys: String, CodingKey {
        case type
        case imageURL
        case clickURL
        case width
        case height
        case videoURL
        case duration
        case title
        case description
        case iconURL
        case ctaText
    }
    
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        
        switch type {
        case "banner":
            let imageURL = try container.decode(String.self, forKey: .imageURL)
            let clickURL = try container.decode(String.self, forKey: .clickURL)
            let width = try container.decode(Int.self, forKey: .width)
            let height = try container.decode(Int.self, forKey: .height)
            self = .banner(imageURL: imageURL, clickURL: clickURL, width: width, height: height)
            
        case "video":
            let videoURL = try container.decode(String.self, forKey: .videoURL)
            let clickURL = try container.decode(String.self, forKey: .clickURL)
            let duration = try container.decode(Int.self, forKey: .duration)
            self = .video(videoURL: videoURL, clickURL: clickURL, duration: duration)
            
        case "native":
            let title = try container.decode(String.self, forKey: .title)
            let description = try container.decode(String.self, forKey: .description)
            let iconURL = try container.decode(String.self, forKey: .iconURL)
            let imageURL = try container.decode(String.self, forKey: .imageURL)
            let clickURL = try container.decode(String.self, forKey: .clickURL)
            let ctaText = try container.decode(String.self, forKey: .ctaText)
            self = .native(title: title, description: description, iconURL: iconURL, imageURL: imageURL, clickURL: clickURL, ctaText: ctaText)
            
        default:
            throw DecodingError.dataCorruptedError(forKey: .type, in: container, debugDescription: "Unknown creative type")
        }
    }
    
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        
        switch self {
        case .banner(let imageURL, let clickURL, let width, let height):
            try container.encode("banner", forKey: .type)
            try container.encode(imageURL, forKey: .imageURL)
            try container.encode(clickURL, forKey: .clickURL)
            try container.encode(width, forKey: .width)
            try container.encode(height, forKey: .height)
            
        case .video(let videoURL, let clickURL, let duration):
            try container.encode("video", forKey: .type)
            try container.encode(videoURL, forKey: .videoURL)
            try container.encode(clickURL, forKey: .clickURL)
            try container.encode(duration, forKey: .duration)
            
        case .native(let title, let description, let iconURL, let imageURL, let clickURL, let ctaText):
            try container.encode("native", forKey: .type)
            try container.encode(title, forKey: .title)
            try container.encode(description, forKey: .description)
            try container.encode(iconURL, forKey: .iconURL)
            try container.encode(imageURL, forKey: .imageURL)
            try container.encode(clickURL, forKey: .clickURL)
            try container.encode(ctaText, forKey: .ctaText)
        }
    }
}

/// Ad instance
public struct Ad: Codable, Sendable {
    public let adId: String
    public let placement: String
    public let adType: AdType
    public let creative: Creative
    public let networkName: String
    public let cpm: Double
    public let expiresAt: Date
    public let metadata: [String: String]
    
    public init(adId: String, placement: String, adType: AdType, creative: Creative, networkName: String, cpm: Double, expiresAt: Date, metadata: [String: String] = [:]) {
        self.adId = adId
        self.placement = placement
        self.adType = adType
        self.creative = creative
        self.networkName = networkName
        self.cpm = cpm
        self.expiresAt = expiresAt
        self.metadata = metadata
    }
}

// MARK: - Configuration Models

/// Placement configuration
public struct PlacementConfig: Codable, Sendable {
    public let placementId: String
    public let adType: AdType
    public let adapterPriority: [String]
    public let timeoutMs: Int
    public let floorCPM: Double
    public let refreshInterval: Int?

    private enum CodingKeys: String, CodingKey {
        case placementId
        case adType
        case adapterPriority
        case timeoutMs
        case floorCPM
        case refreshInterval
    }

    private enum AlternateFloorKeys: String, CodingKey {
        case floorCpm
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        placementId = try container.decode(String.self, forKey: .placementId)
        adType = try container.decode(AdType.self, forKey: .adType)
        adapterPriority = try container.decode([String].self, forKey: .adapterPriority)
        timeoutMs = try container.decode(Int.self, forKey: .timeoutMs)
        refreshInterval = try container.decodeIfPresent(Int.self, forKey: .refreshInterval)

        if let camelCaseValue = try container.decodeIfPresent(Double.self, forKey: .floorCPM) {
            floorCPM = camelCaseValue
        } else {
            let alternateContainer = try decoder.container(keyedBy: AlternateFloorKeys.self)
            // Support configs emitted with snake_case floor keys to match other SDKs
            if let snakeCaseValue = try alternateContainer.decodeIfPresent(Double.self, forKey: .floorCpm) {
                floorCPM = snakeCaseValue
            } else {
                throw DecodingError.keyNotFound(
                    CodingKeys.floorCPM,
                    DecodingError.Context(
                        codingPath: decoder.codingPath,
                        debugDescription: "Config payload missing floor CPM value"
                    )
                )
            }
        }
    }
    
    public init(placementId: String, adType: AdType, adapterPriority: [String], timeoutMs: Int, floorCPM: Double, refreshInterval: Int? = nil) {
        self.placementId = placementId
        self.adType = adType
        self.adapterPriority = adapterPriority
        self.timeoutMs = timeoutMs
        self.floorCPM = floorCPM
        self.refreshInterval = refreshInterval
    }
}

/// Remote configuration
public struct SDKRemoteConfig: Codable, Sendable {
    public let version: Int
    public let placements: [PlacementConfig]
    public let adapters: [String: AdapterConfig]
    public let killswitches: [String]
    public let telemetryEnabled: Bool
    public let enableOmSdk: Bool?
    public let ctvOmSdk: Bool?
    public let signature: String?
    /// Envelope identifier; required for production signature verification.
    public let configId: String?
    /// Envelope timestamp; required for production signature verification.
    public let timestamp: Int64?
    
    public init(
        version: Int,
        placements: [PlacementConfig],
        adapters: [String: AdapterConfig],
        killswitches: [String],
        telemetryEnabled: Bool,
        enableOmSdk: Bool? = nil,
        ctvOmSdk: Bool? = nil,
        signature: String? = nil,
        configId: String? = nil,
        timestamp: Int64? = nil
    ) {
        self.version = version
        self.placements = placements
        self.adapters = adapters
        self.killswitches = killswitches
        self.telemetryEnabled = telemetryEnabled
        self.enableOmSdk = enableOmSdk
        self.ctvOmSdk = ctvOmSdk
        self.signature = signature
        self.configId = configId
        self.timestamp = timestamp
    }
}

/// Adapter configuration
public struct AdapterConfig: Codable, Sendable {
    public let enabled: Bool
    public let settings: [String: AnyCodable]
    
    public init(enabled: Bool, settings: [String: AnyCodable]) {
        self.enabled = enabled
        self.settings = settings
    }
}

/// Type-erased codable wrapper
public struct AnyCodable: Codable {
    public let value: Any
    
    public init(_ value: Any) {
        self.value = value
    }
    
    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        
        if let intValue = try? container.decode(Int.self) {
            value = intValue
        } else if let doubleValue = try? container.decode(Double.self) {
            value = doubleValue
        } else if let stringValue = try? container.decode(String.self) {
            value = stringValue
        } else if let boolValue = try? container.decode(Bool.self) {
            value = boolValue
        } else if let arrayValue = try? container.decode([AnyCodable].self) {
            value = arrayValue.map { $0.value }
        } else if let dictValue = try? container.decode([String: AnyCodable].self) {
            value = dictValue.mapValues { $0.value }
        } else {
            throw DecodingError.dataCorruptedError(in: container, debugDescription: "Unsupported type")
        }
    }
    
    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        
        switch value {
        case let intValue as Int:
            try container.encode(intValue)
        case let doubleValue as Double:
            try container.encode(doubleValue)
        case let stringValue as String:
            try container.encode(stringValue)
        case let boolValue as Bool:
            try container.encode(boolValue)
        case let arrayValue as [Any]:
            try container.encode(arrayValue.map { AnyCodable($0) })
        case let dictValue as [String: Any]:
            try container.encode(dictValue.mapValues { AnyCodable($0) })
        default:
            throw EncodingError.invalidValue(value, EncodingError.Context(codingPath: [], debugDescription: "Unsupported type"))
        }
    }
}

extension AnyCodable: @unchecked Sendable {}

// MARK: - Telemetry Models

/// Event types
public enum EventType: String, Codable, Sendable {
    case sdkInit = "sdk_init"
    case adLoaded = "ad_loaded"
    case adFailed = "ad_failed"
    case adClicked = "ad_clicked"
    case adImpression = "ad_impression"
    case timeout = "timeout"
    case anrDetected = "anr_detected"
    case omSdkStatus = "omsdk_status"
}

/// Telemetry event
public struct TelemetryEvent: Codable, Sendable {
    public let eventType: EventType
    public let timestamp: Date
    public let placement: String?
    public let adType: AdType?
    public let networkName: String?
    public let latency: Int?
    public let errorCode: String?
    public let errorMessage: String?
    public let metadata: [String: String]?
    
    public init(eventType: EventType, timestamp: Date = Date(), placement: String? = nil, adType: AdType? = nil, networkName: String? = nil, latency: Int? = nil, errorCode: String? = nil, errorMessage: String? = nil, metadata: [String: String]? = nil) {
        self.eventType = eventType
        self.timestamp = timestamp
        self.placement = placement
        self.adType = adType
        self.networkName = networkName
        self.latency = latency
        self.errorCode = errorCode
        self.errorMessage = errorMessage
        self.metadata = metadata
    }
}

// MARK: - SDK Configuration

/// SDK configuration
public struct SDKConfig: Codable, Equatable, Sendable {
    public let appId: String
    public let configEndpoint: String
    public let auctionEndpoint: String
    public let telemetryEnabled: Bool
    public let logLevel: LogLevel
    public let testMode: Bool
    public let configSignaturePublicKey: String?
    public let enableOmSdk: Bool

    public init(
        appId: String,
        configEndpoint: String,
        auctionEndpoint: String,
        telemetryEnabled: Bool = true,
        logLevel: LogLevel = .info,
        testMode: Bool = false,
        configSignaturePublicKey: String? = nil,
        enableOmSdk: Bool = false
    ) {
        self.appId = appId
        self.configEndpoint = configEndpoint
        self.auctionEndpoint = auctionEndpoint
        self.telemetryEnabled = telemetryEnabled
        self.logLevel = logLevel
        self.testMode = testMode
        self.configSignaturePublicKey = configSignaturePublicKey
        self.enableOmSdk = enableOmSdk
    }

    public func withAppId(_ newAppId: String) -> SDKConfig {
        SDKConfig(
            appId: newAppId,
            configEndpoint: configEndpoint,
            auctionEndpoint: auctionEndpoint,
            telemetryEnabled: telemetryEnabled,
            logLevel: logLevel,
            testMode: testMode,
            configSignaturePublicKey: configSignaturePublicKey,
            enableOmSdk: enableOmSdk
        )
    }

    public static func `default`(
        appId: String,
        configEndpoint: String = "https://api.apexmediation.ee",
        auctionEndpoint: String = "https://api.apexmediation.ee",
        telemetryEnabled: Bool = true,
        logLevel: LogLevel = .info,
        testMode: Bool = false,
        configSignaturePublicKey: String? = nil,
        enableOmSdk: Bool = false
    ) -> SDKConfig {
        SDKConfig(
            appId: appId,
            configEndpoint: configEndpoint,
            auctionEndpoint: auctionEndpoint,
            telemetryEnabled: telemetryEnabled,
            logLevel: logLevel,
            testMode: testMode,
            configSignaturePublicKey: configSignaturePublicKey,
            enableOmSdk: enableOmSdk
        )
    }
}

/// Log levels
public enum LogLevel: String, Codable, Sendable {
    case verbose = "verbose"
    case debug = "debug"
    case info = "info"
    case warning = "warning"
    case error = "error"
}
