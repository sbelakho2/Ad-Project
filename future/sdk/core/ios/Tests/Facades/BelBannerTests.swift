import XCTest
@testable import RivalApexMediationSDK

#if canImport(UIKit)
import UIKit

@MainActor
final class BelBannerTests: XCTestCase {
    
    var testViewController: UIViewController!
    
    override func setUp() async throws {
        try await super.setUp()
        testViewController = UIViewController()
        
        // Load test view
        _ = testViewController.view
    }
    
    override func tearDown() {
        testViewController = nil
        super.tearDown()
    }
    
    func testBannerSizeDimensions() {
        let standard = BelBanner.BannerSize.standard.dimensions
        XCTAssertEqual(standard.width, 320)
        XCTAssertEqual(standard.height, 50)
        
        let mediumRect = BelBanner.BannerSize.mediumRectangle.dimensions
        XCTAssertEqual(mediumRect.width, 300)
        XCTAssertEqual(mediumRect.height, 250)
        
        let leaderboard = BelBanner.BannerSize.leaderboard.dimensions
        XCTAssertEqual(leaderboard.width, 728)
        XCTAssertEqual(leaderboard.height, 90)
        
        let adaptive = BelBanner.BannerSize.adaptive.dimensions
        XCTAssertGreaterThan(adaptive.width, 0)
        XCTAssertGreaterThan(adaptive.height, 0)
    }
    
    func testIsReadyBeforeLoad() {
        XCTAssertFalse(BelBanner.isReady(placementId: "test-banner"))
    }
    
    func testShowHideDestroy() {
        // These methods should not crash when called on non-existent banners
        BelBanner.show(placementId: "non-existent")
        BelBanner.hide(placementId: "non-existent")
        BelBanner.destroy(placementId: "non-existent")
    }
    
    // Note: Full banner lifecycle test would require SDK initialization
    // and mock ad responses, which is tested in integration tests
}

#else

final class BelBannerTests: XCTestCase {
    func testUIKitUnavailable() throws {
        throw XCTSkip("UIKit not available on this platform")
    }
}

#endif
