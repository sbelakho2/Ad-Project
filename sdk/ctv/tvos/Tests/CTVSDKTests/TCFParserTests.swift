import XCTest
@testable import CTVSDK

final class TCFParserTests: XCTestCase {
    
    func testEmptyStringReturnsFalseConsent() {
        let result = TCFParser.parse(tcfV2: nil, gdprApplies: true)
        
        XCTAssertTrue(result.gdprApplies)
        XCTAssertFalse(result.purpose1Consent)
        XCTAssertEqual(result.raw, "")
    }
    
    func testEmptyStringWithGdprNotApplying() {
        let result = TCFParser.parse(tcfV2: "", gdprApplies: false)
        
        XCTAssertFalse(result.gdprApplies)
        XCTAssertFalse(result.purpose1Consent)
        XCTAssertEqual(result.raw, "")
    }
    
    func testInvalidFormatReturnsFalseConsent() {
        // No dots - invalid TCF format
        let result = TCFParser.parse(tcfV2: "invalid_no_dots", gdprApplies: true)
        
        XCTAssertTrue(result.gdprApplies)
        XCTAssertFalse(result.purpose1Consent)
        XCTAssertEqual(result.raw, "invalid_no_dots")
    }
    
    func testInvalidCharactersReturnsFalseConsent() {
        // Contains invalid characters
        let result = TCFParser.parse(tcfV2: "invalid!@#$.string", gdprApplies: true)
        
        XCTAssertTrue(result.gdprApplies)
        XCTAssertFalse(result.purpose1Consent)
    }
    
    func testValidFormatStringPreservesRaw() {
        let tcfString = "COvFyGBOvFyGBAbAAAENAPCAAOAAAAAAAAAAAEEUACCKAAA.IFoEUQQgAIQwgIwQABAEAAAAOIAACAIAAAAQAIAgEAACEAAAAAgAQBAAAAAAAGBAAgAAAAAAAFAAECAAAgAAQARAEQAAAAAJAAIAAgAAAYQEAAAQmAgBC3ZAYzUw.QFukWSQgAIQwgI0QEByFAAAAeIAACAIgSAAQAIAgEAACEAAAAIgAQBAAAAAAAGDAAgAAAAAFAAICAAAgAAQABEBQAAAAAJAAIACgAAACIQEAAATBQCCMAoFRBGJjQA"
        
        let result = TCFParser.parse(tcfV2: tcfString, gdprApplies: true)
        
        XCTAssertEqual(result.raw, tcfString)
        XCTAssertTrue(result.gdprApplies)
    }
    
    func testHasAdConsentWhenGdprDoesNotApply() {
        let hasConsent = TCFParser.hasAdConsent(nil, gdprApplies: false)
        XCTAssertTrue(hasConsent)
    }
    
    func testHasAdConsentWhenGdprAppliesButNoString() {
        let hasConsent = TCFParser.hasAdConsent(nil, gdprApplies: true)
        XCTAssertFalse(hasConsent)
    }
    
    func testCoreEquatable() {
        let core1 = TCFParser.Core(gdprApplies: true, purpose1Consent: true, raw: "test")
        let core2 = TCFParser.Core(gdprApplies: true, purpose1Consent: true, raw: "test")
        let core3 = TCFParser.Core(gdprApplies: false, purpose1Consent: true, raw: "test")
        
        XCTAssertEqual(core1, core2)
        XCTAssertNotEqual(core1, core3)
    }
    
    func testBase64URLDecodingWithPadding() {
        // A valid-looking TCF string structure
        let tcfString = "CO.test.string"
        
        let result = TCFParser.parse(tcfV2: tcfString, gdprApplies: true)
        
        // Should not crash, should preserve raw
        XCTAssertEqual(result.raw, tcfString)
    }
    
    func testLongValidTCFStringFormat() {
        // A realistic-looking TCF v2 string with proper segments
        let tcfString = "CPXxRfAPXxRfAAfKABENB-CgAAAAAAAAAAYgAAAAAAAA.YAAAAAAAAAA"
        
        let result = TCFParser.parse(tcfV2: tcfString, gdprApplies: true)
        
        XCTAssertTrue(result.gdprApplies)
        XCTAssertEqual(result.raw, tcfString)
    }
}
