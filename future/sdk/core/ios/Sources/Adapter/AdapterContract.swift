import Foundation
#if canImport(CryptoKit)
import CryptoKit
#endif

// MARK: - Data Contracts

/// Publisher-supplied vendor credentials injected at runtime. Keep on-device.
/// Publisher-supplied vendor credentials injected at runtime. Keep on-device and never log.
public struct AdapterCredentials: Codable, Equatable, Sendable {
    public let key: String
    public let secret: String?
    public let appId: String?
    public let accountIds: [String: String]?

    /// Returns a redacted/hashed view for telemetry/debugging (secrets omitted).
    public func redacted() -> [String: String] {
        var result: [String: String] = [:]
        result["key_hash"] = Self.hashValue(key)
        if let appId { result["app_id_hash"] = Self.hashValue(appId) }
        if let accounts = accountIds, !accounts.isEmpty {
            result["account_ids"] = accounts.keys.sorted().joined(separator: ",")
        }
        return result
    }

    private static func hashValue(_ input: String) -> String {
        // lightweight SHA256 hex for non-cryptographic labeling; secrets never emitted raw
        if let data = input.data(using: .utf8) {
            #if canImport(CryptoKit)
            return SHA256.hash(data: data).compactMap { String(format: "%02x", $0) }.joined()
            #else
            return String(input.hashValue)
            #endif
        }
        return ""
    }
}

/// Optional knobs the controller can hand to adapters per placement.
public struct AdapterOptions: Codable, Equatable, Sendable {
    public let startMuted: Bool?
    public let testMode: Bool?
    public let bidFloorMicros: Int64?
}

public enum PartnerRegion: String, Codable, Sendable {
    case us, eu, apac, cn, global
}

/// Normalized privacy metadata forwarded to vendors.
public struct ConsentState: Codable, Equatable, Sendable {
    public let gdprApplies: Bool?
    public let iabTCFv2: String?
    public let iabUSPrivacy: String?
    public let coppa: Bool
    public let attStatus: ATTStatus
    public let limitAdTracking: Bool
    public let privacySandboxOptIn: Bool?
    public let advertisingId: String?
    public let appSetId: String?
}

public enum ATTStatus: String, Codable, Sendable {
    case authorized, denied, notDetermined, restricted
}

/// Canonical init payload mirrored across Android, iOS, and Unity.
public struct PartnerAdapterConfig: Codable, Equatable, Sendable {
    public let partner: String
    public let credentials: AdapterCredentials
    public let placements: [String: String]
    public let privacy: ConsentState
    public let region: PartnerRegion?
    public let options: AdapterOptions?
}

public struct DeviceMeta: Codable, Equatable, Sendable {
    public let os: String
    public let osVersion: String
    public let model: String
}

public struct UserMeta: Codable, Equatable, Sendable {
    public let ageRestricted: Bool
    public let consent: ConsentState
    public let advertisingId: String?
    public let appSetId: String?
}

public struct NetworkMeta: Codable, Equatable, Sendable {
    public let ipPrefixed: String
    public let uaNormalized: String
    public let connType: ConnectionType
}

public enum ConnectionType: String, Codable, Sendable {
    case wifi, cell, other
}

public struct ContextMeta: Codable, Equatable, Sendable {
    public let orientation: Orientation
    public let sessionDepth: Int
}

public enum Orientation: String, Codable, Sendable {
    case portrait, landscape
}

public struct AuctionMeta: Codable, Equatable, Sendable {
    public let floorsMicros: Int64?
    public let sChain: String?
    public let sellersJsonOk: Bool?
}

/// Per-request context (device, user, auction) supplied to adapters.
public struct RequestMeta: Codable, Equatable, Sendable {
    public let requestId: String
    public let device: DeviceMeta
    public let user: UserMeta
    public let net: NetworkMeta
    public let context: ContextMeta
    public let auction: AuctionMeta
}

/// Token referencing a single-use creative returned by an adapter.
public struct AdHandle: Codable, Equatable, Sendable {
    public let id: String
    public let adType: AdType
    public let partnerPlacementId: String?
    public let createdAtMs: Int64
    
    public init(id: String, adType: AdType, partnerPlacementId: String?, createdAtMs: Int64 = Clock.shared.nowMillis()) {
        self.id = id
        self.adType = adType
        self.partnerPlacementId = partnerPlacementId
        self.createdAtMs = createdAtMs
    }
}

/// Successful load response; ttlMs governs cache freshness.
public struct LoadResult: Codable, Sendable {
    public let handle: AdHandle
    public let ttlMs: Int
    public let priceMicros: Int64?
    public let currency: String?
    public let partnerMeta: [String: AnyCodable]
}

/// Outcome of adapter initialization.
public struct InitResult: Codable, Sendable {
    public let success: Bool
    public let error: AdapterError?
    public let partnerMeta: [String: AnyCodable]
}

public struct PaidEvent: Codable, Equatable, Sendable {
    public let valueMicros: Int64
    public let currency: String
    public let precision: PaidEventPrecision
    public let partner: String
    public let partnerUnitId: String?
    public let lineItemId: String?
    public let creativeId: String?
}

public enum PaidEventPrecision: String, Codable, Sendable { case publisher, estimated }

// MARK: - Error Taxonomy

public enum AdapterErrorCode: String, Codable, Sendable {
    case noFill
    case timeout
    case networkError
    case status4xx
    case status5xx
    case belowFloor
    case error
    case circuitOpen
    case config
    case noAdReady
}

/// Normalized failure mapped from vendor-specific errors.
public struct AdapterError: Error, Codable, Equatable, Sendable {
    public let code: AdapterErrorCode
    public let detail: String
    public let vendorCode: String?
    public let recoverable: Bool

    /// Normalizes vendor reasons into canonical strings for parity with Android.
    public func normalizedReason() -> String {
        switch code {
        case .noFill: return "no_fill"
        case .timeout: return "timeout"
        case .networkError: return "network_error"
        case .status4xx: return "status_4xx"
        case .status5xx: return "status_5xx"
        case .belowFloor: return "below_floor"
        case .circuitOpen: return "circuit_open"
        case .config: return "config"
        case .noAdReady: return "no_ad_ready"
        case .error: return "error"
        }
    }
}

// MARK: - Callbacks

public enum CloseReason: String, Codable, Sendable { case completed, skipped, dismissed }

public protocol ShowCallbacks: AnyObject {
    func onImpression(meta: [String: Any]?)
    func onPaidEvent(_ event: PaidEvent)
    func onClick(meta: [String: Any]?)
    func onClosed(reason: CloseReason)
    func onError(_ error: AdapterError)
}

public protocol RewardedCallbacks: ShowCallbacks {
    func onRewardVerified(rewardType: String, rewardAmount: Double)
}

public protocol BannerCallbacks: ShowCallbacks {
    func onViewAttached()
    func onViewDetached(reason: CloseReason)
}

public struct AdSize: Codable, Equatable, Sendable {
    public let width: Int
    public let height: Int
    public static let banner320x50 = AdSize(width: 320, height: 50)
}

// MARK: - Core Adapter Protocol

/// Shared adapter surface enforced across all ApexMediation platforms.
public protocol AdNetworkAdapterV2 {
    func initAdapter(config: PartnerAdapterConfig, timeoutMs: Int) -> InitResult
    func loadInterstitial(placementId: String, meta: RequestMeta, timeoutMs: Int) -> LoadResult
    func showInterstitial(handle: AdHandle, viewController: AnyObject, callbacks: ShowCallbacks)
    func loadRewarded(placementId: String, meta: RequestMeta, timeoutMs: Int) -> LoadResult
    func showRewarded(handle: AdHandle, viewController: AnyObject, callbacks: RewardedCallbacks)
    func loadBanner(placementId: String, size: AdSize, meta: RequestMeta, timeoutMs: Int) -> LoadResult
    func attachBanner(handle: AdHandle, hostView: AnyObject, callbacks: BannerCallbacks)
    func isAdReady(handle: AdHandle) -> Bool
    func expiresAt(handle: AdHandle) -> Int64
    func invalidate(handle: AdHandle)
}
