import XCTest
@testable import RivalApexMediationSDK

final class TCFParserTests: XCTestCase {

    private struct BitWriter {
        private var bits = ""

        mutating func write(_ value: Int, _ count: Int) {
            for i in stride(from: count - 1, through: 0, by: -1) {
                bits.append((value >> i) & 1 == 1 ? "1" : "0")
            }
        }

        func toBase64URL() -> String {
            var padded = bits
            while padded.count % 8 != 0 { padded.append("0") }
            var bytes: [UInt8] = []
            var index = padded.startIndex
            while index < padded.endIndex {
                let next = padded.index(index, offsetBy: 8)
                bytes.append(UInt8(padded[index..<next], radix: 2) ?? 0)
                index = next
            }
            let data = Data(bytes)
            return data.base64EncodedString()
                .replacingOccurrences(of: "+", with: "-")
                .replacingOccurrences(of: "/", with: "_")
                .replacingOccurrences(of: "=", with: "")
        }
    }

    private func tcfString(purpose1: Bool) -> String {
        var w = BitWriter()
        w.write(2, 6) // version
        w.write(0, 36) // created
        w.write(0, 36) // lastUpdated
        w.write(0, 12) // cmpId
        w.write(0, 12) // cmpVersion
        w.write(0, 6) // consentScreen
        w.write(0, 12) // consentLanguage
        w.write(0, 12) // vendorListVersion
        w.write(2, 6) // tcfPolicyVersion
        w.write(0, 1) // isServiceSpecific
        w.write(0, 1) // useNonStandardStacks
        w.write(0, 12) // specialFeatureOptIns
        w.write(purpose1 ? 1 << 23 : 0, 24) // purposesConsent
        w.write(0, 24) // purposesLITransparency
        w.write(0, 1) // purposeOneTreatment
        w.write(0, 12) // publisherCC
        return w.toBase64URL()
    }

    func testParseReadsPurpose1FromTheCoreSegment() {
        XCTAssertTrue(TCFParser.parse(tcfV2: tcfString(purpose1: true), gdprApplies: true).purpose1Consent)
        XCTAssertFalse(TCFParser.parse(tcfV2: tcfString(purpose1: false), gdprApplies: true).purpose1Consent)
    }

    func testParseUsesOnlyTheFirstSegment() {
        let segmented = tcfString(purpose1: true) + ".disclosedvendors"
        XCTAssertTrue(TCFParser.parse(tcfV2: segmented, gdprApplies: true).purpose1Consent)
    }

    func testParseFailsClosedOnMissingMalformedOrTruncatedInput() {
        XCTAssertFalse(TCFParser.parse(tcfV2: nil, gdprApplies: true).purpose1Consent)
        XCTAssertFalse(TCFParser.parse(tcfV2: "", gdprApplies: true).purpose1Consent)
        XCTAssertFalse(TCFParser.parse(tcfV2: "!!!not-base64!!!", gdprApplies: true).purpose1Consent)
        let truncated = Data(repeating: 0, count: 10).base64EncodedString()
        XCTAssertFalse(TCFParser.parse(tcfV2: truncated, gdprApplies: true).purpose1Consent)
        XCTAssertFalse(TCFParser.parse(tcfV2: nil, gdprApplies: false).gdprApplies)
    }

    func testConsentManagerGateMatchesTheAndroidRules() {
        let consenting = tcfString(purpose1: true)
        let manager = ConsentManager(storage: UserDefaults(suiteName: "tcf-parse-tests")!)
        manager.setConsent(ConsentData(gdprApplies: true, gdprConsentString: consenting))
        XCTAssertTrue(manager.canShowPersonalizedAds())

        manager.setConsent(ConsentData(gdprApplies: true, gdprConsentString: tcfString(purpose1: false)))
        XCTAssertFalse(manager.canShowPersonalizedAds())

        manager.setConsent(ConsentData(gdprApplies: true, gdprConsentString: "garbage"))
        XCTAssertFalse(manager.canShowPersonalizedAds())

        manager.setConsent(ConsentData(gdprApplies: true))
        XCTAssertFalse(manager.canShowPersonalizedAds())

        manager.setConsent(ConsentData(gdprApplies: true, gdprConsentString: consenting, coppa: true))
        XCTAssertFalse(manager.canShowPersonalizedAds())

        manager.setConsent(ConsentData(gdprApplies: true, gdprConsentString: consenting, usPrivacyString: "1YNN"))
        XCTAssertFalse(manager.canShowPersonalizedAds())

        manager.setConsent(ConsentData(gdprApplies: true, gdprConsentString: consenting, usPrivacyString: "1NNN"))
        XCTAssertTrue(manager.canShowPersonalizedAds())

        manager.setConsent(ConsentData())
        XCTAssertTrue(manager.canShowPersonalizedAds())
    }
}
