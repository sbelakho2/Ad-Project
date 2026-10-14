import XCTest
@testable import RivalApexMediationSDK

final class ConsentManagerTests: XCTestCase {
    
    override func setUp() {
        super.setUp()
        ConsentManager.shared.clear()
    }
    
    override func tearDown() {
        ConsentManager.shared.clear()
        super.tearDown()
    }
    
    func testDefaultConsent() {
        let consent = ConsentManager.shared.getConsent()
        
        XCTAssertNil(consent.gdprApplies)
        XCTAssertNil(consent.gdprConsentString)
        XCTAssertFalse(consent.ccpaOptOut)
        XCTAssertFalse(consent.coppa)
    }
    
    func testSetAndGetConsent() {
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "test-string",
            usPrivacyString: "1YNN",
            ccpaOptOut: true,
            coppa: false
        )
        
        ConsentManager.shared.setConsent(consent)
        let retrieved = ConsentManager.shared.getConsent()
        
        XCTAssertEqual(retrieved.gdprApplies, true)
        XCTAssertEqual(retrieved.gdprConsentString, "test-string")
        XCTAssertEqual(retrieved.usPrivacyString, "1YNN")
        XCTAssertEqual(retrieved.ccpaOptOut, true)
        XCTAssertEqual(retrieved.coppa, false)
    }
    
    func testConsentPersistence() {
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "persistent-string",
            ccpaOptOut: false,
            coppa: false
        )
        let suiteName = "com.rival.apex.tests.consent"
        let suite = UserDefaults(suiteName: suiteName) ?? .standard
        suite.removePersistentDomain(forName: suiteName)
        let manager = ConsentManager(storage: suite)
        manager.setConsent(consent)
        let newManager = ConsentManager(storage: suite)
        let retrieved = newManager.getConsent()
        
        XCTAssertEqual(retrieved.gdprApplies, true)
        XCTAssertEqual(retrieved.gdprConsentString, "persistent-string")
    }
    
    func testCanShowPersonalizedAds_Default() {
        let canShow = ConsentManager.shared.canShowPersonalizedAds()
        XCTAssertTrue(canShow, "Default should allow personalized ads")
    }
    
    func testCanShowPersonalizedAds_COPPA() {
        let consent = ConsentData(coppa: true)
        ConsentManager.shared.setConsent(consent)
        
        let canShow = ConsentManager.shared.canShowPersonalizedAds()
        XCTAssertFalse(canShow, "COPPA users should not see personalized ads")
    }
    
    func testCanShowPersonalizedAds_CCPAOptOut() {
        let consent = ConsentData(ccpaOptOut: true)
        ConsentManager.shared.setConsent(consent)
        
        let canShow = ConsentManager.shared.canShowPersonalizedAds()
        XCTAssertFalse(canShow, "CCPA opt-out should prevent personalized ads")
    }

    func testCanShowPersonalizedAds_UsesRawUSPrivacyString() {
        let consent = ConsentData(usPrivacyString: "1YNN", ccpaOptOut: false)
        ConsentManager.shared.setConsent(consent)
        XCTAssertFalse(ConsentManager.shared.canShowPersonalizedAds())
    }
    
    func testCanShowPersonalizedAds_GDPRWithConsent() {
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "valid-consent-string"
        )
        ConsentManager.shared.setConsent(consent)
        
        let canShow = ConsentManager.shared.canShowPersonalizedAds()
        XCTAssertTrue(canShow, "GDPR with consent string should allow personalized ads")
    }
    
    func testCanShowPersonalizedAds_GDPRWithoutConsent() {
        let consent = ConsentData(gdprApplies: true, gdprConsentString: nil)
        ConsentManager.shared.setConsent(consent)
        
        let canShow = ConsentManager.shared.canShowPersonalizedAds()
        XCTAssertFalse(canShow, "GDPR without consent string should prevent personalized ads")
    }
    
    func testToAdRequestMetadata_GDPR() {
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "test-consent"
        )
        ConsentManager.shared.setConsent(consent)
        
        let metadata = ConsentManager.shared.toAdRequestMetadata()
        
        XCTAssertEqual(metadata["gdpr"] as? Int, 1)
        XCTAssertEqual(metadata["gdpr_consent"] as? String, "test-consent")
    }
    
    func testToAdRequestMetadata_CCPA() {
        let consent = ConsentData(ccpaOptOut: true)
        ConsentManager.shared.setConsent(consent)
        
        let metadata = ConsentManager.shared.toAdRequestMetadata()
        
        XCTAssertEqual(metadata["us_privacy"] as? String, "1YNN")
    }
    
    func testToAdRequestMetadata_UsesProvidedUSPrivacyString() {
        let consent = ConsentData(usPrivacyString: "1N--", ccpaOptOut: false)
        ConsentManager.shared.setConsent(consent)
        let metadata = ConsentManager.shared.toAdRequestMetadata()
        XCTAssertEqual(metadata["us_privacy"] as? String, "1N--")
    }
    
    func testToAdRequestMetadata_COPPA() {
        let consent = ConsentData(coppa: true)
        ConsentManager.shared.setConsent(consent)
        
        let metadata = ConsentManager.shared.toAdRequestMetadata()
        
        XCTAssertEqual(metadata["coppa"] as? Int, 1)
    }
    
    func testGetRedactedConsentInfo() {
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "sensitive-string-should-be-redacted",
            usPrivacyString: "1YYN",
            ccpaOptOut: true,
            coppa: false
        )
        ConsentManager.shared.setConsent(consent)
        
        let redacted = ConsentManager.shared.getRedactedConsentInfo()
        
        XCTAssertEqual(redacted["gdprApplies"] as? Bool, true)
        XCTAssertEqual(redacted["gdprConsentString"] as? String, "<redacted>")
        XCTAssertEqual(redacted["usPrivacyString"] as? String, "<redacted>")
        XCTAssertEqual(redacted["ccpaOptOut"] as? Bool, true)
        XCTAssertEqual(redacted["coppa"] as? Bool, false)
    }

    func testAdapterConsentPayloadIncludesAttStatus() {
        let consent = ConsentData(
            gdprApplies: true,
            gdprConsentString: "consent-string",
            usPrivacyString: "1---",
            ccpaOptOut: false,
            coppa: false
        )
        ConsentManager.shared.setConsent(consent)
        let payload = ConsentManager.shared.toAdapterConsentPayload { .authorized }
        XCTAssertEqual(payload["iab_tcf_v2"] as? String, "consent-string")
        XCTAssertEqual(payload["iab_us_privacy"] as? String, "1---")
        XCTAssertEqual(payload["att_status"] as? String, ATTStatus.authorized.rawValue)
        XCTAssertEqual(payload["limit_ad_tracking"] as? Bool, false)
    }
}
