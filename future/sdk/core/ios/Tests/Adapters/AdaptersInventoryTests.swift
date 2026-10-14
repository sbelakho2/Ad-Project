import XCTest
@testable import RivalApexMediationSDK

final class AdaptersInventoryTests: XCTestCase {

    func testRegisteredCountAndDiscovery() {
        let registry = AdapterRegistry(sdkVersion: "1.0.0")
        XCTAssertGreaterThanOrEqual(registry.registeredCount, 3, "Expected at least 3 built-in adapters")

        let report = registry.getInitializationReport()
        let names = Set(report.map { $0.networkName })
        XCTAssertTrue(names.contains("admob"))
        XCTAssertTrue(names.contains("applovin"))
        XCTAssertTrue(names.contains("unity"))

        // None should be initialized before initializeAdapter is called
        report.forEach { status in
            if ["admob", "applovin", "unity"].contains(status.networkName) {
                XCTAssertTrue(status.registered)
            }
        }
    }

    func testInitializeAdaptersAndMockLoadUnity() {
        let registry = AdapterRegistry(sdkVersion: "1.0.0")

        // Initialize AdMob and AppLovin (no load expected from their mock implementations)
        if let _ = registry.getAdapter(networkName: "admob") {
            XCTAssertNoThrow(registry.initializeAdapter(networkName: "admob", config: ["app_id": "ca-app-pub-TEST"]))
            XCTAssertTrue(registry.isInitialized(networkName: "admob"))
        }
        if let _ = registry.getAdapter(networkName: "applovin") {
            XCTAssertNoThrow(registry.initializeAdapter(networkName: "applovin", config: ["sdk_key": "AL-TEST-KEY"]))
            XCTAssertTrue(registry.isInitialized(networkName: "applovin"))
        }

        // Unity: initialize and perform a mock ad load which should succeed
        guard let unity = registry.getAdapter(networkName: "unity") else {
            XCTFail("Unity adapter should be available")
            return
        }
        XCTAssertNoThrow(registry.initializeAdapter(networkName: "unity", config: ["game_id": "UNITY-TEST-GAME"]))
        XCTAssertTrue(registry.isInitialized(networkName: "unity"))

        let exp = expectation(description: "Unity mock ad load")
        unity.loadAd(placement: "home", adType: .banner, config: [:]) { result in
            switch result {
            case .success(let ad):
                XCTAssertEqual(ad.networkName, "unity")
                exp.fulfill()
            case .failure(let err):
                XCTFail("Expected success, got error: \(err)")
            }
        }
        waitForExpectations(timeout: 3.0)

        // Diagnostics should reflect initialized status
        let diag = registry.getInitializationReport()
        let byName = Dictionary(uniqueKeysWithValues: diag.map { ($0.networkName, $0) })
        ["admob", "applovin", "unity"].forEach { name in
            guard let status = byName[name] else { return }
            XCTAssertTrue(status.registered)
            XCTAssertEqual(status.initialized, true)
            XCTAssertEqual(status.version, "1.0.0")
        }
    }
}
