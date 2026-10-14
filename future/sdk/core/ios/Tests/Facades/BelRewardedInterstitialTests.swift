import XCTest
@testable import RivalApexMediationSDK

#if canImport(UIKit)
import UIKit

@MainActor
final class BelRewardedInterstitialTests: XCTestCase {
    
    func testIsReadyBeforeLoad() {
        XCTAssertFalse(BelRewardedInterstitial.isReady())
    }
    
    func testShowWithoutLoad() {
        let vc = UIViewController()
        let listener = CapturingListener()
        
        let shown = BelRewardedInterstitial.show(from: vc, placementId: "rewarded_interstitial", listener: listener)
        
        XCTAssertFalse(shown, "Should not show when no ad is loaded")
        XCTAssertEqual(listener.failedToShowPlacement, "rewarded_interstitial")
        XCTAssertEqual(listener.failedToShowError as? SDKError, SDKError.noFill)
    }
}

private final class CapturingListener: BelAdEventListener {
    var failedToShowPlacement: String?
    var failedToShowError: Error?
    func onAdFailedToShow(placementId: String, error: Error) {
        failedToShowPlacement = placementId
        failedToShowError = error
    }
}

#else

final class BelRewardedInterstitialTests: XCTestCase {
    func testUIKitUnavailable() throws {
        throw XCTSkip("UIKit not available on this platform")
    }
}

#endif
