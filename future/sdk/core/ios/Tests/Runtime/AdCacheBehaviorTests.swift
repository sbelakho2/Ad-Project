import XCTest
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
@testable import RivalApexMediationSDK

private final class MutableClock: ClockProtocol, @unchecked Sendable {
    var wallSeconds: TimeInterval
    var monotonicSecondsValue: TimeInterval

    init(wallSeconds: TimeInterval = Date().timeIntervalSince1970, monotonicSeconds: TimeInterval = 0) {
        self.wallSeconds = wallSeconds
        self.monotonicSecondsValue = monotonicSeconds
    }

    func now() -> Date { Date(timeIntervalSince1970: wallSeconds) }
    func nowMillis() -> Int64 { Int64(wallSeconds * 1000) }
    func monotonicMillis() -> Int64 { Int64(monotonicSecondsValue * 1000) }
    func monotonicSeconds() -> TimeInterval { monotonicSecondsValue }

    func advance(seconds: TimeInterval) {
        wallSeconds += seconds
        monotonicSecondsValue += seconds
    }

    func jumpWall(seconds: TimeInterval) {
        wallSeconds += seconds
    }
}

final class AdCacheBehaviorTests: XCTestCase {
    private var sdk: MediationSDK!
    private var previousClock: ClockProtocol!
    private var testClock: MutableClock!

    override func setUp() async throws {
    #if os(Linux)
        throw XCTSkip("Ad cache tests require Apple's networking stack")
    #else
        previousClock = Clock.shared
        testClock = MutableClock(monotonicSeconds: 1000)
        Clock.shared = testClock
        sdk = MediationSDK.shared
        #if DEBUG
        sdk._resetRuntimeForTesting(clock: testClock)
        #endif
        NetworkTestHooks.registerMockProtocol(MockURLProtocolFixture.self)
        MockURLProtocolFixture.reset()
    #endif
    }

    override func tearDown() {
    #if os(Linux)
        return
    #else
        MockURLProtocolFixture.reset()
        NetworkTestHooks.resetMockProtocols()
        sdk?.shutdown()
        Clock.shared = previousClock
    #endif
        super.tearDown()
    }

    func testClaimingAdConsumesCacheEntry() async throws {
        MockURLProtocolFixture.scenario = .success
        MockURLProtocolFixture.enqueueAdResponse(.init(adId: "ios-cache-first", placementId: "interstitial_main", expiresIn: 120))

        let config = testConfig()
        try await sdk.initialize(appId: "ios-cache-app", configuration: config)

        let loaded = try await sdk.loadAd(placementId: "interstitial_main")
        XCTAssertEqual("ios-cache-first", loaded?.adId)
        XCTAssertTrue(sdk.isAdReady(placementId: "interstitial_main"))

        let peeked = await sdk.peekAd(placementId: "interstitial_main")
        XCTAssertEqual("ios-cache-first", peeked?.adId)

        let claimed = await sdk.claimAd(placementId: "interstitial_main")
        XCTAssertEqual("ios-cache-first", claimed?.adId)
        XCTAssertFalse(sdk.isAdReady(placementId: "interstitial_main"))
    }

    func testExpiredAdIsPrunedBeforeClaim() async throws {
        MockURLProtocolFixture.scenario = .success
        MockURLProtocolFixture.enqueueAdResponse(.init(adId: "ios-cache-exp", placementId: "interstitial_main", expiresIn: 0.5))

        let config = testConfig()
        try await sdk.initialize(appId: "ios-cache-exp-app", configuration: config)

        _ = try await sdk.loadAd(placementId: "interstitial_main")
        XCTAssertTrue(sdk.isAdReady(placementId: "interstitial_main"))

        testClock.advance(seconds: 2)

        let claimed = await sdk.claimAd(placementId: "interstitial_main")
        XCTAssertNil(claimed, "Expired ads should not be returned from claimAd")
        XCTAssertFalse(sdk.isAdReady(placementId: "interstitial_main"))
    }

    func testSequentialLoadsQueueMultipleAds() async throws {
        MockURLProtocolFixture.scenario = .success
        MockURLProtocolFixture.enqueueAdResponse(.init(adId: "ios-cache-one", placementId: "interstitial_main", expiresIn: 300))
        MockURLProtocolFixture.enqueueAdResponse(.init(adId: "ios-cache-two", placementId: "interstitial_main", expiresIn: 300))

        let config = testConfig()
        try await sdk.initialize(appId: "ios-cache-queue", configuration: config)

        _ = try await sdk.loadAd(placementId: "interstitial_main")
        _ = try await sdk.loadAd(placementId: "interstitial_main")

        let first = await sdk.claimAd(placementId: "interstitial_main")
        XCTAssertEqual("ios-cache-one", first?.adId)
        XCTAssertTrue(sdk.isAdReady(placementId: "interstitial_main"), "Second ad should remain cached after first claim")

        let second = await sdk.claimAd(placementId: "interstitial_main")
        XCTAssertEqual("ios-cache-two", second?.adId)
        XCTAssertFalse(sdk.isAdReady(placementId: "interstitial_main"))
    }

    func testCachedAdsIgnoreBackwardWallClockJump() async throws {
        MockURLProtocolFixture.scenario = .success
        MockURLProtocolFixture.enqueueAdResponse(.init(adId: "ios-cache-drift", placementId: "interstitial_main", expiresIn: 120))

        let config = testConfig()
        try await sdk.initialize(appId: "ios-cache-drift-app", configuration: config)

        let loaded = try await sdk.loadAd(placementId: "interstitial_main")
        XCTAssertEqual("ios-cache-drift", loaded?.adId)
        XCTAssertTrue(sdk.isAdReady(placementId: "interstitial_main"))

        // Simulate user adjusting device clock backwards by 2 hours while monotonic time marches on
        testClock.jumpWall(seconds: -7200)
        testClock.advance(seconds: 5)

        XCTAssertTrue(sdk.isAdReady(placementId: "interstitial_main"), "Cache readiness should rely on monotonic clock")
        let claimed = await sdk.claimAd(placementId: "interstitial_main")
        XCTAssertEqual("ios-cache-drift", claimed?.adId)
    }

    private func testConfig() -> SDKConfig {
        SDKConfig(
            appId: "test_app",
            configEndpoint: "https://mock-config.test/v1/config",
            auctionEndpoint: "https://mock-auction.test/v1/auction",
            telemetryEnabled: false,
            logLevel: .error,
            testMode: true,
            configSignaturePublicKey: nil
        )
    }
}
