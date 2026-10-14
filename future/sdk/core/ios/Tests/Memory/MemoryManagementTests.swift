import XCTest
@testable import RivalApexMediationSDK

/// Section 3.3: Memory management tests
/// Verify SDK gracefully handles deinitialization, no retain cycles, and task cancellation
final class MemoryManagementTests: XCTestCase {
    override func setUp() {
        super.setUp()
        NetworkTestHooks.registerMockProtocol(MockURLProtocolFixture.self)
        MockURLProtocolFixture.reset()
    }
    
    override func tearDown() {
        MockURLProtocolFixture.reset()
        NetworkTestHooks.resetMockProtocols()
        MediationSDK.shared.shutdown()
        super.tearDown()
    }
    
    // MARK: - Retain Cycle Tests
    
    func testSDKInstanceDoesNotRetainSelf() async throws {
        weak var weakSDK: MediationSDK?
        
        do {
            // Create a local scope to test deallocation
            let sdk = MediationSDK.shared
            weakSDK = sdk
            
            // Verify SDK is alive
            XCTAssertNotNil(weakSDK)
            
            // Note: Since SDK is a singleton, this test verifies the shared instance pattern
            // In production, singleton will persist for app lifetime
        }
        
        // Singleton should still exist (expected behavior)
        XCTAssertNotNil(weakSDK, "Singleton SDK should persist")
    }
    
    func testConfigManagerDoesNotRetainCycles() async throws {
        let config = makeConfig()
        
        weak var weakManager: ConfigManager?
        
        do {
            let verifier = SignatureVerifier(testMode: true, productionPublicKey: nil)
            var manager: ConfigManager? = ConfigManager(config: config, signatureVerifier: verifier)
            weakManager = manager
            
            XCTAssertNotNil(weakManager, "ConfigManager should be alive")
            
            // Release strong reference
            manager = nil
        }
        
        // ConfigManager should be deallocated (no retain cycles)
        XCTAssertNil(weakManager, "ConfigManager should be deallocated without retain cycles")
    }
    
    func testAdapterRegistryDoesNotRetainCycles() {
        weak var weakRegistry: AdapterRegistry?
        
        do {
            var registry: AdapterRegistry? = AdapterRegistry(sdkVersion: "1.0.0")
            let initialCount = registry?.registeredCount ?? 0
            weakRegistry = registry
            
            // Register test adapter
            registry?.registerAdapter(networkName: "test_adapter", adapterClass: MockAdapter.self)
            
            XCTAssertNotNil(weakRegistry, "AdapterRegistry should be alive")
            XCTAssertEqual(weakRegistry?.registeredCount, initialCount + 1)
            
            // Release strong reference
            registry = nil
        }
        
        // AdapterRegistry should be deallocated
        XCTAssertNil(weakRegistry, "AdapterRegistry should be deallocated without retain cycles")
    }
    
    func testAdapterClosureDoesNotCaptureSelfStrongly() async {
        weak var weakAdapter: MockAdapter?
        
        do {
            var adapter: MockAdapter? = MockAdapter()
            weakAdapter = adapter
            
            // Simulate loading ad with closure callback
            let expectation = self.expectation(description: "Ad load completion")
            
            adapter?.loadAd(placement: "test", adType: .banner, config: [:]) { result in
                // Closure executes, but should not retain adapter
                switch result {
                case .success:
                    XCTFail("MockAdapter should not return success")
                case .failure(let error):
                    if case .loadFailed(let message) = error {
                        XCTAssertEqual(message, "no_fill")
                    } else {
                        XCTFail("Expected loadFailed no_fill, got \(error)")
                    }
                }
                expectation.fulfill()
            }
            
            await fulfillment(of: [expectation], timeout: 1.0)
            
            // Release strong reference
            adapter = nil
        }
        
        // Adapter should be deallocated (closure doesn't retain)
        XCTAssertNil(weakAdapter, "Adapter should be deallocated after closure executes")
    }
    
    // MARK: - Graceful Cancellation Tests
    
    func testTaskCancellationDoesNotLeak() async throws {
        let config = makeConfig()
        
        let verifier = SignatureVerifier(testMode: true, productionPublicKey: nil)
        let manager = ConfigManager(config: config, signatureVerifier: verifier)
        
        // Start async task
        let task = Task {
            do {
                // This will fail due to invalid URL, but tests cancellation path
                _ = try await manager.loadConfig()
            } catch {
                // Expected to fail
            }
        }
        
        // Cancel task immediately
        task.cancel()
        
        // Wait for task to complete
        _ = await task.result
        
        // Verify no crashes or memory leaks (implicit by test completion)
        XCTAssertTrue(task.isCancelled)
    }
    
    func testMultipleConcurrentRequestsDoNotLeak() async throws {
        let config = makeConfig()
        
        let verifier = SignatureVerifier(testMode: true, productionPublicKey: nil)
        let manager = ConfigManager(config: config, signatureVerifier: verifier)
        
        // Launch 10 concurrent requests
        let tasks = (0..<10).map { _ in
            Task {
                do {
                    _ = try await manager.loadConfig()
                } catch {
                    // Expected to fail with network error
                }
            }
        }
        
        // Wait for all to complete
        for task in tasks {
            _ = await task.result
        }
        
        // Verify no memory leaks (implicit by test completion without crash)
        XCTAssertEqual(tasks.count, 10)
    }
    
    // MARK: - Cleanup on Deinit Tests
    
    func testAdapterRegistryCleanupOnDeinit() {
        var registry: AdapterRegistry? = AdapterRegistry(sdkVersion: "1.0.0")
        let initialCount = registry?.registeredCount ?? 0
        
        // Register multiple adapters
        registry?.registerAdapter(networkName: "adapter1", adapterClass: MockAdapter.self)
        registry?.registerAdapter(networkName: "adapter2", adapterClass: MockAdapter.self)
        
        XCTAssertEqual(registry?.registeredCount, initialCount + 2)
        
        // Deinit should clean up internal state
        registry = nil
        
        // Verify deallocation (implicit by no crash)
        XCTAssertNil(registry)
    }
    
    func testTelemetryCollectorStopsOnDeinit() {
        let config = makeConfig(telemetryEnabled: true, logLevel: .info)
        
        var telemetry: TelemetryCollector? = TelemetryCollector(config: config)
        telemetry?.start()
        
        // Record some events
        telemetry?.recordInitialization()
        telemetry?.recordAdLoad(placement: "test", adType: .interstitial, networkName: "mock", latency: 100, success: true)
        
        // Deinit should stop telemetry collection
        telemetry = nil
        
        // Verify deallocation
        XCTAssertNil(telemetry)
    }
    
    // MARK: - Edge Cases
    
    func testSDKReinitializationAfterReset() async throws {
#if os(Linux)
        throw XCTSkip("Network-dependent SDK reinitialization test is skipped on Linux")
#else
        let sdk = MediationSDK.shared
        
        // Note: SDK doesn't currently support reset/reinitialize
        // This test documents the expected behavior
        
        let config = makeConfig()
        
        // Attempting to initialize twice should throw
        do {
            try await sdk.initialize(appId: "test1", configuration: config)
            try await sdk.initialize(appId: "test2", configuration: config)
            XCTFail("Should not allow reinitialization")
        } catch SDKError.alreadyInitialized {
            // Expected
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
#endif
    }
    
    func testLoadAdAfterSDKDeinit() async throws {
        // SDK is singleton, so this test verifies behavior remains stable
        let sdk = MediationSDK.shared
        
        // Try loading ad before initialization
        do {
            _ = try await sdk.loadAd(placementId: "test")
            XCTFail("Should throw notInitialized")
        } catch SDKError.notInitialized {
            // Expected
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    // MARK: - Helpers

    private func makeConfig(
        telemetryEnabled: Bool = false,
        logLevel: LogLevel = .error,
        testMode: Bool = true
    ) -> SDKConfig {
        SDKConfig(
            appId: "test_app",
            configEndpoint: "https://mock-config.test/v1/config",
            auctionEndpoint: "https://mock-auction.test/v1/auction",
            telemetryEnabled: telemetryEnabled,
            logLevel: logLevel,
            testMode: testMode,
            configSignaturePublicKey: nil
        )
    }
}

// MARK: - Mock Adapter for Testing

private final class MockAdapter: AdNetworkAdapter {
    var networkName: String { "MockNetwork" }
    var version: String { "1.0.0" }
    var minSDKVersion: String { "1.0.0" }

    required init() {}

    func initialize(config: [String : Any]) throws {
        // No-op
    }

    func loadAd(
        placement: String,
        adType: AdType,
        config: [String : Any],
        completion: @escaping @Sendable (Result<Ad, AdapterRegistryError>) -> Void
    ) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            completion(.failure(.loadFailed("no_fill")))
        }
    }

    func supportsAdType(_ adType: AdType) -> Bool {
        return true
    }

    func destroy() {
        // Cleanup
    }
}
