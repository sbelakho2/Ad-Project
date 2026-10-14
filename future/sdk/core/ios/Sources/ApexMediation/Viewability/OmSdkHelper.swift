#if canImport(UIKit)
import Foundation
import UIKit

private enum OmSdkState {
    case unavailable
    case initialized(partner: AnyObject, sdk: AnyObject?)
}

/// Reflection-only helper for OM SDK. Safe to ship when the host app does not include OMSDK.
public final class OmSdkHelper: @unchecked Sendable {
    public static let shared = OmSdkHelper()

    private let lock = NSLock()
    private var state: OmSdkState = .unavailable

    private init() {}

    /// Whether OM SDK was located and initialized in this process.
    public var isAvailable: Bool {
        lock.withLock {
            if case .initialized = state { return true }
            return false
        }
    }

    /// Initialize OM SDK if the vendor library is present. No-op (and safe) when missing.
    @discardableResult
    public func initializeIfAvailable(partnerName: String = "ApexMediation", version: String = "1.0.0") -> Bool {
        lock.withLock {
            guard case .unavailable = state else { return isAvailable }

            guard let sdkClass = OmSdkHelper.classType(firstAvailable: ["OMIDApexSDK", "OMIDSDK"]) as? NSObject.Type else {
                state = .unavailable
                return false
            }
            guard let partnerClass = OmSdkHelper.classType(firstAvailable: ["OMIDApexPartner", "OMIDPartner"]) as? NSObject.Type else {
                state = .unavailable
                return false
            }

            let sdkInstance = sdkClass.value(forKey: "sharedInstance") as AnyObject?
                ?? sdkClass.value(forKey: "shared") as AnyObject?
            if let active = sdkInstance?.value(forKey: "isActive") as? Bool, !active {
                _ = sdkInstance?.perform(NSSelectorFromString("activate"))
            }

            guard let partner = partnerClass.perform(NSSelectorFromString("partnerWithName:version:"), with: partnerName, with: version)?.takeUnretainedValue()
                ?? partnerClass.perform(NSSelectorFromString("initWithName:version:"), with: partnerName, with: version)?.takeUnretainedValue() else {
                state = .unavailable
                return false
            }

            state = .initialized(partner: partner as AnyObject, sdk: sdkInstance)
            return true
        }
    }

    public struct SessionHandle { let adSession: AnyObject }

    /// Start a native OM session if OMSDK types are present. Returns nil on failure without crashing.
    public func startSession(adView: UIView, isVideo: Bool, friendlyObstructions: [UIView] = []) -> SessionHandle? {
        let snapshot = lock.withLock { state }
        guard case let .initialized(partner, _) = snapshot else { return nil }

          guard let adSessionClass = OmSdkHelper.classType(firstAvailable: ["OMIDAdSession"]) as? NSObject.Type,
              let configClass = OmSdkHelper.classType(firstAvailable: ["OMIDAdSessionConfiguration"]) as? NSObject.Type,
              let contextClass = OmSdkHelper.classType(firstAvailable: ["OMIDAdSessionContext"]) as? NSObject.Type else { return nil }

        let config = OmSdkHelper.makeConfiguration(configClass: configClass, isVideo: isVideo)
        let context = OmSdkHelper.makeContext(contextClass: contextClass, partner: partner, adView: adView)
        guard let configuration = config, let sessionContext = context else { return nil }

        guard let adSession = OmSdkHelper.makeAdSession(adSessionClass: adSessionClass, configuration: configuration, context: sessionContext) else { return nil }
        OmSdkHelper.registerViewIfPossible(adSession: adSession, view: adView)
        friendlyObstructions.forEach { OmSdkHelper.addFriendlyObstructionIfPossible(adSession: adSession, view: $0) }
        OmSdkHelper.startIfPossible(adSession: adSession)
        OmSdkHelper.fireEventsIfPossible(adSession: adSession)
        return SessionHandle(adSession: adSession)
    }

    public func finishSession(_ handle: SessionHandle?) {
        guard let handle else { return }
        _ = handle.adSession.perform(NSSelectorFromString("finish"))
    }
}

private extension OmSdkHelper {
    static func classType(firstAvailable names: [String]) -> AnyClass? {
        for name in names {
            if let klass = NSClassFromString(name) {
                return klass
            }
        }
        return nil
    }

    static func enumValue(named caseName: String, in classNames: [String]) -> AnyObject? {
        for name in classNames {
            if let enumClass = NSClassFromString(name) as? NSObject.Type {
                if let value = enumClass.perform(NSSelectorFromString(caseName))?.takeUnretainedValue() {
                    return value as AnyObject
                }
                if let value = enumClass.value(forKey: caseName) as AnyObject? {
                    return value
                }
            }
        }
        return nil
    }

    static func makeConfiguration(configClass: NSObject.Type, isVideo: Bool) -> AnyObject? {
        let creative = enumValue(named: isVideo ? "video" : "htmlDisplay", in: ["OMIDCreativeType", "OMIDCreativeTypeEnum"])
        let impression = enumValue(named: "viewable", in: ["OMIDImpressionType", "OMIDImpressionTypeEnum"])
        let owner = enumValue(named: "native", in: ["OMIDOwner", "OMIDOwnerEnum"])
        if let instance = configClass.init() as NSObject? {
            if let creative { instance.setValue(creative, forKey: "creativeType") }
            if let impression { instance.setValue(impression, forKey: "impressionType") }
            if let owner {
                instance.setValue(owner, forKey: "impressionOwner")
                instance.setValue(owner, forKey: "mediaEventsOwner")
            }
            instance.setValue(false, forKey: "isolateVerificationScripts")
            return instance
        }
        return nil
    }

    static func makeContext(contextClass: NSObject.Type, partner: AnyObject, adView: UIView) -> AnyObject? {
        if let result = contextClass.perform(NSSelectorFromString("contextWithNativePartner:webView:contentUrl:customReferenceIdentifier:"), with: partner, with: nil)?.takeUnretainedValue() {
            return result as AnyObject
        }
        if let instance = contextClass.init() as NSObject? {
            instance.setValue(partner, forKey: "partner")
            instance.setValue(adView, forKey: "mainAdView")
            return instance
        }
        return nil
    }

    static func makeAdSession(adSessionClass: NSObject.Type, configuration: AnyObject, context: AnyObject) -> AnyObject? {
        if let result = adSessionClass.perform(NSSelectorFromString("adSessionWithConfiguration:adSessionContext:"), with: configuration, with: context)?.takeUnretainedValue() {
            return result as AnyObject
        }
        if let instance = adSessionClass.init() as NSObject? {
            _ = instance.perform(NSSelectorFromString("initWithConfiguration:adSessionContext:"), with: configuration, with: context)
            return instance
        }
        return nil
    }

    static func registerViewIfPossible(adSession: AnyObject, view: UIView) {
        if adSession.responds(to: NSSelectorFromString("setMainAdView:")) {
            adSession.setValue(view, forKey: "mainAdView")
        } else {
            _ = adSession.perform(NSSelectorFromString("registerAdView:"), with: view)
        }
    }

    static func addFriendlyObstructionIfPossible(adSession: AnyObject, view: UIView) {
        let purpose = enumValue(named: "other", in: ["OMIDFriendlyObstructionPurpose"])
        if let purpose {
            _ = adSession.perform(NSSelectorFromString("addFriendlyObstruction:purpose:detailedReason:"), with: view, with: purpose)
        } else {
            _ = adSession.perform(NSSelectorFromString("addFriendlyObstruction:"), with: view)
        }
    }

    static func startIfPossible(adSession: AnyObject) {
        _ = adSession.perform(NSSelectorFromString("start"))
    }

    static func fireEventsIfPossible(adSession: AnyObject) {
        guard let adEventsClass = classType(firstAvailable: ["OMIDAdEvents"]) as? NSObject.Type else { return }
        if let events = adEventsClass.perform(NSSelectorFromString("adEventsWithAdSession:"), with: adSession)?.takeUnretainedValue() {
            _ = (events as AnyObject).perform(NSSelectorFromString("loaded"))
            _ = (events as AnyObject).perform(NSSelectorFromString("impressionOccurred"))
        }
    }
}

#else
import Foundation

/// Stub OMSDK helper for platforms without UIKit (e.g., macOS SwiftPM tests).
public final class OmSdkHelper: @unchecked Sendable {
    public static let shared = OmSdkHelper()
    public var isAvailable: Bool { false }

    @discardableResult
    public func initializeIfAvailable(partnerName: String = "ApexMediation", version: String = "1.0.0") -> Bool { false }

    public struct SessionHandle { let adSession: AnyObject }

    public func startSession(adView: AnyObject, isVideo: Bool, friendlyObstructions: [AnyObject] = []) -> SessionHandle? { nil }
    public func finishSession(_ handle: SessionHandle?) { }
}

#endif

private extension NSLock {
    func withLock<T>(_ body: () -> T) -> T {
        lock()
        defer { unlock() }
        return body()
    }
}
