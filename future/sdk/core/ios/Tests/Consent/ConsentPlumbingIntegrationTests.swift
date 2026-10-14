import XCTest
@testable import RivalApexMediationSDK

final class ConsentPlumbingIntegrationTests: XCTestCase {
    override func setUp() {
        super.setUp()
        ConsentManager.shared.clear()
    }
    
    override func tearDown() {
        ConsentManager.shared.clear()
        super.tearDown()
    }
    
    func testMediationSDKMetadataReflectsSetConsent() {
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "meta-string",
            ccpaOptOut: false,
            coppa: false
        )
        MediationSDK.shared.setConsent(consent)
        let metadata = MediationSDK.shared.consentMetadata()
        XCTAssertEqual(metadata["gdpr"] as? Int, 1)
        XCTAssertEqual(metadata["gdpr_consent"] as? String, "meta-string")
    }
    
    func testConsentSummaryRedactsSensitiveFields() {
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "should-be-redacted",
            ccpaOptOut: true,
            coppa: true
        )
        MediationSDK.shared.setConsent(consent)
        let summary = MediationSDK.shared.consentSummary
        XCTAssertEqual(summary["gdprApplies"] as? Bool, true)
        XCTAssertEqual(summary["gdprConsentString"] as? String, "<redacted>")
        XCTAssertEqual(summary["ccpaOptOut"] as? Bool, true)
        XCTAssertEqual(summary["coppa"] as? Bool, true)
    }
}
