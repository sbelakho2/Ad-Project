import XCTest
@testable import RivalApexMediationSDK

final class AdMobAdapterTests: XCTestCase {

    func testInitializeRequiresAppId() throws {
        let adapter = AdMobAdapter()
        XCTAssertThrowsError(try adapter.initialize(config: [:])) { error in
            guard case AdapterRegistryError.loadFailed(let msg) = error else {
                return XCTFail("Unexpected error: \(error)")
            }
            XCTAssertTrue(msg.contains("app_id"))
        }
    }

    func testLoadBannerSuccessWithCpmOverride() throws {
        let adapter = AdMobAdapter()
        try adapter.initialize(config: ["app_id": "ca-app-pub-TEST", "apx_consent_state": ["gdpr": 1]])

        let exp = expectation(description: "load success")
        adapter.loadAd(placement: "p1", adType: .banner, config: ["cpm": 2.50]) { result in
            switch result {
            case .success(let ad):
                XCTAssertEqual(ad.networkName, "admob")
                XCTAssertEqual(ad.adType, .banner)
                XCTAssertGreaterThan(ad.cpm, 2.0)
                exp.fulfill()
            case .failure(let err):
                XCTFail("Unexpected failure: \(err)")
            }
        }

        wait(for: [exp], timeout: 1.0)
    }

    func testNoFillSimulation() throws {
        let adapter = AdMobAdapter()
        try adapter.initialize(config: ["app_id": "ca-app-pub-TEST"])

        let exp = expectation(description: "no fill")
        adapter.loadAd(placement: "p2", adType: .interstitial, config: ["no_fill": true]) { result in
            switch result {
            case .success:
                XCTFail("Expected no fill")
            case .failure(let err):
                guard case AdapterRegistryError.loadFailed(let msg) = err else {
                    return XCTFail("Unexpected error: \(err)")
                }
                XCTAssertEqual(msg, "no_fill")
                exp.fulfill()
            }
        }

        wait(for: [exp], timeout: 1.0)
    }

    func testTimeoutSimulation() throws {
        let adapter = AdMobAdapter()
        try adapter.initialize(config: ["app_id": "ca-app-pub-TEST"])

        let exp = expectation(description: "timeout")
        adapter.loadAd(placement: "p3", adType: .rewarded, config: ["timeout_ms": 25]) { result in
            switch result {
            case .success:
                XCTFail("Expected timeout")
            case .failure(let err):
                guard case AdapterRegistryError.timeout = err else {
                    return XCTFail("Unexpected error: \(err)")
                }
                exp.fulfill()
            }
        }

        wait(for: [exp], timeout: 2.0)
    }
}
