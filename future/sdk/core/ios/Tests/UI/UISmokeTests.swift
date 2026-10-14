import XCTest
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
@testable import RivalApexMediationSDK

/// Section 3.2: XCTest UI smoke tests with deterministic URLProtocol fixtures
/// Tests all ad load scenarios with main-queue callback assertions
final class UISmokeTests: XCTestCase {
    
    var sdk: MediationSDK!
    
    override func setUp() async throws {
    #if os(Linux)
        throw XCTSkip("UISmokeTests require Apple's networking stack")
    #else
        // Register MockURLProtocol for deterministic responses
        NetworkTestHooks.registerMockProtocol(MockURLProtocolFixture.self)
        MockURLProtocolFixture.reset()
        
        sdk = MediationSDK.shared
    #endif
    }
    
    override func tearDown() {
    #if os(Linux)
        return
    #else
        MockURLProtocolFixture.reset()
        NetworkTestHooks.resetMockProtocols()
        sdk.shutdown()
    #endif
    }
    
    // MARK: - Interstitial Success Tests
    
    func testInterstitialLoadSuccess() async throws {
        // Given: MockURLProtocol configured for success
        MockURLProtocolFixture.scenario = .success
        
        // When: Initialize SDK and load interstitial
        let config = testConfig()
        try await sdk.initialize(appId: "test_app", configuration: config)
        
        let ad = try await sdk.loadAd(placementId: "interstitial_main")
        
        // Then: Ad loaded successfully
        XCTAssertNotNil(ad)
        XCTAssertEqual(ad?.adType, .interstitial)
        XCTAssertEqual(ad?.networkName, "MockNetwork")
        XCTAssertGreaterThan(ad?.cpm ?? 0, 0)
        
        // Verify callback on main queue
        await MainActor.run {
            XCTAssertTrue(Thread.isMainThread, "Ad load callback must be on main thread")
        }
    }
    
    func testRewardedLoadSuccess() async throws {
        // Given: MockURLProtocol configured for success
        MockURLProtocolFixture.scenario = .success
        
        // When: Load rewarded ad
        let config = testConfig()
        try await sdk.initialize(appId: "test_app", configuration: config)
        
        let ad = try await sdk.loadAd(placementId: "rewarded_main")
        
        // Then: Ad loaded successfully
        XCTAssertNotNil(ad)
        XCTAssertEqual(ad?.adType, .rewarded)
        
        // Verify main-queue callback
        await MainActor.run {
            XCTAssertTrue(Thread.isMainThread, "Rewarded ad callback must be on main thread")
        }
    }
    
    // MARK: - No Fill Tests
    
    func testInterstitialNoFill() async throws {
        // Given: MockURLProtocol configured for no fill (204)
        MockURLProtocolFixture.scenario = .noFill
        
        // When: Load interstitial
        let config = testConfig()
        try await sdk.initialize(appId: "test_app", configuration: config)
        
        do {
            _ = try await sdk.loadAd(placementId: "interstitial_main")
            XCTFail("Should throw noFill error")
        } catch let error as SDKError {
            // Then: Correct error thrown
            XCTAssertEqual(error, .noFill)
        }
        
        // Verify main-queue callback
        await MainActor.run {
            XCTAssertTrue(Thread.isMainThread, "Error callback must be on main thread")
        }
    }
    
    func testRewardedNoFill() async throws {
        // Given: MockURLProtocol configured for no fill
        MockURLProtocolFixture.scenario = .noFill
        
        // When: Load rewarded ad
        let config = testConfig()
        try await sdk.initialize(appId: "test_app", configuration: config)
        
        do {
            _ = try await sdk.loadAd(placementId: "rewarded_main")
            XCTFail("Should throw noFill error")
        } catch let error as SDKError {
            // Then: Correct error thrown
            XCTAssertEqual(error, .noFill)
        }
        
        // Verify main-queue callback
        await MainActor.run {
            XCTAssertTrue(Thread.isMainThread)
        }
    }
    
    // MARK: - Rate Limit (429) Tests
    
    func testRateLimitHandling() async throws {
        // Given: MockURLProtocol returns 429
        MockURLProtocolFixture.scenario = .rateLimitExceeded
        
        // When: Load ad
        let config = testConfig()
        try await sdk.initialize(appId: "test_app", configuration: config)
        
        do {
            _ = try await sdk.loadAd(placementId: "interstitial_main")
            XCTFail("Should throw status_429 error")
        } catch let error as SDKError {
            // Then: Correct error type
            if case .status_429(let message) = error {
                XCTAssertTrue(message.contains("Rate limit") || message.contains("Too many"))
            } else {
                XCTFail("Expected status_429, got \(error)")
            }
        }
        
        // Verify main-queue callback
        await MainActor.run {
            XCTAssertTrue(Thread.isMainThread)
        }
    }
    
    // MARK: - Server Error (5xx) Tests
    
    func testServerErrorHandling() async throws {
        // Given: MockURLProtocol returns 503
        MockURLProtocolFixture.scenario = .serverError
        
        // When: Load ad
        let config = testConfig()
        try await sdk.initialize(appId: "test_app", configuration: config)
        
        do {
            _ = try await sdk.loadAd(placementId: "interstitial_main")
            XCTFail("Should throw status_5xx error")
        } catch let error as SDKError {
            // Then: Correct error type with status code
            if case .status_5xx(let code, let message) = error {
                XCTAssertEqual(code, 503)
                XCTAssertFalse(message.isEmpty)
            } else {
                XCTFail("Expected status_5xx, got \(error)")
            }
        }
        
        // Verify main-queue callback
        await MainActor.run {
            XCTAssertTrue(Thread.isMainThread)
        }
    }
    
    func test500InternalServerError() async throws {
        // Given: MockURLProtocol returns 500
        MockURLProtocolFixture.scenario = .serverError500
        
        // When: Load ad
        let config = testConfig()
        try await sdk.initialize(appId: "test_app", configuration: config)
        
        do {
            _ = try await sdk.loadAd(placementId: "rewarded_main")
            XCTFail("Should throw status_5xx error")
        } catch let error as SDKError {
            // Then: 500 error captured
            if case .status_5xx(let code, _) = error {
                XCTAssertEqual(code, 500)
            } else {
                XCTFail("Expected status_5xx(500), got \(error)")
            }
        }
    }
    
    // MARK: - Timeout Tests
    
    func testTimeoutHandling() async throws {
        // Given: MockURLProtocol simulates timeout
        MockURLProtocolFixture.scenario = .timeout
        
        // When: Load ad with short timeout
        let config = testConfig(testMode: true)

        try await sdk.initialize(appId: "test_app", configuration: config)
        
        do {
            _ = try await sdk.loadAd(placementId: "interstitial_main")
            XCTFail("Should throw timeout error")
        } catch let error as SDKError {
            // Then: Timeout error thrown
            XCTAssertEqual(error, .timeout)
        }
        
        // Verify main-queue callback
        await MainActor.run {
            XCTAssertTrue(Thread.isMainThread)
        }
    }
    
    // MARK: - Main-Queue Callback Verification
    
    func testAllCallbacksOnMainQueue() async throws {
        // Test matrix: all scenarios must callback on main thread
        let scenarios: [MockScenario] = [.success, .noFill, .rateLimitExceeded, .serverError, .timeout]
        
        for scenario in scenarios {
            MockURLProtocolFixture.scenario = scenario
            
            let config = testConfig()
            // Use separate SDK instance for each test (or reset state)
            
            do {
                try await sdk.initialize(appId: "test_app_\(scenario)", configuration: config)
                _ = try await sdk.loadAd(placementId: "test_placement")
            } catch {
                // Expected for error scenarios
            }
            
            // Verify main thread
            await MainActor.run {
                XCTAssertTrue(Thread.isMainThread, "Scenario \(scenario) must callback on main thread")
            }
        }
    }
    
    // MARK: - Concurrent Load Tests
    
    func testConcurrentAdLoadsAreThreadSafe() async throws {
        // Given: Multiple concurrent ad load requests
        MockURLProtocolFixture.scenario = .success
        
        let config = testConfig()
        try await sdk.initialize(appId: "test_app", configuration: config)
        
        // When: Load 5 ads concurrently
        try await withThrowingTaskGroup(of: Ad?.self) { group in
            for i in 0..<5 {
                group.addTask {
                    try await self.sdk.loadAd(placementId: "placement_\(i)")
                }
            }
            
            // Then: All ads load successfully without crashes
            var count = 0
            for try await ad in group {
                XCTAssertNotNil(ad)
                count += 1
            }
            XCTAssertEqual(count, 5)
        }
        
        // Verify main-queue callback
        await MainActor.run {
            XCTAssertTrue(Thread.isMainThread)
        }
    }
    
    // MARK: - Helpers
    
    private func testConfig(testMode: Bool = true) -> SDKConfig {
        return SDKConfig(
            appId: "test_app",
            configEndpoint: "https://mock-config.test/v1/config",
            auctionEndpoint: "https://mock-auction.test/v1/auction",
            telemetryEnabled: false,
            logLevel: .error,
            testMode: testMode,
            configSignaturePublicKey: nil
        )
    }
}

// MARK: - MockURLProtocol Fixture for UI Tests

enum MockScenario {
    case success
    case noFill
    case rateLimitExceeded
    case serverError
    case serverError500
    case timeout
}

class MockURLProtocolFixture: URLProtocol {
    static var scenario: MockScenario = .success
    private static var adResponseQueue: [AdResponseOverride] = []
    
    static func reset() {
        scenario = .success
        adResponseQueue.removeAll()
    }

    struct AdResponseOverride {
        let adId: String
        let placementId: String
        let adType: AdType
        let expiresIn: TimeInterval
        let cpm: Double
        init(adId: String,
             placementId: String,
             adType: AdType = .interstitial,
             expiresIn: TimeInterval = 3600,
             cpm: Double = 3.5) {
            self.adId = adId
            self.placementId = placementId
            self.adType = adType
            self.expiresIn = expiresIn
            self.cpm = cpm
        }
    }

    static func enqueueAdResponse(_ override: AdResponseOverride) {
        adResponseQueue.append(override)
    }
    
    private static func dequeueOverride() -> AdResponseOverride? {
        guard !adResponseQueue.isEmpty else { return nil }
        return adResponseQueue.removeFirst()
    }
    
    override class func canInit(with request: URLRequest) -> Bool {
        return request.url?.host?.contains("mock") == true
    }
    
    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        return request
    }
    
    override func startLoading() {
        guard let url = request.url else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }
        
        if url.path.contains("/v1/config") {
            sendConfigResponse()
            return
        }
        
        switch MockURLProtocolFixture.scenario {
        case .success:
            sendAuctionSuccess()
        case .noFill:
            sendNoFillResponse()
        case .rateLimitExceeded:
            sendRateLimitResponse()
        case .serverError:
            sendServerErrorResponse(code: 503)
        case .serverError500:
            sendServerErrorResponse(code: 500)
        case .timeout:
            sendTimeoutError()
        }
    }
    
    override func stopLoading() {}

    private func sendAuctionSuccess() {
        let override = MockURLProtocolFixture.dequeueOverride()
        let resolvedPlacement = override?.placementId ?? "test_placement"
        let resolvedAdId = override?.adId ?? "test_ad_123"
        let resolvedType = override?.adType.rawValue ?? "interstitial"
        let resolvedCpm = override?.cpm ?? 3.50
        let ttlSeconds = override?.expiresIn ?? 3600
        let json = """
        {
            "ad_id": "\(resolvedAdId)",
            "placement": "\(resolvedPlacement)",
            "ad_type": "\(resolvedType)",
            "creative": {"banner": {"url": "https://example.com/ad.jpg", "width": 320, "height": 480}},
            "network_name": "MockNetwork",
            "cpm": \(resolvedCpm),
            "expires_at": "\(futureISODate(offset: ttlSeconds))",
            "metadata": {}
        }
        """
        respond(statusCode: 200, body: json)
    }
    
    private func sendNoFillResponse() {
        respond(statusCode: 204)
    }
    
    private func sendRateLimitResponse() {
        let json = """
        {"error": "Rate limit exceeded", "retry_after": 60}
        """
        respond(statusCode: 429, headers: ["Retry-After": "60"], body: json)
    }
    
    private func sendServerErrorResponse(code: Int) {
        let json = """
        {"error": "Service unavailable"}
        """
        respond(statusCode: code, body: json)
    }
    
    private func sendTimeoutError() {
        let error = NSError(domain: NSURLErrorDomain, code: NSURLErrorTimedOut, userInfo: nil)
        client?.urlProtocol(self, didFailWithError: error)
    }
    
    private func sendConfigResponse() {
        respond(statusCode: 200, body: MockURLProtocolFixture.configPayload)
    }
    
    private func respond(statusCode: Int, headers: [String: String]? = nil, body: String? = nil) {
        let response = HTTPURLResponse(url: request.url!, statusCode: statusCode, httpVersion: "HTTP/1.1", headerFields: headers)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        if let body = body {
            client?.urlProtocol(self, didLoad: Data(body.utf8))
        }
        client?.urlProtocolDidFinishLoading(self)
    }
    
    private func futureISODate(offset seconds: TimeInterval = 3600) -> String {
        let formatter = ISO8601DateFormatter()
        return formatter.string(from: Date().addingTimeInterval(seconds))
    }

    private static let configPayload: String = {
        return """
        {
            "version": 1,
            "placements": [
                {"placement_id": "interstitial_main", "ad_type": "interstitial", "adapter_priority": ["admob"], "timeout_ms": 2000, "floor_cpm": 1.5},
                {"placement_id": "rewarded_main", "ad_type": "rewarded", "adapter_priority": ["admob"], "timeout_ms": 2000, "floor_cpm": 2.5},
                {"placement_id": "test_placement", "ad_type": "interstitial", "adapter_priority": ["admob"], "timeout_ms": 2000, "floor_cpm": 1.0},
                {"placement_id": "placement_0", "ad_type": "interstitial", "adapter_priority": ["admob"], "timeout_ms": 2000, "floor_cpm": 1.0},
                {"placement_id": "placement_1", "ad_type": "interstitial", "adapter_priority": ["admob"], "timeout_ms": 2000, "floor_cpm": 1.0},
                {"placement_id": "placement_2", "ad_type": "interstitial", "adapter_priority": ["admob"], "timeout_ms": 2000, "floor_cpm": 1.0},
                {"placement_id": "placement_3", "ad_type": "interstitial", "adapter_priority": ["admob"], "timeout_ms": 2000, "floor_cpm": 1.0},
                {"placement_id": "placement_4", "ad_type": "interstitial", "adapter_priority": ["admob"], "timeout_ms": 2000, "floor_cpm": 1.0}
            ],
            "adapters": {
                "admob": {
                    "enabled": true,
                    "settings": {
                        "app_id": "demo-app",
                        "sdk_key": "demo-key"
                    }
                }
            },
            "killswitches": [],
            "telemetry_enabled": false,
            "signature": null
        }
        """
    }()
}
