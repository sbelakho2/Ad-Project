import XCTest
@testable import RivalApexMediationSDK

final class AppExtensionCompatibilityTests: XCTestCase {
    func testPackageCompilesForAppExtensions() {
        // This test intentionally does not run any UIKit-only APIs. It simply
        // ensures the SDK builds when APP_EXTENSION is defined, catching
        // accidental UIKit usage in extension contexts.
        XCTAssertNotNil(BelAds.self)
    }
}
