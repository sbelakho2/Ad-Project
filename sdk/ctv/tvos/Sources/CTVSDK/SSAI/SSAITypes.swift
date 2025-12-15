import Foundation

// MARK: - SSAI Types

/// SSAI Session configuration
public struct SSAIConfig {
    /// SSAI service endpoint
    public let endpoint: URL
    
    /// Tenant ID
    public let tenantId: String
    
    /// Content stream URL (origin)
    public let contentUrl: URL
    
    /// Content format
    public let format: SSAIContentFormat
    
    /// Device capabilities
    public let capabilities: SSAIDeviceCapabilities?
    
    /// Session timeout (seconds)
    public let timeoutSeconds: TimeInterval
    
    /// Enable debug mode
    public let debug: Bool
    
    public init(
        endpoint: URL,
        tenantId: String,
        contentUrl: URL,
        format: SSAIContentFormat = .hls,
        capabilities: SSAIDeviceCapabilities? = nil,
        timeoutSeconds: TimeInterval = 3600,
        debug: Bool = false
    ) {
        self.endpoint = endpoint
        self.tenantId = tenantId
        self.contentUrl = contentUrl
        self.format = format
        self.capabilities = capabilities
        self.timeoutSeconds = timeoutSeconds
        self.debug = debug
    }
}

/// Content format
public enum SSAIContentFormat: String, Codable {
    case hls = "hls"
    case dash = "dash"
}

/// Device capabilities for ad selection
public struct SSAIDeviceCapabilities: Codable {
    /// Max supported resolution
    public let maxResolution: SSAIResolution?
    
    /// Supported codecs
    public let codecs: [String]?
    
    /// Supports HDR
    public let supportsHdr: Bool
    
    /// Supports Dolby
    public let supportsDolby: Bool
    
    /// Max bitrate (bps)
    public let maxBitrate: Int?
    
    public init(
        maxResolution: SSAIResolution? = nil,
        codecs: [String]? = nil,
        supportsHdr: Bool = false,
        supportsDolby: Bool = false,
        maxBitrate: Int? = nil
    ) {
        self.maxResolution = maxResolution
        self.codecs = codecs
        self.supportsHdr = supportsHdr
        self.supportsDolby = supportsDolby
        self.maxBitrate = maxBitrate
    }
    
    /// Default tvOS capabilities
    public static var tvOS: SSAIDeviceCapabilities {
        SSAIDeviceCapabilities(
            maxResolution: SSAIResolution(width: 3840, height: 2160),
            codecs: ["avc1", "hvc1", "mp4a"],
            supportsHdr: true,
            supportsDolby: true,
            maxBitrate: 25_000_000
        )
    }
}

/// Resolution
public struct SSAIResolution: Codable {
    public let width: Int
    public let height: Int
    
    public init(width: Int, height: Int) {
        self.width = width
        self.height = height
    }
}

/// SSAI Session response
public struct SSAISession: Codable {
    /// Session ID
    public let sessionId: String
    
    /// Manifest URL (with session token)
    public let manifestUrl: URL
    
    /// Session token for tracking
    public let sessionToken: String
    
    /// Tracking base URL
    public let trackingUrl: URL
    
    /// Session expires at
    public let expiresAt: Date
    
    /// Original content URL
    public let contentUrl: URL
}

/// SSAI Tracking event types
public enum SSAITrackingEventType: String, Codable {
    case impression
    case start
    case firstQuartile
    case midpoint
    case thirdQuartile
    case complete
    case error
    case skip
    case mute
    case unmute
    case pause
    case resume
    case fullscreen
    case exitFullscreen
    case click
}

/// SSAI Tracking event
public struct SSAITrackingEvent: Codable {
    /// Event type
    public let type: SSAITrackingEventType
    
    /// Pod ID
    public let podId: String
    
    /// Slot ID
    public let slotId: String
    
    /// Creative ID
    public let creativeId: String
    
    /// Current media time (seconds)
    public let mediaTime: TimeInterval
    
    /// Timestamp
    public let timestamp: Date
    
    /// Additional metadata
    public let metadata: [String: String]?
    
    public init(
        type: SSAITrackingEventType,
        podId: String,
        slotId: String,
        creativeId: String,
        mediaTime: TimeInterval,
        timestamp: Date = Date(),
        metadata: [String: String]? = nil
    ) {
        self.type = type
        self.podId = podId
        self.slotId = slotId
        self.creativeId = creativeId
        self.mediaTime = mediaTime
        self.timestamp = timestamp
        self.metadata = metadata
    }
}

/// SSAI Ad break (pod) info
public struct SSAIAdBreak: Codable {
    /// Pod ID
    public let podId: String
    
    /// Break position (seconds from start)
    public let position: TimeInterval
    
    /// Break duration (seconds)
    public let duration: TimeInterval
    
    /// Number of ads
    public let adCount: Int
    
    /// Break type
    public let type: SSAIAdBreakType
    
    /// Individual ads
    public let ads: [SSAIAd]
}

/// Ad break type
public enum SSAIAdBreakType: String, Codable {
    case preroll
    case midroll
    case postroll
}

/// SSAI Ad info
public struct SSAIAd: Codable {
    /// Slot ID
    public let slotId: String
    
    /// Creative ID
    public let creativeId: String
    
    /// Ad duration (seconds)
    public let duration: TimeInterval
    
    /// Advertiser name
    public let advertiser: String?
    
    /// Ad title
    public let title: String?
    
    /// Skip offset (seconds, if skippable)
    public let skipOffset: TimeInterval?
    
    /// Click-through URL
    public let clickThroughUrl: URL?
}

/// SSAI Error
public struct SSAIError: Error, Codable {
    /// Error code
    public let code: String
    
    /// Error message
    public let message: String
    
    /// Is fatal
    public let fatal: Bool
    
    /// Related pod ID
    public let podId: String?
    
    /// Related slot ID
    public let slotId: String?
    
    public init(
        code: String,
        message: String,
        fatal: Bool = false,
        podId: String? = nil,
        slotId: String? = nil
    ) {
        self.code = code
        self.message = message
        self.fatal = fatal
        self.podId = podId
        self.slotId = slotId
    }
}

/// SSAI session state
public enum SSAISessionState: String {
    case idle
    case initializing
    case active
    case inAdBreak
    case error
    case expired
}

/// SSAI Metrics
public struct SSAIMetrics {
    /// Total ad breaks seen
    public var adBreakCount: Int = 0
    
    /// Total ads seen
    public var adCount: Int = 0
    
    /// Total ad time (seconds)
    public var totalAdTime: TimeInterval = 0
    
    /// Completed ads
    public var completedAds: Int = 0
    
    /// Skipped ads
    public var skippedAds: Int = 0
    
    /// Errored ads
    public var erroredAds: Int = 0
    
    /// Fill rate (0-1)
    public var fillRate: Double = 0
}
