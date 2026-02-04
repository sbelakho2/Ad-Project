import Foundation
import CoreTelephony

// MARK: - Signal Collection API for S2S Header Bidding

/// Signal Collection API for Server-to-Server (S2S) Header Bidding
///
/// This API enables collection of bid tokens and device signals that can be passed
/// to Prebid Server, AppLovin MAX, or other SSPs for server-side auction participation.
///
/// Industry Standard Alignment:
/// - AppLovin MAX: MASignalCollector
/// - Google AdMob: GADRTBAdapter.collectSignals()
/// - Prebid Mobile: PrebidMobile.fetchDemand()
public final class SignalCollector {
    
    public static let shared = SignalCollector()
    
    private var sdkVersion: String = "1.0.0"
    private var tokenCache: [String: CachedToken] = [:]
    private let cacheLock = NSLock()
    private let tokenTTL: TimeInterval = 300 // 5 minutes
    
    private init() {}
    
    // MARK: - Types
    
    /// Parameters for signal collection
    public struct SignalCollectionParams {
        public let adType: AdType
        public let placementId: String
        public let networkHints: [String]
        public let timeoutMs: Int
        public let customData: [String: String]
        
        public init(
            adType: AdType,
            placementId: String,
            networkHints: [String] = [],
            timeoutMs: Int = 3000,
            customData: [String: String] = [:]
        ) {
            self.adType = adType
            self.placementId = placementId
            self.networkHints = networkHints
            self.timeoutMs = timeoutMs
            self.customData = customData
        }
    }
    
    /// Collected bid signal containing all data needed for S2S bidding
    public struct BidSignal {
        public let bidToken: String
        public let signalVersion: String
        public let collectedAt: Date
        public let adType: AdType
        public let placementId: String
        public let deviceSignals: DeviceSignals
        public let consentSignals: ConsentSignals
        public let networkSignals: [String: NetworkBidToken]
        public let customData: [String: String]
        
        /// Serialize to JSON for transmission
        public func toJSON() -> [String: Any] {
            return [
                "bidToken": bidToken,
                "signalVersion": signalVersion,
                "collectedAt": Int(collectedAt.timeIntervalSince1970 * 1000),
                "adType": adType.rawValue,
                "placementId": placementId,
                "deviceSignals": deviceSignals.toJSON(),
                "consentSignals": consentSignals.toJSON(),
                "networkSignals": networkSignals.mapValues { $0.toJSON() },
                "customData": customData
            ]
        }
        
        /// Get raw token string for SSP
        public func getRawToken() -> String { bidToken }
    }
    
    /// Device-level signals
    public struct DeviceSignals {
        public let osVersion: String
        public let deviceModel: String
        public let deviceName: String
        public let screenScale: CGFloat
        public let screenWidth: CGFloat
        public let screenHeight: CGFloat
        public let language: String
        public let timezone: String
        public let connectionType: String
        public let carrier: String?
        public let isTablet: Bool
        public let sdkVersion: String
        
        public func toJSON() -> [String: Any] {
            return [
                "osVersion": osVersion,
                "deviceModel": deviceModel,
                "deviceName": deviceName,
                "screenScale": screenScale,
                "screenWidth": screenWidth,
                "screenHeight": screenHeight,
                "language": language,
                "timezone": timezone,
                "connectionType": connectionType,
                "carrier": carrier ?? "",
                "isTablet": isTablet,
                "sdkVersion": sdkVersion
            ]
        }
    }
    
    /// Consent and privacy signals
    public struct ConsentSignals {
        public let gdprApplies: Bool?
        public let tcfConsentString: String?
        public let uspString: String?
        public let gppString: String?
        public let coppa: Bool?
        public let attStatus: Int?
        
        public func toJSON() -> [String: Any?] {
            return [
                "gdprApplies": gdprApplies,
                "tcfConsent": tcfConsentString,
                "usPrivacy": uspString,
                "gpp": gppString,
                "coppa": coppa,
                "attStatus": attStatus
            ]
        }
    }
    
    /// Per-network bid token
    public struct NetworkBidToken {
        public let networkName: String
        public let token: String
        public let collectedAt: Date
        public let expiresAt: Date?
        
        public func toJSON() -> [String: Any?] {
            return [
                "network": networkName,
                "token": token,
                "collectedAt": Int(collectedAt.timeIntervalSince1970 * 1000),
                "expiresAt": expiresAt.map { Int($0.timeIntervalSince1970 * 1000) }
            ]
        }
        
        public func isExpired() -> Bool {
            guard let exp = expiresAt else { return false }
            return Date() > exp
        }
    }
    
    /// Signal collection errors
    public enum SignalError: Error {
        case timeout
        case networkError(String)
        case notInitialized
        case invalidParams(String)
        case internalError(String)
        
        public var code: String {
            switch self {
            case .timeout: return "TIMEOUT"
            case .networkError: return "NETWORK_ERROR"
            case .notInitialized: return "NOT_INITIALIZED"
            case .invalidParams: return "INVALID_PARAMS"
            case .internalError: return "INTERNAL_ERROR"
            }
        }
    }
    
    // MARK: - Private Types
    
    private struct CachedToken {
        let signal: BidSignal
        let expiresAt: Date
    }
    
    // MARK: - Public API
    
    /// Initialize the signal collector (called by MediationSDK)
    internal func initialize(version: String) {
        sdkVersion = version
    }
    
    /// Collect signals asynchronously
    public func collectSignals(
        params: SignalCollectionParams,
        completion: @escaping (Result<BidSignal, SignalError>) -> Void
    ) {
        let timeout = DispatchTime.now() + .milliseconds(params.timeoutMs)
        
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self else {
                DispatchQueue.main.async {
                    completion(.failure(.notInitialized))
                }
                return
            }
            
            do {
                let signal = try self.collectSignalsSync(params: params)
                DispatchQueue.main.async {
                    completion(.success(signal))
                }
            } catch let error as SignalError {
                DispatchQueue.main.async {
                    completion(.failure(error))
                }
            } catch {
                DispatchQueue.main.async {
                    completion(.failure(.internalError(error.localizedDescription)))
                }
            }
        }
    }
    
    /// Collect signals with async/await
    @available(iOS 13.0, tvOS 13.0, *)
    public func collectSignals(params: SignalCollectionParams) async throws -> BidSignal {
        return try await withCheckedThrowingContinuation { continuation in
            collectSignals(params: params) { result in
                continuation.resume(with: result)
            }
        }
    }
    
    /// Get cached signal if available
    public func getCachedSignal(placementId: String, adType: AdType) -> BidSignal? {
        let key = "\(placementId):\(adType.rawValue)"
        cacheLock.lock()
        defer { cacheLock.unlock() }
        
        guard let cached = tokenCache[key] else { return nil }
        if Date() > cached.expiresAt {
            tokenCache.removeValue(forKey: key)
            return nil
        }
        return cached.signal
    }
    
    /// Clear all cached signals
    public func clearCache() {
        cacheLock.lock()
        tokenCache.removeAll()
        cacheLock.unlock()
    }
    
    // MARK: - Private Implementation
    
    private func collectSignalsSync(params: SignalCollectionParams) throws -> BidSignal {
        let deviceSignals = collectDeviceSignals()
        let consentSignals = collectConsentSignals()
        
        let bidToken = generateBidToken(
            placementId: params.placementId,
            adType: params.adType,
            deviceSignals: deviceSignals,
            consentSignals: consentSignals
        )
        
        let signal = BidSignal(
            bidToken: bidToken,
            signalVersion: "1.0",
            collectedAt: Date(),
            adType: params.adType,
            placementId: params.placementId,
            deviceSignals: deviceSignals,
            consentSignals: consentSignals,
            networkSignals: [:],
            customData: params.customData
        )
        
        // Cache the signal
        let key = "\(params.placementId):\(params.adType.rawValue)"
        let cached = CachedToken(
            signal: signal,
            expiresAt: Date().addingTimeInterval(tokenTTL)
        )
        cacheLock.lock()
        tokenCache[key] = cached
        cacheLock.unlock()
        
        return signal
    }
    
    private func collectDeviceSignals() -> DeviceSignals {
        let device = UIDevice.current
        let screen = UIScreen.main
        
        var systemInfo = utsname()
        uname(&systemInfo)
        let modelCode = withUnsafePointer(to: &systemInfo.machine) {
            $0.withMemoryRebound(to: CChar.self, capacity: 1) {
                String(validatingUTF8: $0) ?? "Unknown"
            }
        }
        
        return DeviceSignals(
            osVersion: device.systemVersion,
            deviceModel: modelCode,
            deviceName: device.name,
            screenScale: screen.scale,
            screenWidth: screen.bounds.width,
            screenHeight: screen.bounds.height,
            language: Locale.current.languageCode ?? "en",
            timezone: TimeZone.current.identifier,
            connectionType: detectConnectionType(),
            carrier: detectCarrier(),
            isTablet: device.userInterfaceIdiom == .pad,
            sdkVersion: sdkVersion
        )
    }
    
    private func collectConsentSignals() -> ConsentSignals {
        let consent = ConsentManager.shared.getConsent()
        
        var attStatus: Int? = nil
        if #available(iOS 14.0, tvOS 14.0, *) {
            attStatus = Int(ATTrackingManager.trackingAuthorizationStatus.rawValue)
        }
        
        return ConsentSignals(
            gdprApplies: consent.gdprApplies,
            tcfConsentString: consent.tcfConsentString,
            uspString: consent.uspString,
            gppString: consent.gppString,
            coppa: consent.coppa,
            attStatus: attStatus
        )
    }
    
    private func generateBidToken(
        placementId: String,
        adType: AdType,
        deviceSignals: DeviceSignals,
        consentSignals: ConsentSignals
    ) -> String {
        let input = [
            placementId,
            adType.rawValue,
            deviceSignals.osVersion,
            deviceSignals.deviceModel,
            String(describing: consentSignals.gdprApplies),
            consentSignals.tcfConsentString ?? "",
            String(Int(Date().timeIntervalSince1970 * 1000)),
            UUID().uuidString
        ].joined()
        
        let digest = SHA256.hash(data: Data(input.utf8))
        let hashString = digest.compactMap { String(format: "%02x", $0) }.joined()
        return "apex_" + String(hashString.prefix(32))
    }
    
    private func detectConnectionType() -> String {
        // Simplified - in production use Reachability or NWPathMonitor
        return "wifi"
    }
    
    private func detectCarrier() -> String? {
        #if !targetEnvironment(simulator)
        let networkInfo = CTTelephonyNetworkInfo()
        if let carrier = networkInfo.serviceSubscriberCellularProviders?.values.first {
            return carrier.carrierName
        }
        #endif
        return nil
    }
}

// MARK: - SHA256 Helper (simplified)

import CommonCrypto

private enum SHA256 {
    static func hash(data: Data) -> [UInt8] {
        var digest = [UInt8](repeating: 0, count: Int(CC_SHA256_DIGEST_LENGTH))
        data.withUnsafeBytes { buffer in
            _ = CC_SHA256(buffer.baseAddress, CC_LONG(buffer.count), &digest)
        }
        return digest
    }
}

// MARK: - Supporting Types

public enum AdType: String {
    case banner = "BANNER"
    case interstitial = "INTERSTITIAL"
    case rewarded = "REWARDED"
    case rewardedInterstitial = "REWARDED_INTERSTITIAL"
    case native = "NATIVE"
    case appOpen = "APP_OPEN"
}

import AppTrackingTransparency

// Extension to import UIKit types
#if canImport(UIKit)
import UIKit
#endif
