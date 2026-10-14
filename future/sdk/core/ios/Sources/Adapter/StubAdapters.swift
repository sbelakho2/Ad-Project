import Foundation
#if DEBUG || APEX_SANDBOX_ADAPTERS

// MARK: - Base stub adapter

public class BaseStubAdapter: AdNetworkAdapter {
    private let _networkName: String
    private let requiredConfigKey: String
    private var isInitialized = false

    public init(networkName: String, requiredConfigKey: String) {
        self._networkName = networkName
        self.requiredConfigKey = requiredConfigKey
    }

    public required convenience init() {
        // This base class should not be constructed via required init directly
        self.init(networkName: "stub", requiredConfigKey: "key")
    }

    public var networkName: String { _networkName }
    public var version: String { "1.0.0" }
    public var minSDKVersion: String { "1.0.0" }

    public func initialize(config: [String : Any]) throws {
        guard let v = config[requiredConfigKey] as? String, !v.isEmpty else {
            throw AdapterRegistryError.loadFailed("\(requiredConfigKey) required")
        }
        isInitialized = true
    }

    public func loadAd(placement: String, adType: AdType, config: [String : Any], completion: @escaping @Sendable (Result<Ad, AdapterRegistryError>) -> Void) {
        guard isInitialized else { completion(.failure(.notInitialized)); return }
        guard supportsAdType(adType) else { completion(.failure(.unsupportedAdType)); return }
        // Return a basic mock banner
        let creative: Creative = .banner(
            imageURL: "https://example.invalid/ad.png",
            clickURL: "https://example.invalid/click",
            width: 320,
            height: 50
        )
        let ad = Ad(
            adId: "mock-\(networkName)-\(placement)",
            placement: placement,
            adType: adType,
            creative: creative,
            networkName: networkName,
            cpm: 0.99,
            expiresAt: Date().addingTimeInterval(3600),
            metadata: [:]
        )
        completion(.success(ad))
    }

    public func supportsAdType(_ adType: AdType) -> Bool {
        switch adType { case .banner, .interstitial, .rewarded: return true; default: return false }
    }

    public func destroy() { isInitialized = false }
}

// MARK: - Concrete stub adapters (12)

public final class IronSourceAdapter: BaseStubAdapter { public required init() { super.init(networkName: "ironsource", requiredConfigKey: "app_key") } }
public final class FacebookAdapter: BaseStubAdapter { public required init() { super.init(networkName: "facebook", requiredConfigKey: "placement_id") } }
public final class VungleAdapter: BaseStubAdapter { public required init() { super.init(networkName: "vungle", requiredConfigKey: "app_id") } }
public final class ChartboostAdapter: BaseStubAdapter { public required init() { super.init(networkName: "chartboost", requiredConfigKey: "app_id") } }
public final class PangleAdapter: BaseStubAdapter { public required init() { super.init(networkName: "pangle", requiredConfigKey: "app_id") } }
public final class MintegralAdapter: BaseStubAdapter { public required init() { super.init(networkName: "mintegral", requiredConfigKey: "app_id") } }
public final class AdColonyAdapter: BaseStubAdapter { public required init() { super.init(networkName: "adcolony", requiredConfigKey: "app_id") } }
public final class TapjoyAdapter: BaseStubAdapter { public required init() { super.init(networkName: "tapjoy", requiredConfigKey: "sdk_key") } }
public final class MolocoAdapter: BaseStubAdapter { public required init() { super.init(networkName: "moloco", requiredConfigKey: "seat_id") } }
public final class FyberAdapter: BaseStubAdapter { public required init() { super.init(networkName: "fyber", requiredConfigKey: "app_id") } }
public final class SmaatoAdapter: BaseStubAdapter { public required init() { super.init(networkName: "smaato", requiredConfigKey: "publisher_id") } }
public final class AmazonAdapter: BaseStubAdapter { public required init() { super.init(networkName: "amazon", requiredConfigKey: "app_key") } }

// Simple stubs for networks referenced by the registry that were missing here
public final class AppLovinAdapter: BaseStubAdapter { public required init() { super.init(networkName: "applovin", requiredConfigKey: "sdk_key") } }
public final class UnityAdsAdapter: BaseStubAdapter { public required init() { super.init(networkName: "unity", requiredConfigKey: "game_id") } }

// Enhanced AdMob simulation stub used by sandbox/testing
public final class AdMobAdapter: AdNetworkAdapter {
    public var networkName: String { "admob" }
    public var version: String { "1.0.0" }
    public var minSDKVersion: String { "1.0.0" }
    private var isInitialized = false
    private var globalConfig: [String: Any] = [:]
    public required init() {}
    public func initialize(config: [String : Any]) throws {
        guard let appId = config["app_id"] as? String, !appId.isEmpty else {
            throw AdapterRegistryError.loadFailed("app_id required")
        }
        globalConfig = config
        isInitialized = true
    }
    public func loadAd(placement: String, adType: AdType, config: [String : Any], completion: @escaping @Sendable (Result<Ad, AdapterRegistryError>) -> Void) {
        guard isInitialized else { completion(.failure(.notInitialized)); return }
        guard supportsAdType(adType) else { completion(.failure(.unsupportedAdType)); return }
        var settings = globalConfig; config.forEach { settings[$0.key] = $0.value }
        if let noFill = settings["no_fill"] as? Bool, noFill { completion(.failure(.loadFailed("no_fill"))); return }
        if let timeoutMs = settings["timeout_ms"] as? Int, timeoutMs > 0 {
            let deadline = DispatchTime.now() + .milliseconds(max(1, timeoutMs))
            DispatchQueue.global().asyncAfter(deadline: deadline) { completion(.failure(.timeout)) }
            return
        }
        let cpm = (settings["cpm"] as? Double) ?? 1.20
        let adId = "admob-\(UUID().uuidString.prefix(8))"
        let creative: Creative = ({ () -> Creative in
            if case .banner = adType {
                return .banner(imageURL: "https://example.invalid/admob/banner.png", clickURL: "https://example.invalid/click", width: 320, height: 50)
            } else {
                return .video(videoURL: "https://example.invalid/admob/video.mp4", clickURL: "https://example.invalid/click", duration: 15)
            }
        })()
        var metadata: [String: String] = [:]
        if let consent = settings["apx_consent_state"] as? [String: Any] {
            if let gdpr = consent["gdpr"] as? Int { metadata["gdpr"] = String(gdpr) }
            if let usPrivacy = consent["us_privacy"] as? String { metadata["us_privacy"] = usPrivacy }
            if let coppa = consent["coppa"] as? Int { metadata["coppa"] = String(coppa) }
        }
        if let sandbox = settings["apx_sandbox"] as? Bool, sandbox { metadata["sandbox"] = "1" }
        let ad = Ad(adId: adId, placement: placement, adType: adType, creative: creative, networkName: networkName, cpm: cpm, expiresAt: Date().addingTimeInterval(1800), metadata: metadata)
        completion(.success(ad))
    }
    public func supportsAdType(_ adType: AdType) -> Bool { switch adType { case .banner, .interstitial, .rewarded: return true; default: return false } }
    public func destroy() { isInitialized = false }
}

#endif
