import XCTest
@testable import ApexAdapterDevKit
import RivalApexMediationSDK

// BYO test adapters (not part of production core) used to pre‑wire conformance
final class BYOTestAdMobAdapter: AdNetworkAdapter {
    var networkName: String { "admob" }
    var version: String { "test-1.0" }
    var minSDKVersion: String { "1.0.0" }
    required init() {}
    private var inited = false
    func initialize(config: [String : Any]) throws {
        guard let appId = config["app_id"] as? String, !appId.isEmpty else { throw AdapterRegistryError.loadFailed("app_id required") }
        inited = true
    }
    func loadAd(placement: String, adType: AdType, config: [String : Any], completion: @escaping (Result<Ad, AdapterRegistryError>) -> Void) {
        guard inited else { return completion(.failure(.notInitialized)) }
        guard adType == .interstitial || adType == .rewarded else { return completion(.failure(.unsupportedAdType)) }
        let ad = Ad(adId: "admob-\(placement)", placement: placement, adType: adType, creative: .video(videoURL: "https://example.invalid/v.mp4", clickURL: "https://example.invalid/c", duration: 15), networkName: networkName, cpm: 1.23, expiresAt: Date().addingTimeInterval(600), metadata: [:])
        completion(.success(ad))
    }
    func supportsAdType(_ adType: AdType) -> Bool { adType == .interstitial || adType == .rewarded }
    func destroy() { inited = false }
}

final class BYOTestAppLovinAdapter: BYOTestAdMobAdapter {
    override var networkName: String { "applovin" }
}

final class BYOTestIronSourceAdapter: BYOTestAdMobAdapter {
    override var networkName: String { "ironsource" }
}

final class PrewiredConformanceTests: XCTestCase {

    private func ctx() -> AdapterTestContext {
        AdapterTestContext(appId: "test-app", placementInterstitial: "test_interstitial", placementRewarded: "test_rewarded", timeout: 5)
    }

    func testConformance_AdMob() async throws {
        await MediationSDK.shared.registerAdapter(networkName: "admob", adapterType: BYOTestAdMobAdapter.self)
        let suite = ConformanceSuite()
        var failure: Error?
        do { try await suite.runAll(adapterName: "admob", registerAdapter: {}, context: ctx()) }
        catch { failure = error }
        XCTAssertNil(failure, "AdMob prewired adapter must pass the conformance suite: \(String(describing: failure))")
    }

    func testConformance_AppLovin() async throws {
        await MediationSDK.shared.registerAdapter(networkName: "applovin", adapterType: BYOTestAppLovinAdapter.self)
        let suite = ConformanceSuite()
        var failure: Error?
        do { try await suite.runAll(adapterName: "applovin", registerAdapter: {}, context: ctx()) }
        catch { failure = error }
        XCTAssertNil(failure, "AppLovin prewired adapter must pass the conformance suite: \(String(describing: failure))")
    }

    func testConformance_IronSource() async throws {
        await MediationSDK.shared.registerAdapter(networkName: "ironsource", adapterType: BYOTestIronSourceAdapter.self)
        let suite = ConformanceSuite()
        var failure: Error?
        do { try await suite.runAll(adapterName: "ironsource", registerAdapter: {}, context: ctx()) }
        catch { failure = error }
        XCTAssertNil(failure, "IronSource prewired adapter must pass the conformance suite: \(String(describing: failure))")
    }
}
