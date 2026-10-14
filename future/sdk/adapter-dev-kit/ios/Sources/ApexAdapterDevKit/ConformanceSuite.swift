import Foundation
import RivalApexMediationSDK

public enum ConformanceError: Error, LocalizedError {
    case initializationFailed(String)
    case loadTimedOut(String)
    case invalidAd(String)

    public var errorDescription: String? {
        switch self {
        case .initializationFailed(let m): return "Initialization failed: \(m)"
        case .loadTimedOut(let p): return "Load timed out for placement: \(p)"
        case .invalidAd(let r): return "Invalid ad: \(r)"
        }
    }
}

public struct ConformanceSuite {
    public init() {}

    public func runAll(
        adapterName: String,
        registerAdapter: () -> Void,
        context: AdapterTestContext
    ) async throws {
        // Allow caller to register BYO adapter type before initializing the SDK
        registerAdapter()
        try await initializeSDK(context: context)
        await MediationSDK.shared.setSandboxForceAdapterPipeline(true)
        await MediationSDK.shared.setSandboxAdapterWhitelist([adapterName])

        try await testInterstitial(placement: context.placementInterstitial, timeout: context.timeout)
        try await testRewarded(placement: context.placementRewarded, timeout: context.timeout)
    }

    private func initializeSDK(context: AdapterTestContext) async throws {
        if MediationSDK.shared.isInitialized { return }
        let cfg = SDKConfig(
            appId: context.appId,
            configEndpoint: SDKConfig.default(appId: context.appId).configEndpoint,
            auctionEndpoint: SDKConfig.default(appId: context.appId).auctionEndpoint,
            telemetryEnabled: true,
            logLevel: .debug,
            testMode: true,
            configSignaturePublicKey: nil
        )
        do {
            _ = try await MediationSDK.shared.initialize(appId: context.appId, configuration: cfg)
        } catch {
            throw ConformanceError.initializationFailed(String(describing: error))
        }
    }

    private func testInterstitial(placement: String, timeout: TimeInterval) async throws {
        let ad = try await withTimeout(seconds: timeout) {
            try await MediationSDK.shared.loadAd(placementId: placement)
        }
        guard let ad = ad else { throw ConformanceError.invalidAd("nil interstitial") }
        guard ad.networkName.isEmpty == false else { throw ConformanceError.invalidAd("missing network name") }
    }

    private func testRewarded(placement: String, timeout: TimeInterval) async throws {
        let ad = try await withTimeout(seconds: timeout) {
            try await MediationSDK.shared.loadAd(placementId: placement)
        }
        guard let ad = ad else { throw ConformanceError.invalidAd("nil rewarded") }
        guard ad.networkName.isEmpty == false else { throw ConformanceError.invalidAd("missing network name") }
    }
}

private func withTimeout<T>(seconds: TimeInterval, _ block: @escaping () async throws -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await block() }
        group.addTask {
            try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
            throw ConformanceError.loadTimedOut("timeout")
        }
        let value = try await group.next()!
        group.cancelAll()
        return value
    }
}
