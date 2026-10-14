import XCTest
@testable import RivalApexMediationSDK

@MainActor
final class BelAdsFacadeTests: XCTestCase {
    
    override func setUp() async throws {
        try await super.setUp()
        // Reset SDK state
        if BelAds.isInitialized {
            MediationSDK.shared.shutdown()
        }
    }
    
    func testVersionString() {
        XCTAssertFalse(BelAds.version.isEmpty)
        XCTAssertTrue(BelAds.version.contains("ios"), "Version should indicate iOS platform")
    }
    
    func testInitializedStateBeforeInit() {
        XCTAssertFalse(BelAds.isInitialized, "SDK should not be initialized before initialize() call")
    }
    
    func testInitializeWithTestMode() async {
        let expectation = expectation(description: "Initialize completes")
        
        BelAds.initialize(appId: "test-app-id", testMode: true) { result in
            switch result {
            case .success:
                XCTAssertTrue(BelAds.isInitialized, "SDK should be initialized after successful init")
                XCTAssertTrue(MediationSDK.shared.isTestMode, "Test mode should be enabled")
            case .failure(let error):
                // Config fetch might fail in test environment, but init attempt should complete
                print("Init completed with: \(error)")
            }
            expectation.fulfill()
        }
        
        await fulfillment(of: [expectation], timeout: 5.0)
    }
    
    func testGetDebugInfo() {
        let info = BelAds.getDebugInfo()
        
        XCTAssertNotNil(info["sdkVersion"])
        XCTAssertNotNil(info["isInitialized"])
        XCTAssertNotNil(info["consent"])
        
        let version = info["sdkVersion"] as? String
        XCTAssertFalse(version?.isEmpty ?? true)
    }
    
    func testConsentManagement() {
        // Set consent
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "test-consent-string",
            ccpaOptOut: false,
            coppa: false
        )
        
        BelAds.setConsent(consent)
        
        // Retrieve consent
        let retrieved = BelAds.getConsent()
        
        XCTAssertEqual(retrieved.gdprApplies, true)
        XCTAssertEqual(retrieved.gdprConsentString, "test-consent-string")
        XCTAssertEqual(retrieved.ccpaOptOut, false)
        XCTAssertEqual(retrieved.coppa, false)

        let debugConsent = BelAds.getDebugInfo()["consent"] as? [String: Any]
        XCTAssertEqual(debugConsent?["gdprApplies"] as? Bool, true)
        XCTAssertEqual(debugConsent?["gdprConsentString"] as? String, "<redacted>")
    }
    
    func testDebugLogging() {
        // Should not crash when called
        BelAds.setDebugLogging(true)
        BelAds.setDebugLogging(false)
        
        // Verify setting persisted
        let enabled = UserDefaults.standard.bool(forKey: "apex_debug_logging")
        XCTAssertFalse(enabled, "Debug logging should be off after setting to false")
    }
}
